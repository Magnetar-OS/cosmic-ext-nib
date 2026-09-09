// SPDX-License-Identifier: MPL-2.0

//! Fitting slices into gaps they do not match — what paste actually does.

use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::fragment::Fragment;
use nib_model::node::Node;
use nib_model::schema::Schema;
use nib_model::slice::Slice;
use nib_model::transform::Transform;
use nib_model::{attrs, nodes};

fn schema() -> Schema {
    basic::schema()
}

fn b() -> Builder {
    Builder::new(schema())
}

fn tr(doc: Node) -> Transform {
    Transform::new(schema(), doc)
}

fn whole(doc: &Node) -> Slice {
    doc.slice(0, doc.content_size(), false)
}

fn list(items: &[&str]) -> Node {
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
// Pasting blocks into inline content
// ---------------------------------------------------------------------------

#[test]
fn pasting_a_paragraph_into_a_paragraph_splits_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    let source = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("NEW")])]);
    let mut tr = tr(doc);
    tr.replace_fitted(6, 6, whole(&source)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("hello"), paragraph("NEW"), paragraph(" world"))"#
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn pasting_a_list_into_a_paragraph_keeps_the_list() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    let source = list(&["one", "two"]);
    let mut tr = tr(doc);
    tr.replace_fitted(6, 6, whole(&source)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("hello"), bullet_list(list_item(paragraph("one")), \
list_item(paragraph("two"))), paragraph(" world"))"#
            .replace("\\\n", "")
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn pasting_inline_content_stays_inline() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    let source = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("NEW")])]);
    // An open slice: the tail of a paragraph, not a paragraph.
    let mut tr = tr(doc);
    tr.replace_fitted(6, 6, source.slice(1, 4, false)).unwrap();
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph("helloNEW world"))"#);
}

#[test]
fn pasting_marked_text_keeps_its_marks() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("ab")])]);
    let source = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.mark(marks::STRONG, None, nodes![b.text("X")])]
    )]);
    let mut tr = tr(doc);
    tr.replace_fitted(2, 2, source.slice(1, 2, false)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("a", strong("X"), "b"))"#
    );
}

// ---------------------------------------------------------------------------
// Pasting into content the schema restricts
// ---------------------------------------------------------------------------

#[test]
fn pasting_a_paragraph_into_a_code_block_keeps_only_the_text() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("abc")])]);
    let source = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.mark(marks::STRONG, None, nodes![b.text("X")])]
    )]);
    let mut tr = tr(doc);
    tr.replace_fitted(2, 2, source.slice(1, 2, false)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(code_block("aXbc"))"#,
        "a code block admits text and no marks"
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn pasting_a_paragraph_into_a_list_item_wraps_where_it_must() {
    let doc = list(&["one"]);
    let b = b();
    let source = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("X")])]);
    // Inside the item's paragraph, after "one".
    let mut tr = tr(doc);
    tr.replace_fitted(6, 6, whole(&source)).unwrap();
    assert_eq!(tr.doc().check(), Ok(()));
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(bullet_list(list_item(paragraph("one"), paragraph("X"))))"#
    );
}

#[test]
fn pasting_inline_content_between_list_items_wraps_it_into_an_item() {
    // This is what a clipboard paste of a paragraph's *contents* looks like:
    // an open slice. The fitter finds that a bullet list needs a list item,
    // and a list item needs a paragraph, and builds both.
    let doc = list(&["one", "two"]);
    let b = b();
    let source = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("X")])]);
    let at = doc.resolve(2).after(2);
    let mut tr = tr(doc);
    tr.replace_fitted(at, at, source.slice(1, 2, false)).unwrap();
    assert_eq!(tr.doc().check(), Ok(()));
    assert_eq!(
        tr.doc().to_string(),
        concat!(
            r#"doc(bullet_list(list_item(paragraph("one")), "#,
            r#"list_item(paragraph("X")), list_item(paragraph("two"))))"#
        )
    );
}

#[test]
fn pasting_a_closed_paragraph_between_list_items_breaks_the_list() {
    // The counterpart, and it is the right answer rather than a shortcoming:
    // a *closed* paragraph is a whole block, and a whole block that the list
    // cannot hold goes beside the list, not inside it. The distinction is
    // exactly what a slice's open depths are for.
    let doc = list(&["one", "two"]);
    let b = b();
    let source = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("X")])]);
    let at = doc.resolve(2).after(2);
    let mut tr = tr(doc);
    tr.replace_fitted(at, at, whole(&source)).unwrap();
    assert_eq!(tr.doc().check(), Ok(()));
    assert_eq!(
        tr.doc().to_string(),
        concat!(
            r#"doc(bullet_list(list_item(paragraph("one"))), paragraph("X"), "#,
            r#"bullet_list(list_item(paragraph("two"))))"#
        )
    );
}

