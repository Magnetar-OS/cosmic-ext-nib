// SPDX-License-Identifier: MPL-2.0

//! How a document looks.
//!
//! # Two vocabularies, kept apart
//!
//! The model says `strong`, `code`, `heading level 2`, and — through
//! decorations — `keyword`, `string`, `spelling-error`. None of those are
//! colours or sizes. This module is where they become colours and sizes, and
//! it is the only place that knows both vocabularies.
//!
//! Keeping them apart is what lets the same document render in COSMIC's light
//! and dark themes without the highlighter or the schema knowing which is on,
//! and what lets an application restyle emphasis without forking anything.

use std::collections::BTreeMap;

use cosmic::iced::advanced::text::{Span, Wrapping};
use cosmic::iced::font::{Style as FontStyle, Weight};
use cosmic::iced::{Color, Font, Pixels};
use nib_model::decoration::{DecorationSet, Rgba, Style as DecorationStyle};
use nib_model::node::Node;

use crate::blocks::{Block, Kind};

/// What the caret looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Caret {
    /// A thin vertical line between characters. The desktop default.
    #[default]
    Line,
    /// A thicker vertical bar.
    Bar,
    /// A filled rectangle over the next character, as a terminal draws it.
    Block,
    /// A line under the next character.
    Underline,
}

impl Caret {
    /// How wide the caret is, given the text size.
    #[must_use]
    pub fn width(self, text_size: f32) -> f32 {
        match self {
            Self::Line => 1.5,
            Self::Bar => (text_size * 0.12).max(2.0),
            Self::Block | Self::Underline => (text_size * 0.55).max(6.0),
        }
    }
}

/// The colours a document is drawn in.
///
/// Resolved from the COSMIC theme rather than fixed, so a document follows the
/// desktop into dark mode without anything else being told.
#[derive(Debug, Clone, PartialEq)]
pub struct Colors {
    pub text: Color,
    /// What the editor is drawn on.
    ///
    /// Not painted by the widget — it draws no ground of its own — but needed
    /// all the same: an authored colour has to be checked against what it will
    /// actually sit on before it can be allowed. See [`legible`].
    pub background: Color,
    /// Markers, rules, and anything that is furniture rather than content.
    pub muted: Color,
    pub link: Color,
    pub selection: Color,
    pub caret: Color,
    pub code_background: Color,
    pub quote_bar: Color,
    /// Decoration classes — `keyword`, `string`, `comment` — to colours.
    pub syntax: BTreeMap<String, Color>,
}

impl Colors {
    /// The colours of a COSMIC theme.
    #[must_use]
    pub fn from_theme(theme: &cosmic::Theme) -> Self {
        let cosmic = theme.cosmic();
        let text = Color::from(cosmic.on_bg_color());
        let background = Color::from(cosmic.bg_color());
        let accent = Color::from(cosmic.accent_color());
        let muted = Color {
            a: text.a * 0.55,
            ..text
        };
        Self {
            text,
            background,
            muted,
            link: accent,
            selection: Color { a: 0.30, ..accent },
            caret: accent,
            code_background: Color::from(cosmic.bg_component_color()),
            quote_bar: Color { a: 0.4, ..accent },
            syntax: syntax_palette(cosmic),
        }
    }
}

/// A syntax palette derived from the theme's own accents.
///
/// Not a fixed set of hex values: a highlighter that hard-codes its colours is
/// a highlighter that is unreadable in half the themes it will meet. These are
/// the desktop's own hues, and an application with a considered scheme replaces
/// the map wholesale.
fn syntax_palette(cosmic: &cosmic::cosmic_theme::Theme) -> BTreeMap<String, Color> {
    let accent = Color::from(cosmic.accent_color());
    let text = Color::from(cosmic.on_bg_color());
    let warn = Color::from(cosmic.warning_color());
    let success = Color::from(cosmic.success_color());
    let destructive = Color::from(cosmic.destructive_color());
    let comment = Color {
        a: text.a * 0.5,
        ..text
    };

    BTreeMap::from([
        ("keyword".to_owned(), accent),
        ("type".to_owned(), warn),
        ("function".to_owned(), accent),
        ("string".to_owned(), success),
        ("number".to_owned(), warn),
        ("comment".to_owned(), comment),
        ("punctuation".to_owned(), comment),
        ("variable".to_owned(), text),
        ("constant".to_owned(), warn),
        ("attribute".to_owned(), accent),
        ("error".to_owned(), destructive),
        // Not a syntax class, but it arrives the same way and wants the same
        // treatment: a colour the theme owns rather than one a checker chose.
        ("spelling-error".to_owned(), destructive),
        ("search-hit".to_owned(), warn),
        ("search-current".to_owned(), accent),
    ])
}

