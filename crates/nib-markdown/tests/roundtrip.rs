// SPDX-License-Identifier: MPL-2.0

//! Markdown in, Markdown out, and the two dialects on top of it.

use nib_markdown::{Dialect, Markdown, with_components};
use nib_model::basic;
use nib_model::schema::Schema;

fn gfm() -> Markdown {
    Markdown::new(&basic::schema(), Dialect::Gfm)
}

fn components(dialect: Dialect) -> Markdown {
    static SCHEMA: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    let schema = SCHEMA
        .get_or_init(|| with_components(&basic::schema()).expect("the component schema compiles"));
    Markdown::new(schema, dialect)
}

fn round(md: &Markdown, input: &str) -> String {
    md.to_markdown(&md.parse(input))
}

// ---------------------------------------------------------------------------
// CommonMark
// ---------------------------------------------------------------------------

#[test]
fn blocks_round_trip() {
    let md = gfm();
    for input in [
        "Just a paragraph.\n",
        "# Heading one\n",
        "### Heading three\n",
        "```\nplain code\n```\n",
        "```rust\nfn main() {}\n```\n",
        "---\n",
        "> A quotation.\n",
        "- one\n- two\n",
        "1. first\n2. second\n",
    ] {
        assert_eq!(round(&md, input), input, "round trip of {input:?}");
    }
}

#[test]
fn inline_marks_round_trip() {
    let md = gfm();
    for input in [
        "Some **bold** text.\n",
        "Some *emphasis* here.\n",
        "Some `code` here.\n",
        "Some ~~gone~~ text.\n",
        "A [link](http://example.test) here.\n",
        "An ![image](cat.png) here.\n",
    ] {
        assert_eq!(round(&md, input), input, "round trip of {input:?}");
    }
}

#[test]
fn a_table_round_trips() {
    let md = gfm();
    let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
    assert_eq!(round(&md, input), input);
}

#[test]
fn a_task_list_keeps_its_boxes() {
    let md = gfm();
    let input = "- [x] done\n- [ ] not done\n";
    assert_eq!(round(&md, input), input);
}

#[test]
fn nested_structure_parses_into_a_valid_document() {
    let md = gfm();
    let doc = md.parse(
        "# Title\n\n\
         > A quote with a list:\n>\n\
         > - one\n> - two\n\n\
         ```rust\nfn main() {}\n```\n",
    );
    assert_eq!(doc.check(), Ok(()));
    assert!(doc.to_string().contains("blockquote"));
    assert!(doc.to_string().contains("code_block[language=rust]"));
}

#[test]
fn a_nested_list_survives() {
    let md = gfm();
    let doc = md.parse("- one\n  - nested\n- two\n");
    assert_eq!(doc.check(), Ok(()));
    assert_eq!(
        doc.to_string(),
        concat!(
            r#"doc(bullet_list(list_item(paragraph("one"), "#,
            r#"bullet_list(list_item(paragraph("nested")))), "#,
            r#"list_item(paragraph("two"))))"#
        )
    );
}

#[test]
fn punctuation_that_would_read_as_syntax_is_escaped() {
    let md = gfm();
    let doc = md.parse("a \\* b\n");
    assert_eq!(doc.text_content(), "a * b");
    assert_eq!(md.to_markdown(&doc), "a \\* b\n");
}

#[test]
fn commonmark_leaves_the_github_extensions_alone() {
    let plain = Markdown::new(&basic::schema(), Dialect::CommonMark);
    // Without the table extension, the pipes are just text.
    let doc = plain.parse("| a | b |\n| --- | --- |\n");
    assert!(!doc.to_string().contains("table"), "{doc}");
}

// ---------------------------------------------------------------------------
// MDC
// ---------------------------------------------------------------------------

#[test]
fn an_mdc_block_component_becomes_a_node_with_its_props() {
    let md = components(Dialect::Mdc);
    let doc = md.parse("::card{title=\"Hello\" count=3 open}\nInside.\n::\n");
    assert_eq!(doc.check(), Ok(()));
    let component = doc.child(0).expect("a component");
    assert_eq!(component.type_name(), "component_block");
    assert_eq!(component.attrs().get_str("name"), Some("card"));
    assert_eq!(component.attrs().get_str("title"), Some("Hello"));
    assert_eq!(component.attrs().get_int("count"), Some(3));
    assert_eq!(component.attrs().get_bool("open"), Some(true));
    assert_eq!(component.text_content(), "Inside.");
}

