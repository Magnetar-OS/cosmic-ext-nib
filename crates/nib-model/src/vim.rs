// SPDX-License-Identifier: MPL-2.0

//! Modal editing.
//!
//! # Why this is not a [`Keymap`](crate::keymap::Keymap)
//!
//! A keymap is a map from a key to a command, and that is all modeless editing
//! needs. Vim is not modeless: `d` means nothing on its own, `2d3w` multiplies
//! two counts that arrived either side of an operator, and `j` is a cursor
//! move in one mode and a letter in another. That is a state machine, so this
//! is a state machine — [`Vim`] holds the mode, the half-typed count and the
//! pending operator between key presses.
//!
//! It sits *in front of* the keymap rather than replacing it. A host feeds
//! every key press to [`Vim::key`] first; a [`Response::Pass`] means Vim wants
//! nothing to do with this key and the ordinary keymap should see it. That is
//! how insert mode keeps every Ctrl binding the editor already has.
//!
//! # What is here
//!
//! Motions `h j k l w b e 0 ^ $ gg G` and the arrow keys; operators `d c y >
//! <` with motions, doubled (`dd`), and over a visual selection; the text
//! objects `iw aw ip ap`; `x X D C s S o O i I a A p P J r ~ u` and Ctrl-R;
//! visual and visual-line mode; counts on all of it.
//!
//! # What is not
//!
//! Named registers, marks, macros, `.` repeat, and Ex commands. `/ ? n N :`
//! come back as [`Response::Prompt`] because a search prompt is a piece of
//! interface, and this crate draws none.

use crate::commands::{self as cmd, Command, chain};
use crate::keymap::{Binding, Key, Mods};
use crate::motion;
use crate::node::Node;
use crate::schema::Schema;
use crate::slice::Slice;
use crate::state::{EditorState, Selection, Transaction};

/// Which mode the editor is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    /// The name Vim shows on the last line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
        }
    }

    /// Whether the caret should be drawn as a block.
    ///
    /// Everywhere but insert mode: in Vim the caret is *on* a character rather
    /// than between two, and the block is what says so.
    #[must_use]
    pub fn block_caret(self) -> bool {
        self != Mode::Insert
    }
}

/// What the host should do with a key it just handed to [`Vim::key`].
pub enum Response {
    /// Vim wants nothing to do with this key. Handle it as usual — which in
    /// insert mode means inserting the character.
    Pass,
    /// Consumed, with nothing to apply: a mode change, or a count still being
    /// typed.
    Consumed,
    /// Consumed. Apply this transaction.
    Apply(Box<Transaction>),
    /// Consumed, and Vim wants the host's own prompt: `/`, `?` or `:`, or `n`
    /// and `N` to step through what that prompt last found.
    Prompt(char),
}

/// What shape a yank left in the register, which is what a put has to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// A run of characters. `p` puts it after the caret.
    Chars,
    /// Whole blocks. `p` puts them after the block the caret is in.
    Blocks,
    /// One line out of a block that holds several — a line of code. `p` opens
    /// a line below and puts it there, because a line of code is not a block
    /// and inserting it as one would break the block in two.
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
}

impl Operator {
    fn of(c: char) -> Option<Self> {
        match c {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            '>' => Some(Operator::Indent),
            '<' => Some(Operator::Outdent),
            _ => None,
        }
    }

    fn key(self) -> char {
        match self {
            Operator::Delete => 'd',
            Operator::Change => 'c',
            Operator::Yank => 'y',
            Operator::Indent => '>',
            Operator::Outdent => '<',
        }
    }
}

/// A key that means nothing until the next one arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `g`, waiting for the second half of `gg`.
    Goto,
    /// `r`, waiting for the character to put down.
    Replace,
    /// `i` or `a` after an operator, waiting for the object. The flag is `a`.
    Object(bool),
}

