// SPDX-License-Identifier: MPL-2.0

//! Styling that is not in the document.
//!
//! A spelling underline, a search hit, a collaborator's cursor, a syntax
//! colour inside a code block — none of these belong in the document. They are
//! not what the author wrote, they must not survive a copy to the clipboard,
//! and they must not turn into a step on the undo stack. But they have to move
//! with the text they describe, and there is already a mechanism for that:
//! positions map.
//!
//! A [`Decoration`] is a range or a point plus a description of how it should
//! look. A [`DecorationSet`] is a collection of them that can be mapped
//! through a change in one call.
//!
//! # Why classes, and not colours
//!
//! Because the thing producing a decoration knows *what a token is* and the
//! thing drawing it knows *what the theme is*, and those are different
//! programs. A highlighter says `keyword`; the view asks COSMIC what colour
//! keywords are in the current theme, light or dark. Baking a colour in here
//! would make every syntax theme a fork of the highlighter, and would make
//! this crate — which cannot see a screen — decide what a screen shows.
//!
//! [`Style::color`] exists anyway, for the cases where the colour genuinely is
//! the datum rather than the theme's business: a specific collaborator's
//! cursor is that person's colour, not a role in a palette.

use std::sync::Arc;

use crate::transform::Mapping;

/// A colour, as it travels through a toolkit-free crate: eight bits a channel,
/// `0xRRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rgba(pub u32);

impl Rgba {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(u32::from_be_bytes([r, g, b, a]))
    }

    #[must_use]
    pub const fn channels(self) -> (u8, u8, u8, u8) {
        let [r, g, b, a] = self.0.to_be_bytes();
        (r, g, b, a)
    }
}

/// How a decorated range should look.
///
/// Every field is optional and the view applies what it is given, so a
/// highlighter that only knows class names sets only `class`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// A role the theme resolves: `keyword`, `string`, `spelling-error`.
    pub class: Option<Arc<str>>,
    pub color: Option<Rgba>,
    pub background: Option<Rgba>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl Style {
    #[must_use]
    pub fn class(name: impl Into<Arc<str>>) -> Self {
        Self {
            class: Some(name.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn colored(color: Rgba) -> Self {
        Self {
            color: Some(color),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_background(mut self, color: Rgba) -> Self {
        self.background = Some(color);
        self
    }

    #[must_use]
    pub fn with_bold(mut self) -> Self {
        self.bold = Some(true);
        self
    }

    #[must_use]
    pub fn with_italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }

    #[must_use]
    pub fn with_underline(mut self) -> Self {
        self.underline = Some(true);
        self
    }
}

/// What a decoration decorates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Styles the inline content in the range.
    Inline(Style),
    /// Styles the node covering the range as a whole — a background behind a
    /// paragraph, a border on the block a collaborator is editing.
    Node(Style),
    /// A point where the view should draw something of its own, named so the
    /// view can decide what. A collapsed-quote marker, a line number, a
    /// collaborator's caret.
    ///
    /// `side` breaks the tie when a widget sits exactly where content is
    /// inserted: negative to stay before it, positive to move after.
    Widget { name: Arc<str>, side: i32 },
}

/// One decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    from: usize,
    to: usize,
    kind: Kind,
}

impl Decoration {
    /// Styles a range of inline content.
    #[must_use]
    pub fn inline(from: usize, to: usize, style: Style) -> Self {
        Self {
            from,
            to,
            kind: Kind::Inline(style),
        }
    }

    /// Styles a whole node.
    #[must_use]
    pub fn node(from: usize, to: usize, style: Style) -> Self {
        Self {
            from,
            to,
            kind: Kind::Node(style),
        }
    }

    /// Places a widget at a point.
    #[must_use]
    pub fn widget(pos: usize, name: impl Into<Arc<str>>, side: i32) -> Self {
        Self {
            from: pos,
            to: pos,
            kind: Kind::Widget {
                name: name.into(),
                side,
            },
        }
    }

    #[must_use]
    pub fn from(&self) -> usize {
        self.from
    }

    #[must_use]
    pub fn to(&self) -> usize {
        self.to
    }

    #[must_use]
    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    #[must_use]
    pub fn style(&self) -> Option<&Style> {
        match &self.kind {
            Kind::Inline(style) | Kind::Node(style) => Some(style),
            Kind::Widget { .. } => None,
        }
    }

    /// This decoration in a document the mapping's changes have been applied
    /// to, or `None` when what it described is gone.
    #[must_use]
    pub fn map(&self, mapping: &Mapping) -> Option<Self> {
        match &self.kind {
            Kind::Widget { side, .. } => {
                let result = mapping.map_result(self.from, if *side < 0 { -1 } else { 1 });
                (!result.deleted()).then(|| Self {
                    from: result.pos(),
                    to: result.pos(),
                    kind: self.kind.clone(),
                })
            }
            _ => {
                let from = mapping.map_result(self.from, 1);
                let to = mapping.map_result(self.to, -1);
                // A range whose content was removed has nothing left to
                // describe; a range that merely shrank is kept.
                if from.deleted_across() && to.deleted_across() {
                    return None;
                }
                let (from, to) = (from.pos(), to.pos());
                (from < to || matches!(self.kind, Kind::Node(_))).then(|| Self {
                    from,
                    to: to.max(from),
                    kind: self.kind.clone(),
                })
            }
        }
    }
}

/// A collection of decorations, mapped as one.
///
/// Kept sorted by start position, so [`DecorationSet::in_range`] can find what
/// overlaps a block without scanning the lot. Flat rather than tree-shaped:
/// ProseMirror's `DecorationSet` mirrors the document tree to keep mapping
/// cheap on very large documents, and that is an optimisation to make when a
/// profile asks for it, not before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecorationSet {
    decorations: Vec<Decoration>,
}

impl DecorationSet {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn new(mut decorations: Vec<Decoration>) -> Self {
        decorations.sort_by_key(|d| (d.from, d.to));
        Self { decorations }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decorations.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.decorations.len()
    }

    #[must_use]
    pub fn all(&self) -> &[Decoration] {
        &self.decorations
    }

    /// Every decoration overlapping `from..to`, plus point widgets inside it.
    pub fn in_range(&self, from: usize, to: usize) -> impl Iterator<Item = &Decoration> {
        self.decorations
            .iter()
            .filter(move |d| d.from <= to && d.to >= from)
    }

    /// Adds decorations.
    #[must_use]
    pub fn with(&self, more: Vec<Decoration>) -> Self {
        let mut all = self.decorations.clone();
        all.extend(more);
        Self::new(all)
    }

    /// Drops the decorations `keep` rejects — how a highlighter replaces just
    /// the block it re-parsed.
    #[must_use]
    pub fn retaining(&self, keep: impl Fn(&Decoration) -> bool) -> Self {
        Self {
            decorations: self
                .decorations
                .iter()
                .filter(|d| keep(d))
                .cloned()
                .collect(),
        }
    }

    /// This set in a document the mapping's changes have been applied to.
    #[must_use]
    pub fn map(&self, mapping: &Mapping) -> Self {
        Self::new(
            self.decorations
                .iter()
                .filter_map(|d| d.map(mapping))
                .collect(),
        )
    }
}

impl FromIterator<Decoration> for DecorationSet {
    fn from_iter<I: IntoIterator<Item = Decoration>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}
