// SPDX-License-Identifier: MPL-2.0

//! Modal editing, driven the way a user drives it: keys in, document out.

use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::keymap::{Binding, Key, Mods};
use nib_model::node::Node;
use nib_model::state::{EditorState, Selection};
use nib_model::vim::{Mode, Response, Vim};
use nib_model::{history, motion, nodes};

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn paragraphs(texts: &[&str]) -> Node {
    let b = b();
    b.doc(
        texts
            .iter()
            .map(|t| b.node(nodes::PARAGRAPH, nodes![b.text(t)]))
            .collect::<Vec<_>>(),
    )
}

fn state_at(doc: Node, pos: usize) -> EditorState {
    EditorState::with_selection(
        basic::schema(),
        doc,
        Selection::cursor(pos),
        vec![history::history(history::Options::default())],
    )
}

/// Types a run of keys, one per character, and returns where they left things.
fn keys(state: &EditorState, vim: &mut Vim, input: &str) -> EditorState {
    let mut state = state.clone();
    for c in input.chars() {
        let binding = Binding::plain(Key::Char(c));
        if let Response::Apply(tr) = vim.key(&state, &binding) {
            state = state.applied(*tr);
        }
    }
    state
}

/// The document's top-level blocks, joined by a bar — so an empty block is
/// visible rather than silently absent.
fn text(state: &EditorState) -> String {
    state
        .doc()
        .content()
        .iter()
        .map(nib_model::node::Node::text_content)
        .collect::<Vec<_>>()
        .join("|")
}

// -- motions ---------------------------------------------------------------

#[test]
fn h_and_l_stay_inside_their_block() {
    let doc = paragraphs(&["ab", "cd"]);
    let mut vim = Vim::new(&basic::schema());
    // Paragraph one spans 1..3.
    let state = state_at(doc, 1);
    let moved = keys(&state, &mut vim, "hhh");
    assert_eq!(moved.selection().head(), 1, "h stops at the block start");

    let state = state_at(paragraphs(&["ab", "cd"]), 1);
    let moved = keys(&state, &mut vim, "lllll");
    assert_eq!(moved.selection().head(), 3, "l stops at the block end");
}

#[test]
fn w_and_b_step_over_words() {
    let doc = paragraphs(&["one two three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "w");
    assert_eq!(after.selection().head(), 5, "w lands on `two`");
    let after = keys(&after, &mut vim, "w");
    assert_eq!(after.selection().head(), 9, "w lands on `three`");
    let after = keys(&after, &mut vim, "b");
    assert_eq!(after.selection().head(), 5, "b comes back");
}

#[test]
fn e_lands_on_the_end_of_a_word() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "e");
    assert_eq!(after.selection().head(), 3, "on the `e`, not past it");
}

#[test]
fn a_count_multiplies_a_motion() {
    let doc = paragraphs(&["one two three four"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "3w");
    assert_eq!(after.selection().head(), 15, "three words along, at `four`");
}

#[test]
fn dollar_and_caret_find_the_ends_of_a_line() {
    let doc = paragraphs(&["  indented"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 5);
    assert_eq!(keys(&state, &mut vim, "$").selection().head(), 11);
    assert_eq!(keys(&state, &mut vim, "^").selection().head(), 3);
    assert_eq!(keys(&state, &mut vim, "0").selection().head(), 1);
}

#[test]
fn gg_and_g_reach_the_ends_of_a_document() {
    let doc = paragraphs(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 7);
    assert_eq!(
        keys(&state, &mut vim, "gg").selection().head(),
        motion::document_start(state.doc())
    );
    assert_eq!(
        keys(&state, &mut vim, "G").selection().head(),
        motion::document_end(state.doc())
    );
    let second = keys(&state, &mut vim, "2G");
    assert_eq!(second.selection().head(), 6, "2G is the second block");
}

// -- modes -----------------------------------------------------------------

#[test]
fn i_enters_insert_and_escape_leaves_it() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 2);
    let _ = keys(&state, &mut vim, "i");
    assert_eq!(vim.mode(), Mode::Insert);
    let out = match vim.key(&state, &Binding::plain(Key::Escape)) {
        Response::Apply(tr) => state.applied(*tr),
        _ => state.clone(),
    };
    assert_eq!(vim.mode(), Mode::Normal);
    assert_eq!(out.selection().head(), 1, "escape steps back one");
}

