// SPDX-License-Identifier: MPL-2.0

//! Typing that means something.
//!
//! `# ` at the start of a line becomes a heading. `- ` becomes a bullet.
//! `"` becomes `“` or `”` depending on which side of a word it lands.
//! ` -- ` becomes an em dash. Each is a pattern matched against the text
//! immediately before the caret, and a transaction produced when it matches.
//!
//! # Why this is not the parser
//!
//! Because it is not parsing. [`nib-markdown`] turns a *document* of Markdown
//! into a document; this turns a *keystroke* into a structural edit and then
//! forgets it happened. The two produce the same heading from the same `# `,
//! and they have to be separate: a rule that fired on pasted text would
//! rewrite a code sample, and a parser that ran per keystroke would fight the
//! user for the rest of the paragraph.
//!
//! # Undoing one
//!
//! A rule that fires when the user did not mean it must come back with one
//! Backspace, not with an undo that also takes back the sentence. The rule
//! records what it replaced on the transaction it produces, and
//! [`undo_input_rule`] reverses exactly that.
//!
//! [`nib-markdown`]: https://github.com/Magnetar-OS/cosmic-ext-nib

use std::sync::Arc;

use regex::Regex;

use crate::attrs::Attrs;
use crate::commands::{Command, command};
use crate::fragment::Fragment;
use crate::node::Node;
use crate::schema::{MarkTypeId, NodeTypeId};
use crate::slice::Slice;
use crate::state::{EditorState, Selection, Transaction};
use crate::transform::structure::find_wrapping;

/// The meta name under which a fired rule records what it replaced.
pub const APPLIED: &str = "inputRule";

/// How far back from the caret a rule may look.
///
/// A rule is about what the user just typed, not about the paragraph. Bounding
/// the window keeps a long line from being re-matched against every rule on
/// every keystroke.
const LOOKBEHIND: usize = 500;

/// What a fired rule undid, kept so one Backspace can put it back.
#[derive(Debug, Clone)]
pub struct Applied {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

/// What happens when a rule's pattern matches.
type Handler = Arc<
    dyn Fn(&EditorState, &regex::Captures<'_>, usize, usize) -> Option<Transaction> + Send + Sync,
>;

/// A pattern and what to do when the text before the caret matches it.
#[derive(Clone)]
pub struct InputRule {
    pattern: Regex,
    handler: Handler,
    /// Whether the rule may fire inside a node whose schema says `code`.
    in_code: bool,
}

impl std::fmt::Debug for InputRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRule")
            .field("pattern", &self.pattern.as_str())
            .finish_non_exhaustive()
    }
}

impl InputRule {
    /// A rule that runs `handler` when `pattern` matches.
    ///
    /// The pattern must be anchored at its end (`$`), because it is matched
    /// against the text *up to* the caret.
    ///
    /// # Panics
    ///
    /// If `pattern` is not a valid regular expression. Rules are literals in
    /// source, so a bad one is a programming error.
    #[must_use]
    pub fn new(
        pattern: &str,
        handler: impl Fn(&EditorState, &regex::Captures<'_>, usize, usize) -> Option<Transaction>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            pattern: Regex::new(pattern).expect("input rule pattern should compile"),
            handler: Arc::new(handler),
            in_code: false,
        }
    }

    /// A rule that replaces the match with fixed text — smart quotes, dashes,
    /// ellipses.
    ///
    /// # Panics
    ///
    /// If `pattern` is not a valid regular expression.
    #[must_use]
    pub fn text(pattern: &str, replacement: &'static str) -> Self {
        Self::new(pattern, move |state, _, from, to| {
            let mut tr = state.tr();
            let marks = state.doc().resolve(from).marks();
            let node = state.schema().text(replacement, marks);
            tr.replace(from, to, Slice::new(Fragment::from(node), 0, 0))
                .ok()?;
            Some(tr.clone())
        })
    }

    /// Lets this rule fire inside code.
    ///
    /// Off by default, and that default is the point: nobody wants `--` turned
    /// into an em dash inside a shell command.
    #[must_use]
    pub fn allowed_in_code(mut self) -> Self {
        self.in_code = true;
        self
    }
}

