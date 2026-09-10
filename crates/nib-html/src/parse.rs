// SPDX-License-Identifier: MPL-2.0

//! HTML in.
//!
//! # One parser
//!
//! html5ever, and nothing else. Real-world HTML is not a tree until a
//! specification-conformant parser has made it one — unclosed `<li>`s,
//! `<table>`s with implied `<tbody>`, text where a `<tr>` should be — and a
//! hand-written tokeniser is how a sanitiser and a renderer come to disagree
//! about what some bytes mean. Every sanitiser bypass ever written is that
//! disagreement.
//!
//! # And one schema
//!
//! What comes out is not the HTML tree; it is the closest *valid document*.
//! The parse walks the DOM and asks the schema where each thing may go, using
//! the same [`ContentMatch`](nib_model::ContentMatch) searches the fitter uses:
//! a `<p>` inside a `<p>` becomes two paragraphs, a `<li>` outside a list gets
//! a list built around it, and anything with nowhere to go is dropped rather
//! than smuggled in. Nothing downstream has to re-check the result.

use std::collections::BTreeMap;

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use nib_model::attrs::Attrs;
use nib_model::content::ContentMatch;
use nib_model::fragment::Fragment;
use nib_model::mark::{Mark, Marks};
use nib_model::node::Node;
use nib_model::schema::{NodeTypeId, Whitespace};
use nib_model::slice::Slice;

use crate::rules::{Element, ElementAttrs, Rules, Target};

/// A node being built.
struct Open {
    typ: NodeTypeId,
    attrs: Attrs,
    matched: ContentMatch,
    content: Vec<Node>,
    /// The outermost context: content that will not go anywhere inside it is
    /// dropped rather than escaping the document.
    solid: bool,
    /// Whitespace is kept exactly — inside a code block.
    preserve: bool,
}

struct Ctx<'a> {
    rules: &'a Rules,
    stack: Vec<Open>,
    marks: Marks,
    /// True when the last thing written was whitespace, so a run of it
    /// collapses to one space.
    pending_space: bool,
}

impl<'a> Ctx<'a> {
    fn new(rules: &'a Rules, top: NodeTypeId) -> Self {
        let schema = rules.schema();
        Self {
            rules,
            stack: vec![Open {
                typ: top,
                attrs: Attrs::none(),
                matched: schema.node_type(top).content_match(),
                content: Vec::new(),
                solid: true,
                preserve: false,
            }],
            marks: Marks::none(),
            pending_space: false,
        }
    }

    fn depth(&self) -> usize {
        self.stack.len() - 1
    }

    fn preserving(&self) -> bool {
        self.stack.last().is_some_and(|o| o.preserve)
    }

    /// Whether the innermost open node's schema permits this mark type.
    fn allows_mark(&self, mark: nib_model::schema::MarkTypeId) -> bool {
        self.stack
            .last()
            .is_some_and(|o| self.rules.schema().node_type(o.typ).allows_mark_type(mark))
    }

    /// Opens a node context.
    fn open(&mut self, typ: NodeTypeId, attrs: Attrs) {
        let t = self.rules.schema().node_type(typ);
        let preserve = self.preserving() || t.spec().whitespace == Whitespace::Pre;
        self.stack.push(Open {
            typ,
            attrs,
            matched: t.content_match(),
            content: Vec::new(),
            solid: false,
            preserve,
        });
    }

    /// Closes the innermost context, filling in anything the schema still
    /// requires, and places the finished node in its parent.
    fn close(&mut self) {
        let Some(open) = self.stack.pop() else { return };
        let schema = self.rules.schema();
        let mut content = Fragment::from_vec(open.content);
        // A `<li>` that held only text needs the paragraph its schema requires.
        if let Some(matched) = schema
            .node_type(open.typ)
            .content_match()
            .match_fragment(&content)
            && let Some(fill) = matched.fill_before(schema, &Fragment::empty(), true, 0)
        {
            content = content.append(&fill);
        }
        let Ok(node) = schema.create(open.typ, Some(&open.attrs), content, Marks::none()) else {
            return;
        };
        self.add(node);
    }

