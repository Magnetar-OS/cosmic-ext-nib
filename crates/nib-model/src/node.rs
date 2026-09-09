// SPDX-License-Identifier: MPL-2.0

//! The document node.
//!
//! One type for every node in the tree, including the document itself and
//! including text. A text node is a node with a `text` and no content; every
//! other node is a node with content and no text. There is no separate leaf
//! type, because "leaf" is a property of the schema (an empty content
//! expression), not of the value — and a paragraph with nothing in it is not a
//! leaf, which is exactly the distinction a separate type would lose.
//!
//! # Persistent, and shared
//!
//! Nodes are immutable. Every method that would change one returns a new node
//! sharing everything it did not touch: [`Fragment`] is behind an `Arc`, so is
//! the mark set, so are the attributes, and so is the node type. Replacing one
//! character in a paragraph rebuilds that paragraph and the spine of ancestors
//! above it — a handful of small allocations — and leaves every sibling
//! subtree alone, still pointed at by both the old document and the new.
//!
//! That is what makes undo affordable (a history entry is a pointer, not a
//! copy), what makes the view's redraw check affordable (`Arc::ptr_eq` on a
//! subtree answers "did this change?" in one comparison), and what makes a
//! transaction safe to abandon halfway.

use std::fmt;
use std::sync::Arc;

use crate::attrs::Attrs;
use crate::fragment::Fragment;
use crate::mark::{Mark, Marks};
use crate::schema::{NodeType, NodeTypeId};

/// What [`Node::nodes_between`] and [`Node::descendants`] call for each node
/// they reach: the node, its position, its parent, and its index in that
/// parent. Returning `false` skips the node's children.
pub type Visitor<'f> = &'f mut dyn FnMut(&Node, usize, Option<&Node>, usize) -> bool;

/// A node in a document.
#[derive(Debug, Clone)]
pub struct Node {
    typ: Arc<NodeType>,
    attrs: Attrs,
    content: Fragment,
    marks: Marks,
    /// `Some` exactly when this is a text node.
    text: Option<Arc<str>>,
}

impl Node {
    /// Builds a node directly. Prefer [`NodeType::create`], which applies
    /// attribute defaults and rejects content the schema forbids.
    #[must_use]
    pub fn new(typ: Arc<NodeType>, attrs: Attrs, content: Fragment, marks: Marks) -> Self {
        Self {
            typ,
            attrs,
            content,
            marks,
            text: None,
        }
    }

    /// Builds a text node directly. Prefer [`NodeType::text`].
    #[must_use]
    pub fn new_text(typ: Arc<NodeType>, text: impl Into<Arc<str>>, marks: Marks) -> Self {
        Self {
            typ,
            attrs: Attrs::none(),
            content: Fragment::empty(),
            marks,
            text: Some(text.into()),
        }
    }

    #[must_use]
    pub fn typ(&self) -> &Arc<NodeType> {
        &self.typ
    }

    #[must_use]
    pub fn type_id(&self) -> NodeTypeId {
        self.typ.id()
    }

    #[must_use]
    pub fn type_name(&self) -> &str {
        self.typ.name()
    }

    #[must_use]
    pub fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    #[must_use]
    pub fn content(&self) -> &Fragment {
        &self.content
    }

    #[must_use]
    pub fn marks(&self) -> &Marks {
        &self.marks
    }

    /// The text, for a text node.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    // -- shape -------------------------------------------------------------

    #[must_use]
    pub fn is_text(&self) -> bool {
        self.text.is_some()
    }

    #[must_use]
    pub fn is_inline(&self) -> bool {
        self.typ.is_inline()
    }

    #[must_use]
    pub fn is_block(&self) -> bool {
        !self.typ.is_inline()
    }

