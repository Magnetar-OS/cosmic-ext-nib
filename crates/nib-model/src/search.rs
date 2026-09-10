// SPDX-License-Identifier: MPL-2.0

//! Finding text, and replacing it.
//!
//! # Why this is in the model
//!
//! Because a match is a range of *document positions*, not of characters in a
//! rendering. A search that worked over the flattened text would find things
//! the user cannot see the boundaries of — a phrase that spans a block
//! boundary, or one that only exists because two paragraphs were run together
//! to make a string. Searching per textblock gives ranges that map back
//! exactly, and those ranges are what a selection, a decoration and a
//! replacement all need.
//!
//! # Case, whole words, and regular expressions
//!
//! All three are the same question — how a candidate is compared — so they are
//! one enum rather than three booleans, and adding a fourth kind of matching
//! does not change any call site.

use regex::{Regex, RegexBuilder};

use crate::decoration::{Decoration, DecorationSet, Style};
use crate::fragment::Fragment;
use crate::mark::Marks;
use crate::node::Node;
use crate::slice::Slice;
use crate::state::{EditorState, Selection, Transaction};

/// The class a search hit is decorated with.
pub const HIT: &str = "search-hit";

/// The class the current hit is decorated with, so it can be picked out from
/// the others.
pub const CURRENT: &str = "search-current";

/// How a query is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Matching {
    /// Any occurrence, ignoring case. What a reader means by "find".
    #[default]
    Loose,
    /// Exactly, including case.
    CaseSensitive,
    /// Whole words only, ignoring case.
    WholeWord,
    /// The query is a regular expression.
    Regex,
}

/// What to look for, and how.
#[derive(Debug, Clone)]
pub struct Query {
    pattern: Regex,
    /// The text as the user typed it, for a UI that wants to show it back.
    source: String,
}