    /// Closes contexts until `depth` is innermost.
    fn close_to(&mut self, depth: usize) {
        while self.depth() > depth {
            self.close();
        }
    }

    /// Places a finished node, opening whatever the schema needs around it and
    /// closing whatever is in the way.
    ///
    /// Returns false when nothing anywhere would take it — the node is
    /// dropped, which is the only honest answer for `<li>` in a code block.
    fn add(&mut self, node: Node) -> bool {
        let schema = self.rules.schema().clone();
        let typ = node.type_id();

        for depth in (0..self.stack.len()).rev() {
            if let Some(wrapping) = self.stack[depth].matched.find_wrapping(&schema, typ) {
                self.close_to(depth);
                for wrapper in wrapping {
                    self.open(wrapper, Attrs::none());
                }
                self.push(node);
                return true;
            }
            // Something may be required before it — a paragraph before a
            // nested list inside a list item.
            if let Some(fill) = self.stack[depth].matched.fill_before(
                &schema,
                &Fragment::from(node.clone()),
                false,
                0,
            ) && !fill.is_empty()
            {
                self.close_to(depth);
                for filler in &fill {
                    self.push(filler.clone());
                }
                if let Some(wrapping) = self.stack[depth].matched.find_wrapping(&schema, typ) {
                    for wrapper in wrapping {
                        self.open(wrapper, Attrs::none());
                    }
                    self.push(node);
                    return true;
                }
            }
            if self.stack[depth].solid {
                break;
            }
        }
        false
    }

    /// Appends to the innermost context, advancing its match.
    fn push(&mut self, node: Node) {
        let schema = self.rules.schema().clone();
        let Some(open) = self.stack.last_mut() else {
            return;
        };
        // A mark may have been opened where it was allowed and the text may
        // have travelled somewhere it is not. The node it lands in has the
        // last word.
        let typ = schema.node_type(open.typ);
        let node = if node.marks().is_empty() {
            node
        } else {
            node.with_marks(typ.allowed_marks(node.marks()))
        };
        if let Some(next) = open.matched.match_type(node.type_id()) {
            open.matched = next;
        }
        // Two text runs that arrived as separate elements but carry the same
        // markup are one run. Leaving them split makes a redraw check report a
        // difference the reader cannot see.
        if let Some(last) = open.content.last()
            && last.is_text()
            && node.is_text()
            && last.same_markup(&node)
        {
            let joined = format!("{}{}", last.text().unwrap_or(""), node.text().unwrap_or(""));
            let merged = last.with_text(joined);
            let index = open.content.len() - 1;
            open.content[index] = merged;
            return;
        }
        open.content.push(node);
    }

    /// Adds text, collapsing whitespace unless the context preserves it.
    fn text(&mut self, raw: &str) {
        let schema = self.rules.schema().clone();
        let text = if self.preserving() {
            raw.to_owned()
        } else {
            let collapsed = collapse(raw);
            if collapsed.is_empty() {
                // Whitespace between blocks is not content, but whitespace
                // between two inline runs is.
                if raw.chars().any(char::is_whitespace) {
                    self.pending_space = true;
                }
                return;
            }
            let leading = raw.starts_with(char::is_whitespace);
            let mut out = String::new();
            if (leading || self.pending_space) && self.has_inline_content() {
                out.push(' ');
            }
            out.push_str(&collapsed);
            if raw.ends_with(char::is_whitespace) {
                out.push(' ');
            }
            out
        };
        self.pending_space = false;
        if text.is_empty() {
            return;
        }
        let node = schema.text(text.as_str(), self.marks.clone());
        // Text can only go where inline content is allowed; `add` finds a
        // textblock to put it in, creating one if the schema offers.
        self.add(node);
    }

