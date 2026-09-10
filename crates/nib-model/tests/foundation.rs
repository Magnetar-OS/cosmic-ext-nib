// SPDX-License-Identifier: MPL-2.0

//! The document model's foundation: schema compilation, node sizes, cuts,
//! mark sets, and the two searches (`fill_before`, `find_wrapping`) that every
//! structural edit is built on.

use nib_model::attrs::Attrs;
use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::content::ContentError;
use nib_model::fragment::Fragment;
use nib_model::mark::Marks;
use nib_model::schema::{AttrSpec, MarkSpec, NodeSpec, Schema, SchemaError};
use nib_model::{attrs, nodes};

fn b() -> Builder {
    Builder::new(basic::schema())
}

// ---------------------------------------------------------------------------
// Schema construction
// ---------------------------------------------------------------------------

#[test]
fn a_schema_needs_a_text_node_and_says_so() {
    let err = Schema::builder()
        .node("doc", NodeSpec::new().content("paragraph+"))
        .node("paragraph", NodeSpec::new())
        .build()
        .unwrap_err();
    assert_eq!(err, SchemaError::MissingText);
}

#[test]
fn the_text_node_must_be_inline_and_contentless() {
    let err = Schema::builder()
        .node("doc", NodeSpec::new().content("text*"))
        .node("text", NodeSpec::new())
        .build()
        .unwrap_err();
    assert_eq!(err, SchemaError::BadText);
}

#[test]
fn a_content_expression_naming_nothing_is_refused() {
    let err = Schema::builder()
        .node("doc", NodeSpec::new().content("nonesuch+"))
        .node("text", NodeSpec::new().inline().group("inline"))
        .build()
        .unwrap_err();
    let SchemaError::Content { source, .. } = err else {
        panic!("expected a content error, got {err:?}");
    };
    assert!(
        matches!(source, ContentError::UnknownName { .. }),
        "{source:?}"
    );
}

#[test]
fn a_required_position_only_text_can_fill_is_refused() {
    // `doc` would require an image, and no image can be generated without a
    // `src`. A schema like this compiles happily and then can never produce a
    // document, which is exactly the failure worth catching at build time.
    let err = Schema::builder()
        .node("doc", NodeSpec::new().content("image+"))
        .node(
            "image",
            NodeSpec::new()
                .inline()
                .group("inline")
                .attr("src", AttrSpec::required()),
        )
        .node("text", NodeSpec::new().inline().group("inline"))
        .build()
        .unwrap_err();
    let SchemaError::Content { source, .. } = err else {
        panic!("expected a content error, got {err:?}");
    };
    assert!(
        matches!(source, ContentError::NotGeneratable { .. }),
        "{source:?}"
    );
}

#[test]
fn duplicate_declarations_are_refused() {
    let err = Schema::builder()
        .node("text", NodeSpec::new().inline().group("inline"))
        .node("text", NodeSpec::new().inline().group("inline"))
        .build()
        .unwrap_err();
    assert_eq!(err, SchemaError::DuplicateNode("text".into()));
}

// ---------------------------------------------------------------------------
// Content expressions
// ---------------------------------------------------------------------------

#[test]
fn groups_expand_to_a_choice_of_their_members() {
    let schema = basic::schema();
    let doc = schema.node_type_by_name(nodes::DOC).unwrap();
    let start = doc.content_match();
    // `block+` admits every member of the block group and nothing else.
    for name in [
        nodes::PARAGRAPH,
        nodes::HEADING,
        nodes::BLOCKQUOTE,
        nodes::CODE_BLOCK,
        nodes::BULLET_LIST,
    ] {
        let id = schema.node_id(name).unwrap();
        assert!(start.match_type(id).is_some(), "{name} should be allowed");
    }
    let text = schema.node_id(nodes::TEXT).unwrap();
    assert!(start.match_type(text).is_none(), "text is not a block");
}

