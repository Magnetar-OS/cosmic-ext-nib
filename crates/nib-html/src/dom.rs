// SPDX-License-Identifier: MPL-2.0

//! The tree html5ever builds, holding what the parse reads and nothing else.
//!
//! html5ever decides what the bytes mean; it does not keep the result. It
//! calls a [`TreeSink`] — make an element, put this under that, move those
//! children over there — and whatever implements the sink is the tree. This
//! is that implementation.
//!
//! # An arena, not pointers
//!
//! Every node lives in one `Vec` and is named by its index. Three things
//! follow, and each is a reason on its own:
//!
//! - Dropping the tree is dropping a `Vec`. A tree of reference-counted nodes
//!   is dropped by recursion unless something stops it, and markup nested
//!   fifty thousand deep is a quarter of a megabyte of mail.
//! - What the parser holds a node by is a number and a name, so its walks up
//!   its stack of open elements, which it makes for every block-level tag,
//!   read that stack alone rather than chasing a pointer per element and
//!   counting its references up and down. See [`Handle`].
//! - The walk in [`parse`](crate::parse) borrows the finished tree plainly,
//!   with no cells to open at each step.
//!
//! # What is kept
//!
//! Elements with their names and attributes, and text. Comments, doctypes and
//! processing instructions keep their *place* — text either side of a comment
//! is two runs to the parser, and stays two here — but not their contents,
//! which nothing reads. A `<template>`'s contents go to a fragment of their
//! own, as the specification says, and so are not among its children.
//!
//! One thing the specification asks of a tree is left undone: copying a
//! selected `<option>`'s content into its `<select>`'s `<selectedcontent>`.
//! That element repeats, for display, what the option already says; in a
//! document it would be the same text twice.
//!
//! # Never panics
//!
//! The sink is driven by a parser fed arbitrary input, so no call may panic
//! whatever the parser asks: a node moved while it still has a parent is
//! detached first, and a request the tree cannot honour — insert before a
//! node that is nowhere — is ignored rather than asserted.

use std::borrow::Cow;
use std::cell::RefCell;

use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, ExpandedName, LocalName, Namespace, QualName};

/// A node's place in the arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Id(usize);

/// What a node is.
#[derive(Debug)]
pub(crate) enum Data {
    /// The document, or the fragment a `<template>`'s contents live in.
    Root,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        /// The fragment holding this element's contents, for a `<template>`.
        template: Option<Id>,
        /// Whether this is a `MathML` `annotation-xml` element that takes
        /// HTML content, which the parser asks about and cannot work out
        /// again from the name.
        integration_point: bool,
    },
    Text(StrTendril),
    /// A comment, doctype or processing instruction: a place among its
    /// siblings and nothing more.
    Skipped,
}

#[derive(Debug)]
struct Node {
    parent: Option<Id>,
    children: Vec<Id>,
    data: Data,
}

/// A parsed tree.
#[derive(Debug)]
pub(crate) struct Dom {
    nodes: Vec<Node>,
}

impl Default for Dom {
    /// An empty document.
    fn default() -> Self {
        Self {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                data: Data::Root,
            }],
        }
    }
}

impl Dom {
    /// The document node, which every tree has.
    pub(crate) const DOCUMENT: Id = Id(0);

    pub(crate) fn data(&self, id: Id) -> &Data {
        &self.nodes[id.0].data
    }

    pub(crate) fn children(&self, id: Id) -> &[Id] {
        &self.nodes[id.0].children
    }