// ---------------------------------------------------------------------------
// Range replacement
// ---------------------------------------------------------------------------

#[test]
fn pasting_a_heading_over_an_empty_paragraph_replaces_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![])]);
    let source = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        attrs! { "level" => 2_i64 },
        nodes![b.text("Title")]
    )]);
    let mut tr = tr(doc);
    tr.replace_range(1, 1, whole(&source)).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(heading[level=2]("Title"))"#,
        "an empty paragraph is replaced, not filled"
    );
}

#[test]
fn replace_range_with_inserts_a_block_beside_a_paragraph() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("text")])]);
    let rule = b.node(nodes::HORIZONTAL_RULE, nodes![]);
    let mut tr = tr(doc);
    tr.replace_range_with(5, 5, rule).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph("text"), horizontal_rule)"#
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

// ---------------------------------------------------------------------------
// Range deletion
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_fully_covered_paragraph_removes_the_paragraph() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    // Exactly the text of the first paragraph.
    let mut tr = tr(doc);
    tr.delete_range(1, 4).unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(paragraph, paragraph("two"))"#,
        "a paragraph may legally be empty, so it is emptied rather than removed"
    );
}

#[test]
fn deleting_across_whole_blocks_removes_them() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("three")]),
    ]);
    let mut tr = tr(doc);
    tr.delete_range(1, 9).unwrap();
    assert_eq!(tr.doc().check(), Ok(()));
    assert_eq!(tr.doc().to_string(), r#"doc(paragraph, paragraph("three"))"#);
}

#[test]
fn deleting_the_only_item_of_a_list_leaves_a_valid_document() {
    let doc = list(&["only"]);
    let mut tr = tr(doc);
    // The whole list's content.
    tr.delete_range(3, 7).unwrap();
    assert_eq!(tr.doc().check(), Ok(()));
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn content_that_cannot_go_inside_a_block_closes_it_and_goes_beside_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("abc")])]);
    // A rule cannot live in a code block, but the frontier can be closed and
    // reopened around it — which is what the user meant by pasting it there.
    let rule = b.node(nodes::HORIZONTAL_RULE, nodes![]);
    let mut tr = tr(doc);
    tr.replace_fitted(2, 2, Slice::new(Fragment::from(rule), 0, 0))
        .unwrap();
    assert_eq!(
        tr.doc().to_string(),
        r#"doc(code_block("a"), horizontal_rule, code_block("bc"))"#
    );
    assert_eq!(tr.doc().check(), Ok(()));
}

#[test]
fn a_slice_with_nothing_placeable_changes_nothing() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("abc")])]);
    let mut tr = tr(doc.clone());
    tr.replace_fitted(2, 2, Slice::empty()).unwrap();
    assert_eq!(tr.doc().to_string(), doc.to_string());
}

#[test]
fn every_fitted_paste_leaves_a_valid_document() {
    // A sweep: every slice of a mixed document, pasted at every block
    // boundary of another, must produce something the schema accepts.
    let b = b();
    let source = b.doc(nodes![
        b.attr_node(nodes::HEADING, attrs! { "level" => 1_i64 }, nodes![b.text("H")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("para")]),
        b.node(
            nodes::BULLET_LIST,
            nodes![b.node(
                nodes::LIST_ITEM,
                nodes![b.node(nodes::PARAGRAPH, nodes![b.text("item")])]
            )]
        ),
        b.node(nodes::CODE_BLOCK, nodes![b.text("code")]),
    ]);
    let target = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("target")]),
        b.node(
            nodes::BLOCKQUOTE,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text("quoted")])]
        ),
    ]);

    let mut checked = 0;
    for from in 0..source.content_size() {
        for to in (from + 1)..=source.content_size() {
            let slice = source.slice(from, to, false);
            for at in 0..=target.content_size() {
                let mut tr = tr(target.clone());
                if tr.replace_fitted(at, at, slice.clone()).is_err() {
                    continue;
                }
                assert_eq!(
                    tr.doc().check(),
                    Ok(()),
                    "pasting source[{from}..{to}] at {at} produced {}",
                    tr.doc()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1000, "the sweep should be large: {checked}");
}
