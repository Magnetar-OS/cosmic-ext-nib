//! What one keystroke costs, measured.
//!
//! Builds a document of a given size, then times the two halves of a rebuild:
//! flattening the document into blocks, and shaping every block into a
//! paragraph. Run it before changing either.

use std::time::Instant;

use cosmic::iced::advanced::graphics::text::Paragraph;
use nib::blocks;
use nib::style::Style;
use nib_model::basic;
use nib_model::decoration::DecorationSet;

fn main() {
    let paragraphs: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(400);

    let schema = basic::schema();
    let b = nib_model::build::Builder::new(schema.clone());
    let blocks_in: Vec<nib_model::node::Node> = (0..paragraphs)
        .map(|i| {
            if i % 12 == 0 {
                b.attr_node(
                    basic::nodes::HEADING,
                    &nib_model::attrs! { "level" => 2_i64 },
                    vec![b.text(&format!("Section {i}"))],
                )
            } else {
                let mut inline = vec![b.text(&format!("Paragraph {i} with "))];
                inline.extend(b.mark(basic::marks::STRONG, None, vec![b.text("bold")]));
                inline.push(b.text(
                    " and a run of ordinary words that goes on long enough to wrap over \
                     more than one line when it is laid out.",
                ));
                b.node(basic::nodes::PARAGRAPH, inline)
            }
        })
        .collect();
    let doc = b.doc(blocks_in);
    println!("{paragraphs} blocks, {} bytes", doc.content_size());

    let style = Style::from_theme(&cosmic::theme::Theme::dark());
    let decorations = DecorationSet::empty();
    let width: f32 = 800.0;

    let started = Instant::now();
    let flat = blocks::flatten(&doc);
    let flatten = started.elapsed();

    let started = Instant::now();
    let mut kept = Vec::new();
    for block in &flat {
        let available = (width - style.padding * 2.0 - style.indent_of(block)).max(style.text_size);
        let spans = nib::style::spans(block, &style, &decorations);
        let paragraph = <Paragraph as cosmic::iced::advanced::text::Paragraph>::with_spans(
            cosmic::iced::advanced::text::Text {
                content: spans.as_slice(),
                bounds: cosmic::iced::Size::new(available, f32::INFINITY),
                size: style.size_of(block).into(),
                line_height: cosmic::iced::advanced::text::LineHeight::Absolute(
                    style.line_height_of(block).into(),
                ),
                font: style.body_font,
                align_x: cosmic::iced::advanced::text::Alignment::Default,
                align_y: cosmic::iced::alignment::Vertical::Top,
                shaping: cosmic::iced::advanced::text::Shaping::Advanced,
                wrapping: nib::style::wrapping(block, &style),
                ellipsize: cosmic::iced::advanced::text::Ellipsize::None,
            },
        );
        kept.push(paragraph);
    }
    let shape = started.elapsed();

    // The diff a rebuild would do instead, on the document as one keystroke
    // leaves it: built by a transaction, so it shares everything it did not
    // touch with the one before.
    let state = nib_model::state::EditorState::with_selection(
        schema.clone(),
        doc.clone(),
        nib_model::state::Selection::cursor(doc.content_size() / 2),
        Vec::new(),
    );
    let mut tr = state.tr();
    tr.insert_text("x").expect("type one character");
    let edited = tr.doc().clone();
    let started = Instant::now();
    for _ in 0..100 {
        let _ = doc.content().find_diff_start(edited.content(), 0);
        let _ = doc.content().find_diff_end(
            edited.content(),
            doc.content_size(),
            edited.content_size(),
        );
    }
    let diff = started.elapsed() / 100;

    println!("flatten: {flatten:?}");
    println!(
        "shape:   {shape:?}   ({} paragraphs)  <- what a full rebuild costs",
        kept.len()
    );
    println!("diff:    {diff:?}");
    keystroke(&doc, &edited, &style, &decorations, width);
}

/// What one keystroke costs once the layout is only redone where the document
/// changed: the same diff, plus the blocks it says are new.
fn keystroke(
    doc: &nib_model::node::Node,
    edited: &nib_model::node::Node,
    style: &Style,
    decorations: &DecorationSet,
    width: f32,
) {
    let old_flat = blocks::flatten(doc);
    let new_flat = blocks::flatten(edited);
    let started = Instant::now();
    let start = doc.content().find_diff_start(edited.content(), 0);
    let ends =
        doc.content()
            .find_diff_end(edited.content(), doc.content_size(), edited.content_size());
    let before = start.unwrap_or(usize::MAX);
    let after = ends.map_or(0, |(_, new)| new.max(before));
    let prefix = new_flat
        .iter()
        .take(old_flat.len())
        .take_while(|b| b.to < before)
        .count();
    let suffix = new_flat
        .iter()
        .rev()
        .zip(old_flat.iter().rev())
        .take_while(|(b, _)| b.node_pos >= after)
        .count();
    let rebuilt = new_flat.len().saturating_sub(prefix + suffix);
    for block in &new_flat[prefix..prefix + rebuilt] {
        let available = (width - style.padding * 2.0 - style.indent_of(block)).max(style.text_size);
        let spans = nib::style::spans(block, style, decorations);
        let _ = <Paragraph as cosmic::iced::advanced::text::Paragraph>::with_spans(
            cosmic::iced::advanced::text::Text {
                content: spans.as_slice(),
                bounds: cosmic::iced::Size::new(available, f32::INFINITY),
                size: style.size_of(block).into(),
                line_height: cosmic::iced::advanced::text::LineHeight::Absolute(
                    style.line_height_of(block).into(),
                ),
                font: style.body_font,
                align_x: cosmic::iced::advanced::text::Alignment::Default,
                align_y: cosmic::iced::alignment::Vertical::Top,
                shaping: cosmic::iced::advanced::text::Shaping::Advanced,
                wrapping: nib::style::wrapping(block, style),
                ellipsize: cosmic::iced::advanced::text::Ellipsize::None,
            },
        );
    }
    let keystroke = started.elapsed();
    println!("keystroke: {keystroke:?}   (kept {prefix} + {suffix}, shaped {rebuilt})");
}