#[test]
fn mdc_shorthands_become_class_and_id() {
    let md = components(Dialect::Mdc);
    let doc = md.parse("::note{.warning .big #first}\ntext\n::\n");
    let component = doc.child(0).expect("a component");
    assert_eq!(component.attrs().get_str("class"), Some("warning big"));
    assert_eq!(component.attrs().get_str("id"), Some("first"));
}

#[test]
fn an_mdc_component_round_trips() {
    let md = components(Dialect::Mdc);
    let input = "::card{title=\"Hello\"}\nInside.\n::\n";
    assert_eq!(round(&md, input), input);
}

#[test]
fn markdown_around_an_mdc_component_is_still_markdown() {
    let md = components(Dialect::Mdc);
    let doc = md.parse("# Before\n\n::card\ninner\n::\n\nAfter.\n");
    assert_eq!(doc.check(), Ok(()));
    let names: Vec<&str> = doc
        .content()
        .iter()
        .map(nib_model::Node::type_name)
        .collect();
    assert_eq!(names, ["heading", "component_block", "paragraph"]);
}

// ---------------------------------------------------------------------------
// MDX
// ---------------------------------------------------------------------------

#[test]
fn an_mdx_component_becomes_a_node() {
    let md = components(Dialect::Mdx);
    let doc = md.parse("<Card title=\"Hello\" />\n");
    assert_eq!(doc.check(), Ok(()));
    let component = doc.child(0).expect("a component");
    assert_eq!(component.attrs().get_str("name"), Some("Card"));
    assert_eq!(component.attrs().get_str("title"), Some("Hello"));
}

#[test]
fn mdx_module_lines_are_kept_verbatim() {
    let md = components(Dialect::Mdx);
    let doc = md.parse("import Card from './card'\n\n# Title\n");
    assert_eq!(doc.check(), Ok(()));
    let esm = doc.child(0).expect("the import");
    assert_eq!(esm.type_name(), "esm");
    assert_eq!(
        esm.attrs().get_str("source"),
        Some("import Card from './card'")
    );
}

#[test]
fn a_lowercase_tag_is_html_and_not_a_component() {
    let md = components(Dialect::Mdx);
    let doc = md.parse("<div>text</div>\n");
    assert!(!doc.to_string().contains("component"), "{doc}");
}

#[test]
fn an_mdx_component_round_trips() {
    let md = components(Dialect::Mdx);
    let input = "<Card title=\"Hello\" />\n";
    assert_eq!(round(&md, input), input);
}

// ---------------------------------------------------------------------------
// Round trips of content that looks like syntax
// ---------------------------------------------------------------------------

/// Parses, writes and parses again; the two documents must be the same.
///
/// Compared as documents rather than as text, because the writer may choose a
/// different spelling of the same thing — and must never choose a spelling of
/// a different thing.
fn survives(md: &Markdown, input: &str) {
    let doc = md.parse(input);
    let written = md.to_markdown(&doc);
    assert_eq!(
        md.parse(&written),
        doc,
        "{input:?} was written as {written:?}"
    );
}

#[test]
fn code_that_contains_its_own_delimiters_survives() {
    let md = gfm();
    for input in [
        "````\na\n```\nb\n````\n",
        "x `` a`b `` y\n",
        "x `` `a `` y\n",
        "x ``` a``b ``` y\n",
    ] {
        survives(&md, input);
    }
}

#[test]
fn link_and_image_targets_and_titles_survive() {
    let md = gfm();
    for input in [
        "[a](http://x.test \"The title\")\n",
        "[a](<http://x.test/a b>)\n",
        "[a](http://x.test/\\(b)\n",
        "[a](http://x.test \"say \\\"hi\\\"\")\n",
        "![a](x.png \"T\")\n",
        "![a \\] b](x.png)\n",
    ] {
        survives(&md, input);
    }
}

#[test]
fn a_code_block_is_not_read_as_a_component_or_a_module_line() {
    let mdx = components(Dialect::Mdx);
    let doc = mdx.parse("```js\nimport x from 'y'\n```\n");
    assert_eq!(
        doc.to_string(),
        r#"doc(code_block[language=js]("import x from 'y'\n"))"#
    );

    let mdc = components(Dialect::Mdc);
    let doc = mdc.parse("```\n::card\nhi\n::\n```\n");
    assert_eq!(doc.to_string(), r#"doc(code_block("::card\nhi\n::\n"))"#);

    // Nor does a fence inside a component's body end the component early.
    let doc = mdc.parse("::card\n```\n::\n```\n::\n");
    assert_eq!(
        doc.to_string(),
        r#"doc(component_block[name=card](code_block("::\n")))"#
    );
}
