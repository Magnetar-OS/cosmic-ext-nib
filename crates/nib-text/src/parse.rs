// SPDX-License-Identifier: MPL-2.0

//! Plain text into a document.
//!
//! Blank-line-separated paragraphs, and quoted runs as blockquotes. Nothing
//! cleverer: a plain-text body that looks like Markdown is usually a plain-text
//! body, and turning `*` into emphasis because it happened to be there is how
//! a shell command becomes italics.

use nib_model::basic::nodes;
use nib_model::fragment::Fragment;
use nib_model::mark::Marks;
use nib_model::node::Node;
use nib_model::schema::Schema;

use crate::quote::{Block, blocks};

/// Reads plain text into a document.
#[must_use]
pub fn parse(schema: &Schema, text: &str) -> Node {
    let mut content: Vec<Node> = Vec::new();
    for block in blocks(text) {
        match block {
            // A signature is prose that happens to be at the bottom; the
            // separator itself is not content.
            Block::Prose(text) | Block::Signature(text) => {
                content.extend(paragraphs(schema, &text));
            }
            Block::Quoted { text, .. } => {
                let inner = paragraphs(schema, &text);
                if inner.is_empty() {
                    continue;
                }
                if let Some(typ) = schema.node_id(nodes::BLOCKQUOTE)
                    && let Ok(node) = schema.create(
                        typ,
                        None,
                        Fragment::from_vec(inner.clone()),
                        Marks::none(),
                    )
                {
                    content.push(node);
                } else {
                    content.extend(inner);
                }
            }
        }
    }
    schema
        .create_and_fill(
            schema.top_node_type(),
            None,
            Fragment::from_vec(content),
            Marks::none(),
        )
        .unwrap_or_else(|| schema.empty_doc())
}

/// One paragraph per blank-line-separated run, with single newlines kept as
/// hard breaks — a hard-wrapped body would otherwise lose its shape.
fn paragraphs(schema: &Schema, text: &str) -> Vec<Node> {
    let Some(paragraph) = schema.node_id(nodes::PARAGRAPH) else {
        return Vec::new();
    };
    let hard_break = schema.node_id(nodes::HARD_BREAK);

    text.split("\n\n")
        .filter(|run| !run.trim().is_empty())
        .filter_map(|run| {
            let mut inline: Vec<Node> = Vec::new();
            for (i, line) in run.lines().enumerate() {
                if i > 0 && let Some(typ) = hard_break
                    && let Ok(node) =
                        schema.create(typ, None, Fragment::empty(), Marks::none())
                {
                    inline.push(node);
                }
                if !line.is_empty() {
                    inline.push(schema.text(line, Marks::none()));
                }
            }
            schema
                .create(
                    paragraph,
                    None,
                    Fragment::from_vec(inline),
                    Marks::none(),
                )
                .ok()
        })
        .collect()
}