/// Everything about how the editor draws.
///
/// Compared as a whole when deciding whether a layout can be kept, so every
/// field in it has to be one that can be compared.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub text_size: f32,
    /// A multiple of the text size.
    pub line_height: f32,
    /// Space between blocks, as a multiple of the text size.
    pub block_spacing: f32,
    /// How far one list level indents, as a multiple of the text size.
    pub indent: f32,
    pub quote_bar_width: f32,
    pub padding: f32,
    /// Heading sizes, as multiples of the text size, levels one to six.
    pub heading_scale: [f32; 6],
    pub body_font: Font,
    pub mono_font: Font,
    pub caret: Caret,
    /// Whether code blocks get a gutter of line numbers.
    ///
    /// Only code blocks: numbering prose is numbering something that has no
    /// lines until it is laid out, and a number that changes when the window
    /// is resized is not a reference anybody can use.
    pub line_numbers: bool,
    /// Whether code blocks wrap.
    ///
    /// Off is the setting a programmer wants — a line broken mid-identifier is
    /// worse than one that scrolls — and on is the setting a reader wants.
    /// Prose always wraps; the question does not arise for it.
    pub wrap_code: bool,
    /// How long a full blink cycle takes. Zero means a caret that does not
    /// blink, which is what a user who finds it distracting sets.
    pub blink_period: std::time::Duration,
    /// How long the caret takes to slide to a new position. Zero means it
    /// jumps.
    pub caret_glide: std::time::Duration,
    pub colors: Colors,
}

impl Style {
    /// The default style for a COSMIC theme.
    #[must_use]
    pub fn from_theme(theme: &cosmic::Theme) -> Self {
        Self {
            text_size: 14.0,
            line_height: 1.5,
            block_spacing: 0.75,
            indent: 1.75,
            quote_bar_width: 3.0,
            padding: 12.0,
            // A quiet scale: 2, 1.6, 1.3, 1.15, 1.05, 1. Doubling for a level
            // one and halving thereafter looks like a specimen sheet, not a
            // document.
            heading_scale: [2.0, 1.6, 1.3, 1.15, 1.05, 1.0],
            body_font: cosmic::font::default(),
            mono_font: cosmic::font::mono(),
            caret: Caret::default(),
            line_numbers: false,
            wrap_code: true,
            blink_period: std::time::Duration::from_millis(1060),
            caret_glide: std::time::Duration::from_millis(70),
            colors: Colors::from_theme(theme),
        }
    }

    /// The text size a block is set in.
    #[must_use]
    pub fn size_of(&self, block: &Block) -> f32 {
        match block.level {
            Some(level) => {
                let index = usize::try_from(level.clamp(1, 6)).unwrap_or(1) - 1;
                self.text_size * self.heading_scale[index]
            }
            None => self.text_size,
        }
    }

    /// The height of one line in a block.
    #[must_use]
    pub fn line_height_of(&self, block: &Block) -> f32 {
        // Headings are set tighter: a display size at body leading looks
        // unmoored from the text under it.
        let leading = if block.level.is_some() {
            self.line_height * 0.82
        } else {
            self.line_height
        };
        self.size_of(block) * leading
    }

    /// How far a block is indented from the left.
    #[must_use]
    pub fn indent_of(&self, block: &Block) -> f32 {
        // A document deep enough for these to lose precision is one nobody
        // can scroll to the bottom of.
        #[allow(clippy::cast_precision_loss)]
        let (indent, quotes) = (block.indent as f32, block.quote_depth as f32);
        self.text_size * self.indent * indent
            + (self.quote_bar_width + self.text_size * 0.75) * quotes
    }
}

