// SPDX-License-Identifier: MPL-2.0

//! Authored styling: `style=` attributes, and the presentational attributes
//! that predate them.
//!
//! # Why this is marks rather than a style tree
//!
//! Because the document is the thing that survives. A parallel tree of
//! computed styles would have to be mapped through every edit, serialised
//! alongside the document, and kept in agreement with it — three chances to
//! disagree with the text it describes. A mark is carried by the text it
//! applies to, and the model already maps, splits and merges marks through
//! every change.
//!
//! # What is not decided here
//!
//! Whether a colour can be *seen*. This turns `color: #fff` into a mark
//! saying `#fff`; whether that is legible depends on what it is drawn on, and
//! only the view knows that. See `nib::style::legible` — it is what makes
//! white-on-white a thing the reader cannot be shown rather than a thing a
//! parser has to guess at.

use nib_css::{Align, Declarations};
use nib_model::attrs::Attrs;
use nib_model::basic::marks;
use nib_model::mark::Mark;
use nib_model::schema::Schema;

use crate::rules::Element;

/// Everything one element says about how its contents should look.
#[derive(Debug, Clone, Default)]
pub struct Styling {
    /// Marks to apply to this element's descendants.
    pub marks: Vec<Mark>,
    /// The block alignment, when this element is a textblock.
    pub align: Option<Align>,
    /// True when the author tried to hide this element's text.
    ///
    /// The text is shown anyway — that is the point — but a client that counts
    /// what a message tried to do wants to know.
    pub hidden: bool,
}

impl Styling {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.align.is_none()
    }
}

/// Reads an element's styling, from `style=` and from what came before it.
///
/// The presentational attributes are read *first* and the `style` attribute
/// second, because that is CSS's own precedence: a `bgcolor` is a presentation
/// hint and a declaration overrides it.
#[must_use]
pub fn of(schema: &Schema, element: &Element) -> Styling {
    let mut declarations = presentational(element);
    if let Some(style) = element.attr("style") {
        let inline = nib_css::parse(style);
        let hidden = inline.hides();
        // The inline declarations win, and the attributes fill what they do
        // not mention.
        declarations = inline.inherited_from(&declarations);
        declarations.hiding.clear();
        if hidden {
            declarations.hiding.push(nib_css::Hiding::Removed);
        }
    }
    into_marks(schema, &declarations)
}

/// The attributes that did this job before CSS existed, and still arrive.
///
/// Mail is written by generators that target twenty-year-old clients, so
/// `bgcolor` and `<font color>` are not historical curiosities here — they are
/// how a large share of real messages are styled.
fn presentational(element: &Element) -> Declarations {
    let mut out = Declarations::default();

    if let Some(value) = element.attr("color") {
        out.color = nib_css::parse_color(value);
    }
    if let Some(value) = element.attr("bgcolor") {
        out.background = nib_css::parse_color(value);
    }
    if let Some(value) = element.attr("align") {
        out.align = Align::parse(value);
    }
    // `<font size>` is a 1..=7 scale, and `+1`/`-1` step relative to 3.
    if element.tag == "font"
        && let Some(size) = element.attr("size")
    {
        out.scale = font_size_attribute(size);
    }
    out
}

/// HTML's `<font size>`: absolute 1–7, or a signed step from the default of 3.
fn font_size_attribute(value: &str) -> Option<f32> {
    const STEPS: [f32; 7] = [0.6, 0.75, 1.0, 1.125, 1.5, 2.0, 3.0];

    let value = value.trim();
    let level = if let Some(rest) = value.strip_prefix('+') {
        3 + rest.parse::<i32>().ok()?
    } else if let Some(rest) = value.strip_prefix('-') {
        3 - rest.parse::<i32>().ok()?
    } else {
        value.parse::<i32>().ok()?
    };
    let index = usize::try_from(level.clamp(1, 7)).ok()?.checked_sub(1)?;
    STEPS.get(index).copied()
}

/// Turns declarations into the marks a schema actually declares.
///
/// A schema without the styling marks — one an application built for its own
/// purposes — silently gets no styling rather than an error. That is the same
/// contract every other rule here has: the schema decides what a document can
/// hold, and this asks rather than insists.
fn into_marks(schema: &Schema, declarations: &Declarations) -> Styling {
    let mut marks = Vec::new();

    let mut add = |name: &str, attrs: Option<Attrs>| {
        if let Some(id) = schema.mark_id(name)
            && let Ok(mark) = schema.mark_by_id(id, attrs.as_ref())
        {
            marks.push(mark);
        }
    };

    // The properties that already have a mark keep it, rather than growing a
    // second way to say the same thing: `font-weight: bold` and `<b>` produce
    // the same document, so they round-trip to the same HTML.
    if declarations.bold == Some(true) {
        add(marks::STRONG, None);
    }
    if declarations.italic == Some(true) {
        add(marks::EM, None);
    }
    if declarations.underline == Some(true) {
        add(marks::UNDERLINE, None);
    }
    if declarations.strikethrough == Some(true) {
        add(marks::STRIKETHROUGH, None);
    }

    if let Some(color) = declarations.color {
        add(marks::TEXT_COLOR, Some(color_attrs(color)));
    }
    if let Some(background) = declarations.background {
        add(marks::BACKGROUND_COLOR, Some(color_attrs(background)));
    }
    if let Some(scale) = declarations.scale {
        add(
            marks::FONT_SIZE,
            Some(Attrs::none().set("scale", f64::from(scale))),
        );
    }

    Styling {
        marks,
        align: declarations.align,
        hidden: declarations.hides(),
    }
}

/// A colour as the `#rrggbb` or `#rrggbbaa` text a mark carries.
///
/// Normalised rather than kept verbatim: `red`, `#f00` and `rgb(255,0,0)` are
/// one colour, and a document in which they are three values is a document
/// where two identical spans will not merge.
fn color_attrs(color: nib_model::decoration::Rgba) -> Attrs {
    let (r, g, b, a) = color.channels();
    let text = if a == 0xff {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    };
    Attrs::none().set("value", text)
}
