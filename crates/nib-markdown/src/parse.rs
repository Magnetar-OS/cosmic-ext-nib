// SPDX-License-Identifier: MPL-2.0

//! Markdown in.
//!
//! pulldown-cmark supplies the events in document order and this walks them
//! onto a stack of open nodes, asking the schema where each thing may go — the
//! same discipline as the HTML parser, and for the same reason: what comes out
//! is a valid document rather than a transcription.
//!
//! # Components
//!
//! MDC and MDX are handled by a pass *before* the Markdown parser rather than
//! inside it, because neither is Markdown: `::card{title="x"}` is a fence that
//! happens to contain Markdown, and `<Card title="x" />` is a tag that happens
//! to sit in it. Splitting the source into component and non-component
//! segments first means the Markdown parser never has to know they exist, and
//! the segments' contents go through it unchanged.

use nib_model::attrs::{Attrs, Value};
use nib_model::content::ContentMatch;
use nib_model::fragment::Fragment;
use nib_model::mark::{Mark, Marks};
use nib_model::node::Node;
use nib_model::schema::{NodeTypeId, Schema};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::Dialect;
use crate::components::{COMPONENT_BLOCK, COMPONENT_INLINE, ESM, NAME, SOURCE};

/// Parses Markdown into a document.
#[must_use]
pub fn parse(schema: &Schema, dialect: Dialect, text: &str) -> Node {
    let mut ctx = Ctx::new(schema);
    for segment in split(dialect, text) {
        match segment {
            Segment::Markdown(source) => walk(&mut ctx, dialect, &source),
            Segment::Component { name, props, body } => {
                let Some(typ) = schema.node_id(COMPONENT_BLOCK) else {
                    walk(&mut ctx, dialect, &body);
                    continue;
                };
                let inner = parse(schema, dialect, &body);
                let attrs = props.set(NAME, name.as_str());
                if let Ok(node) =
                    schema.create(typ, Some(&attrs), inner.content().clone(), Marks::none())
                {
                    ctx.add(node);
                }
            }
            Segment::Esm(source) => {
                let Some(typ) = schema.node_id(ESM) else {
                    continue;
                };
                let attrs = Attrs::none().set(SOURCE, source.as_str());
                if let Ok(node) = schema.create(typ, Some(&attrs), Fragment::empty(), Marks::none())
                {
                    ctx.add(node);
                }
            }
        }
    }
    ctx.finish()
}

// ---------------------------------------------------------------------------
// The component pre-pass
// ---------------------------------------------------------------------------

/// One run of the source: ordinary Markdown, a component, or an ESM line.
enum Segment {
    Markdown(String),
    Component {
        name: String,
        props: Attrs,
        body: String,
    },
    Esm(String),
}