/// Builds the styled spans for a block's inline content.
///
/// One span per run of identical styling, which is not the same as one span
/// per node: a decoration covering half a text node splits it, and two
/// adjacent nodes with the same marks and no decoration between them do not
/// need two spans.
#[must_use]
pub fn spans<'a>(
    block: &'a Block,
    style: &Style,
    decorations: &DecorationSet,
) -> Vec<Span<'a, (), Font>> {
    let size = style.size_of(block);
    let code_block = is_code(block);
    let mut out: Vec<Span<'a, (), Font>> = Vec::new();

    for (node, segment) in block.inline.iter().zip(&block.segments) {
        let base = span_style(node, style, code_block);
        // Split the node's text where decorations start and stop.
        for (from, to, decoration) in split_by_decorations(segment, decorations) {
            let text = &block.text[from..to];
            if text.is_empty() {
                continue;
            }
            let mut span = Span::new(text)
                .size(Pixels(size * base.scale))
                .font(base.font)
                .color(base.color);
            if let Some(background) = base.background {
                span = span.background(background);
            }
            if base.underline {
                span = span.underline(true);
            }
            if base.strikethrough {
                span = span.strikethrough(true);
            }
            if let Some(decoration) = decoration {
                span = apply_decoration(span, &decoration, style);
            }
            out.push(span);
        }
    }
    out
}

struct Inline {
    font: Font,
    color: Color,
    /// An authored background, already composited and only where it was
    /// legible enough to keep.
    background: Option<Color>,
    underline: bool,
    strikethrough: bool,
    /// The authored size as a multiple of the block's own.
    scale: f32,
}

/// How a node's marks turn into font and colour.
fn span_style(node: &Node, style: &Style, code_block: bool) -> Inline {
    use nib_model::basic::marks;

    let mut font = if code_block {
        style.mono_font
    } else {
        style.body_font
    };
    let mut color = style.colors.text;
    let mut underline = false;
    let mut strikethrough = false;
    let mut authored_color = None;
    let mut authored_background = None;
    let mut scale = 1.0_f32;

    for mark in node.marks() {
        match mark.name() {
            marks::STRONG => font.weight = Weight::Bold,
            marks::EM => font.style = FontStyle::Italic,
            marks::UNDERLINE => underline = true,
            marks::STRIKETHROUGH => strikethrough = true,
            marks::CODE => font = style.mono_font,
            marks::LINK => {
                color = style.colors.link;
                underline = true;
            }
            marks::TEXT_COLOR => authored_color = mark_color(mark),
            marks::BACKGROUND_COLOR => authored_background = mark_color(mark),
            marks::FONT_SIZE => {
                if let Some(value) = mark.attrs().get_float("scale") {
                    #[allow(clippy::cast_possible_truncation, reason = "a ratio, already clamped")]
                    {
                        scale = value as f32;
                    }
                }
            }
            _ => {}
        }
    }

    // The ground this text will actually sit on: the author's background where
    // they set one, the reader's otherwise. Both colours are judged against it
    // rather than against each other, so a legible pair stays legible and an
    // invisible one cannot be arranged out of two individually plausible
    // values.
    let ground = authored_background.map_or(style.colors.background, |background| {
        composite(background, style.colors.background)
    });

    // A background is kept only when the text on it can be read. Dropping it
    // rather than the text is the right way round: the words are the message
    // and the paint is not.
    let background = authored_background.filter(|_| {
        let on = authored_color.map_or(color, |c| composite(c, ground));
        contrast(on, ground) >= MIN_CONTRAST
    });
    let ground = background.map_or(style.colors.background, |b| {
        composite(b, style.colors.background)
    });

    if let Some(authored) = authored_color {
        color = legible(authored, ground, color);
    }

    Inline {
        font,
        color,
        background,
        underline,
        strikethrough,
        scale,
    }
}