    fn create(&mut self, data: Data) -> Id {
        let id = Id(self.nodes.len());
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data,
        });
        id
    }

    /// Where a node sits among its parent's children, if it has a parent.
    fn position(&self, id: Id) -> Option<(Id, usize)> {
        let parent = self.nodes[id.0].parent?;
        let index = self.nodes[parent.0]
            .children
            .iter()
            .position(|child| *child == id)?;
        Some((parent, index))
    }

    fn detach(&mut self, id: Id) {
        if let Some((parent, index)) = self.position(id) {
            self.nodes[parent.0].children.remove(index);
        }
        self.nodes[id.0].parent = None;
    }

    /// Puts a node, or text, among `parent`'s children at `index`.
    ///
    /// Text arriving next to text joins it: the parser hands characters over
    /// in whatever pieces its input came in, and one run in the source is one
    /// text node in the tree.
    fn insert(&mut self, parent: Id, index: usize, child: NodeOrText<Id>) {
        let node = match child {
            NodeOrText::AppendText(text) => {
                let before = index
                    .checked_sub(1)
                    .and_then(|before| self.nodes[parent.0].children.get(before).copied());
                if let Some(before) = before
                    && let Data::Text(existing) = &mut self.nodes[before.0].data
                {
                    existing.push_slice(&text);
                    return;
                }
                self.create(Data::Text(text))
            }
            NodeOrText::AppendNode(node) => node,
        };
        self.nodes[node.0].parent = Some(parent);
        self.nodes[parent.0].children.insert(index, node);
    }

    fn append(&mut self, parent: Id, child: NodeOrText<Id>) {
        if let NodeOrText::AppendNode(node) = &child {
            self.detach(*node);
        }
        let end = self.nodes[parent.0].children.len();
        self.insert(parent, end, child);
    }

    fn append_before(&mut self, sibling: Id, child: NodeOrText<Id>) {
        if let NodeOrText::AppendNode(node) = &child {
            self.detach(*node);
        }
        // Looked up after the detach, which moves the sibling down one when
        // the node came from earlier under the same parent.
        if let Some((parent, index)) = self.position(sibling) {
            self.insert(parent, index, child);
        }
    }
}

/// What the parser holds a node by: its place in the arena, and its name.
///
/// The name rides along because of how often the parser asks for it. Every
/// block-level start tag makes it walk its stack of open elements looking for
/// a `<p>` to close, asking each element its name on the way, so markup
/// nested fifty thousand deep asks over a billion times. Answered from the
/// handle, that walk reads the parser's own stack and nothing else: no cell
/// to open, no node to fetch. An element's name never changes, so the copy
/// cannot go stale; a node that is not an element carries the empty name,
/// which matches nothing the parser looks for.
///
/// Names html5ever knows are interned statically, so cloning a handle copies
/// three words and counts nothing.
#[derive(Debug, Clone)]
pub(crate) struct Handle {
    id: Id,
    ns: Namespace,
    local: LocalName,
}

impl Handle {
    fn unnamed(id: Id) -> Self {
        Self {
            id,
            ns: Namespace::default(),
            local: LocalName::default(),
        }
    }
}

/// A node or text to insert, by arena index rather than by handle.
fn by_id(child: NodeOrText<Handle>) -> NodeOrText<Id> {
    match child {
        NodeOrText::AppendNode(handle) => NodeOrText::AppendNode(handle.id),
        NodeOrText::AppendText(text) => NodeOrText::AppendText(text),
    }
}

/// The [`TreeSink`] html5ever builds a [`Dom`] through.
///
/// The trait takes `&self` throughout, so the tree sits in a cell; each call
/// opens it, does its one thing and closes it, and none calls another with
/// it open.
#[derive(Debug, Default)]
pub(crate) struct Sink {
    dom: RefCell<Dom>,
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Dom;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Dom {
        self.dom.into_inner()
    }