    /// True when the innermost context already holds inline content, so a
    /// leading space would separate two runs rather than indent a block.
    fn has_inline_content(&self) -> bool {
        self.stack
            .last()
            .and_then(|o| o.content.last())
            .is_some_and(Node::is_inline)
    }

    fn finish(mut self) -> Node {
        while self.stack.len() > 1 {
            self.close();
        }
        let open = self.stack.pop().expect("the top context is never popped");
        let schema = self.rules.schema();
        let content = Fragment::from_vec(open.content);
        schema
            .create_and_fill(open.typ, None, content, Marks::none())
            .unwrap_or_else(|| schema.empty_doc())
    }
}

/// Collapses runs of whitespace to single spaces and trims the ends.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads an element's attributes into a plain map.
fn element_attrs(attrs: &[html5ever::Attribute]) -> ElementAttrs {
    attrs
        .iter()
        .map(|a| (a.name.local.to_string(), a.value.to_string()))
        .collect()
}

/// The attributes of the first child element of each tag name.
///
/// Enough for a rule to look one level in without this module handing
/// html5ever's types to the rule table.
fn child_elements(handle: &Handle) -> BTreeMap<String, ElementAttrs> {
    let mut out = BTreeMap::new();
    for child in handle.children.borrow().iter() {
        if let NodeData::Element { name, attrs, .. } = &child.data {
            out.entry(name.local.to_string())
                .or_insert_with(|| element_attrs(&attrs.borrow()));
        }
    }
    out
}

fn walk(ctx: &mut Ctx<'_>, handle: &Handle) {
    match &handle.data {
        NodeData::Text { contents } => {
            ctx.text(&contents.borrow());
        }
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string();
            let attrs = element_attrs(&attrs.borrow());
            let element = Element {
                tag: tag.clone(),
                children: child_elements(handle),
                attrs: attrs.clone(),
            };
            // Styling first, and for *every* element rather than only the ones
            // a rule names: a `<div style="color:#c00">` has no mark of its
            // own — it is transparent — and its colour still belongs to the
            // text inside it. Read before the rule is looked up so the two
            // cannot disagree about which elements are styled.
            let styling = crate::styling::of(ctx.rules.schema(), &element);
            let outer_marks = ctx.marks.clone();
            for mark in &styling.marks {
                ctx.marks = Mark::add_to_set(mark, &ctx.marks);
            }

            let Some(rule) = ctx.rules.rule_for(&tag, &attrs) else {
                // An unknown tag is a container, not content. Dropping it
                // outright would lose text; treating it as its own node would
                // invent structure the schema never declared.
                walk_children(ctx, handle);
                ctx.marks = outer_marks;
                return;
            };
            walk_rule(ctx, handle, &element, rule, &styling);
            ctx.marks = outer_marks;
        }
        NodeData::Document | NodeData::Doctype { .. } => walk_children(ctx, handle),
        NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. } => {}
    }
}

