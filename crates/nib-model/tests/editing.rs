// SPDX-License-Identifier: MPL-2.0

//! The editing layer, driven the way a user drives it: a state, a key, a new
//! state.

use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::commands as cmd;
use nib_model::history::{self, History};
use nib_model::keymap::{Binding, Key, Keymap, Mods};
use nib_model::node::Node;
use nib_model::schema::Schema;
use nib_model::state::{EditorState, Selection};
use nib_model::{attrs, nodes};

fn schema() -> Schema {
    basic::schema()
}

fn b() -> Builder {
    Builder::new(schema())
}

fn id(name: &str) -> usize {
    schema().node_id(name).unwrap()
}

fn mark_id(name: &str) -> usize {
    schema().mark_id(name).unwrap()
}

/// A state over `doc` with the caret at `pos` and the history installed.
fn state_at(doc: Node, pos: usize) -> EditorState {
    EditorState::with_selection(
        schema(),
        doc,
        Selection::cursor(pos),
        vec![history::history(history::Options::default())],
    )
}

/// Runs a command and returns the new state, or the old one if it did not
/// apply.
fn run(state: &EditorState, command: &cmd::Command) -> EditorState {
    command(state).map_or_else(|| state.clone(), |tr| state.applied(tr))
}

fn press(state: &EditorState, map: &Keymap, binding: &Binding) -> EditorState {
    map.handle(state, binding)
        .map_or_else(|| state.clone(), |tr| state.applied(tr))
}

fn one_paragraph(text: &str) -> Node {
    let b = b();
    b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])])
}

fn a_list(items: &[&str]) -> Node {
    let b = b();
    let item = |text: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    b.doc(nodes![b.node(
        nodes::BULLET_LIST,
        items.iter().map(|t| item(t)).collect::<Vec<_>>()
    )])
}

// ---------------------------------------------------------------------------
// Typing
// ---------------------------------------------------------------------------

#[test]
fn typing_inserts_at_the_caret_and_moves_it() {
    let state = state_at(one_paragraph("helo"), 4);
    let mut tr = state.tr();
    tr.insert_text("l").unwrap();
    let state = state.applied(tr);
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("hello"))"#);
    assert_eq!(state.selection().cursor_pos(), Some(5));
}

#[test]
fn typing_over_a_selection_replaces_it() {
    let doc = one_paragraph("hello world");
    let state = EditorState::with_selection(
        schema(),
        doc,
        Selection::text(1, 6),
        Vec::new(),
    );
    let mut tr = state.tr();
    tr.insert_text("goodbye").unwrap();
    let state = state.applied(tr);
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("goodbye world"))"#);
    assert_eq!(state.selection().cursor_pos(), Some(8));
}

// ---------------------------------------------------------------------------
// Enter
// ---------------------------------------------------------------------------

#[test]
fn enter_splits_a_paragraph_and_leaves_the_caret_in_the_second() {
    let state = state_at(one_paragraph("hello world"), 6);
    let state = run(&state, &cmd::split_block());
    assert_eq!(
        state.doc().to_string(),
        r#"doc(paragraph("hello"), paragraph(" world"))"#
    );
    assert_eq!(state.selection().cursor_pos(), Some(8), "in the second block");
}

#[test]
fn enter_at_the_end_of_a_heading_starts_a_paragraph() {
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        &attrs! { "level" => 2_i64 },
        nodes![b.text("Title")]
    )]);
    let state = state_at(doc, 6);
    let state = run(&state, &cmd::split_block());
    assert_eq!(
        state.doc().to_string(),
        r#"doc(heading[level=2]("Title"), paragraph)"#
    );
}

#[test]
fn enter_in_a_list_item_makes_another_item() {
    let state = state_at(a_list(&["one"]), 6);
    let state = run(&state, &cmd::split_list_item(id(nodes::LIST_ITEM)));
    assert_eq!(
        state.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("one")), list_item(paragraph)))"#
    );
    assert_eq!(state.doc().check(), Ok(()));
}

#[test]
fn enter_in_an_empty_list_item_leaves_the_list() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BULLET_LIST,
        nodes![
            b.node(
                nodes::LIST_ITEM,
                nodes![b.node(nodes::PARAGRAPH, nodes![b.text("one")])]
            ),
            b.node(
                nodes::LIST_ITEM,
                nodes![b.node(nodes::PARAGRAPH, nodes![])]
            ),
        ]
    )]);
    // Inside the empty second item's paragraph.
    let pos = doc.content_size() - 3;
    let state = state_at(doc, pos);
    // The real Enter binding: split the item first, and fall through to
    // lifting when the item is empty.
    let enter = cmd::chain(vec![
        cmd::split_list_item(id(nodes::LIST_ITEM)),
        cmd::lift_empty_block(),
        cmd::split_block(),
    ]);
    let state = run(&state, &enter);
    assert_eq!(state.doc().check(), Ok(()));
    assert_eq!(
        state.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("one"))), paragraph)"#
    );
}

