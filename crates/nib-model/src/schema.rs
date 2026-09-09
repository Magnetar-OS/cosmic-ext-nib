// SPDX-License-Identifier: MPL-2.0

//! The schema: what documents of this kind are made of.
//!
//! A schema names the node types and mark types a document may contain, what
//! each may contain in turn, and what attributes each carries. It is built
//! once, shared for the life of the editor, and every document, step and
//! command is defined against it.
//!
//! # Why a schema is the load-bearing idea
//!
//! Without one, "make this bold" and "press Enter" are heuristics over a soup
//! of tags, and every editor that works that way spends its life producing
//! documents its own renderer cannot make sense of — a list item outside a
//! list, a heading inside a heading, a link wrapping half a paragraph and
//! nothing else. With one, those are not bugs to fix but states the model
//! cannot reach: a step that would produce them does not apply.
//!
//! The cost is that the schema must be able to answer harder questions than
//! "is this allowed". [`ContentMatch`] is where that is paid.
//!
//! # Building
//!
//! Two passes, because content expressions name node types and node types must
//! therefore all exist before any expression compiles:
//!
//! ```
//! # use nib_model::schema::{Schema, NodeSpec, MarkSpec};
//! let schema = Schema::builder()
//!     .node("doc", NodeSpec::new().content("block+"))
//!     .node("paragraph", NodeSpec::new().content("inline*").group("block"))
//!     .node("text", NodeSpec::new().inline().group("inline"))
//!     .mark("strong", MarkSpec::new())
//!     .build()
//!     .expect("a schema this small has nothing to get wrong");
//! assert!(schema.node_id("paragraph").is_some());
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use crate::attrs::{Attrs, Value};
use crate::content::{ContentError, ContentExpr, ContentMatch};
use crate::fragment::Fragment;
use crate::mark::{Mark, Marks};
use crate::node::Node;

/// A node type's index in its schema. Stable for the schema's lifetime.
pub type NodeTypeId = usize;

/// A mark type's index in its schema. Also its rank, which is why mark order
/// in the builder is meaningful: it decides the order marks are stored and
/// serialised in.
pub type MarkTypeId = usize;

/// The name every schema's text node must have.
pub const TEXT: &str = "text";

/// Why a schema could not be built.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("the schema has no node type named {0:?}")]
    UnknownNode(String),
    #[error("the schema has no mark type named {0:?}")]
    UnknownMark(String),
    #[error("duplicate node type {0:?}")]
    DuplicateNode(String),
    #[error("duplicate mark type {0:?}")]
    DuplicateMark(String),
    #[error("the schema needs a {TEXT:?} node type, declared inline with no content")]
    MissingText,
    #[error("the {TEXT:?} node type must be inline and must have no content")]
    BadText,
    #[error("the top node type {0:?} is not in the schema")]
    MissingTopNode(String),
    #[error("in the content of {node:?}: {source}")]
    Content {
        node: String,
        #[source]
        source: ContentError,
    },
    #[error("no value supplied for required attribute {attr:?} of {owner:?}")]
    MissingAttr { owner: String, attr: String },
    #[error("content is not valid for a {node:?}: {found}")]
    InvalidContent { node: String, found: String },
}

/// One attribute a node or mark type carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSpec {
    /// The value used when none is given. `None` makes the attribute
    /// required — and a type with a required attribute can never be created
    /// automatically, which is what keeps `fill_before` from inventing an
    /// image with no `src`.
    pub default: Option<Value>,
}

impl AttrSpec {
    #[must_use]
    pub fn new(default: impl Into<Value>) -> Self {
        Self {
            default: Some(default.into()),
        }
    }

    /// An attribute with no default; every creation must supply it.
    #[must_use]
    pub fn required() -> Self {
        Self { default: None }
    }
}

/// How whitespace in a node's text is to be treated when parsing into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Whitespace {
    /// Runs of whitespace collapse to one space; leading and trailing go.
    #[default]
    Normal,
    /// Preserved exactly, including newlines. What a code block wants.
    Pre,
}

