// SPDX-License-Identifier: MPL-2.0

//! What a keystroke costs: which blocks a change leaves alone.
//!
//! The point of [`blocks::reusable`] is that laying a document out again for
//! every keystroke is hundreds of times more work than the keystroke needs. It
//! has to be right in one direction absolutely — a block it says is unchanged
//! had better be unchanged, or the view draws stale text — and right in the
//! other direction only usefully, so these tests check both: that the answer
//! is safe, and that it is actually small.

use nib::blocks::{self, Block};
use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::decoration::{Decoration, DecorationSet, Style};
use nib_model::node::Node;
use nib_model::nodes;
use nib_model::state::{EditorState, Selection};

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn paragraphs(count: usize) -> Node {
    let b = b();
    b.doc(
        (0..count)
            .map(|i| b.node(nodes::PARAGRAPH, nodes![b.text(&format!("Paragraph {i}"))]))
            .collect::<Vec<_>>(),
    )
}

fn state(doc: Node, pos: usize) -> EditorState {
    EditorState::with_selection(basic::schema(), doc, Selection::cursor(pos), Vec::new())
}

/// Types `text` at `pos` and returns the document it leaves.
fn typed(doc: &Node, pos: usize, text: &str) -> Node {
    let state = state(doc.clone(), pos);
    let mut tr = state.tr();
    tr.insert_text(text).expect("type");
    tr.doc().clone()
}

/// The answer, and the blocks either side of it.
fn survey(
    old: &Node,
    new: &Node,
    old_dec: &DecorationSet,
    new_dec: &DecorationSet,
) -> (usize, usize, Vec<Block>, Vec<Block>) {
    let old_blocks = blocks::flatten(old);
    let new_blocks = blocks::flatten(new);
    let (prefix, suffix) = blocks::reusable(old, new, old_dec, new_dec, &old_blocks, &new_blocks);
    (prefix, suffix, old_blocks, new_blocks)
}

/// Every block the answer claims is unchanged really is.
fn assert_sound(prefix: usize, suffix: usize, old: &[Block], new: &[Block]) {
    let same = |a: &Block, b: &Block| {
        a.text == b.text
            && a.type_name == b.type_name
            && a.level == b.level
            && a.indent == b.indent
            && a.quote_depth == b.quote_depth
            && a.marker == b.marker
            && a.cell == b.cell
    };
    for i in 0..prefix {
        assert!(
            same(&old[i], &new[i]),
            "block {i} was claimed unchanged but is not"
        );
        assert_eq!(old[i].from, new[i].from, "a prefix block also may not move");
    }
    for k in 1..=suffix {
        let (a, c) = (&old[old.len() - k], &new[new.len() - k]);
        assert!(
            same(a, c),
            "block {k} from the end was claimed unchanged but is not"
        );
    }
    assert!(prefix + suffix <= new.len(), "the two halves overlap");
    assert!(prefix + suffix <= old.len(), "the two halves overlap");
}

fn empty() -> DecorationSet {
    DecorationSet::empty()
}

// -- the document ----------------------------------------------------------

#[test]
fn typing_in_one_block_leaves_every_other_alone() {
    let old = paragraphs(50);
    // Somewhere in the middle of the 25th.
    let at = blocks::flatten(&old)[25].from + 3;
    let new = typed(&old, at, "x");

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(
        c.len() - prefix - suffix,
        1,
        "one block changed, so one block is laid out again"
    );
}

