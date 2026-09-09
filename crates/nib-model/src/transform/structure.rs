// SPDX-License-Identifier: MPL-2.0

//! Structural edits: splitting, joining, lifting, wrapping, retyping.
//!
//! These are the operations a user thinks in — press Enter, press Backspace at
//! the start of a list item, indent, make this a heading — expressed as steps.
//! Each comes in two halves, and the split is deliberate:
//!
//! - a **question** ([`can_split`], [`can_join`], [`lift_target`],
//!   [`find_wrapping`]) that consults the schema and returns whether, and at
//!   what depth, the edit is possible;
//! - a **method on [`Transform`]** that performs it.
//!
//! Commands ask the question first, which is how a keymap decides whether Enter
//! splits a paragraph or exits a code block without either one half-happening.
//! Every performing method emits *structural* steps, so a change that would
//! have to destroy content it did not expect refuses instead.

use crate::attrs::Attrs;
use crate::fragment::Fragment;
use crate::mark::{Mark, Marks};
use crate::node::Node;
use crate::resolve::NodeRange;
use crate::schema::{NodeTypeId, Schema};
use crate::slice::Slice;
use crate::transform::step::{Step, StepError};
use crate::transform::Transform;

/// A node type and the attributes to create it with — what a wrapping or a
/// split names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAndAttrs {
    pub typ: NodeTypeId,
    pub attrs: Option<Attrs>,
}

impl TypeAndAttrs {
    #[must_use]
    pub fn new(typ: NodeTypeId) -> Self {
        Self { typ, attrs: None }
    }

    #[must_use]
    pub fn with_attrs(typ: NodeTypeId, attrs: Attrs) -> Self {
        Self {
            typ,
            attrs: Some(attrs),
        }
    }
}

/// True when the node at `pos` can be split `depth` levels deep.
///
/// `types_after` optionally names what the node *after* each split should
/// become — how "Enter at the end of a heading starts a paragraph" is
/// expressed, rather than starting a second heading.
#[must_use]
pub fn can_split(
    schema: &Schema,
    doc: &Node,
    pos: usize,
    depth: usize,
    types_after: Option<&[Option<TypeAndAttrs>]>,
) -> bool {
    let at = doc.resolve(pos);
    let Some(base) = at.depth().checked_sub(depth) else {
        return false;
    };
    let parent = at.parent();

    // The innermost node after the split: whatever was named, or the parent
    // continuing as itself.
    let tail = parent
        .content()
        .cut_by_index(at.index(at.depth()), parent.child_count());
    let inner_valid = match types_after.and_then(|t| t.last().and_then(Option::as_ref)) {
        Some(named) => schema.node_type(named.typ).valid_content(&tail),
        None => parent.typ().valid_content(&tail),
    };

    if parent.typ().spec().isolating
        || !parent.can_replace(
            at.index(at.depth()),
            parent.child_count(),
            &Fragment::empty(),
        )
        || !inner_valid
    {
        return false;
    }

    let mut d = at.depth().wrapping_sub(1);
    let mut i = depth as isize - 2;
    while d > base && d != usize::MAX {
        let node = at.node(d);
        let index = at.index(d);
        if node.typ().spec().isolating {
            return false;
        }
        let mut rest = node.content().cut_by_index(index, node.child_count());
        if let Some(override_child) = types_after
            .and_then(|t| usize::try_from(i + 1).ok().and_then(|k| t.get(k)))
            .and_then(Option::as_ref)
            && let Some(first) = rest.child(0)
        {
            let replacement = replacement_node(schema, override_child, first);
            rest = rest.replace_child(0, replacement);
        }
        let after_ok = match types_after
            .and_then(|t| usize::try_from(i).ok().and_then(|k| t.get(k)))
            .and_then(Option::as_ref)
        {
            Some(named) => schema.node_type(named.typ).valid_content(&rest),
            None => node.typ().valid_content(&rest),
        };
        if !node.can_replace(index + 1, node.child_count(), &Fragment::empty()) || !after_ok {
            return false;
        }
        d = d.wrapping_sub(1);
        i -= 1;
    }

    let index = at.index_after(base);
    let base_type = types_after
        .and_then(|t| t.first().and_then(Option::as_ref))
        .map_or_else(|| at.node(base + 1).type_id(), |t| t.typ);
    at.node(base).can_replace_with(index, index, base_type, None)
}

