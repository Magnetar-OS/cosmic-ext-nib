// SPDX-License-Identifier: MPL-2.0

//! The parts of the view that do not need a window: how a document becomes
//! boxes, how a hit becomes a position, and how a caret behaves over time.

use std::time::{Duration, Instant};

use nib::blocks::{self, Marker};
use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::node::Node;
use nib_model::{attrs, nodes};

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn flatten(doc: &Node) -> Vec<blocks::Block> {
    blocks::flatten(doc)
}

// ---------------------------------------------------------------------------
// Flattening
// ---------------------------------------------------------------------------

#[test]
fn a_document_becomes_one_box_per_block_in_order() {
    let b = b();
    let doc = b.doc(nodes![
        b.attr_node(
            nodes::HEADING,
            &attrs! { "level" => 1_i64 },
            nodes![b.text("Title")]
        ),
        b.node(nodes::PARAGRAPH, nodes![b.text("body")]),
    ]);
    let found = flatten(&doc);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].type_name, nodes::HEADING);
    assert_eq!(found[0].level, Some(1));
    assert_eq!(found[1].text, "body");
}

#[test]
fn a_blocks_range_is_where_its_content_lives() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello")])]);
    let block = &flatten(&doc)[0];
    // 0 <p> 1 "hello" 6 </p>
    assert_eq!((block.node_pos, block.from, block.to), (0, 1, 6));
    assert!(block.contains(1) && block.contains(6));
    assert!(!block.contains(7));
}

#[test]
fn a_quote_records_its_depth_and_a_list_its_indent() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(
            nodes::BULLET_LIST,
            nodes![b.node(
                nodes::LIST_ITEM,
                nodes![b.node(nodes::PARAGRAPH, nodes![b.text("deep")])]
            )]
        )]
    )]);
    let block = &flatten(&doc)[0];
    assert_eq!(block.quote_depth, 1);
    assert_eq!(block.indent, 1);
    assert_eq!(block.marker, Some(Marker::Bullet));
}

#[test]
fn an_ordered_list_numbers_from_its_start() {
    let b = b();
    let item = |t: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(t)])],
        )
    };
    let doc = b.doc(nodes![b.attr_node(
        nodes::ORDERED_LIST,
        &attrs! { "start" => 3_i64 },
        nodes![item("a"), item("b")]
    )]);
    let found = flatten(&doc);
    assert_eq!(found[0].marker, Some(Marker::Number(3)));
    assert_eq!(found[1].marker, Some(Marker::Number(4)));
}

#[test]
fn a_task_item_gets_a_checkbox() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BULLET_LIST,
        nodes![b.attr_node(
            nodes::LIST_ITEM,
            &attrs! { "checked" => true },
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text("done")])]
        )]
    )]);
    assert_eq!(flatten(&doc)[0].marker, Some(Marker::Check(true)));
}

#[test]
fn a_marker_belongs_to_the_items_first_block_only() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BULLET_LIST,
        nodes![b.node(
            nodes::LIST_ITEM,
            nodes![
                b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
                b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
            ]
        )]
    )]);
    let found = flatten(&doc);
    assert_eq!(found[0].marker, Some(Marker::Bullet));
    assert_eq!(found[1].marker, None, "the second block is not a new item");
}

#[test]
fn a_table_flattens_to_its_cells_contents() {
    let b = b();
    let cell = |t: &str| {
        b.node(
            nodes::TABLE_CELL,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(t)])],
        )
    };
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![b.node(nodes::TABLE_ROW, nodes![cell("a"), cell("b")])]
    )]);
    let found = flatten(&doc);
    assert_eq!(found.len(), 2);
    let cells: Vec<_> = found.iter().filter_map(|block| block.cell).collect();
    assert_eq!(cells.len(), 2);
    assert_eq!((cells[0].row, cells[0].column), (0, 0));
    assert_eq!((cells[1].row, cells[1].column), (0, 1));
    assert_eq!(found[0].text, "a");
    assert_eq!(found[1].text, "b");
}

