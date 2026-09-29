// SPDX-License-Identifier: MPL-2.0

//! HTML in, HTML out, and what happens to HTML that is not what it claims.

use nib_html::Html;
use nib_model::basic;

fn html() -> Html {
    Html::new(&basic::schema())
}

/// Parses and re-serialises; the result should be the input for anything the
/// schema fully understands.
fn round(input: &str) -> String {
    let html = html();
    html.to_html(&html.parse(input))
}

fn shape(input: &str) -> String {
    html().parse(input).to_string()
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

#[test]
fn paragraphs_and_inline_marks_survive() {
    for input in [
        "<p>plain</p>",
        "<p>Hello <strong>there</strong></p>",
        "<p><em>emphasis</em> and <strong>strength</strong></p>",
        "<p>a<br />b</p>",
        r#"<p><a href="http://example.test">link</a></p>"#,
        "<p><code>code</code></p>",
        "<p><s>gone</s></p>",
        "<p><u>under</u></p>",
    ] {
        assert_eq!(round(input), input, "round trip of {input}");
    }
}

#[test]
fn block_structure_survives() {
    for input in [
        "<h1>Title</h1>",
        "<h3>Deeper</h3>",
        "<blockquote><p>quoted</p></blockquote>",
        "<ul><li><p>one</p></li><li><p>two</p></li></ul>",
        "<ol><li><p>first</p></li></ol>",
        r#"<ol start="3"><li><p>third</p></li></ol>"#,
        "<hr />",
        "<pre><code>fn main() {}</code></pre>",
        r#"<pre><code class="language-rust">fn main() {}</code></pre>"#,
    ] {
        assert_eq!(round(input), input, "round trip of {input}");
    }
}

#[test]
fn tables_survive() {
    let input = "<table><tr><th><p>h</p></th></tr><tr><td><p>c</p></td></tr></table>";
    assert_eq!(round(input), input);
}

#[test]
fn an_image_keeps_its_attributes() {
    // Attributes come out in name order rather than source order: two exports
    // of one document have to be byte-identical, or a diff of them shows the
    // serialiser's mood rather than the author's edit.
    let doc = html().parse(r#"<p><img src="cid:1" alt="a cat" /></p>"#);
    assert_eq!(
        html().to_html(&doc),
        r#"<p><img alt="a cat" src="cid:1" /></p>"#
    );
}

#[test]
fn nested_marks_come_out_in_schema_order_however_they_went_in() {
    // Both spellings are the same document, so both serialise the same way.
    let a = round("<p><strong><em>x</em></strong></p>");
    let b = round("<p><em><strong>x</strong></em></p>");
    assert_eq!(a, b);
    assert_eq!(a, "<p><em><strong>x</strong></em></p>");
}

#[test]
fn tag_aliases_are_read_and_written_canonically() {
    assert_eq!(round("<p><b>x</b></p>"), "<p><strong>x</strong></p>");
    assert_eq!(round("<p><i>x</i></p>"), "<p><em>x</em></p>");
    assert_eq!(round("<p><del>x</del></p>"), "<p><s>x</s></p>");
}

// ---------------------------------------------------------------------------
// HTML that is not well-behaved
// ---------------------------------------------------------------------------

#[test]
fn unclosed_tags_are_recovered_by_the_parser() {
    assert_eq!(
        shape("<p>one<p>two"),
        r#"doc(paragraph("one"), paragraph("two"))"#
    );
    assert_eq!(
        shape("<ul><li>one<li>two</ul>"),
        concat!(
            r#"doc(bullet_list(list_item(paragraph("one")), "#,
            r#"list_item(paragraph("two"))))"#
        )
    );
}

#[test]
fn wrapper_elements_are_seen_through() {
    assert_eq!(
        shape("<div><section><p>text</p></section></div>"),
        r#"doc(paragraph("text"))"#
    );
}

#[test]
fn scripts_and_styles_are_dropped_whole() {
    assert_eq!(
        shape("<p>before</p><script>alert(1)</script><p>after</p>"),
        r#"doc(paragraph("before"), paragraph("after"))"#
    );
    assert_eq!(
        shape("<style>p{color:red}</style><p>text</p>"),
        r#"doc(paragraph("text"))"#
    );
}

#[test]
fn bare_text_lands_in_a_paragraph() {
    assert_eq!(shape("just text"), r#"doc(paragraph("just text"))"#);
}

#[test]
fn a_list_item_outside_a_list_gets_a_list_built_around_it() {
    assert_eq!(
        shape("<li>orphan</li>"),
        r#"doc(bullet_list(list_item(paragraph("orphan"))))"#
    );
}

#[test]
fn a_table_gets_its_implied_body_seen_through() {
    assert_eq!(
        shape("<table><tbody><tr><td>x</td></tr></tbody></table>"),
        concat!(
            r#"doc(table(table_row(table_cell[colspan=1, rowspan=1]"#,
            r#"(paragraph("x")))))"#
        )
    );
}

#[test]
fn whitespace_between_blocks_is_not_content() {
    assert_eq!(
        shape("<p>one</p>\n\n   <p>two</p>"),
        r#"doc(paragraph("one"), paragraph("two"))"#
    );
}

#[test]
fn whitespace_inside_a_paragraph_collapses() {
    assert_eq!(
        shape("<p>one   two\n\nthree</p>"),
        r#"doc(paragraph("one two three"))"#
    );
}

#[test]
fn whitespace_in_a_code_block_is_kept() {
    let doc = html().parse("<pre><code>fn main() {\n    ok\n}</code></pre>");
    assert_eq!(doc.text_content(), "fn main() {\n    ok\n}");
}

#[test]
fn marks_the_schema_forbids_are_dropped_where_they_land() {
    let doc = html().parse("<pre><code><strong>bold</strong> code</code></pre>");
    assert_eq!(doc.to_string(), r#"doc(code_block("bold code"))"#);
    assert_eq!(doc.check(), Ok(()));
}

#[test]
fn everything_parsed_is_a_valid_document() {
    for input in [
        "<p>a<p>b",
        "<ul><li><ul><li>deep</li></ul></li></ul>",
        "<blockquote><ul><li>quoted list</li></ul></blockquote>",
        "<table><tr><td><ul><li>cell list</li></ul></td></tr></table>",
        "<h1><strong>bold heading</strong></h1>",
        "<p><img src=\"x\"></p><hr><p>after</p>",
        "<div><span><b><i>deeply nested</i></b></span></div>",
        "",
        "<html><body></body></html>",
    ] {
        let doc = html().parse(input);
        assert_eq!(doc.check(), Ok(()), "parsing {input:?} gave {doc}");
    }
}

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

#[test]
fn text_is_escaped_on_the_way_out() {
    let doc = html().parse("<p>a &lt; b &amp; c</p>");
    assert_eq!(doc.text_content(), "a < b & c");
    assert_eq!(html().to_html(&doc), "<p>a &lt; b &amp; c</p>");
}

#[test]
fn attribute_values_are_escaped() {
    let doc = html().parse(r#"<p><a href="x?a=1&amp;b=2">link</a></p>"#);
    assert_eq!(
        html().to_html(&doc),
        r#"<p><a href="x?a=1&amp;b=2">link</a></p>"#
    );
}

// ---------------------------------------------------------------------------
// Slices, for the clipboard
// ---------------------------------------------------------------------------

#[test]
fn pasted_inline_html_is_an_inline_slice() {
    let slice = html().parse_slice("some <strong>text</strong>");
    assert_eq!(slice.content().child_count(), 2);
    assert!(
        slice
            .content()
            .child(0)
            .is_some_and(nib_model::Node::is_text),
        "inline HTML should not arrive wrapped in a block"
    );
}

#[test]
fn pasted_block_html_is_a_block_slice() {
    let slice = html().parse_slice("<p>one</p><p>two</p>");
    assert_eq!(slice.content().child_count(), 2);
    assert!(
        slice
            .content()
            .child(0)
            .is_some_and(nib_model::Node::is_block)
    );
}

#[test]
fn a_slice_written_back_keeps_its_marks() {
    let h = html();
    let slice = h.parse_slice("some <strong>text</strong>");
    assert_eq!(h.slice_to_html(&slice), "some <strong>text</strong>");
}

// ---------------------------------------------------------------------------
// Authored styling
// ---------------------------------------------------------------------------

use nib_model::basic::marks as m;

/// Every mark on the first text node of the document.
fn first_text_marks(doc: &nib_model::node::Node) -> Vec<String> {
    fn walk(node: &nib_model::node::Node, out: &mut Option<Vec<String>>) {
        if out.is_some() {
            return;
        }
        if node.text().is_some() {
            *out = Some(node.marks().iter().map(|k| k.name().to_owned()).collect());
            return;
        }
        for child in node.content() {
            walk(child, out);
        }
    }
    let mut out = None;
    walk(doc, &mut out);
    out.unwrap_or_default()
}

#[test]
fn a_colour_on_a_span_reaches_its_text() {
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p><span style="color: #cc0000">red</span></p>"#);
    assert!(first_text_marks(&doc).contains(&m::TEXT_COLOR.to_owned()));
}

#[test]
fn a_colour_on_a_transparent_element_still_reaches_its_text() {
    // The case a tag-list would miss: `<div>` has no mark of its own, and its
    // colour still belongs to the text inside it.
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<div style="color: #cc0000"><p>red</p></div>"#);
    assert!(first_text_marks(&doc).contains(&m::TEXT_COLOR.to_owned()));
}

#[test]
fn css_weight_and_style_become_the_marks_that_already_exist() {
    // Not a second way to say bold: `font-weight: bold` and `<b>` produce the
    // same document, so they serialise to the same HTML.
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p><span style="font-weight:bold; font-style:italic">x</span></p>"#);
    let marks = first_text_marks(&doc);
    assert!(marks.contains(&m::STRONG.to_owned()), "{marks:?}");
    assert!(marks.contains(&m::EM.to_owned()), "{marks:?}");
    // Nested in the schema's mark order, which is fixed so that a diff of two
    // exports shows what changed rather than how the marks were sorted.
    assert_eq!(html.to_html(&doc), "<p><em><strong>x</strong></em></p>");
}

#[test]
fn the_presentational_attributes_are_read_too() {
    // Mail generators target twenty-year-old clients; these are not historical
    // curiosities in an inbox.
    let html = Html::new(&basic::schema());
    let doc = html.parse(r##"<p><font color="#cc0000" size="5">big red</font></p>"##);
    let marks = first_text_marks(&doc);
    assert!(marks.contains(&m::TEXT_COLOR.to_owned()), "{marks:?}");
    assert!(marks.contains(&m::FONT_SIZE.to_owned()), "{marks:?}");
}

#[test]
fn a_style_declaration_beats_the_attribute_beside_it() {
    let html = Html::new(&basic::schema());
    let doc = html.parse(r##"<p><font color="#00ff00" style="color:#cc0000">x</font></p>"##);
    let out = html.to_html(&doc);
    assert!(out.contains("#cc0000"), "{out}");
    assert!(!out.contains("#00ff00"), "{out}");
}

#[test]
fn a_colour_round_trips_as_one_normalised_spelling() {
    // `red`, `#f00` and `rgb(255,0,0)` are one colour. A document in which
    // they are three values is one where two identical spans will not merge.
    let html = Html::new(&basic::schema());
    for spelling in ["red", "#f00", "#ff0000", "rgb(255, 0, 0)"] {
        let doc = html.parse(&format!(
            r#"<p><span style="color: {spelling}">x</span></p>"#
        ));
        assert_eq!(
            html.to_html(&doc),
            r#"<p><span style="color: #ff0000">x</span></p>"#,
            "{spelling} did not normalise"
        );
    }
}

#[test]
fn alignment_lands_on_the_block_not_the_text() {
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p style="text-align: center">middle</p>"#);
    let paragraph = doc.child(0).expect("a paragraph");
    assert_eq!(
        paragraph.attrs().get_str(nib_model::basic::attrs::ALIGN),
        Some("center")
    );
}

#[test]
fn a_style_that_would_fetch_leaves_no_trace_in_the_document() {
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p style="background: url(https://tracker.example/p.gif)">x</p>"#);
    let out = html.to_html(&doc);
    assert!(!out.contains("tracker.example"), "{out}");
    assert!(!out.contains("url("), "{out}");
}

#[test]
fn a_style_that_would_hide_is_not_obeyed() {
    // The text stays readable. Honouring this would reintroduce the attack
    // that text extraction exists to expose.
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p><span style="display:none">secret</span></p>"#);
    assert_eq!(doc.text_content(), "secret");
}

#[test]
fn a_font_size_goes_out_as_the_ratio_it_came_in_as() {
    let html = Html::new(&basic::schema());
    let doc = html.parse(r#"<p><span style="font-size: 150%">big</span></p>"#);
    assert_eq!(
        html.to_html(&doc),
        r#"<p><span style="font-size: 150%">big</span></p>"#
    );
}

#[test]
fn alignment_goes_back_out_on_the_block_it_came_in_on() {
    for input in [
        r#"<p style="text-align: center">middle</p>"#,
        r#"<h2 style="text-align: right">heading</h2>"#,
    ] {
        assert_eq!(round(input), input);
    }
    // The attribute spelling is read, and written as the declaration.
    assert_eq!(
        round(r#"<p align="justify">x</p>"#),
        r#"<p style="text-align: justify">x</p>"#
    );
}

#[test]
fn a_link_that_would_run_script_or_open_a_local_file_keeps_its_text_not_its_target() {
    // Mail HTML is untrusted, and a link's target is handed to the desktop's
    // URL opener when it is clicked.
    for href in [
        "javascript:alert(1)",
        " JavaScript:alert(1)",
        "java&#9;script:alert(1)",
        "java&#10;script:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///home/user/.bashrc",
        "smb://attacker.example/share",
    ] {
        let doc = html().parse(&format!(r#"<p><a href="{href}">x</a></p>"#));
        assert_eq!(
            doc.to_string(),
            r#"doc(paragraph("x"))"#,
            "{href:?} survived"
        );
    }
    for href in [
        "https://example.test/",
        "HTTP://example.test/",
        "mailto:ada@example.test",
        "tel:+15550100",
        "#section",
        "/relative/path",
        "page.html?at=12:30",
    ] {
        let input = format!(r#"<p><a href="{href}">x</a></p>"#);
        assert_eq!(round(&input), input, "{href:?} was dropped");
    }
}

#[test]
fn nesting_deeper_than_any_real_message_neither_overflows_nor_loses_the_text() {
    // Fifty kilobytes of markup, parsed on a thread with the stack an
    // async runtime's worker gets. Before the depth cap this aborted the
    // process: a stack overflow is not a panic, so nothing above can catch it.
    for (open, close) in [
        ("<div>", "</div>"),
        ("<blockquote>", "</blockquote>"),
        ("<span>", "</span>"),
        ("<ul><li>", "</li></ul>"),
    ] {
        let source = format!("{}deep{}", open.repeat(10_000), close.repeat(10_000));
        let text = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || html().parse(&source).text_content())
            .expect("a thread")
            .join()
            .expect("the parse finished");
        assert_eq!(text, "deep", "{open}");
    }
}

#[test]
fn a_script_below_the_depth_cap_is_still_dropped() {
    let source = format!(
        "{}<script>alert(1)</script>kept{}",
        "<div>".repeat(1_000),
        "</div>".repeat(1_000)
    );
    let text = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || html().parse(&source).text_content())
        .expect("a thread")
        .join()
        .expect("the parse finished");
    assert_eq!(text, "kept");
}

#[test]
fn inline_markup_outside_a_paragraph_keeps_its_marks() {
    // A browser puts a selection on the clipboard as bare inline HTML, and
    // mail writes list items without paragraphs; the paragraph is implicit,
    // and the marks belong to the text inside it.
    for (input, expected) in [
        ("<b>x</b>", r#"doc(paragraph(strong("x")))"#),
        ("<em>x</em> y", r#"doc(paragraph(em("x"), " y"))"#),
        (
            "<ul><li><b>x</b></li></ul>",
            r#"doc(bullet_list(list_item(paragraph(strong("x")))))"#,
        ),
        (
            "<blockquote><i>q</i></blockquote>",
            r#"doc(blockquote(paragraph(em("q"))))"#,
        ),
    ] {
        assert_eq!(html().parse(input).to_string(), expected, "{input}");
    }
    // A code block still holds no marks.
    assert_eq!(
        html().parse("<pre><code><b>x</b></code></pre>").to_string(),
        r#"doc(code_block("x"))"#
    );
}