impl Query {
    /// Compiles a query.
    ///
    /// # Errors
    ///
    /// Returns the regular-expression error when [`Matching::Regex`] is used
    /// and the pattern does not compile. The other kinds escape their input
    /// and cannot fail.
    pub fn new(text: &str, matching: Matching) -> Result<Self, regex::Error> {
        let pattern = match matching {
            Matching::Regex => RegexBuilder::new(text).build()?,
            Matching::CaseSensitive => RegexBuilder::new(&regex::escape(text)).build()?,
            Matching::WholeWord => RegexBuilder::new(&format!(r"\b{}\b", regex::escape(text)))
                .case_insensitive(true)
                .build()?,
            Matching::Loose => RegexBuilder::new(&regex::escape(text))
                .case_insensitive(true)
                .build()?,
        };
        Ok(Self {
            pattern,
            source: text.to_owned(),
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// One match: a range of document positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub from: usize,
    pub to: usize,
}

/// Every match in a document, in order.
///
/// Searched per textblock, so a hit never spans a boundary the reader cannot
/// see. An empty query finds nothing rather than everything.
#[must_use]
pub fn find(doc: &Node, query: &Query) -> Vec<Hit> {
    if query.source.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    doc.descendants(&mut |node, pos, _, _| {
        if !node.is_textblock() {
            return true;
        }
        let text = node.text_content();
        // The block's content starts one position past the node itself, and a
        // textblock's text offsets are its content offsets.
        let start = pos + 1;
        for found in query.pattern.find_iter(&text) {
            if found.is_empty() {
                continue;
            }
            hits.push(Hit {
                from: start + found.start(),
                to: start + found.end(),
            });
        }
        false
    });
    hits.sort_unstable();
    hits
}

/// The hit at or after `pos`, wrapping to the first.
#[must_use]
pub fn next_from(hits: &[Hit], pos: usize) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    Some(hits.iter().position(|h| h.from >= pos).unwrap_or(0))
}

/// The hit before `pos`, wrapping to the last.
#[must_use]
pub fn previous_from(hits: &[Hit], pos: usize) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    Some(
        hits.iter()
            .rposition(|h| h.to < pos)
            .unwrap_or(hits.len() - 1),
    )
}

/// The transaction that selects a hit.
#[must_use]
pub fn select(state: &EditorState, hit: Hit) -> Transaction {
    let mut tr = state.tr();
    tr.set_selection(Selection::text(hit.from, hit.to));
    tr.clone().scroll_into_view()
}

/// The transaction that replaces one hit, keeping the marks that were on it.
///
/// Returns `None` when the replacement does not apply.
#[must_use]
pub fn replace(state: &EditorState, hit: Hit, replacement: &str) -> Option<Transaction> {
    let marks = state.doc().resolve(hit.from).marks();
    let mut tr = state.tr().now();
    let slice = if replacement.is_empty() {
        Slice::empty()
    } else {
        Slice::new(
            Fragment::from(state.schema().text(replacement, marks)),
            0,
            0,
        )
    };
    tr.replace(hit.from, hit.to, slice).ok()?;
    let end = hit.from + replacement.len();
    tr.set_selection(Selection::text(hit.from, end));
    Some(tr.clone().scroll_into_view())
}

/// The transaction that replaces every hit, as one undoable change.
///
/// Applied back to front, so each replacement's positions are still valid when
/// it happens — mapping them forward would work too and would cost a mapping
/// per hit for no gain.
#[must_use]
pub fn replace_all(state: &EditorState, query: &Query, replacement: &str) -> Option<Transaction> {
    let hits = find(state.doc(), query);
    if hits.is_empty() {
        return None;
    }
    let mut tr = state.tr().now();
    for hit in hits.iter().rev() {
        let marks = tr.doc().resolve(hit.from).marks();
        let slice = if replacement.is_empty() {
            Slice::empty()
        } else {
            Slice::new(
                Fragment::from(state.schema().text(replacement, marks)),
                0,
                0,
            )
        };
        let _ = tr.replace(hit.from, hit.to, slice);
    }
    tr.doc_changed().then(|| tr.clone().scroll_into_view())
}

/// Decorations marking every hit, with `current` picked out.
#[must_use]
pub fn decorations(hits: &[Hit], current: Option<usize>) -> DecorationSet {
    hits.iter()
        .enumerate()
        .map(|(i, hit)| {
            let class = if current == Some(i) { CURRENT } else { HIT };
            Decoration::inline(hit.from, hit.to, Style::class(class))
        })
        .collect()
}

/// How many words and characters a document holds.
///
/// Words are runs separated by whitespace, counted per block so a heading and
/// the paragraph under it do not run together into one word at the join.
#[must_use]
pub fn count(doc: &Node) -> Count {
    let mut count = Count::default();
    doc.descendants(&mut |node, _, _, _| {
        if !node.is_textblock() {
            count.blocks += usize::from(node.is_block());
            return true;
        }
        count.blocks += 1;
        let text = node.text_content();
        count.characters += text.chars().count();
        count.words += text.split_whitespace().count();
        false
    });
    count
}

/// What [`count`] found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
    pub words: usize,
    pub characters: usize,
    pub blocks: usize,
}

/// Every heading in a document, for an outline.
#[must_use]
pub fn outline(doc: &Node) -> Vec<Heading> {
    let mut out = Vec::new();
    doc.descendants(&mut |node, pos, _, _| {
        if node.type_name() != crate::basic::nodes::HEADING {
            return true;
        }
        out.push(Heading {
            pos: pos + 1,
            level: node.attrs().get_int("level").unwrap_or(1),
            text: node.text_content(),
        });
        false
    });
    out
}

/// One entry in a document's outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The position of the heading's first character, for jumping to it.
    pub pos: usize,
    pub level: i64,
    pub text: String,
}

/// A marker so an unused import is impossible to leave behind.
#[allow(dead_code)]
const _: Option<Marks> = None;
