// SPDX-License-Identifier: MPL-2.0

//! Components, for MDC and MDX.
//!
//! Both dialects have the same shape underneath — a named thing with a bag of
//! props, either standing on its own as a block or sitting inside a line — and
//! differ only in spelling. So they are one pair of node types, and the
//! dialect decides how they are written back out.

use nib_model::attrs::Value;
use nib_model::schema::{AttrSpec, NodeSpec, Schema, SchemaError};

/// The block component's node type name.
pub const COMPONENT_BLOCK: &str = "component_block";

/// The inline component's node type name.
pub const COMPONENT_INLINE: &str = "component_inline";

/// The attribute holding a component's name. Every other attribute is a prop.
pub const NAME: &str = "name";

/// The attribute holding an MDX `import`/`export` line, kept verbatim.
pub const SOURCE: &str = "source";

/// The node type name for an ESM line in MDX.
pub const ESM: &str = "esm";

/// Adds the component node types to a schema's declarations.
///
/// Rebuilds the schema rather than mutating one, because a `Schema` is
/// immutable by design — the whole point of node types being compared by
/// identity is that a document cannot outlive the schema it was built for.
///
/// # Errors
///
/// [`SchemaError`] when the resulting schema does not compile — a name clash
/// with something the caller already declared.
pub fn with_components(schema: &Schema) -> Result<Schema, SchemaError> {
    let mut builder = Schema::builder();
    for typ in schema.node_types() {
        builder = builder.node(typ.name(), typ.spec().clone());
    }
    builder = builder
        .node(
            COMPONENT_BLOCK,
            // `block*`, not `block+`: a self-closing component has no slot
            // content, and requiring some would make the parser invent an
            // empty paragraph inside every one.
            NodeSpec::new()
                .content("block*")
                .group("block")
                .open()
                .attr(NAME, AttrSpec::required()),
        )
        .node(
            COMPONENT_INLINE,
            NodeSpec::new()
                .inline()
                .group("inline")
                .atom()
                .open()
                .attr(NAME, AttrSpec::required()),
        )
        .node(
            ESM,
            NodeSpec::new()
                .group("block")
                .attr(SOURCE, AttrSpec::new(Value::Null)),
        );
    for typ in schema.mark_types() {
        builder = builder.mark(typ.name(), typ.spec().clone());
    }
    builder
        .top_node(schema.node_type(schema.top_node_type()).name().to_owned())
        .build()
}
