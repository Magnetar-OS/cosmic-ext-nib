// SPDX-License-Identifier: MPL-2.0

//! Steps, mapping, and the two properties everything else rests on: that a
//! step inverts, and that a step maps.

use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::fragment::Fragment;
use nib_model::node::Node;
use nib_model::slice::Slice;
use nib_model::transform::{Mapping, Step, StepMap, Transform};
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

/// Applies a step, then its inverse, and insists the document comes back.
fn round_trips(doc: &Node, step: &Step) -> Node {
    let after = step.apply(doc).expect("the step should apply");
    let back = step
        .invert(doc)
        .apply(&after)
        .expect("the inverse should apply");
    assert_eq!(
        back, *doc,
        "inverting {step:?} did not restore the document"
    );
    after
}

// ---------------------------------------------------------------------------
// Inversion — half of undo, and half of collaboration
// ---------------------------------------------------------------------------

#[test]
fn a_deletion_inverts_to_the_content_it_removed() {
    let doc = two_paragraphs();
    let after = round_trips(&doc, &Step::replace(2, 8, Slice::empty()));
    assert_eq!(after.to_string(), r#"doc(paragraph("oo"))"#);
}

#[test]
fn an_insertion_inverts_to_a_deletion() {
    let b = b();
    let doc = two_paragraphs();
    let step = Step::replace(2, 2, Slice::new(Fragment::from(b.text("XX")), 0, 0));
    let after = round_trips(&doc, &step);
    assert_eq!(after.child(0).unwrap().text_content(), "oXXne");
}

#[test]
fn a_split_inverts_to_a_join() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])
    ]);
    let slice = Slice::new(
        Fragment::from_vec(vec![
            b.node(nodes::PARAGRAPH, nodes![]),
            b.node(nodes::PARAGRAPH, nodes![]),
        ]),
        1,
        1,
    );
    let after = round_trips(&doc, &Step::replace(6, 6, slice));
    assert_eq!(after.child_count(), 2);
}

#[test]
fn a_mark_step_inverts_to_its_opposite() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello")])]);
    let strong = basic::schema().mark(marks::STRONG, None).unwrap();
    let after = round_trips(
        &doc,
        &Step::AddMark {
            from: 1,
            to: 4,
            mark: strong,
        },
    );
    assert_eq!(after.to_string(), r#"doc(paragraph(strong("hel"), "lo"))"#);
}

#[test]
fn an_attribute_step_inverts_to_the_old_value() {
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        &attrs! { "level" => 1_i64 },
        nodes![b.text("Title")]
    )]);
    let after = round_trips(
        &doc,
        &Step::SetAttr {
            pos: 0,
            attr: "level".into(),
            value: 3_i64.into(),
        },
    );
    assert_eq!(after.child(0).unwrap().attrs().get_int("level"), Some(3));
    assert_eq!(
        after.child(0).unwrap().text_content(),
        "Title",
        "changing an attribute must not disturb the content"
    );
}

#[test]
fn a_gap_replace_wrapping_a_paragraph_inverts_to_unwrapping_it() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("quote me")])]);
    // Wrap 0..10 (the whole paragraph) in a blockquote, keeping the paragraph
    // as the gap and inserting it one position in — inside the new quote.
    let quote = b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(nodes::PARAGRAPH, nodes![])],
    );
    let step = Step::ReplaceAround {
        from: 0,
        to: doc.content_size(),
        gap_from: 0,
        gap_to: doc.content_size(),
        slice: Slice::new(Fragment::from(quote), 0, 0)
            .remove_between(1, 3)
            .unwrap(),
        insert: 1,
        structure: true,
    };
    let after = round_trips(&doc, &step);
    assert_eq!(
        after.to_string(),
        r#"doc(blockquote(paragraph("quote me")))"#
    );
}

// ---------------------------------------------------------------------------
// Mapping — the other half
// ---------------------------------------------------------------------------

#[test]
fn a_position_after_an_insertion_moves_by_its_length() {
    // Two characters inserted at 3.
    let map = StepMap::single(3, 0, 2);
    assert_eq!(map.map(2, 1), 2, "before the insertion, unmoved");
    assert_eq!(map.map(5, 1), 7, "after it, pushed along");
}

#[test]
fn association_decides_which_side_of_an_insertion_a_position_lands_on() {
    let map = StepMap::single(3, 0, 2);
    assert_eq!(map.map(3, 1), 5, "a caret follows what was typed");
    assert_eq!(map.map(3, -1), 3, "an anchor stays put");
}

#[test]
fn a_position_inside_a_deletion_collapses_to_its_start_and_says_so() {
    // Positions 3..7 deleted.
    let map = StepMap::single(3, 4, 0);
    let result = map.map_result(5, 1);
    assert_eq!(result.pos(), 3);
    assert!(result.deleted_across(), "it was strictly inside");
    assert!(result.deleted());

    let edge = map.map_result(7, 1);
    assert_eq!(edge.pos(), 3);
    assert!(!edge.deleted_across(), "the far edge was not inside");
}

#[test]
fn a_map_inverts() {
    let map = StepMap::single(3, 0, 2);
    let back = map.invert();
    assert_eq!(back.map(map.map(5, 1), 1), 5);
}

#[test]
fn a_mapping_carries_a_position_through_a_sequence_of_changes() {
    let mut mapping = Mapping::new();
    mapping.append_map(StepMap::single(0, 0, 5), None); // insert 5 at the front
    mapping.append_map(StepMap::single(10, 3, 0), None); // delete 3 at 10
    assert_eq!(mapping.map(2, 1), 7);
    assert_eq!(mapping.map(20, 1), 22);
}