/// Splits the source into segments. With no component dialect, one segment.
fn split(dialect: Dialect, text: &str) -> Vec<Segment> {
    if !dialect.has_components() {
        return vec![Segment::Markdown(text.to_owned())];
    }
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        // MDX keeps its module lines verbatim; they are not prose.
        if dialect == Dialect::Mdx && (line.starts_with("import ") || line.starts_with("export ")) {
            flush(&mut out, &mut plain);
            out.push(Segment::Esm(line.to_owned()));
            continue;
        }
        if dialect == Dialect::Mdc
            && let Some((fence, name, props)) = open_fence(line)
        {
            flush(&mut out, &mut plain);
            let mut body = String::new();
            for inner in lines.by_ref() {
                if closes(inner, fence) {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            out.push(Segment::Component { name, props, body });
            continue;
        }
        plain.push_str(line);
        plain.push('\n');
    }
    flush(&mut out, &mut plain);
    out
}

fn flush(out: &mut Vec<Segment>, plain: &mut String) {
    if plain.trim().is_empty() {
        plain.clear();
    } else {
        out.push(Segment::Markdown(std::mem::take(plain)));
    }
}

/// `::name{prop=value}` — the colons, the name, the props.
fn open_fence(line: &str) -> Option<(usize, String, Attrs)> {
    let trimmed = line.trim_end();
    let colons = trimmed.chars().take_while(|c| *c == ':').count();
    if colons < 2 {
        return None;
    }
    let rest = &trimmed[colons..];
    let name_end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    Some((colons, name.to_owned(), parse_props(&rest[name_end..])))
}

fn closes(line: &str, fence: usize) -> bool {
    let trimmed = line.trim();
    trimmed.chars().all(|c| c == ':') && trimmed.len() >= fence
}

/// `{a=1 b="two" flag .class #id}` into attributes.
///
/// Shorthands follow MDC: `.name` is a class, `#name` an id. Values are typed
/// where they obviously are — a bare word is a string, `true` a boolean, a
/// number a number — because a prop that arrives as a string and is compared
/// to a number is the bug this saves.
fn parse_props(source: &str) -> Attrs {
    let inner = source
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or("");
    let mut attrs = Attrs::none();
    let mut classes: Vec<String> = Vec::new();

    for token in tokenize_props(inner) {
        if let Some(class) = token.strip_prefix('.') {
            classes.push(class.to_owned());
        } else if let Some(id) = token.strip_prefix('#') {
            attrs = attrs.set("id", id);
        } else if let Some((name, value)) = token.split_once('=') {
            attrs = attrs.set(name.trim(), typed(value.trim()));
        } else if !token.is_empty() {
            attrs = attrs.set(token.as_str(), Value::Bool(true));
        }
    }
    if !classes.is_empty() {
        attrs = attrs.set("class", classes.join(" "));
    }
    attrs
}

/// Splits on whitespace, but not inside quotes.
fn tokenize_props(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in source.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            (_, c) => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn typed(value: &str) -> Value {
    // JSX writes non-strings as `{3}`; MDC writes them bare. Both arrive here.
    let value = value
        .strip_prefix('{')
        .and_then(|v| v.strip_suffix('}'))
        .unwrap_or(value);
    let bare = value.trim_matches(|c| c == '"' || c == '\'');
    if bare != value {
        return Value::Str(bare.into());
    }
    match bare {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => bare.parse::<i64>().map_or_else(
            |_| {
                bare.parse::<f64>()
                    .map_or_else(|_| Value::Str(bare.into()), Value::Float)
            },
            Value::Int,
        ),
    }
}

// ---------------------------------------------------------------------------
// The Markdown walk
// ---------------------------------------------------------------------------

struct Open {
    typ: NodeTypeId,
    attrs: Attrs,
    /// How far through this node's content expression its children have got.
    ///
    /// Advanced when a child is *opened*, not when it closes. That is the
    /// whole trick: a list item whose paragraph is still open must already
    /// know it has one, or the nested list arriving next will look impossible
    /// and be placed at the document root instead.
    matched: ContentMatch,
    content: Vec<Node>,
}

struct Ctx<'a> {
    schema: &'a Schema,
    stack: Vec<Open>,
    /// How deep the stack was when each still-open tag started.
    ///
    /// Placing content can open nodes the source never mentioned — a tight
    /// list item holds text directly, and the schema wants a paragraph around
    /// it. Without this, the item's own end event would close that implicit
    /// paragraph and leave the item open.
    open_tags: Vec<usize>,
    marks: Marks,
    /// An image being read: its target, title, and the alt text arriving as
    /// events between its start and end.
    pending_image: Option<(String, String, String)>,
}

impl<'a> Ctx<'a> {
    fn new(schema: &'a Schema) -> Self {
        let top = schema.top_node_type();
        Self {
            schema,
            stack: vec![Open {
                typ: top,
                attrs: Attrs::none(),
                matched: schema.node_type(top).content_match(),
                content: Vec::new(),
            }],
            open_tags: Vec::new(),
            marks: Marks::none(),
            pending_image: None,
        }
    }

    /// Opens a node context, advancing the parent's match past it.
    fn open(&mut self, typ: NodeTypeId, attrs: Attrs) {
        if let Some(parent) = self.stack.last_mut()
            && let Some(next) = parent.matched.match_type(typ)
        {
            parent.matched = next;
        }
        self.stack.push(Open {
            typ,
            attrs,
            matched: self.schema.node_type(typ).content_match(),
            content: Vec::new(),
        });
    }

    /// Closes the innermost context and appends it to its parent.
    ///
    /// No placement and no match advance: the parent accounted for this child
    /// when it was opened.
    fn close(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let open = self.stack.pop().expect("just checked the length");
        let content = Fragment::from_vec(open.content);
        let Some(node) =
            self.schema
                .create_and_fill(open.typ, Some(&open.attrs), content, Marks::none())
        else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.content.push(node);
        }
    }

    /// Opens a node for a tag, first finding somewhere it may go.
    fn open_tag(&mut self, typ: NodeTypeId, attrs: Attrs) {
        self.place(typ);
        self.open_tags.push(self.stack.len());
        self.open(typ, attrs);
    }

    /// Closes back to where the innermost open tag started, taking any nodes
    /// opened implicitly inside it.
    fn close_tag(&mut self) {
        let Some(depth) = self.open_tags.pop() else {
            self.close();
            return;
        };
        while self.stack.len() > depth {
            self.close();
        }
    }

    /// Closes what is in the way and opens what the schema requires, so that a
    /// node of `typ` may go in the innermost context.
    fn place(&mut self, typ: NodeTypeId) -> bool {
        for depth in (0..self.stack.len()).rev() {
            let Some(wrapping) = self.stack[depth].matched.find_wrapping(self.schema, typ) else {
                continue;
            };
            while self.stack.len() > depth + 1 {
                self.close();
            }
            for wrapper in wrapping {
                self.open(wrapper, Attrs::none());
            }
            return true;
        }
        false
    }

    /// Places a finished node and appends it, advancing the match.
    fn add(&mut self, node: Node) {
        self.place(node.type_id());
        self.push(node);
    }

    fn push(&mut self, node: Node) {
        let Some(open) = self.stack.last_mut() else {
            return;
        };
        if let Some(next) = open.matched.match_type(node.type_id()) {
            open.matched = next;
        }
        if let Some(last) = open.content.last()
            && last.is_text()
            && node.is_text()
            && last.same_markup(&node)
        {
            let joined = format!("{}{}", last.text().unwrap_or(""), node.text().unwrap_or(""));
            let index = open.content.len() - 1;
            open.content[index] = last.with_text(joined);
            return;
        }
        open.content.push(node);
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // An image's alt text arrives as the events inside it.
        if let Some((_, _, alt)) = self.pending_image.as_mut() {
            alt.push_str(text);
            return;
        }
        let allowed = self.stack.last().map_or_else(Marks::none, |o| {
            self.schema.node_type(o.typ).allowed_marks(&self.marks)
        });
        let node = self.schema.text(text, allowed);
        self.add(node);
    }

    /// Sets an attribute on the innermost open node of this type — how a task
    /// list marker, which arrives after its item has opened, reaches it.
    fn set_open_attr(&mut self, type_name: &str, name: &str, value: Value) {
        let Some(typ) = self.schema.node_id(type_name) else {
            return;
        };
        if let Some(open) = self.stack.iter_mut().rev().find(|o| o.typ == typ) {
            open.attrs = open.attrs.set(name, value);
        }
    }

    fn add_mark(&mut self, name: &str, attrs: Option<&Attrs>) {
        if let Ok(mark) = self.schema.mark(name, attrs) {
            self.marks = Mark::add_to_set(&mark, &self.marks);
        }
    }

    fn remove_mark(&mut self, name: &str) {
        let Some(id) = self.schema.mark_id(name) else {
            return;
        };
        self.marks = Marks::from_vec(
            self.marks
                .iter()
                .filter(|m| m.typ().id() != id)
                .cloned()
                .collect(),
        );
    }

    fn leaf(&mut self, name: &str, attrs: Option<&Attrs>) {
        let Some(typ) = self.schema.node_id(name) else {
            return;
        };
        if let Ok(node) = self
            .schema
            .create(typ, attrs, Fragment::empty(), Marks::none())
        {
            self.add(node);
        }
    }

    fn finish(mut self) -> Node {
        self.open_tags.clear();
        while self.stack.len() > 1 {
            self.close();
        }
        let open = self.stack.pop().expect("the top context is never popped");
        self.schema
            .create_and_fill(
                open.typ,
                None,
                Fragment::from_vec(open.content),
                Marks::none(),
            )
            .unwrap_or_else(|| self.schema.empty_doc())
    }
}

