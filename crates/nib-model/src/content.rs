// SPDX-License-Identifier: MPL-2.0

//! Content expressions: what a node is allowed to contain.
//!
//! A schema says `doc` contains `block+`, `paragraph` contains `inline*`,
//! `list` contains `list_item+`, `table_row` contains `(table_cell |
//! table_header)+`. Those strings are a regular language over node types, and
//! this module compiles them the way regular languages are compiled: parse to
//! a tree, build an NFA, subset-construct a DFA, and hand out a cursor into
//! it.
//!
//! # Why a DFA and not a validity check
//!
//! Because the interesting question is never "is this document valid?" — it is
//! *"what would make it valid?"*. Pressing Enter at the end of a list item has
//! to know what may follow; pasting a heading into a table cell has to know
//! what to wrap it in; deleting the only row of a table has to know what must
//! be inserted so the table does not become illegal. A boolean cannot answer
//! any of those. A cursor into a DFA answers all three, because at every state
//! it knows the set of types that may come next.
//!
//! So [`ContentMatch`] is not a validator. It is the thing
//! [`fill_before`](ContentMatch::fill_before) and
//! [`find_wrapping`](ContentMatch::find_wrapping) search, and those two are
//! what every structural edit in the engine ultimately calls.
//!
//! # Grammar
//!
//! ```text
//! expr        ::= seq ('|' seq)*
//! seq         ::= subscripted*
//! subscripted ::= atom ('+' | '*' | '?' | '{' num (',' num?)? '}')*
//! atom        ::= '(' expr ')' | name
//! ```
//!
//! A `name` is a node type or a group; a group expands to the choice of its
//! members. Whitespace between tokens is insignificant.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::fragment::Fragment;
use crate::node::Node;
use crate::schema::{NodeTypeId, Schema};

/// Why a content expression could not be compiled.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("unexpected end of content expression {expr:?}")]
    UnexpectedEnd { expr: String },
    #[error("unexpected {found:?} at offset {at} in content expression {expr:?}")]
    Unexpected {
        expr: String,
        found: String,
        at: usize,
    },
    #[error("no node type or group named {name:?} in content expression {expr:?}")]
    UnknownName { expr: String, name: String },
    #[error("range {{{min},{max}}} in content expression {expr:?} is inside out")]
    BadRange { expr: String, min: u32, max: i64 },
    #[error(
        "content expression {expr:?} requires content at a point where only \
         {types} may go, and none of those can be generated (text, or a node \
         with a required attribute)"
    )]
    NotGeneratable { expr: String, types: String },
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    Choice(Vec<Expr>),
    Seq(Vec<Expr>),
    Plus(Box<Expr>),
    Star(Box<Expr>),
    Opt(Box<Expr>),
    /// `max == -1` is "unbounded", spelled `{n,}`.
    Range {
        min: u32,
        max: i64,
        expr: Box<Expr>,
    },
    Name(NodeTypeId),
    /// The empty expression — what `""` compiles to, and what a leaf node's
    /// content is.
    Empty,
}

struct Stream<'a> {
    source: &'a str,
    tokens: Vec<(usize, &'a str)>,
    pos: usize,
}

impl<'a> Stream<'a> {
    fn new(source: &'a str) -> Self {
        let mut tokens = Vec::new();
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
                {
                    i += 1;
                }
                tokens.push((start, &source[start..i]));
            } else {
                tokens.push((i, &source[i..=i]));
                i += 1;
            }
        }
        Self {
            source,
            tokens,
            pos: 0,
        }
    }

    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.pos).map(|(_, t)| *t)
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        let t = self.peek();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.source.len(), |(o, _)| *o)
    }

    fn err_unexpected(&self) -> ContentError {
        match self.peek() {
            Some(found) => ContentError::Unexpected {
                expr: self.source.to_owned(),
                found: found.to_owned(),
                at: self.offset(),
            },
            None => ContentError::UnexpectedEnd {
                expr: self.source.to_owned(),
            },
        }
    }
}

