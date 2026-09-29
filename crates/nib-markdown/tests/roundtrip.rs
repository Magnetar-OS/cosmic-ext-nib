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
fn a_quote_of_several_paragraphs_stays_one_quote() {
    let md = gfm();
    let input = "> a\n>\n> b\n";
    assert_eq!(round(&md, input), input);
    survives(&md, "- item\n\n  > a\n  >\n  > b\n");
}

#[test]
fn prose_that_would_read_as_syntax_survives() {
    let md = gfm();
    for input in [
        "a \\<b\\> c\n",
        "AT\\&amp;T\n",
        "1986\\. A good year\n",
        "1\\) one\n",
        "a \\~\\~b\\~\\~ c\n",
        "a  \n\\===\n",
        "| a | b |\n| --- | --- |\n| x \\| y | z |\n",
        "| a | b |\n| --- | --- |\n| `x \\| y` | z |\n",
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
fn a_list_inside_a_quote_keeps_its_markers_in_order() {
    let md = gfm();
    let input = "> - one\n> - two\n";
    assert_eq!(round(&md, input), input);
}

#[test]
fn a_code_block_is_not_read_as_a_component_or_a_module_line() {
    let mdx = components(Dialect::Mdx);
    let doc = mdx.parse("```js\nimport x from 'y'\n```\n");
    assert_eq!(
        doc.to_string(),
        r#"doc(code_block[language=js]("import x from 'y'"))"#
    );

    let mdc = components(Dialect::Mdc);
    let doc = mdc.parse("```\n::card\nhi\n::\n```\n");
    assert_eq!(doc.to_string(), r#"doc(code_block("::card\nhi\n::"))"#);

    // Nor does a fence inside a component's body end the component early.
    let doc = mdc.parse("::card\n```\n::\n```\n::\n");
    assert_eq!(
        doc.to_string(),
        r#"doc(component_block[name=card](code_block("::")))"#
    );
}

#[test]
fn a_link_that_would_run_script_or_open_a_local_file_keeps_its_text_not_its_target() {
    // A downloaded file is no more trusted than a mail, and a link's target is
    // handed to the desktop's URL opener when it is clicked.
    let md = gfm();
    for input in [
        "[x](javascript:alert(1))\n",
        "[x](<java\tscript:alert(1)>)\n",
        "[x](file:///home/user/.bashrc)\n",
        "[x](smb://attacker.example/share)\n",
        "[x](data:text/html,hi)\n",
    ] {
        assert_eq!(
            md.parse(input).to_string(),
            r#"doc(paragraph("x"))"#,
            "{input:?} kept its target"
        );
    }
    for input in [
        "[x](https://example.test/)\n",
        "[x](mailto:ada@example.test)\n",
        "[x](#section)\n",
    ] {
        assert_eq!(round(&md, input), input, "{input:?} was dropped");
    }
}

#[test]
fn nesting_deeper_than_any_real_document_neither_overflows_nor_loses_the_text() {
    // Parsed, written and dropped on a thread with the stack an async
    // runtime's worker gets. Before the depth cap fifty thousand `>` built a
    // document fifty thousand quotes deep, and dropping it aborted the
    // process: a stack overflow is not a panic, so nothing above can catch it.
    for source in [
        format!("{}deep\n", ">".repeat(50_000)),
        format!("{}deep\n", "- ".repeat(20_000)),
        format!("{}deep\n", "> - ".repeat(20_000)),
    ] {
        let (text, written) = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let md = gfm();
                let doc = md.parse(&source);
                (doc.text_content(), md.to_markdown(&doc))
            })
            .expect("a thread")
            .join()
            .expect("the parse finished");
        assert_eq!(text, "deep");
        assert!(written.trim_end().ends_with("deep"), "{written:?}");
    }
}

#[test]
fn nesting_within_the_depth_cap_keeps_its_structure() {
    let md = gfm();
    let input = "> > > - a\n";
    let deep = format!("{}x\n", "> ".repeat(100));
    for input in [input, deep.as_str()] {
        survives(&md, input);
    }
    assert_eq!(round(&md, &deep), deep);
}

#[test]
fn a_tight_list_item_keeps_its_marks() {
    // A tight list, the kind nearly everyone writes, has no paragraph event
    // around an item's text; the paragraph is implicit, and so were the marks
    // it lost.
    let md = gfm();
    assert_eq!(
        md.parse("- **bold** and *em*\n- [link](https://x.test) `code`\n")
            .to_string(),
        r#"doc(bullet_list(list_item(paragraph(strong("bold"), " and ", em("em"))), list_item(paragraph(link("link"), " ", code("code")))))"#
    );
}

// ---------------------------------------------------------------------------
// Found by the generated round trips (tests/generated.rs)
// ---------------------------------------------------------------------------

/// Builds a one-paragraph-per-entry document from the basic schema.
fn written_and_read(md: &Markdown, doc: &nib_model::node::Node) -> nib_model::node::Node {
    md.parse(&md.to_markdown(doc))
}

#[test]
fn code_keeps_its_trailing_spaces_and_gains_no_newline() {
    let md = gfm();
    survives(&md, "```\nkeep  \nthese   \n```\n");
    // Code typed in the editor has no newline at its end; saving and
    // reopening must not add one.
    let b = nib_model::build::Builder::new(basic::schema());
    let doc = b.doc(nib_model::nodes![b.node(
        nib_model::basic::nodes::CODE_BLOCK,
        nib_model::nodes![b.text("code")]
    )]);
    assert_eq!(written_and_read(&md, &doc), doc);
}

#[test]
fn two_lists_side_by_side_stay_two_lists() {
    let md = gfm();
    for input in ["- a\n\n* b\n", "1. a\n\n1) b\n", "- a\n\n* b\n\n- c\n"] {
        survives(&md, input);
        assert_eq!(
            md.parse(input).child_count(),
            input.matches("\n\n").count() + 1
        );
    }
}

#[test]
fn line_breaks_in_a_row_stay_in_their_paragraph() {
    // Two in a row as trailing spaces is a line of nothing but spaces, which
    // is a blank line, which ends the paragraph.
    let md = gfm();
    let b = nib_model::build::Builder::new(basic::schema());
    let br = || b.node(nib_model::basic::nodes::HARD_BREAK, nib_model::nodes![]);
    let doc = b.doc(nib_model::nodes![b.node(
        nib_model::basic::nodes::PARAGRAPH,
        nib_model::nodes![b.text("a"), br(), br(), b.text("b")]
    )]);
    assert_eq!(written_and_read(&md, &doc), doc);
}

#[test]
fn emphasis_no_delimiter_can_spell_is_written_as_tags_and_read_back() {
    let md = gfm();
    // `**` between a letter and punctuation cannot open, so no delimiter
    // spelling of this exists.
    let b = nib_model::build::Builder::new(basic::schema());
    let mut content = nib_model::nodes![b.text("word")];
    content.extend(b.mark(
        nib_model::basic::marks::STRONG,
        None,
        b.mark(
            nib_model::basic::marks::STRIKETHROUGH,
            None,
            nib_model::nodes![b.text("~bold~")],
        ),
    ));
    content.push(b.text("&x"));
    let doc = b.doc(nib_model::nodes![
        b.node(nib_model::basic::nodes::PARAGRAPH, content)
    ]);
    let written = md.to_markdown(&doc);
    assert!(written.contains("<strong>"), "{written}");
    assert_eq!(md.parse(&written), doc);
    // Where delimiters work, they are what is written.
    assert_eq!(round(&md, "a **b** c\n"), "a **b** c\n");
}

#[test]
fn inline_emphasis_tags_are_read_and_end_with_their_block() {
    let md = gfm();
    assert_eq!(
        md.parse("a <em>b</em> <b>c</b> <del>d</del>\n").to_string(),
        r#"doc(paragraph("a ", em("b"), " ", strong("c"), " ", strikethrough("d")))"#
    );
    // An unclosed tag does not reach the next block, and a stray closing tag
    // does not end emphasis a delimiter began.
    assert_eq!(
        md.parse("<b>a\n\nb **c</b>d**\n").to_string(),
        r#"doc(paragraph(strong("a")), paragraph("b ", strong("cd")))"#
    );
}

#[test]
fn escapes_that_read_back_as_other_things_are_escaped() {
    let md = gfm();
    for input in [
        // A `!` before a link would make it an image.
        "a \\![b](http://x.test)\n",
        // `#` at a heading's end would be its closing sequence.
        "# C \\#\n",
        // An entity in a destination, a title or an info string is decoded
        // when read.
        "[a](http://x.test/?q=\\&amp;)\n",
        "[a](http://x.test \"\\&amp;\")\n",
        "```\\&amp;\nx\n```\n",
    ] {
        survives(&md, input);
    }
}

#[test]
fn list_items_that_are_empty_or_hold_more_keep_their_shape() {
    let md = gfm();
    for input in [
        // An empty item inside a quote.
        "> - \n> - b\n",
        // A task item's second paragraph lines up under its text, not its
        // checkbox.
        "- [ ] a\n\n  b\n",
        // A nested list that cannot interrupt a paragraph.
        "- a\n\n  1. \n",
        "- a\n\n  3. b\n",
    ] {
        survives(&md, input);
    }
}

#[test]
fn whitespace_at_the_edge_of_emphasis_is_read_where_it_can_be_written() {
    // Markdown cannot put a space just inside a delimiter, so a document read
    // from Markdown never holds one there either.
    let md = gfm();
    assert_eq!(
        md.parse("_a <span>_ c\n").to_string(),
        r#"doc(paragraph(em("a"), "  c"))"#
    );
}
