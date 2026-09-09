// SPDX-License-Identifier: MPL-2.0

//! Positions, slices, and the replace algorithm.

use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::fragment::Fragment;
use nib_model::node::Node;
use nib_model::replace::ReplaceError;
use nib_model::slice::Slice;
use nib_model::{attrs, nodes};

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn two_paragraphs() -> Node {
    let b = b();
    b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ])
}

// ---------------------------------------------------------------------------
// Resolving
// ---------------------------------------------------------------------------

#[test]
fn a_position_knows_its_ancestors_and_where_it_sits_in_each() {
    let doc = two_paragraphs();
    // Position 2 is between "o" and "n" of the first paragraph.
    let at = doc.resolve(2);
    assert_eq!(at.depth(), 1);
    assert_eq!(at.parent().type_name(), nodes::PARAGRAPH);
    assert_eq!(at.parent_offset(), 1);
    assert_eq!(at.text_offset(), 1);
    assert_eq!(at.start(1), 1);
    assert_eq!(at.end(1), 4);
    assert_eq!(at.before(1), 0);
    assert_eq!(at.after(1), 5);
    assert_eq!(at.index(0), 0, "the first paragraph");
}

#[test]
fn a_position_on_a_node_boundary_has_no_text_offset() {
    let doc = two_paragraphs();
    let between = doc.resolve(5);
    assert_eq!(between.depth(), 0, "between the paragraphs, directly in the doc");
    assert_eq!(between.text_offset(), 0);
    assert_eq!(between.index(0), 1);
    assert_eq!(between.node_before().unwrap().text_content(), "one");
    assert_eq!(between.node_after().unwrap().text_content(), "two");
}

#[test]
fn node_before_and_after_cut_the_text_when_the_position_is_inside_it() {
    let doc = two_paragraphs();
    let inside = doc.resolve(2);
    assert_eq!(inside.node_before().unwrap().text(), Some("o"));
    assert_eq!(inside.node_after().unwrap().text(), Some("ne"));
}

#[test]
fn shared_depth_finds_the_deepest_common_ancestor() {
    let doc = two_paragraphs();
    let inside_first = doc.resolve(2);
    assert_eq!(inside_first.shared_depth(3), 1, "same paragraph");
    assert_eq!(inside_first.shared_depth(8), 0, "different paragraphs");
}

#[test]
fn a_block_range_covers_the_sibling_blocks_a_selection_touches() {
    let doc = two_paragraphs();
    let range = doc
        .resolve(2)
        .block_range(&doc.resolve(8), None)
        .expect("two paragraphs are a block range");
    assert_eq!(range.depth(), 0);
    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 10);
    assert_eq!(range.start_index(), 0);
    assert_eq!(range.end_index(), 2);
    assert_eq!(range.parent().type_name(), nodes::DOC);
}

// ---------------------------------------------------------------------------
// Slicing
// ---------------------------------------------------------------------------

#[test]
fn a_slice_across_two_blocks_is_open_at_both_ends() {
    let doc = two_paragraphs();
    let slice = doc.slice(2, 8, false);
    assert_eq!(slice.open_start(), 1);
    assert_eq!(slice.open_end(), 1);
    assert_eq!(slice.content().child_count(), 2);
    assert_eq!(slice.content().child(0).unwrap().text_content(), "ne");
    assert_eq!(slice.content().child(1).unwrap().text_content(), "tw");
}

#[test]
fn a_slice_within_one_block_is_closed() {
    let doc = two_paragraphs();
    let slice = doc.slice(2, 3, false);
    assert_eq!(slice.open_start(), 0);
    assert_eq!(slice.open_end(), 0);
    assert_eq!(slice.content().text_content(), "n");
}

#[test]
fn a_slices_size_excludes_the_boundaries_it_will_not_insert() {
    let doc = two_paragraphs();
    // "ne" + "tw" plus two blocks' worth of boundaries, less the two open ones.
    let slice = doc.slice(2, 8, false);
    assert_eq!(slice.content().size(), 8);
    assert_eq!(slice.size(), 6, "8 less the two open boundaries");
}

// ---------------------------------------------------------------------------
// Deleting
// ---------------------------------------------------------------------------

#[test]
fn deleting_inside_one_text_node_shortens_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    let after = doc.delete(3, 5).unwrap();
    assert_eq!(after.text_content(), "heo world");
    assert_eq!(after.check(), Ok(()));
}

#[test]
fn deleting_across_a_block_boundary_joins_the_two_blocks() {
    let doc = two_paragraphs();
    // From inside the first to inside the second.
    let after = doc.delete(2, 8).unwrap();
    assert_eq!(after.to_string(), r#"doc(paragraph("oo"))"#);
    assert_eq!(after.check(), Ok(()));
}

#[test]
fn deleting_a_whole_block_removes_it() {
    let doc = two_paragraphs();
    let after = doc.delete(0, 5).unwrap();
    assert_eq!(after.to_string(), r#"doc(paragraph("two"))"#);
}

#[test]
fn deleting_everything_leaves_the_document_valid() {
    let doc = two_paragraphs();
    // `doc` is `block+`, so emptying it entirely is not allowed — the caller
    // must leave a block behind. The model says so rather than producing an
    // invalid document.
    let err = doc.delete(0, doc.content_size()).unwrap_err();
    assert!(matches!(err, ReplaceError::InvalidContent { .. }), "{err:?}");
}

// ---------------------------------------------------------------------------
// Inserting
// ---------------------------------------------------------------------------

#[test]
fn inserting_text_into_a_paragraph_merges_with_what_is_there() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("helo")])]);
    let slice = Slice::new(Fragment::from(b.text("l")), 0, 0);
    let after = doc.replace(3, 3, &slice).unwrap();
    assert_eq!(after.text_content(), "hello");
    assert_eq!(
        after.child(0).unwrap().child_count(),
        1,
        "one run, not three"
    );
}