/// What one element's rule does, with its styling already on the mark stack.
fn walk_rule(
    ctx: &mut Ctx<'_>,
    handle: &Handle,
    element: &Element,
    rule: &crate::rules::ParseRule,
    styling: &crate::styling::Styling,
) {
    match rule.target.clone() {
        Target::Ignore => {}
        Target::Transparent => walk_children(ctx, handle),
        Target::Mark(id) => {
            // A mark the surrounding node forbids is not applied — the
            // element becomes transparent. This is what makes
            // `<pre><code>` one code block rather than a code block
            // full of inline code, without a special case for it.
            if !ctx.allows_mark(id) {
                walk_children(ctx, handle);
                return;
            }
            let mark_attrs = rule.attrs.as_ref().and_then(|f| f(element));
            let Ok(mark) = ctx.rules.schema().mark_by_id(id, mark_attrs.as_ref()) else {
                walk_children(ctx, handle);
                return;
            };
            let outer = ctx.marks.clone();
            ctx.marks = Mark::add_to_set(&mark, &outer);
            walk_children(ctx, handle);
            ctx.marks = outer;
        }
        Target::Node(id) => {
            let mut node_attrs = rule
                .attrs
                .as_ref()
                .and_then(|f| f(element))
                .unwrap_or_else(Attrs::none);
            let schema = ctx.rules.schema().clone();
            // Alignment is a block property, so it lands on the node
            // rather than on the text inside it — and only where the
            // schema declared somewhere to put it.
            if let Some(align) = styling.align
                && schema
                    .node_type(id)
                    .spec()
                    .attrs
                    .contains_key(nib_model::basic::attrs::ALIGN)
            {
                node_attrs = node_attrs.set(nib_model::basic::attrs::ALIGN, align.as_str());
            }
            if schema.node_type(id).is_leaf() {
                if let Ok(node) =
                    schema.create(id, Some(&node_attrs), Fragment::empty(), Marks::none())
                {
                    ctx.add(node);
                }
                return;
            }
            let depth = ctx.depth();
            ctx.open(id, node_attrs);
            // A block boundary breaks a run of inline whitespace.
            ctx.pending_space = false;
            walk_children(ctx, handle);
            ctx.close_to(depth);
            ctx.pending_space = false;
        }
    }
}

fn walk_children(ctx: &mut Ctx<'_>, handle: &Handle) {
    for child in handle.children.borrow().iter() {
        walk(ctx, child);
    }
}

/// Parses a document.
///
/// # Panics
///
/// Never: html5ever recovers from anything, and a reader over a `&str` cannot
/// fail.
///
/// Never fails: HTML has no parse errors, only recoveries, and anything the
/// schema will not hold is dropped. An empty or unusable input gives the
/// schema's empty document.
#[must_use]
pub fn parse(rules: &Rules, html: &str) -> Node {
    let dom = html5ever::parse_document(RcDom::default(), html5ever::ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap_or_default();
    let mut ctx = Ctx::new(rules, rules.schema().top_node_type());
    walk(&mut ctx, &dom.document);
    ctx.finish()
}

/// Parses a fragment as a slice, for a paste.
///
/// The open depths are worked out from the shape of what came back: a fragment
/// that is one textblock's worth of inline content is opened at both ends, so
/// pasting it into a paragraph continues the paragraph rather than starting a
/// new one.
///
/// # Panics
///
/// Never; see [`parse`].
#[must_use]
pub fn parse_slice(rules: &Rules, html: &str) -> Slice {
    let doc = parse(rules, html);
    let content = doc.content().clone();
    if content.is_empty() {
        return Slice::empty();
    }
    // A single textblock came from inline HTML: hand back its content, open.
    if content.child_count() == 1
        && content.child(0).is_some_and(Node::is_textblock)
        && !html_looks_like_blocks(html)
    {
        let block = content.child(0).expect("just checked");
        return Slice::new(block.content().clone(), 0, 0);
    }
    Slice::new(content, 0, 0)
}

/// Whether the source spelled out block structure. A paste of `<p>a</p>` is a
/// paragraph; a paste of `a` is text that happened to land in one.
fn html_looks_like_blocks(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    [
        "<p",
        "<div",
        "<h1",
        "<h2",
        "<h3",
        "<h4",
        "<h5",
        "<h6",
        "<ul",
        "<ol",
        "<li",
        "<blockquote",
        "<pre",
        "<table",
        "<hr",
    ]
    .iter()
    .any(|tag| lower.contains(tag))
}

/// Every element name the rules know how to read — for a settings page, or a
/// test that the table has not drifted.
#[must_use]
pub fn known_tags(rules: &Rules) -> BTreeMap<String, String> {
    rules
        .parse
        .iter()
        .map(|r| (r.tag.clone(), format!("{:?}", r.target)))
        .collect()
}
