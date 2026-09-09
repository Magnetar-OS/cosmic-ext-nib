// SPDX-License-Identifier: MPL-2.0

//! Input rules and decorations.

use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::decoration::{Decoration, DecorationSet, Style};
use nib_model::input_rules::{self, InputRule};
use nib_model::node::Node;
use nib_model::schema::Schema;
use nib_model::state::{EditorState, Selection};
use nib_model::transform::{Mapping, StepMap};
use nib_model::nodes;

fn schema() -> Schema {
    basic::schema()
}

fn b() -> Builder {
    Builder::new(schema())
}

fn state_at(doc: Node, pos: usize) -> EditorState {
    EditorState::with_selection(
        schema(),
        doc,
        Selection::cursor(pos),
        vec![input_rules::input_rules_plugin()],
    )
}

/// Types `text` at the caret, letting the rules have first refusal.
fn type_text(state: &EditorState, rules: &[InputRule], text: &str) -> EditorState {
    let (from, to) = (state.selection().from(), state.selection().to());
    if let Some(tr) = input_rules::apply(state, rules, from, to, text) {
        return state.applied(tr);
    }
    let mut tr = state.tr();
    tr.insert_text(text).unwrap();
    state.applied(tr)
}

fn one_paragraph(text: &str) -> Node {
    let b = b();
    if text.is_empty() {
        return b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![])]);
    }
    b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])])
}

// ---------------------------------------------------------------------------
// Input rules
// ---------------------------------------------------------------------------

#[test]
fn hash_space_makes_a_heading() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("#"), 2);
    let state = type_text(&state, &rules, " ");
    assert_eq!(state.doc().to_string(), r#"doc(heading[level=1])"#);
}

#[test]
fn three_hashes_make_a_level_three_heading() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("###"), 4);
    let state = type_text(&state, &rules, " ");
    assert_eq!(state.doc().to_string(), r#"doc(heading[level=3])"#);
}

#[test]
fn a_dash_and_a_space_make_a_list() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("-"), 2);
    let state = type_text(&state, &rules, " ");
    assert_eq!(
        state.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph)))"#
    );
    assert_eq!(state.doc().check(), Ok(()));
}

#[test]
fn a_greater_than_and_a_space_make_a_quote() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph(">"), 2);
    let state = type_text(&state, &rules, " ");
    assert_eq!(state.doc().to_string(), r#"doc(blockquote(paragraph))"#);
}

#[test]
fn backticks_make_a_code_block_with_its_language() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("```rust"), 8);
    let state = type_text(&state, &rules, " ");
    assert_eq!(
        state.doc().to_string(),
        r#"doc(code_block[language=rust])"#
    );
}

#[test]
fn double_stars_make_the_text_between_them_bold() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("**bold*"), 8);
    let state = type_text(&state, &rules, "*");
    assert_eq!(state.doc().to_string(), r#"doc(paragraph(strong("bold")))"#);
}

#[test]
fn backticks_around_text_make_it_code() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("`x"), 3);
    let state = type_text(&state, &rules, "`");
    assert_eq!(state.doc().to_string(), r#"doc(paragraph(code("x")))"#);
}

#[test]
fn two_hyphens_become_an_em_dash() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("a-"), 3);
    let state = type_text(&state, &rules, "-");
    assert_eq!(state.doc().text_content(), "a\u{2014}");
}

#[test]
fn three_dots_become_an_ellipsis() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph(".."), 3);
    let state = type_text(&state, &rules, ".");
    assert_eq!(state.doc().text_content(), "\u{2026}");
}

#[test]
fn no_rule_fires_inside_a_code_block() {
    let rules = input_rules::base(&schema());
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("x-")])]);
    let state = state_at(doc, 3);
    let state = type_text(&state, &rules, "-");
    assert_eq!(
        state.doc().text_content(),
        "x--",
        "an em dash in a shell command is not what anyone meant"
    );
}