/// True when the two nodes on either side of `pos` can be joined into one.
#[must_use]
pub fn can_join(doc: &Node, pos: usize) -> bool {
    let at = doc.resolve(pos);
    let index = at.index(at.depth());
    let (Some(before), Some(after)) = (at.node_before(), at.node_after()) else {
        return false;
    };
    !before.is_leaf()
        && before.can_append(&after)
        && at
            .parent()
            .can_replace(index, index + 1, &Fragment::empty())
}

/// The nearest position at or after `pos` (searching in `dir`) where a join is
/// possible.
#[must_use]
pub fn join_point(doc: &Node, pos: usize, dir: i32) -> Option<usize> {
    let mut at = doc.resolve(pos);
    let mut pos = pos;
    loop {
        if can_join(doc, pos) {
            return Some(pos);
        }
        if at.depth() == 0 {
            return None;
        }
        let index = at.index(at.depth());
        let boundary = if dir > 0 {
            index == at.parent().child_count()
        } else {
            index == 0
        };
        if !boundary {
            return None;
        }
        pos = if dir > 0 {
            at.after(at.depth())
        } else {
            at.before(at.depth())
        };
        at = doc.resolve(pos);
    }
}

/// The depth a range can be lifted to, or `None` when it cannot be lifted.
///
/// "Lifting" is pulling content out of the node that wraps it: a list item out
/// of its list, a paragraph out of a blockquote. The answer is a depth rather
/// than a boolean because outdenting a nested list moves it one level, not all
/// the way out.
#[must_use]
pub fn lift_target(range: &NodeRange) -> Option<usize> {
    let parent = range.parent();
    let content = parent
        .content()
        .cut_by_index(range.start_index(), range.end_index());
    let mut depth = range.depth();
    loop {
        let node = range.from().node(depth);
        let index = range.from().index(depth);
        let end_index = range.to().index_after(depth);
        if depth < range.depth() && node.can_replace(index, end_index, &content) {
            return Some(depth);
        }
        if depth == 0 || node.typ().spec().isolating || !can_cut(node, index, end_index) {
            return None;
        }
        depth -= 1;
    }
}

fn can_cut(node: &Node, start: usize, end: usize) -> bool {
    (start == 0 || node.can_replace(start, node.child_count(), &Fragment::empty()))
        && (end == node.child_count() || node.can_replace(0, end, &Fragment::empty()))
}

/// The chain of node types a range must be wrapped in to become a `node_type`,
/// or `None` when it cannot.
///
/// The chain has three parts: whatever must go *outside* the new node for it to
/// be legal where the range is, the new node itself, and whatever must go
/// *inside* it for the range to be legal in it. Wrapping paragraphs in a
/// bullet list needs the third part — each paragraph must become a list item.
#[must_use]
pub fn find_wrapping(
    schema: &Schema,
    range: &NodeRange,
    node_type: NodeTypeId,
    attrs: Option<Attrs>,
    inner_range: Option<&NodeRange>,
) -> Option<Vec<TypeAndAttrs>> {
    let inner_range = inner_range.unwrap_or(range);
    let around = find_wrapping_outside(schema, range, node_type)?;
    let inside = find_wrapping_inside(schema, inner_range, node_type)?;
    let mut wrapping: Vec<TypeAndAttrs> = around.into_iter().map(TypeAndAttrs::new).collect();
    wrapping.push(TypeAndAttrs { typ: node_type, attrs });
    wrapping.extend(inside.into_iter().map(TypeAndAttrs::new));
    Some(wrapping)
}