    /// Recoveries are the parser's business; what they produced is the tree.
    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        Handle::unnamed(Dom::DOCUMENT)
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        ExpandedName {
            ns: &target.ns,
            local: &target.local,
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let mut dom = self.dom.borrow_mut();
        let template = flags.template.then(|| dom.create(Data::Root));
        let (ns, local) = (name.ns.clone(), name.local.clone());
        let id = dom.create(Data::Element {
            name,
            attrs,
            template,
            integration_point: flags.mathml_annotation_xml_integration_point,
        });
        Handle { id, ns, local }
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        Handle::unnamed(self.dom.borrow_mut().create(Data::Skipped))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        Handle::unnamed(self.dom.borrow_mut().create(Data::Skipped))
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.dom.borrow_mut().append(parent.id, by_id(child));
    }

    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        self.dom
            .borrow_mut()
            .append_before(sibling.id, by_id(child));
    }

    /// Foster parenting: content that may not sit in a table goes before the
    /// table when the table is in the tree, and otherwise into the element
    /// that was open before it.
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let mut dom = self.dom.borrow_mut();
        if dom.nodes[element.id.0].parent.is_some() {
            dom.append_before(element.id, by_id(child));
        } else {
            dom.append(prev_element.id, by_id(child));
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let mut dom = self.dom.borrow_mut();
        let doctype = dom.create(Data::Skipped);
        dom.append(Dom::DOCUMENT, NodeOrText::AppendNode(doctype));
    }

    /// A `<template>`'s fragment. Asked of any other node — which the parser
    /// does not do — the answer is a fresh fragment attached to nothing, so
    /// what is put there is simply not in the document.
    fn get_template_contents(&self, target: &Handle) -> Handle {
        let mut dom = self.dom.borrow_mut();
        if let Data::Element {
            template: Some(contents),
            ..
        } = dom.nodes[target.id.0].data
        {
            return Handle::unnamed(contents);
        }
        Handle::unnamed(dom.create(Data::Root))
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }

    /// Quirks mode changes how the parser builds the tree, which it tracks
    /// itself; nothing read from the tree afterwards depends on it.
    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut dom = self.dom.borrow_mut();
        let Data::Element {
            attrs: existing, ..
        } = &mut dom.nodes[target.id.0].data
        else {
            return;
        };
        for attr in attrs {
            if !existing.iter().any(|have| have.name == attr.name) {
                existing.push(attr);
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        self.dom.borrow_mut().detach(target.id);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        if node.id == new_parent.id {
            return;
        }
        let mut dom = self.dom.borrow_mut();
        let children = std::mem::take(&mut dom.nodes[node.id.0].children);
        for child in &children {
            dom.nodes[child.0].parent = Some(new_parent.id);
        }
        dom.nodes[new_parent.id.0].children.extend(children);
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &Handle) -> bool {
        matches!(
            self.dom.borrow().nodes[target.id.0].data,
            Data::Element {
                integration_point: true,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use html5ever::interface::create_element;
    use html5ever::tendril::TendrilSink;
    use html5ever::{local_name, ns};

    use super::*;

    fn build(html: &str) -> Dom {
        html5ever::parse_document(Sink::default(), html5ever::ParseOpts::default()).one(html)
    }

    /// The tree in the notation the HTML parsing tests use: one node a line,
    /// two spaces a level, attributes under their element, a `<template>`'s
    /// contents under `content`. A comment or doctype prints as `<!>`, which
    /// is all of it this tree keeps.
    fn tree(html: &str) -> String {
        fn write(dom: &Dom, id: Id, depth: usize, out: &mut String) {
            let indent = " ".repeat(depth * 2);
            for child in dom.children(id) {
                match dom.data(*child) {
                    Data::Element {
                        name,
                        attrs,
                        template,
                        ..
                    } => {
                        let prefix = if name.ns == ns!(html) {
                            ""
                        } else if name.ns == ns!(svg) {
                            "svg "
                        } else {
                            "math "
                        };
                        let _ = writeln!(out, "| {indent}<{prefix}{}>", name.local);
                        let mut attrs: Vec<_> = attrs
                            .iter()
                            .map(|attr| format!("{}=\"{}\"", attr.name.local, attr.value))
                            .collect();
                        attrs.sort();
                        for attr in attrs {
                            let _ = writeln!(out, "| {indent}  {attr}");
                        }
                        if let Some(contents) = template {
                            let _ = writeln!(out, "| {indent}  content");
                            write(dom, *contents, depth + 2, out);
                        }
                        write(dom, *child, depth + 1, out);
                    }
                    Data::Text(text) => {
                        let _ = writeln!(out, "| {indent}\"{text}\"");
                    }
                    Data::Skipped => {
                        let _ = writeln!(out, "| {indent}<!>");
                    }
                    Data::Root => {
                        let _ = writeln!(out, "| {indent}#root");
                    }
                }
            }
        }
        let mut out = String::new();
        write(&build(html), Dom::DOCUMENT, 0, &mut out);
        out
    }

    /// Strips the margin a test's expected tree is indented by.
    fn expected(tree: &str) -> String {
        tree.lines()
            .map(str::trim_start)
            .filter(|line| !line.is_empty())
            .fold(String::new(), |mut out, line| {
                out.push_str(line);
                out.push('\n');
                out
            })
    }

    #[test]
    fn an_unclosed_paragraph_ends_where_the_next_begins() {
        assert_eq!(
            tree("<p>One<p>Two"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <p>
                |       "One"
                |     <p>
                |       "Two"
                "#
            )
        );
    }

    /// The two cases the specification itself walks through under
    /// "misnested tags", and two more from its conformance tests where the
    /// formatting element has to be cloned into each block it straddles.
    #[test]
    fn misnested_formatting_is_untangled_by_the_adoption_agency() {
        assert_eq!(
            tree("<p>1<b>2<i>3</b>4</i>5</p>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <p>
                |       "1"
                |       <b>
                |         "2"
                |         <i>
                |           "3"
                |       <i>
                |         "4"
                |       "5"
                "#
            )
        );
        assert_eq!(
            tree("<b>1<p>2</b>3</p>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <b>
                |       "1"
                |     <p>
                |       <b>
                |         "2"
                |       "3"
                "#
            )
        );
        assert_eq!(
            tree("<a>1<div>2<div>3</a>4</div>5</div>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <a>
                |       "1"
                |     <div>
                |       <a>
                |         "2"
                |       <div>
                |         <a>
                |           "3"
                |         "4"
                |       "5"
                "#
            )
        );
        assert_eq!(
            tree("<a>1<button>2</a>3</button>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <a>
                |       "1"
                |     <button>
                |       <a>
                |         "2"
                |       "3"
                "#
            )
        );
    }

    #[test]
    fn a_link_opened_inside_a_link_closes_the_first() {
        assert_eq!(
            tree("<a><p>X<a>Y</a>Z</p></a>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <a>
                |     <p>
                |       <a>
                |         "X"
                |       <a>
                |         "Y"
                |       "Z"
                "#
            )
        );
    }

    #[test]
    fn formatting_left_open_is_reopened_in_the_next_block() {
        assert_eq!(
            tree("<p><b><i><u></p> <p>X"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <p>
                |       <b>
                |         <i>
                |           <u>
                |     <b>
                |       <i>
                |         <u>
                |           " "
                |           <p>
                |             "X"
                "#
            )
        );
    }

    /// The specification's own example of "unexpected markup in tables":
    /// the `<b>` and the stray text leave the table, in source order, and the
    /// table gets the `<tbody>` nobody wrote.
    #[test]
    fn content_that_may_not_sit_in_a_table_is_put_before_it() {
        assert_eq!(
            tree("<table><b><tr><td>aaa</td></tr>bbb</table>ccc"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <b>
                |     <b>
                |       "bbb"
                |     <table>
                |       <tbody>
                |         <tr>
                |           <td>
                |             "aaa"
                |     <b>
                |       "ccc"
                "#
            )
        );
    }

    #[test]
    fn text_moved_out_of_a_table_joins_the_text_already_there() {
        // Two runs, each put before the table separately; one text node.
        assert_eq!(
            tree("<table>a<tr>b<td>c</table>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     "ab"
                |     <table>
                |       <tbody>
                |         <tr>
                |           <td>
                |             "c"
                "#
            )
        );
    }

    #[test]
    fn a_templates_contents_are_a_fragment_of_their_own() {
        assert_eq!(
            tree("<template>Hello</template>"),
            expected(
                r#"
                | <html>
                |   <head>
                |     <template>
                |       content
                |         "Hello"
                |   <body>
                "#
            )
        );
        // A template is allowed in a table; a `<div>` is not, and leaves it.
        assert_eq!(
            tree("<table><template></template><div></div>"),
            expected(
                r"
                | <html>
                |   <head>
                |   <body>
                |     <div>
                |     <table>
                |       <template>
                |         content
                "
            )
        );
    }

    #[test]
    fn a_repeated_html_or_body_tag_adds_only_the_attributes_that_are_missing() {
        assert_eq!(
            tree("<html a=b><body c=d><html e=f a=x><body g=h c=y>"),
            expected(
                r#"
                | <html>
                |   a="b"
                |   e="f"
                |   <head>
                |   <body>
                |     c="d"
                |     g="h"
                "#
            )
        );
    }

    #[test]
    fn a_comment_keeps_its_place_and_the_text_either_side_stays_apart() {
        assert_eq!(
            tree("<!doctype html>a<!-- x -->b"),
            expected(
                r#"
                | <!>
                | <html>
                |   <head>
                |   <body>
                |     "a"
                |     <!>
                |     "b"
                "#
            )
        );
    }

    /// The parser asks the sink whether an `annotation-xml` takes HTML, an
    /// answer that depends on an attribute it has already handed over.
    #[test]
    fn html_stays_inside_the_mathml_element_that_is_declared_to_hold_it() {
        assert_eq!(
            tree(r#"<math><annotation-xml encoding="text/html"><p>x</p></annotation-xml></math>"#),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <math math>
                |       <math annotation-xml>
                |         encoding="text/html"
                |         <p>
                |           "x"
                "#
            )
        );
        assert_eq!(
            tree("<math><annotation-xml><p>x</p></annotation-xml></math>"),
            expected(
                r#"
                | <html>
                |   <head>
                |   <body>
                |     <math math>
                |       <math annotation-xml>
                |     <p>
                |       "x"
                "#
            )
        );
    }

    #[test]
    fn a_tree_nested_deeper_than_any_stack_is_built_and_dropped() {
        // Inline elements, so the parser's own work stays linear and this
        // measures the tree alone: built, walked and dropped on a thread
        // with a stack a recursive drop would run off.
        const DEPTH: usize = 50_000;
        let depth = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let dom = build(&"<span>".repeat(DEPTH));
                let mut depth = 0;
                let mut at = Dom::DOCUMENT;
                while let Some(child) = dom.children(at).last() {
                    at = *child;
                    depth += 1;
                }
                depth
            })
            .expect("a thread")
            .join()
            .expect("neither the build nor the drop overflowed");
        // `<html>`, `<body>`, then the spans.
        assert_eq!(depth, DEPTH + 2);
    }

    fn element(sink: &Sink, name: LocalName) -> Handle {
        create_element(sink, QualName::new(None, ns!(html), name), Vec::new())
    }

    fn node(handle: &Handle) -> NodeOrText<Handle> {
        NodeOrText::AppendNode(handle.clone())
    }

    /// The parser detaches a node before it moves it, and asks for a
    /// template's contents only of a template. The sink is fed by a parser
    /// fed anything at all, so it holds up when those promises are not kept.
    #[test]
    fn calls_the_parser_does_not_make_are_absorbed_not_asserted() {
        let sink = Sink::default();
        let document = sink.get_document();
        let (first, second, third) = (
            element(&sink, local_name!("div")),
            element(&sink, local_name!("p")),
            element(&sink, local_name!("span")),
        );
        sink.append(&document, node(&first));
        sink.append(&document, node(&second));

        // Appending a node that is already somewhere moves it.
        sink.append(&first, node(&third));
        sink.append(&second, node(&third));

        // Moving a node before a later sibling under the same parent keeps
        // it before that sibling.
        sink.append_before_sibling(&second, node(&first));

        // Inserting before a node that is nowhere does nothing.
        let nowhere = element(&sink, local_name!("em"));
        sink.append_before_sibling(&nowhere, node(&element(&sink, local_name!("i"))));

        // A node cannot be emptied into itself.
        sink.reparent_children(&second, &second);

        // Only an element has a name, and only a template has contents.
        assert_eq!(*sink.elem_name(&document).local, LocalName::default());
        let contents = sink.get_template_contents(&first);
        sink.append(&contents, NodeOrText::AppendText("lost".into()));

        let dom = sink.finish();
        assert_eq!(dom.children(Dom::DOCUMENT), [first.id, second.id]);
        assert_eq!(dom.children(first.id), []);
        assert_eq!(dom.children(second.id), [third.id]);
    }
}
