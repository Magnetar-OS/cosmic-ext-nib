// SPDX-License-Identifier: MPL-2.0

//! The structural edits a user actually presses keys for.

use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::node::Node;
use nib_model::schema::Schema;
use nib_model::transform::structure::{MarkFilter, TypeAndAttrs};
use nib_model::transform::{
    Transform, can_join, can_split, find_wrapping, insert_point, lift_target,
};
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

fn tr(doc: Node) -> Transform {
    Transform::new(schema(), doc)
}

// ---------------------------------------------------------------------------
// Splitting — what Enter does
// ---------------------------------------------------------------------------

#[test]
fn a_paragraph_splits_in_two() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    assert!(can_split(&schema(), &doc, 6, 1, None));
    let mut tr = tr(doc);
    tr.split(6, 1, None).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("hello"), paragraph(" world"))"#
    );
}

#[test]
fn splitting_can_name_what_the_second_half_becomes() {
    // Enter at the end of a heading should start a paragraph, not a second
    // heading. That is `types_after`, not a special case in the command.
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        attrs! { "level" => 2_i64 },
        nodes![b.text("Title")]
    )]);
    let after = [Some(TypeAndAttrs::new(id(nodes::PARAGRAPH)))];
    assert!(can_split(&schema(), &doc, 6, 1, Some(&after)));
    let mut tr = tr(doc);
    tr.split(6, 1, Some(&after)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(heading[level=2]("Title"), paragraph)"#
    );
}

#[test]
fn a_list_item_splits_two_levels_deep() {
    let b = b();
    let item = |text: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(text)])],
        )
    };
    let doc = b.doc(nodes![b.node(nodes::BULLET_LIST, nodes![item("onetwo")])]);
    // doc 0 <ul> 1 <li> 2 <p> 3 "onetwo" 9 </p> 10 </li> 11 </ul>
    assert!(can_split(&schema(), &doc, 6, 2, None));
    let mut tr = tr(doc);
    tr.split(6, 2, None).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("one")), list_item(paragraph("two"))))"#
    );
}

#[test]
fn a_split_that_would_be_invalid_is_refused_before_it_is_attempted() {
    let b = b();
    // A code block's content is `text*`; splitting it into two code blocks is
    // fine, but splitting two levels deep has nothing to split.
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("fn main")])]);
    assert!(can_split(&schema(), &doc, 4, 1, None));
    assert!(!can_split(&schema(), &doc, 4, 2, None));
}

// ---------------------------------------------------------------------------
// Joining — what Backspace at a block start does
// ---------------------------------------------------------------------------

#[test]
fn two_paragraphs_join() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    assert!(can_join(&doc, 5));
    let mut tr = tr(doc);
    tr.join(5, 1).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph("onetwo"))"#);
}

#[test]
fn a_paragraph_will_not_join_onto_a_code_block() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::CODE_BLOCK, nodes![b.text("code")]),
        b.node(nodes::BULLET_LIST, nodes![b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text("x")])]
        )]),
    ]);
    assert!(!can_join(&doc, 6), "a list is not code-block content");
}

// ---------------------------------------------------------------------------
// Lifting — what outdent does
// ---------------------------------------------------------------------------

#[test]
fn a_paragraph_lifts_out_of_a_blockquote() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("a")])]
    )]);
    let at = doc.resolve(3);
    let range = at.block_range(&at, None).expect("a block range");
    assert_eq!(range.depth(), 1, "the quote's children");
    let target = lift_target(&range).expect("liftable");
    assert_eq!(target, 0);

    let mut tr = tr(doc);
    tr.lift(&range, target).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph("a"))"#);
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn lifting_one_of_several_keeps_the_others_wrapped() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![
            b.node(nodes::PARAGRAPH, nodes![b.text("a")]),
            b.node(nodes::PARAGRAPH, nodes![b.text("b")]),
        ]
    )]);
    // Inside the second paragraph.
    let at = doc.resolve(6);
    let range = at.block_range(&at, None).unwrap();
    let target = lift_target(&range).unwrap();
    let mut tr = tr(doc);
    tr.lift(&range, target).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(blockquote(paragraph("a")), paragraph("b"))"#
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

// ---------------------------------------------------------------------------
// Wrapping — what "make this a quote / a list" does
// ---------------------------------------------------------------------------

#[test]
fn paragraphs_wrap_in_a_blockquote() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("a")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("b")]),
    ]);
    let range = doc.resolve(2).block_range(&doc.resolve(5), None).unwrap();
    let wrapping =
        find_wrapping(&schema(), &range, id(nodes::BLOCKQUOTE), None, None).expect("wrappable");
    assert_eq!(wrapping.len(), 1);

    let mut tr = tr(doc);
    tr.wrap(&range, &wrapping).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(blockquote(paragraph("a"), paragraph("b")))"#
    );
}