#[test]
fn a_cell_spanning_columns_moves_the_next_one_along() {
    let b = b();
    let wide = b.attr_node(
        nodes::TABLE_CELL,
        &attrs! { "colspan" => 2_i64 },
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("wide")])],
    );
    let narrow = b.node(
        nodes::TABLE_CELL,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("after")])],
    );
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![b.node(nodes::TABLE_ROW, nodes![wide, narrow])]
    )]);
    let found = flatten(&doc);
    let cells: Vec<_> = found.iter().filter_map(|block| block.cell).collect();
    assert_eq!(cells[0].span, 2);
    assert_eq!(cells[1].column, 2, "the next cell starts past the span");
}

#[test]
fn a_header_row_is_marked_as_one() {
    let b = b();
    let header = b.node(
        nodes::TABLE_HEADER,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("name")])],
    );
    let body = b.node(
        nodes::TABLE_CELL,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("value")])],
    );
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![
            b.node(nodes::TABLE_ROW, nodes![header]),
            b.node(nodes::TABLE_ROW, nodes![body]),
        ]
    )]);
    let found = flatten(&doc);
    assert!(found[0].cell.expect("a cell").header);
    assert!(!found[1].cell.expect("a cell").header);
    assert_eq!(found[1].cell.expect("a cell").row, 1);
}

#[test]
fn two_tables_in_a_row_are_told_apart() {
    let b = b();
    let table = |text: &str| {
        b.node(
            nodes::TABLE,
            nodes![b.node(
                nodes::TABLE_ROW,
                nodes![b.node(
                    nodes::TABLE_CELL,
                    nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])]
                )]
            )],
        )
    };
    let doc = b.doc(nodes![table("first"), table("second")]);
    let found = flatten(&doc);
    assert_ne!(
        found[0].cell.expect("a cell").table,
        found[1].cell.expect("a cell").table
    );
}

#[test]
fn a_rule_is_a_box_with_no_text() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("before")]),
        b.node(nodes::HORIZONTAL_RULE, nodes![]),
    ]);
    let found = flatten(&doc);
    assert_eq!(found[1].kind, blocks::Kind::Rule);
    assert!(found[1].text.is_empty());
}

// ---------------------------------------------------------------------------
// Positions and text offsets
// ---------------------------------------------------------------------------

#[test]
fn every_position_in_a_block_round_trips_through_its_text() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("plain "),
            b.mark("strong", None, nodes![b.text("bold")]),
            b.text(" end"),
        ]
    )]);
    let block = &flatten(&doc)[0];
    for pos in block.from..=block.to {
        let offset = block.text_offset(pos);
        assert_eq!(
            block.doc_position(offset),
            pos,
            "position {pos} did not survive the trip through offset {offset}"
        );
    }
}

#[test]
fn an_inline_atom_is_one_position_however_it_is_drawn() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("a"),
            b.attr_node(nodes::IMAGE, &attrs! { "src" => "x" }, nodes![]),
            b.text("b"),
        ]
    )]);
    let block = &flatten(&doc)[0];
    // The image draws as a placeholder character but occupies one position.
    assert_eq!(block.text.chars().count(), 3);
    assert_eq!(block.to - block.from, 3);
    assert_eq!(
        block.doc_position(block.text_offset(block.from + 2)),
        block.from + 2
    );
}

#[test]
fn a_hard_break_is_a_newline_in_both_coordinate_systems() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("one"),
            b.node(nodes::HARD_BREAK, nodes![]),
            b.text("two"),
        ]
    )]);
    let block = &flatten(&doc)[0];
    assert_eq!(block.text, "one\ntwo");
    assert_eq!(block.line_count(), 2);
    assert_eq!(block.line_start(1), 4);
    assert_eq!(block.line_of(5), (1, 1));
    assert_eq!(block.line_length(0), 3);
}

#[test]
fn text_offsets_are_bytes_so_multibyte_characters_do_not_shift_anything() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("γειά")])]);
    let block = &flatten(&doc)[0];
    assert_eq!(block.text.len(), 8);
    assert_eq!(block.to - block.from, 8);
    assert_eq!(block.doc_position(4), block.from + 4);
}

// ---------------------------------------------------------------------------
// The caret
// ---------------------------------------------------------------------------

fn rect(x: f32) -> cosmic::iced::Rectangle {
    cosmic::iced::Rectangle {
        x,
        y: 0.0,
        width: 1.5,
        height: 20.0,
    }
}

