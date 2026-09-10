// SPDX-License-Identifier: MPL-2.0

//! What HTML means, in both directions.
//!
//! One table serves parsing and serialising, because a round trip that loses
//! something is a table where the two halves disagreed. A rule says: this tag
//! is that node type, these attributes come from those; and, going the other
//! way, this node type is that tag with those attributes.

use std::collections::BTreeMap;
use std::sync::Arc;

use nib_model::attrs::{Attrs, Value};
use nib_model::node::Node;
use nib_model::schema::{MarkTypeId, NodeTypeId, Schema};

/// An HTML element's attributes, as parsed.
pub type ElementAttrs = BTreeMap<String, String>;

/// An element, as a rule sees it.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub tag: String,
    pub attrs: ElementAttrs,
    /// The attributes of the first child element of each tag name.
    ///
    /// One level, and no more: `<pre>` needs to read the `language-rust` class
    /// off its `<code>`, and a rule that needed to walk further would be doing
    /// the parser's job.
    pub children: BTreeMap<String, ElementAttrs>,
}

impl Element {
    /// An attribute of the element itself.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    /// An attribute of the first child element with this tag.
    #[must_use]
    pub fn child_attr(&self, tag: &str, name: &str) -> Option<&str> {
        self.children.get(tag)?.get(name).map(String::as_str)
    }
}

/// Reads a node or mark's attributes out of an element.
pub type ReadAttrs = Arc<dyn Fn(&Element) -> Option<Attrs> + Send + Sync>;

/// Writes a node or mark's attributes into an element's.
pub type WriteAttrs = Arc<dyn Fn(&Attrs) -> ElementAttrs + Send + Sync>;

/// What an element becomes.
#[derive(Clone)]
pub enum Target {
    /// A node of this type.
    Node(NodeTypeId),
    /// A mark on whatever is inside it.
    Mark(MarkTypeId),
    /// The element and everything in it is dropped. `<script>`, `<style>`.
    Ignore,
    /// The element is skipped and its children parsed in its place. `<div>`,
    /// `<span>` with nothing to say.
    Transparent,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(id) => write!(f, "Node({id})"),
            Self::Mark(id) => write!(f, "Mark({id})"),
            Self::Ignore => f.write_str("Ignore"),
            Self::Transparent => f.write_str("Transparent"),
        }
    }
}

/// How one tag is read.
#[derive(Clone)]
pub struct ParseRule {
    pub tag: String,
    pub target: Target,
    pub attrs: Option<ReadAttrs>,
    /// Higher wins when two rules claim the same tag. Lets an application
    /// override a default without removing it.
    pub priority: i32,
    /// A rule that only applies when the element carries this attribute with
    /// this value — how `<td>` and `<th>`, or a `<span class="mention">`, are
    /// told apart.
    pub requires: Option<(String, Option<String>)>,
}

impl std::fmt::Debug for ParseRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseRule")
            .field("tag", &self.tag)
            .field("target", &self.target)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

impl ParseRule {
    #[must_use]
    pub fn new(tag: impl Into<String>, target: Target) -> Self {
        Self {
            tag: tag.into(),
            target,
            attrs: None,
            priority: 0,
            requires: None,
        }
    }

    #[must_use]
    pub fn reading(
        mut self,
        f: impl Fn(&Element) -> Option<Attrs> + Send + Sync + 'static,
    ) -> Self {
        self.attrs = Some(Arc::new(f));
        self
    }

    #[must_use]
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Restricts the rule to elements carrying an attribute.
    #[must_use]
    pub fn requiring(mut self, name: impl Into<String>, value: Option<String>) -> Self {
        self.requires = Some((name.into(), value));
        self
    }

    pub(crate) fn matches(&self, tag: &str, attrs: &ElementAttrs) -> bool {
        if self.tag != tag {
            return false;
        }
        match &self.requires {
            None => true,
            Some((name, None)) => attrs.contains_key(name),
            Some((name, Some(want))) => attrs.get(name).is_some_and(|v| v == want),
        }
    }
}

/// How one node or mark type is written.
#[derive(Clone)]
pub struct WriteRule {
    pub tag: String,
    pub attrs: Option<WriteAttrs>,
    /// Tags to wrap the content in, innermost last — `<pre><code>` for a code
    /// block.
    pub inner: Vec<String>,
    /// Whether the element takes no children: `<br>`, `<hr>`, `<img>`.
    pub void: bool,
}