fn find_wrapping_outside(
    schema: &Schema,
    range: &NodeRange,
    typ: NodeTypeId,
) -> Option<Vec<NodeTypeId>> {
    let parent = range.parent();
    let around = parent
        .content_match_at(range.start_index())?
        .find_wrapping(schema, typ)?;
    let outer = around.first().copied().unwrap_or(typ);
    parent
        .can_replace_with(range.start_index(), range.end_index(), outer, None)
        .then_some(around)
}

fn find_wrapping_inside(
    schema: &Schema,
    range: &NodeRange,
    typ: NodeTypeId,
) -> Option<Vec<NodeTypeId>> {
    let parent = range.parent();
    let inner = parent.child(range.start_index())?;
    let inside = schema
        .node_type(typ)
        .content_match()
        .find_wrapping(schema, inner.type_id())?;
    let last = inside.last().copied().unwrap_or(typ);
    let mut inner_match = schema.node_type(last).content_match();
    for i in range.start_index()..range.end_index() {
        inner_match = inner_match.match_type(parent.child(i)?.type_id())?;
    }
    inner_match.valid_end().then_some(inside)
}

/// The nearest position at or after `pos` where a node of `typ` may be
/// inserted, or `None` when there is none in the surrounding chain.
#[must_use]
pub fn insert_point(doc: &Node, pos: usize, typ: NodeTypeId) -> Option<usize> {
    let at = doc.resolve(pos);
    if at.parent().can_replace_with(
        at.index(at.depth()),
        at.index(at.depth()),
        typ,
        None,
    ) {
        return Some(pos);
    }

    // Try after the ancestors that end here, then before the ones that start
    // here — the two ways out of a node whose content will not take this.
    if at.parent_offset() == 0 {
        for d in (0..at.depth()).rev() {
            let index = at.index(d);
            if at.node(d).can_replace_with(index, index, typ, None) {
                return Some(at.before(d + 1));
            }
            if index > 0 {
                return None;
            }
        }
    }
    if at.parent_offset() == at.parent().content_size() {
        for d in (0..at.depth()).rev() {
            let index = at.index_after(d);
            if at.node(d).can_replace_with(index, index, typ, None) {
                return Some(at.after(d + 1));
            }
            if index < at.node(d).child_count() {
                return None;
            }
        }
    }
    None
}

fn replacement_node(schema: &Schema, spec: &TypeAndAttrs, like: &Node) -> Node {
    schema
        .create(
            spec.typ,
            spec.attrs.as_ref(),
            Fragment::empty(),
            like.marks().clone(),
        )
        .unwrap_or_else(|_| like.clone())
}

// ---------------------------------------------------------------------------
// The performing half
// ---------------------------------------------------------------------------

impl Transform {
    /// Splits the node at `pos`, `depth` levels deep.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the split would produce invalid content — ask
    /// [`can_split`] first.
    pub fn split(
        &mut self,
        pos: usize,
        depth: usize,
        types_after: Option<&[Option<TypeAndAttrs>]>,
    ) -> Result<&mut Self, StepError> {
        let at = self.doc().resolve(pos);
        let mut before = Fragment::empty();
        let mut after = Fragment::empty();
        let mut d = at.depth();
        let mut i = depth as isize - 1;
        let end = at.depth().saturating_sub(depth);
        while d > end {
            before = Fragment::from(at.node(d).copy(before));
            let named = types_after
                .and_then(|t| usize::try_from(i).ok().and_then(|k| t.get(k)))
                .and_then(Option::as_ref);
            after = Fragment::from(match named {
                Some(spec) => self
                    .schema
                    .create(spec.typ, spec.attrs.as_ref(), after, Marks::none())
                    .map_err(|_| StepError::NoNodeAt(pos))?,
                None => at.node(d).copy(after),
            });
            d -= 1;
            i -= 1;
        }
        let slice = Slice::new(before.append(&after), depth, depth);
        self.step(Step::replace_structure(pos, pos, slice))
    }