#[test]
fn normal_mode_never_types_into_the_document() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    for c in "qzZ".chars() {
        assert!(
            matches!(
                vim.key(&state, &Binding::plain(Key::Char(c))),
                Response::Consumed
            ),
            "`{c}` is swallowed rather than inserted"
        );
    }
}

#[test]
fn insert_mode_passes_typing_through() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let state = keys(&state, &mut vim, "i");
    assert!(matches!(
        vim.key(&state, &Binding::plain(Key::Char('x'))),
        Response::Pass
    ));
}

#[test]
fn a_appends_after_the_caret() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "a");
    assert_eq!(vim.mode(), Mode::Insert);
    assert_eq!(after.selection().head(), 2);
}

#[test]
fn capital_a_goes_to_the_end_of_the_line() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "A");
    assert_eq!(after.selection().head(), 4);
    assert_eq!(vim.mode(), Mode::Insert);
}

// -- operators -------------------------------------------------------------

#[test]
fn dw_deletes_a_word() {
    let doc = paragraphs(&["one two three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "dw");
    assert_eq!(text(&after), "two three");
}

#[test]
fn d_dollar_deletes_to_the_end_of_the_line() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 4);
    let after = keys(&state, &mut vim, "d$");
    assert_eq!(text(&after), "one");
}

#[test]
fn de_takes_the_last_character_of_the_word() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "de");
    assert_eq!(text(&after), " two", "inclusive: the `e` goes too");
}

#[test]
fn dd_deletes_a_whole_block() {
    let doc = paragraphs(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 6);
    let after = keys(&state, &mut vim, "dd");
    assert_eq!(text(&after), "one|three");
}

#[test]
fn a_count_before_dd_takes_several_blocks() {
    let doc = paragraphs(&["one", "two", "three", "four"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "3dd");
    assert_eq!(text(&after), "four");
}

#[test]
fn a_count_between_operator_and_motion_multiplies() {
    let doc = paragraphs(&["one two three four"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "d3w");
    assert_eq!(text(&after), "four");
}

#[test]
fn cw_deletes_and_enters_insert() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "cw");
    assert_eq!(text(&after), "two");
    assert_eq!(vim.mode(), Mode::Insert);
}

#[test]
fn cc_empties_the_line_but_keeps_it() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "cc");
    assert_eq!(text(&after), "|two", "the block survives, empty");
    assert_eq!(vim.mode(), Mode::Insert);
}

#[test]
fn x_deletes_the_character_under_the_caret() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    assert_eq!(text(&keys(&state, &mut vim, "x")), "bc");
    assert_eq!(text(&keys(&state, &mut vim, "2x")), "c");
}

#[test]
fn x_stops_at_the_end_of_a_block() {
    let doc = paragraphs(&["ab", "cd"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 2);
    let after = keys(&state, &mut vim, "5x");
    assert_eq!(text(&after), "a|cd", "it never eats the block boundary");
}

#[test]
fn capital_d_deletes_to_the_end_of_the_line() {
    let doc = paragraphs(&["one two", "next"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 4);
    assert_eq!(text(&keys(&state, &mut vim, "D")), "one|next");
}

// -- yank and put ----------------------------------------------------------

#[test]
fn yy_and_p_duplicate_a_block() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "yyp");
    assert_eq!(text(&after), "one|one|two");
}

#[test]
fn dd_then_p_moves_a_block_down() {
    let doc = paragraphs(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "ddp");
    assert_eq!(text(&after), "two|one|three");
}