#[test]
fn plus_requires_one_and_star_does_not() {
    let schema = basic::schema();
    let doc = schema.node_type_by_name(nodes::DOC).unwrap();
    let paragraph = schema.node_type_by_name(nodes::PARAGRAPH).unwrap();
    // `block+` may not end immediately; `inline*` may.
    assert!(!doc.content_match().valid_end());
    assert!(paragraph.content_match().valid_end());
    let p = schema.node_id(nodes::PARAGRAPH).unwrap();
    assert!(doc.content_match().match_type(p).unwrap().valid_end());
}

#[test]
fn a_sequence_enforces_its_order() {
    // `list_item` is `paragraph block*`: a paragraph first, then anything.
    let schema = basic::schema();
    let item = schema.node_type_by_name(nodes::LIST_ITEM).unwrap();
    let paragraph = schema.node_id(nodes::PARAGRAPH).unwrap();
    let quote = schema.node_id(nodes::BLOCKQUOTE).unwrap();

    assert!(item.content_match().match_type(quote).is_none());
    let after_p = item.content_match().match_type(paragraph).unwrap();
    assert!(after_p.valid_end());
    assert!(after_p.match_type(quote).is_some());
}

#[test]
fn ranges_count() {
    let schema = Schema::builder()
        .node("doc", NodeSpec::new().content("paragraph{2,3}"))
        .node("paragraph", NodeSpec::new().content("inline*"))
        .node("text", NodeSpec::new().inline().group("inline"))
        .build()
        .unwrap();
    let p = schema.node_id("paragraph").unwrap();
    let m = schema.node_type_by_name("doc").unwrap().content_match();
    assert!(!m.valid_end(), "zero paragraphs is too few");
    let m1 = m.match_type(p).unwrap();
    assert!(!m1.valid_end(), "one paragraph is too few");
    let m2 = m1.match_type(p).unwrap();
    assert!(m2.valid_end(), "two is enough");
    let m3 = m2.match_type(p).unwrap();
    assert!(m3.valid_end(), "three is still fine");
    assert!(m3.match_type(p).is_none(), "four is too many");
}

#[test]
fn an_alternation_of_sequences_compiles() {
    let schema = Schema::builder()
        .node(
            "doc",
            NodeSpec::new().content("(heading paragraph) | paragraph+"),
        )
        .node("heading", NodeSpec::new().content("inline*"))
        .node("paragraph", NodeSpec::new().content("inline*"))
        .node("text", NodeSpec::new().inline().group("inline"))
        .build()
        .unwrap();
    let (h, p) = (
        schema.node_id("heading").unwrap(),
        schema.node_id("paragraph").unwrap(),
    );
    let start = schema.node_type_by_name("doc").unwrap().content_match();
    // heading then exactly one paragraph
    let after = start.match_type(h).unwrap().match_type(p).unwrap();
    assert!(after.valid_end());
    assert!(after.match_type(p).is_none());
    // or one or more paragraphs, and no heading after them
    let ps = start.match_type(p).unwrap().match_type(p).unwrap();
    assert!(ps.valid_end());
    assert!(ps.match_type(h).is_none());
}

// ---------------------------------------------------------------------------
// fill_before and find_wrapping — the two searches
// ---------------------------------------------------------------------------

#[test]
fn an_empty_list_item_is_filled_with_the_paragraph_it_requires() {
    let schema = basic::schema();
    let item = schema.node_id(nodes::LIST_ITEM).unwrap();
    let filled = schema
        .create_and_fill(item, None, Fragment::empty(), Marks::none())
        .expect("a list item can always be created");
    assert_eq!(filled.to_string(), "list_item(paragraph)");
}

#[test]
fn an_empty_document_is_the_top_node_filled() {
    assert_eq!(basic::schema().empty_doc().to_string(), "doc(paragraph)");
}

#[test]
fn wrapping_finds_the_chain_a_paragraph_needs_to_enter_a_list() {
    let schema = basic::schema();
    let list = schema.node_type_by_name(nodes::BULLET_LIST).unwrap();
    let paragraph = schema.node_id(nodes::PARAGRAPH).unwrap();
    let wrapping = list
        .content_match()
        .find_wrapping(&schema, paragraph)
        .expect("a paragraph belongs in a list item");
    let names: Vec<&str> = wrapping
        .iter()
        .map(|t| schema.node_type(*t).name())
        .collect();
    assert_eq!(names, [nodes::LIST_ITEM]);
}

