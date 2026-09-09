// SPDX-License-Identifier: MPL-2.0

//! The standard document schema.
//!
//! Paragraphs, headings, lists, quotes, code, rules, images, breaks; emphasis,
//! strong, underline, strikethrough, inline code, links. It is deliberately
//! the set every editor has, because the point of a default is to be the thing
//! nobody has to think about — an application that needs a mention chip or a
//! table adds it, and an application that needs *less* builds its own schema
//! from the same pieces.
//!
//! # Choices worth naming
//!
//! - **`code_block` allows no marks and no inline nodes but text.** Emphasis
//!   inside a code block is a formatting artefact of the editor that produced
//!   it, never something the author meant, and the schema is the only place
//!   that can say so once instead of everywhere.
//! - **`link` is not inclusive.** Typing at the end of a link continues the
//!   sentence, not the link. Every editor that gets this wrong produces
//!   documents where the trailing space is part of the URL.
//! - **`code` excludes everything.** A bold monospace run is two decisions
//!   fighting, and inline code is the one that means something.
//! - **Table cells are `isolating`.** A selection, a join or a lift that
//!   reached across a cell boundary would turn a grid into prose. Isolation is
//!   the schema saying so once, rather than every command checking.
//! - **`heading`, `blockquote`, `list_item` and `code_block` are `defining`.**
//!   Content lifted out of them keeps them: pasting a paragraph from a quote
//!   into a quote keeps one quote, not two, and pasting a list item into
//!   ordinary prose drops the item rather than smuggling it in.
//!
//! ```
//! # use nib_model::basic;
//! let schema = basic::schema();
//! let doc = schema.empty_doc();
//! assert_eq!(doc.to_string(), "doc(paragraph)");
//! ```

use crate::attrs::Value;
use crate::schema::{AttrSpec, MarkSpec, NodeSpec, Schema};

/// The node type names this schema declares.
pub mod nodes {
    pub const DOC: &str = "doc";
    pub const PARAGRAPH: &str = "paragraph";
    pub const HEADING: &str = "heading";
    pub const BLOCKQUOTE: &str = "blockquote";
    pub const CODE_BLOCK: &str = "code_block";
    pub const HORIZONTAL_RULE: &str = "horizontal_rule";
    pub const BULLET_LIST: &str = "bullet_list";
    pub const ORDERED_LIST: &str = "ordered_list";
    pub const LIST_ITEM: &str = "list_item";
    pub const TABLE: &str = "table";
    pub const TABLE_ROW: &str = "table_row";
    pub const TABLE_CELL: &str = "table_cell";
    pub const TABLE_HEADER: &str = "table_header";
    pub const TEXT: &str = "text";
    pub const IMAGE: &str = "image";
    pub const HARD_BREAK: &str = "hard_break";
}

/// The mark type names this schema declares, in rank order.
pub mod marks {
    pub const LINK: &str = "link";
    pub const EM: &str = "em";
    pub const STRONG: &str = "strong";
    pub const UNDERLINE: &str = "underline";
    pub const STRIKETHROUGH: &str = "strikethrough";
    pub const CODE: &str = "code";
}

/// The standard schema.
///
/// One instance, built on first use and shared thereafter. That is not a
/// micro-optimisation — node types are compared by identity, so a document
/// built against one `Schema` and a document built against another are not
/// interchangeable even when the two were declared identically. Handing out
/// the same instance makes the common case correct rather than making it a
/// rule to remember. Applications that build their own schema should hold one
/// and clone it (`Schema` is `Arc`-backed and cloning is a pointer bump) for
/// exactly the same reason.
///
/// # Panics
///
/// Never: the declarations below are fixed and are covered by this crate's
/// tests. A panic here is a bug in this function, not in its caller.
#[must_use]
pub fn schema() -> Schema {
    static SCHEMA: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(build).clone()
}