/// Resolves a name to the node types it stands for: a single type, or every
/// member of a group.
fn resolve_name(
    stream: &Stream<'_>,
    name: &str,
    types: &BTreeMap<String, NodeTypeId>,
    groups: &BTreeMap<String, Vec<NodeTypeId>>,
) -> Result<Vec<NodeTypeId>, ContentError> {
    if let Some(id) = types.get(name) {
        return Ok(vec![*id]);
    }
    if let Some(ids) = groups.get(name) {
        return Ok(ids.clone());
    }
    Err(ContentError::UnknownName {
        expr: stream.source.to_owned(),
        name: name.to_owned(),
    })
}

fn parse_expr(
    stream: &mut Stream<'_>,
    types: &BTreeMap<String, NodeTypeId>,
    groups: &BTreeMap<String, Vec<NodeTypeId>>,
) -> Result<Expr, ContentError> {
    let mut exprs = vec![parse_seq(stream, types, groups)?];
    while stream.eat("|") {
        exprs.push(parse_seq(stream, types, groups)?);
    }
    Ok(if exprs.len() == 1 {
        exprs.pop().expect("just checked length")
    } else {
        Expr::Choice(exprs)
    })
}

fn parse_seq(
    stream: &mut Stream<'_>,
    types: &BTreeMap<String, NodeTypeId>,
    groups: &BTreeMap<String, Vec<NodeTypeId>>,
) -> Result<Expr, ContentError> {
    let mut exprs = Vec::new();
    loop {
        exprs.push(parse_subscript(stream, types, groups)?);
        match stream.peek() {
            None | Some(")" | "|") => break,
            Some(_) => {}
        }
    }
    Ok(if exprs.len() == 1 {
        exprs.pop().expect("just checked length")
    } else {
        Expr::Seq(exprs)
    })
}

fn parse_subscript(
    stream: &mut Stream<'_>,
    types: &BTreeMap<String, NodeTypeId>,
    groups: &BTreeMap<String, Vec<NodeTypeId>>,
) -> Result<Expr, ContentError> {
    let mut expr = parse_atom(stream, types, groups)?;
    loop {
        if stream.eat("+") {
            expr = Expr::Plus(Box::new(expr));
        } else if stream.eat("*") {
            expr = Expr::Star(Box::new(expr));
        } else if stream.eat("?") {
            expr = Expr::Opt(Box::new(expr));
        } else if stream.eat("{") {
            expr = parse_range(stream, expr)?;
        } else {
            return Ok(expr);
        }
    }
}

fn parse_num(stream: &mut Stream<'_>) -> Result<u32, ContentError> {
    let err = stream.err_unexpected();
    let token = stream.next().ok_or_else(|| err.clone())?;
    token.parse::<u32>().map_err(|_| err)
}

fn parse_range(stream: &mut Stream<'_>, expr: Expr) -> Result<Expr, ContentError> {
    let min = parse_num(stream)?;
    let max = if stream.eat(",") {
        if stream.peek() == Some("}") {
            -1
        } else {
            i64::from(parse_num(stream)?)
        }
    } else {
        i64::from(min)
    };
    if !stream.eat("}") {
        return Err(stream.err_unexpected());
    }
    if max != -1 && max < i64::from(min) {
        return Err(ContentError::BadRange {
            expr: stream.source.to_owned(),
            min,
            max,
        });
    }
    Ok(Expr::Range {
        min,
        max,
        expr: Box::new(expr),
    })
}

fn parse_atom(
    stream: &mut Stream<'_>,
    types: &BTreeMap<String, NodeTypeId>,
    groups: &BTreeMap<String, Vec<NodeTypeId>>,
) -> Result<Expr, ContentError> {
    if stream.eat("(") {
        let inner = parse_expr(stream, types, groups)?;
        if !stream.eat(")") {
            return Err(stream.err_unexpected());
        }
        return Ok(inner);
    }
    let Some(token) = stream.peek() else {
        return Err(stream.err_unexpected());
    };
    let is_name = token
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if !is_name {
        return Err(stream.err_unexpected());
    }
    stream.next();
    let ids = resolve_name(stream, token, types, groups)?;
    let mut exprs: Vec<Expr> = ids.into_iter().map(Expr::Name).collect();
    Ok(match exprs.len() {
        // A group with no members matches nothing, which is not the same as
        // matching the empty sequence: `Choice([])` has no outgoing edges.
        0 => Expr::Choice(Vec::new()),
        1 => exprs.pop().expect("just checked length"),
        _ => Expr::Choice(exprs),
    })
}