// ---------------------------------------------------------------------------
// Backspace
// ---------------------------------------------------------------------------

#[test]
fn backspace_at_the_start_of_a_paragraph_joins_it_to_the_one_before() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    let state = state_at(doc, 6);
    let state = run(&state, &cmd::delete_backward());
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("onetwo"))"#);
}

#[test]
fn backspace_at_the_start_of_a_quote_unquotes_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("quoted")])]
    )]);
    let state = state_at(doc, 2);
    let state = run(&state, &cmd::delete_backward());
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("quoted"))"#);
}

#[test]
fn backspace_with_a_selection_removes_the_selection() {
    let doc = one_paragraph("hello world");
    let state = EditorState::with_selection(schema(), doc, Selection::text(1, 7), Vec::new());
    let state = run(&state, &cmd::delete_backward());
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("world"))"#);
}

// ---------------------------------------------------------------------------
// Marks
// ---------------------------------------------------------------------------

#[test]
fn toggling_bold_on_a_caret_makes_the_next_typing_bold() {
    let state = state_at(one_paragraph("plain "), 7);
    let state = run(&state, &cmd::toggle_mark(mark_id(marks::STRONG), None));
    assert_eq!(
        state.doc().to_string(),
        r#"doc(paragraph("plain "))"#,
        "toggling on a caret changes no text"
    );
    assert!(state.stored_marks().is_some_and(|m| m.len() == 1));

    let mut tr = state.tr();
    tr.insert_text("bold").unwrap();
    let state = state.applied(tr);
    assert_eq!(
        state.doc().to_string(),
        r#"doc(paragraph("plain ", strong("bold")))"#
    );
}

#[test]
fn toggling_bold_on_a_selection_applies_and_removes_it() {
    let doc = one_paragraph("hello");
    let state = EditorState::with_selection(schema(), doc, Selection::text(1, 6), Vec::new());
    let bold = cmd::toggle_mark(mark_id(marks::STRONG), None);

    let state = run(&state, &bold);
    assert_eq!(state.doc().to_string(), r#"doc(paragraph(strong("hello")))"#);

    let state = run(&state, &bold);
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("hello"))"#);
}

#[test]
fn bold_does_not_apply_inside_a_code_block() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("code")])]);
    let state = EditorState::with_selection(schema(), doc, Selection::text(1, 5), Vec::new());
    assert!(
        cmd::toggle_mark(mark_id(marks::STRONG), None)(&state).is_none(),
        "the command should refuse, not produce an empty transaction"
    );
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn a_paragraph_becomes_a_list() {
    let state = state_at(one_paragraph("item"), 2);
    let state = run(&state, &cmd::wrap_in_list(id(nodes::BULLET_LIST), None));
    assert_eq!(
        state.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("item"))))"#
    );
}