#[test]
fn wrapping_in_a_list_adds_the_item_the_schema_requires() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("a")])]);
    let at = doc.resolve(2);
    let range = at.block_range(&at, None).unwrap();
    let wrapping =
        find_wrapping(&schema(), &range, id(nodes::BULLET_LIST), None, None).expect("wrappable");
    let s = schema();
    let names: Vec<&str> = wrapping
        .iter()
        .map(|w| s.node_type(w.typ).name())
        .collect();
    assert_eq!(
        names,
        [nodes::BULLET_LIST, nodes::LIST_ITEM],
        "a paragraph cannot go straight into a list"
    );

    let mut tr = tr(doc);
    tr.wrap(&range, &wrapping).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("a"))))"#
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn nothing_wraps_a_paragraph_in_a_code_block() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("a")])]);
    let at = doc.resolve(2);
    let range = at.block_range(&at, None).unwrap();
    assert_eq!(
        find_wrapping(&schema(), &range, id(nodes::CODE_BLOCK), None, None),
        None
    );
}

// ---------------------------------------------------------------------------
// Retyping
// ---------------------------------------------------------------------------

#[test]
fn a_paragraph_becomes_a_heading() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("Title")])]);
    let mut tr = tr(doc);
    tr.set_block_type(1, 1, id(nodes::HEADING), Some(&attrs! { "level" => 2_i64 }))
        .unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(heading[level=2]("Title"))"#
    );
}

#[test]
fn retyping_to_a_code_block_drops_the_marks_it_cannot_carry() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("let "),
            b.mark(marks::STRONG, None, nodes![b.text("x")]),
        ]
    )]);
    let mut tr = tr(doc);
    tr.set_block_type(1, 1, id(nodes::CODE_BLOCK), None).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(code_block("let x"))"#);
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn every_selected_block_is_retyped() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("a")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("b")]),
    ]);
    let mut tr = tr(doc);
    tr.set_block_type(1, 5, id(nodes::HEADING), None).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(heading[level=1]("a"), heading[level=1]("b"))"#
    );
}

// ---------------------------------------------------------------------------
// Marks over a range
// ---------------------------------------------------------------------------

#[test]
fn a_mark_applies_across_blocks_in_one_transaction() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    let strong = schema().mark(marks::STRONG, None).unwrap();
    let mut tr = tr(doc);
    tr.add_mark(2, 8, &strong).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("o", strong("ne")), paragraph(strong("tw"), "o"))"#
    );
}

#[test]
fn applying_code_over_bold_removes_the_bold_in_the_same_transaction() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.mark(marks::STRONG, None, nodes![b.text("bold")])]
    )]);
    let code = schema().mark(marks::CODE, None).unwrap();
    let mut tr = tr(doc);
    tr.add_mark(1, 5, &code).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph(code("bold")))"#);
    // And one undo takes both back.
    let mut back = tr.doc().clone();
    for step in tr.invert() {
        back = step.apply(&back).unwrap();
    }
    assert_eq!(back.to_string(), r#"doc(paragraph(strong("bold")))"#);
}

#[test]
fn a_mark_is_not_applied_where_the_schema_forbids_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("code")])]);
    let strong = schema().mark(marks::STRONG, None).unwrap();
    let mut tr = tr(doc.clone());
    tr.add_mark(1, 5, &strong).unwrap();
    assert_eq!(tr.doc(), &doc, "a code block admits no marks");
}

#[test]
fn removing_by_type_takes_the_link_off_without_knowing_its_target() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.mark(
            marks::LINK,
            Some(attrs! { "href" => "http://example.test" }),
            nodes![b.text("here")]
        )]
    )]);
    let mut tr = tr(doc);
    tr.remove_mark(1, 5, MarkFilter::OfType(schema().mark_id(marks::LINK).unwrap()))
        .unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph("here"))"#);
}

#[test]
fn removing_everything_clears_every_mark_in_the_range() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.mark(marks::STRONG, None, nodes![b.text("a")]),
            b.mark(marks::EM, None, nodes![b.text("b")]),
        ]
    )]);
    let mut tr = tr(doc);
    tr.remove_mark(1, 3, MarkFilter::All).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph("ab"))"#);
}

// ---------------------------------------------------------------------------
// Insertion points
// ---------------------------------------------------------------------------

#[test]
fn an_insertion_point_is_found_outside_a_block_that_will_not_take_the_node() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("text")])]);
    // A horizontal rule cannot go inside a paragraph, but can go beside it.
    let rule = id(nodes::HORIZONTAL_RULE);
    assert_eq!(insert_point(&doc, 3, rule), None, "mid-paragraph, nowhere");
    assert_eq!(insert_point(&doc, 1, rule), Some(0), "at its start, before it");
    assert_eq!(insert_point(&doc, 5, rule), Some(6), "at its end, after it");
}