/// Turns the block the caret is in into `typ` when the pattern matches.
///
/// `attrs` is given the capture groups, so `#{1,6} ` can set the heading
/// level from the number of hashes.
///
/// # Panics
///
/// If `pattern` is not a valid regular expression.
#[must_use]
pub fn block_rule(
    pattern: &str,
    typ: NodeTypeId,
    attrs: impl Fn(&regex::Captures<'_>) -> Option<Attrs> + Send + Sync + 'static,
) -> InputRule {
    InputRule::new(pattern, move |state, caps, from, to| {
        let at = state.doc().resolve(from);
        let depth = at.depth();
        let index = at.index(depth.checked_sub(1)?);
        if !at
            .node(depth - 1)
            .can_replace_with(index, index + 1, typ, None)
        {
            return None;
        }
        let attrs = attrs(caps);
        let mut tr = state.tr();
        tr.delete(from, to).ok()?;
        tr.set_block_type(from, from, typ, attrs.as_ref()).ok()?;
        Some(tr.clone())
    })
}

/// Wraps the block the caret is in when the pattern matches — `> ` for a
/// quote, `- ` for a list.
///
/// # Panics
///
/// If `pattern` is not a valid regular expression.
#[must_use]
pub fn wrap_rule(
    pattern: &str,
    typ: NodeTypeId,
    attrs: impl Fn(&regex::Captures<'_>) -> Option<Attrs> + Send + Sync + 'static,
) -> InputRule {
    InputRule::new(pattern, move |state, caps, from, to| {
        let at = state.doc().resolve(from);
        let range = at.block_range(&at, None)?;
        let wrapping = find_wrapping(state.schema(), &range, typ, attrs(caps), None)?;
        let mut tr = state.tr();
        tr.delete(from, to).ok()?;
        let range = {
            let doc = tr.doc().clone();
            let at = doc.resolve(tr.mapping().map(from, 1));
            at.block_range(&at, None)?
        };
        tr.wrap(&range, &wrapping).ok()?;
        Some(tr.clone())
    })
}

/// Applies a mark to the text the pattern captured — `**bold**`, `` `code` ``.
///
/// The capture group named by `group` is what keeps its text; everything else
/// the pattern matched is the delimiters, and goes.
///
/// # Panics
///
/// If `pattern` is not a valid regular expression.
#[must_use]
pub fn mark_rule(pattern: &str, mark: MarkTypeId, group: usize) -> InputRule {
    InputRule::new(pattern, move |state, caps, from, to| {
        let whole = caps.get(0)?;
        let inner = caps.get(group)?;
        // Positions of the captured text within the document.
        let text_start = from + (inner.start() - whole.start());
        let text_end = from + (inner.end() - whole.start());

        let made = state.schema().mark_by_id(mark, None).ok()?;
        let mut tr = state.tr();
        // Remove the closing delimiter first, so the opening one's position is
        // still valid when we get to it.
        if text_end < to {
            tr.delete(text_end, to).ok()?;
        }
        if from < text_start {
            tr.delete(from, text_start).ok()?;
        }
        let end = from + inner.len();
        tr.add_mark(from, end, &made).ok()?;
        tr.remove_stored_mark(&made);
        Some(tr.clone())
    })
}

/// The transaction produced by the rules for text typed at `from..to`, or
/// `None` when none of them match.
///
/// The view calls this on every text input, before inserting the text itself:
/// a rule that fires replaces the insertion.
#[must_use]
pub fn apply(
    state: &EditorState,
    rules: &[InputRule],
    from: usize,
    to: usize,
    text: &str,
) -> Option<Transaction> {
    let at = state.doc().resolve(from);
    // Never inside an atom: its content is not the user's to type into.
    if at.parent().is_atom() {
        return None;
    }
    let in_code = at.parent().typ().spec().code;

    let start = at.start(at.depth());
    let before = at.parent().text_between(
        (from - start).saturating_sub(LOOKBEHIND),
        from - start,
        None,
        Some(&|_| "\u{fffc}".to_owned()),
    );
    let candidate = format!("{before}{text}");

    for rule in rules {
        if in_code && !rule.in_code {
            continue;
        }
        let Some(caps) = rule.pattern.captures(&candidate) else {
            continue;
        };
        let whole = caps.get(0)?;
        // Where in the document the match begins.
        let match_from = from - (candidate.len() - whole.start() - text.len()).min(from);
        let tr = (rule.handler)(state, &caps, match_from, to)?;
        return Some(tr.set_meta(
            APPLIED,
            Applied {
                from: match_from,
                to: match_from + text.len(),
                text: candidate[whole.start()..].to_owned(),
            },
        ));
    }
    None
}

/// Puts back what an input rule just replaced.
///
/// Bound to Backspace ahead of the ordinary delete, so the keystroke after an
/// unwanted `# ` undoes the heading rather than eating a character.
#[must_use]
pub fn undo_input_rule() -> Command {
    command(|state| {
        // The rule's own record travels on the transaction that fired it, and
        // the plugin below is what remembers it into the next keystroke.
        let applied = state.plugin_state::<Option<Applied>>(PLUGIN)?.clone()?;
        let mut tr = state.tr();
        let doc = tr.doc().clone();
        let end = applied.from + applied.text.chars().count();
        if end > doc.content_size() {
            return None;
        }
        let marks = doc.resolve(applied.from).marks();
        let node = state.schema().text(applied.text.as_str(), marks);
        tr.replace(
            applied.from,
            applied.to,
            Slice::new(Fragment::from(node), 0, 0),
        )
        .ok()?;
        Some(tr.clone())
    })
}

/// The key the last-fired rule is remembered under.
pub const PLUGIN: crate::state::PluginKey = crate::state::PluginKey::new("inputRules");

/// A plugin that remembers the most recent fired rule, so
/// [`undo_input_rule`] has something to undo.
#[must_use]
pub fn input_rules_plugin() -> crate::state::Plugin {
    struct Field;
    impl crate::state::StateField for Field {
        fn init(&self, _doc: &Node, _selection: &Selection) -> crate::state::FieldValue {
            Arc::new(Option::<Applied>::None)
        }

        fn apply(
            &self,
            tr: &Transaction,
            _value: &crate::state::FieldValue,
            _old: &EditorState,
            _new_doc: &Node,
            _new_selection: &Selection,
        ) -> crate::state::FieldValue {
            // Only the transaction that fired keeps the record; the next one
            // clears it, so Backspace undoes a rule only immediately after.
            Arc::new(tr.meta::<Applied>(APPLIED).cloned())
        }
    }
    crate::state::Plugin::new(PLUGIN).with_state(Field)
}

/// The rules an editor has unless it says otherwise.
///
/// Built against a schema by name, so a schema without headings gets no
/// heading rule.
#[must_use]
pub fn base(schema: &crate::schema::Schema) -> Vec<InputRule> {
    use crate::basic::{marks, nodes};

    let mut rules = vec![
        // Typography. Straight quotes become curly ones based on what is
        // before them: after a space or nothing, an opening quote.
        InputRule::text(r#"(?:^|[\s\(\[\{<])(")$"#, "\u{201c}"),
        InputRule::text(r#"(")$"#, "\u{201d}"),
        InputRule::text(r"(?:^|[\s\(\[\{<])(')$", "\u{2018}"),
        InputRule::text(r"(')$", "\u{2019}"),
        InputRule::text(r"\.\.\.$", "\u{2026}"),
        InputRule::text(r"--$", "\u{2014}"),
    ];

    if let Some(id) = schema.node_id(nodes::HEADING) {
        rules.push(block_rule(r"^(#{1,6})\s$", id, |caps| {
            let level = i64::try_from(caps.get(1).map_or(1, |m| m.len())).unwrap_or(1);
            Some(crate::attrs! { "level" => level })
        }));
    }
    if let Some(id) = schema.node_id(nodes::CODE_BLOCK) {
        rules.push(block_rule(r"^```(\w*)\s?$", id, |caps| {
            let language = caps.get(1).map_or("", |m| m.as_str());
            (!language.is_empty()).then(|| crate::attrs! { "language" => language })
        }));
    }
    if let Some(id) = schema.node_id(nodes::BLOCKQUOTE) {
        rules.push(wrap_rule(r"^\s*>\s$", id, |_| None));
    }
    if let Some(id) = schema.node_id(nodes::BULLET_LIST) {
        rules.push(wrap_rule(r"^\s*([-+*])\s$", id, |_| None));
    }
    if let Some(id) = schema.node_id(nodes::ORDERED_LIST) {
        rules.push(wrap_rule(r"^(\d+)\.\s$", id, |caps| {
            let start = caps.get(1)?.as_str().parse::<i64>().ok()?;
            Some(crate::attrs! { "start" => start })
        }));
    }

    // Inline marks. Two delimiters before one, so `**bold**` is not read as
    // emphasis around `*bold*`.
    if let Some(id) = schema.mark_id(marks::STRONG) {
        rules.push(mark_rule(r"(?:\*\*)([^*]+)(?:\*\*)$", id, 1));
        rules.push(mark_rule(r"(?:__)([^_]+)(?:__)$", id, 1));
    }
    if let Some(id) = schema.mark_id(marks::EM) {
        rules.push(mark_rule(r"(?:^|[^*])\*([^*]+)\*$", id, 1));
        rules.push(mark_rule(r"(?:^|[^_])_([^_]+)_$", id, 1));
    }
    if let Some(id) = schema.mark_id(marks::STRIKETHROUGH) {
        rules.push(mark_rule(r"(?:~~)([^~]+)(?:~~)$", id, 1));
    }
    if let Some(id) = schema.mark_id(marks::CODE) {
        rules.push(mark_rule(r"(?:`)([^`]+)(?:`)$", id, 1).allowed_in_code());
    }

    rules
}