/// Cuts a segment's text at every decoration boundary that crosses it.
fn split_by_decorations(
    segment: &crate::blocks::Segment,
    decorations: &DecorationSet,
) -> Vec<(usize, usize, Option<DecorationStyle>)> {
    let overlapping: Vec<(usize, usize, DecorationStyle)> = decorations
        .in_range(segment.doc_from, segment.doc_to)
        .filter_map(|d| {
            let style = match d.kind() {
                nib_model::decoration::Kind::Inline(style) => style.clone(),
                _ => return None,
            };
            let from = d.from().max(segment.doc_from);
            let to = d.to().min(segment.doc_to);
            (from < to).then_some((from, to, style))
        })
        .collect();

    if overlapping.is_empty() {
        return vec![(segment.text_from, segment.text_to, None)];
    }

    // Every boundary, in order, and the style covering each run between them.
    let mut cuts: Vec<usize> = vec![segment.doc_from, segment.doc_to];
    for (from, to, _) in &overlapping {
        cuts.push(*from);
        cuts.push(*to);
    }
    cuts.sort_unstable();
    cuts.dedup();

    let width = segment.text_to - segment.text_from;
    let positions = segment.doc_to - segment.doc_from;
    let to_text = |pos: usize| {
        if width == positions {
            segment.text_from + (pos - segment.doc_from)
        } else if pos <= segment.doc_from {
            segment.text_from
        } else {
            segment.text_to
        }
    };

    cuts.windows(2)
        .filter_map(|pair| {
            let (from, to) = (pair[0], pair[1]);
            if from >= to {
                return None;
            }
            let style = overlapping
                .iter()
                .find(|(a, b, _)| *a <= from && *b >= to)
                .map(|(_, _, style)| style.clone());
            Some((to_text(from), to_text(to), style))
        })
        .collect()
}

fn apply_decoration<'a>(
    mut span: Span<'a, (), Font>,
    decoration: &DecorationStyle,
    style: &Style,
) -> Span<'a, (), Font> {
    if let Some(class) = &decoration.class
        && let Some(color) = style.colors.syntax.get(class.as_ref())
    {
        span = span.color(*color);
    }
    if let Some(color) = decoration.color {
        span = span.color(to_color(color));
    }
    if let Some(background) = decoration.background {
        span = span.background(to_color(background));
    }
    if decoration.bold == Some(true) {
        let mut font = span.font.unwrap_or_default();
        font.weight = Weight::Bold;
        span = span.font(font);
    }
    if decoration.italic == Some(true) {
        let mut font = span.font.unwrap_or_default();
        font.style = FontStyle::Italic;
        span = span.font(font);
    }
    if decoration.underline == Some(true) {
        span = span.underline(true);
    }
    if decoration.strikethrough == Some(true) {
        span = span.strikethrough(true);
    }
    span
}

fn to_color(rgba: Rgba) -> Color {
    let (r, g, b, a) = rgba.channels();
    Color::from_rgba8(r, g, b, f32::from(a) / 255.0)
}

/// The wrapping strategy a block is laid out with.
#[must_use]
pub fn wrapping(block: &Block, style: &Style) -> Wrapping {
    if !is_code(block) {
        return Wrapping::Word;
    }
    // Code never wraps at word boundaries: a line broken mid-identifier reads
    // as two identifiers.
    if style.wrap_code {
        Wrapping::Glyph
    } else {
        Wrapping::None
    }
}

/// True when a block holds code.
#[must_use]
pub fn is_code(block: &Block) -> bool {
    block.language.is_some() || block.type_name == "code_block"
}

/// How wide a code block's line-number gutter is.
///
/// Zero when the block is not code or numbers are off, so the caller can add
/// it unconditionally.
#[must_use]
pub fn gutter_width(block: &Block, style: &Style) -> f32 {
    if !style.line_numbers || !is_code(block) {
        return 0.0;
    }
    // Sized to the widest number the block will show, so a block of nine lines
    // does not indent as far as one of ninety.
    let digits = block.line_count().to_string().len().max(2);
    #[allow(clippy::cast_precision_loss)]
    let digits = digits as f32;
    style.text_size * 0.62 * digits + style.text_size * 0.9
}

