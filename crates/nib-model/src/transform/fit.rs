// SPDX-License-Identifier: MPL-2.0

//! Fitting a slice into a gap it does not exactly match.
//!
//! [`Node::replace`](crate::node::Node::replace) is exact: hand it a slice
//! whose open depths do not line up with the range, and it refuses. That is
//! the right behaviour for a step, which must be deterministic and invertible,
//! and the wrong behaviour for a *paste*, where the user copied two list items
//! out of a list and dropped them into the middle of a paragraph and expects
//! something reasonable to happen.
//!
//! [`replace_step`] is what stands between the two. It searches for a way to
//! place the slice's content into the gap — opening the slice further, dropping
//! nodes off its edges, wrapping what is left in whatever the schema requires —
//! and returns the exact step that does it, or `None` when nothing would.
//!
//! # How the search works
//!
//! Three things are tracked as content is placed:
//!
//! - the **frontier**: the stack of still-open nodes on the left of the gap,
//!   starting as the chain of ancestors above `from` and each remembering how
//!   far through its content expression it has got;
//! - the **unplaced** slice: what has not been consumed yet;
//! - the **placed** fragment: what has.
//!
//! Each round looks for a *fittable* pairing — some depth in the unplaced
//! slice whose first node the frontier will accept at some depth, either
//! directly, or after inserting what the content expression requires in
//! between, or wrapped in nodes the schema can supply. Failing that, the slice
//! is opened one level deeper (so its children are considered independently)
//! or a node is dropped from it. The loop always terminates, because both
//! fallbacks strictly shrink what is left to place.
//!
//! When the slice is exhausted, the frontier is reconciled with the right side
//! of the gap, and the whole thing becomes one `Replace` — or, when inline
//! content on both sides has to be pulled together, one `ReplaceAround`.
//!
//! This is a port of ProseMirror's `Fitter`. It is the single most intricate
//! piece of the engine, and it is intricate because "paste this here" is a
//! genuinely underdetermined request that a schema is the only thing able to
//! answer.

use crate::attrs::Attrs;
use crate::content::ContentMatch;
use crate::fragment::Fragment;
use crate::mark::Marks;
use crate::node::Node;
use crate::resolve::ResolvedPos;
use crate::schema::{NodeTypeId, Schema};
use crate::slice::Slice;
use crate::transform::step::Step;

/// The step that replaces `from..to` with `slice`, fitting the slice to the
/// gap, or `None` when nothing can be made to fit.
#[must_use]
pub fn replace_step(schema: &Schema, doc: &Node, from: usize, to: usize, slice: &Slice) -> Option<Step> {
    if from == to && slice.is_empty() {
        return None;
    }
    let r_from = doc.resolve(from);
    let r_to = doc.resolve(to);
    if fits_trivially(&r_from, &r_to, slice) {
        return Some(Step::replace(from, to, slice.clone()));
    }
    Fitter::new(schema, r_from, r_to, slice.clone()).fit()
}

/// True when the slice can go straight in with no rearrangement.
fn fits_trivially(from: &ResolvedPos, to: &ResolvedPos, slice: &Slice) -> bool {
    slice.open_start() == 0
        && slice.open_end() == 0
        && from.start(from.depth()) == to.start(to.depth())
        && from.parent().can_replace(
            from.index(from.depth()),
            to.index(to.depth()),
            slice.content(),
        )
}

/// A place the slice's content can go: a depth in the slice, a depth on the
/// frontier, and what has to happen in between.
struct Fittable {
    slice_depth: usize,
    frontier_depth: usize,
    /// The node whose content is being placed, when placing below the slice's
    /// top level.
    parent: Option<Node>,
    /// Content the frontier's expression requires before what is being placed.
    inject: Option<Fragment>,
    /// Node types to open around the content so it fits.
    wrap: Option<Vec<NodeTypeId>>,
}

struct Fitter<'a> {
    schema: &'a Schema,
    from: ResolvedPos,
    to: ResolvedPos,
    unplaced: Slice,
    /// The open left side of the replacement: one entry per depth, each
    /// remembering how far through its content expression it has got.
    frontier: Vec<(NodeTypeId, ContentMatch)>,
    placed: Fragment,
}

