// SPDX-License-Identifier: MPL-2.0

//! Replacing and deleting *ranges*, as opposed to exact position spans.
//!
//! The difference is what the user meant. Selecting the whole text of a
//! paragraph and pressing Delete does not mean "remove those characters and
//! leave an empty paragraph" if the selection also covered the paragraph's
//! siblings — it means remove the paragraphs. Pasting a heading into an empty
//! paragraph does not mean "put a heading inside this paragraph", it means
//! replace the paragraph.
//!
//! [`Transform::delete_range`] and [`Transform::replace_range`] widen the range
//! outwards through nodes the range fully covers, and stop at nodes the schema
//! marks `defining` — which is what keeps a paste from escaping a table cell or
//! dissolving the blockquote it landed in.

use crate::fragment::Fragment;
use crate::mark::Marks;
use crate::node::Node;
use crate::resolve::ResolvedPos;
use crate::schema::NodeTypeId;
use crate::slice::Slice;
use crate::transform::fit::replace_step;
use crate::transform::step::StepError;
use crate::transform::Transform;

/// Every depth at which `from..to` covers the whole content of the node.
///
/// Outermost last. A deletion widens through these, because a range that
/// covers everything inside a node was probably meant to include the node.
fn covered_depths(from: &ResolvedPos, to: &ResolvedPos) -> Vec<usize> {
    let mut result = Vec::new();
    let min_depth = from.depth().min(to.depth());
    for d in (0..=min_depth).rev() {
        let start = from.start(d);
        if start < from.pos() - (from.depth() - d)
            || to.end(d) > to.pos() + (to.depth() - d)
            || from.node(d).typ().spec().isolating
            || to.node(d).typ().spec().isolating
        {
            break;
        }
        let both_inline = d == from.depth()
            && d == to.depth()
            && from.parent().is_textblock()
            && to.parent().is_textblock()
            && d > 0
            && to.start(d - 1) == start - 1;
        if start == to.start(d) || both_inline {
            result.push(d);
        }
    }
    result
}

/// True when content lifted out of this type should keep the type around it.
fn defines_content(node: &Node) -> bool {
    node.typ().spec().defining
}

impl Transform {
    /// Replaces `from..to` with `slice`, fitting the slice to the gap.
    ///
    /// Unlike [`Transform::replace`], which requires the slice to line up
    /// exactly, this searches for a way to make it fit — opening the slice,
    /// dropping its edges, wrapping what is left. What paste calls.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the fitted step does not apply. A slice that cannot
    /// be fitted at all is not an error: nothing happens.
    pub fn replace_fitted(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        let schema = self.schema().clone();
        if let Some(step) = replace_step(&schema, self.doc(), from, to, &slice) {
            self.step(step)?;
        }
        Ok(self)
    }

    /// Deletes `from..to`, widening to whole nodes the range fully covers.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the deletion does not apply.
    pub fn delete_range(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        let doc = self.doc().clone();
        let r_from = doc.resolve(from);
        let r_to = doc.resolve(to);
        let covered = covered_depths(&r_from, &r_to);

        for (i, &depth) in covered.iter().enumerate() {
            let last = i == covered.len() - 1;
            // The node's own content may legally be empty: empty it in place.
            if (last && depth == 0)
                || r_from
                    .node(depth)
                    .typ()
                    .content_match()
                    .valid_end()
            {
                return self.delete(r_from.start(depth), r_to.end(depth));
            }
            // Otherwise take the node itself, if the parent will allow it.
            if depth > 0
                && (last
                    || r_from.node(depth - 1).can_replace(
                        r_from.index(depth - 1),
                        r_to.index_after(depth - 1),
                        &Fragment::empty(),
                    ))
            {
                return self.delete(r_from.before(depth), r_to.after(depth));
            }
        }

        // A selection that starts at the very beginning of a node and ends
        // past its end: take the node's opening with it, so backspacing from
        // the first character of a list item does not leave the item behind.
        for d in 1..=r_from.depth().min(r_to.depth()) {
            if from - r_from.start(d) == r_from.depth() - d
                && to > r_from.end(d)
                && r_to.end(d) - to != r_to.depth() - d
                && r_from.start(d - 1) == r_to.start(d - 1)
                && r_from.node(d - 1).can_replace(
                    r_from.index(d - 1),
                    r_to.index(d - 1),
                    &Fragment::empty(),
                )
            {
                return self.delete(r_from.before(d), to);
            }
        }
        self.delete(from, to)
    }

