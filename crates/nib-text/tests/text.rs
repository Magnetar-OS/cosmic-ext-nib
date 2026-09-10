// SPDX-License-Identifier: MPL-2.0

//! Plain text out, plain text in, and mail quoting.

use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::{attrs, nodes};
use nib_text::{Block, Options, Text, blocks, quote, wrap};

fn text() -> Text {
    Text::new(&basic::schema())
}

fn b() -> Builder {
    Builder::new(basic::schema())
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

#[test]
fn a_heading_is_underlined() {
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        &attrs! { "level" => 1_i64 },
        nodes![b.text("Title")]
    )]);
    assert_eq!(text().to_text(&doc), "Title\n=====");
}

#[test]
fn a_second_level_heading_uses_hyphens() {
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::HEADING,
        &attrs! { "level" => 2_i64 },
        nodes![b.text("Sub")]
    )]);
    assert_eq!(text().to_text(&doc), "Sub\n---");
}

#[test]
fn a_quote_is_prefixed() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::BLOCKQUOTE,
        nodes![b.node(nodes::PARAGRAPH, nodes![b.text("quoted")])]
    )]);
    assert_eq!(text().to_text(&doc), "> quoted");
}

#[test]
fn a_list_is_bulleted() {
    let b = b();
    let item = |t: &str| {
        b.node(
            nodes::LIST_ITEM,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(t)])],
        )
    };
    let doc = b.doc(nodes![
        b.node(nodes::BULLET_LIST, nodes![item("one"), item("two")])
    ]);
    assert_eq!(text().to_text(&doc), "- one\n- two");
}

#[test]
fn a_code_block_is_indented_not_fenced() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::CODE_BLOCK, nodes![b.text("fn main() {}")])
    ]);
    assert_eq!(
        text().to_text(&doc),
        "    fn main() {}",
        "backticks in plain text are punctuation; indentation is what reads as code"
    );
}

#[test]
fn a_link_target_is_written_out() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.mark(
            "link",
            Some(&attrs! { "href" => "http://example.test" }),
            nodes![b.text("here")]
        )]
    )]);
    assert_eq!(text().to_text(&doc), "here <http://example.test>");
}

#[test]
fn a_table_is_aligned_columns() {
    let b = b();
    let cell = |t: &str| {
        b.node(
            nodes::TABLE_CELL,
            nodes![b.node(nodes::PARAGRAPH, nodes![b.text(t)])],
        )
    };
    let doc = b.doc(nodes![b.node(
        nodes::TABLE,
        nodes![
            b.node(nodes::TABLE_ROW, nodes![cell("name"), cell("n")]),
            b.node(nodes::TABLE_ROW, nodes![cell("a"), cell("1")]),
        ]
    )]);
    assert_eq!(text().to_text(&doc), "name  n\n----  -\na     1");
}

// ---------------------------------------------------------------------------
// Wrapping
// ---------------------------------------------------------------------------

#[test]
fn lines_wrap_at_word_boundaries() {
    let wrapped = wrap("one two three four five", 10);
    assert_eq!(wrapped, ["one two", "three four", "five"]);
}

#[test]
fn a_word_longer_than_the_column_is_left_long() {
    let long = "http://example.test/a/very/long/path/that/will/not/fit";
    assert_eq!(wrap(long, 20), [long]);
}

#[test]
fn the_default_wrap_leaves_room_to_be_quoted_twice() {
    let b = b();
    let long = "word ".repeat(30);
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text(long.trim())])
    ]);
    let out = text().to_text(&doc);
    assert!(out.lines().all(|l| l.chars().count() <= 72), "{out}");
    // Two rounds of quoting still fit inside 78.
    let twice = quote(&quote(&out));
    assert!(twice.lines().all(|l| l.chars().count() <= 78), "{twice}");
}

// ---------------------------------------------------------------------------
// Quoting
// ---------------------------------------------------------------------------

#[test]
fn a_body_splits_into_prose_and_history() {
    let body = "My reply.\n\n> What they wrote.\n> More of it.";
    assert_eq!(
        blocks(body),
        [
            Block::Prose("My reply.".into()),
            Block::Quoted {
                depth: 1,
                text: "What they wrote.\nMore of it.".into()
            },
        ]
    );
}

#[test]
fn depth_ignores_the_spaces_between_markers() {
    for spelling in ["> > deep", ">> deep", "> >deep"] {
        let Some(Block::Quoted { depth, .. }) = blocks(spelling).first().cloned() else {
            panic!("expected a quote for {spelling:?}");
        };
        assert_eq!(depth, 2, "{spelling:?}");
    }
}

#[test]
fn a_run_that_dips_deeper_and_comes_back_is_one_piece_of_history() {
    let body = "> one\n> > two\n> three";
    let found = blocks(body);
    assert_eq!(found.len(), 1, "{found:?}");
    let Block::Quoted { depth, .. } = &found[0] else {
        panic!("expected a quote");
    };
    assert_eq!(*depth, 1, "the shallowest depth in the run");
}

#[test]
fn a_blank_line_inside_a_quote_stays_in_it() {
    let body = "> one\n\n> two";
    assert_eq!(blocks(body).len(), 1, "senders leave real blank lines");
}

#[test]
fn a_signature_ends_the_message() {
    let body = "The message.\n\n-- \nName\nTitle";
    assert_eq!(
        blocks(body),
        [
            Block::Prose("The message.".into()),
            Block::Signature("Name\nTitle".into()),
        ]
    );
}

#[test]
fn a_quoted_signature_marker_is_not_this_messages_signature() {
    let body = "Mine.\n\n> Theirs.\n> -- \n> Their name";
    let found = blocks(body);
    assert!(
        !found.iter().any(|b| matches!(b, Block::Signature(_))),
        "{found:?}"
    );
}

#[test]
fn quoting_a_body_prefixes_every_line_and_leaves_no_trailing_space() {
    assert_eq!(quote("one\n\ntwo"), "> one\n>\n> two");
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[test]
fn blank_lines_separate_paragraphs() {
    let doc = text().parse("one\n\ntwo");
    assert_eq!(
        doc.to_string(),
        r#"doc(paragraph("one"), paragraph("two"))"#
    );
}

#[test]
fn a_quoted_run_becomes_a_blockquote() {
    let doc = text().parse("mine\n\n> theirs");
    assert_eq!(
        doc.to_string(),
        r#"doc(paragraph("mine"), blockquote(paragraph("theirs")))"#
    );
    assert_eq!(doc.check(), Ok(()));
}

#[test]
fn a_hard_wrapped_paragraph_keeps_its_line_breaks() {
    let doc = text().parse("one\ntwo");
    assert_eq!(
        doc.to_string(),
        r#"doc(paragraph("one", hard_break, "two"))"#
    );
}

#[test]
fn plain_text_is_not_read_as_markdown() {
    let doc = text().parse("a * b and _c_");
    assert_eq!(doc.to_string(), r#"doc(paragraph("a * b and _c_"))"#);
}

#[test]
fn wrapping_is_optional() {
    let b = b();
    let long = "word ".repeat(40);
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text(long.trim())])
    ]);
    let unwrapped = Text::with_options(
        &basic::schema(),
        Options {
            wrap: None,
            ..Options::default()
        },
    );
    assert_eq!(unwrapped.to_text(&doc).lines().count(), 1);
}