#[test]
fn ordinary_typing_is_untouched() {
    let rules = input_rules::base(&schema());
    let state = state_at(one_paragraph("hello"), 6);
    let state = type_text(&state, &rules, "!");
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("hello!"))"#);
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

#[test]
fn a_table_is_a_valid_document() {
    let b = b();
    let cell = |text: &str| {
        b.node(
            nodes::TABLE_CELL,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![
            b.node(nodes::TABLE_ROW, nodes![cell("a"), cell("b")]),
            b.node(nodes::TABLE_ROW, nodes![cell("c"), cell("d")]),
        ]
    )]);
    assert_eq!(doc.check(), Ok(()));
    assert_eq!(doc.text_content(), "abcd");
}

#[test]
fn backspace_does_not_reach_across_a_cell_boundary() {
    let b = b();
    let cell = |text: &str| {
        b.node(
            nodes::TABLE_CELL,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![b.node(nodes::TABLE_ROW, nodes![cell("a"), cell("b")])]
    )]);
    // The start of the second cell's paragraph: 0 <table> 1 <row> 2 <cell>
    // 3 <p> 4 "a" 5 </p> 6 </cell> 7 <cell> 8 <p> 9 …
    let state = state_at(doc, 9);
    assert!(
        nib_model::commands::delete_backward()(&state).is_none(),
        "isolation is the schema saying a grid is not nested prose"
    );
}

#[test]
fn backspace_inside_one_cell_still_joins_its_blocks() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![b.node(
            nodes::TABLE_ROW,
            nodes![b.node(
                nodes::TABLE_CELL,
                nodes![
                    b.node(nodes::PARAGRAPH, nodes![b.text("a")]),
                    b.node(nodes::PARAGRAPH, nodes![b.text("b")]),
                ]
            )]
        )]
    )]);
    // 0 <table> 1 <row> 2 <cell> 3 <p> 4 "a" 5 </p> 6 <p> 7 "b" …
    let state = state_at(doc, 7);
    let tr = nib_model::commands::delete_backward()(&state)
        .expect("within a cell, ordinary rules apply");
    let after = state.applied(tr);
    assert_eq!(
        after.doc().to_string(),
        r#"doc(table(table_row(table_cell[colspan=1, rowspan=1](paragraph("ab")))))"#
    );
}

// ---------------------------------------------------------------------------
// Decorations
// ---------------------------------------------------------------------------

#[test]
fn a_decoration_moves_with_the_text_it_describes() {
    let decoration = Decoration::inline(5, 10, Style::class("keyword"));
    // Two characters inserted at the front.
    let mapping = Mapping::from_maps(vec![StepMap::single(0, 0, 2)]);
    let moved = decoration.map(&mapping).expect("it still describes something");
    assert_eq!((moved.from(), moved.to()), (7, 12));
}

#[test]
fn a_decoration_whose_content_is_gone_goes_with_it() {
    let decoration = Decoration::inline(5, 10, Style::class("keyword"));
    let mapping = Mapping::from_maps(vec![StepMap::single(4, 8, 0)]);
    assert_eq!(decoration.map(&mapping), None);
}

#[test]
fn a_set_finds_what_overlaps_a_block() {
    let set = DecorationSet::new(vec![
        Decoration::inline(0, 5, Style::class("a")),
        Decoration::inline(10, 15, Style::class("b")),
        Decoration::inline(20, 25, Style::class("c")),
    ]);
    let hits: Vec<&str> = set
        .in_range(9, 16)
        .filter_map(|d| d.style()?.class.as_deref())
        .collect();
    assert_eq!(hits, ["b"]);
}

#[test]
fn a_widget_takes_a_side_when_text_lands_on_it() {
    let before = Decoration::widget(5, "marker", -1);
    let after = Decoration::widget(5, "marker", 1);
    let mapping = Mapping::from_maps(vec![StepMap::single(5, 0, 3)]);
    assert_eq!(before.map(&mapping).unwrap().from(), 5);
    assert_eq!(after.map(&mapping).unwrap().from(), 8);
}
