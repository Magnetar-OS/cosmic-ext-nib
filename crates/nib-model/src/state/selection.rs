// SPDX-License-Identifier: MPL-2.0

//! What is selected.
//!
//! Three shapes, and the third is the reason the first two are not enough:
//!
//! - **Text** — a range of inline content, possibly collapsed to a caret. The
//!   ordinary case.
//! - **Node** — a whole node selected as a unit. An image has no inside for a
//!   caret to be in; clicking it must select *it*, and pressing a key must
//!   replace it.
//! - **All** — the whole document, including structure. Distinct from a text
//!   selection covering everything, because replacing "all" may remove the
//!   blocks themselves and replacing a text range may not.
//!
//! # Anchor and head, not start and end
//!
//! A selection knows which end the user is dragging. Shift-arrow extends from
//! the head; the anchor stays. Collapsing to `from`/`to` loses that, so both
//! are kept and `from`/`to` are derived.
//!
//! # Selections are mapped, never recomputed
//!
//! After any change — local, remote, or an undo — the selection goes through
//! the same [`Mapping`](crate::transform::Mapping) as every other position.
//! When mapping lands the head somewhere a caret cannot be, [`Selection::near`]
//! searches outwards for the closest place one can.

use crate::mark::Marks;
use crate::node::Node;
use crate::resolve::ResolvedPos;
use crate::slice::Slice;
use crate::transform::Mapping;

/// Which of the three shapes a selection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A range of inline content.
    Text,
    /// One node, selected whole.
    Node,
    /// The entire document.
    All,
}

/// A selection: a kind and two positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    kind: Kind,
    anchor: usize,
    head: usize,
}

impl Selection {
    /// A text selection between two positions.
    #[must_use]
    pub fn text(anchor: usize, head: usize) -> Self {
        Self {
            kind: Kind::Text,
            anchor,
            head,
        }
    }

    /// A collapsed text selection — a caret.
    #[must_use]
    pub fn cursor(pos: usize) -> Self {
        Self::text(pos, pos)
    }

    /// Selects the node starting at `pos`.
    ///
    /// Returns `None` when there is no node there, or the schema says it is
    /// not selectable.
    #[must_use]
    pub fn node(doc: &Node, pos: usize) -> Option<Self> {
        let at = doc.resolve(pos);
        let node = at.node_after()?;
        if !node.typ().spec().selectable || at.text_offset() > 0 {
            return None;
        }
        Some(Self {
            kind: Kind::Node,
            anchor: pos,
            head: pos + node.node_size(),
        })
    }