#[test]
fn inserting_a_whole_block_between_two_others() {
    let b = b();
    let doc = two_paragraphs();
    let inserted = b.node(nodes::PARAGRAPH, nodes![b.text("mid")]);
    let slice = Slice::new(Fragment::from(inserted), 0, 0);
    let after = doc.replace(5, 5, &slice).unwrap();
    assert_eq!(
        after.to_string(),
        r#"doc(paragraph("one"), paragraph("mid"), paragraph("two"))"#
    );
}

#[test]
fn splitting_a_paragraph_is_a_replace_with_an_open_slice() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    // Two empty paragraphs, both edges open: the left one joins what precedes
    // the caret, the right one what follows.
    let slice = Slice::new(
        Fragment::from_vec(vec![
            b.node(nodes::PARAGRAPH, nodes![]),
            b.node(nodes::PARAGRAPH, nodes![]),
        ]),
        1,
        1,
    );
    let after = doc.replace(6, 6, &slice).unwrap();
    assert_eq!(
        after.to_string(),
        r#"doc(paragraph("hello"), paragraph(" world"))"#
    );
    assert_eq!(after.check(), Ok(()));
}

#[test]
fn pasting_an_open_slice_across_a_boundary_joins_both_sides() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("start end")]),
    ]);
    // A slice cut across two paragraphs, pasted into the middle of one: the
    // slice's tail joins what is before the caret, its head joins what is
    // after, and the result is one more paragraph, not two more.
    let source = two_paragraphs();
    let slice = source.slice(2, 8, false);
    let after = doc.replace(6, 6, &slice).unwrap();
    assert_eq!(
        after.to_string(),
        r#"doc(paragraph("startne"), paragraph("tw end"))"#
    );
    assert_eq!(after.check(), Ok(()));
}

#[test]
fn replacing_a_selection_with_a_block_removes_and_inserts_at_once() {
    let b = b();
    let doc = two_paragraphs();
    let heading = b.attr_node(
        nodes::HEADING,
        attrs! { "level" => 2_i64 },
        nodes![b.text("Title")],
    );
    let slice = Slice::new(Fragment::from(heading), 0, 0);
    let after = doc.replace(0, 5, &slice).unwrap();
    assert_eq!(
        after.to_string(),
        r#"doc(heading[level=2]("Title"), paragraph("two"))"#
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_slice_deeper_than_its_destination_is_refused() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hi")])]);
    let slice = Slice::new(Fragment::from(b.text("x")), 3, 0);
    let err = doc.replace(1, 1, &slice).unwrap_err();
    assert!(matches!(err, ReplaceError::TooDeep { .. }), "{err:?}");
}

#[test]
fn mismatched_open_depths_are_refused() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hi")])]);
    let slice = Slice::new(
        Fragment::from(b.node(nodes::PARAGRAPH, nodes![b.text("x")])),
        1,
        0,
    );
    let err = doc.replace(1, 2, &slice).unwrap_err();
    assert!(
        matches!(err, ReplaceError::InconsistentDepths { .. }),
        "{err:?}"
    );
}

#[test]
fn content_the_schema_forbids_is_refused_rather_than_written() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hi")])]);
    // A list item cannot go directly in a document.
    let item = b.node(
        nodes::LIST_ITEM,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("x")])],
    );
    let slice = Slice::new(Fragment::from(item), 0, 0);
    let err = doc.replace(4, 4, &slice).unwrap_err();
    assert!(
        matches!(err, ReplaceError::InvalidContent { .. }),
        "{err:?}"
    );
}

// ---------------------------------------------------------------------------
// Nested structure
// ---------------------------------------------------------------------------

#[test]
fn deleting_across_list_items_joins_them() {
    let b = b();
    let item = |text: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let doc = b.doc(nodes![b.node(
        nodes::BULLET_LIST,
        nodes![item("one"), item("two")]
    )]);
    // doc 0 <ul> 1 <li> 2 <p> 3 "one" 6 </p> 7 </li> 8 <li> 9 <p> 10 "two" ...
    let after = doc.delete(4, 11).unwrap();
    assert_eq!(
        after.to_string(),
        r#"doc(bullet_list(list_item(paragraph("owo"))))"#
    );
    assert_eq!(after.check(), Ok(()));
}

#[test]
fn text_keeps_its_marks_through_a_replace() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("plain "),
            b.mark("strong", None, nodes![b.text("bold")]),
        ]
    )]);
    let after = doc.delete(1, 4).unwrap();
    assert_eq!(after.to_string(), r#"doc(paragraph("in ", strong("bold")))"#);
}