// ---------------------------------------------------------------------------
// NFA
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct NfaEdge {
    /// `None` is an epsilon edge.
    term: Option<NodeTypeId>,
    to: usize,
}

/// A handle to an edge whose destination has not been decided yet.
type Dangling = (usize, usize);

struct Nfa {
    nodes: Vec<Vec<NfaEdge>>,
}

impl Nfa {
    fn new() -> Self {
        Self {
            nodes: vec![Vec::new()],
        }
    }

    fn node(&mut self) -> usize {
        self.nodes.push(Vec::new());
        self.nodes.len() - 1
    }

    fn edge(&mut self, from: usize, term: Option<NodeTypeId>) -> Dangling {
        // `to` is a placeholder until `connect` fills it in. `usize::MAX` is
        // never a valid node index, so a forgotten `connect` panics on the
        // subsequent lookup instead of silently linking to node zero.
        self.nodes[from].push(NfaEdge {
            term,
            to: usize::MAX,
        });
        (from, self.nodes[from].len() - 1)
    }

    fn connect(&mut self, edges: &[Dangling], to: usize) {
        for &(node, index) in edges {
            self.nodes[node][index].to = to;
        }
    }

    fn compile(&mut self, expr: &Expr, from: usize) -> Vec<Dangling> {
        match expr {
            Expr::Empty => vec![self.edge(from, None)],
            Expr::Name(typ) => vec![self.edge(from, Some(*typ))],
            Expr::Choice(exprs) => {
                let mut out = Vec::new();
                for expr in exprs {
                    out.extend(self.compile(expr, from));
                }
                out
            }
            Expr::Seq(exprs) => {
                let mut from = from;
                for (i, expr) in exprs.iter().enumerate() {
                    let next = self.compile(expr, from);
                    if i == exprs.len() - 1 {
                        return next;
                    }
                    from = self.node();
                    self.connect(&next, from);
                }
                vec![self.edge(from, None)]
            }
            Expr::Star(inner) => {
                let loop_node = self.node();
                let entry = self.edge(from, None);
                self.connect(&[entry], loop_node);
                let body = self.compile(inner, loop_node);
                self.connect(&body, loop_node);
                vec![self.edge(loop_node, None)]
            }
            Expr::Plus(inner) => {
                let loop_node = self.node();
                let first = self.compile(inner, from);
                self.connect(&first, loop_node);
                let again = self.compile(inner, loop_node);
                self.connect(&again, loop_node);
                vec![self.edge(loop_node, None)]
            }
            Expr::Opt(inner) => {
                let mut out = vec![self.edge(from, None)];
                out.extend(self.compile(inner, from));
                out
            }
            Expr::Range { min, max, expr } => {
                let mut cur = from;
                for _ in 0..*min {
                    let next = self.node();
                    let body = self.compile(expr, cur);
                    self.connect(&body, next);
                    cur = next;
                }
                if *max == -1 {
                    let body = self.compile(expr, cur);
                    self.connect(&body, cur);
                } else {
                    let max = u32::try_from(*max).unwrap_or(*min);
                    for _ in *min..max {
                        let next = self.node();
                        let skip = self.edge(cur, None);
                        self.connect(&[skip], next);
                        let body = self.compile(expr, cur);
                        self.connect(&body, next);
                        cur = next;
                    }
                }
                vec![self.edge(cur, None)]
            }
        }
    }

    /// The set of states reachable from `node` by epsilon edges alone,
    /// including `node` itself when it has any non-epsilon way out.
    fn null_from(&self, node: usize) -> Vec<usize> {
        let mut result = Vec::new();
        self.scan(node, &mut result);
        result.sort_unstable();
        result
    }