fn options(dialect: Dialect) -> Options {
    let mut options = Options::empty();
    if dialect.has_gfm() {
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);
    }
    options
}

#[allow(clippy::too_many_lines)]
fn walk(ctx: &mut Ctx<'_>, dialect: Dialect, source: &str) {
    use nib_model::basic::{marks, nodes};

    let mut in_head = false;
    for event in Parser::new_ext(source, options(dialect)) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => open_named(ctx, nodes::PARAGRAPH, Attrs::none()),
                Tag::Heading { level, .. } => {
                    let level = heading_level(level);
                    open_named(ctx, nodes::HEADING, Attrs::none().set("level", level));
                }
                Tag::BlockQuote(_) => open_named(ctx, nodes::BLOCKQUOTE, Attrs::none()),
                Tag::CodeBlock(kind) => {
                    let language = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or(""),
                        CodeBlockKind::Indented => "",
                    };
                    let attrs = if language.is_empty() {
                        Attrs::none()
                    } else {
                        Attrs::none().set("language", language)
                    };
                    open_named(ctx, nodes::CODE_BLOCK, attrs);
                }
                Tag::List(Some(start)) => open_named(
                    ctx,
                    nodes::ORDERED_LIST,
                    Attrs::none().set("start", i64::try_from(start).unwrap_or(1)),
                ),
                Tag::List(None) => open_named(ctx, nodes::BULLET_LIST, Attrs::none()),
                Tag::Item => open_named(ctx, nodes::LIST_ITEM, Attrs::none()),
                Tag::Table(_) => open_named(ctx, nodes::TABLE, Attrs::none()),
                Tag::TableHead => {
                    in_head = true;
                    open_named(ctx, nodes::TABLE_ROW, Attrs::none());
                }
                Tag::TableRow => open_named(ctx, nodes::TABLE_ROW, Attrs::none()),
                Tag::TableCell => {
                    let name = if in_head {
                        nodes::TABLE_HEADER
                    } else {
                        nodes::TABLE_CELL
                    };
                    open_named(ctx, name, Attrs::none());
                }
                Tag::Emphasis => ctx.add_mark(marks::EM, None),
                Tag::Strong => ctx.add_mark(marks::STRONG, None),
                Tag::Strikethrough => ctx.add_mark(marks::STRIKETHROUGH, None),
                Tag::Link {
                    dest_url, title, ..
                } => {
                    let mut attrs = Attrs::none().set("href", dest_url.as_ref());
                    if !title.is_empty() {
                        attrs = attrs.set("title", title.as_ref());
                    }
                    ctx.add_mark(marks::LINK, Some(&attrs));
                }
                Tag::Image {
                    dest_url, title, ..
                } => {
                    ctx.pending_image =
                        Some((dest_url.to_string(), title.to_string(), String::new()));
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Emphasis => ctx.remove_mark(marks::EM),
                TagEnd::Strong => ctx.remove_mark(marks::STRONG),
                TagEnd::Strikethrough => ctx.remove_mark(marks::STRIKETHROUGH),
                TagEnd::Link => ctx.remove_mark(marks::LINK),
                TagEnd::TableHead => {
                    in_head = false;
                    ctx.close_tag();
                }
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableRow
                | TagEnd::TableCell => ctx.close_tag(),
                TagEnd::Image => {
                    if let Some((src, title, alt)) = ctx.pending_image.take() {
                        let mut attrs = Attrs::none().set("src", src.as_str());
                        if !alt.is_empty() {
                            attrs = attrs.set("alt", alt.as_str());
                        }
                        if !title.is_empty() {
                            attrs = attrs.set("title", title.as_str());
                        }
                        ctx.leaf(nodes::IMAGE, Some(&attrs));
                    }
                }
                _ => {}
            },
            Event::Text(text) => ctx.text(&text),
            Event::Code(code) => {
                ctx.add_mark(marks::CODE, None);
                ctx.text(&code);
                ctx.remove_mark(marks::CODE);
            }
            Event::SoftBreak => ctx.text(" "),
            Event::HardBreak => ctx.leaf(nodes::HARD_BREAK, None),
            Event::Rule => ctx.leaf(nodes::HORIZONTAL_RULE, None),
            Event::TaskListMarker(checked) => {
                ctx.set_open_attr(nodes::LIST_ITEM, "checked", Value::Bool(checked));
            }
            Event::Html(html) | Event::InlineHtml(html) if dialect == Dialect::Mdx => {
                mdx_tag(ctx, &html);
            }
            _ => {}
        }
    }
}