impl<'a> Fitter<'a> {
    fn new(schema: &'a Schema, from: ResolvedPos, to: ResolvedPos, unplaced: Slice) -> Self {
        let mut frontier = Vec::with_capacity(from.depth() + 1);
        for i in 0..=from.depth() {
            let node = from.node(i);
            let matched = node
                .content_match_at(from.index_after(i))
                .unwrap_or_else(|| node.typ().content_match());
            frontier.push((node.type_id(), matched));
        }
        let mut placed = Fragment::empty();
        for d in (1..=from.depth()).rev() {
            placed = Fragment::from(from.node(d).copy(placed));
        }
        Self {
            schema,
            from,
            to,
            unplaced,
            frontier,
            placed,
        }
    }

    fn depth(&self) -> usize {
        self.frontier.len() - 1
    }

    fn fit(mut self) -> Option<Step> {
        while !self.unplaced.content().is_empty() {
            if let Some(fit) = self.find_fittable() {
                self.place_nodes(&fit);
            } else if !self.open_more() {
                self.drop_node();
            }
        }

        let move_inline = self.must_move_inline();
        let placed_size = self
            .placed
            .size()
            .saturating_sub(self.depth())
            .saturating_sub(self.from.depth());
        let target = match move_inline {
            Some(pos) => self.from.doc().resolve(pos),
            None => self.to.clone(),
        };
        let to = self.close(&target)?;

        let mut content = self.placed.clone();
        let mut open_start = self.from.depth();
        let mut open_end = to.depth();
        // A single wrapper open on both sides carries no information: drop it
        // and let the join take care of the level.
        while open_start > 0 && open_end > 0 && content.child_count() == 1 {
            let Some(only) = content.child(0) else { break };
            content = only.content().clone();
            open_start -= 1;
            open_end -= 1;
        }
        let slice = Slice::new(content, open_start, open_end);

        if let Some(pos) = move_inline {
            return Some(Step::ReplaceAround {
                from: self.from.pos(),
                to: pos,
                gap_from: self.to.pos(),
                gap_to: self.to.end(self.to.depth()),
                slice,
                insert: placed_size,
                structure: false,
            });
        }
        if !slice.is_empty() || self.from.pos() != self.to.pos() {
            return Some(Step::replace(self.from.pos(), to.pos(), slice));
        }
        None
    }

    /// Finds a depth in the unplaced slice whose content the frontier will
    /// take, at some frontier depth.
    ///
    /// Two passes: the first looks for a place that needs no wrapping, and
    /// only if none exists does the second consider wrapping the content in
    /// nodes the schema supplies. Wrapping first would turn a paragraph pasted
    /// into a list into a list inside a list item.
    fn find_fittable(&self) -> Option<Fittable> {
        let mut start_depth = self.unplaced.open_start();
        let mut cur = self.unplaced.content().clone();
        let mut open_end = self.unplaced.open_end();
        for d in 0..start_depth {
            let Some(node) = cur.first_child().cloned() else {
                break;
            };
            if cur.child_count() > 1 {
                open_end = 0;
            }
            if node.typ().spec().isolating && open_end <= d {
                start_depth = d;
                break;
            }
            cur = node.content().clone();
        }

        for pass in 1..=2 {
            let top = if pass == 1 {
                start_depth
            } else {
                self.unplaced.open_start()
            };
            for slice_depth in (0..=top).rev() {
                let (fragment, parent) = if slice_depth > 0 {
                    let parent = content_at(self.unplaced.content(), slice_depth - 1)
                        .first_child()
                        .cloned()?;
                    (parent.content().clone(), Some(parent))
                } else {
                    (self.unplaced.content().clone(), None)
                };
                let first = fragment.first_child();

                for frontier_depth in (0..=self.depth()).rev() {
                    let (typ, matched) = &self.frontier[frontier_depth];
                    let typ = self.schema.node_type(*typ);

                    if pass == 1 {
                        let mut inject = None;
                        let ok = match first {
                            Some(first) => {
                                matched.match_type(first.type_id()).is_some() || {
                                    inject = matched.fill_before(
                                        self.schema,
                                        &Fragment::from(first.clone()),
                                        false,
                                        0,
                                    );
                                    inject.is_some()
                                }
                            }
                            None => parent
                                .as_ref()
                                .is_some_and(|p| typ.is_compatible_content(p.typ())),
                        };
                        if ok {
                            return Some(Fittable {
                                slice_depth,
                                frontier_depth,
                                parent: parent.clone(),
                                inject,
                                wrap: None,
                            });
                        }
                    } else if let Some(first) = first
                        && let Some(wrap) = matched.find_wrapping(self.schema, first.type_id())
                    {
                        return Some(Fittable {
                            slice_depth,
                            frontier_depth,
                            parent: parent.clone(),
                            inject: None,
                            wrap: Some(wrap),
                        });
                    }

                    // If the whole parent node would fit here, do not keep
                    // looking further out — that would place its content
                    // somewhere its parent could have gone whole.
                    if let Some(parent) = &parent
                        && matched.match_type(parent.type_id()).is_some()
                    {
                        break;
                    }
                }
            }
        }
        None
    }