    fn scan(&self, node: usize, result: &mut Vec<usize>) {
        let edges = &self.nodes[node];
        // A state whose only exit is one epsilon edge is not a state worth
        // labelling; skipping it keeps the DFA's state sets small.
        if edges.len() == 1 && edges[0].term.is_none() {
            return self.scan(edges[0].to, result);
        }
        if result.contains(&node) {
            return;
        }
        result.push(node);
        for edge in edges {
            if edge.term.is_none() && !result.contains(&edge.to) {
                self.scan(edge.to, result);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DFA
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DfaState {
    valid_end: bool,
    /// Sorted by node type id, so [`ContentMatch::edge`] has a stable order
    /// and `default_type` is deterministic.
    next: Vec<(NodeTypeId, usize)>,
}

/// A compiled content expression.
///
/// Shared by every [`ContentMatch`] cursor into it, so a cursor is two words
/// and cloning one is an `Arc` bump.
#[derive(Debug)]
pub struct ContentExpr {
    source: String,
    states: Vec<DfaState>,
}

impl ContentExpr {
    /// Compiles `source` against the node types and groups of a schema under
    /// construction.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError`] if the expression does not parse, names
    /// something the schema does not have, or admits a loop that consumes no
    /// input.
    pub fn compile(
        source: &str,
        types: &BTreeMap<String, NodeTypeId>,
        groups: &BTreeMap<String, Vec<NodeTypeId>>,
    ) -> Result<Arc<Self>, ContentError> {
        let expr = if source.trim().is_empty() {
            Expr::Empty
        } else {
            let mut stream = Stream::new(source);
            let parsed = parse_expr(&mut stream, types, groups)?;
            if stream.peek().is_some() {
                return Err(stream.err_unexpected());
            }
            parsed
        };

        let mut nfa = Nfa::new();
        let dangling = nfa.compile(&expr, 0);
        let accept = nfa.node();
        nfa.connect(&dangling, accept);
        let states = subset_construct(&nfa, accept);
        Ok(Arc::new(Self {
            source: source.to_owned(),
            states,
        }))
    }

    /// The empty expression: matches nothing, ends immediately. What a leaf
    /// node — an image, a horizontal rule — contains.
    #[must_use]
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            source: String::new(),
            states: vec![DfaState {
                valid_end: true,
                next: Vec::new(),
            }],
        })
    }

    /// The expression as written in the schema.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every position where content is required, and the types that may fill
    /// it. See [`required_positions`].
    pub fn required_positions(&self) -> impl Iterator<Item = Vec<NodeTypeId>> + '_ {
        required_positions(&self.states)
    }
}

impl fmt::Display for ContentExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

/// Every state at which content is *required* — the expression may not end
/// there — paired with the node types that may fill it.
///
/// The schema checks these once every node type exists: a required position
/// that only text or attribute-requiring nodes can fill is a schema that can
/// declare a document but never create one, and it is far better to say so at
/// build time than to have `create_and_fill` quietly return `None` forever.
fn required_positions(states: &[DfaState]) -> impl Iterator<Item = Vec<NodeTypeId>> + '_ {
    states
        .iter()
        .filter(|s| !s.valid_end)
        .map(|s| s.next.iter().map(|(t, _)| *t).collect())
}

/// Subset construction: each DFA state is a set of NFA states.
fn subset_construct(nfa: &Nfa, accept: usize) -> Vec<DfaState> {
    let mut labeled: BTreeMap<Vec<usize>, usize> = BTreeMap::new();
    let mut states: Vec<DfaState> = Vec::new();
    let mut pending: Vec<(usize, Vec<usize>)> = Vec::new();

    let start = nfa.null_from(0);
    labeled.insert(start.clone(), 0);
    states.push(DfaState {
        valid_end: start.contains(&accept),
        next: Vec::new(),
    });
    pending.push((0, start));

    while let Some((index, set)) = pending.pop() {
        // Gather, per term, the union of null-closures of its destinations.
        let mut out: Vec<(NodeTypeId, Vec<usize>)> = Vec::new();
        for &node in &set {
            for edge in &nfa.nodes[node] {
                let Some(term) = edge.term else { continue };
                let slot = if let Some((_, slot)) = out.iter_mut().find(|(t, _)| *t == term) {
                    slot
                } else {
                    out.push((term, Vec::new()));
                    &mut out.last_mut().expect("just pushed").1
                };
                for reached in nfa.null_from(edge.to) {
                    if !slot.contains(&reached) {
                        slot.push(reached);
                    }
                }
            }
        }

        let mut next = Vec::with_capacity(out.len());
        for (term, mut set) in out {
            set.sort_unstable();
            let target = if let Some(&existing) = labeled.get(&set) {
                existing
            } else {
                let target = states.len();
                states.push(DfaState {
                    valid_end: set.contains(&accept),
                    next: Vec::new(),
                });
                labeled.insert(set.clone(), target);
                pending.push((target, set));
                target
            };
            next.push((term, target));
        }
        next.sort_unstable_by_key(|(term, _)| *term);
        states[index].next = next;
    }

    states
}