#[test]
fn a_position_deleted_and_restored_comes_back_where_it_was() {
    // The undo case: a deletion, then its own inverse. Without the mirror the
    // position would be stranded at the edge of the hole.
    let deletion = StepMap::single(3, 4, 0);
    let mut mapping = Mapping::new();
    mapping.append_map(deletion.clone(), None);
    mapping.append_map(deletion.invert(), Some(0));
    assert_eq!(
        mapping.map(5, 1),
        5,
        "the position was inside the deletion and the deletion was undone"
    );
}

// ---------------------------------------------------------------------------
// Rebasing — mapping a step, not just a position
// ---------------------------------------------------------------------------

#[test]
fn a_step_maps_through_a_concurrent_change_before_it() {
    let b = b();
    let doc = two_paragraphs();
    // Someone else inserts "XX" at position 1, ahead of our edit at 6..9.
    let theirs = Step::replace(1, 1, Slice::new(Fragment::from(b.text("XX")), 0, 0));
    let ours = Step::replace(6, 9, Slice::new(Fragment::from(b.text("TWO")), 0, 0));

    let after_theirs = theirs.apply(&doc).unwrap();
    let mapping = Mapping::from_maps(vec![theirs.step_map()]);
    let rebased = ours.map(&mapping).expect("our edit still has a target");
    let result = rebased.apply(&after_theirs).unwrap();

    assert_eq!(
        result.to_string(),
        r#"doc(paragraph("XXone"), paragraph("TWO"))"#
    );
}

#[test]
fn a_step_whose_target_was_deleted_maps_to_nothing() {
    let b = b();
    // They delete the whole second paragraph; our edit inside it has no home.
    let theirs = Step::replace(5, 10, Slice::empty());
    let ours = Step::replace(7, 8, Slice::new(Fragment::from(b.text("X")), 0, 0));
    let mapping = Mapping::from_maps(vec![theirs.step_map()]);
    assert_eq!(ours.map(&mapping), None);
}

// ---------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------

#[test]
fn consecutive_typing_merges_into_one_step() {
    let b = b();
    let first = Step::replace(1, 1, Slice::new(Fragment::from(b.text("a")), 0, 0));
    let second = Step::replace(2, 2, Slice::new(Fragment::from(b.text("b")), 0, 0));
    let merged = first.merge(&second).expect("adjacent insertions merge");
    let Step::Replace {
        from, to, slice, ..
    } = &merged
    else {
        panic!("expected a replace");
    };
    assert_eq!((*from, *to), (1, 1));
    assert_eq!(slice.content().text_content(), "ab");
}

#[test]
fn backspacing_merges_backwards() {
    let first = Step::replace(5, 6, Slice::empty());
    let second = Step::replace(4, 5, Slice::empty());
    let merged = first.merge(&second).expect("adjacent deletions merge");
    let Step::Replace { from, to, .. } = &merged else {
        panic!("expected a replace");
    };
    assert_eq!((*from, *to), (4, 6));
}

#[test]
fn steps_that_are_not_adjacent_do_not_merge() {
    let b = b();
    let first = Step::replace(1, 1, Slice::new(Fragment::from(b.text("a")), 0, 0));
    let far = Step::replace(9, 9, Slice::new(Fragment::from(b.text("b")), 0, 0));
    assert_eq!(first.merge(&far), None);
}

// ---------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------

#[test]
fn a_transform_records_what_it_did_and_can_take_it_back() {
    let b = b();
    let doc = two_paragraphs();
    let mut tr = Transform::new(basic::schema(), doc.clone());
    tr.delete(2, 3).unwrap();
    tr.replace(2, 2, Slice::new(Fragment::from(b.text("XY")), 0, 0))
        .unwrap();

    assert!(tr.doc_changed());
    assert_eq!(tr.steps().len(), 2);
    assert_eq!(tr.before(), &doc);
    assert_eq!(tr.doc().child(0).unwrap().text_content(), "oXYe");

    // Undo: the inverted steps, applied in order, restore the document.
    let mut back = tr.doc().clone();
    for step in tr.invert() {
        back = step.apply(&back).expect("the inverse applies");
    }
    assert_eq!(back, doc);
}

#[test]
fn a_transform_maps_a_position_across_everything_it_did() {
    let b = b();
    let mut tr = Transform::new(basic::schema(), two_paragraphs());
    tr.replace(1, 1, Slice::new(Fragment::from(b.text("XX")), 0, 0))
        .unwrap();
    tr.replace(3, 3, Slice::new(Fragment::from(b.text("YY")), 0, 0))
        .unwrap();
    // A caret that was at the start of the second paragraph.
    assert_eq!(tr.mapping().map(6, 1), 10);
}

#[test]
fn a_step_that_does_not_apply_leaves_the_transform_alone() {
    let mut tr = Transform::new(basic::schema(), two_paragraphs());
    // Deleting every block would leave `doc` — which is `block+` — empty.
    let err = tr.delete(0, 10).unwrap_err();
    assert!(
        !tr.doc_changed(),
        "a failed step must change nothing: {err}"
    );
}

#[test]
fn a_structural_replace_refuses_to_swallow_content() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("inside")])]
    )]);
    // Collapsing 0..1 (the quote's opening) is fine as a structural step only
    // if nothing lies between the ends. Reaching into the text is not.
    let bad = Step::replace_structure(0, 3, Slice::empty());
    assert!(bad.apply(&doc).is_err());
}