#[allow(clippy::too_many_lines)]
fn build() -> Schema {
    Schema::builder()
        .node(nodes::DOC, NodeSpec::new().content("block+"))
        .node(
            nodes::PARAGRAPH,
            NodeSpec::new().content("inline*").group("block"),
        )
        .node(
            nodes::HEADING,
            NodeSpec::new()
                .content("inline*")
                .group("block")
                .defining()
                .attr("level", AttrSpec::new(1_i64)),
        )
        .node(
            nodes::BLOCKQUOTE,
            NodeSpec::new().content("block+").group("block").defining(),
        )
        .node(
            nodes::CODE_BLOCK,
            // `text*`, not `inline*`: no images, no breaks, no marks. The
            // `.code()` also switches whitespace handling to `Pre`, which is
            // what keeps a parsed code block's indentation.
            NodeSpec::new()
                .content("text*")
                .marks("")
                .group("block")
                .code()
                .defining()
                .attr("language", AttrSpec::new(Value::Null)),
        )
        .node(
            nodes::HORIZONTAL_RULE,
            NodeSpec::new().group("block"),
        )
        .node(
            nodes::BULLET_LIST,
            NodeSpec::new().content("list_item+").group("block"),
        )
        .node(
            nodes::ORDERED_LIST,
            NodeSpec::new()
                .content("list_item+")
                .group("block")
                .attr("start", AttrSpec::new(1_i64)),
        )
        .node(
            nodes::LIST_ITEM,
            // A list item begins with a paragraph and may then hold anything,
            // including a nested list. Requiring the leading paragraph is what
            // gives the caret somewhere to be in an empty item.
            NodeSpec::new().content("paragraph block*").defining(),
        )
        // Tables. Cells are `isolating`, which is what stops a selection, a
        // join or a lift from reaching across a cell boundary — the property
        // that makes a table behave like a grid rather than like nested
        // blockquotes that happen to be drawn in a row.
        .node(
            nodes::TABLE,
            NodeSpec::new()
                .content("table_row+")
                .group("block")
                .isolating(),
        )
        .node(
            nodes::TABLE_ROW,
            NodeSpec::new().content("(table_cell | table_header)+"),
        )
        .node(
            nodes::TABLE_CELL,
            NodeSpec::new()
                .content("block+")
                .isolating()
                .attr("colspan", AttrSpec::new(1_i64))
                .attr("rowspan", AttrSpec::new(1_i64))
                .attr("align", AttrSpec::new(Value::Null)),
        )
        .node(
            nodes::TABLE_HEADER,
            NodeSpec::new()
                .content("block+")
                .isolating()
                .attr("colspan", AttrSpec::new(1_i64))
                .attr("rowspan", AttrSpec::new(1_i64))
                .attr("align", AttrSpec::new(Value::Null)),
        )
        .node(
            nodes::TEXT,
            NodeSpec::new().inline().group("inline"),
        )
        .node(
            nodes::IMAGE,
            NodeSpec::new()
                .inline()
                .group("inline")
                .attr("src", AttrSpec::required())
                .attr("alt", AttrSpec::new(Value::Null))
                .attr("title", AttrSpec::new(Value::Null)),
        )
        .node(
            nodes::HARD_BREAK,
            NodeSpec::new().inline().group("inline").not_selectable(),
        )
        .mark(
            marks::LINK,
            MarkSpec::new()
                .not_inclusive()
                .attr("href", AttrSpec::required())
                .attr("title", AttrSpec::new(Value::Null)),
        )
        .mark(marks::EM, MarkSpec::new())
        .mark(marks::STRONG, MarkSpec::new())
        .mark(marks::UNDERLINE, MarkSpec::new())
        .mark(marks::STRIKETHROUGH, MarkSpec::new())
        .mark(marks::CODE, MarkSpec::new().code().excludes("_"))
        .build()
        .expect("the standard schema is fixed and is covered by tests")
}