/// The declaration of a node type.
///
/// A pile of booleans, and deliberately so: each is an independent property
/// the schema author sets or does not, and folding them into enums would
/// invent relationships between them that do not exist.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct NodeSpec {
    /// The content expression. Absent means the node is a leaf.
    pub content: Option<String>,
    /// Which marks the node's *children* may carry. `"_"` allows all, `""`
    /// none, a space-separated list allows those, and absent means all for a
    /// node with inline content and none otherwise.
    pub marks: Option<String>,
    /// Space-separated group names, for content expressions to refer to.
    pub group: Option<String>,
    /// Inline nodes live inside a textblock; block nodes do not.
    pub inline: bool,
    /// Treated as a single unit by the cursor even though it has content.
    pub atom: bool,
    /// Selectable as a whole by clicking it.
    pub selectable: bool,
    /// Its text is code: no input rules, no smart quotes, whitespace kept.
    pub code: bool,
    pub whitespace: Whitespace,
    /// Content lifted out of this node keeps the node around it — the reason
    /// pasting a paragraph into a blockquote keeps the quote, and pasting a
    /// list item into a paragraph does not keep the item.
    pub defining: bool,
    /// Nothing may be joined, lifted or split across this node's boundary. A
    /// table cell.
    pub isolating: bool,
    pub attrs: BTreeMap<String, AttrSpec>,
}

impl NodeSpec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            selectable: true,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn content(mut self, expr: impl Into<String>) -> Self {
        self.content = Some(expr.into());
        self
    }

    #[must_use]
    pub fn marks(mut self, expr: impl Into<String>) -> Self {
        self.marks = Some(expr.into());
        self
    }

    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    #[must_use]
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    #[must_use]
    pub fn atom(mut self) -> Self {
        self.atom = true;
        self
    }

    #[must_use]
    pub fn code(mut self) -> Self {
        self.code = true;
        self.whitespace = Whitespace::Pre;
        self
    }

    #[must_use]
    pub fn whitespace(mut self, whitespace: Whitespace) -> Self {
        self.whitespace = whitespace;
        self
    }

    #[must_use]
    pub fn defining(mut self) -> Self {
        self.defining = true;
        self
    }

    #[must_use]
    pub fn isolating(mut self) -> Self {
        self.isolating = true;
        self
    }

    #[must_use]
    pub fn not_selectable(mut self) -> Self {
        self.selectable = false;
        self
    }

    #[must_use]
    pub fn attr(mut self, name: impl Into<String>, spec: AttrSpec) -> Self {
        self.attrs.insert(name.into(), spec);
        self
    }
}

/// The declaration of a mark type.
#[derive(Debug, Clone)]
pub struct MarkSpec {
    /// Whether typing at the end of a run of this mark continues it. True for
    /// emphasis, false for a link.
    pub inclusive: bool,
    /// Which marks this one excludes. Absent means "another of my own type",
    /// which is what makes re-linking replace rather than nest. `"_"` excludes
    /// every mark, `""` excludes none.
    pub excludes: Option<String>,
    /// Space-separated group names.
    pub group: Option<String>,
    /// Whether the mark may be split across non-text content.
    pub spanning: bool,
    /// The marked text is code.
    pub code: bool,
    pub attrs: BTreeMap<String, AttrSpec>,
}

impl Default for MarkSpec {
    fn default() -> Self {
        Self {
            inclusive: true,
            excludes: None,
            group: None,
            spanning: true,
            code: false,
            attrs: BTreeMap::new(),
        }
    }
}

impl MarkSpec {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Typing at the end of a run of this mark does not continue it.
    #[must_use]
    pub fn not_inclusive(mut self) -> Self {
        self.inclusive = false;
        self
    }

    #[must_use]
    pub fn excludes(mut self, expr: impl Into<String>) -> Self {
        self.excludes = Some(expr.into());
        self
    }

    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    #[must_use]
    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    #[must_use]
    pub fn not_spanning(mut self) -> Self {
        self.spanning = false;
        self
    }

    #[must_use]
    pub fn attr(mut self, name: impl Into<String>, spec: AttrSpec) -> Self {
        self.attrs.insert(name.into(), spec);
        self
    }
}

/// What the second build pass works out, once every type exists.
#[derive(Debug)]
struct Compiled {
    content_match: ContentMatch,
    /// `None` allows every mark.
    mark_set: Option<Vec<MarkTypeId>>,
    inline_content: bool,
    is_leaf: bool,
}