    /// True when the schema allows this node no content at all — an image, a
    /// horizontal rule. An empty paragraph is *not* a leaf.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.typ.is_leaf()
    }

    /// True when the node is to be treated as a single unit by the cursor:
    /// every leaf, plus anything the schema marks `atom` (a mention chip, a
    /// date pill — things with content the user may not put a caret inside).
    #[must_use]
    pub fn is_atom(&self) -> bool {
        self.typ.is_atom()
    }

    /// True when the node is a block whose content is inline — a paragraph, a
    /// heading. The blocks that hold text, and so the blocks a caret lives in.
    #[must_use]
    pub fn is_textblock(&self) -> bool {
        self.is_block() && self.typ.is_inline_content()
    }

    /// The node's size in its parent: the text length in bytes for text, one
    /// for a leaf, and the content size plus the two boundary positions for
    /// everything else.
    #[must_use]
    pub fn node_size(&self) -> usize {
        match &self.text {
            Some(text) => text.len(),
            None if self.is_leaf() => 1,
            None => 2 + self.content.size(),
        }
    }

    #[must_use]
    pub fn content_size(&self) -> usize {
        self.content.size()
    }

    #[must_use]
    pub fn child_count(&self) -> usize {
        self.content.child_count()
    }

    #[must_use]
    pub fn child(&self, index: usize) -> Option<&Node> {
        self.content.child(index)
    }

    #[must_use]
    pub fn first_child(&self) -> Option<&Node> {
        self.content.first_child()
    }

    #[must_use]
    pub fn last_child(&self) -> Option<&Node> {
        self.content.last_child()
    }

    // -- deriving new nodes ------------------------------------------------

    /// The same node with different content.
    #[must_use]
    pub fn copy(&self, content: Fragment) -> Self {
        if content == self.content {
            return self.clone();
        }
        Self {
            typ: Arc::clone(&self.typ),
            attrs: self.attrs.clone(),
            content,
            marks: self.marks.clone(),
            text: self.text.clone(),
        }
    }

    /// The same node with a different mark set.
    #[must_use]
    pub fn with_marks(&self, marks: Marks) -> Self {
        if marks == self.marks {
            return self.clone();
        }
        Self {
            typ: Arc::clone(&self.typ),
            attrs: self.attrs.clone(),
            content: self.content.clone(),
            marks,
            text: self.text.clone(),
        }
    }

    /// The same node with different attributes.
    #[must_use]
    pub fn with_attrs(&self, attrs: Attrs) -> Self {
        if attrs == self.attrs {
            return self.clone();
        }
        Self {
            typ: Arc::clone(&self.typ),
            attrs,
            content: self.content.clone(),
            marks: self.marks.clone(),
            text: self.text.clone(),
        }
    }

    /// The same text node with different text.
    ///
    /// # Panics
    ///
    /// If this is not a text node.
    #[must_use]
    pub fn with_text(&self, text: impl Into<Arc<str>>) -> Self {
        assert!(self.is_text(), "with_text on a non-text node");
        Self {
            typ: Arc::clone(&self.typ),
            attrs: self.attrs.clone(),
            content: Fragment::empty(),
            marks: self.marks.clone(),
            text: Some(text.into()),
        }
    }

    /// The part of this node between `from` and `to`, in the node's own
    /// content coordinates (or byte offsets, for text).
    ///
    /// # Panics
    ///
    /// If a text offset falls inside a character.
    #[must_use]
    pub fn cut(&self, from: usize, to: usize) -> Self {
        if let Some(text) = &self.text {
            if from == 0 && to == text.len() {
                return self.clone();
            }
            assert!(
                text.is_char_boundary(from) && text.is_char_boundary(to),
                "cut {from}..{to} splits a character in {text:?}"
            );
            return self.with_text(&text[from..to]);
        }
        if from == 0 && to == self.content.size() {
            return self.clone();
        }
        self.copy(self.content.cut(from, to))
    }

    // -- comparison --------------------------------------------------------

    /// True when two nodes agree on everything except their content: type,
    /// attributes and marks. What decides whether two adjacent text nodes may
    /// merge, and whether a fragment diff can descend rather than replace.
    ///
    /// Types are compared by identity, not by name, so nodes built against two
    /// separately-built `Schema`s never match — even when the two schemas
    /// declare the same thing. That is deliberate: a document is only
    /// meaningful against the schema it was built for. Hold one `Schema` and
    /// clone it.
    #[must_use]
    pub fn same_markup(&self, other: &Self) -> bool {
        self.has_markup(&other.typ, &other.attrs, &other.marks)
    }

    #[must_use]
    pub fn has_markup(&self, typ: &Arc<NodeType>, attrs: &Attrs, marks: &Marks) -> bool {
        Arc::ptr_eq(&self.typ, typ) && self.attrs == *attrs && self.marks == *marks
    }

    // -- reading -----------------------------------------------------------

    #[must_use]
    pub fn text_content(&self) -> String {
        if let Some(text) = &self.text {
            return (**text).to_owned();
        }
        self.content.text_content()
    }

    /// The text of `from..to`, with `block_separator` between blocks.
    #[must_use]
    pub fn text_between(
        &self,
        from: usize,
        to: usize,
        block_separator: Option<&str>,
        leaf_text: Option<&dyn Fn(&Node) -> String>,
    ) -> String {
        if let Some(text) = &self.text {
            return text[from..to].to_owned();
        }
        self.content
            .text_between(from, to, block_separator, leaf_text)
    }

    /// Calls `f` for every descendant overlapping `from..to`, with its
    /// position relative to the start of this node's content.
    ///
    /// Returning `false` skips that node's children.
    pub fn nodes_between(&self, from: usize, to: usize, f: Visitor<'_>) {
        self.content.nodes_between(from, to, f, 0, Some(self));
    }

    /// Calls `f` for every descendant.
    pub fn descendants(&self, f: Visitor<'_>) {
        self.nodes_between(0, self.content_size(), f);
    }

    /// The innermost node starting at or containing `pos`, descending as far
    /// as it can.
    #[must_use]
    pub fn node_at(&self, pos: usize) -> Option<&Node> {
        let mut node = self;
        let mut pos = pos;
        loop {
            let (index, offset) = node.content.find_index_round(pos, -1);
            let child = node.content.child(index)?;
            if offset == pos || child.is_text() {
                return Some(child);
            }
            pos -= offset + 1;
            node = child;
        }
    }

    /// The child after `pos`, with its index and start position.
    #[must_use]
    pub fn child_after(&self, pos: usize) -> Option<(&Node, usize, usize)> {
        let (index, offset) = self.content.find_index_round(pos, -1);
        self.content.child(index).map(|c| (c, index, offset))
    }

    /// The child before `pos`, with its index and start position.
    #[must_use]
    pub fn child_before(&self, pos: usize) -> Option<(&Node, usize, usize)> {
        if pos == 0 {
            return None;
        }
        let (index, offset) = self.content.find_index_round(pos, -1);
        if offset < pos {
            return self.content.child(index).map(|c| (c, index, offset));
        }
        let index = index.checked_sub(1)?;
        let node = self.content.child(index)?;
        Some((node, index, offset - node.node_size()))
    }

    // -- schema questions --------------------------------------------------

    /// True when every inline node between `from` and `to` carries a mark of
    /// this type. What decides whether a toggle turns the mark on or off.
    #[must_use]
    pub fn range_has_mark(&self, from: usize, to: usize, mark: crate::schema::MarkTypeId) -> bool {
        let mut found = false;
        self.nodes_between(from, to, &mut |node, _, _, _| {
            if found || !node.is_inline() {
                return true;
            }
            if node.marks().iter().any(|m| m.typ().id() == mark) {
                found = true;
            }
            true
        });
        found
    }

    /// A content-match cursor positioned after this node's first `index`
    /// children — what may legally follow them.
    ///
    /// Returns `None` when the children up to `index` are already invalid,
    /// which cannot happen in a document the steps built.
    #[must_use]
    pub fn content_match_at(&self, index: usize) -> Option<crate::content::ContentMatch> {
        self.typ
            .content_match()
            .match_fragment_range(&self.content, 0, index)
    }

    /// True when `marks` may all be applied to a node of this type here.
    #[must_use]
    pub fn allows_marks(&self, marks: &Marks) -> bool {
        marks.iter().all(|m| self.typ.allows_mark_type(m.typ().id()))
    }

    /// `marks` filtered down to those this node's type permits.
    #[must_use]
    pub fn allowed_marks(&self, marks: &Marks) -> Marks {
        marks.retain_allowed(|t| self.typ.allows_mark_type(t.id()))
    }

    /// True when the content between `from` and `to` may be replaced with
    /// `replacement`.
    #[must_use]
    pub fn can_replace(&self, from: usize, to: usize, replacement: &Fragment) -> bool {
        let Some(one) = self
            .typ
            .content_match()
            .match_fragment_range(&self.content, 0, from)
        else {
            return false;
        };
        let Some(two) = one.match_fragment(replacement) else {
            return false;
        };
        let Some(three) =
            two.match_fragment_range(&self.content, to, self.content.child_count())
        else {
            return false;
        };
        if !three.valid_end() {
            return false;
        }
        replacement
            .iter()
            .all(|child| self.typ.allows_marks(child.marks()))
    }

    /// True when a single node of `typ` with `marks` may replace
    /// `from..to`.
    #[must_use]
    pub fn can_replace_with(
        &self,
        from: usize,
        to: usize,
        typ: NodeTypeId,
        marks: Option<&Marks>,
    ) -> bool {
        if let Some(marks) = marks
            && !self.typ.allows_marks(marks)
        {
            return false;
        }
        self.typ
            .content_match()
            .match_fragment_range(&self.content, 0, from)
            .and_then(|m| m.match_type(typ))
            .and_then(|m| {
                m.match_fragment_range(&self.content, to, self.content.child_count())
            })
            .is_some_and(|m| m.valid_end())
    }

    /// True when `other`'s content may be appended to this node's.
    #[must_use]
    pub fn can_append(&self, other: &Self) -> bool {
        if other.content.child_count() > 0 {
            self.can_replace(self.child_count(), self.child_count(), &other.content)
        } else {
            self.typ.is_compatible_content(&other.typ)
        }
    }

    /// Checks the node against its schema, returning the first violation.
    ///
    /// Not called on every edit — the steps are supposed to preserve validity,
    /// and re-validating a document per keystroke would cost more than the
    /// edit. It exists so that tests and untrusted input (a parsed HTML paste)
    /// can be held to the schema, and so that a bug in a step is found where
    /// it happened rather than three edits later.
    ///
    /// # Errors
    ///
    /// Returns a description of the first structural violation found.
    pub fn check(&self) -> Result<(), String> {
        if !self
            .typ
            .content_match()
            .match_fragment(&self.content)
            .is_some_and(|m| m.valid_end())
        {
            return Err(format!(
                "invalid content for node {}: {:?}",
                self.typ.name(),
                self.content
                    .iter()
                    .map(Node::type_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for mark in self.marks.iter() {
            if !self.typ.allows_mark_type(mark.typ().id()) {
                return Err(format!(
                    "mark {} is not allowed on node {}",
                    mark.name(),
                    self.typ.name()
                ));
            }
        }
        for child in self.content.iter() {
            if !self.typ.allows_marks(child.marks()) {
                return Err(format!(
                    "child of {} carries a mark the parent forbids",
                    self.typ.name()
                ));
            }
            child.check()?;
        }
        Ok(())
    }

    /// Adds a mark to this inline node.
    #[must_use]
    pub fn add_mark(&self, mark: &Mark) -> Self {
        self.with_marks(mark.add_to_set(&self.marks))
    }

    /// Removes a mark from this inline node.
    #[must_use]
    pub fn remove_mark(&self, mark: &Mark) -> Self {
        self.with_marks(mark.remove_from_set(&self.marks))
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.typ, &other.typ)
            && self.attrs == other.attrs
            && self.marks == other.marks
            && self.text == other.text
            && self.content == other.content
    }
}

impl Eq for Node {}

impl fmt::Display for Node {
    /// The shape of the node, for tests and for panics — not a serialisation.
    /// `doc(paragraph("hi", em("there")))`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(text) = &self.text {
            for mark in self.marks.iter() {
                write!(f, "{}(", mark.name())?;
            }
            write!(f, "{text:?}")?;
            for _ in self.marks.iter() {
                f.write_str(")")?;
            }
            return Ok(());
        }
        write!(f, "{}", self.typ.name())?;
        // Null attributes are skipped: an unset `alt` or `language` is noise in
        // a shape, and every node carrying one would drown the structure this
        // is here to show. `Debug` keeps them.
        let mut set = self.attrs.iter().filter(|(_, v)| !v.is_null()).peekable();
        if set.peek().is_some() {
            f.write_str("[")?;
            for (i, (name, value)) in set.enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{name}={value}")?;
            }
            f.write_str("]")?;
        }
        if self.content.is_empty() {
            return Ok(());
        }
        f.write_str("(")?;
        for (i, child) in self.content.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{child}")?;
        }
        f.write_str(")")
    }
}