    /// Opens the slice one level deeper, so its children become independently
    /// placeable. Returns false when there is nothing left to open.
    fn open_more(&mut self) -> bool {
        let (content, open_start, open_end) = (
            self.unplaced.content().clone(),
            self.unplaced.open_start(),
            self.unplaced.open_end(),
        );
        let inner = content_at(&content, open_start);
        if inner.is_empty() || inner.first_child().is_some_and(Node::is_leaf) {
            return false;
        }
        let new_open_end = if inner.size() + open_start >= content.size() - open_end {
            open_start + 1
        } else {
            0
        };
        self.unplaced = Slice::new(content, open_start + 1, open_end.max(new_open_end));
        true
    }

    /// Drops the first node of the slice's open spine — the fallback when
    /// nothing about the content can be made to fit.
    fn drop_node(&mut self) {
        let (content, open_start, open_end) = (
            self.unplaced.content().clone(),
            self.unplaced.open_start(),
            self.unplaced.open_end(),
        );
        let inner = content_at(&content, open_start);
        if inner.child_count() <= 1 && open_start > 0 {
            let open_at_end = content.size() - open_start <= open_start + inner.size();
            self.unplaced = Slice::new(
                drop_from_fragment(&content, open_start - 1, 1),
                open_start - 1,
                if open_at_end { open_start - 1 } else { open_end },
            );
        } else {
            self.unplaced = Slice::new(
                drop_from_fragment(&content, open_start, 1),
                open_start,
                open_end,
            );
        }
    }

    /// Moves content from the unplaced slice onto the frontier.
    fn place_nodes(&mut self, fit: &Fittable) {
        while self.depth() > fit.frontier_depth {
            self.close_frontier_node();
        }
        if let Some(wrap) = &fit.wrap {
            for typ in wrap {
                self.open_frontier_node(*typ, None, Fragment::empty());
            }
        }

        let slice = self.unplaced.clone();
        let fragment = fit
            .parent
            .as_ref()
            .map_or_else(|| slice.content().clone(), |p| p.content().clone());
        let open_start = slice.open_start().saturating_sub(fit.slice_depth);

        let mut taken = 0;
        let mut add: Vec<Node> = Vec::new();
        let (frontier_type, mut matched) = {
            let (t, m) = &self.frontier[fit.frontier_depth];
            (*t, m.clone())
        };
        let frontier_type = self.schema.node_type(frontier_type).clone();

        if let Some(inject) = &fit.inject {
            for child in inject.iter() {
                add.push(child.clone());
            }
            if let Some(next) = matched.match_fragment(inject) {
                matched = next;
            }
        }

        // How many nodes at the end of `fragment` stay open. Zero means the
        // parent is open but nothing below it; negative means nothing is.
        let mut open_end_count = (fragment.size() + fit.slice_depth) as isize
            - (slice.content().size() - slice.open_end()) as isize;

        while taken < fragment.child_count() {
            let Some(next) = fragment.child(taken) else {
                break;
            };
            let Some(matches) = matched.match_type(next.type_id()) else {
                break;
            };
            taken += 1;
            // Drop an open node that turned out to be empty — the tail of a
            // paragraph the user's selection only clipped.
            if taken > 1 || open_start == 0 || next.content_size() > 0 {
                matched = matches;
                let marked = next.with_marks(frontier_type.allowed_marks(next.marks()));
                let node = close_node_start(
                    self.schema,
                    &marked,
                    if taken == 1 { open_start } else { 0 },
                    if taken == fragment.child_count() {
                        open_end_count
                    } else {
                        -1
                    },
                );
                add.push(node);
            }
        }

        let to_end = taken == fragment.child_count();
        if !to_end {
            open_end_count = -1;
        }

        self.placed = add_to_fragment(&self.placed, fit.frontier_depth, &Fragment::from_vec(add));
        self.frontier[fit.frontier_depth].1 = matched;

        // The node we took content from is closed and its type matches the
        // frontier's: close that frontier level now rather than leaving an
        // empty open node behind.
        if open_end_count < 0
            && let Some(parent) = &fit.parent
            && parent.type_id() == self.frontier[self.depth()].0
            && self.frontier.len() > 1
        {
            self.close_frontier_node();
        }

        // Anything still open at the end of what we placed becomes new
        // frontier levels.
        let mut cur = fragment.clone();
        for _ in 0..open_end_count.max(0) {
            let Some(node) = cur.last_child().cloned() else {
                break;
            };
            let matched = node
                .content_match_at(node.child_count())
                .unwrap_or_else(|| node.typ().content_match());
            self.frontier.push((node.type_id(), matched));
            cur = node.content().clone();
        }

        self.unplaced = if !to_end {
            Slice::new(
                drop_from_fragment(slice.content(), fit.slice_depth, taken),
                slice.open_start(),
                slice.open_end(),
            )
        } else if fit.slice_depth == 0 {
            Slice::empty()
        } else {
            Slice::new(
                drop_from_fragment(slice.content(), fit.slice_depth - 1, 1),
                fit.slice_depth - 1,
                if open_end_count < 0 {
                    slice.open_end()
                } else {
                    fit.slice_depth - 1
                },
            )
        };
    }