#[test]
fn wrapping_is_empty_when_the_target_already_fits() {
    let schema = basic::schema();
    let doc = schema.node_type_by_name(nodes::DOC).unwrap();
    let paragraph = schema.node_id(nodes::PARAGRAPH).unwrap();
    assert_eq!(
        doc.content_match().find_wrapping(&schema, paragraph),
        Some(vec![])
    );
}

#[test]
fn wrapping_fails_when_nothing_would_help() {
    let schema = basic::schema();
    // Nothing can carry a block into a code block, whose content is `text*`.
    let code = schema.node_type_by_name(nodes::CODE_BLOCK).unwrap();
    let paragraph = schema.node_id(nodes::PARAGRAPH).unwrap();
    assert_eq!(code.content_match().find_wrapping(&schema, paragraph), None);
}

// ---------------------------------------------------------------------------
// Sizes and positions
// ---------------------------------------------------------------------------

#[test]
fn sizes_count_boundaries_and_bytes() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hi")])]);
    let paragraph = doc.child(0).unwrap();
    assert_eq!(
        paragraph.child(0).unwrap().node_size(),
        2,
        "\"hi\" is 2 bytes"
    );
    assert_eq!(paragraph.content_size(), 2);
    assert_eq!(paragraph.node_size(), 4, "2 bytes plus 2 boundaries");
    assert_eq!(doc.content_size(), 4);
    assert_eq!(doc.node_size(), 6);
}

#[test]
fn text_is_measured_in_utf8_bytes_not_characters() {
    let b = b();
    // Greek: two bytes a letter. An emoji: four.
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("γειά 👋")])]);
    let text = doc.child(0).unwrap().child(0).unwrap();
    assert_eq!(text.node_size(), "γειά 👋".len());
    assert_eq!(text.node_size(), 13);
}

#[test]
fn a_leaf_is_one_position_wide_and_an_empty_paragraph_is_not_a_leaf() {
    let b = b();
    let rule = b.node(nodes::HORIZONTAL_RULE, nodes![]);
    assert!(rule.is_leaf());
    assert_eq!(rule.node_size(), 1);

    let empty = b.node(nodes::PARAGRAPH, nodes![]);
    assert!(
        !empty.is_leaf(),
        "an empty paragraph still has a content expression"
    );
    assert_eq!(empty.node_size(), 2);
}

#[test]
fn node_at_descends_to_the_innermost_node() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    // 0 = before the first paragraph; 1 = inside it, at the text.
    assert_eq!(doc.node_at(0).unwrap().type_name(), nodes::PARAGRAPH);
    assert_eq!(doc.node_at(1).unwrap().text(), Some("one"));
    assert_eq!(doc.node_at(6).unwrap().text(), Some("two"));
}

// ---------------------------------------------------------------------------
// Cutting
// ---------------------------------------------------------------------------

#[test]
fn cutting_a_fragment_slices_the_text_nodes_at_the_edges() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])
    ]);
    // Positions inside the paragraph's content: 0..11 over "hello world".
    let paragraph = doc.child(0).unwrap();
    assert_eq!(paragraph.cut(0, 5).text_content(), "hello");
    assert_eq!(paragraph.cut(6, 11).text_content(), "world");
    assert_eq!(paragraph.cut(3, 8).text_content(), "lo wo");
}

#[test]
#[should_panic(expected = "splits a character")]
fn cutting_inside_a_character_panics_rather_than_rounding() {
    let b = b();
    let text = b.text("γ");
    // One byte into a two-byte letter.
    let _ = text.cut(0, 1);
}

#[test]
fn cutting_across_blocks_keeps_the_partial_blocks() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    // doc content: [0] <p> 1 "one" 4 </p> [5] <p> 6 "two" 9 </p> [10]
    let cut = doc.content().cut(2, 8);
    assert_eq!(cut.child_count(), 2);
    assert_eq!(cut.child(0).unwrap().text_content(), "ne");
    assert_eq!(cut.child(1).unwrap().text_content(), "tw");
}