/// A node type in a built schema.
#[derive(Debug)]
pub struct NodeType {
    id: NodeTypeId,
    name: Arc<str>,
    spec: NodeSpec,
    groups: Vec<String>,
    /// All attributes with their defaults; `None` when one is required.
    default_attrs: Option<Attrs>,
    has_required_attrs: bool,
    is_text: bool,
    compiled: OnceLock<Compiled>,
}

impl NodeType {
    #[must_use]
    pub fn id(&self) -> NodeTypeId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    #[must_use]
    pub fn is_inline(&self) -> bool {
        self.spec.inline
    }

    #[must_use]
    pub fn is_block(&self) -> bool {
        !self.spec.inline
    }

    #[must_use]
    pub fn is_text(&self) -> bool {
        self.is_text
    }

    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.compiled().is_leaf
    }

    #[must_use]
    pub fn is_atom(&self) -> bool {
        self.is_leaf() || self.spec.atom
    }

    /// True when this type's content is inline — that is, when it is a
    /// textblock and a caret can live in it.
    #[must_use]
    pub fn is_inline_content(&self) -> bool {
        self.compiled().inline_content
    }

    #[must_use]
    pub fn is_textblock(&self) -> bool {
        self.is_block() && self.is_inline_content()
    }

    #[must_use]
    pub fn in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    /// A cursor at the start of this type's content expression.
    #[must_use]
    pub fn content_match(&self) -> ContentMatch {
        self.compiled().content_match.clone()
    }

    /// True when some attribute has no default, so the type cannot be created
    /// without explicit values.
    #[must_use]
    pub fn has_required_attrs(&self) -> bool {
        self.has_required_attrs
    }

    #[must_use]
    pub fn allows_mark_type(&self, mark: MarkTypeId) -> bool {
        match &self.compiled().mark_set {
            None => true,
            Some(set) => set.contains(&mark),
        }
    }

    #[must_use]
    pub fn allows_marks(&self, marks: &Marks) -> bool {
        marks.iter().all(|m| self.allows_mark_type(m.typ().id()))
    }

    /// `marks` with everything this type forbids removed — how content is
    /// coerced as it moves into a stricter node.
    #[must_use]
    pub fn allowed_marks(&self, marks: &Marks) -> Marks {
        marks.retain_allowed(|t| self.allows_mark_type(t.id()))
    }

    /// True when `content` is legal for this type, start to finish.
    #[must_use]
    pub fn valid_content(&self, content: &Fragment) -> bool {
        self.content_match()
            .match_fragment(content)
            .is_some_and(|m| m.valid_end())
    }

    /// True when the two types' contents are interchangeable enough to join.
    #[must_use]
    pub fn is_compatible_content(&self, other: &Self) -> bool {
        self.id == other.id || self.content_match().compatible(&other.content_match())
    }

    fn compiled(&self) -> &Compiled {
        self.compiled
            .get()
            .expect("node type used before its schema finished building")
    }

    /// Fills in defaults for attributes the caller did not supply.
    fn compute_attrs(&self, given: Option<&Attrs>) -> Result<Attrs, SchemaError> {
        let Some(given) = given else {
            return self
                .default_attrs
                .clone()
                .ok_or_else(|| self.first_missing_attr(&Attrs::none()));
        };
        let mut out = Attrs::none();
        for (name, spec) in &self.spec.attrs {
            let value = given
                .get(name)
                .cloned()
                .or_else(|| spec.default.clone())
                .ok_or_else(|| SchemaError::MissingAttr {
                    owner: self.name.to_string(),
                    attr: name.clone(),
                })?;
            out = out.set(name.as_str(), value);
        }
        Ok(out)
    }

    fn first_missing_attr(&self, given: &Attrs) -> SchemaError {
        let attr = self
            .spec
            .attrs
            .iter()
            .find(|(name, spec)| spec.default.is_none() && given.get(name).is_none())
            .map_or_else(|| "?".to_owned(), |(name, _)| name.clone());
        SchemaError::MissingAttr {
            owner: self.name.to_string(),
            attr,
        }
    }
}

/// A mark type in a built schema.
#[derive(Debug)]
pub struct MarkType {
    id: MarkTypeId,
    name: Arc<str>,
    spec: MarkSpec,
    groups: Vec<String>,
    default_attrs: Option<Attrs>,
    has_required_attrs: bool,
    excluded: OnceLock<Vec<MarkTypeId>>,
}

impl MarkType {
    #[must_use]
    pub fn id(&self) -> MarkTypeId {
        self.id
    }