    /// When inline content sits directly after both the frontier and the end
    /// of the gap, the two have to be pulled into one textblock — which is a
    /// gap-replace, not a replace. Returns the position to fit to.
    fn must_move_inline(&self) -> Option<usize> {
        if !self.to.parent().is_textblock() {
            return None;
        }
        let (top_type, top_match) = &self.frontier[self.depth()];
        let top_type = self.schema.node_type(*top_type);
        if !top_type.is_textblock()
            || content_after_fits(self.schema, &self.to, self.to.depth(), top_type, top_match, false)
                .is_none()
        {
            return None;
        }
        if self.to.depth() == self.depth()
            && self
                .find_close_level(&self.to)
                .is_some_and(|l| l.depth == self.depth())
        {
            return None;
        }

        let mut depth = self.to.depth();
        let mut after = self.to.after(depth);
        while depth > 1 {
            depth -= 1;
            if after != self.to.end(depth) {
                break;
            }
            after += 1;
        }
        Some(after)
    }

    fn find_close_level(&self, to: &ResolvedPos) -> Option<CloseLevel> {
        'scan: for i in (0..=self.depth().min(to.depth())).rev() {
            let (typ, matched) = &self.frontier[i];
            let typ = self.schema.node_type(*typ);
            let drop_inner =
                i < to.depth() && to.end(i + 1) == to.pos() + (to.depth() - (i + 1));
            let Some(fit) =
                content_after_fits(self.schema, to, i, typ, matched, drop_inner)
            else {
                continue;
            };
            for d in (0..i).rev() {
                let (typ, matched) = &self.frontier[d];
                let typ = self.schema.node_type(*typ);
                match content_after_fits(self.schema, to, d, typ, matched, true) {
                    Some(f) if f.is_empty() => {}
                    _ => continue 'scan,
                }
            }
            return Some(CloseLevel {
                depth: i,
                fit,
                move_to: if drop_inner {
                    to.doc().resolve(to.after(i + 1))
                } else {
                    to.clone()
                },
            });
        }
        None
    }

    fn close(&mut self, to: &ResolvedPos) -> Option<ResolvedPos> {
        let close = self.find_close_level(to)?;
        while self.depth() > close.depth {
            self.close_frontier_node();
        }
        if !close.fit.is_empty() {
            self.placed = add_to_fragment(&self.placed, close.depth, &close.fit);
        }
        let to = close.move_to;
        for d in (close.depth + 1)..=to.depth() {
            let node = to.node(d);
            let add = node
                .typ()
                .content_match()
                .fill_before(self.schema, node.content(), true, to.index(d))
                .unwrap_or_else(Fragment::empty);
            self.open_frontier_node(node.type_id(), Some(node.attrs().clone()), add);
        }
        Some(to)
    }

    fn open_frontier_node(&mut self, typ: NodeTypeId, attrs: Option<Attrs>, content: Fragment) {
        let depth = self.depth();
        if let Some(next) = self.frontier[depth].1.match_type(typ) {
            self.frontier[depth].1 = next;
        }
        let node = self
            .schema
            .create(typ, attrs.as_ref(), content, Marks::none())
            .expect("the frontier only opens types the content match offered");
        self.placed = add_to_fragment(&self.placed, depth, &Fragment::from(node));
        self.frontier
            .push((typ, self.schema.node_type(typ).content_match()));
    }

    fn close_frontier_node(&mut self) {
        let Some((_, matched)) = self.frontier.pop() else {
            return;
        };
        if let Some(add) = matched.fill_before(self.schema, &Fragment::empty(), true, 0)
            && !add.is_empty()
        {
            self.placed = add_to_fragment(&self.placed, self.frontier.len(), &add);
        }
    }
}

