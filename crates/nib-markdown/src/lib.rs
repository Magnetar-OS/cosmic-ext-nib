// SPDX-License-Identifier: MPL-2.0

//! Markdown in and out of a Nib document.
//!
//! Three dialects, each a superset of the one before:
//!
//! - [`Dialect::CommonMark`] — the specification, nothing added.
//! - [`Dialect::Gfm`] — plus tables, strikethrough, task lists and
//!   autolinks. What almost everyone means by "Markdown".
//! - [`Dialect::Mdc`] — plus components: `::name{prop=value}` as a block,
//!   `:name{...}` inline, and `{.class #id}` attributes on a span. Nuxt's
//!   syntax.
//! - [`Dialect::Mdx`] — plus JSX-shaped components, `<Name prop="v" />` and
//!   `<Name>…</Name>`, and `import`/`export` lines kept as opaque blocks.
//!
//! # What a component is in the model
//!
//! A node type with [`open_attrs`](nib_model::NodeSpec::open_attrs): its
//! props are the author's, not the schema's, so the schema declares only the
//! `name` it requires and keeps whatever else arrives. That is the one place
//! the model bends its rule that an undeclared attribute is a mistake, and it
//! bends because a component's prop list is genuinely open — everywhere else,
//! an attribute nobody declared is one nothing can rely on.
//!
//! ```
//! # use nib_model::basic;
//! # use nib_markdown::{Markdown, Dialect};
//! let md = Markdown::new(&basic::schema(), Dialect::Gfm);
//! let doc = md.parse("# Title\n\nSome **bold** text.");
//! assert_eq!(md.to_markdown(&doc), "# Title\n\nSome **bold** text.\n");
//! ```

pub mod components;
pub mod parse;
pub mod write;

pub use components::{COMPONENT_BLOCK, COMPONENT_INLINE, with_components};
pub use parse::parse;
pub use write::to_markdown;

use nib_model::node::Node;
use nib_model::schema::Schema;

/// Which flavour of Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    /// The specification, nothing added.
    CommonMark,
    /// Plus tables, strikethrough, task lists and autolinks.
    #[default]
    Gfm,
    /// Plus Nuxt's component syntax.
    Mdc,
    /// Plus JSX-shaped components and ESM lines.
    Mdx,
}

impl Dialect {
    /// Whether tables, strikethrough and task lists are on.
    #[must_use]
    pub fn has_gfm(self) -> bool {
        !matches!(self, Self::CommonMark)
    }

    /// Whether components are recognised, and in which spelling.
    #[must_use]
    pub fn has_components(self) -> bool {
        matches!(self, Self::Mdc | Self::Mdx)
    }
}

/// The Markdown converter for one schema and dialect.
#[derive(Debug, Clone)]
pub struct Markdown {
    schema: Schema,
    dialect: Dialect,
}

impl Markdown {
    #[must_use]
    pub fn new(schema: &Schema, dialect: Dialect) -> Self {
        Self {
            schema: schema.clone(),
            dialect,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Parses a document.
    #[must_use]
    pub fn parse(&self, text: &str) -> Node {
        parse(&self.schema, self.dialect, text)
    }

    /// Writes a document.
    #[must_use]
    pub fn to_markdown(&self, doc: &Node) -> String {
        to_markdown(&self.schema, self.dialect, doc)
    }
}
