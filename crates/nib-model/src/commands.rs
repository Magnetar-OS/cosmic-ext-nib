// SPDX-License-Identifier: MPL-2.0

//! What the keys do.
//!
//! A [`Command`] takes a state and returns the transaction that carries out
//! some editing intent, or `None` when the intent does not apply here. Asking
//! "would Enter do anything?" and doing it are the same call — which is what
//! lets [`chain`] try a list of commands in order and stop at the first that
//! bites.
//!
//! That one signature is the whole extension point. `Backspace` is not a
//! special case in an event handler; it is
//! `chain([delete_selection, join_backward, select_node_backward])`, and an
//! extension that wants different behaviour at the start of its own node type
//! inserts a command into that list.
//!
//! # Why a transaction, not a boolean plus a callback
//!
//! ProseMirror's commands take a `dispatch` and return whether they applied,
//! so a caller can ask without doing. Returning `Option<Transaction>` says the
//! same thing in one value: `is_some()` is "would apply", and the transaction
//! is there when you want it. The cost is building a transaction that may be
//! thrown away, and a transaction is a document pointer and a few steps.

use std::sync::Arc;

use crate::attrs::Attrs;
use crate::content::ContentMatch;
use crate::fragment::Fragment;
use crate::mark::Marks;
use crate::node::Node;
use crate::resolve::{NodeRange, ResolvedPos};
use crate::schema::{MarkTypeId, NodeTypeId};
use crate::slice::Slice;
use crate::state::selection::Kind;
use crate::state::{EditorState, Selection, Transaction};
use crate::transform::fit::replace_step;
use crate::transform::structure::{
    MarkFilter, TypeAndAttrs, can_join, can_split, find_wrapping, lift_target,
};
use crate::transform::Step;

/// An editing intent.
pub type Command = Arc<dyn Fn(&EditorState) -> Option<Transaction> + Send + Sync>;

/// Wraps a closure as a [`Command`].
pub fn command(
    f: impl Fn(&EditorState) -> Option<Transaction> + Send + Sync + 'static,
) -> Command {
    Arc::new(f)
}