    /// Joins the nodes on either side of `pos`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when they cannot be joined — ask [`can_join`] first.
    pub fn join(&mut self, pos: usize, depth: usize) -> Result<&mut Self, StepError> {
        self.step(Step::replace_structure(
            pos - depth,
            pos + depth,
            Slice::empty(),
        ))
    }

    /// Lifts a range out to `target` depth.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the lift does not apply — ask [`lift_target`] first.
    pub fn lift(&mut self, range: &NodeRange, target: usize) -> Result<&mut Self, StepError> {
        let (from, to, depth) = (range.from(), range.to(), range.depth());
        let gap_start = from.before(depth + 1);
        let gap_end = to.after(depth + 1);
        let mut start = gap_start;
        let mut end = gap_end;

        // Whatever the range does not start at the beginning of has to stay
        // behind, so it is kept in the slice and the replaced range grows to
        // include it.
        let mut before = Fragment::empty();
        let mut open_start = 0;
        let mut splitting = false;
        for d in ((target + 1)..=depth).rev() {
            if splitting || from.index(d) > 0 {
                splitting = true;
                before = Fragment::from(from.node(d).copy(before));
                open_start += 1;
            } else {
                start -= 1;
            }
        }
        let mut after = Fragment::empty();
        let mut open_end = 0;
        let mut splitting = false;
        for d in ((target + 1)..=depth).rev() {
            if splitting || to.after(d + 1) < to.end(d) {
                splitting = true;
                after = Fragment::from(to.node(d).copy(after));
                open_end += 1;
            } else {
                end += 1;
            }
        }

        let content = before.append(&after);
        let insert = before.size() - open_start;
        self.step(Step::ReplaceAround {
            from: start,
            to: end,
            gap_from: gap_start,
            gap_to: gap_end,
            slice: Slice::new(content, open_start, open_end),
            insert,
            structure: true,
        })
    }

    /// Wraps a range in the given chain of node types, outermost first.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the wrapping does not apply — ask [`find_wrapping`]
    /// first, which produces a chain that does.
    pub fn wrap(
        &mut self,
        range: &NodeRange,
        wrappers: &[TypeAndAttrs],
    ) -> Result<&mut Self, StepError> {
        let mut content = Fragment::empty();
        for spec in wrappers.iter().rev() {
            let node = self
                .schema
                .create(spec.typ, spec.attrs.as_ref(), content, Marks::none())
                .map_err(|_| StepError::NoNodeAt(range.start()))?;
            content = Fragment::from(node);
        }
        let (start, end) = (range.start(), range.end());
        self.step(Step::ReplaceAround {
            from: start,
            to: end,
            gap_from: start,
            gap_to: end,
            slice: Slice::new(content, 0, 0),
            insert: wrappers.len(),
            structure: true,
        })
    }

    /// Changes the type of every textblock touching `from..to`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a change does not apply. Blocks that cannot take the
    /// new type are skipped rather than failing the whole call — turning a
    /// mixed selection into headings should convert what it can.
    pub fn set_block_type(
        &mut self,
        from: usize,
        to: usize,
        typ: NodeTypeId,
        attrs: Option<&Attrs>,
    ) -> Result<&mut Self, StepError> {
        if !self.schema.node_type(typ).is_textblock() {
            return Err(StepError::NoNodeAt(from));
        }
        let map_from = self.steps().len();
        // Collect first: the document is rewritten as we go, so walking it and
        // stepping it at the same time would be walking a stale tree.
        let mut targets: Vec<usize> = Vec::new();
        self.doc().nodes_between(from, to, &mut |node, pos, _, _| {
            if node.is_textblock() && node.type_id() != typ {
                targets.push(pos);
                return false;
            }
            true
        });

        for pos in targets {
            let mapped = self.mapping().slice(map_from, self.mapping().len()).map(pos, 1);
            let doc = self.doc().clone();
            let Some(node) = doc.node_at(mapped) else {
                continue;
            };
            let at = doc.resolve(mapped);
            let index = at.index(at.depth());
            if !at.parent().can_replace_with(index, index + 1, typ, None) {
                continue;
            }
            self.clear_incompatible(mapped + 1, typ)?;
            let mapping = self.mapping().slice(map_from, self.mapping().len());
            let start = mapping.map(pos, 1);
            let end = mapping.map(pos + node.node_size(), 1);
            let replacement = self
                .schema
                .create(typ, attrs, Fragment::empty(), node.marks().clone())
                .map_err(|_| StepError::NoNodeAt(start))?;
            self.step(Step::ReplaceAround {
                from: start,
                to: end,
                gap_from: start + 1,
                gap_to: end - 1,
                slice: Slice::new(Fragment::from(replacement), 0, 0),
                insert: 1,
                structure: true,
            })?;
        }
        Ok(self)
    }

