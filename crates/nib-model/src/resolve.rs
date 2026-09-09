// SPDX-License-Identifier: MPL-2.0

//! Turning a flat position into a path through the tree.
//!
//! # Why positions are integers
//!
//! A position could be a path — "third child of the first child of the doc,
//! four characters in" — and paths are what the tree actually is. But every
//! edit invalidates every path that points after it, so a path-shaped position
//! has to be rewritten by hand at every call site, and the ones that are
//! forgotten are the bugs where the cursor jumps a paragraph after someone
//! else's edit arrives.
//!
//! A flat integer has one rule instead — *map it through the change* — and
//! that rule is mechanical, is implemented once in
//! [`Mapping`](crate::transform::Mapping), and applies equally to the cursor,
//! a selection, a decoration, a comment anchor and a collaborator's caret.
//!
//! # How the integers are laid out
//!
//! Every position is a place *between* things, counted from the start of the
//! document:
//!
//! ```text
//!  0   1     4    5  6    9   10
//!  <p>  o n e  </p> <p> t w o </p>
//! ```
//!
//! - Entering or leaving a node costs one.
//! - A character costs its length in UTF-8 bytes.
//! - A leaf costs one, and has no position inside it.
//!
//! So position 0 is before the first paragraph, 1 is at its start, 4 is at its
//! end, 5 is between the paragraphs.
//!
//! [`ResolvedPos`] is the expensive form: the chain of ancestors and the index
//! within each. Commands resolve a position once and then ask it questions.

use crate::mark::Marks;
use crate::node::Node;

/// A position with its context: the chain of nodes containing it, and where it
/// sits in each.
#[derive(Debug, Clone)]
pub struct ResolvedPos {
    pos: usize,
    /// One entry per depth, outermost first: the node at that depth, the index
    /// of the child the position is in or before, and that child's absolute
    /// start position.
    path: Vec<(Node, usize, usize)>,
    parent_offset: usize,
}

impl ResolvedPos {
    /// Resolves `pos` against `doc`.
    ///
    /// # Panics
    ///
    /// If `pos` is outside the document.
    #[must_use]
    pub fn resolve(doc: &Node, pos: usize) -> Self {
        assert!(
            pos <= doc.content_size(),
            "position {pos} is outside a document of {}",
            doc.content_size()
        );
        let mut path = Vec::new();
        let mut start = 0;
        let mut parent_offset = pos;
        let mut node = doc.clone();
        loop {
            let (index, offset) = node.content().find_index_round(parent_offset, -1);
            let rem = parent_offset - offset;
            let child = node.child(index).cloned();
            path.push((node, index, start + offset));
            if rem == 0 {
                break;
            }
            let Some(child) = child else { break };
            if child.is_text() {
                break;
            }
            parent_offset = rem - 1;
            start += offset + 1;
            node = child;
        }
        Self {
            pos,
            path,
            parent_offset,
        }
    }