/// Where a motion lands, and how an operator should read the trip.
#[derive(Debug, Clone, Copy)]
struct Target {
    to: usize,
    /// Whole blocks, not a span of characters: `j`, `G`, `dd`.
    linewise: bool,
    /// The character under the landing position is part of the range: `e`,
    /// `$`. Motions are exclusive unless they say otherwise.
    inclusive: bool,
}

impl Target {
    fn exclusive(to: usize) -> Self {
        Self {
            to,
            linewise: false,
            inclusive: false,
        }
    }

    fn inclusive(to: usize) -> Self {
        Self {
            to,
            linewise: false,
            inclusive: true,
        }
    }

    fn linewise(to: usize) -> Self {
        Self {
            to,
            linewise: true,
            inclusive: false,
        }
    }
}

/// The modal state machine.
///
/// One per editor. Feed it every key press before the keymap sees it.
pub struct Vim {
    mode: Mode,
    count: Option<usize>,
    operator: Option<Operator>,
    operator_count: Option<usize>,
    pending: Option<Pending>,
    /// Where a visual selection started. Vim's anchor survives motions that
    /// the selection's own anchor would not.
    anchor: usize,
    register: Option<(Slice, Held)>,
    /// What `>` and `<` do — sinking a list item where there is a list,
    /// indenting text where there is code.
    indent: Option<Command>,
    outdent: Option<Command>,
}