    /// The type's rank, which is its position in the schema. Mark sets are
    /// kept in this order.
    #[must_use]
    pub fn rank(&self) -> usize {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn spec(&self) -> &MarkSpec {
        &self.spec
    }

    #[must_use]
    pub fn is_inclusive(&self) -> bool {
        self.spec.inclusive
    }

    #[must_use]
    pub fn in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    #[must_use]
    pub fn has_required_attrs(&self) -> bool {
        self.has_required_attrs
    }

    /// True when a mark of this type cannot coexist with one of `other`.
    ///
    /// # Panics
    ///
    /// If the schema is still being built. A `MarkType` only escapes
    /// [`SchemaBuilder::build`] once its exclusions are resolved.
    #[must_use]
    pub fn excludes(&self, other: MarkTypeId) -> bool {
        self.excluded
            .get()
            .expect("mark type used before its schema finished building")
            .contains(&other)
    }
}

// ---------------------------------------------------------------------------
// The schema
// ---------------------------------------------------------------------------

/// A built schema.
///
/// Cheap to clone (`Arc` inside), and every document built against it holds
/// its types directly, so a node knows what it is without consulting anything.
#[derive(Debug, Clone)]
pub struct Schema {
    inner: Arc<SchemaInner>,
}

#[derive(Debug)]
struct SchemaInner {
    nodes: Vec<Arc<NodeType>>,
    marks: Vec<Arc<MarkType>>,
    node_by_name: BTreeMap<String, NodeTypeId>,
    mark_by_name: BTreeMap<String, MarkTypeId>,
    top: NodeTypeId,
    text: NodeTypeId,
}

impl Schema {
    /// Starts building a schema.
    #[must_use]
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    #[must_use]
    pub fn node_types(&self) -> &[Arc<NodeType>] {
        &self.inner.nodes
    }

    #[must_use]
    pub fn mark_types(&self) -> &[Arc<MarkType>] {
        &self.inner.marks
    }

    /// # Panics
    ///
    /// If `id` is not from this schema.
    #[must_use]
    pub fn node_type(&self, id: NodeTypeId) -> &Arc<NodeType> {
        &self.inner.nodes[id]
    }

    /// # Panics
    ///
    /// If `id` is not from this schema.
    #[must_use]
    pub fn mark_type(&self, id: MarkTypeId) -> &Arc<MarkType> {
        &self.inner.marks[id]
    }

    #[must_use]
    pub fn node_id(&self, name: &str) -> Option<NodeTypeId> {
        self.inner.node_by_name.get(name).copied()
    }

    #[must_use]
    pub fn mark_id(&self, name: &str) -> Option<MarkTypeId> {
        self.inner.mark_by_name.get(name).copied()
    }

    #[must_use]
    pub fn node_type_by_name(&self, name: &str) -> Option<&Arc<NodeType>> {
        self.node_id(name).map(|id| self.node_type(id))
    }

    #[must_use]
    pub fn mark_type_by_name(&self, name: &str) -> Option<&Arc<MarkType>> {
        self.mark_id(name).map(|id| self.mark_type(id))
    }

    /// The type a document's root node has.
    #[must_use]
    pub fn top_node_type(&self) -> NodeTypeId {
        self.inner.top
    }

    /// The text node type.
    #[must_use]
    pub fn text_type(&self) -> NodeTypeId {
        self.inner.text
    }

    /// A text node.
    ///
    /// # Panics
    ///
    /// Never: [`SchemaBuilder::build`] refuses a schema with no text type, so
    /// by the time a `Schema` exists there is one.
    #[must_use]
    pub fn text(&self, text: impl Into<Arc<str>>, marks: Marks) -> Node {
        Node::new_text(Arc::clone(self.node_type(self.inner.text)), text, marks)
    }

    /// A mark of the named type.
    ///
    /// # Errors
    ///
    /// If the schema has no such mark, or a required attribute is missing.
    pub fn mark(&self, name: &str, attrs: Option<&Attrs>) -> Result<Mark, SchemaError> {
        let id = self
            .mark_id(name)
            .ok_or_else(|| SchemaError::UnknownMark(name.to_owned()))?;
        self.mark_by_id(id, attrs)
    }

