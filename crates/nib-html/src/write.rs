// SPDX-License-Identifier: MPL-2.0

//! HTML out.
//!
//! The inverse of the parse, from the same table, so a round trip is a
//! property rather than a hope. What it emits is deliberately plain: tags and
//! the attributes the schema declared, no classes, no inline styles, no
//! wrapper divs. A document is structure; how it looks is the reader's.
//!
//! # Marks come out as nesting
//!
//! Marks are a set on each inline node, and HTML has no such thing — so a run
//! of text carrying `strong` and `em` must be written as one inside the other.
//! The order is the schema's mark order, which is why that order is fixed:
//! serialising the same document twice writes the same bytes, and a diff of
//! two exports shows what changed rather than how the marks were sorted.

use nib_model::fragment::Fragment;
use nib_model::mark::Marks;
use nib_model::node::Node;
use nib_model::slice::Slice;

use crate::rules::{ElementAttrs, Rules, WriteRule};

/// Escapes text for a text node.
fn escape_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(ch),
        }
    }
}

/// Escapes a value for a double-quoted attribute.
fn escape_attr(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

fn open_tag(tag: &str, attrs: &ElementAttrs, void: bool, out: &mut String) {
    out.push('<');
    out.push_str(tag);
    for (name, value) in attrs {
        // Two private keys steer serialisation rather than becoming
        // attributes: one overrides the tag, one styles the inner wrapper.
        if name.starts_with("__") {
            continue;
        }
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape_attr(value, out);
        out.push('"');
    }
    if void {
        out.push_str(" />");
    } else {
        out.push('>');
    }
}

fn write_node(rules: &Rules, node: &Node, out: &mut String) {
    if let Some(text) = node.text() {
        escape_text(text, out);
        return;
    }
    let Some(rule) = rules.write_node(node) else {
        // No rule: write what is inside it rather than losing the text.
        write_fragment(rules, node.content(), out);
        return;
    };
    let attrs = rule
        .attrs
        .as_ref()
        .map_or_else(ElementAttrs::new, |f| f(node.attrs()));
    let tag = attrs
        .get("__tag")
        .cloned()
        .unwrap_or_else(|| rule.tag.clone());

    open_tag(&tag, &attrs, rule.void, out);
    if rule.void {
        return;
    }
    for (i, inner) in rule.inner.iter().enumerate() {
        let inner_attrs = if i == 0 {
            attrs
                .get("__inner_class")
                .map(|class| ElementAttrs::from([("class".to_owned(), class.clone())]))
                .unwrap_or_default()
        } else {
            ElementAttrs::new()
        };
        open_tag(inner, &inner_attrs, false, out);
    }
    write_fragment(rules, node.content(), out);
    for inner in rule.inner.iter().rev() {
        out.push_str("</");
        out.push_str(inner);
        out.push('>');
    }
    out.push_str("</");
    out.push_str(&tag);
    out.push('>');
}

/// Writes a run of siblings, opening and closing mark elements around the runs
/// that share them.
fn write_fragment(rules: &Rules, fragment: &Fragment, out: &mut String) {
    // Owned rather than borrowed: the mark set is derived per child, so a
    // reference into it would not outlive the loop iteration that made it.
    let mut open: Vec<nib_model::mark::Mark> = Vec::new();
    let mut open_rules: Vec<&WriteRule> = Vec::new();

    for child in fragment {
        let marks = if child.is_inline() {
            child.marks().clone()
        } else {
            Marks::none()
        };

        // Close every open mark the new node does not share, innermost first.
        let shared = open
            .iter()
            .zip(marks.iter())
            .take_while(|(a, b)| *a == *b)
            .count();
        while open.len() > shared {
            open.pop();
            if let Some(rule) = open_rules.pop() {
                out.push_str("</");
                out.push_str(&rule.tag);
                out.push('>');
            }
        }
        // Open the rest.
        for mark in marks.iter().skip(shared) {
            let Some(rule) = rules.write_marks.get(&mark.typ().id()) else {
                continue;
            };
            let attrs = rule
                .attrs
                .as_ref()
                .map_or_else(ElementAttrs::new, |f| f(mark.attrs()));
            open_tag(&rule.tag, &attrs, false, out);
            open.push(mark.clone());
            open_rules.push(rule);
        }
        write_node(rules, child, out);
    }
    while let Some(rule) = open_rules.pop() {
        out.push_str("</");
        out.push_str(&rule.tag);
        out.push('>');
    }
}

/// The document as HTML.
#[must_use]
pub fn to_html(rules: &Rules, doc: &Node) -> String {
    let mut out = String::new();
    write_fragment(rules, doc.content(), &mut out);
    out
}

/// A slice as HTML, for the clipboard.
///
/// Open edges are written as their content, without the block that was cut
/// into — which is what makes copying half a paragraph and pasting it into
/// another paragraph continue that paragraph.
#[must_use]
pub fn slice_to_html(rules: &Rules, slice: &Slice) -> String {
    let mut out = String::new();
    if slice.open_start() > 0
        && slice.content().child_count() == 1
        && let Some(only) = slice.content().child(0)
    {
        write_fragment(rules, only.content(), &mut out);
        return out;
    }
    write_fragment(rules, slice.content(), &mut out);
    out
}