/// The marker text a list item is drawn with.
#[must_use]
pub fn marker_text(marker: &crate::blocks::Marker) -> String {
    match marker {
        crate::blocks::Marker::Bullet => "\u{2022}".to_owned(),
        crate::blocks::Marker::Number(n) => format!("{n}."),
        crate::blocks::Marker::Check(true) => "\u{2611}".to_owned(),
        crate::blocks::Marker::Check(false) => "\u{2610}".to_owned(),
    }
}

/// True when a block draws a background behind itself.
#[must_use]
pub fn has_background(block: &Block) -> bool {
    matches!(block.kind, Kind::Text) && block.type_name == "code_block"
}

// ---------------------------------------------------------------------------
// Legibility
// ---------------------------------------------------------------------------

/// The contrast below which text stops being readable rather than merely
/// low-contrast.
///
/// WCAG's bar for body text is 4.5, but that is a *design* target: plenty of
/// deliberate, legitimate styling sits under it, and a reader that overrode all
/// of it would be rewriting messages rather than showing them. 3.0 is the
/// standard's own threshold for large text and interface components, and it is
/// comfortably above the range the hiding tricks live in — white on white is
/// 1.0, and "#fefefe on #ffffff" is 1.01.
const MIN_CONTRAST: f32 = 3.0;

/// An authored colour, or the reader's own where the authored one cannot be
/// seen.
///
/// # Why this exists
///
/// Honouring a sender's colours reintroduces the oldest trick in hostile mail:
/// text the recipient cannot see, sitting in a message that reads innocently,
/// put there for whatever is scanning it rather than for them. The extractor
/// counts white-on-white for exactly this reason.
///
/// Checking contrast at the point of drawing closes it by construction rather
/// than by detection. There is no list of suspicious colours to keep up to
/// date, and no way to phrase a colour that evades the check, because the check
/// is not on the phrasing — it is on the result, against the pixels the text
/// will actually land on.
#[must_use]
pub fn legible(authored: Color, background: Color, fallback: Color) -> Color {
    let on = composite(authored, background);
    if contrast(on, background) >= MIN_CONTRAST {
        on
    } else {
        fallback
    }
}

/// A colour with alpha, flattened onto what is behind it.
///
/// Transparency is a way of asking for low contrast without naming a low
/// contrast colour, so it has to be resolved before the ratio is taken.
#[must_use]
pub fn composite(over: Color, under: Color) -> Color {
    let a = over.a.clamp(0.0, 1.0);
    Color {
        r: over.r * a + under.r * (1.0 - a),
        g: over.g * a + under.g * (1.0 - a),
        b: over.b * a + under.b * (1.0 - a),
        a: 1.0,
    }
}

/// The WCAG 2 contrast ratio between two opaque colours, from 1.0 to 21.0.
#[must_use]
pub fn contrast(a: Color, b: Color) -> f32 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// WCAG relative luminance.
fn luminance(c: Color) -> f32 {
    fn channel(v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// A CSS colour from a mark's `value` attribute.
fn mark_color(mark: &nib_model::mark::Mark) -> Option<Color> {
    let value = mark.attrs().get_str("value")?;
    nib_css::parse_color(value).map(to_color)
}

/// How a block's text is aligned.
///
/// `Default` where the author said nothing, which is what lets the reader's own
/// direction decide instead of a sender's assumption about which side text
/// starts on. `Justify` becomes `Default`: iced shapes one line at a time and
/// has no justification pass, and faking it by padding spaces would break hit
/// testing against the text that is actually there.
#[must_use]
pub fn alignment(block: &Block) -> cosmic::iced::advanced::text::Alignment {
    use cosmic::iced::advanced::text::Alignment;

    match block.align {
        None | Some(nib_css::Align::Justify) => Alignment::Default,
        Some(nib_css::Align::Left) => Alignment::Left,
        Some(nib_css::Align::Center) => Alignment::Center,
        Some(nib_css::Align::Right) => Alignment::Right,
    }
}