impl Vim {
    /// A Vim bound to a schema.
    ///
    /// The schema decides what `>` can do: with lists it sinks a list item,
    /// with code blocks it indents their text, and with neither it does
    /// nothing rather than failing.
    #[must_use]
    pub fn new(schema: &Schema) -> Self {
        use crate::basic::nodes;

        let width = crate::keymap::CODE_INDENT;
        let mut indent = vec![cmd::indent_code(width)];
        let mut outdent = vec![cmd::outdent_code(width)];
        if let Some(item) = schema.node_id(nodes::LIST_ITEM) {
            indent.insert(0, cmd::sink_list_item(item));
            outdent.insert(0, cmd::lift_list_item(item));
        }

        Self {
            mode: Mode::Normal,
            count: None,
            operator: None,
            operator_count: None,
            pending: None,
            anchor: 0,
            register: None,
            indent: Some(chain(indent)),
            outdent: Some(chain(outdent)),
        }
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Drops back to normal mode, forgetting any half-typed command.
    ///
    /// For the host to call when focus leaves, or a file is loaded under the
    /// cursor — anything that makes a pending `2d` meaningless.
    pub fn reset(&mut self) {
        self.mode = Mode::Normal;
        self.clear();
    }

    /// The half-typed command, for the corner of a status bar: `"2d"`.
    #[must_use]
    pub fn pending(&self) -> String {
        let mut out = String::new();
        if let Some(n) = self.operator_count {
            out.push_str(&n.to_string());
        }
        if let Some(op) = self.operator {
            out.push(op.key());
        }
        if let Some(n) = self.count {
            out.push_str(&n.to_string());
        }
        match self.pending {
            Some(Pending::Goto) => out.push('g'),
            Some(Pending::Replace) => out.push('r'),
            Some(Pending::Object(all)) => out.push(if all { 'a' } else { 'i' }),
            None => {}
        }
        out
    }

    fn clear(&mut self) {
        self.count = None;
        self.operator = None;
        self.operator_count = None;
        self.pending = None;
    }

    /// The count typed for a motion, and 1 when none was.
    fn steps(&self) -> usize {
        let motion = self.count.unwrap_or(1);
        let operator = self.operator_count.unwrap_or(1);
        motion.saturating_mul(operator).max(1)
    }

    /// Handles a key press.
    ///
    /// # Panics
    ///
    /// Never. Every failed edit comes back as [`Response::Consumed`].
    pub fn key(&mut self, state: &EditorState, binding: &Binding) -> Response {
        // A modifier that is not Shift is never part of a Vim command — those
        // are the host's own bindings, and Ctrl-R is the one exception.
        if binding.mods.contains(Mods::PRIMARY) || binding.mods.contains(Mods::SECONDARY) {
            if self.mode != Mode::Insert && binding.key == Key::Char('r') {
                self.clear();
                return run(state, &cmd::redo());
            }
            return Response::Pass;
        }

        match self.mode {
            Mode::Insert => self.insert_key(state, binding),
            _ => self.command_key(state, binding),
        }
    }

    fn insert_key(&mut self, state: &EditorState, binding: &Binding) -> Response {
        if binding.key != Key::Escape {
            return Response::Pass;
        }
        self.mode = Mode::Normal;
        self.clear();
        // Vim steps back off the character it was about to type in front of.
        let doc = state.doc();
        let at = state.selection().head();
        let back =
            motion::by_grapheme(doc, at, false).filter(|&to| to >= motion::block_start(doc, at));
        match back {
            Some(to) => self.move_to(state, to, false),
            None => Response::Consumed,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn command_key(&mut self, state: &EditorState, binding: &Binding) -> Response {
        let doc = state.doc();
        let at = state.selection().head();

        if binding.key == Key::Escape {
            let was_visual = self.mode != Mode::Normal;
            self.mode = Mode::Normal;
            self.clear();
            return if was_visual {
                self.move_to(state, at, false)
            } else {
                Response::Consumed
            };
        }

        // A pending `r` swallows whatever comes next.
        if self.pending == Some(Pending::Replace) {
            self.pending = None;
            let Key::Char(c) = binding.key else {
                self.clear();
                return Response::Consumed;
            };
            return self.replace_char(state, c);
        }

        if let Some(Pending::Object(all)) = self.pending {
            self.pending = None;
            let Key::Char(c) = binding.key else {
                self.clear();
                return Response::Consumed;
            };
            return self.text_object(state, c, all);
        }

        if self.pending == Some(Pending::Goto) {
            self.pending = None;
            if binding.key != Key::Char('g') {
                self.clear();
                return Response::Consumed;
            }
            let to = match self.count.or(self.operator_count) {
                Some(n) => nth_block(doc, n),
                None => motion::document_start(doc),
            };
            return self.go(state, Target::linewise(to));
        }

        let Key::Char(c) = binding.key else {
            return self.named_key(state, binding);
        };

        // Counts. A leading `0` is the motion, not a digit.
        if c.is_ascii_digit() && !(c == '0' && self.count.is_none()) {
            let digit = usize::from(c as u8 - b'0');
            let slot = if self.operator.is_some() {
                &mut self.count
            } else {
                &mut self.operator_count
            };
            *slot = Some(slot.unwrap_or(0).saturating_mul(10).saturating_add(digit));
            return Response::Consumed;
        }

        // An operator, or the same operator twice for its whole line.
        if let Some(op) = Operator::of(c) {
            if self.mode != Mode::Normal {
                let (from, to) = self.visual_range(state);
                let linewise = self.mode == Mode::VisualLine;
                self.mode = Mode::Normal;
                return self.operate(state, op, from, to, linewise);
            }
            if self.operator == Some(op) {
                let count = self.steps();
                let mut last = at;
                for _ in 1..count {
                    match motion::by_line(doc, last, true) {
                        Some(next) => last = next,
                        None => break,
                    }
                }
                return self.operate(state, op, at, last, true);
            }
            self.operator = Some(op);
            self.operator_count = self.operator_count.or(self.count.take());
            return Response::Consumed;
        }

        // A text object, but only while an operator or a visual selection is
        // waiting for one — bare `i` and `a` are insert commands.
        if (c == 'i' || c == 'a') && (self.operator.is_some() || self.mode != Mode::Normal) {
            self.pending = Some(Pending::Object(c == 'a'));
            return Response::Consumed;
        }

        if c == 'g' {
            self.pending = Some(Pending::Goto);
            return Response::Consumed;
        }

        if let Some(target) = self.motion(doc, at, c) {
            return self.go(state, target);
        }

        self.action(state, c)
    }

    fn named_key(&mut self, state: &EditorState, binding: &Binding) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        let count = self.steps();

        let target = match binding.key {
            Key::ArrowLeft | Key::Backspace => self.motion(doc, at, 'h'),
            Key::ArrowRight => self.motion(doc, at, 'l'),
            Key::ArrowUp => self.motion(doc, at, 'k'),
            Key::ArrowDown => self.motion(doc, at, 'j'),
            Key::Home => Some(Target::exclusive(motion::line_start(doc, at))),
            Key::End => Some(Target::inclusive(motion::line_end(doc, at))),
            Key::Delete => return self.delete_chars(state, count, true),
            Key::Enter => {
                let to = repeat(doc, at, count, motion::next_block);
                Some(Target::linewise(motion::first_non_blank(doc, to)))
            }
            _ => None,
        };
        match target {
            Some(target) => self.go(state, target),
            // Anything Vim has no opinion about — a function key — belongs to
            // the host, even in normal mode.
            None => Response::Pass,
        }
    }

    fn motion(&self, doc: &Node, at: usize, c: char) -> Option<Target> {
        let count = self.steps();
        Some(match c {
            'h' => Target::exclusive(repeat(doc, at, count, |d, p| {
                motion::by_grapheme(d, p, false).filter(|&to| to >= motion::line_start(d, p))
            })),
            'l' => Target::exclusive(repeat(doc, at, count, |d, p| {
                motion::by_grapheme(d, p, true).filter(|&to| to <= motion::line_end(d, p))
            })),
            'j' => Target::linewise(repeat(doc, at, count, |d, p| motion::by_line(d, p, true))),
            'k' => Target::linewise(repeat(doc, at, count, |d, p| motion::by_line(d, p, false))),
            'w' => Target::exclusive(repeat(doc, at, count, |d, p| motion::by_word(d, p, true))),
            'b' => Target::exclusive(repeat(doc, at, count, |d, p| motion::by_word(d, p, false))),
            'e' => {
                // `word_end` gives the position *after* the word; Vim's block
                // caret sits *on* the last character, one grapheme back.
                let after = repeat(doc, at, count, motion::word_end);
                let on = motion::by_grapheme(doc, after, false)
                    .filter(|&to| to >= motion::line_start(doc, after))
                    .unwrap_or(after);
                Target::inclusive(on)
            }
            '0' => Target::exclusive(motion::line_start(doc, at)),
            '^' => Target::exclusive(motion::first_non_blank(doc, at)),
            '$' => Target::inclusive(motion::line_end(doc, at)),
            '{' => Target::linewise(repeat(doc, at, count, motion::previous_block)),
            '}' => Target::linewise(repeat(doc, at, count, motion::next_block)),
            'G' => Target::linewise(match self.count.or(self.operator_count) {
                Some(n) => nth_block(doc, n),
                None => motion::document_end(doc),
            }),
            _ => return None,
        })
    }

    /// Runs a motion: either as a move, or as the second half of an operator.
    fn go(&mut self, state: &EditorState, target: Target) -> Response {
        let Some(op) = self.operator else {
            self.clear();
            return self.move_to(state, target.to, self.mode != Mode::Normal);
        };
        {
            {
                let at = state.selection().head();
                let doc = state.doc();
                let mut to = target.to;
                // An inclusive motion — `e`, `$` — takes the character it
                // landed on with it.
                if target.inclusive && to >= at {
                    to = motion::by_grapheme(doc, to, true)
                        .filter(|&n| n <= motion::line_end(doc, to))
                        .unwrap_or(to);
                }
                self.operate(state, op, at, to, target.linewise)
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn action(&mut self, state: &EditorState, c: char) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        let count = self.steps();
        if c == 'r' {
            // `r` is not finished — the character to write has not arrived,
            // and neither has the count been spent.
            self.pending = Some(Pending::Replace);
            return Response::Consumed;
        }
        self.clear();

        match c {
            'i' => self.enter_insert(state, at),
            'I' => {
                let to = motion::first_non_blank(doc, at);
                self.enter_insert(state, to)
            }
            'a' => {
                let to = motion::by_grapheme(doc, at, true)
                    .filter(|&to| to <= motion::line_end(doc, at))
                    .unwrap_or(at);
                self.enter_insert(state, to)
            }
            'A' => {
                let to = motion::line_end(doc, at);
                self.enter_insert(state, to)
            }
            'v' => self.enter_visual(state, Mode::Visual),
            'V' => self.enter_visual(state, Mode::VisualLine),
            'x' => self.delete_chars(state, count, true),
            'X' => self.delete_chars(state, count, false),
            'D' => self.operate(
                state,
                Operator::Delete,
                at,
                motion::block_end(doc, at),
                false,
            ),
            'C' => self.operate(
                state,
                Operator::Change,
                at,
                motion::block_end(doc, at),
                false,
            ),
            's' => {
                let to = motion::by_grapheme(doc, at, true)
                    .filter(|&to| to <= motion::line_end(doc, at))
                    .unwrap_or(at);
                self.operate(state, Operator::Change, at, to, false)
            }
            'S' => self.operate(state, Operator::Change, at, at, true),
            'o' => self.open_line(state, true),
            'O' => self.open_line(state, false),
            'p' => self.paste(state, true, count),
            'P' => self.paste(state, false, count),
            'J' => {
                // Vim joins from wherever the caret is; the command wants it
                // at the end of the block, which is the same join.
                let end = motion::block_end(doc, at);
                let moved = with_selection(state, Selection::cursor(end));
                match cmd::join_forward()(&moved) {
                    Some(tr) => Response::Apply(Box::new(tr)),
                    None => Response::Consumed,
                }
            }
            'u' => run(state, &cmd::undo()),
            '~' => swap_case(state, count),
            'Y' => self.operate(state, Operator::Yank, at, at, true),
            '/' | '?' | ':' | 'n' | 'N' => Response::Prompt(c),
            // Every other letter is a Vim command this does not have. Swallow
            // it: normal mode must never type into the document.
            _ => Response::Consumed,
        }
    }

    // -- the things actions do ---------------------------------------------

    fn enter_insert(&mut self, state: &EditorState, at: usize) -> Response {
        self.mode = Mode::Insert;
        self.move_to(state, at, false)
    }

    fn enter_visual(&mut self, state: &EditorState, mode: Mode) -> Response {
        self.mode = mode;
        self.anchor = state.selection().head();
        if mode == Mode::VisualLine {
            let doc = state.doc();
            let head = motion::block_end(doc, self.anchor);
            return selection(
                state,
                Selection::text(motion::block_start(doc, self.anchor), head),
            );
        }
        Response::Consumed
    }

    /// `x` and `X`.
    fn delete_chars(&mut self, state: &EditorState, count: usize, forward: bool) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        if self.mode != Mode::Normal {
            let (from, to) = self.visual_range(state);
            let linewise = self.mode == Mode::VisualLine;
            self.mode = Mode::Normal;
            return self.operate(state, Operator::Delete, from, to, linewise);
        }
        self.clear();
        let limit = if forward {
            motion::line_end(doc, at)
        } else {
            motion::line_start(doc, at)
        };
        let mut to = at;
        for _ in 0..count {
            match motion::by_grapheme(doc, to, forward) {
                Some(next) if forward && next <= limit => to = next,
                Some(next) if !forward && next >= limit => to = next,
                _ => break,
            }
        }
        if to == at {
            return Response::Consumed;
        }
        self.operate(state, Operator::Delete, at, to, false)
    }

    fn replace_char(&mut self, state: &EditorState, c: char) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        let count = self.steps();
        self.clear();

        let mut to = at;
        for _ in 0..count {
            match motion::by_grapheme(doc, to, true).filter(|&n| n <= motion::line_end(doc, at)) {
                Some(next) => to = next,
                None => return Response::Consumed,
            }
        }
        if to == at {
            return Response::Consumed;
        }
        let text: String = std::iter::repeat_n(c, count).collect();
        let mut tr = state.tr();
        if tr.replace(at, to, Slice::empty()).is_err() {
            return Response::Consumed;
        }
        if tr.insert_text(&text).is_err() {
            return Response::Consumed;
        }
        // Vim leaves the caret on the last character it wrote, not after it.
        let end = tr.selection().head();
        let back = motion::by_grapheme(tr.doc(), end, false).unwrap_or(end);
        tr.set_selection(Selection::cursor(back));
        Response::Apply(Box::new(tr.scroll_into_view()))
    }

    /// `o` and `O`.
    fn open_line(&mut self, state: &EditorState, below: bool) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        self.clear();
        self.mode = Mode::Insert;

        let split = if below {
            motion::block_end(doc, at)
        } else {
            motion::block_start(doc, at)
        };
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(split));
        if tr.split(split, 1, None).is_err() {
            return Response::Consumed;
        }
        // Splitting leaves the caret in the second of the two blocks, which is
        // where `o` wants it and the block above where `O` does.
        if !below {
            tr.set_selection(Selection::cursor(split));
        }
        Response::Apply(Box::new(tr.scroll_into_view()))
    }

