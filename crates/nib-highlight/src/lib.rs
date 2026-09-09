// SPDX-License-Identifier: MPL-2.0

//! Syntax highlighting for a document's code blocks.
//!
//! # Why this is a separate crate
//!
//! Because it is the only part of Nib with an opinion about programming
//! languages, and most documents do not have any code in them. A mail composer
//! should not carry a megabyte of Sublime Text grammars to write a paragraph.
//!
//! # What it produces
//!
//! Decorations, not colours. A highlighter knows that a token is a keyword; it
//! does not know whether keywords are blue today. So this emits
//! [`Decoration`]s carrying class names — `keyword`, `string`, `comment` — and
//! the view resolves them against the COSMIC theme. That is what lets one
//! document render correctly in light and dark without the highlighter being
//! told which is on.
//!
//! # Which grammar
//!
//! The code block's `language` attribute, matched against Sublime's syntax set
//! by name and then by file extension, so both `rust` and `rs` work. A block
//! with no language, or one nobody recognises, is left alone rather than
//! guessed at — a guess that lands on the wrong grammar is worse than no
//! colour, because it colours the wrong things confidently.
//!
//! ```
//! # use nib_model::{basic, build::Builder, nodes};
//! # use nib_highlight::Highlighter;
//! let b = Builder::new(basic::schema());
//! let doc = b.doc(nodes![b.attr_node(
//!     "code_block",
//!     &nib_model::attrs! { "language" => "rust" },
//!     nodes![b.text("fn main() {}")],
//! )]);
//! let decorations = Highlighter::new().decorate(&doc);
//! assert!(!decorations.is_empty());
//! ```

use std::sync::Arc;

use nib_model::decoration::{Decoration, DecorationSet, Style};
use nib_model::node::Node;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// Turns a document's code blocks into decorations.
///
/// Holds the syntax set, which is expensive to build and cheap to share, so an
/// application builds one and keeps it.
#[derive(Clone)]
pub struct Highlighter {
    syntaxes: Arc<SyntaxSet>,
    /// How a scope's first component maps to a class the theme knows.
    classes: Vec<(Scope, Arc<str>)>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("syntaxes", &self.syntaxes.syntaxes().len())
            .finish_non_exhaustive()
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

/// The scopes worth telling apart, most specific first.
///
/// Sublime's scopes are a deep hierarchy and a theme that distinguished every
/// level would be unreadable. These are the distinctions a reader actually
/// uses, and the order matters: `entity.name.function` must be tried before
/// `entity`.
const SCOPES: &[(&str, &str)] = &[
    ("comment", "comment"),
    ("string", "string"),
    ("constant.numeric", "number"),
    ("constant.language", "constant"),
    ("constant.character.escape", "string"),
    ("constant", "constant"),
    ("entity.name.function", "function"),
    ("entity.name.type", "type"),
    ("entity.name.namespace", "type"),
    ("entity.name.tag", "keyword"),
    ("entity.other.attribute-name", "attribute"),
    ("entity.name", "type"),
    ("support.function", "function"),
    ("support.type", "type"),
    ("support.class", "type"),
    ("support.constant", "constant"),
    // `storage.type` covers `fn`, `struct`, `int` — words that declare a type
    // rather than name one. Readers see them as keywords, and every widely
    // used theme colours them that way; `entity.name.type` and `support.type`
    // above are the ones that are actually type *names*.
    ("storage", "keyword"),
    ("keyword.operator", "punctuation"),
    ("keyword", "keyword"),
    ("variable.parameter", "variable"),
    ("variable.function", "function"),
    ("variable", "variable"),
    ("punctuation", "punctuation"),
    ("invalid", "error"),
];

impl Highlighter {
    /// Builds a highlighter over the bundled grammars.
    ///
    /// # Panics
    ///
    /// Never: the scope names above are literals checked by this crate's
    /// tests, and `SyntaxSet::load_defaults_newlines` cannot fail.
    #[must_use]
    pub fn new() -> Self {
        Self::with_syntaxes(Arc::new(SyntaxSet::load_defaults_newlines()))
    }