#[test]
fn charwise_p_pastes_after_the_caret() {
    let doc = paragraphs(&["abcd"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "ylp");
    assert_eq!(text(&after), "aabcd");
}

#[test]
fn capital_p_pastes_before() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 6);
    let after = keys(&state, &mut vim, "yyP");
    assert_eq!(text(&after), "one|two|two");
}

// -- text objects ----------------------------------------------------------

#[test]
fn diw_deletes_the_word_under_the_caret() {
    let doc = paragraphs(&["one two three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 6);
    let after = keys(&state, &mut vim, "diw");
    assert_eq!(text(&after), "one  three");
}

#[test]
fn daw_takes_the_space_with_it() {
    let doc = paragraphs(&["one two three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 6);
    let after = keys(&state, &mut vim, "daw");
    assert_eq!(text(&after), "one three");
}

#[test]
fn dip_empties_the_block() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "dip");
    assert_eq!(text(&after), "|two");
}

// -- opening lines ---------------------------------------------------------

#[test]
fn o_opens_a_line_below() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "o");
    assert_eq!(text(&after), "one||two");
    assert_eq!(vim.mode(), Mode::Insert);
    assert_eq!(after.selection().head(), 6, "the caret is in the new block");
}

#[test]
fn capital_o_opens_a_line_above() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "O");
    assert_eq!(text(&after), "|one|two");
    assert_eq!(after.selection().head(), 1);
}

// -- visual ----------------------------------------------------------------