    fn paste(&mut self, state: &EditorState, after: bool, count: usize) -> Response {
        let Some((slice, held)) = self.register.clone() else {
            self.clear();
            return Response::Consumed;
        };
        let doc = state.doc();
        let at = state.selection().head();
        self.clear();

        let mut tr = state.tr();
        // The position is always given in the document as it was, and mapped
        // forward — so opening a line for the content to go on does not have to
        // be accounted for by hand. Which side of an insertion the position
        // belongs on is what `assoc` says.
        let (pos, assoc) = match (held, after) {
            (Held::Blocks, true) => ((motion::block_end(doc, at) + 1).min(doc.content_size()), 1),
            (Held::Blocks, false) => (motion::block_start(doc, at).saturating_sub(1), 1),
            (Held::Chars, true) => (
                motion::by_grapheme(doc, at, true)
                    .filter(|&to| to <= motion::line_end(doc, at))
                    .unwrap_or(at),
                1,
            ),
            (Held::Chars, false) => (at, 1),
            // A line out of a block: open a line for it first, then fill it.
            // Putting the line in as a block would split the block it lands
            // in, which is not what `p` on a line of code means.
            (Held::Line, _) => {
                let edge = if after {
                    motion::line_end(doc, at)
                } else {
                    motion::line_start(doc, at)
                };
                tr.set_selection(Selection::cursor(edge));
                if tr.insert_text("\n").is_err() {
                    return Response::Consumed;
                }
                // `p` goes after the newline it just made, `P` before it.
                (edge, if after { 1 } else { -1 })
            }
        };

        for _ in 0..count {
            let target = tr.mapping().map(pos, assoc);
            if tr.replace(target, target, slice.clone()).is_err() {
                return Response::Consumed;
            }
        }
        // Vim leaves the caret on the first character of what it pasted.
        let landed = tr.mapping().map(pos, assoc).min(tr.doc().content_size());
        let near = Selection::near(tr.doc(), &tr.doc().resolve(landed), 1);
        tr.set_selection(near);
        Response::Apply(Box::new(tr.scroll_into_view()))
    }