fn open_named(ctx: &mut Ctx<'_>, name: &str, attrs: Attrs) {
    match ctx.schema.node_id(name) {
        Some(typ) => ctx.open_tag(typ, attrs),
        // The schema has no such type: still record the tag, so its end event
        // closes nothing rather than closing something else.
        None => ctx.open_tags.push(ctx.stack.len()),
    }
}

fn heading_level(level: HeadingLevel) -> i64 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// `<Name prop="v" />` — a JSX-shaped component, as MDX spells it.
///
/// Only self-closing and opening tags with a capitalised name are taken as
/// components; a lowercase tag is HTML and is not this crate's business.
fn mdx_tag(ctx: &mut Ctx<'_>, html: &str) {
    let trimmed = html.trim();
    let Some(rest) = trimmed.strip_prefix('<') else {
        return;
    };
    let rest = rest.trim_end_matches('>').trim_end_matches('/');
    let name_end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() || !name.starts_with(char::is_uppercase) {
        return;
    }
    let props = parse_props(&format!("{{{}}}", &rest[name_end..]));
    let attrs = props.set(NAME, name);
    let inline = ctx
        .stack
        .last()
        .is_some_and(|o| ctx.schema.node_type(o.typ).is_inline_content());
    let type_name = if inline {
        COMPONENT_INLINE
    } else {
        COMPONENT_BLOCK
    };
    ctx.leaf(type_name, Some(&attrs));
}

/// The alignment of a table column, kept for a serialiser that wants it.
#[must_use]
pub fn alignment_name(alignment: Alignment) -> Option<&'static str> {
    match alignment {
        Alignment::None => None,
        Alignment::Left => Some("left"),
        Alignment::Center => Some("center"),
        Alignment::Right => Some("right"),
    }
}