// ---------------------------------------------------------------------------
// Text extraction
// ---------------------------------------------------------------------------

#[test]
fn text_between_separates_blocks_when_asked() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("one")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("two")]),
    ]);
    assert_eq!(doc.text_content(), "onetwo");
    assert_eq!(
        doc.text_between(0, doc.content_size(), Some("\n"), None),
        "one\ntwo"
    );
}

// ---------------------------------------------------------------------------
// Marks
// ---------------------------------------------------------------------------

#[test]
fn marks_are_kept_in_schema_rank_order_however_they_are_added() {
    let schema = basic::schema();
    let em = schema.mark(marks::EM, None).unwrap();
    let strong = schema.mark(marks::STRONG, None).unwrap();

    let a = strong.add_to_set(&em.add_to_set(&Marks::none()));
    let b = em.add_to_set(&strong.add_to_set(&Marks::none()));
    assert_eq!(a, b, "`<b><i>` and `<i><b>` are the same document");
    let names: Vec<&str> = a.iter().map(nib_model::Mark::name).collect();
    assert_eq!(names, [marks::EM, marks::STRONG]);
}

#[test]
fn re_linking_replaces_rather_than_nests() {
    let schema = basic::schema();
    let first = schema
        .mark(marks::LINK, Some(&attrs! { "href" => "http://one" }))
        .unwrap();
    let second = schema
        .mark(marks::LINK, Some(&attrs! { "href" => "http://two" }))
        .unwrap();
    let set = second.add_to_set(&first.add_to_set(&Marks::none()));
    assert_eq!(set.len(), 1);
    assert_eq!(
        set.iter().next().unwrap().attrs().get_str("href"),
        Some("http://two")
    );
}

#[test]
fn code_excludes_everything_else() {
    let schema = basic::schema();
    let code = schema.mark(marks::CODE, None).unwrap();
    let em = schema.mark(marks::EM, None).unwrap();

    // Adding code to an emphasised run drops the emphasis.
    let set = code.add_to_set(&em.add_to_set(&Marks::none()));
    let names: Vec<&str> = set.iter().map(nib_model::Mark::name).collect();
    assert_eq!(names, [marks::CODE]);

    // And emphasis will not go on to code.
    let set = em.add_to_set(&set);
    let names: Vec<&str> = set.iter().map(nib_model::Mark::name).collect();
    assert_eq!(names, [marks::CODE]);
}

#[test]
fn adding_a_mark_that_is_already_there_changes_nothing() {
    let schema = basic::schema();
    let em = schema.mark(marks::EM, None).unwrap();
    let once = em.add_to_set(&Marks::none());
    assert_eq!(em.add_to_set(&once), once);
}

#[test]
fn a_code_block_admits_no_marks() {
    let schema = basic::schema();
    let code_block = schema.node_type_by_name(nodes::CODE_BLOCK).unwrap();
    let em = schema.mark_id(marks::EM).unwrap();
    assert!(!code_block.allows_mark_type(em));

    let paragraph = schema.node_type_by_name(nodes::PARAGRAPH).unwrap();
    assert!(paragraph.allows_mark_type(em));
}

// ---------------------------------------------------------------------------
// Structural sharing and equality
// ---------------------------------------------------------------------------

#[test]
fn adjacent_text_with_the_same_markup_merges_on_append() {
    let b = b();
    let merged = Fragment::from(b.text("hello ")).append(&Fragment::from(b.text("world")));
    assert_eq!(
        merged.child_count(),
        1,
        "one run, written twice, is one run"
    );
    assert_eq!(merged.child(0).unwrap().text(), Some("hello world"));
}

#[test]
fn adjacent_text_with_different_marks_does_not_merge() {
    let b = b();
    let plain = Fragment::from(b.text("hello "));
    let bold = Fragment::from_vec(b.mark(marks::STRONG, None, nodes![b.text("world")]));
    assert_eq!(plain.append(&bold).child_count(), 2);
}