    /// A mark of the given type.
    ///
    /// # Errors
    ///
    /// If a required attribute is missing.
    pub fn mark_by_id(&self, id: MarkTypeId, attrs: Option<&Attrs>) -> Result<Mark, SchemaError> {
        let typ = self.mark_type(id);
        let attrs = compute_attrs(&typ.spec.attrs, typ.default_attrs.as_ref(), attrs, &typ.name)?;
        Ok(Mark::new(Arc::clone(typ), attrs))
    }

    /// A node of the named type, without checking that `content` fits.
    ///
    /// Use when the content is already known good — a step that has already
    /// consulted the content match, or a builder in a test.
    ///
    /// # Errors
    ///
    /// If the schema has no such node type, or a required attribute is
    /// missing.
    pub fn node(
        &self,
        name: &str,
        attrs: Option<&Attrs>,
        content: Fragment,
        marks: Marks,
    ) -> Result<Node, SchemaError> {
        let id = self
            .node_id(name)
            .ok_or_else(|| SchemaError::UnknownNode(name.to_owned()))?;
        self.create(id, attrs, content, marks)
    }

    /// A node of the given type, without checking that `content` fits.
    ///
    /// # Errors
    ///
    /// If a required attribute is missing.
    pub fn create(
        &self,
        typ: NodeTypeId,
        attrs: Option<&Attrs>,
        content: Fragment,
        marks: Marks,
    ) -> Result<Node, SchemaError> {
        let t = self.node_type(typ);
        let attrs = t.compute_attrs(attrs)?;
        Ok(Node::new(Arc::clone(t), attrs, content, marks))
    }

