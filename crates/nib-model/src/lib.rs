// SPDX-License-Identifier: MPL-2.0

//! The document model behind Nib.
//!
//! This crate knows what a rich text document *is*. It does not know what a
//! pixel is, what a font is, or that a toolkit exists, and it never will —
//! everything here is a data structure and a pure function over one. That is
//! not modesty about scope; it is the property that makes the rest possible.
//! A model that cannot render can be tested exhaustively in milliseconds, run
//! on a server to convert a document with no display attached, and kept when
//! the view layer is replaced.
//!
//! The design is ProseMirror's, because ProseMirror got the hard parts right
//! and there is no Rust prior art to fork. What follows is a port of its
//! ideas, not its code.
//!
//! # The shape of it
//!
//! - A [`Schema`](schema::Schema) declares the node and mark types a document
//!   may contain, and — through [`content`] — compiles each type's content
//!   expression into a DFA that can be *searched*, not merely consulted.
//! - A document is a tree of [`Node`](node::Node)s. Nodes are immutable and
//!   share structure, so a keystroke rebuilds one paragraph and the spine
//!   above it and leaves every other subtree pointed at by both versions.
//! - Positions into that tree are flat integers, resolved on demand into a
//!   path by [`ResolvedPos`](resolve::ResolvedPos).
//! - Changes are [`Step`](transform::Step)s. Every step can be *inverted*
//!   against the document it applied to, and *mapped* through other steps.
//!   Those two properties are all of undo and all of collaborative editing;
//!   neither is a separate mechanism bolted on later.
//!
//! # Two decisions worth stating up front
//!
//! **Positions are UTF-8 byte offsets.** ProseMirror counts UTF-16 code units
//! because it lives in a browser. Counting bytes instead means a text node's
//! size is `str::len()`, a cut is a `&str` slice, and the offsets handed to
//! the shaper need no conversion — `cosmic_text` also works in bytes. The cost
//! is that a position can name a place inside a character, so every cut
//! asserts on a character boundary rather than rounding to one. A silent round
//! would turn a mapping bug into mojibake three edits later; a panic puts it
//! where it happened.
//!
//! **The schema is enforced, not advisory.** A step that would produce a list
//! item outside a list does not apply. This costs a DFA and the machinery in
//! [`content`], and buys the absence of a whole category of bug that
//! tag-soup editors never stop paying for.

pub mod attrs;
pub mod basic;
pub mod build;
pub mod commands;
pub mod content;
pub mod fragment;
pub mod history;
pub mod keymap;
pub mod mark;
pub mod node;
pub mod replace;
pub mod resolve;
pub mod schema;
pub mod slice;
pub mod state;
pub mod transform;

pub use attrs::{Attrs, Value};
pub use build::Builder;
pub use content::{ContentExpr, ContentMatch};
pub use fragment::Fragment;
pub use mark::{Mark, Marks};
pub use node::Node;
pub use replace::ReplaceError;
pub use resolve::{NodeRange, ResolvedPos};
pub use slice::Slice;
pub use commands::Command;
pub use keymap::{Binding, Key, Keymap, Mods};
pub use state::{EditorState, Plugin, PluginKey, Selection, Transaction};
pub use schema::{
    AttrSpec, MarkSpec, MarkType, MarkTypeId, NodeSpec, NodeType, NodeTypeId, Schema, SchemaError,
};