    /// Builds a highlighter over a syntax set of the caller's own — one loaded
    /// from a directory of grammars, say.
    ///
    /// # Panics
    ///
    /// Never; see [`Highlighter::new`].
    #[must_use]
    pub fn with_syntaxes(syntaxes: Arc<SyntaxSet>) -> Self {
        let classes = SCOPES
            .iter()
            .map(|(scope, class)| {
                (
                    Scope::new(scope).expect("the scope names above are valid"),
                    Arc::from(*class),
                )
            })
            .collect();
        Self { syntaxes, classes }
    }

    /// The grammars this highlighter knows, by name.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.syntaxes.syntaxes().iter().map(|s| s.name.as_str())
    }

    /// True when a language name will be recognised.
    #[must_use]
    pub fn knows(&self, language: &str) -> bool {
        self.syntax(language).is_some()
    }

    fn syntax(&self, language: &str) -> Option<&SyntaxReference> {
        let language = language.trim();
        if language.is_empty() {
            return None;
        }
        self.syntaxes
            .find_syntax_by_token(language)
            .or_else(|| self.syntaxes.find_syntax_by_extension(language))
            .or_else(|| self.syntaxes.find_syntax_by_name(language))
    }

    /// The decorations for every code block in a document.
    #[must_use]
    pub fn decorate(&self, doc: &Node) -> DecorationSet {
        let mut out = Vec::new();
        doc.descendants(&mut |node, pos, _, _| {
            if node.type_name() != nib_model::basic::nodes::CODE_BLOCK {
                return true;
            }
            let Some(language) = node.attrs().get_str("language") else {
                return false;
            };
            let Some(syntax) = self.syntax(language) else {
                return false;
            };
            // `pos` is the position of the node; its content starts one on.
            self.decorate_block(syntax, &node.text_content(), pos + 1, &mut out);
            false
        });
        DecorationSet::new(out)
    }

    /// The decorations for one code block, given where its text begins.
    #[must_use]
    pub fn decorate_text(&self, language: &str, text: &str, start: usize) -> DecorationSet {
        let Some(syntax) = self.syntax(language) else {
            return DecorationSet::empty();
        };
        let mut out = Vec::new();
        self.decorate_block(syntax, text, start, &mut out);
        DecorationSet::new(out)
    }

    fn decorate_block(
        &self,
        syntax: &SyntaxReference,
        text: &str,
        start: usize,
        out: &mut Vec<Decoration>,
    ) {
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut offset = start;

        // syntect wants each line to end in a newline; a block's last line
        // usually does not, and feeding it without one leaves the parser
        // mid-construct.
        for line in text.split_inclusive('\n') {
            let owned;
            let line = if line.ends_with('\n') {
                line
            } else {
                owned = format!("{line}\n");
                &owned
            };
            let Ok(ops) = state.parse_line(line, &self.syntaxes) else {
                // A grammar that fails on a line is a grammar that will fail
                // on the rest; the block keeps what it has and stops.
                return;
            };

            let mut run_start = 0;
            for (at, op) in ops {
                if at > run_start
                    && let Some(class) = self.class_of(&stack)
                {
                    out.push(Decoration::inline(
                        offset + run_start,
                        offset + at,
                        Style::class(class),
                    ));
                }
                run_start = at;
                if stack.apply(&op).is_err() {
                    return;
                }
            }
            // The tail of the line, after the last scope change.
            let end = line.trim_end_matches('\n').len();
            if end > run_start
                && let Some(class) = self.class_of(&stack)
            {
                out.push(Decoration::inline(
                    offset + run_start,
                    offset + end,
                    Style::class(class),
                ));
            }
            offset += line.trim_end_matches('\n').len();
            // The newline is a position in the document even though the
            // highlighter has nothing to say about it.
            if line.ends_with('\n') && offset < start + text.len() {
                offset += 1;
            }
        }
    }

    /// The class a scope stack resolves to, innermost scope first.
    fn class_of(&self, stack: &ScopeStack) -> Option<Arc<str>> {
        for scope in stack.scopes.iter().rev() {
            for (prefix, class) in &self.classes {
                if prefix.is_prefix_of(*scope) {
                    return Some(Arc::clone(class));
                }
            }
        }
        None
    }
}