    /// A node of the given type, refusing content the schema forbids.
    ///
    /// # Errors
    ///
    /// If a required attribute is missing, or `content` is not valid for the
    /// type.
    pub fn create_checked(
        &self,
        typ: NodeTypeId,
        attrs: Option<&Attrs>,
        content: Fragment,
        marks: Marks,
    ) -> Result<Node, SchemaError> {
        let t = self.node_type(typ);
        if !t
            .content_match()
            .match_fragment(&content)
            .is_some_and(|m| m.valid_end())
        {
            return Err(SchemaError::InvalidContent {
                node: t.name().to_owned(),
                found: content
                    .iter()
                    .map(Node::type_name)
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        self.create(typ, attrs, content, marks)
    }

    /// A node of the given type with whatever nodes are needed on either side
    /// of `content` to make it legal, or `None` if nothing would.
    ///
    /// This is what turns "put a paragraph in a table cell" into a valid
    /// table, and what `Enter` uses to decide what an empty new block is.
    #[must_use]
    pub fn create_and_fill(
        &self,
        typ: NodeTypeId,
        attrs: Option<&Attrs>,
        content: Fragment,
        marks: Marks,
    ) -> Option<Node> {
        let t = self.node_type(typ);
        let attrs = t.compute_attrs(attrs).ok()?;
        let start = t.content_match();

        let content = if content.is_empty() {
            content
        } else {
            let before = start.fill_before(self, &content, false, 0)?;
            before.append(&content)
        };
        let matched = start.match_fragment(&content)?;
        let after = matched.fill_before(self, &Fragment::empty(), true, 0)?;
        Some(Node::new(
            Arc::clone(t),
            attrs,
            content.append(&after),
            marks,
        ))
    }

    /// An empty document of this schema: the top node, filled with whatever
    /// its content expression requires.
    ///
    /// # Panics
    ///
    /// If the top node type cannot be filled — a schema whose `doc` requires
    /// content nothing can supply, which is a schema bug rather than a runtime
    /// condition.
    #[must_use]
    pub fn empty_doc(&self) -> Node {
        self.create_and_fill(self.inner.top, None, Fragment::empty(), Marks::none())
            .expect("the top node type cannot be created empty; check its content expression")
    }
}

fn compute_attrs(
    specs: &BTreeMap<String, AttrSpec>,
    defaults: Option<&Attrs>,
    given: Option<&Attrs>,
    owner: &str,
) -> Result<Attrs, SchemaError> {
    let Some(given) = given else {
        return defaults.cloned().ok_or_else(|| {
            let attr = specs
                .iter()
                .find(|(_, s)| s.default.is_none())
                .map_or_else(|| "?".to_owned(), |(n, _)| n.clone());
            SchemaError::MissingAttr {
                owner: owner.to_owned(),
                attr,
            }
        });
    };
    let mut out = Attrs::none();
    for (name, spec) in specs {
        let value = given
            .get(name)
            .cloned()
            .or_else(|| spec.default.clone())
            .ok_or_else(|| SchemaError::MissingAttr {
                owner: owner.to_owned(),
                attr: name.clone(),
            })?;
        out = out.set(name.as_str(), value);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// Collects declarations, then compiles them.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    nodes: Vec<(String, NodeSpec)>,
    marks: Vec<(String, MarkSpec)>,
    top: Option<String>,
}

impl SchemaBuilder {
    /// Declares a node type. Declaration order decides node type ids.
    #[must_use]
    pub fn node(mut self, name: impl Into<String>, spec: NodeSpec) -> Self {
        self.nodes.push((name.into(), spec));
        self
    }

    /// Declares a mark type. Declaration order decides mark rank, and so the
    /// order marks are stored and serialised in.
    #[must_use]
    pub fn mark(mut self, name: impl Into<String>, spec: MarkSpec) -> Self {
        self.marks.push((name.into(), spec));
        self
    }

    /// Names the document's root type. Defaults to `doc`, or to the first
    /// declared node type when there is no `doc`.
    #[must_use]
    pub fn top_node(mut self, name: impl Into<String>) -> Self {
        self.top = Some(name.into());
        self
    }

    /// Compiles the declarations into a [`Schema`].
    ///
    /// # Errors
    ///
    /// If a name is declared twice, a content expression names something that
    /// does not exist or cannot be compiled, or the schema lacks a usable
    /// `text` node type.
    #[allow(clippy::too_many_lines)]
    pub fn build(self) -> Result<Schema, SchemaError> {
        // -- pass one: the types themselves --------------------------------
        let mut node_by_name = BTreeMap::new();
        let mut nodes: Vec<Arc<NodeType>> = Vec::with_capacity(self.nodes.len());
        for (id, (name, spec)) in self.nodes.iter().enumerate() {
            if node_by_name.insert(name.clone(), id).is_some() {
                return Err(SchemaError::DuplicateNode(name.clone()));
            }
            let (default_attrs, has_required_attrs) = defaults_for(&spec.attrs);
            nodes.push(Arc::new(NodeType {
                id,
                name: Arc::from(name.as_str()),
                spec: spec.clone(),
                groups: split_groups(spec.group.as_deref()),
                default_attrs,
                has_required_attrs,
                is_text: name == TEXT,
                compiled: OnceLock::new(),
            }));
        }

        let mut mark_by_name = BTreeMap::new();
        let mut marks: Vec<Arc<MarkType>> = Vec::with_capacity(self.marks.len());
        for (id, (name, spec)) in self.marks.iter().enumerate() {
            if mark_by_name.insert(name.clone(), id).is_some() {
                return Err(SchemaError::DuplicateMark(name.clone()));
            }
            let (default_attrs, has_required_attrs) = defaults_for(&spec.attrs);
            marks.push(Arc::new(MarkType {
                id,
                name: Arc::from(name.as_str()),
                spec: spec.clone(),
                groups: split_groups(spec.group.as_deref()),
                default_attrs,
                has_required_attrs,
                excluded: OnceLock::new(),
            }));
        }

        // The text node is not optional. Every schema has inline content
        // somewhere, and the alternative to requiring it here is discovering
        // its absence at the first keystroke.
        let text = *node_by_name
            .get(TEXT)
            .ok_or(SchemaError::MissingText)?;
        if !nodes[text].spec.inline || nodes[text].spec.content.is_some() {
            return Err(SchemaError::BadText);
        }

        let top_name = self.top.clone().unwrap_or_else(|| {
            if node_by_name.contains_key("doc") {
                "doc".to_owned()
            } else {
                self.nodes
                    .first()
                    .map_or_else(String::new, |(n, _)| n.clone())
            }
        });
        let top = *node_by_name
            .get(&top_name)
            .ok_or(SchemaError::MissingTopNode(top_name))?;

        // -- pass two: content expressions, mark sets, exclusions ----------
        let groups = gather_node_groups(&nodes);

        for (id, node) in nodes.iter().enumerate() {
            let source = node.spec.content.as_deref().unwrap_or("");
            let expr = if source.trim().is_empty() {
                ContentExpr::empty()
            } else {
                let expr = ContentExpr::compile(source, &node_by_name, &groups).map_err(
                    |source| SchemaError::Content {
                        node: node.name.to_string(),
                        source,
                    },
                )?;
                // Now that every node type exists, the expression can be held
                // to a standard it could not be held to while compiling: that
                // each position it *requires* can actually be filled.
                for required in expr.required_positions() {
                    if required
                        .iter()
                        .all(|t| nodes[*t].is_text || nodes[*t].has_required_attrs)
                    {
                        let types = required
                            .iter()
                            .map(|t| nodes[*t].name.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(SchemaError::Content {
                            node: node.name.to_string(),
                            source: ContentError::NotGeneratable {
                                expr: source.to_owned(),
                                types,
                            },
                        });
                    }
                }
                expr
            };
            let content_match = ContentMatch::start(expr);
            let is_leaf = content_match.edge_count() == 0 && content_match.valid_end();
            let inline_content = content_match
                .edge(0)
                .is_some_and(|(first, _)| nodes[first].spec.inline);

            let mark_set = resolve_mark_set(
                node.spec.marks.as_deref(),
                inline_content,
                &mark_by_name,
                &marks,
            )
            .map_err(SchemaError::UnknownMark)?;

            let _ = nodes[id].compiled.set(Compiled {
                content_match,
                mark_set,
                inline_content,
                is_leaf,
            });
        }

        for (id, mark) in marks.iter().enumerate() {
            let expr = mark.spec.excludes.as_deref().unwrap_or(mark.name());
            let excluded = if expr.trim().is_empty() {
                Vec::new()
            } else if expr.trim() == "_" {
                (0..marks.len()).collect()
            } else {
                gather_marks(expr, &mark_by_name, &marks).map_err(SchemaError::UnknownMark)?
            };
            let _ = marks[id].excluded.set(excluded);
        }

        Ok(Schema {
            inner: Arc::new(SchemaInner {
                nodes,
                marks,
                node_by_name,
                mark_by_name,
                top,
                text,
            }),
        })
    }
}

fn defaults_for(specs: &BTreeMap<String, AttrSpec>) -> (Option<Attrs>, bool) {
    let has_required = specs.values().any(|s| s.default.is_none());
    if has_required {
        return (None, true);
    }
    let mut attrs = Attrs::none();
    for (name, spec) in specs {
        if let Some(default) = &spec.default {
            attrs = attrs.set(name.as_str(), default.clone());
        }
    }
    (Some(attrs), false)
}

fn split_groups(group: Option<&str>) -> Vec<String> {
    group
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn gather_node_groups(nodes: &[Arc<NodeType>]) -> BTreeMap<String, Vec<NodeTypeId>> {
    let mut groups: BTreeMap<String, Vec<NodeTypeId>> = BTreeMap::new();
    for node in nodes {
        for group in &node.groups {
            groups.entry(group.clone()).or_default().push(node.id);
        }
    }
    groups
}

/// Resolves a node spec's `marks` expression. `Ok(None)` means "all marks".
fn resolve_mark_set(
    expr: Option<&str>,
    inline_content: bool,
    by_name: &BTreeMap<String, MarkTypeId>,
    marks: &[Arc<MarkType>],
) -> Result<Option<Vec<MarkTypeId>>, String> {
    match expr {
        Some("_") => Ok(None),
        Some(e) if !e.trim().is_empty() => Ok(Some(gather_marks(e, by_name, marks)?)),
        // An explicit empty string means no marks; so does a node whose
        // content is not inline, since there is nothing there to mark.
        Some(_) | None if !inline_content => Ok(Some(Vec::new())),
        Some(_) => Ok(Some(Vec::new())),
        None => Ok(None),
    }
}

/// Resolves a space-separated list of mark names and group names.
fn gather_marks(
    expr: &str,
    by_name: &BTreeMap<String, MarkTypeId>,
    marks: &[Arc<MarkType>],
) -> Result<Vec<MarkTypeId>, String> {
    let mut out = Vec::new();
    for name in expr.split_whitespace() {
        if let Some(id) = by_name.get(name) {
            if !out.contains(id) {
                out.push(*id);
            }
            continue;
        }
        let members: Vec<MarkTypeId> = marks
            .iter()
            .filter(|m| m.in_group(name))
            .map(|m| m.id)
            .collect();
        if members.is_empty() {
            return Err(name.to_owned());
        }
        for id in members {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out.sort_unstable();
    Ok(out)
}