    /// Replaces `from..to` with `slice`, widening the range where the slice's
    /// content would rather replace a node than sit inside it.
    ///
    /// # Errors
    ///
    /// [`StepError`] when no attempt applies.
    pub fn replace_range(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        if slice.is_empty() {
            return self.delete_range(from, to);
        }
        let doc = self.doc().clone();
        let r_from = doc.resolve(from);
        let r_to = doc.resolve(to);

        // The depths worth trying, most-preferred first. Negative entries mean
        // "expand the start but not the end", which is what an insertion at
        // the very start of a node wants.
        let mut targets: Vec<isize> = covered_depths(&r_from, &r_to)
            .into_iter()
            .map(|d| d as isize)
            .collect();
        if targets.last() == Some(&0) {
            targets.pop();
        }
        let mut preferred: isize = -((r_from.depth() + 1) as isize);
        targets.insert(0, preferred);

        {
            let mut pos = r_from.pos().wrapping_sub(1);
            for d in (1..=r_from.depth()).rev() {
                let spec = r_from.node(d).typ().spec();
                if spec.defining || spec.isolating {
                    break;
                }
                if targets.contains(&(d as isize)) {
                    preferred = d as isize;
                } else if r_from.before(d) == pos {
                    targets.insert(1, -(d as isize));
                }
                pos = pos.wrapping_sub(1);
            }
        }
        let preferred_index = targets.iter().position(|d| *d == preferred).unwrap_or(0);

        // The chain of first children down the slice's open left edge: the
        // candidates for "what is actually being inserted".
        let mut left_nodes: Vec<Node> = Vec::new();
        {
            let mut content = slice.content().clone();
            for i in 0.. {
                let Some(node) = content.first_child().cloned() else {
                    break;
                };
                left_nodes.push(node.clone());
                if i == slice.open_start() {
                    break;
                }
                content = node.content().clone();
            }
        }

        // Prefer inserting at the depth of a `defining` node — a heading, a
        // list item — over the textblock inside it, so pasting a list item
        // replaces the item rather than burying it.
        let mut preferred_depth = slice.open_start();
        for d in (0..preferred_depth).rev() {
            let Some(node) = left_nodes.get(d) else { break };
            let def = defines_content(node);
            let target_node_type = usize::try_from(targets[preferred_index].abs())
                .ok()
                .filter(|d| *d <= r_from.depth())
                .map(|d| r_from.node(d).type_id());
            if def && target_node_type != Some(node.type_id()) {
                preferred_depth = d;
            } else if def || !node.is_textblock() {
                break;
            }
        }

        for j in (0..=slice.open_start()).rev() {
            let open_depth = (j + preferred_depth + 1) % (slice.open_start() + 1);
            let Some(insert) = left_nodes.get(open_depth) else {
                continue;
            };
            for i in 0..targets.len() {
                let raw = targets[(i + preferred_index) % targets.len()];
                let expand = raw >= 0;
                let Ok(target_depth) = usize::try_from(raw.abs()) else {
                    continue;
                };
                // `before` and `after` are defined one deeper than the
                // position itself, and that depth is the preferred target.
                if target_depth == 0 || target_depth > r_from.depth() + 1 {
                    continue;
                }
                let parent = r_from.node(target_depth - 1);
                let index = r_from.index(target_depth - 1);
                if parent.can_replace_with(index, index, insert.type_id(), Some(insert.marks())) {
                    let content = close_fragment(
                        self,
                        slice.content(),
                        0,
                        slice.open_start(),
                        open_depth,
                        None,
                    );
                    let end = if expand {
                        r_to.after(target_depth)
                    } else {
                        to
                    };
                    return self.replace_fitted(
                        r_from.before(target_depth),
                        end,
                        Slice::new(content, open_depth, slice.open_end()),
                    );
                }
            }
        }

        // Nothing preferred worked: try the plain fit, then progressively
        // wider ranges.
        let start_steps = self.steps().len();
        let (mut from, mut to) = (from, to);
        for i in (0..targets.len()).rev() {
            self.replace_fitted(from, to, slice.clone())?;
            if self.steps().len() > start_steps {
                break;
            }
            let Ok(depth) = usize::try_from(targets[i]) else {
                continue;
            };
            if depth == 0 || depth > r_from.depth() || depth > r_to.depth() {
                continue;
            }
            from = r_from.before(depth);
            to = r_to.after(depth);
        }
        Ok(self)
    }

    /// Replaces `from..to` with a single node, widening as
    /// [`Transform::replace_range`] does.
    ///
    /// # Errors
    ///
    /// [`StepError`] when no attempt applies.
    pub fn replace_range_with(
        &mut self,
        from: usize,
        to: usize,
        node: Node,
    ) -> Result<&mut Self, StepError> {
        // An inline node dropped into a textblock does not want the range
        // widening: it goes exactly where the caret is.
        if !node.is_inline()
            && from == to
            && let Some(point) = crate::transform::structure::insert_point(
                self.doc(),
                from,
                node.type_id(),
            )
        {
            return self.replace_fitted(point, point, Slice::new(Fragment::from(node), 0, 0));
        }
        self.replace_range(from, to, Slice::new(Fragment::from(node), 0, 0))
    }

    /// Inserts a node at `pos`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the insertion does not apply.
    pub fn insert(&mut self, pos: usize, node: Node) -> Result<&mut Self, StepError> {
        self.replace_range_with(pos, pos, node)
    }

    /// Creates a node of `typ` and inserts it at `pos`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the type cannot be created, or the insertion does
    /// not apply.
    pub fn insert_type(
        &mut self,
        pos: usize,
        typ: NodeTypeId,
        content: Fragment,
    ) -> Result<&mut Self, StepError> {
        let node = self
            .schema()
            .create(typ, None, content, Marks::none())
            .map_err(|_| StepError::NoNodeAt(pos))?;
        self.insert(pos, node)
    }
}

/// Re-closes a slice's left edge at a shallower open depth, filling in
/// whatever the schema then requires.
fn close_fragment(
    tr: &Transform,
    fragment: &Fragment,
    depth: usize,
    old_open: usize,
    new_open: usize,
    parent: Option<&Node>,
) -> Fragment {
    let mut fragment = fragment.clone();
    if depth < old_open
        && let Some(first) = fragment.first_child().cloned()
    {
        let inner = close_fragment(
            tr,
            first.content(),
            depth + 1,
            old_open,
            new_open,
            Some(&first),
        );
        fragment = fragment.replace_child(0, first.copy(inner));
    }
    if depth > new_open
        && let Some(parent) = parent
        && let Some(matched) = parent.content_match_at(0)
    {
        let schema = tr.schema();
        if let Some(before) = matched.fill_before(schema, &fragment, false, 0) {
            fragment = before.append(&fragment);
        }
        if let Some(after) = matched
            .match_fragment(&fragment)
            .and_then(|m| m.fill_before(schema, &Fragment::empty(), true, 0))
        {
            fragment = fragment.append(&after);
        }
    }
    fragment
}