#[test]
fn a_caret_glides_to_a_nearby_position() {
    let start = Instant::now();
    let glide = Duration::from_millis(100);
    let mut caret = nib::caret::Animation::new(start);

    caret.aim(Some(rect(0.0)), start, glide);
    caret.tick(start, glide);
    assert_eq!(caret.visible(start, Duration::ZERO).map(|r| r.x), Some(0.0));

    // Aiming somewhere close starts a glide rather than a jump.
    caret.aim(Some(rect(20.0)), start, glide);
    let midway = start + Duration::from_millis(50);
    assert!(caret.tick(midway, glide), "still moving");
    let x = caret.visible(midway, Duration::ZERO).map(|r| r.x).unwrap();
    assert!(x > 0.0 && x < 20.0, "caret should be between the two: {x}");

    let after = start + Duration::from_millis(150);
    assert!(!caret.tick(after, glide), "arrived");
    assert_eq!(
        caret.visible(after, Duration::ZERO).map(|r| r.x),
        Some(20.0)
    );
}

#[test]
fn a_caret_jumps_rather_than_crawling_across_a_document() {
    let start = Instant::now();
    let glide = Duration::from_millis(100);
    let mut caret = nib::caret::Animation::new(start);
    caret.aim(Some(rect(0.0)), start, glide);
    caret.tick(start, glide);

    caret.aim(Some(rect(4000.0)), start, glide);
    assert_eq!(
        caret.visible(start, Duration::ZERO).map(|r| r.x),
        Some(4000.0),
        "beyond the glide limit it appears"
    );
}

#[test]
fn a_caret_stays_solid_just_after_it_moves() {
    let start = Instant::now();
    let period = Duration::from_millis(1000);
    let mut caret = nib::caret::Animation::new(start);
    caret.aim(Some(rect(0.0)), start, Duration::ZERO);
    caret.tick(start, Duration::ZERO);

    // Half a second in, a blinking caret would be dark; this one is not,
    // because it was just touched.
    let soon = start + Duration::from_millis(499);
    assert!(caret.visible(soon, period).is_some());
}

#[test]
fn a_caret_blinks_once_it_has_settled() {
    let start = Instant::now();
    let period = Duration::from_millis(1000);
    let mut caret = nib::caret::Animation::new(start);
    caret.aim(Some(rect(0.0)), start, Duration::ZERO);
    caret.tick(start, Duration::ZERO);

    // The first half-period coincides with the grace after a move, so the
    // caret is lit from 0 to 500ms, dark from 500 to 1000, and lit again after.
    assert!(
        caret
            .visible(start + Duration::from_millis(200), period)
            .is_some()
    );
    assert!(
        caret
            .visible(start + Duration::from_millis(600), period)
            .is_none()
    );
    assert!(
        caret
            .visible(start + Duration::from_millis(1100), period)
            .is_some()
    );
}

#[test]
fn a_caret_that_does_not_blink_is_always_lit() {
    let start = Instant::now();
    let mut caret = nib::caret::Animation::new(start);
    caret.aim(Some(rect(0.0)), start, Duration::ZERO);
    caret.tick(start, Duration::ZERO);
    for after in [10, 600, 1100, 5000] {
        let at = start + Duration::from_millis(after);
        assert!(caret.visible(at, Duration::ZERO).is_some(), "at {after}ms");
    }
}

#[test]
fn the_next_blink_is_scheduled_rather_than_polled() {
    let start = Instant::now();
    let period = Duration::from_millis(1000);
    let mut caret = nib::caret::Animation::new(start);
    caret.aim(Some(rect(0.0)), start, Duration::ZERO);
    caret.tick(start, Duration::ZERO);

    let next = caret
        .next_blink(start + Duration::from_millis(600), period)
        .expect("a blinking caret has a next edge");
    assert!(next > start + Duration::from_millis(600));
    assert!(next <= start + Duration::from_millis(1100));
}

// ---------------------------------------------------------------------------
// Caret shapes
// ---------------------------------------------------------------------------

#[test]
fn each_caret_shape_occupies_what_it_should() {
    use nib::Caret;

    let line = rect(10.0);
    let thin = nib::caret::rectangle(Caret::Line, line, 14.0, None);
    assert!(thin.width < 3.0);
    assert!((thin.height - line.height).abs() < f32::EPSILON);

    let block = nib::caret::rectangle(Caret::Block, line, 14.0, Some(9.0));
    assert!(
        (block.width - 9.0).abs() < f32::EPSILON,
        "a block covers the next character"
    );
    assert!((block.height - line.height).abs() < f32::EPSILON);

    let under = nib::caret::rectangle(Caret::Underline, line, 14.0, Some(9.0));
    assert!(
        under.height < line.height,
        "an underline is a rule, not a bar"
    );
    assert!(under.y > line.y);
}