    /// Changes the type, attributes or marks of the single node at `pos`,
    /// keeping its content.
    ///
    /// # Errors
    ///
    /// [`StepError`] when there is no node there, or its content would not be
    /// valid for the new type.
    pub fn set_node_markup(
        &mut self,
        pos: usize,
        typ: Option<NodeTypeId>,
        attrs: Option<&Attrs>,
        marks: Option<Marks>,
    ) -> Result<&mut Self, StepError> {
        let doc = self.doc().clone();
        let node = doc.node_at(pos).ok_or(StepError::NoNodeAt(pos))?;
        let typ = typ.unwrap_or_else(|| node.type_id());
        let marks = marks.unwrap_or_else(|| node.marks().clone());
        let new_node = self
            .schema
            .create(typ, attrs, Fragment::empty(), marks)
            .map_err(|_| StepError::NoNodeAt(pos))?;

        if node.is_leaf() {
            return self.step(Step::replace(
                pos,
                pos + node.node_size(),
                Slice::new(Fragment::from(new_node), 0, 0),
            ));
        }
        if !self.schema.node_type(typ).valid_content(node.content()) {
            return Err(StepError::NoNodeAt(pos));
        }
        self.step(Step::ReplaceAround {
            from: pos,
            to: pos + node.node_size(),
            gap_from: pos + 1,
            gap_to: pos + node.node_size() - 1,
            slice: Slice::new(Fragment::from(new_node), 0, 0),
            insert: 1,
            structure: true,
        })
    }

    /// Removes whatever the node at `pos - 1` contains that a `parent_type`
    /// would not allow, so its type can be changed.
    ///
    /// The unglamorous half of retyping a block: turning a paragraph with a
    /// link and an image into a code block has to drop both, and doing it as
    /// explicit steps is what makes the change undoable in one go.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a clearing step does not apply.
    pub fn clear_incompatible(
        &mut self,
        pos: usize,
        parent_type: NodeTypeId,
    ) -> Result<&mut Self, StepError> {
        let doc = self.doc().clone();
        let Some(node) = doc.node_at(pos - 1) else {
            return Ok(self);
        };
        let parent = self.schema.node_type(parent_type).clone();
        let mut matched = parent.content_match();
        let mut steps: Vec<Step> = Vec::new();
        let mut cur = pos;

        for child in node.content().iter() {
            let end = cur + child.node_size();
            match matched.match_type(child.type_id()) {
                None => steps.push(Step::replace(cur, end, Slice::empty())),
                Some(next) => {
                    matched = next;
                    for mark in child.marks().iter() {
                        if !parent.allows_mark_type(mark.typ().id()) {
                            steps.push(Step::RemoveMark {
                                from: cur,
                                to: end,
                                mark: mark.clone(),
                            });
                        }
                    }
                }
            }
            cur = end;
        }

        if !matched.valid_end()
            && let Some(fill) = matched.fill_before(&self.schema, &Fragment::empty(), true, 0)
        {
            self.replace(cur, cur, Slice::new(fill, 0, 0))?;
        }
        // Backwards, so earlier positions are still valid as we go.
        for step in steps.into_iter().rev() {
            self.step(step)?;
        }
        Ok(self)
    }