#[test]
fn an_untouched_document_is_kept_whole() {
    let doc = paragraphs(20);
    let (prefix, suffix, a, c) = survey(&doc, &doc, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(prefix + suffix, 20, "nothing changed, so nothing is redone");
}

#[test]
fn splitting_a_block_redoes_only_the_two_halves() {
    let old = paragraphs(30);
    let at = blocks::flatten(&old)[10].from + 4;
    let state = state(old.clone(), at);
    let mut tr = state.tr();
    tr.split(at, 1, None).expect("split");
    let new = tr.doc().clone();

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(c.len(), 31, "a split makes a block");
    assert!(
        c.len() - prefix - suffix <= 2,
        "at most the two halves, got {}",
        c.len() - prefix - suffix
    );
}

#[test]
fn deleting_a_block_redoes_nothing_but_its_neighbours() {
    let old = paragraphs(30);
    let flat = blocks::flatten(&old);
    let state = state(old.clone(), flat[10].from);
    let mut tr = state.tr();
    tr.delete_range(flat[10].from - 1, flat[10].to + 1)
        .expect("delete the block");
    let new = tr.doc().clone();

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(c.len(), 29);
    assert!(
        c.len() - prefix - suffix <= 1,
        "one block went, got {} redone",
        c.len() - prefix - suffix
    );
}

#[test]
fn a_change_of_markup_is_a_change() {
    let old = paragraphs(10);
    let flat = blocks::flatten(&old);
    let state = state(old.clone(), flat[4].from);
    let schema = basic::schema();
    let strong = schema.mark_id(marks::STRONG).expect("strong");
    let mut tr = state.tr();
    tr.add_mark(
        flat[4].from,
        flat[4].to,
        &nib_model::mark::Mark::new(
            schema.mark_type(strong).clone(),
            nib_model::attrs::Attrs::none(),
        ),
    )
    .expect("bold it");
    let new = tr.doc().clone();

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert!(
        prefix <= 4 && prefix + suffix < c.len(),
        "the bolded block is redone"
    );
}

#[test]
fn a_document_replaced_wholesale_keeps_nothing() {
    let old = paragraphs(10);
    let new = paragraphs(10);
    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    // Two documents built separately share no structure, so the diff has to
    // walk them — but it must never claim more than it can prove.
    assert!(prefix + suffix <= c.len());
}

// -- the decorations -------------------------------------------------------

fn squiggle(from: usize, to: usize) -> Decoration {
    Decoration::inline(from, to, Style::class("spelling-error"))
}

#[test]
fn a_new_decoration_redoes_the_block_it_lands_on() {
    let doc = paragraphs(20);
    let flat = blocks::flatten(&doc);
    let after = DecorationSet::new(vec![squiggle(flat[8].from, flat[8].from + 4)]);

    let (prefix, suffix, a, c) = survey(&doc, &doc, &empty(), &after);
    assert_sound(prefix, suffix, &a, &c);
    assert!(prefix <= 8, "the block under it is not kept");
    assert!(
        prefix + suffix >= 18,
        "and every other block is, got {prefix} + {suffix}"
    );
}

#[test]
fn a_decoration_that_only_shifted_costs_nothing() {
    let old = paragraphs(20);
    let flat = blocks::flatten(&old);
    // A squiggle late in the document, and a keystroke early in it.
    let before = DecorationSet::new(vec![squiggle(flat[15].from, flat[15].from + 4)]);
    let new = typed(&old, flat[2].from, "xy");
    let after = DecorationSet::new(vec![squiggle(flat[15].from + 2, flat[15].from + 6)]);

    let (prefix, suffix, a, c) = survey(&old, &new, &before, &after);
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(
        c.len() - prefix - suffix,
        1,
        "the squiggle moved with the text, which is not a change to it"
    );
}

#[test]
fn a_decoration_that_changed_style_redoes_its_block() {
    let doc = paragraphs(20);
    let flat = blocks::flatten(&doc);
    let before = DecorationSet::new(vec![squiggle(flat[8].from, flat[8].from + 4)]);
    let after = DecorationSet::new(vec![Decoration::inline(
        flat[8].from,
        flat[8].from + 4,
        Style::class("search-hit"),
    )]);

    let (prefix, suffix, a, c) = survey(&doc, &doc, &before, &after);
    assert_sound(prefix, suffix, &a, &c);
    assert!(
        prefix <= 8,
        "the block whose colour changed is laid out again"
    );
}

#[test]
fn identical_decorations_are_not_a_reason_to_redo_anything() {
    let doc = paragraphs(20);
    let flat = blocks::flatten(&doc);
    let set = || DecorationSet::new(vec![squiggle(flat[8].from, flat[8].from + 4)]);
    let (prefix, suffix, a, c) = survey(&doc, &doc, &set(), &set());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(prefix + suffix, 20);
}

#[test]
fn a_decoration_removed_redoes_where_it_was() {
    let doc = paragraphs(20);
    let flat = blocks::flatten(&doc);
    let before = DecorationSet::new(vec![squiggle(flat[8].from, flat[8].from + 4)]);

    let (prefix, suffix, a, c) = survey(&doc, &doc, &before, &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert!(prefix <= 8, "the block it left is laid out again");
}

// -- structure -------------------------------------------------------------

#[test]
fn typing_inside_a_list_leaves_the_rest_of_it_alone() {
    let b = b();
    let item = |text: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let old = b.doc(nodes![
        b.node(
            nodes::BULLET_LIST,
            (0..12)
                .map(|i| item(&format!("item {i}")))
                .collect::<Vec<_>>()
        )
    ]);
    let flat = blocks::flatten(&old);
    let new = typed(&old, flat[6].from + 2, "!");

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(c.len() - prefix - suffix, 1, "one item, not the whole list");
}

#[test]
fn typing_in_a_table_cell_leaves_the_other_cells_alone() {
    let b = b();
    let cell = |text: &str| {
        b.node(
            nodes::TABLE_CELL,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let row = |a: &str, c: &str| b.node(nodes::TABLE_ROW, nodes![cell(a), cell(c)]);
    let old = b.doc(nodes![
        b.node(
            nodes::TABLE,
            (0..6)
                .map(|i| row(&format!("left {i}"), &format!("right {i}")))
                .collect::<Vec<_>>()
        )
    ]);
    let flat = blocks::flatten(&old);
    let new = typed(&old, flat[5].from + 1, "z");

    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(
        c.len() - prefix - suffix,
        1,
        "the diff still narrows to one cell; the view is what decides a table \
         is laid out as a whole"
    );
}

#[test]
fn an_empty_document_is_handled() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, Vec::new())]);
    let (prefix, suffix, a, c) = survey(&doc, &doc, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
    assert_eq!(prefix + suffix, 1);
}

#[test]
fn a_document_that_grew_from_nothing_keeps_nothing_it_cannot() {
    let b = b();
    let old = b.doc(nodes![b.node(nodes::PARAGRAPH, Vec::new())]);
    let new = paragraphs(5);
    let (prefix, suffix, a, c) = survey(&old, &new, &empty(), &empty());
    assert_sound(prefix, suffix, &a, &c);
}
