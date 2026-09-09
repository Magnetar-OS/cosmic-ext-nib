// SPDX-License-Identifier: MPL-2.0

//! HTML in and out of a Nib document.
//!
//! ```
//! # use nib_model::basic;
//! # use nib_html::Html;
//! let html = Html::new(&basic::schema());
//! let doc = html.parse("<p>Hello <strong>there</strong></p>");
//! assert_eq!(html.to_html(&doc), "<p>Hello <strong>there</strong></p>");
//! ```
//!
//! # What it will not do
//!
//! Preserve HTML it does not understand. A `<span style="color:red">` comes
//! back as its text, and a `<marquee>` as its content. That is the schema
//! doing its job: a document model whose contents are "whatever tags happened
//! to arrive" is a tag soup with extra steps, and the reason to have one is
//! that everything downstream can rely on what it holds.
//!
//! Applications that need to keep something add it to the schema and to the
//! [`Rules`], in that order.

pub mod parse;
pub mod rules;
pub mod write;

pub use parse::{parse, parse_slice};
pub use rules::{Element, ParseRule, Rules, Target, WriteRule, base};
pub use write::{slice_to_html, to_html};

use nib_model::node::Node;
use nib_model::schema::Schema;
use nib_model::slice::Slice;

/// The HTML converter for one schema.
///
/// A thin handle over [`Rules`], for the common case where the defaults are
/// wanted. Applications that need custom tags build a [`Rules`] and call the
/// free functions.
#[derive(Debug, Clone)]
pub struct Html {
    rules: Rules,
}

impl Html {
    /// The default rules for a schema.
    #[must_use]
    pub fn new(schema: &Schema) -> Self {
        Self {
            rules: base(schema),
        }
    }

    /// A converter over a custom rule table.
    #[must_use]
    pub fn with_rules(rules: Rules) -> Self {
        Self { rules }
    }

    #[must_use]
    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Parses a whole document.
    #[must_use]
    pub fn parse(&self, html: &str) -> Node {
        parse(&self.rules, html)
    }

    /// Parses a fragment as a slice, for a paste.
    #[must_use]
    pub fn parse_slice(&self, html: &str) -> Slice {
        parse_slice(&self.rules, html)
    }

    /// Writes a document.
    #[must_use]
    pub fn to_html(&self, doc: &Node) -> String {
        to_html(&self.rules, doc)
    }

    /// Writes a slice, for the clipboard.
    #[must_use]
    pub fn slice_to_html(&self, slice: &Slice) -> String {
        slice_to_html(&self.rules, slice)
    }
}
