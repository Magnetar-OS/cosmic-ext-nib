// SPDX-License-Identifier: MPL-2.0

//! A piece of a document, cut out with its edges left open.
//!
//! # Why "open" is the whole idea
//!
//! Select from the middle of one paragraph to the middle of the next and copy.
//! What is on the clipboard is not two paragraphs, and it is not one run of
//! text either. It is *the tail of a paragraph, then the head of a paragraph* —
//! and when it is pasted into the middle of a third paragraph, the tail must
//! join what is before the caret and the head must join what is after, leaving
//! one paragraph, not three.
//!
//! [`Slice`] records that as two numbers. `open_start` is how many nodes at
//! the left edge were cut into rather than taken whole; `open_end` the same on
//! the right. A slice with both zero is a run of complete nodes and pastes as
//! a block; a slice with `open_start == 1` pastes its first block's *content*
//! into whatever it lands in.
//!
//! Nothing else in the model needs to know about clipboards, drag and drop, or
//! Enter: they all reduce to a slice and a range to put it in.

use crate::fragment::Fragment;
use crate::node::Node;

/// A fragment with its edges marked open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Slice {
    content: Fragment,
    open_start: usize,
    open_end: usize,
}

impl Slice {
    #[must_use]
    pub fn new(content: Fragment, open_start: usize, open_end: usize) -> Self {
        Self {
            content,
            open_start,
            open_end,
        }
    }

    /// The empty slice: nothing to insert.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            content: Fragment::empty(),
            open_start: 0,
            open_end: 0,
        }
    }

    #[must_use]
    pub fn content(&self) -> &Fragment {
        &self.content
    }

    /// How many nodes at the left edge are open.
    #[must_use]
    pub fn open_start(&self) -> usize {
        self.open_start
    }

    /// How many nodes at the right edge are open.
    #[must_use]
    pub fn open_end(&self) -> usize {
        self.open_end
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// The number of positions the slice occupies once inserted — its content
    /// size less the boundaries that are open, since an open boundary is not
    /// inserted but joined.
    #[must_use]
    pub fn size(&self) -> usize {
        self.content.size() - self.open_start - self.open_end
    }

    /// The same slice with different open depths.
    #[must_use]
    pub fn with_open(&self, open_start: usize, open_end: usize) -> Self {
        Self {
            content: self.content.clone(),
            open_start,
            open_end,
        }
    }

    /// Inserts a fragment at `pos` within the slice's content, descending
    /// through open nodes so the insertion lands where a caret at `pos` would
    /// be.
    ///
    /// Returns `None` when `pos` does not name a place inline content can go.
    #[must_use]
    pub fn insert_at(&self, pos: usize, fragment: Fragment) -> Option<Self> {
        let content = insert_into(&self.content, pos + self.open_start, fragment)?;
        Some(Self {
            content,
            open_start: self.open_start,
            open_end: self.open_end,
        })
    }

    /// Removes `from..to` from the slice's content, in the coordinates of the
    /// slice's inserted positions.
    ///
    /// Returns `None` when the range is not flat — when it starts inside one
    /// node and ends inside another. A gap-replace's gap is always flat by
    /// construction, and this is what checks that.
    #[must_use]
    pub fn remove_between(&self, from: usize, to: usize) -> Option<Self> {
        let content = remove_range(
            &self.content,
            from + self.open_start,
            to + self.open_start,
        )?;
        Some(Self {
            content,
            open_start: self.open_start,
            open_end: self.open_end,
        })
    }
}

fn remove_range(content: &Fragment, from: usize, to: usize) -> Option<Fragment> {
    let (index, offset) = content.find_index_round(from, -1);
    let child = content.child(index);
    let (index_to, offset_to) = content.find_index_round(to, -1);
    if offset == from || child.is_some_and(Node::is_text) {
        if offset_to != to && !content.child(index_to).is_some_and(Node::is_text) {
            return None;
        }
        return Some(
            content
                .cut(0, from)
                .append(&content.cut(to, content.size())),
        );
    }
    if index != index_to {
        return None;
    }
    let child = child?;
    let inner = remove_range(child.content(), from - offset - 1, to - offset - 1)?;
    Some(content.replace_child(index, child.copy(inner)))
}

fn insert_into(content: &Fragment, dist: usize, insert: Fragment) -> Option<Fragment> {
    let (index, offset) = content.find_index_round(dist, -1);
    let child = content.child(index);
    if offset == dist || child.is_some_and(Node::is_text) {
        if content.child(index).is_some_and(Node::is_text) && offset != dist {
            return None;
        }
        return Some(
            content
                .cut(0, dist)
                .append(&insert)
                .append(&content.cut(dist, content.size())),
        );
    }
    let child = child?;
    let inner = insert_into(child.content(), dist - offset - 1, insert)?;
    Some(content.replace_child(index, child.copy(inner)))
}

impl Node {
    /// The slice between two positions.
    ///
    /// With `include_parents`, the slice reaches up to the document root
    /// rather than to the deepest node containing both ends — which is what a
    /// drag of whole blocks wants, and what a text selection does not.
    ///
    /// # Panics
    ///
    /// If either position is outside the node.
    #[must_use]
    pub fn slice(&self, from: usize, to: usize, include_parents: bool) -> Slice {
        if from == to {
            return Slice::empty();
        }
        let r_from = self.resolve(from);
        let r_to = self.resolve(to);
        let depth = if include_parents {
            0
        } else {
            r_from.shared_depth(to)
        };
        let start = r_from.start(depth);
        let node = r_from.node(depth);
        let content = node.content().cut(r_from.pos() - start, r_to.pos() - start);
        Slice::new(
            content,
            r_from.depth() - depth,
            r_to.depth() - depth,
        )
    }
}
