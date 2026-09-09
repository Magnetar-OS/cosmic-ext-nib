// SPDX-License-Identifier: MPL-2.0

//! Building documents by hand.
//!
//! Parsers construct documents from bytes and commands construct them from
//! steps, but tests, fixtures and defaults construct them from nothing, and
//! doing that through [`Schema::create`](crate::Schema::create) directly is
//! four arguments of ceremony per node.
//!
//! ```
//! # use nib_model::{basic, build::Builder, nodes};
//! let b = Builder::new(basic::schema());
//! let doc = b.doc(nodes![
//!     b.node("paragraph", nodes![
//!         b.text("plain "),
//!         b.mark("strong", None, nodes![b.text("bold")]),
//!     ]),
//! ]);
//! assert_eq!(doc.to_string(), r#"doc(paragraph("plain ", strong("bold")))"#);
//! ```
//!
//! Every method panics on a schema it cannot satisfy — an unknown type name, a
//! missing required attribute. That is the right trade here and nowhere else:
//! the input is a literal in the source, so a failure is a typo the programmer
//! should see immediately, not a condition the program should carry a `Result`
//! for. Parsers, whose input is a file, return errors instead.

use crate::attrs::Attrs;
use crate::fragment::Fragment;
use crate::mark::Marks;
use crate::node::Node;
use crate::schema::Schema;

/// Anything that can stand in a node list: one node, or several.
pub trait IntoNodes {
    fn into_nodes(self) -> Vec<Node>;
}

impl IntoNodes for Node {
    fn into_nodes(self) -> Vec<Node> {
        vec![self]
    }
}

impl IntoNodes for Vec<Node> {
    fn into_nodes(self) -> Vec<Node> {
        self
    }
}

/// Flattens a mixture of nodes and node lists into one list.
///
/// Marks apply to runs rather than to single nodes, so a builder call that
/// applies one returns a list; this is what lets both spellings sit in the
/// same argument.
#[macro_export]
macro_rules! nodes {
    () => { ::std::vec::Vec::<$crate::node::Node>::new() };
    ($($item:expr),+ $(,)?) => {{
        let mut out = ::std::vec::Vec::new();
        $(out.extend($crate::build::IntoNodes::into_nodes($item));)+
        out
    }};
}

/// Builds nodes against one schema.
#[derive(Debug, Clone)]
pub struct Builder {
    schema: Schema,
}

impl Builder {
    #[must_use]
    pub fn new(schema: Schema) -> Self {
        Self { schema }
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// A text node.
    #[must_use]
    pub fn text(&self, text: &str) -> Node {
        self.schema.text(text, Marks::none())
    }

    /// A node of the named type.
    ///
    /// # Panics
    ///
    /// If the schema has no such type, if a required attribute has no default,
    /// or if the content is not valid for the type.
    #[must_use]
    pub fn node(&self, name: &str, content: impl IntoNodes) -> Node {
        self.attr_node(name, &Attrs::none(), content)
    }

    /// A node of the named type, with attributes.
    ///
    /// # Panics
    ///
    /// If the schema has no such type, if a required attribute is missing, or
    /// if the content is not valid for the type.
    #[must_use]
    pub fn attr_node(&self, name: &str, attrs: &Attrs, content: impl IntoNodes) -> Node {
        let id = self
            .schema
            .node_id(name)
            .unwrap_or_else(|| panic!("no node type named {name:?} in this schema"));
        let attrs = if attrs.is_empty() { None } else { Some(attrs) };
        self.schema
            .create_checked(
                id,
                attrs,
                Fragment::from_vec(content.into_nodes()),
                Marks::none(),
            )
            .unwrap_or_else(|e| panic!("building {name:?}: {e}"))
    }

    /// The document's top node.
    ///
    /// # Panics
    ///
    /// If the content is not valid for it.
    #[must_use]
    pub fn doc(&self, content: impl IntoNodes) -> Node {
        let top = self.schema.top_node_type();
        self.schema
            .create_checked(
                top,
                None,
                Fragment::from_vec(content.into_nodes()),
                Marks::none(),
            )
            .unwrap_or_else(|e| panic!("building the document: {e}"))
    }

    /// Adds a mark to every node in `content`, and to their inline
    /// descendants.
    ///
    /// # Panics
    ///
    /// If the schema has no such mark, or a required attribute is missing.
    #[must_use]
    pub fn mark(&self, name: &str, attrs: Option<&Attrs>, content: impl IntoNodes) -> Vec<Node> {
        let mark = self
            .schema
            .mark(name, attrs)
            .unwrap_or_else(|e| panic!("building mark {name:?}: {e}"));
        content
            .into_nodes()
            .into_iter()
            .map(|node| node.add_mark(&mark))
            .collect()
    }
}
