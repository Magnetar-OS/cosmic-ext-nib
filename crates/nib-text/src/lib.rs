// SPDX-License-Identifier: MPL-2.0

//! Plain text out of a Nib document, and plain text back in.
//!
//! # Why this is its own crate
//!
//! Because plain text is not a degraded Markdown. A document rendered for a
//! terminal, a `text/plain` mail part, or a screen reader has different rules
//! from one rendered for a Markdown file: no delimiters to read as syntax, a
//! wrap column that matters, and — for mail — quoting that has to survive
//! being quoted again.
//!
//! # The wrap column
//!
//! RFC 5322 asks for lines under 78 characters; 72 is the convention on top of
//! it, and the six characters of headroom are what let a message survive being
//! quoted twice without the wrap collapsing. That is why the default is 72 and
//! not 78.
//!
//! ```
//! # use nib_model::basic;
//! # use nib_text::{Text, Options};
//! let text = Text::new(&basic::schema());
//! let doc = basic::schema().empty_doc();
//! assert_eq!(text.to_text(&doc), "");
//! ```

pub mod parse;
pub mod quote;
pub mod write;

pub use parse::parse;
pub use quote::{Block, blocks, quote, unquote_one};
pub use write::{Options, to_text, wrap};

use nib_model::node::Node;
use nib_model::schema::Schema;

/// The plain-text converter for one schema.
#[derive(Debug, Clone)]
pub struct Text {
    schema: Schema,
    options: Options,
}

impl Text {
    #[must_use]
    pub fn new(schema: &Schema) -> Self {
        Self {
            schema: schema.clone(),
            options: Options::default(),
        }
    }

    #[must_use]
    pub fn with_options(schema: &Schema, options: Options) -> Self {
        Self {
            schema: schema.clone(),
            options,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Writes a document as plain text.
    #[must_use]
    pub fn to_text(&self, doc: &Node) -> String {
        to_text(doc, self.options)
    }

    /// Reads plain text into a document: blank-line-separated paragraphs, with
    /// quoted runs becoming blockquotes.
    #[must_use]
    pub fn parse(&self, text: &str) -> Node {
        parse(&self.schema, text)
    }
}