impl std::fmt::Debug for WriteRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteRule")
            .field("tag", &self.tag)
            .field("inner", &self.inner)
            .field("void", &self.void)
            .finish_non_exhaustive()
    }
}

impl WriteRule {
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            attrs: None,
            inner: Vec::new(),
            void: false,
        }
    }

    #[must_use]
    pub fn writing(mut self, f: impl Fn(&Attrs) -> ElementAttrs + Send + Sync + 'static) -> Self {
        self.attrs = Some(Arc::new(f));
        self
    }

    #[must_use]
    pub fn wrapping(mut self, inner: impl Into<String>) -> Self {
        self.inner.push(inner.into());
        self
    }

    #[must_use]
    pub fn void(mut self) -> Self {
        self.void = true;
        self
    }
}

/// The whole table, for one schema.
#[derive(Debug, Clone)]
pub struct Rules {
    pub(crate) schema: Schema,
    pub(crate) parse: Vec<ParseRule>,
    pub(crate) write_nodes: BTreeMap<NodeTypeId, WriteRule>,
    pub(crate) write_marks: BTreeMap<MarkTypeId, WriteRule>,
}

impl Rules {
    /// An empty table: every tag transparent, nothing serialisable.
    #[must_use]
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            parse: Vec::new(),
            write_nodes: BTreeMap::new(),
            write_marks: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Adds a parse rule.
    #[must_use]
    pub fn parsing(mut self, rule: ParseRule) -> Self {
        self.parse.push(rule);
        self.parse.sort_by_key(|r| -r.priority);
        self
    }

    /// Adds a serialisation rule for a node type.
    #[must_use]
    pub fn writing_node(mut self, typ: NodeTypeId, rule: WriteRule) -> Self {
        self.write_nodes.insert(typ, rule);
        self
    }

    /// Adds a serialisation rule for a mark type.
    #[must_use]
    pub fn writing_mark(mut self, typ: MarkTypeId, rule: WriteRule) -> Self {
        self.write_marks.insert(typ, rule);
        self
    }

    /// Both directions at once, for a node type that maps to one plain tag.
    #[must_use]
    pub fn node(self, typ: NodeTypeId, tag: &str) -> Self {
        self.parsing(ParseRule::new(tag, Target::Node(typ)))
            .writing_node(typ, WriteRule::new(tag))
    }

    /// Both directions at once, for a mark type that maps to one plain tag.
    #[must_use]
    pub fn mark(self, typ: MarkTypeId, tag: &str) -> Self {
        self.parsing(ParseRule::new(tag, Target::Mark(typ)))
            .writing_mark(typ, WriteRule::new(tag))
    }

    /// A tag that is read as a mark but never written — `<b>` alongside
    /// `<strong>`.
    #[must_use]
    pub fn mark_alias(self, typ: MarkTypeId, tag: &str) -> Self {
        self.parsing(ParseRule::new(tag, Target::Mark(typ)))
    }

    pub(crate) fn rule_for(&self, tag: &str, attrs: &ElementAttrs) -> Option<&ParseRule> {
        self.parse.iter().find(|r| r.matches(tag, attrs))
    }

    pub(crate) fn write_node(&self, node: &Node) -> Option<&WriteRule> {
        self.write_nodes.get(&node.type_id())
    }
}

