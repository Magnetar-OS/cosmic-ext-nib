// SPDX-License-Identifier: MPL-2.0

//! What the highlighter says about code, and what it declines to say.

use nib_highlight::Highlighter;
use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::decoration::Kind;
use nib_model::node::Node;
use nib_model::{attrs, nodes};

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn code(language: &str, text: &str) -> Node {
    let b = b();
    b.doc(nodes![b.attr_node(
        nodes::CODE_BLOCK,
        &attrs! { "language" => language },
        nodes![b.text(text)]
    )])
}

fn classes(doc: &Node) -> Vec<(usize, usize, String)> {
    Highlighter::new()
        .decorate(doc)
        .all()
        .iter()
        .filter_map(|d| match d.kind() {
            Kind::Inline(style) => Some((
                d.from(),
                d.to(),
                style.class.as_deref().unwrap_or("").to_owned(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_keyword_is_marked_as_one() {
    let doc = code("rust", "fn main() {}");
    let found = classes(&doc);
    // The code block's text starts at position 1.
    assert!(
        found.iter().any(|(from, to, class)| {
            *from == 1 && *to == 3 && class == "keyword"
        }),
        "expected `fn` to be a keyword: {found:?}"
    );
}

#[test]
fn a_string_and_a_comment_are_told_apart() {
    let doc = code("rust", "// note\nlet s = \"text\";");
    let found = classes(&doc);
    assert!(found.iter().any(|(_, _, c)| c == "comment"), "{found:?}");
    assert!(found.iter().any(|(_, _, c)| c == "string"), "{found:?}");
}

#[test]
fn every_decoration_lands_inside_the_block() {
    let source = "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}";
    let doc = code("rust", source);
    let block = doc.child(0).expect("the code block");
    let (from, to) = (1, 1 + block.content_size());
    for (a, b, class) in classes(&doc) {
        assert!(a >= from && b <= to, "{class} at {a}..{b} escaped {from}..{to}");
        assert!(a < b, "{class} at {a}..{b} is empty");
    }
}

#[test]
fn the_offsets_name_the_text_they_describe() {
    let doc = code("rust", "let value = 42;");
    let block = doc.child(0).expect("the code block");
    let text = block.text_content();
    for (from, to, class) in classes(&doc) {
        let slice = &text[from - 1..to - 1];
        if class == "number" {
            assert_eq!(slice, "42", "the number decoration should cover the number");
        }
    }
}

#[test]
fn several_languages_are_recognised() {
    let highlighter = Highlighter::new();
    for language in ["rust", "python", "javascript", "json", "html", "c"] {
        assert!(highlighter.knows(language), "{language} should be known");
    }
}

#[test]
fn an_extension_works_as_well_as_a_name() {
    let highlighter = Highlighter::new();
    assert!(highlighter.knows("rs"));
    assert!(highlighter.knows("py"));
}

#[test]
fn a_block_with_no_language_is_left_alone() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::CODE_BLOCK, nodes![b.text("fn main() {}")])]);
    assert!(classes(&doc).is_empty(), "a guess is worse than no colour");
}

#[test]
fn a_language_nobody_knows_is_left_alone() {
    let doc = code("nonesuch", "fn main() {}");
    assert!(classes(&doc).is_empty());
}

#[test]
fn prose_is_not_highlighted() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.text("fn main() is not code here")]
    )]);
    assert!(classes(&doc).is_empty());
}

#[test]
fn two_code_blocks_are_both_highlighted_in_their_own_coordinates() {
    let b = b();
    let doc = b.doc(nodes![
        b.attr_node(
            nodes::CODE_BLOCK,
            &attrs! { "language" => "rust" },
            nodes![b.text("fn a() {}")]
        ),
        b.attr_node(
            nodes::CODE_BLOCK,
            &attrs! { "language" => "rust" },
            nodes![b.text("fn b() {}")]
        ),
    ]);
    let found = classes(&doc);
    let first = doc.child(0).expect("first").node_size();
    assert!(found.iter().any(|(from, _, c)| *from == 1 && c == "keyword"));
    assert!(
        found.iter().any(|(from, _, c)| *from == first + 1 && c == "keyword"),
        "the second block's offsets start after the first: {found:?}"
    );
}