#[test]
fn tab_indents_a_list_item_and_shift_tab_outdents_it() {
    let doc = a_list(&["one", "two"]);
    // Inside the second item.
    let state = state_at(doc, 11);
    let sunk = run(&state, &cmd::sink_list_item(id(nodes::LIST_ITEM)));
    assert_eq!(sunk.doc().check(), Ok(()));
    assert_eq!(
        sunk.doc().to_string(),
        concat!(
            r#"doc(bullet_list(list_item(paragraph("one"), "#,
            r#"bullet_list(list_item(paragraph("two"))))))"#
        )
    );

    let lifted = run(&sunk, &cmd::lift_list_item(id(nodes::LIST_ITEM)));
    assert_eq!(lifted.doc().check(), Ok(()));
    assert_eq!(lifted.doc().to_string(), a_list(&["one", "two"]).to_string());
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[test]
fn undo_takes_back_a_change_and_redo_puts_it_forward() {
    let state = state_at(one_paragraph("hello"), 6);
    let before = state.doc().clone();

    let mut tr = state.tr().at(1000);
    tr.insert_text(" world").unwrap();
    let typed = state.applied(tr);
    assert_eq!(typed.doc().to_string(), r#"doc(paragraph("hello world"))"#);

    let undone = run(&typed, &cmd::undo());
    assert_eq!(undone.doc(), &before);

    let redone = run(&undone, &cmd::redo());
    assert_eq!(redone.doc().to_string(), r#"doc(paragraph("hello world"))"#);
}

#[test]
fn typing_in_quick_succession_is_one_undo() {
    let mut state = state_at(one_paragraph(""), 1);
    for (i, ch) in "hello".chars().enumerate() {
        let mut tr = state.tr().at(1000 + i as u64 * 50);
        tr.insert_text(&ch.to_string()).unwrap();
        state = state.applied(tr);
    }
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("hello"))"#);
    assert_eq!(
        state.plugin_state::<History>(history::KEY).unwrap().undo_depth(),
        1,
        "five keystrokes half a second apart are one event"
    );

    let undone = run(&state, &cmd::undo());
    assert_eq!(undone.doc().to_string(), r"doc(paragraph)");
}

#[test]
fn a_pause_starts_a_new_undo_event() {
    let mut state = state_at(one_paragraph(""), 1);
    for (i, word) in ["hello", " there"].iter().enumerate() {
        // Two seconds apart: well past the grouping delay.
        let mut tr = state.tr().at(1000 + i as u64 * 2000);
        tr.insert_text(word).unwrap();
        state = state.applied(tr);
    }
    assert_eq!(
        state.plugin_state::<History>(history::KEY).unwrap().undo_depth(),
        2
    );
    let undone = run(&state, &cmd::undo());
    assert_eq!(undone.doc().to_string(), r#"doc(paragraph("hello"))"#);
}

#[test]
fn undo_restores_the_selection_the_change_started_from() {
    let state = state_at(one_paragraph("hello"), 3);
    let mut tr = state.tr().at(1000);
    tr.insert_text("XYZ").unwrap();
    let typed = state.applied(tr);
    assert_eq!(typed.selection().cursor_pos(), Some(6));

    let undone = run(&typed, &cmd::undo());
    assert_eq!(undone.selection().cursor_pos(), Some(3));
}

#[test]
fn a_change_marked_out_of_history_is_not_undone_but_is_followed() {
    let state = state_at(one_paragraph("hello"), 6);

    let mut tr = state.tr().at(1000);
    tr.insert_text("!").unwrap();
    let state = state.applied(tr);

    // Something else edits the document — a collaborator, say.
    let mut remote = state.tr().at(1100);
    remote.insert_text("[").unwrap();
    let state = state.applied(remote.set_meta(history::ADD_TO_HISTORY, false));
    assert_eq!(state.doc().to_string(), r#"doc(paragraph("hello!["))"#);

    // Undo takes back our "!" and leaves theirs alone.
    let undone = run(&state, &cmd::undo());
    assert_eq!(undone.doc().to_string(), r#"doc(paragraph("hello["))"#);
}

// ---------------------------------------------------------------------------
// The keymap
// ---------------------------------------------------------------------------

#[test]
fn the_default_keymap_binds_the_keys_an_editor_has() {
    let map = Keymap::base(&schema());
    for binding in [
        Binding::plain(Key::Enter),
        Binding::plain(Key::Backspace),
        Binding::primary(Key::Char('b')),
        Binding::primary(Key::Char('z')),
        Binding::plain(Key::Tab),
    ] {
        assert!(map.command(&binding).is_some(), "{binding:?} should be bound");
    }
}

#[test]
fn pressing_ctrl_b_over_a_selection_bolds_it() {
    let map = Keymap::base(&schema());
    let doc = one_paragraph("hello");
    let state = EditorState::with_selection(schema(), doc, Selection::text(1, 6), Vec::new());
    let state = press(&state, &map, &Binding::primary(Key::Char('b')));
    assert_eq!(state.doc().to_string(), r#"doc(paragraph(strong("hello")))"#);
}

#[test]
fn pressing_enter_splits_the_block() {
    let map = Keymap::base(&schema());
    let state = state_at(one_paragraph("hello world"), 6);
    let state = press(&state, &map, &Binding::plain(Key::Enter));
    assert_eq!(
        state.doc().to_string(),
        r#"doc(paragraph("hello"), paragraph(" world"))"#
    );
}

#[test]
fn an_unbound_key_changes_nothing() {
    let map = Keymap::base(&schema());
    let state = state_at(one_paragraph("hello"), 3);
    let after = press(
        &state,
        &map,
        &Binding::new(Key::Char('q'), Mods::PRIMARY.union(Mods::ALT)),
    );
    assert_eq!(after.doc(), state.doc());
}