    /// The position itself.
    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// How deep the position sits. Zero is directly in the document.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.path.len() - 1
    }

    /// The offset of the position within its parent's content.
    #[must_use]
    pub fn parent_offset(&self) -> usize {
        self.parent_offset
    }

    /// The node the position is directly inside.
    #[must_use]
    pub fn parent(&self) -> &Node {
        self.node(self.depth())
    }

    /// The document.
    #[must_use]
    pub fn doc(&self) -> &Node {
        self.node(0)
    }

    /// The ancestor at `depth`.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position.
    #[must_use]
    pub fn node(&self, depth: usize) -> &Node {
        &self.path[depth].0
    }

    /// The index, within the node at `depth`, of the child the position is in
    /// or immediately before.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position.
    #[must_use]
    pub fn index(&self, depth: usize) -> usize {
        self.path[depth].1
    }

    /// The index *after* the position, within the node at `depth`. Differs
    /// from [`ResolvedPos::index`] when the position is inside a text node,
    /// which it is then both before and after.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position.
    #[must_use]
    pub fn index_after(&self, depth: usize) -> usize {
        let bump = usize::from(!(depth == self.depth() && self.text_offset() == 0));
        self.index(depth) + bump
    }

    /// The position at the start of the node at `depth`'s content.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position.
    #[must_use]
    pub fn start(&self, depth: usize) -> usize {
        if depth == 0 {
            0
        } else {
            self.path[depth - 1].2 + 1
        }
    }

    /// The position at the end of the node at `depth`'s content.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position.
    #[must_use]
    pub fn end(&self, depth: usize) -> usize {
        self.start(depth) + self.node(depth).content_size()
    }

    /// The position directly before the node at `depth`.
    ///
    /// # Panics
    ///
    /// If `depth` is zero — the document has nothing before it — or deeper
    /// than this position plus one.
    #[must_use]
    pub fn before(&self, depth: usize) -> usize {
        assert!(depth > 0, "there is no position before the document");
        if depth == self.depth() + 1 {
            self.pos
        } else {
            self.path[depth - 1].2
        }
    }

    /// The position directly after the node at `depth`.
    ///
    /// # Panics
    ///
    /// If `depth` is zero, or deeper than this position plus one.
    #[must_use]
    pub fn after(&self, depth: usize) -> usize {
        assert!(depth > 0, "there is no position after the document");
        if depth == self.depth() + 1 {
            self.pos
        } else {
            self.path[depth - 1].2 + self.path[depth].0.node_size()
        }
    }

    /// How far into its text node the position is — zero when it is on a node
    /// boundary rather than inside text.
    #[must_use]
    pub fn text_offset(&self) -> usize {
        self.pos - self.path[self.path.len() - 1].2
    }

    /// The node directly after the position, cut to start there when the
    /// position is inside text.
    #[must_use]
    pub fn node_after(&self) -> Option<Node> {
        let parent = self.parent();
        let index = self.index(self.depth());
        let child = parent.child(index)?;
        let offset = self.text_offset();
        if offset == 0 {
            Some(child.clone())
        } else {
            Some(child.cut(offset, child.node_size()))
        }
    }

    /// The node directly before the position, cut to end there when the
    /// position is inside text.
    #[must_use]
    pub fn node_before(&self) -> Option<Node> {
        let index = self.index(self.depth());
        let offset = self.text_offset();
        if offset > 0 {
            return self.parent().child(index).map(|c| c.cut(0, offset));
        }
        self.parent().child(index.checked_sub(1)?).cloned()
    }

    /// The position of the `index`th child of the node at `depth`.
    ///
    /// # Panics
    ///
    /// If `depth` is deeper than this position, or `index` is out of range.
    #[must_use]
    pub fn pos_at_index(&self, index: usize, depth: usize) -> usize {
        let node = self.node(depth);
        let mut pos = self.start(depth);
        for i in 0..index {
            pos += node.child(i).expect("index in range").node_size();
        }
        pos
    }

    /// The marks a character typed here would carry.
    ///
    /// Between two runs, the marks of the one *before* — that is what makes
    /// typing after bold text continue it — except for marks whose type is not
    /// inclusive, which stop at their run's edge. A link is the case that
    /// matters: continuing a sentence after a link must not extend the link.
    #[must_use]
    pub fn marks(&self) -> Marks {
        let parent = self.parent();
        if parent.content_size() == 0 {
            return Marks::none();
        }
        let index = self.index(self.depth());
        if self.text_offset() > 0 {
            return parent
                .child(index)
                .map_or_else(Marks::none, |c| c.marks().clone());
        }

        let before = index.checked_sub(1).and_then(|i| parent.child(i));
        let after = parent.child(index);
        // With nothing before, the run after is the one to inherit from.
        let (main, other) = match (before, after) {
            (Some(before), after) => (before, after),
            (None, Some(after)) => (after, None),
            (None, None) => return Marks::none(),
        };

        let mut marks = main.marks().clone();
        for mark in main.marks() {
            let inclusive = mark.typ().is_inclusive();
            let continues = other.is_some_and(|o| mark.is_in_set(o.marks()));
            if !inclusive && !continues {
                marks = mark.remove_from_set(&marks);
            }
        }
        marks
    }

    /// The marks that survive a replacement spanning from here to `end` — the
    /// marks the inserted text should take on.
    #[must_use]
    pub fn marks_across(&self, end: &Self) -> Option<Marks> {
        let after = self.parent().child(self.index(self.depth()))?;
        if !after.is_inline() {
            return None;
        }
        let next = end.parent().child(end.index(end.depth()));
        let mut marks = after.marks().clone();
        for mark in after.marks() {
            let inclusive = mark.typ().is_inclusive();
            let continues = next.is_some_and(|n| mark.is_in_set(n.marks()));
            if !inclusive && !continues {
                marks = mark.remove_from_set(&marks);
            }
        }
        Some(marks)
    }

    /// The deepest node containing both this position and `pos`.
    #[must_use]
    pub fn shared_depth(&self, pos: usize) -> usize {
        let mut depth = self.depth();
        while depth > 0 {
            if self.start(depth) <= pos && self.end(depth) >= pos {
                return depth;
            }
            depth -= 1;
        }
        0
    }

    /// True when two positions sit directly in the same node.
    #[must_use]
    pub fn same_parent(&self, other: &Self) -> bool {
        self.depth() == other.depth() && self.pos - self.parent_offset == other.pos - other.parent_offset
    }

    /// The range of sibling blocks spanned by this position and `other`,
    /// optionally restricted to a parent satisfying `pred`.
    ///
    /// This is the shape almost every structural command works on: "the list
    /// items the selection touches", "the paragraphs to indent". `pred` is how
    /// `lift` finds the nearest ancestor it is allowed to lift out of.
    #[must_use]
    pub fn block_range(
        &self,
        other: &Self,
        pred: Option<&dyn Fn(&Node) -> bool>,
    ) -> Option<NodeRange> {
        if other.pos < self.pos {
            return other.block_range(self, pred);
        }
        // A range of blocks starts one level out from a position that sits in
        // inline content — and also from a collapsed position, which is asking
        // "which blocks am I in" rather than "which blocks do I span".
        let out = self.parent().is_textblock() || self.pos == other.pos;
        let start = self.depth().checked_sub(usize::from(out))?;
        for depth in (0..=start).rev() {
            if other.pos <= self.end(depth) && pred.is_none_or(|p| p(self.node(depth))) {
                return Some(NodeRange {
                    from: self.clone(),
                    to: other.clone(),
                    depth,
                });
            }
        }
        None
    }

    /// The greater of two positions.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if other.pos > self.pos { other } else { self }
    }

    /// The lesser of two positions.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        if other.pos < self.pos { other } else { self }
    }
}

