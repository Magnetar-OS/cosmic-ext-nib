// SPDX-License-Identifier: MPL-2.0

//! The safe subset of CSS a Nib document can carry.
//!
//! # Why a subset, and why it is not a sanitiser
//!
//! A sanitiser takes arbitrary CSS and tries to remove the dangerous parts.
//! Every bypass ever written lives in the gap between what the sanitiser
//! thought it removed and what the renderer went on to do. This is the other
//! shape: nothing is removed, because nothing arbitrary is ever represented.
//! A declaration either maps onto one of the properties in [`Declarations`] —
//! a colour, a weight, an alignment — or it does not exist as far as anything
//! downstream is concerned.
//!
//! That is the same argument the document schema makes about elements, applied
//! one level down. `<script>` cannot reach a document because there is no node
//! it could become; `behavior: url(x.htc)` cannot reach a view because there is
//! no field it could land in.
//!
//! # The three things this refuses on purpose
//!
//! **Anything that fetches.** A value containing `url(`, or any `@` rule, is
//! dropped whole and recorded in [`Declarations::refused`]. No property here
//! takes a URL, so this is belt and braces — but a `background` shorthand that
//! quietly carried an image past the colour parser is exactly the kind of
//! near-miss worth failing loudly on.
//!
//! **Anything that hides.** `display: none`, `visibility: hidden`, a
//! transparent or near-transparent `opacity`, a zero `font-size`. These are not
//! honoured *and* not silently ignored: they are counted in
//! [`Declarations::hiding`], because a message that tried to hide text from its
//! reader has said something about itself worth repeating. Honouring them would
//! reintroduce the exact attack that text extraction exists to expose.
//!
//! **Anything positional.** `position`, `float`, `z-index`, negative margins.
//! A document is a sequence of blocks; text that can be moved on top of other
//! text is text that can be hidden underneath it.
//!
//! # What it does not decide
//!
//! Whether a colour is *legible*. This crate reports what the author asked
//! for; a view knows what it is drawing on, and the check belongs where that
//! knowledge is. See `nib::style::legible`.

use nib_model::decoration::Rgba;

mod color;

pub use color::parse_color;

/// How a block's text is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    /// The CSS keyword, and the value an `align=` attribute would carry.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => Some(Self::Left),
            "center" | "centre" | "middle" => Some(Self::Center),
            "right" | "end" => Some(Self::Right),
            "justify" => Some(Self::Justify),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }
}

/// Why a declaration was dropped.
///
/// Kept rather than discarded so that a client can say what a message tried to
/// do, which is the same thing the hidden-text accounting is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The value would have caused a fetch: a `url()`, or an `@` rule.
    Fetches { property: String },
    /// The declaration would have taken the text out of the flow, where it can
    /// be put on top of or underneath other text.
    Positions { property: String },
    /// A property this subset has no field for. The common case, and not a
    /// problem — recorded at all only so the count is honest.
    Unknown { property: String },
}

/// How a declaration would have hidden text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hiding {
    /// `display: none`, `visibility: hidden`.
    Removed,
    /// `opacity` at or near zero.
    Transparent,
    /// A font size of zero, or small enough to be unreadable.
    Shrunk,
}

/// The properties of one declaration list that survived.
///
/// Every field is optional, and absent means "the author said nothing" rather
/// than "the author said the default" — so a nested element inherits from its
/// parent by the caller leaving the field alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Declarations {
    pub color: Option<Rgba>,
    pub background: Option<Rgba>,
    /// `font-weight: bold`, or any numeric weight of 600 or more.
    pub bold: Option<bool>,
    /// `font-style: italic` or `oblique`.
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub align: Option<Align>,
    /// A multiple of the reader's own text size, never an absolute one.
    ///
    /// A message that asks for 9px asks for 9 of *its* pixels, on a display it
    /// cannot see, for a reader whose base size it does not know. Scaling the
    /// reader's size keeps the sender's emphasis — this line is bigger than
    /// that one — without letting them set the floor.
    pub scale: Option<f32>,
    /// Attempts to hide text, in the order they were found.
    pub hiding: Vec<Hiding>,
    /// Declarations that were dropped, and why.
    pub refused: Vec<Refusal>,
}

impl Declarations {
    /// True when nothing survived and nothing was suspicious — the overwhelming
    /// case, and worth testing for before allocating a mark.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.background.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.strikethrough.is_none()
            && self.align.is_none()
            && self.scale.is_none()
    }

    /// True when the author tried to make this text unreadable.
    #[must_use]
    pub fn hides(&self) -> bool {
        !self.hiding.is_empty()
    }

    /// Fills anything this does not say from `parent`.
    ///
    /// CSS inheritance, for the properties that inherit. Alignment and the
    /// text properties do; `background` does not — a child with no background
    /// of its own shows what is behind it rather than repainting its parent's.
    #[must_use]
    pub fn inherited_from(mut self, parent: &Self) -> Self {
        self.color = self.color.or(parent.color);
        self.bold = self.bold.or(parent.bold);
        self.italic = self.italic.or(parent.italic);
        self.underline = self.underline.or(parent.underline);
        self.strikethrough = self.strikethrough.or(parent.strikethrough);
        self.align = self.align.or(parent.align);
        self.scale = match (self.scale, parent.scale) {
            // Two nested scales compound, the way two nested `font-size: 80%`
            // rules do — and are clamped again afterwards, because otherwise
            // the floor leaks: four nested `60%` elements would each be within
            // the limit and the text at the bottom would still be invisible.
            (Some(mine), Some(theirs)) => Some((mine * theirs).clamp(MIN_SCALE, MAX_SCALE)),
            (mine, theirs) => mine.or(theirs),
        };
        self
    }
}