    /// The whole document.
    #[must_use]
    pub fn all(doc: &Node) -> Self {
        Self {
            kind: Kind::All,
            anchor: 0,
            head: doc.content_size(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    #[must_use]
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    #[must_use]
    pub fn head(&self) -> usize {
        self.head
    }

    #[must_use]
    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    #[must_use]
    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// True when nothing is selected — a caret.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kind == Kind::Text && self.anchor == self.head
    }

    /// The caret position, when the selection is one.
    #[must_use]
    pub fn cursor_pos(&self) -> Option<usize> {
        self.is_empty().then_some(self.head)
    }

    /// The selected node, when this is a node selection.
    #[must_use]
    pub fn selected_node(&self, doc: &Node) -> Option<Node> {
        (self.kind == Kind::Node)
            .then(|| doc.resolve(self.anchor).node_after())
            .flatten()
    }

    /// The selected content as a slice.
    #[must_use]
    pub fn content(&self, doc: &Node) -> Slice {
        doc.slice(self.from(), self.to(), self.kind == Kind::All)
    }

    /// The marks a character typed at a collapsed selection would carry.
    #[must_use]
    pub fn marks(&self, doc: &Node) -> Marks {
        doc.resolve(self.head).marks()
    }

    /// This selection in a document `mapping`'s changes have been applied to.
    ///
    /// The head is mapped first, because it is the end the user is holding; if
    /// it lands somewhere inline content cannot be, the whole selection moves
    /// to the nearest place it can.
    #[must_use]
    pub fn map(&self, doc: &Node, mapping: &Mapping) -> Self {
        match self.kind {
            Kind::All => Self::all(doc),
            Kind::Node => {
                let result = mapping.map_result(self.anchor, 1);
                let at = doc.resolve(result.pos().min(doc.content_size()));
                if result.deleted() {
                    return Self::near(doc, &at, 1);
                }
                Self::node(doc, result.pos()).unwrap_or_else(|| Self::near(doc, &at, 1))
            }
            Kind::Text => {
                let head = mapping.map(self.head, 1).min(doc.content_size());
                let r_head = doc.resolve(head);
                if !r_head.parent().is_textblock() {
                    return Self::near(doc, &r_head, 1);
                }
                let anchor = mapping.map(self.anchor, 1).min(doc.content_size());
                let r_anchor = doc.resolve(anchor);
                if r_anchor.parent().is_textblock() {
                    Self::text(anchor, head)
                } else {
                    Self::text(head, head)
                }
            }
        }
    }

    /// A valid selection as close to `at` as possible, searching in `bias`
    /// first and then the other way.
    #[must_use]
    pub fn near(doc: &Node, at: &ResolvedPos, bias: i32) -> Self {
        Self::find_from(doc, at, bias, false)
            .or_else(|| Self::find_from(doc, at, -bias, false))
            .unwrap_or_else(|| Self::all(doc))
    }

    /// A selection at the very start of the document.
    #[must_use]
    pub fn at_start(doc: &Node) -> Self {
        find_selection_in(doc, doc, 0, 0, 1, false).unwrap_or_else(|| Self::all(doc))
    }

    /// A selection at the very end of the document.
    #[must_use]
    pub fn at_end(doc: &Node) -> Self {
        find_selection_in(doc, doc, doc.content_size(), doc.child_count(), -1, false)
            .unwrap_or_else(|| Self::all(doc))
    }

    /// The first valid selection at or after `at`, searching in `dir`.
    ///
    /// With `text_only`, node selections are not considered — used when
    /// extending a text selection, which must not swallow an image as a node.
    #[must_use]
    pub fn find_from(doc: &Node, at: &ResolvedPos, dir: i32, text_only: bool) -> Option<Self> {
        if at.parent().is_textblock() {
            return Some(Self::cursor(at.pos()));
        }
        if let Some(inner) = find_selection_in(
            doc,
            at.parent(),
            at.pos(),
            at.index(at.depth()),
            dir,
            text_only,
        ) {
            return Some(inner);
        }
        for depth in (0..at.depth()).rev() {
            let found = if dir < 0 {
                find_selection_in(
                    doc,
                    at.node(depth),
                    at.before(depth + 1),
                    at.index(depth),
                    dir,
                    text_only,
                )
            } else {
                find_selection_in(
                    doc,
                    at.node(depth),
                    at.after(depth + 1),
                    at.index(depth) + 1,
                    dir,
                    text_only,
                )
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// A text selection between two positions, moved to somewhere a caret can
    /// actually be.
    #[must_use]
    pub fn between(doc: &Node, anchor: usize, head: usize) -> Self {
        let r_head = doc.resolve(head);
        let bias = if anchor >= head { 1 } else { -1 };

        let head = if r_head.parent().is_textblock() {
            head
        } else {
            match Self::find_from(doc, &r_head, bias, true)
                .or_else(|| Self::find_from(doc, &r_head, -bias, true))
            {
                Some(found) => found.head,
                None => return Self::near(doc, &r_head, bias),
            }
        };

        let r_anchor = doc.resolve(anchor);
        if r_anchor.parent().is_textblock() {
            return Self::text(anchor, head);
        }
        if anchor == head {
            return Self::text(head, head);
        }
        let moved = Self::find_from(doc, &r_anchor, -bias, true)
            .or_else(|| Self::find_from(doc, &r_anchor, bias, true))
            .map_or(head, |s| s.anchor);
        if (moved < head) == (anchor < head) {
            Self::text(moved, head)
        } else {
            Self::text(head, head)
        }
    }
}

/// Walks `node`'s children from `index` in `dir`, looking for the first place
/// a selection can be.
fn find_selection_in(
    doc: &Node,
    node: &Node,
    pos: usize,
    index: usize,
    dir: i32,
    text_only: bool,
) -> Option<Selection> {
    if node.is_textblock() {
        return Some(Selection::cursor(pos));
    }
    let mut pos = pos;
    let mut i = if dir > 0 {
        index.cast_signed()
    } else {
        index.cast_signed() - 1
    };
    while if dir > 0 {
        i.cast_unsigned() < node.child_count()
    } else {
        i >= 0
    } {
        let child = node.child(i.cast_unsigned())?;
        if child.is_atom() {
            if !text_only && child.typ().spec().selectable {
                let at = if dir < 0 {
                    pos - child.node_size()
                } else {
                    pos
                };
                if let Some(selection) = Selection::node(doc, at) {
                    return Some(selection);
                }
            }
        } else if let Some(inner) = find_selection_in(
            doc,
            child,
            if dir > 0 { pos + 1 } else { pos - 1 },
            if dir < 0 { child.child_count() } else { 0 },
            dir,
            text_only,
        ) {
            return Some(inner);
        }
        pos = if dir > 0 {
            pos + child.node_size()
        } else {
            pos - child.node_size()
        };
        i += isize::try_from(dir).unwrap_or(0);
    }
    None
}