/// The rules for the standard schema, matched by name.
///
/// A schema that renames its types builds its own table; one built on
/// [`basic`](nib_model::basic) gets all of this.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn base(schema: &Schema) -> Rules {
    use nib_model::basic::{marks, nodes};

    let mut rules = Rules::new(schema.clone());
    let node = |name: &str| schema.node_id(name);
    let mark = |name: &str| schema.mark_id(name);

    // -- structure ---------------------------------------------------------
    if let Some(id) = node(nodes::PARAGRAPH) {
        rules = rules.node(id, "p");
    }
    if let Some(id) = node(nodes::BLOCKQUOTE) {
        rules = rules.node(id, "blockquote");
    }
    if let Some(id) = node(nodes::HORIZONTAL_RULE) {
        rules = rules
            .parsing(ParseRule::new("hr", Target::Node(id)))
            .writing_node(id, WriteRule::new("hr").void());
    }
    if let Some(id) = node(nodes::HEADING) {
        for level in 1..=6_i64 {
            rules = rules.parsing(
                ParseRule::new(format!("h{level}"), Target::Node(id))
                    .reading(move |_| Some(nib_model::attrs! { "level" => level })),
            );
        }
        rules = rules.writing_node(
            id,
            // The tag depends on the level, so serialisation looks at the
            // attributes; `tag` here is only the fallback.
            WriteRule::new("h1").writing(|attrs| {
                let level = attrs.get_int("level").unwrap_or(1).clamp(1, 6);
                BTreeMap::from([("__tag".to_owned(), format!("h{level}"))])
            }),
        );
    }
    if let Some(id) = node(nodes::CODE_BLOCK) {
        rules = rules
            .parsing(
                ParseRule::new("pre", Target::Node(id))
                    .priority(10)
                    // Highlighters spell the language as a class on the inner
                    // `<code>`; that is where it has to be read from.
                    .reading(|el| {
                        let class = el.child_attr("code", "class")?;
                        let language = class
                            .split_whitespace()
                            .find_map(|c| c.strip_prefix("language-"))?;
                        Some(nib_model::attrs! { "language" => language })
                    }),
            )
            .writing_node(
                id,
                WriteRule::new("pre").wrapping("code").writing(|attrs| {
                    match attrs.get_str("language") {
                        Some(lang) if !lang.is_empty() => BTreeMap::from([(
                            "__inner_class".to_owned(),
                            format!("language-{lang}"),
                        )]),
                        _ => BTreeMap::new(),
                    }
                }),
            );
    }

    // -- lists -------------------------------------------------------------
    if let Some(id) = node(nodes::BULLET_LIST) {
        rules = rules.node(id, "ul");
    }
    if let Some(id) = node(nodes::ORDERED_LIST) {
        rules = rules
            .parsing(ParseRule::new("ol", Target::Node(id)).reading(|el| {
                let start = el.attr("start")?.parse::<i64>().ok()?;
                Some(nib_model::attrs! { "start" => start })
            }))
            .writing_node(
                id,
                WriteRule::new("ol").writing(|attrs| match attrs.get_int("start") {
                    Some(start) if start != 1 => {
                        BTreeMap::from([("start".to_owned(), start.to_string())])
                    }
                    _ => BTreeMap::new(),
                }),
            );
    }
    if let Some(id) = node(nodes::LIST_ITEM) {
        rules = rules.node(id, "li");
    }

    // -- tables ------------------------------------------------------------
    if let Some(id) = node(nodes::TABLE) {
        rules = rules.node(id, "table");
    }
    if let Some(id) = node(nodes::TABLE_ROW) {
        rules = rules.node(id, "tr");
    }
    for (name, tag) in [(nodes::TABLE_CELL, "td"), (nodes::TABLE_HEADER, "th")] {
        if let Some(id) = node(name) {
            rules = rules
                .parsing(ParseRule::new(tag, Target::Node(id)).reading(|el| Some(span_attrs(el))))
                .writing_node(id, WriteRule::new(tag).writing(write_span_attrs));
        }
    }
    // `<tbody>`, `<thead>` and `<tfoot>` carry no meaning the model keeps.
    for tag in ["tbody", "thead", "tfoot", "colgroup", "col"] {
        rules = rules.parsing(ParseRule::new(tag, Target::Transparent));
    }

    // -- inline ------------------------------------------------------------
    if let Some(id) = node(nodes::HARD_BREAK) {
        rules = rules
            .parsing(ParseRule::new("br", Target::Node(id)))
            .writing_node(id, WriteRule::new("br").void());
    }
    if let Some(id) = node(nodes::IMAGE) {
        rules = rules
            .parsing(ParseRule::new("img", Target::Node(id)).reading(|el| {
                let mut out = nib_model::attrs! { "src" => el.attr("src")? };
                if let Some(alt) = el.attr("alt") {
                    out = out.set("alt", alt);
                }
                if let Some(title) = el.attr("title") {
                    out = out.set("title", title);
                }
                Some(out)
            }))
            .writing_node(
                id,
                WriteRule::new("img").void().writing(|attrs| {
                    let mut out = BTreeMap::new();
                    for name in ["src", "alt", "title"] {
                        if let Some(value) = attrs.get(name)
                            && !value.is_null()
                        {
                            out.insert(name.to_owned(), value.to_string());
                        }
                    }
                    out
                }),
            );
    }

    // -- marks -------------------------------------------------------------
    if let Some(id) = mark(marks::STRONG) {
        rules = rules.mark(id, "strong").mark_alias(id, "b");
    }
    if let Some(id) = mark(marks::EM) {
        rules = rules.mark(id, "em").mark_alias(id, "i");
    }
    if let Some(id) = mark(marks::UNDERLINE) {
        rules = rules.mark(id, "u");
    }
    if let Some(id) = mark(marks::STRIKETHROUGH) {
        rules = rules
            .mark(id, "s")
            .mark_alias(id, "del")
            .mark_alias(id, "strike");
    }
    if let Some(id) = mark(marks::CODE) {
        rules = rules.mark(id, "code");
    }

    // Authored styling goes back out as `<span style>`, never as the `<font>`
    // or `bgcolor` it may have arrived as. Reading many spellings and writing
    // one is the point of having a model in the middle: what comes out says
    // what the document means, not what its author's generator happened to
    // emit in 2004.
    //
    // There is no `parsing` rule here — `styling::of` reads `style=` off every
    // element, not off a list of tags, so a colour on a `<div>` or a `<td>`
    // reaches its text the same way one on a `<span>` does.
    if let Some(id) = mark(marks::TEXT_COLOR) {
        rules = rules.writing_mark(
            id,
            WriteRule::new("span").writing(|attrs| style_attr("color", attrs.get_str("value"))),
        );
    }
    if let Some(id) = mark(marks::BACKGROUND_COLOR) {
        rules = rules.writing_mark(
            id,
            WriteRule::new("span")
                .writing(|attrs| style_attr("background-color", attrs.get_str("value"))),
        );
    }
    if let Some(id) = mark(marks::FONT_SIZE) {
        rules = rules.writing_mark(
            id,
            WriteRule::new("span").writing(|attrs| {
                let Some(scale) = attrs.get_float("scale") else {
                    return BTreeMap::new();
                };
                // Back out as a percentage, which is what it always was: the
                // ratio the author chose, not a measurement in anyone's pixels.
                style_attr("font-size", Some(&format!("{:.0}%", scale * 100.0)))
            }),
        );
    }
    if let Some(id) = mark(marks::LINK) {
        rules = rules
            .parsing(ParseRule::new("a", Target::Mark(id)).reading(|el| {
                let mut out = nib_model::attrs! { "href" => el.attr("href")? };
                if let Some(title) = el.attr("title") {
                    out = out.set("title", title);
                }
                Some(out)
            }))
            .writing_mark(
                id,
                WriteRule::new("a").writing(|attrs| {
                    let mut out = BTreeMap::new();
                    for name in ["href", "title"] {
                        if let Some(value) = attrs.get(name)
                            && !value.is_null()
                        {
                            out.insert(name.to_owned(), value.to_string());
                        }
                    }
                    out
                }),
            );
    }

    // -- dropped and skipped -----------------------------------------------
    for tag in [
        "script", "style", "head", "meta", "link", "title", "noscript",
    ] {
        rules = rules.parsing(ParseRule::new(tag, Target::Ignore).priority(100));
    }
    for tag in [
        "div",
        "span",
        "section",
        "article",
        "main",
        "header",
        "footer",
        "nav",
        "aside",
        "body",
        "html",
        "font",
        "center",
        "small",
        "big",
        "figure",
        "figcaption",
        "picture",
        "source",
    ] {
        rules = rules.parsing(ParseRule::new(tag, Target::Transparent).priority(-10));
    }

    rules
}

/// One declaration, as the `style` attribute a span carries.
fn style_attr(property: &str, value: Option<&str>) -> ElementAttrs {
    let mut out = BTreeMap::new();
    if let Some(value) = value {
        out.insert("style".to_owned(), format!("{property}: {value}"));
    }
    out
}

fn span_attrs(el: &Element) -> Attrs {
    let mut out = Attrs::none();
    for name in ["colspan", "rowspan"] {
        if let Some(value) = el.attr(name).and_then(|v| v.parse::<i64>().ok()) {
            out = out.set(name, value);
        }
    }
    if let Some(align) = el.attr("align") {
        out = out.set("align", align);
    }
    out
}

fn write_span_attrs(attrs: &Attrs) -> ElementAttrs {
    let mut out = BTreeMap::new();
    for name in ["colspan", "rowspan"] {
        if let Some(value) = attrs.get_int(name)
            && value != 1
        {
            out.insert(name.to_owned(), value.to_string());
        }
    }
    if let Some(Value::Str(align)) = attrs.get("align") {
        out.insert("align".to_owned(), align.to_string());
    }
    out
}