#[test]
fn a_document_equals_an_identically_built_one() {
    let b = b();
    let build = || b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hi")])]);
    assert_eq!(build(), build());
}

#[test]
fn find_diff_start_reports_the_first_changed_byte() {
    let b = b();
    let before = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello")])]);
    let after = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("help")])]);
    // Positions: 0 is before the paragraph, 1 is its first character.
    assert_eq!(
        before.content().find_diff_start(after.content(), 0),
        Some(4),
        "\"hel\" is shared; the difference starts at the fourth character"
    );
}

#[test]
fn find_diff_start_is_none_for_identical_documents() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("same")])]);
    assert_eq!(doc.content().find_diff_start(doc.content(), 0), None);
}

// ---------------------------------------------------------------------------
// Validity
// ---------------------------------------------------------------------------

#[test]
fn check_accepts_a_well_formed_document() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::HEADING, nodes![b.text("Title")]),
        b.node(
            nodes::BULLET_LIST,
            nodes![b.node(
                nodes::LIST_ITEM,
                nodes![b.node(nodes::PARAGRAPH, nodes![b.text("one")])]
            )]
        ),
    ]);
    assert_eq!(doc.check(), Ok(()));
}

#[test]
fn create_checked_refuses_content_the_schema_forbids() {
    let schema = basic::schema();
    let b = Builder::new(schema.clone());
    let list = schema.node_id(nodes::BULLET_LIST).unwrap();
    let err = schema
        .create_checked(
            list,
            None,
            Fragment::from(b.node(nodes::PARAGRAPH, nodes![])),
            Marks::none(),
        )
        .unwrap_err();
    assert!(
        matches!(err, SchemaError::InvalidContent { .. }),
        "a paragraph is not a list item: {err:?}"
    );
}

#[test]
fn a_required_attribute_must_be_supplied() {
    let schema = basic::schema();
    let image = schema.node_id(nodes::IMAGE).unwrap();
    let err = schema
        .create(image, None, Fragment::empty(), Marks::none())
        .unwrap_err();
    assert_eq!(
        err,
        SchemaError::MissingAttr {
            owner: nodes::IMAGE.into(),
            attr: "src".into()
        }
    );

    let ok = schema.create(
        image,
        Some(&attrs! { "src" => "cid:1" }),
        Fragment::empty(),
        Marks::none(),
    );
    assert_eq!(ok.unwrap().attrs().get_str("src"), Some("cid:1"));
}

#[test]
fn defaults_fill_in_the_attributes_not_given() {
    let schema = basic::schema();
    let heading = schema.node_id(nodes::HEADING).unwrap();
    let node = schema
        .create(heading, None, Fragment::empty(), Marks::none())
        .unwrap();
    assert_eq!(node.attrs().get_int("level"), Some(1));

    let node = schema
        .create(
            heading,
            Some(&attrs! { "level" => 3_i64 }),
            Fragment::empty(),
            Marks::none(),
        )
        .unwrap();
    assert_eq!(node.attrs().get_int("level"), Some(3));
}

#[test]
fn unused_marks_and_specs_compile() {
    // A schema with a mark group and an explicit mark list on a node.
    let schema = Schema::builder()
        .node("doc", NodeSpec::new().content("paragraph+"))
        .node(
            "paragraph",
            NodeSpec::new().content("inline*").marks("formatting"),
        )
        .node("text", NodeSpec::new().inline().group("inline"))
        .mark("em", MarkSpec::new().group("formatting"))
        .mark("strong", MarkSpec::new().group("formatting"))
        .mark("secret", MarkSpec::new())
        .build()
        .unwrap();
    let paragraph = schema.node_type_by_name("paragraph").unwrap();
    assert!(paragraph.allows_mark_type(schema.mark_id("em").unwrap()));
    assert!(paragraph.allows_mark_type(schema.mark_id("strong").unwrap()));
    assert!(!paragraph.allows_mark_type(schema.mark_id("secret").unwrap()));
    let _ = Attrs::none();
}