/// A run of sibling nodes, named by two positions and the depth of their
/// shared parent.
#[derive(Debug, Clone)]
pub struct NodeRange {
    from: ResolvedPos,
    to: ResolvedPos,
    depth: usize,
}

impl NodeRange {
    /// A range over the children of the node at `depth`, spanned by two
    /// positions.
    ///
    /// Prefer [`ResolvedPos::block_range`], which finds the depth; this is for
    /// commands that already know which one they want.
    #[must_use]
    pub fn new(from: ResolvedPos, to: ResolvedPos, depth: usize) -> Self {
        Self { from, to, depth }
    }

    #[must_use]
    pub fn from(&self) -> &ResolvedPos {
        &self.from
    }

    #[must_use]
    pub fn to(&self) -> &ResolvedPos {
        &self.to
    }

    /// The depth of the parent whose children the range covers.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The position before the range's first node.
    #[must_use]
    pub fn start(&self) -> usize {
        self.from.before(self.depth + 1)
    }

    /// The position after the range's last node.
    #[must_use]
    pub fn end(&self) -> usize {
        self.to.after(self.depth + 1)
    }

    /// The node whose children the range covers.
    #[must_use]
    pub fn parent(&self) -> &Node {
        self.from.node(self.depth)
    }

    /// The index of the first covered child.
    #[must_use]
    pub fn start_index(&self) -> usize {
        self.from.index(self.depth)
    }

    /// The index after the last covered child.
    #[must_use]
    pub fn end_index(&self) -> usize {
        self.to.index_after(self.depth)
    }
}

impl Node {
    /// Resolves a position against this node as the document root.
    ///
    /// # Panics
    ///
    /// If `pos` is outside the node.
    #[must_use]
    pub fn resolve(&self, pos: usize) -> ResolvedPos {
        ResolvedPos::resolve(self, pos)
    }
}