struct CloseLevel {
    depth: usize,
    fit: Fragment,
    move_to: ResolvedPos,
}

/// What must be inserted for the content after `to` at `depth` to follow the
/// frontier legally, or `None` when nothing would.
fn content_after_fits(
    schema: &Schema,
    to: &ResolvedPos,
    depth: usize,
    typ: &std::sync::Arc<crate::schema::NodeType>,
    matched: &ContentMatch,
    open: bool,
) -> Option<Fragment> {
    let node = to.node(depth);
    let index = if open {
        to.index_after(depth)
    } else {
        to.index(depth)
    };
    if index == node.child_count() && !typ.is_compatible_content(node.typ()) {
        return None;
    }
    let fit = matched.fill_before(schema, node.content(), true, index)?;
    // Content that carries marks the frontier's type forbids cannot simply be
    // joined onto it; the caller must find another level.
    for i in index..node.child_count() {
        if let Some(child) = node.child(i)
            && !typ.allows_marks(child.marks())
        {
            return None;
        }
    }
    Some(fit)
}

/// Fills in whatever a node's content expression requires before `open_start`
/// levels of its left edge, so a clipped node can stand on its own.
fn close_node_start(schema: &Schema, node: &Node, open_start: usize, open_end: isize) -> Node {
    if open_start == 0 {
        return node.clone();
    }
    let mut frag = node.content().clone();
    if open_start > 1
        && let Some(first) = frag.child(0)
    {
        let inner = close_node_start(
            schema,
            first,
            open_start - 1,
            if frag.child_count() == 1 { open_end - 1 } else { 0 },
        );
        frag = frag.replace_child(0, inner);
    }
    let start = node.typ().content_match();
    if let Some(before) = start.fill_before(schema, &frag, false, 0) {
        frag = before.append(&frag);
    }
    if open_end <= 0
        && let Some(matched) = start.match_fragment(&frag)
        && let Some(after) = matched.fill_before(schema, &Fragment::empty(), true, 0)
    {
        frag = frag.append(&after);
    }
    node.copy(frag)
}

/// The fragment `depth` levels down the first-child spine.
fn content_at(fragment: &Fragment, depth: usize) -> Fragment {
    let mut cur = fragment.clone();
    for _ in 0..depth {
        let Some(first) = cur.first_child().cloned() else {
            break;
        };
        cur = first.content().clone();
    }
    cur
}

/// Drops `count` children `depth` levels down the first-child spine.
fn drop_from_fragment(fragment: &Fragment, depth: usize, count: usize) -> Fragment {
    if depth == 0 {
        return fragment.cut_by_index(count.min(fragment.child_count()), fragment.child_count());
    }
    let Some(first) = fragment.first_child() else {
        return fragment.clone();
    };
    let inner = drop_from_fragment(first.content(), depth - 1, count);
    fragment.replace_child(0, first.copy(inner))
}

/// Appends `content` `depth` levels down the last-child spine.
fn add_to_fragment(fragment: &Fragment, depth: usize, content: &Fragment) -> Fragment {
    if depth == 0 {
        return fragment.append(content);
    }
    let Some(last) = fragment.last_child() else {
        return fragment.append(content);
    };
    let inner = add_to_fragment(last.content(), depth - 1, content);
    fragment.replace_child(fragment.child_count() - 1, last.copy(inner))
}
