// SPDX-License-Identifier: MPL-2.0

//! A run of sibling nodes.
//!
//! [`Fragment`] is what a node contains and what a [`Slice`](crate::slice::Slice)
//! carries. It is persistent — every operation returns a new fragment sharing
//! whatever it did not change — because a transaction rewrites one paragraph
//! in a document of thousands, and the other thousands must not be copied.
//!
//! # Sizes are bytes
//!
//! A fragment knows its own size, which is the sum of its children's. The unit
//! is the UTF-8 byte, so a text node's size is `text.len()`. See the crate
//! documentation for why that unit and not code points; here it means only
//! that a cut offset landing inside a character is a bug, and is treated as
//! one rather than rounded.

use std::sync::Arc;

use crate::node::{Node, Visitor};

/// A run of sibling nodes, with its total size cached.
#[derive(Debug, Clone)]
pub struct Fragment {
    content: Arc<[Node]>,
    size: usize,
}

impl Default for Fragment {
    fn default() -> Self {
        Self::empty()
    }
}

impl Fragment {
    /// The empty fragment.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            content: Arc::from(Vec::new()),
            size: 0,
        }
    }

    /// A fragment of the given nodes, taking their sizes as given.
    #[must_use]
    pub fn from_vec(content: Vec<Node>) -> Self {
        if content.is_empty() {
            return Self::empty();
        }
        let size = content.iter().map(Node::node_size).sum();
        Self {
            content: Arc::from(content),
            size,
        }
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.content
    }

    /// The total size: node boundaries counted, text counted in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    #[must_use]
    pub fn child_count(&self) -> usize {
        self.content.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    #[must_use]
    pub fn child(&self, index: usize) -> Option<&Node> {
        self.content.get(index)
    }

    #[must_use]
    pub fn first_child(&self) -> Option<&Node> {
        self.content.first()
    }

    #[must_use]
    pub fn last_child(&self) -> Option<&Node> {
        self.content.last()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Node> {
        self.content.iter()
    }

    /// Returns a copy with `index` replaced.
    ///
    /// # Panics
    ///
    /// If `index` is out of range.
    #[must_use]
    pub fn replace_child(&self, index: usize, node: Node) -> Self {
        let current = &self.content[index];
        if current == &node {
            return self.clone();
        }
        let size = self.size + node.node_size() - current.node_size();
        let mut content = self.content.to_vec();
        content[index] = node;
        Self {
            content: Arc::from(content),
            size,
        }
    }

    /// `self` followed by `other`, merging the two text nodes that meet in the
    /// middle when they carry identical markup.
    ///
    /// That merge is not an optimisation. Two adjacent text nodes with the
    /// same marks are the same run written twice, and leaving them split makes
    /// `find_diff_start` report a difference where the reader sees none — so
    /// the view would redraw a paragraph because a transaction happened to
    /// break a string in a new place.
    ///
    /// # Panics
    ///
    /// If the two fragments are not from the same schema, so the merged text
    /// node's markup cannot be compared.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let mut content = self.content.to_vec();
        let mut start = 0;
        let (last, first) = (
            content.last().expect("non-empty"),
            other.first_child().expect("non-empty"),
        );
        if last.is_text() && first.is_text() && last.same_markup(first) {
            let joined = format!(
                "{}{}",
                last.text().expect("is_text"),
                first.text().expect("is_text")
            );
            let index = content.len() - 1;
            content[index] = last.with_text(joined);
            start = 1;
        }
        content.extend(other.content[start..].iter().cloned());
        let size = self.size + other.size;
        Self {
            content: Arc::from(content),
            size,
        }
    }

    /// The sub-fragment covering `from..to` in this fragment's own
    /// coordinates.
    ///
    /// # Panics
    ///
    /// If the range is out of bounds, or if an endpoint falls inside a
    /// character.
    #[must_use]
    pub fn cut(&self, from: usize, to: usize) -> Self {
        assert!(
            from <= to && to <= self.size,
            "cut {from}..{to} out of range for fragment of {}",
            self.size
        );
        if from == 0 && to == self.size {
            return self.clone();
        }
        let mut result = Vec::new();
        let mut pos = 0;
        for child in self.content.iter() {
            if pos >= to {
                break;
            }
            let end = pos + child.node_size();
            if end > from {
                let piece = if pos < from || end > to {
                    if child.is_text() {
                        child.cut(from.saturating_sub(pos), (to - pos).min(child.node_size()))
                    } else {
                        let inner = child.content_size();
                        child.cut(
                            (from.saturating_sub(pos)).saturating_sub(1),
                            (to.saturating_sub(pos).saturating_sub(1)).min(inner),
                        )
                    }
                } else {
                    child.clone()
                };
                result.push(piece);
            }
            pos = end;
        }
        Self::from_vec(result)
    }

    /// The children from `from` to `to`, counted as indices rather than
    /// positions.
    ///
    /// # Panics
    ///
    /// If the indices are out of range.
    #[must_use]
    pub fn cut_by_index(&self, from: usize, to: usize) -> Self {
        if from == to {
            return Self::empty();
        }
        if from == 0 && to == self.content.len() {
            return self.clone();
        }
        Self::from_vec(self.content[from..to].to_vec())
    }

    /// Adds a node at the front, merging with the first child when both are
    /// text with the same markup.
    ///
    /// # Panics
    ///
    /// If a text offset falls inside a character.
    #[must_use]
    pub fn add_to_start(&self, node: Node) -> Self {
        Self::from(node).append(self)
    }

    /// Adds a node at the end, merging with the last child when both are text
    /// with the same markup.
    ///
    /// # Panics
    ///
    /// If a text offset falls inside a character.
    #[must_use]
    pub fn add_to_end(&self, node: Node) -> Self {
        self.append(&Self::from(node))
    }

    /// The child index at `pos`, and that child's start offset.
    ///
    /// `pos` must fall on a child boundary; use [`Fragment::find_index_round`]
    /// when it may fall inside a text node.
    ///
    /// # Panics
    ///
    /// If `pos` is out of range.
    #[must_use]
    pub fn find_index(&self, pos: usize) -> (usize, usize) {
        self.find_index_round(pos, -1)
    }

    /// As [`Fragment::find_index`], but `round` decides which side of a text
    /// node an interior position belongs to: negative for the index of the
    /// node containing it, positive for the index after it.
    ///
    /// # Panics
    ///
    /// If `pos` is out of range.
    #[must_use]
    pub fn find_index_round(&self, pos: usize, round: i32) -> (usize, usize) {
        assert!(
            pos <= self.size,
            "position {pos} out of range for fragment of {}",
            self.size
        );
        if pos == 0 {
            return (0, 0);
        }
        if pos == self.size {
            return (self.content.len(), pos);
        }
        let mut cur = 0;
        for (i, child) in self.content.iter().enumerate() {
            let end = cur + child.node_size();
            if end >= pos {
                if end == pos || round > 0 {
                    return (i + 1, end);
                }
                return (i, cur);
            }
            cur = end;
        }
        unreachable!("pos < size but no child contained it");
    }

    /// Calls `f` for every node that overlaps `from..to`, with the node's
    /// start position in this fragment's coordinates offset by `node_start`.
    ///
    /// Returning `false` from `f` skips that node's children.
    pub fn nodes_between(
        &self,
        from: usize,
        to: usize,
        f: Visitor<'_>,
        node_start: usize,
        parent: Option<&Node>,
    ) {
        let mut pos = 0;
        for (i, child) in self.content.iter().enumerate() {
            if pos >= to {
                break;
            }
            let end = pos + child.node_size();
            if end > from {
                let descend = f(child, node_start + pos, parent, i);
                if descend && child.content_size() > 0 {
                    let start = pos + 1;
                    child.content().nodes_between(
                        from.saturating_sub(start),
                        (to.saturating_sub(start)).min(child.content_size()),
                        f,
                        node_start + start,
                        Some(child),
                    );
                }
            }
            pos = end;
        }
    }

    /// The text of `from..to`, with `block_separator` between block boundaries
    /// and `leaf_text` standing in for non-text leaves.
    #[must_use]
    pub fn text_between(
        &self,
        from: usize,
        to: usize,
        block_separator: Option<&str>,
        leaf_text: Option<&dyn Fn(&Node) -> String>,
    ) -> String {
        let mut text = String::new();
        let mut separated = true;
        self.nodes_between(
            from,
            to,
            &mut |node, pos, _, _| {
                if node.is_text() {
                    let content = node.text().unwrap_or("");
                    let start = from.saturating_sub(pos).min(content.len());
                    let end = (to - pos).min(content.len());
                    text.push_str(&content[start..end]);
                    separated = block_separator.is_none();
                } else if let Some(leaf) = leaf_text
                    && node.is_leaf()
                {
                    text.push_str(&leaf(node));
                    separated = block_separator.is_none();
                } else if !separated && node.is_block() {
                    text.push_str(block_separator.unwrap_or(""));
                    separated = true;
                }
                true
            },
            0,
            None,
        );
        text
    }

    /// The concatenated text of everything in the fragment.
    #[must_use]
    pub fn text_content(&self) -> String {
        self.text_between(0, self.size, None, None)
    }

    /// The first position at which `self` and `other` differ, in the
    /// coordinates of the shared parent starting at `pos`.
    #[must_use]
    pub fn find_diff_start(&self, other: &Self, pos: usize) -> Option<usize> {
        let mut pos = pos;
        for i in 0.. {
            match (self.child(i), other.child(i)) {
                (None, None) => return None,
                (Some(a), Some(b)) => {
                    if a == b {
                        pos += a.node_size();
                        continue;
                    }
                    if !a.same_markup(b) {
                        return Some(pos);
                    }
                    if a.is_text() {
                        let (x, y) = (a.text().unwrap_or(""), b.text().unwrap_or(""));
                        let shared = x.bytes().zip(y.bytes()).take_while(|(p, q)| p == q).count();
                        // Never split a character: walk back to a boundary.
                        let mut shared = shared;
                        while shared > 0 && !x.is_char_boundary(shared) {
                            shared -= 1;
                        }
                        return Some(pos + shared);
                    }
                    if let Some(inner) = a.content().find_diff_start(b.content(), pos + 1) {
                        return Some(inner);
                    }
                    pos += a.node_size();
                }
                _ => return Some(pos),
            }
        }
        None
    }

    /// The last position at which `self` and `other` differ, as a pair of
    /// positions — one in each fragment, because after the difference the two
    /// sides are at different offsets.
    ///
    /// # Panics
    ///
    /// If either position is out of range for its fragment.
    #[must_use]
    pub fn find_diff_end(
        &self,
        other: &Self,
        pos_a: usize,
        pos_b: usize,
    ) -> Option<(usize, usize)> {
        let (mut ia, mut ib) = (self.child_count(), other.child_count());
        let (mut pos_a, mut pos_b) = (pos_a, pos_b);
        loop {
            if ia == 0 || ib == 0 {
                return if ia == ib { None } else { Some((pos_a, pos_b)) };
            }
            ia -= 1;
            ib -= 1;
            let a = self.child(ia).expect("index in range");
            let b = other.child(ib).expect("index in range");
            let size = a.node_size();
            if a == b {
                pos_a -= size;
                pos_b -= size;
                continue;
            }
            if !a.same_markup(b) {
                return Some((pos_a, pos_b));
            }
            if a.is_text() {
                let (x, y) = (a.text().unwrap_or(""), b.text().unwrap_or(""));
                let mut same = x
                    .bytes()
                    .rev()
                    .zip(y.bytes().rev())
                    .take_while(|(p, q)| p == q)
                    .count();
                while same > 0 && !x.is_char_boundary(x.len() - same) {
                    same -= 1;
                }
                return Some((pos_a - same, pos_b - same));
            }
            if (a.content_size() > 0 || b.content_size() > 0)
                && let Some(inner) = a.content().find_diff_end(b.content(), pos_a - 1, pos_b - 1)
            {
                return Some(inner);
            }
            pos_a -= size;
            pos_b -= b.node_size();
        }
    }
}

impl From<Node> for Fragment {
    fn from(node: Node) -> Self {
        let size = node.node_size();
        Self {
            content: Arc::from(vec![node]),
            size,
        }
    }
}

impl From<Vec<Node>> for Fragment {
    fn from(nodes: Vec<Node>) -> Self {
        Self::from_vec(nodes)
    }
}

impl FromIterator<Node> for Fragment {
    fn from_iter<I: IntoIterator<Item = Node>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

impl PartialEq for Fragment {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.content, &other.content)
            || (self.size == other.size && *self.content == *other.content)
    }
}

impl Eq for Fragment {}

impl<'a> IntoIterator for &'a Fragment {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