#[test]
fn visual_mode_extends_a_selection() {
    let doc = paragraphs(&["abcdef"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "vll");
    assert_eq!(vim.mode(), Mode::Visual);
    assert_eq!((after.selection().from(), after.selection().to()), (1, 3));
}

#[test]
fn visual_d_deletes_the_selection_inclusively() {
    let doc = paragraphs(&["abcdef"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "vlld");
    assert_eq!(text(&after), "def", "abc goes: visual includes the caret");
    assert_eq!(vim.mode(), Mode::Normal);
}

#[test]
fn visual_line_takes_whole_blocks() {
    let doc = paragraphs(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "Vjd");
    assert_eq!(text(&after), "three");
}

#[test]
fn escape_leaves_visual_mode_with_a_caret() {
    let doc = paragraphs(&["abcdef"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let state = keys(&state, &mut vim, "vll");
    let out = match vim.key(&state, &Binding::plain(Key::Escape)) {
        Response::Apply(tr) => state.applied(*tr),
        _ => state.clone(),
    };
    assert_eq!(vim.mode(), Mode::Normal);
    assert!(out.selection().is_empty());
}

// -- small edits -----------------------------------------------------------

#[test]
fn r_replaces_one_character() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "rx");
    assert_eq!(text(&after), "xbc");
    assert_eq!(vim.mode(), Mode::Normal, "r does not enter insert mode");
}

#[test]
fn tilde_swaps_case_and_a_count_takes_several() {
    let doc = paragraphs(&["abc"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    assert_eq!(text(&keys(&state, &mut vim, "~")), "Abc");
    assert_eq!(text(&keys(&state, &mut vim, "3~")), "ABC");
}

#[test]
fn u_undoes_and_ctrl_r_redoes() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let cut = keys(&state, &mut vim, "dw");
    assert_eq!(text(&cut), "two");

    let undone = match vim.key(&cut, &Binding::plain(Key::Char('u'))) {
        Response::Apply(tr) => cut.applied(*tr),
        _ => cut.clone(),
    };
    assert_eq!(text(&undone), "one two");

    let ctrl_r = Binding::new(Key::Char('r'), Mods::PRIMARY);
    let redone = match vim.key(&undone, &ctrl_r) {
        Response::Apply(tr) => undone.applied(*tr),
        _ => undone.clone(),
    };
    assert_eq!(text(&redone), "two");
}

#[test]
fn j_joins_two_blocks() {
    let doc = paragraphs(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "J");
    assert_eq!(text(&after), "onetwo");
}

// -- prompts and pass-through ----------------------------------------------

#[test]
fn search_keys_ask_the_host_for_a_prompt() {
    let doc = paragraphs(&["one"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    for c in "/?:nN".chars() {
        assert!(
            matches!(
                vim.key(&state, &Binding::plain(Key::Char(c))),
                Response::Prompt(got) if got == c
            ),
            "`{c}` comes back as a prompt"
        );
    }
}

#[test]
fn the_hosts_own_shortcuts_pass_straight_through() {
    let doc = paragraphs(&["one"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let save = Binding::new(Key::Char('s'), Mods::PRIMARY);
    assert!(matches!(vim.key(&state, &save), Response::Pass));
}

#[test]
fn the_pending_command_is_visible() {
    let doc = paragraphs(&["one two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let _ = keys(&state, &mut vim, "2d");
    assert_eq!(vim.pending(), "2d");
    let _ = keys(&state, &mut vim, "3");
    assert_eq!(vim.pending(), "2d3");
    vim.reset();
    assert_eq!(vim.pending(), "");
}

#[test]
fn a_block_caret_says_which_mode_it_is() {
    assert!(Mode::Normal.block_caret());
    assert!(Mode::Visual.block_caret());
    assert!(!Mode::Insert.block_caret());
}

// -- code blocks, where a block holds many lines ---------------------------

fn code(lines: &[&str]) -> Node {
    let b = b();
    b.doc(nodes![b.node(
        nodes::CODE_BLOCK,
        nodes![b.text(&lines.join("\n"))]
    )])
}

fn code_text(state: &EditorState) -> String {
    state.doc().text_content()
}

#[test]
fn j_moves_one_line_inside_a_code_block() {
    let doc = code(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "j");
    assert_eq!(after.selection().head(), 5, "the start of `two`, not past the block");
}

#[test]
fn dollar_stops_at_the_end_of_a_code_line() {
    let doc = code(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "$");
    assert_eq!(after.selection().head(), 4, "the end of `one`, not of the block");
}

#[test]
fn dd_deletes_one_line_of_code_not_the_block() {
    let doc = code(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 5);
    let after = keys(&state, &mut vim, "dd");
    assert_eq!(code_text(&after), "one\nthree", "the block survives");
}

#[test]
fn dd_on_the_last_line_of_code_takes_the_newline_before_it() {
    let doc = code(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 5);
    let after = keys(&state, &mut vim, "dd");
    assert_eq!(code_text(&after), "one", "no blank line left behind");
}

#[test]
fn cc_empties_a_code_line_and_keeps_the_block() {
    let doc = code(&["one", "two", "three"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 5);
    let after = keys(&state, &mut vim, "cc");
    assert_eq!(code_text(&after), "one\n\nthree");
    assert_eq!(vim.mode(), Mode::Insert);
}

#[test]
fn yy_and_p_duplicate_a_line_of_code() {
    let doc = code(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 1);
    let after = keys(&state, &mut vim, "yyp");
    assert_eq!(code_text(&after), "one\none\ntwo");
}

#[test]
fn capital_p_puts_a_code_line_above() {
    let doc = code(&["one", "two"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 5);
    let after = keys(&state, &mut vim, "yyP");
    assert_eq!(code_text(&after), "one\ntwo\ntwo");
}

#[test]
fn x_stops_at_the_end_of_a_code_line() {
    let doc = code(&["ab", "cd"]);
    let mut vim = Vim::new(&basic::schema());
    let state = state_at(doc, 2);
    let after = keys(&state, &mut vim, "5x");
    assert_eq!(code_text(&after), "a\ncd", "it never eats the newline");
}

#[test]
fn caret_finds_the_indent_of_a_code_line() {
    let doc = code(&["fn main() {", "    body()", "}"]);
    let mut vim = Vim::new(&basic::schema());
    // Somewhere in the second line.
    let state = state_at(doc, 20);
    let after = keys(&state, &mut vim, "^");
    assert_eq!(after.selection().head(), 17, "past the four spaces");
}