// ---------------------------------------------------------------------------
// The cursor
// ---------------------------------------------------------------------------

/// A position inside a compiled content expression.
///
/// Cheap to clone and to carry: an `Arc` and an index. Every structural
/// decision in the engine — what may be inserted here, what must be inserted
/// to make this legal, what this must be wrapped in — is a question asked of
/// one of these.
#[derive(Debug, Clone)]
pub struct ContentMatch {
    expr: Arc<ContentExpr>,
    state: usize,
}

impl PartialEq for ContentMatch {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.expr, &other.expr) && self.state == other.state
    }
}

impl Eq for ContentMatch {}

impl ContentMatch {
    /// The cursor at the start of `expr`.
    #[must_use]
    pub fn start(expr: Arc<ContentExpr>) -> Self {
        Self { expr, state: 0 }
    }

    /// True when the content may legally end here.
    #[must_use]
    pub fn valid_end(&self) -> bool {
        self.expr.states[self.state].valid_end
    }

    /// How many distinct node types may come next.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.expr.states[self.state].next.len()
    }

    /// The `n`th type that may come next, and where it leads.
    #[must_use]
    pub fn edge(&self, n: usize) -> Option<(NodeTypeId, Self)> {
        let (typ, next) = *self.expr.states[self.state].next.get(n)?;
        Some((
            typ,
            Self {
                expr: Arc::clone(&self.expr),
                state: next,
            },
        ))
    }

    /// Every type that may come next.
    pub fn edges(&self) -> impl Iterator<Item = (NodeTypeId, Self)> + '_ {
        self.expr.states[self.state]
            .next
            .iter()
            .map(|&(typ, next)| {
                (
                    typ,
                    Self {
                        expr: Arc::clone(&self.expr),
                        state: next,
                    },
                )
            })
    }

    /// Advances past one node of type `typ`, or `None` if it is not allowed
    /// here.
    #[must_use]
    pub fn match_type(&self, typ: NodeTypeId) -> Option<Self> {
        self.expr.states[self.state]
            .next
            .binary_search_by_key(&typ, |(t, _)| *t)
            .ok()
            .map(|i| {
                let (_, next) = self.expr.states[self.state].next[i];
                Self {
                    expr: Arc::clone(&self.expr),
                    state: next,
                }
            })
    }

    /// Advances past `fragment[start..end]`, or `None` at the first child that
    /// is not allowed.
    #[must_use]
    pub fn match_fragment_range(
        &self,
        fragment: &Fragment,
        start: usize,
        end: usize,
    ) -> Option<Self> {
        let mut cur = self.clone();
        for i in start..end {
            cur = cur.match_type(fragment.child(i)?.type_id())?;
        }
        Some(cur)
    }

    /// Advances past the whole fragment.
    #[must_use]
    pub fn match_fragment(&self, fragment: &Fragment) -> Option<Self> {
        self.match_fragment_range(fragment, 0, fragment.child_count())
    }

    /// The type to insert here when the user has expressed no preference —
    /// pressing Enter, or filling a gap. The first allowed type that is not
    /// text and does not need attributes nobody has supplied.
    #[must_use]
    pub fn default_type(&self, schema: &Schema) -> Option<NodeTypeId> {
        self.expr.states[self.state]
            .next
            .iter()
            .map(|(typ, _)| *typ)
            .find(|typ| {
                let t = schema.node_type(*typ);
                !t.is_text() && !t.has_required_attrs()
            })
    }

    /// True when everything this match allows, `other` allows too — used to
    /// decide whether two nodes' contents may be joined.
    #[must_use]
    pub fn compatible(&self, other: &Self) -> bool {
        self.expr.states[self.state].next.iter().any(|(a, _)| {
            other.expr.states[other.state]
                .next
                .iter()
                .any(|(b, _)| a == b)
        })
    }

    /// The nodes that must be inserted at this point for `after` to be legal
    /// from `start_index` on.
    ///
    /// `Some(empty)` means nothing is needed; `None` means no insertion can
    /// rescue it. With `to_end`, the content must also be allowed to *end*
    /// after `after` — which is the difference between "may I put this here"
    /// and "may I close the node after putting this here".
    #[must_use]
    pub fn fill_before(
        &self,
        schema: &Schema,
        after: &Fragment,
        to_end: bool,
        start_index: usize,
    ) -> Option<Fragment> {
        let mut seen = vec![self.state];
        self.search_fill(schema, after, to_end, start_index, &mut seen)
    }

    fn search_fill(
        &self,
        schema: &Schema,
        after: &Fragment,
        to_end: bool,
        start_index: usize,
        seen: &mut Vec<usize>,
    ) -> Option<Fragment> {
        if let Some(finished) = self.match_fragment_range(after, start_index, after.child_count())
            && (!to_end || finished.valid_end())
        {
            return Some(Fragment::empty());
        }
        for (typ, next) in self.edges() {
            let t = schema.node_type(typ);
            if t.is_text() || t.has_required_attrs() || seen.contains(&next.state) {
                continue;
            }
            seen.push(next.state);
            if let Some(inner) = next.search_fill(schema, after, to_end, start_index, seen) {
                let node = schema.create_and_fill(
                    typ,
                    None,
                    Fragment::empty(),
                    crate::mark::Marks::none(),
                )?;
                return Some(Fragment::from(node).append(&inner));
            }
        }
        None
    }

    /// The chain of node types `target` must be wrapped in to be legal here,
    /// outermost first. `Some(vec![])` means it is already legal.
    #[must_use]
    pub fn find_wrapping(&self, schema: &Schema, target: NodeTypeId) -> Option<Vec<NodeTypeId>> {
        struct Step {
            state: ContentMatch,
            typ: Option<NodeTypeId>,
            via: Option<usize>,
        }
        let mut seen: Vec<NodeTypeId> = Vec::new();
        let mut active: Vec<Step> = vec![Step {
            state: self.clone(),
            typ: None,
            via: None,
        }];
        let mut head = 0;
        while head < active.len() {
            let current = &active[head];
            if current.state.match_type(target).is_some() {
                let mut result = Vec::new();
                let mut cursor = Some(head);
                while let Some(i) = cursor {
                    let Some(typ) = active[i].typ else { break };
                    result.push(typ);
                    cursor = active[i].via;
                }
                result.reverse();
                return Some(result);
            }
            let is_root = current.typ.is_none();
            let candidates: Vec<(NodeTypeId, ContentMatch)> = current.state.edges().collect();
            for (typ, next) in candidates {
                let t = schema.node_type(typ);
                if t.is_leaf() || t.has_required_attrs() || seen.contains(&typ) {
                    continue;
                }
                // Below the first level, wrapping is only allowed in a type
                // that may legally *end* where we entered it — otherwise the
                // wrapper itself would need filling, and that is a different
                // question than the one being asked.
                if !is_root && !next.valid_end() {
                    continue;
                }
                seen.push(typ);
                active.push(Step {
                    state: t.content_match(),
                    typ: Some(typ),
                    via: Some(head),
                });
            }
            head += 1;
        }
        None
    }
}

/// The nodes needed to make `fragment` a legal child list, or `None`.
///
/// A convenience over [`ContentMatch::fill_before`] for the common "can this
/// whole fragment go in a node of this type" question.
#[must_use]
pub fn fits(schema: &Schema, typ: NodeTypeId, fragment: &Fragment) -> bool {
    schema
        .node_type(typ)
        .content_match()
        .match_fragment(fragment)
        .is_some_and(|m| m.valid_end())
}

/// A single node, wrapped in `wrapping` outermost-first.
///
/// # Panics
///
/// Never: wrapping a node always leaves exactly one node.
///
/// # Errors
///
/// If a wrapping type has a required attribute, which
/// [`ContentMatch::find_wrapping`] never proposes but a hand-written wrapping
/// list may name.
pub fn wrap(
    schema: &Schema,
    node: Node,
    wrapping: &[NodeTypeId],
) -> Result<Node, crate::schema::SchemaError> {
    let mut current = Fragment::from(node);
    for &typ in wrapping.iter().rev() {
        current = Fragment::from(schema.create(typ, None, current, crate::mark::Marks::none())?);
    }
    Ok(current
        .child(0)
        .cloned()
        .expect("wrapping always leaves exactly one node"))
}