    // -- operators ---------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn operate(
        &mut self,
        state: &EditorState,
        op: Operator,
        from: usize,
        to: usize,
        linewise: bool,
    ) -> Response {
        let doc = state.doc();
        self.clear();
        if self.mode != Mode::Insert {
            self.mode = Mode::Normal;
        }

        let (low, high) = (from.min(to), from.max(to));
        // What "linewise" covers depends on what a line is here. A paragraph
        // is a block and a line at once, so taking the line means taking the
        // block — boundaries and all, or it is left behind empty. A code block
        // holds many lines, and taking one of those must leave the block
        // standing. Change is the exception either way: `cc` empties the line
        // and stays in it.
        let whole_blocks =
            linewise && motion::line_is_block(doc, low) && motion::line_is_block(doc, high);
        let held = match (linewise, whole_blocks) {
            (false, _) => Held::Chars,
            (true, true) => Held::Blocks,
            (true, false) => Held::Line,
        };
        let stays = op == Operator::Change || op == Operator::Indent || op == Operator::Outdent;
        // What goes in the register is the line itself. The newline that ends
        // it belongs to the *deletion* — a put opens its own line to go on, so
        // a register that carried one would put two.
        let register = if held == Held::Line {
            Some(doc.slice(
                motion::line_start(doc, low),
                motion::line_end(doc, high),
                false,
            ))
        } else {
            None
        };
        let (low, high) = if linewise {
            let start = motion::line_start(doc, low);
            let end = motion::line_end(doc, high);
            match (whole_blocks, stays) {
                (_, true) => (start, end),
                (true, false) => (start.saturating_sub(1), (end + 1).min(doc.content_size())),
                // One line of several: take the newline that separates it from
                // the next, or from the one before when it is the last.
                (false, false) => {
                    if end < motion::block_end(doc, high) {
                        (start, end + 1)
                    } else {
                        (
                            start.saturating_sub(1).max(motion::block_start(doc, low)),
                            end,
                        )
                    }
                }
            }
        } else {
            (low, high)
        };

        match op {
            Operator::Yank => {
                let kept = register.unwrap_or_else(|| doc.slice(low, high, false));
                self.register = Some((kept, held));
                if linewise {
                    // `yy` leaves the caret exactly where it was. Only a yank
                    // over a motion pulls it back to the start of the range.
                    Response::Consumed
                } else {
                    self.move_to(state, low, false)
                }
            }
            Operator::Delete | Operator::Change => {
                if low == high {
                    return Response::Consumed;
                }
                let kept = register.unwrap_or_else(|| doc.slice(low, high, false));
                self.register = Some((kept, held));
                let mut tr = state.tr();
                let deleted = if whole_blocks && op == Operator::Delete {
                    tr.delete_range(low, high)
                } else {
                    tr.delete(low, high)
                };
                if deleted.is_err() {
                    return Response::Consumed;
                }
                let landed = tr.mapping().map(low, 1).min(tr.doc().content_size());
                let near = Selection::near(tr.doc(), &tr.doc().resolve(landed), 1);
                tr.set_selection(near);
                if op == Operator::Change {
                    self.mode = Mode::Insert;
                }
                Response::Apply(Box::new(tr.scroll_into_view()))
            }
            Operator::Indent | Operator::Outdent => {
                let command = if op == Operator::Indent {
                    self.indent.clone()
                } else {
                    self.outdent.clone()
                };
                let Some(command) = command else {
                    return Response::Consumed;
                };
                let moved = with_selection(state, Selection::text(low, high));
                match command(&moved) {
                    Some(tr) => Response::Apply(Box::new(tr.scroll_into_view())),
                    None => Response::Consumed,
                }
            }
        }
    }

    /// The document range a visual selection covers.
    fn visual_range(&self, state: &EditorState) -> (usize, usize) {
        let head = state.selection().head();
        let (low, high) = (self.anchor.min(head), self.anchor.max(head));
        if self.mode == Mode::VisualLine {
            return (low, high);
        }
        // Visual mode includes the character under the caret.
        let doc = state.doc();
        let end = motion::by_grapheme(doc, high, true)
            .filter(|&to| to <= motion::line_end(doc, high))
            .unwrap_or(high);
        (low, end)
    }

    // -- plumbing ----------------------------------------------------------

    /// Moves the caret, extending the visual selection if there is one.
    fn move_to(&self, state: &EditorState, to: usize, extend: bool) -> Response {
        let doc = state.doc();
        let to = to.min(doc.content_size());
        let wanted = if extend {
            if self.mode == Mode::VisualLine {
                let (low, high) = (self.anchor.min(to), self.anchor.max(to));
                Selection::text(motion::block_start(doc, low), motion::block_end(doc, high))
            } else {
                Selection::text(self.anchor, to)
            }
        } else {
            // `to` came out of arithmetic; `near` is what turns a position
            // into one a caret can actually occupy.
            Selection::near(doc, &doc.resolve(to), 1)
        };
        selection(state, wanted)
    }

    fn text_object(&mut self, state: &EditorState, c: char, all: bool) -> Response {
        let doc = state.doc();
        let at = state.selection().head();
        let range = match c {
            'w' => motion::word_range(doc, at, all),
            'p' => Some((motion::block_start(doc, at), motion::block_end(doc, at))),
            _ => None,
        };
        let Some((from, to)) = range else {
            self.clear();
            return Response::Consumed;
        };
        let linewise = c == 'p';

        let Some(op) = self.operator else {
            self.clear();
            self.anchor = from;
            return selection(state, Selection::text(from, to));
        };
        self.operate(state, op, from, to, linewise && all)
    }
}