    /// Adds a mark to every inline node in `from..to` whose parent allows it.
    ///
    /// Emits one step per run rather than one per node, and removes marks the
    /// new one excludes — applying `code` over a bold run drops the bold in the
    /// same transaction, so one undo takes both back.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a step does not apply.
    pub fn add_mark(&mut self, from: usize, to: usize, mark: &Mark) -> Result<&mut Self, StepError> {
        let doc = self.doc().clone();
        let mut removed: Vec<Step> = Vec::new();
        let mut added: Vec<Step> = Vec::new();

        doc.nodes_between(from, to, &mut |node, pos, parent, _| {
            if !node.is_inline() {
                return true;
            }
            let allowed = parent.is_some_and(|p| p.typ().allows_mark_type(mark.typ().id()));
            if mark.is_in_set(node.marks()) || !allowed {
                return true;
            }
            let start = pos.max(from);
            let end = (pos + node.node_size()).min(to);
            let new_set = mark.add_to_set(node.marks());

            for existing in node.marks().iter() {
                if existing.is_in_set(&new_set) {
                    continue;
                }
                match removed.last_mut() {
                    Some(Step::RemoveMark {
                        to: last_to, mark: m, ..
                    }) if *last_to == start && m == existing => *last_to = end,
                    _ => removed.push(Step::RemoveMark {
                        from: start,
                        to: end,
                        mark: existing.clone(),
                    }),
                }
            }
            match added.last_mut() {
                Some(Step::AddMark { to: last_to, .. }) if *last_to == start => *last_to = end,
                _ => added.push(Step::AddMark {
                    from: start,
                    to: end,
                    mark: mark.clone(),
                }),
            }
            true
        });

        for step in removed.into_iter().chain(added) {
            self.step(step)?;
        }
        Ok(self)
    }

    /// Removes a mark — or every mark of a type, or every mark — from
    /// `from..to`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a step does not apply.
    pub fn remove_mark(
        &mut self,
        from: usize,
        to: usize,
        which: MarkFilter<'_>,
    ) -> Result<&mut Self, StepError> {
        let doc = self.doc().clone();
        let mut matched: Vec<(Mark, usize, usize)> = Vec::new();

        doc.nodes_between(from, to, &mut |node, pos, _, _| {
            if !node.is_inline() {
                return true;
            }
            let start = pos.max(from);
            let end = (pos + node.node_size()).min(to);
            for mark in node.marks().iter() {
                if !which.matches(mark) {
                    continue;
                }
                // Extend the run when this is the same mark continuing.
                if let Some(found) = matched
                    .iter_mut()
                    .find(|(m, _, run_end)| m == mark && *run_end == start)
                {
                    found.2 = end;
                } else {
                    matched.push((mark.clone(), start, end));
                }
            }
            true
        });

        for (mark, start, end) in matched {
            self.step(Step::RemoveMark {
                from: start,
                to: end,
                mark,
            })?;
        }
        Ok(self)
    }
}

/// Which marks [`Transform::remove_mark`] should take off.
#[derive(Debug, Clone, Copy)]
pub enum MarkFilter<'a> {
    /// One exact mark, attributes included.
    Exact(&'a Mark),
    /// Every mark of a type, whatever its attributes — "remove the link",
    /// without having to know which link.
    OfType(crate::schema::MarkTypeId),
    /// Everything.
    All,
}

impl MarkFilter<'_> {
    #[must_use]
    fn matches(self, mark: &Mark) -> bool {
        match self {
            Self::Exact(m) => m == mark,
            Self::OfType(t) => mark.typ().id() == t,
            Self::All => true,
        }
    }
}