/// The smallest and largest a sender may scale the reader's text.
///
/// The floor is what stops `font-size: 1px` from being a hiding place that
/// [`Hiding::Shrunk`] did not catch — a size can be legibly small and still be
/// chosen to be skipped over. The ceiling stops one word from filling a screen.
const MIN_SCALE: f32 = 0.6;
const MAX_SCALE: f32 = 3.0;

/// Reads one declaration list — the contents of a `style` attribute.
///
/// ```
/// # use nib_css::parse;
/// let d = parse("color: #c00; font-weight: bold; display: none");
/// assert!(d.bold == Some(true));
/// assert!(d.hides(), "the display rule is reported, not obeyed");
/// ```
#[must_use]
pub fn parse(declarations: &str) -> Declarations {
    let mut out = Declarations::default();

    // An `@` rule has no business in a style attribute, and the ones that
    // appear there in the wild are trying something. Refuse the whole list
    // rather than the one declaration: the parse is already not what the
    // author expected, and guessing which half was meant is how a bypass gets
    // in.
    if declarations.contains('@') {
        out.refused.push(Refusal::Fetches {
            property: "@rule".to_owned(),
        });
        return out;
    }

    for declaration in declarations.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            continue;
        }
        apply(&mut out, &property, value);
    }
    out
}

/// Whether a value would cause something to be fetched or executed.
fn fetches(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    // `url(` is the only way a value reaches the network, and the historic
    // execution vectors — IE's `expression()` and `behavior:` — are refused
    // with it rather than trusted to be dead.
    lower.contains("url(") || lower.contains("expression(") || lower.contains("javascript:")
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per property; a table would hide the value parsing"
)]
fn apply(out: &mut Declarations, property: &str, value: &str) {
    if fetches(value) {
        out.refused.push(Refusal::Fetches {
            property: property.to_owned(),
        });
        return;
    }

    let lower = value.to_ascii_lowercase();
    match property {
        "color" => out.color = parse_color(value),

        // Only the colour is read out of `background`. The shorthand can also
        // carry an image, a position and a repeat, and a value that had one
        // was refused by `fetches` above before reaching here.
        "background-color" | "background" => out.background = parse_color(value),

        "font-weight" => {
            out.bold = Some(match lower.as_str() {
                "bold" | "bolder" => true,
                "normal" | "lighter" => false,
                other => other.parse::<u32>().is_ok_and(|w| w >= 600),
            });
        }

        "font-style" => out.italic = Some(matches!(lower.as_str(), "italic" | "oblique")),

        "text-decoration" | "text-decoration-line" => {
            // A shorthand can name both, and naming neither turns both off.
            out.underline = Some(lower.contains("underline"));
            out.strikethrough = Some(lower.contains("line-through"));
        }

        "text-align" => out.align = Align::parse(value),

        "font-size" => match font_scale(&lower) {
            Some(scale) if scale <= 0.0 => out.hiding.push(Hiding::Shrunk),
            Some(scale) => out.scale = Some(scale.clamp(MIN_SCALE, MAX_SCALE)),
            None => {}
        },

        "display" if lower == "none" => out.hiding.push(Hiding::Removed),
        "visibility" if lower == "hidden" || lower == "collapse" => {
            out.hiding.push(Hiding::Removed);
        }
        "opacity" => {
            if lower.parse::<f32>().is_ok_and(|o| o <= 0.05) {
                out.hiding.push(Hiding::Transparent);
            }
        }

        "position" | "float" | "z-index" | "clip" | "clip-path" | "transform" => {
            out.refused.push(Refusal::Positions {
                property: property.to_owned(),
            });
        }

        _ => out.refused.push(Refusal::Unknown {
            property: property.to_owned(),
        }),
    }
}

/// A `font-size` value as a multiple of the reader's own size.
///
/// Absolute units are converted against a nominal 16px, which is not the
/// reader's actual size and is not meant to be: the point is to preserve the
/// *ratio* the author chose, not the measurement.
fn font_scale(value: &str) -> Option<f32> {
    const NOMINAL_PX: f32 = 16.0;

    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(|p| p / 100.0);
    }
    if let Some(em) = value.strip_suffix("em") {
        return em.trim().parse::<f32>().ok();
    }
    if let Some(rem) = value.strip_suffix("rem") {
        return rem.trim().parse::<f32>().ok();
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse::<f32>().ok().map(|p| p / NOMINAL_PX);
    }
    if let Some(pt) = value.strip_suffix("pt") {
        // 1pt is 4/3 of a CSS pixel.
        return pt
            .trim()
            .parse::<f32>()
            .ok()
            .map(|p| p * 4.0 / 3.0 / NOMINAL_PX);
    }
    // `smaller` and `larger` are relative to the parent in CSS; here they are
    // one step of the absolute scale, which is what they amount to in a
    // document whose parent is usually the reader's own size.
    #[allow(
        clippy::match_same_arms,
        reason = "the keywords differ even where the ratios agree"
    )]
    match value {
        "xx-small" => Some(0.6),
        "x-small" => Some(0.75),
        "small" => Some(0.875),
        "medium" => Some(1.0),
        "large" => Some(1.125),
        "x-large" => Some(1.5),
        "xx-large" => Some(2.0),
        "smaller" => Some(0.875),
        "larger" => Some(1.125),
        _ => None,
    }
}