/// A transaction that only moves the caret, or nothing when it is already
/// there — an unchanged selection should not cost the host a redraw.
fn selection(state: &EditorState, selection: Selection) -> Response {
    if &selection == state.selection() {
        return Response::Consumed;
    }
    let mut tr = state.tr();
    tr.set_selection(selection);
    Response::Apply(Box::new(tr.scroll_into_view()))
}

/// Runs an ordinary command, for the Vim keys that are one.
fn run(state: &EditorState, command: &Command) -> Response {
    match command(state) {
        Some(tr) => Response::Apply(Box::new(tr)),
        None => Response::Consumed,
    }
}

/// `~`: the case of the next `count` characters, flipped.
fn swap_case(state: &EditorState, count: usize) -> Response {
    let doc = state.doc();
    let at = state.selection().head();

    let block_end = motion::block_end(doc, at);
    let mut to = at;
    for _ in 0..count {
        match motion::by_grapheme(doc, to, true).filter(|&n| n <= block_end) {
            Some(next) => to = next,
            None => break,
        }
    }
    if to == at {
        return Response::Consumed;
    }
    let swapped: String = plain(doc, at, to)
        .chars()
        .map(|c| {
            if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        })
        .collect();

    let mut tr = state.tr();
    if tr.replace(at, to, Slice::empty()).is_err() || tr.insert_text(&swapped).is_err() {
        return Response::Consumed;
    }
    Response::Apply(Box::new(tr.scroll_into_view()))
}

/// Applies a motion `count` times, stopping wherever it runs out of document.
fn repeat(
    doc: &Node,
    from: usize,
    count: usize,
    step: impl Fn(&Node, usize) -> Option<usize>,
) -> usize {
    let mut at = from;
    for _ in 0..count {
        match step(doc, at) {
            Some(next) => at = next,
            None => break,
        }
    }
    at
}

/// The start of the `n`th block, counting from one — Vim's `5G`.
fn nth_block(doc: &Node, n: usize) -> usize {
    let mut at = motion::document_start(doc);
    for _ in 1..n.max(1) {
        match motion::next_block(doc, at) {
            Some(next) => at = next,
            None => break,
        }
    }
    at
}

/// The state as it would be with a different selection.
///
/// Commands read the selection off the state, so this is how an operator hands
/// its range to one. The transaction has no steps, so the document — and every
/// position in it — is untouched, and a transaction built against the result
/// applies just as well to the original.
fn with_selection(state: &EditorState, selection: Selection) -> EditorState {
    let mut tr = state.tr();
    tr.set_selection(selection);
    state.applied(tr)
}

/// The plain text between two positions.
fn plain(doc: &Node, from: usize, to: usize) -> String {
    doc.text_between(from, to, None, None)
}