/// Tries each command in turn, taking the first that applies.
#[must_use]
pub fn chain(commands: Vec<Command>) -> Command {
    Arc::new(move |state| commands.iter().find_map(|c| c(state)))
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// The caret, when the selection is one sitting in inline content.
fn cursor(state: &EditorState) -> Option<ResolvedPos> {
    let pos = state.selection().cursor_pos()?;
    Some(state.doc().resolve(pos))
}

/// The caret, when it sits at the very start of its textblock.
fn at_block_start(state: &EditorState) -> Option<ResolvedPos> {
    let at = cursor(state)?;
    (at.parent_offset() == 0).then_some(at)
}

/// The caret, when it sits at the very end of its textblock.
fn at_block_end(state: &EditorState) -> Option<ResolvedPos> {
    let at = cursor(state)?;
    (at.parent_offset() == at.parent().content_size()).then_some(at)
}

/// The nearest position before `at` where two siblings meet.
fn find_cut_before(at: &ResolvedPos) -> Option<ResolvedPos> {
    if at.parent().typ().spec().isolating {
        return None;
    }
    for i in (0..at.depth()).rev() {
        if at.index(i) > 0 {
            return Some(at.doc().resolve(at.before(i + 1)));
        }
        if at.node(i).typ().spec().isolating {
            break;
        }
    }
    None
}

/// The nearest position after `at` where two siblings meet.
fn find_cut_after(at: &ResolvedPos) -> Option<ResolvedPos> {
    if at.parent().typ().spec().isolating {
        return None;
    }
    for i in (0..at.depth()).rev() {
        let parent = at.node(i);
        if at.index(i) + 1 < parent.child_count() {
            return Some(at.doc().resolve(at.after(i + 1)));
        }
        if parent.typ().spec().isolating {
            break;
        }
    }
    None
}

/// True when descending `node`'s first (or last) children reaches a textblock.
fn textblock_at(node: &Node, start: bool, only: bool) -> bool {
    let mut scan = Some(node.clone());
    while let Some(node) = scan {
        if node.is_textblock() {
            return true;
        }
        if only && node.child_count() != 1 {
            return false;
        }
        scan = if start {
            node.first_child().cloned()
        } else {
            node.last_child().cloned()
        };
    }
    false
}

/// The first textblock type a content match will accept — what a new empty
/// block should be.
fn default_block_at(state: &EditorState, matched: &ContentMatch) -> Option<NodeTypeId> {
    (0..matched.edge_count()).find_map(|i| {
        let (typ, _) = matched.edge(i)?;
        let t = state.schema().node_type(typ);
        (t.is_textblock() && !t.has_required_attrs()).then_some(typ)
    })
}

// ---------------------------------------------------------------------------
// Deleting
// ---------------------------------------------------------------------------

/// Removes the selection, when there is one.
#[must_use]
pub fn delete_selection() -> Command {
    command(|state| {
        if state.selection().is_empty() {
            return None;
        }
        let mut tr = state.tr();
        tr.delete_selection().ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Backspace at the start of a block: join with what is before, or lift out.
#[must_use]
pub fn join_backward() -> Command {
    command(|state| {
        let at = at_block_start(state)?;

        let Some(cut) = find_cut_before(&at) else {
            // Nothing before it in the document: try to lift it out of
            // whatever wraps it. Backspace at the start of the first
            // paragraph of a blockquote unquotes it.
            let range = at.block_range(&at, None)?;
            let target = lift_target(&range)?;
            let mut tr = state.tr();
            tr.lift(&range, target).ok()?;
            return Some(tr.clone().scroll_into_view());
        };

        if let Some(tr) = delete_barrier(state, &cut, -1) {
            return Some(tr);
        }

        let before = cut.node_before()?;

        // An empty block after something selectable: remove the block and
        // select or enter what was before it.
        if at.parent().content_size() == 0
            && (textblock_at(&before, false, false) || before.typ().spec().selectable)
        {
            for depth in (1..=at.depth()).rev() {
                let step = replace_step(
                    state.schema(),
                    state.doc(),
                    at.before(depth),
                    at.after(depth),
                    &Slice::empty(),
                );
                if let Some(step @ Step::Replace { from, to, .. }) = step.as_ref()
                    && let Step::Replace { slice, .. } = step
                    && slice.size() < to - from
                {
                    let mut tr = state.tr();
                    tr.step(step.clone()).ok()?;
                    let doc = tr.doc().clone();
                    let landing = tr.mapping().map(cut.pos(), -1).min(doc.content_size());
                    let selection = if textblock_at(&before, false, false) {
                        Selection::find_from(&doc, &doc.resolve(landing), -1, false)
                    } else {
                        Selection::node(&doc, cut.pos() - before.node_size())
                    };
                    if let Some(selection) = selection {
                        tr.set_selection(selection);
                    }
                    return Some(tr.clone().scroll_into_view());
                }
                if depth == 1 || at.node(depth - 1).child_count() > 1 {
                    break;
                }
            }
        }

        // An atom before the caret: delete it.
        if before.is_atom() && cut.depth() == at.depth() - 1 {
            let mut tr = state.tr();
            tr.delete(cut.pos() - before.node_size(), cut.pos()).ok()?;
            return Some(tr.clone().scroll_into_view());
        }
        None
    })
}

/// Delete at the end of a block: join with what follows.
#[must_use]
pub fn join_forward() -> Command {
    command(|state| {
        let at = at_block_end(state)?;
        let cut = find_cut_after(&at)?;
        if let Some(tr) = delete_barrier(state, &cut, 1) {
            return Some(tr);
        }
        let after = cut.node_after()?;
        if after.is_atom() && cut.depth() == at.depth() - 1 {
            let mut tr = state.tr();
            tr.delete(cut.pos(), cut.pos() + after.node_size()).ok()?;
            return Some(tr.clone().scroll_into_view());
        }
        None
    })
}

/// Selects the node before the caret rather than deleting it — the last resort
/// of Backspace, so a rule or an image gets selected before it is destroyed.
#[must_use]
pub fn select_node_backward() -> Command {
    command(|state| {
        let at = at_block_start(state)?;
        let cut = find_cut_before(&at)?;
        let before = cut.node_before()?;
        let selection = Selection::node(state.doc(), cut.pos() - before.node_size())?;
        let mut tr = state.tr();
        tr.set_selection(selection);
        Some(tr.clone().scroll_into_view())
    })
}

/// Selects the node after the caret rather than deleting it.
#[must_use]
pub fn select_node_forward() -> Command {
    command(|state| {
        let at = at_block_end(state)?;
        let cut = find_cut_after(&at)?;
        let selection = Selection::node(state.doc(), cut.pos())?;
        let mut tr = state.tr();
        tr.set_selection(selection);
        Some(tr.clone().scroll_into_view())
    })
}

/// The join that happens at a boundary between two blocks.
///
/// Four outcomes, tried in order: join the two directly; wrap the second in
/// whatever the first will accept and join; lift the second out; or pull the
/// second's text into the first. Which one fires is what makes Backspace at
/// the start of a list item behave like the user expects rather than like the
/// tree does.
fn delete_barrier(state: &EditorState, cut: &ResolvedPos, dir: i32) -> Option<Transaction> {
    let before = cut.node_before()?;
    let after = cut.node_after()?;
    let isolated = before.typ().spec().isolating || after.typ().spec().isolating;

    if !isolated
        && let Some(tr) = join_maybe_clear(state, cut)
    {
        return Some(tr);
    }

    let index = cut.index(cut.depth());
    let can_delete_after =
        !isolated && cut.parent().can_replace(index, index + 1, &Fragment::empty());

    if can_delete_after {
        let matched = before.content_match_at(before.child_count())?;
        if let Some(conn) = matched.find_wrapping(state.schema(), after.type_id()) {
            let head = conn.first().copied().unwrap_or_else(|| after.type_id());
            if matched.match_type(head).is_some_and(|m| m.valid_end()) {
                let end = cut.pos() + after.node_size();
                let mut wrap = Fragment::empty();
                for typ in conn.iter().rev() {
                    let node = state
                        .schema()
                        .create(*typ, None, wrap, Marks::none())
                        .ok()?;
                    wrap = Fragment::from(node);
                }
                wrap = Fragment::from(before.copy(wrap));
                let mut tr = state.tr();
                tr.step(Step::ReplaceAround {
                    from: cut.pos() - 1,
                    to: end,
                    gap_from: cut.pos(),
                    gap_to: end,
                    slice: Slice::new(wrap, 1, 0),
                    insert: conn.len(),
                    structure: true,
                })
                .ok()?;
                // Wrapping may have brought two nodes of the same type
                // together; join them so the user sees one list, not two.
                let join_at = end + 2 * conn.len();
                let doc = tr.doc().clone();
                if join_at <= doc.content_size() {
                    let at = doc.resolve(join_at);
                    if at
                        .node_after()
                        .is_some_and(|n| n.type_id() == before.type_id())
                        && can_join(&doc, join_at)
                    {
                        let _ = tr.join(join_at, 1);
                    }
                }
                return Some(tr.clone().scroll_into_view());
            }
        }
    }

    // Lift what follows out of whatever wraps it.
    let sel_after = if after.typ().spec().isolating || (dir > 0 && isolated) {
        None
    } else {
        Selection::find_from(state.doc(), cut, 1, false)
    };
    if let Some(sel) = sel_after {
        let from = state.doc().resolve(sel.from());
        let to = state.doc().resolve(sel.to());
        if let Some(range) = from.block_range(&to, None)
            && let Some(target) = lift_target(&range)
            && target >= cut.depth()
        {
            let mut tr = state.tr();
            tr.lift(&range, target).ok()?;
            return Some(tr.clone().scroll_into_view());
        }
    }

    // Pull the following textblock's content into the preceding one.
    if can_delete_after && textblock_at(&after, true, true) && textblock_at(&before, false, false) {
        let mut wrap = Vec::new();
        let mut at = before.clone();
        loop {
            wrap.push(at.clone());
            if at.is_textblock() {
                break;
            }
            at = at.last_child()?.clone();
        }
        let mut after_text = after.clone();
        let mut after_depth = 1;
        while !after_text.is_textblock() {
            after_text = after_text.first_child()?.clone();
            after_depth += 1;
        }
        if at.can_replace(at.child_count(), at.child_count(), after_text.content()) {
            let mut end = Fragment::empty();
            for node in wrap.iter().rev() {
                end = Fragment::from(node.copy(end));
            }
            let mut tr = state.tr();
            tr.step(Step::ReplaceAround {
                from: cut.pos() - wrap.len(),
                to: cut.pos() + after.node_size(),
                gap_from: cut.pos() + after_depth,
                gap_to: cut.pos() + after.node_size() - after_depth,
                slice: Slice::new(end, wrap.len(), 0),
                insert: 0,
                structure: true,
            })
            .ok()?;
            return Some(tr.clone().scroll_into_view());
        }
    }
    None
}

fn join_maybe_clear(state: &EditorState, at: &ResolvedPos) -> Option<Transaction> {
    let before = at.node_before()?;
    let after = at.node_after()?;
    if !before.typ().is_compatible_content(after.typ()) {
        return None;
    }
    let index = at.index(at.depth());
    // An empty block before: just take it away.
    if before.content_size() == 0
        && index > 0
        && at
            .parent()
            .can_replace(index - 1, index, &Fragment::empty())
    {
        let mut tr = state.tr();
        tr.delete(at.pos() - before.node_size(), at.pos()).ok()?;
        return Some(tr.clone().scroll_into_view());
    }
    if !at
        .parent()
        .can_replace(index, index + 1, &Fragment::empty())
        || !(after.is_textblock() || can_join(state.doc(), at.pos()))
    {
        return None;
    }
    let mut tr = state.tr();
    tr.join(at.pos(), 1).ok()?;
    Some(tr.clone().scroll_into_view())
}

/// Backspace.
#[must_use]
pub fn delete_backward() -> Command {
    chain(vec![
        delete_selection(),
        join_backward(),
        select_node_backward(),
    ])
}

/// Delete.
#[must_use]
pub fn delete_forward() -> Command {
    chain(vec![
        delete_selection(),
        join_forward(),
        select_node_forward(),
    ])
}

// ---------------------------------------------------------------------------
// Enter
// ---------------------------------------------------------------------------

/// Splits the block at the caret.
#[must_use]
pub fn split_block() -> Command {
    command(|state| {
        let selection = state.selection();
        let doc = state.doc();
        let from = doc.resolve(selection.from());
        let to = doc.resolve(selection.to());

        if selection.kind() == Kind::Node {
            let node = selection.selected_node(doc)?;
            if node.is_block() {
                if from.parent_offset() == 0 || !can_split(state.schema(), doc, from.pos(), 1, None)
                {
                    return None;
                }
                let mut tr = state.tr();
                tr.split(from.pos(), 1, None).ok()?;
                return Some(tr.clone().scroll_into_view());
            }
        }
        if !from.parent().is_block() {
            return None;
        }

        let at_end = to.parent_offset() == to.parent().content_size();
        let mut tr = state.tr();
        if selection.kind() == Kind::Text && !selection.is_empty() {
            tr.delete_selection().ok()?;
        }

        // What a new block after this one should be: whatever the parent's
        // content expression offers next. Enter at the end of a heading gives
        // a paragraph, because that is the first textblock `block+` allows.
        let default = (from.depth() > 0)
            .then(|| {
                from.node(from.depth() - 1)
                    .content_match_at(from.index_after(from.depth() - 1))
            })
            .flatten()
            .and_then(|m| default_block_at(state, &m));

        let named = default.map(|typ| [Some(TypeAndAttrs::new(typ))]);
        let types: Option<&[Option<TypeAndAttrs>]> = if at_end {
            named.as_ref().map(|t| &t[..])
        } else {
            None
        };

        let pos = tr.mapping().map(from.pos(), 1);
        let mut can = can_split(state.schema(), tr.doc(), pos, 1, types);
        let mut types = types;
        if types.is_none() && !can {
            let fallback: Option<&[Option<TypeAndAttrs>]> = named.as_ref().map(|t| &t[..]);
            if can_split(state.schema(), tr.doc(), pos, 1, fallback) {
                types = fallback;
                can = true;
            }
        }
        if !can {
            return None;
        }
        tr.split(pos, 1, types).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Enter in an empty block that is wrapped in something: lift it out. What
/// makes Enter on an empty list item leave the list.
#[must_use]
pub fn lift_empty_block() -> Command {
    command(|state| {
        let at = cursor(state)?;
        if at.parent().content_size() != 0 {
            return None;
        }
        if at.depth() > 1 && at.after(at.depth()) != at.end(at.depth() - 1) {
            let before = at.before(at.depth());
            if can_split(state.schema(), state.doc(), before, 1, None) {
                let mut tr = state.tr();
                tr.split(before, 1, None).ok()?;
                return Some(tr.clone().scroll_into_view());
            }
        }
        let range = at.block_range(&at, None)?;
        let target = lift_target(&range)?;
        let mut tr = state.tr();
        tr.lift(&range, target).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Puts a new empty paragraph beside a selected block node.
#[must_use]
pub fn create_paragraph_near() -> Command {
    command(|state| {
        let selection = state.selection();
        let doc = state.doc();
        let from = doc.resolve(selection.from());
        if from.parent().is_textblock() || selection.kind() == Kind::Text {
            return None;
        }
        let matched = from.node(from.depth()).content_match_at(from.index(from.depth()))?;
        let typ = default_block_at(state, &matched)?;
        let node = state
            .schema()
            .create_and_fill(typ, None, Fragment::empty(), Marks::none())?;
        let side = if from.parent_offset() == 0 {
            selection.from()
        } else {
            selection.to()
        };
        let mut tr = state.tr();
        tr.insert(side, node).ok()?;
        let doc = tr.doc().clone();
        tr.set_selection(Selection::near(&doc, &doc.resolve(side + 1), 1));
        Some(tr.clone().scroll_into_view())
    })
}

/// Newline inside a code block, where Enter must not split.
#[must_use]
pub fn new_line_in_code() -> Command {
    command(|state| {
        let at = cursor(state)?;
        if !at.parent().typ().spec().code {
            return None;
        }
        let mut tr = state.tr();
        tr.insert_text("\n").ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Leaves a code block from its end, creating a block after it.
#[must_use]
pub fn exit_code() -> Command {
    command(|state| {
        let at = cursor(state)?;
        if !at.parent().typ().spec().code || at.parent_offset() != at.parent().content_size() {
            return None;
        }
        let depth = at.depth().checked_sub(1)?;
        let matched = at.node(depth).content_match_at(at.index_after(depth))?;
        let typ = default_block_at(state, &matched)?;
        let node = state
            .schema()
            .create_and_fill(typ, None, Fragment::empty(), Marks::none())?;
        let after = at.after(at.depth());
        let mut tr = state.tr();
        tr.replace(after, after, Slice::new(Fragment::from(node), 0, 0))
            .ok()?;
        let doc = tr.doc().clone();
        tr.set_selection(Selection::near(&doc, &doc.resolve(after), 1));
        Some(tr.clone().scroll_into_view())
    })
}

// ---------------------------------------------------------------------------
// Selecting
// ---------------------------------------------------------------------------

/// Selects the whole document.
#[must_use]
pub fn select_all() -> Command {
    command(|state| {
        let mut tr = state.tr();
        tr.set_selection(Selection::all(state.doc()));
        Some(tr.clone())
    })
}

/// Widens the selection to the node containing it — press it repeatedly to
/// walk up the tree.
#[must_use]
pub fn select_parent_node() -> Command {
    command(|state| {
        let selection = state.selection();
        let doc = state.doc();
        let from = doc.resolve(selection.from());
        let same = from.shared_depth(selection.to());
        if same == 0 {
            return None;
        }
        let pos = from.before(same);
        let mut tr = state.tr();
        tr.set_selection(Selection::node(doc, pos)?);
        Some(tr.clone().scroll_into_view())
    })
}

// ---------------------------------------------------------------------------
// Marks and block types
// ---------------------------------------------------------------------------

/// Turns a mark on where it is off and off where it is on.
///
/// On a collapsed selection it changes the *pending* marks rather than the
/// document, so pressing Ctrl+B and then typing produces bold text — the
/// behaviour every editor has and none of them get by editing the document.
#[must_use]
pub fn toggle_mark(mark: MarkTypeId, attrs: Option<Attrs>) -> Command {
    command(move |state| {
        let selection = state.selection();
        let doc = state.doc();
        let mark_type = state.schema().mark_type(mark);

        if let Some(pos) = selection.cursor_pos() {
            let current = state
                .stored_marks()
                .cloned()
                .unwrap_or_else(|| doc.resolve(pos).marks());
            let mut tr = state.tr();
            if current.iter().any(|m| m.typ().id() == mark) {
                let existing = current
                    .iter()
                    .find(|m| m.typ().id() == mark)
                    .expect("just checked")
                    .clone();
                tr.remove_stored_mark(&existing);
            } else {
                let made = state.schema().mark_by_id(mark, attrs.as_ref()).ok()?;
                tr.add_stored_mark(&made);
            }
            return Some(tr.clone());
        }

        let (from, to) = (selection.from(), selection.to());
        if from == to {
            return None;
        }
        // Refuse where no node in the range could carry the mark, rather than
        // producing an empty transaction that looks like it worked.
        let mut applies = false;
        doc.nodes_between(from, to, &mut |node, _, parent, _| {
            if node.is_inline()
                && parent.is_some_and(|p| p.typ().allows_mark_type(mark_type.id()))
            {
                applies = true;
            }
            true
        });
        if !applies {
            return None;
        }

        let mut tr = state.tr();
        if doc.range_has_mark(from, to, mark) {
            tr.remove_mark(from, to, MarkFilter::OfType(mark)).ok()?;
        } else {
            let made = state.schema().mark_by_id(mark, attrs.as_ref()).ok()?;
            tr.add_mark(from, to, &made).ok()?;
        }
        Some(tr.clone().scroll_into_view())
    })
}

/// Retypes the textblocks the selection touches.
#[must_use]
pub fn set_block_type(typ: NodeTypeId, attrs: Option<Attrs>) -> Command {
    command(move |state| {
        let selection = state.selection();
        let (from, to) = (selection.from(), selection.to());
        // Nothing to do if every touched block is already this.
        let mut needed = false;
        state
            .doc()
            .nodes_between(from, to, &mut |node, _, _, _| {
                if node.is_textblock() && node.type_id() != typ {
                    needed = true;
                }
                true
            });
        if !needed {
            return None;
        }
        let mut tr = state.tr();
        tr.set_block_type(from, to, typ, attrs.as_ref()).ok()?;
        tr.doc_changed().then(|| tr.clone().scroll_into_view())
    })
}

/// Wraps the selection in a node of `typ`.
#[must_use]
pub fn wrap_in(typ: NodeTypeId, attrs: Option<Attrs>) -> Command {
    command(move |state| {
        let doc = state.doc();
        let from = doc.resolve(state.selection().from());
        let to = doc.resolve(state.selection().to());
        let range = from.block_range(&to, None)?;
        let wrapping = find_wrapping(state.schema(), &range, typ, attrs.clone(), None)?;
        let mut tr = state.tr();
        tr.wrap(&range, &wrapping).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Lifts the selection out of its parent — the general outdent.
#[must_use]
pub fn lift() -> Command {
    command(|state| {
        let doc = state.doc();
        let from = doc.resolve(state.selection().from());
        let to = doc.resolve(state.selection().to());
        let range = from.block_range(&to, None)?;
        let target = lift_target(&range)?;
        let mut tr = state.tr();
        tr.lift(&range, target).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// Enter inside a list item: a new item, or — in an empty one — an exit.
#[must_use]
pub fn split_list_item(item_type: NodeTypeId) -> Command {
    command(move |state| {
        let selection = state.selection();
        if selection.kind() == Kind::Node {
            return None;
        }
        let doc = state.doc();
        let from = doc.resolve(selection.from());
        let to = doc.resolve(selection.to());
        if from.depth() < 2 || !from.same_parent(&to) {
            return None;
        }
        let grandparent = from.node(from.depth() - 1);
        if grandparent.type_id() != item_type {
            return None;
        }
        // An empty item at the end of its list: let the next command lift it
        // out instead of adding another empty one.
        if from.parent().content_size() == 0
            && grandparent.child_count() == from.index_after(from.depth() - 1)
        {
            return None;
        }

        let next_type = (to.pos() == from.end(from.depth()))
            .then(|| grandparent.content_match_at(0).and_then(|m| m.default_type(state.schema())))
            .flatten();
        let mut tr = state.tr();
        tr.delete(from.pos(), to.pos()).ok()?;
        let types = next_type.map(|t| [None, Some(TypeAndAttrs::new(t))]);
        let types_ref: Option<&[Option<TypeAndAttrs>]> = types.as_ref().map(|t| &t[..]);
        if !can_split(state.schema(), tr.doc(), from.pos(), 2, types_ref) {
            return None;
        }
        tr.split(from.pos(), 2, types_ref).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Outdents a list item.
#[must_use]
pub fn lift_list_item(item_type: NodeTypeId) -> Command {
    command(move |state| {
        let doc = state.doc();
        let from = doc.resolve(state.selection().from());
        let to = doc.resolve(state.selection().to());
        let is_item = |node: &Node| {
            node.child_count() > 0
                && node.first_child().is_some_and(|c| c.type_id() == item_type)
        };
        let range = from.block_range(&to, Some(&is_item))?;
        let target = lift_target(&range)?;
        let mut tr = state.tr();
        tr.lift(&range, target).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Indents a list item, nesting it under the one before.
#[must_use]
pub fn sink_list_item(item_type: NodeTypeId) -> Command {
    command(move |state| {
        let doc = state.doc();
        let from = doc.resolve(state.selection().from());
        let to = doc.resolve(state.selection().to());
        let is_item = |node: &Node| {
            node.child_count() > 0
                && node.first_child().is_some_and(|c| c.type_id() == item_type)
        };
        let range = from.block_range(&to, Some(&is_item))?;
        let start_index = range.start_index();
        if start_index == 0 {
            return None;
        }
        let parent = range.parent();
        let node_before = parent.child(start_index - 1)?;
        if node_before.type_id() != item_type {
            return None;
        }

        // When the item before already ends in a nested list, the new item
        // joins that list rather than starting a second one.
        let nested_before = node_before
            .last_child()
            .is_some_and(|c| c.type_id() == parent.type_id());
        let inner = if nested_before {
            Fragment::from(
                state
                    .schema()
                    .create_and_fill(item_type, None, Fragment::empty(), Marks::none())?,
            )
        } else {
            Fragment::empty()
        };
        let list = state
            .schema()
            .create(parent.type_id(), None, inner, Marks::none())
            .ok()?;
        let item = state
            .schema()
            .create(item_type, None, Fragment::from(list), Marks::none())
            .ok()?;
        let slice = Slice::new(
            Fragment::from(item),
            if nested_before { 3 } else { 1 },
            0,
        );
        let (before, after) = (range.start(), range.end());
        let mut tr = state.tr();
        tr.step(Step::ReplaceAround {
            from: before - if nested_before { 3 } else { 1 },
            to: after,
            gap_from: before,
            gap_to: after,
            slice,
            insert: 1,
            structure: true,
        })
        .ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

/// Turns the selected blocks into a list.
#[must_use]
pub fn wrap_in_list(list_type: NodeTypeId, attrs: Option<Attrs>) -> Command {
    command(move |state| {
        let doc = state.doc();
        let from = doc.resolve(state.selection().from());
        let to = doc.resolve(state.selection().to());
        let mut range = from.block_range(&to, None)?;
        let mut outer = range.clone();

        // Already at the top of a list item: wrap from outside the item, so
        // the new list nests rather than replacing.
        if range.depth() >= 2
            && from
                .node(range.depth() - 1)
                .typ()
                .is_compatible_content(state.schema().node_type(list_type))
            && range.start_index() == 0
        {
            if from.index(range.depth() - 1) == 0 {
                return None;
            }
            let insert = doc.resolve(range.start() - 2);
            outer = NodeRange::new(insert.clone(), insert, range.depth());
            if range.end_index() < range.parent().child_count() {
                range = NodeRange::new(
                    from.clone(),
                    doc.resolve(to.end(range.depth())),
                    range.depth(),
                );
            }
        }

        let wrapping = find_wrapping(state.schema(), &outer, list_type, attrs.clone(), Some(&range))?;
        let mut tr = state.tr();
        tr.wrap(&range, &wrapping).ok()?;
        Some(tr.clone().scroll_into_view())
    })
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// Undo.
#[must_use]
pub fn undo() -> Command {
    command(crate::history::undo)
}

/// Redo.
#[must_use]
pub fn redo() -> Command {
    command(crate::history::redo)
}