// ---------------------------------------------------------------------------
// The placeholder
// ---------------------------------------------------------------------------

/// What [`nib::Editor::placeholder`] tests before it draws.
///
/// The placeholder shows while the document is empty, and "empty" cannot mean
/// "no blocks": the top node's content expression is `block+`, so a document
/// holds at least one. Empty is *one* block with no text in it, and this is the
/// property the draw depends on — a change to the empty document's shape would
/// otherwise silently stop the placeholder appearing.
#[test]
fn an_empty_document_is_one_block_with_no_text() {
    let found = flatten(&basic::schema().empty_doc());
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].text.is_empty());
}

/// And a document with anything in it is not that, so the placeholder goes
/// away as soon as the first character arrives.
#[test]
fn a_document_with_one_character_no_longer_looks_empty() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("x")])]);
    let found = flatten(&doc);
    assert_eq!(found.len(), 1);
    assert!(!found[0].text.is_empty());
}

// ---------------------------------------------------------------------------
// Authored colour, and whether it can be seen
// ---------------------------------------------------------------------------

use cosmic::iced::Color;
use nib::style::{contrast, legible};

const WHITE: Color = Color::WHITE;
const BLACK: Color = Color::BLACK;

#[test]
fn contrast_is_the_wcag_ratio_at_both_extremes() {
    assert!((contrast(BLACK, WHITE) - 21.0).abs() < 0.01);
    assert!((contrast(WHITE, WHITE) - 1.0).abs() < 0.01);
}

#[test]
fn white_on_white_cannot_be_drawn() {
    // The oldest trick in hostile mail: text the recipient cannot see, in a
    // message that reads innocently, put there for whatever is scanning it.
    // Closed by construction — there is no list of suspicious colours to keep
    // current, because the check is on the result rather than the phrasing.
    let fallback = BLACK;
    assert_eq!(legible(WHITE, WHITE, fallback), fallback);
}

#[test]
fn a_colour_just_short_of_the_background_cannot_be_drawn_either() {
    // `#fefefe` on `#ffffff` is 1.01:1 — invisible, and not white.
    let nearly = Color::from_rgb8(0xfe, 0xfe, 0xfe);
    assert_eq!(legible(nearly, WHITE, BLACK), BLACK);
}

#[test]
fn transparency_is_not_a_way_around_it() {
    // Asking for low contrast without naming a low-contrast colour. The alpha
    // is resolved against the ground before the ratio is taken.
    let ghost = Color { a: 0.02, ..BLACK };
    assert_eq!(legible(ghost, WHITE, BLACK), BLACK);
}

#[test]
fn a_colour_that_can_be_read_is_kept() {
    // The reader is showing the message, not rewriting it.
    let red = Color::from_rgb8(0xcc, 0, 0);
    let shown = legible(red, WHITE, BLACK);
    assert!((shown.r - red.r).abs() < f32::EPSILON, "{shown:?}");
    assert!(shown.g.abs() < f32::EPSILON);
}

#[test]
fn the_check_is_against_the_ground_not_against_the_theme() {
    // White text is unreadable on white and perfectly readable on black, and
    // the same call has to answer both — which is why the background is a
    // parameter rather than a constant.
    assert_eq!(legible(WHITE, WHITE, BLACK), BLACK);
    let on_black = legible(WHITE, BLACK, BLACK);
    assert!((on_black.r - 1.0).abs() < f32::EPSILON, "{on_black:?}");
}

#[test]
fn alignment_is_absent_until_the_author_says_otherwise() {
    use cosmic::iced::advanced::text::Alignment;

    let b = b();
    let plain = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("x")])]);
    assert!(matches!(
        nib::style::alignment(&flatten(&plain)[0]),
        Alignment::Default
    ));

    let centred = b.doc(nodes![b.attr_node(
        nodes::PARAGRAPH,
        &attrs! { "align" => "center" },
        nodes![b.text("x")],
    )]);
    assert!(matches!(
        nib::style::alignment(&flatten(&centred)[0]),
        Alignment::Center
    ));
}
