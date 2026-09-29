// SPDX-License-Identifier: MPL-2.0

//! Markdown out.
//!
//! # Escaping, and not over-escaping
//!
//! A `*` in prose has to come back as `*` and not as emphasis, so it is
//! escaped — but only where it could be read as syntax. Escaping every
//! punctuation mark produces text that is technically correct and unreadable,
//! and Markdown that a human will not edit by hand has lost the only reason to
//! choose Markdown.
//!
//! # Block separation
//!
//! One blank line between blocks, always, and no trailing whitespace. A
//! serialiser that varies its spacing makes every export a diff.

use std::fmt::Write as _;

use nib_model::mark::Marks;
use nib_model::node::Node;
use nib_model::schema::Schema;

use crate::Dialect;
use crate::components::{COMPONENT_BLOCK, COMPONENT_INLINE, ESM, NAME, SOURCE};

/// Writes a document as Markdown.
#[must_use]
pub fn to_markdown(schema: &Schema, dialect: Dialect, doc: &Node) -> String {
    let mut out = String::new();
    let w = Writer {
        schema,
        dialect,
        prefix: String::new(),
    };
    w.blocks(doc, &mut out);
    out
}

struct Writer<'a> {
    schema: &'a Schema,
    dialect: Dialect,
    /// What every line of the current block starts with — `> ` inside a quote,
    /// spaces inside a list item.
    prefix: String,
}

impl Writer<'_> {
    fn nested(&self, more: &str) -> Writer<'_> {
        Writer {
            schema: self.schema,
            dialect: self.dialect,
            prefix: format!("{}{more}", self.prefix),
        }
    }

    /// Writes a run of block nodes, one blank line apart.
    fn blocks(&self, parent: &Node, out: &mut String) {
        use nib_model::basic::nodes;
        for (i, child) in parent.content().into_iter().enumerate() {
            if i > 0 && !Self::tight_after(parent, i) {
                // The blank line keeps the prefix's markers: a bare blank
                // line inside a quote ends it, and the next paragraph would
                // come back as a second quote.
                self.line("", out);
            }
            // Two lists of one kind side by side are one list to a reader of
            // the output unless their markers differ, so every other one in a
            // run takes the alternative.
            let is_list = matches!(child.type_name(), nodes::BULLET_LIST | nodes::ORDERED_LIST);
            let run = parent
                .content()
                .iter()
                .take(i)
                .rev()
                .take_while(|before| is_list && before.type_name() == child.type_name())
                .count();
            if run % 2 == 1 {
                let start = (child.type_name() == nodes::ORDERED_LIST)
                    .then(|| child.attrs().get_int("start").unwrap_or(1));
                self.list(child, start, true, out);
            } else {
                self.block(child, out);
            }
        }
    }

    /// Whether the block at `index` follows the one before with no blank line.
    ///
    /// A list nested under a paragraph in a list item is the case: a blank
    /// line there makes the list loose, which re-parses as a different
    /// document and renders with different spacing.
    fn tight_after(parent: &Node, index: usize) -> bool {
        use nib_model::basic::nodes;
        let Some(node) = parent.child(index) else {
            return false;
        };
        // Only a list that can interrupt a paragraph may follow one without a
        // blank line: not one whose first item is empty, and not a numbered
        // one that starts anywhere but 1. Either would be read as more of the
        // paragraph.
        let interrupts = node
            .first_child()
            .is_some_and(|item| item.content_size() > 2)
            && (node.type_name() == nodes::BULLET_LIST
                || node.attrs().get_int("start").unwrap_or(1) == 1);
        parent.type_name() == nodes::LIST_ITEM
            && matches!(node.type_name(), nodes::BULLET_LIST | nodes::ORDERED_LIST)
            && interrupts
    }

    #[allow(clippy::too_many_lines)]
    fn block(&self, node: &Node, out: &mut String) {
        use nib_model::basic::nodes;

        match node.type_name() {
            nodes::PARAGRAPH => {
                self.line(&self.inline(node), out);
            }
            nodes::HEADING => {
                let level = node.attrs().get_int("level").unwrap_or(1).clamp(1, 6);
                let hashes = "#".repeat(usize::try_from(level).unwrap_or(1));
                let mut text = self.inline(node);
                // A run of `#` at the end, after a space or alone, would be
                // read as the closing sequence and dropped.
                let before = text.trim_end_matches('#');
                if before.len() < text.len() && (before.is_empty() || before.ends_with([' ', '\t']))
                {
                    text.insert(before.len(), '\\');
                }
                // An ATX heading is one line, so a line break inside one needs
                // the underlined form — which exists for the first two levels
                // only.
                if level <= 2 && text.contains('\n') {
                    self.line(&text, out);
                    self.line(if level == 1 { "===" } else { "---" }, out);
                } else {
                    self.line(&format!("{hashes} {text}"), out);
                }
            }
            nodes::HORIZONTAL_RULE => self.line("---", out),
            nodes::CODE_BLOCK => {
                let language = node.attrs().get_str("language").unwrap_or("");
                let text = node.text_content();
                // Longer than any run of backticks the code holds, so no line
                // of it can close the block early.
                // A backtick fence's info string may not hold a backtick; a
                // tilde fence's may.
                let fence_char = if language.contains('`') { '~' } else { '`' };
                let fence = fence_char
                    .to_string()
                    .repeat(longest_run(&text, fence_char).max(2) + 1);
                // The info string has its backslash escapes and entities
                // decoded when read, so both are escaped when written.
                let mut info = String::new();
                escape_destination(language, &mut info);
                self.line(&format!("{fence}{info}"), out);
                // Every line as it is, trailing spaces included: in code they
                // are content. The newline before the closing fence is the
                // fence's, which is why the parser takes one off.
                for line in text.split('\n') {
                    self.code_line(line, out);
                }
                self.line(&fence, out);
            }
            nodes::BLOCKQUOTE => {
                let inner = self.nested("> ");
                inner.blocks(node, out);
            }
            nodes::BULLET_LIST => self.list(node, None, false, out),
            nodes::ORDERED_LIST => {
                let start = node.attrs().get_int("start").unwrap_or(1);
                self.list(node, Some(start), false, out);
            }
            nodes::TABLE if self.dialect.has_gfm() => self.table(node, out),
            name if name == COMPONENT_BLOCK => self.component_block(node, out),
            name if name == ESM => {
                self.line(node.attrs().get_str(SOURCE).unwrap_or(""), out);
            }
            // Anything without a spelling of its own comes out as its blocks,
            // which is better than losing it.
            _ if node.is_textblock() => self.line(&self.inline(node), out),
            _ => self.blocks(node, out),
        }
    }

    /// A list, with the alternative markers (`*`, `1)`) when `alternate`.
    fn list(&self, node: &Node, start: Option<i64>, alternate: bool, out: &mut String) {
        // No blank line between items: a tight list is what the author almost
        // always wrote, and a loose one re-parses as tight anyway.
        for (i, item) in node.content().into_iter().enumerate() {
            let marker = match (start, alternate) {
                (Some(start), false) => format!("{}. ", start + i64::try_from(i).unwrap_or(0)),
                (Some(start), true) => format!("{}) ", start + i64::try_from(i).unwrap_or(0)),
                (None, false) => "- ".to_owned(),
                (None, true) => "* ".to_owned(),
            };
            let marker = match item.attrs().get_bool("checked") {
                Some(true) => format!("{marker}[x] "),
                Some(false) => format!("{marker}[ ] "),
                None => marker,
            };
            // The first line carries the marker; the rest line up under the
            // item's content, which starts after the list marker — a task's
            // checkbox is part of the content, not of the marker.
            let width = match (start, alternate) {
                (Some(start), _) => format!("{}. ", start + i64::try_from(i).unwrap_or(0)).len(),
                (None, _) => 2,
            };
            let indent = " ".repeat(width);
            let nested = self.nested(&indent);
            let mut body = String::new();
            nested.blocks(item, &mut body);
            let mut lines = body.lines();
            if let Some(first) = lines.next() {
                // The first line already carries the whole prefix, the
                // enclosing quote's markers included; the marker replaces
                // only the indent at its end.
                out.push_str(&self.prefix);
                out.push_str(&marker);
                // An empty item's line is the prefix alone, trimmed at its end.
                out.push_str(
                    first
                        .strip_prefix(nested.prefix.as_str())
                        .or_else(|| first.strip_prefix(nested.prefix.trim_end()))
                        .unwrap_or(first)
                        .trim_start(),
                );
                out.push('\n');
            }
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
    }

    fn table(&self, node: &Node, out: &mut String) {
        let rows: Vec<Vec<String>> = node
            .content()
            .iter()
            .map(|row| {
                row.content()
                    .iter()
                    // A pipe ends the cell wherever it is, inside a code span
                    // or a link included, so every one is escaped.
                    .map(|cell| self.inline_of_blocks(cell).replace('|', "\\|"))
                    .collect()
            })
            .collect();
        let Some(header) = rows.first() else { return };

        self.line(&format!("| {} |", header.join(" | ")), out);
        let rule: Vec<String> = header.iter().map(|_| "---".to_owned()).collect();
        self.line(&format!("| {} |", rule.join(" | ")), out);
        for row in rows.iter().skip(1) {
            self.line(&format!("| {} |", row.join(" | ")), out);
        }
    }

    fn component_block(&self, node: &Node, out: &mut String) {
        let name = node.attrs().get_str(NAME).unwrap_or("component");
        let props = write_props(node, &[NAME], self.dialect);
        if self.dialect == Dialect::Mdx {
            if node.content().is_empty() {
                self.line(&format!("<{name}{props} />"), out);
                return;
            }
            self.line(&format!("<{name}{props}>"), out);
            self.blocks(node, out);
            self.line(&format!("</{name}>"), out);
            return;
        }
        self.line(&format!("::{name}{props}"), out);
        if !node.content().is_empty() {
            self.blocks(node, out);
        }
        self.line("::", out);
    }

    /// A line, with the current prefix and no trailing whitespace.
    /// A line of code: kept whole, since trailing spaces in code are content.
    /// Only a blank one is trimmed, so the prefix's own spaces do not linger.
    fn code_line(&self, text: &str, out: &mut String) {
        if text.is_empty() {
            self.line(text, out);
        } else {
            out.push_str(&self.prefix);
            out.push_str(text);
            out.push('\n');
        }
    }

    fn line(&self, text: &str, out: &mut String) {
        let line = format!("{}{text}", self.prefix);
        out.push_str(line.trim_end());
        out.push('\n');
    }

    /// A textblock's inline content.
    ///
    /// Emphasis is written with `*`, `**` and `~~` wherever those read back as
    /// the same marks, and as `<em>`, `<strong>` and `<del>` where they would
    /// not. Whether a delimiter opens or closes depends on the characters on
    /// either side of it, and some mark boundaries — a letter on one side,
    /// punctuation on the other — have no delimiter spelling at all. Rather
    /// than approximate those rules, the delimiter spelling is read back with
    /// the parser, and the tags, which have no such rules, are used for any
    /// block it does not survive.
    fn inline(&self, node: &Node) -> String {
        let delimited = self.inline_spelled(node, Spelling::Delimiters);
        if self.reads_back(node, &delimited) {
            delimited
        } else {
            self.inline_spelled(node, Spelling::Tags)
        }
    }

    /// Whether `written` reads back as `node`'s content.
    fn reads_back(&self, node: &Node, written: &str) -> bool {
        let expected =
            crate::parse::textblock_content(node.content().iter().cloned().collect(), false);
        if expected
            .iter()
            .all(|child| child.marks().iter().all(|m| delimiter(m.name()).is_none()))
        {
            return true;
        }
        let doc = crate::parse::parse(self.schema, Dialect::Gfm, written);
        match doc.content().iter().collect::<Vec<_>>().as_slice() {
            [block] if block.is_textblock() => {
                block.content().iter().cloned().collect::<Vec<_>>() == expected
            }
            _ => false,
        }
    }

    fn inline_spelled(&self, node: &Node, spelling: Spelling) -> String {
        let mut out = String::new();
        // The delimiters open, innermost last, by mark name.
        let mut open: Vec<(String, &'static str)> = Vec::new();
        // Whitespace that ended the last run, held back until it is known
        // which delimiters close after it.
        let mut held = String::new();
        for child in node.content() {
            let marks = if child.is_inline() {
                child.marks().clone()
            } else {
                Marks::none()
            };
            let wanted: Vec<(String, (&'static str, &'static str))> = marks
                .iter()
                .filter_map(|m| spelling.of(m.name()).map(|d| (m.name().to_owned(), d)))
                .collect();

            // Marked whitespace alone would be a pair of delimiters around
            // nothing, which reads back as literal text; it is written as the
            // plain whitespace it looks like.
            let bare = !marks.iter().any(|m| {
                matches!(
                    m.name(),
                    nib_model::basic::marks::CODE | nib_model::basic::marks::LINK
                )
            });
            if !wanted.is_empty()
                && bare
                && let Some(text) = child.text()
                && text.trim().is_empty()
            {
                held.push_str(text);
                continue;
            }

            let shared = open
                .iter()
                .zip(&wanted)
                .take_while(|(a, b)| a.0 == b.0)
                .count();
            while open.len() > shared {
                if let Some((_, close)) = open.pop() {
                    out.push_str(close);
                }
            }
            out.push_str(&std::mem::take(&mut held));

            let is_code = marks
                .iter()
                .any(|m| m.name() == nib_model::basic::marks::CODE);
            let link = marks
                .iter()
                .find(|m| m.name() == nib_model::basic::marks::LINK);

            // A delimiter against whitespace neither opens nor closes, so
            // the whitespace at the edges of plain marked text goes outside
            // them: before the ones opening here, after the ones closing
            // after.
            let text = child.text().filter(|_| !is_code && link.is_none());
            if let Some(text) = text
                && wanted.len() > shared
            {
                let lead = &text[..text.len() - text.trim_start().len()];
                escape(lead, &mut out);
            }
            for (name, (opening, closing)) in wanted.iter().skip(shared) {
                out.push_str(opening);
                open.push((name.clone(), closing));
            }

            // Inline code excludes every other mark, so a code run stands
            // alone and is written whole: its delimiter depends on what it
            // holds, which a shared delimiter cannot.
            if is_code && let Some(text) = child.text() {
                code_span(text, &mut out);
                continue;
            }

            // A link's brackets sit outside its delimiters and carry a target,
            // so they are written around the run rather than as a delimiter.
            if let Some(link) = link {
                // A `!` just before the bracket would make the link an image.
                if out.ends_with('!') && !out.ends_with("\\!") {
                    out.pop();
                    out.push_str("\\!");
                }
                out.push('[');
                self.inline_node(child, &mut out);
                out.push(']');
                target(
                    link.attrs().get_str("href").unwrap_or(""),
                    link.attrs().get_str("title"),
                    &mut out,
                );
            } else if let Some(text) = text
                && !wanted.is_empty()
            {
                let start = if wanted.len() > shared {
                    text.len() - text.trim_start().len()
                } else {
                    0
                };
                let end = text.trim_end().len().max(start);
                escape(&text[start..end], &mut out);
                held.push_str(&text[end..]);
            } else {
                self.inline_node(child, &mut out);
            }
        }
        while let Some((_, close)) = open.pop() {
            out.push_str(close);
        }
        out.push_str(&held);
        out
    }

    /// A cell's content, flattened — a table cell holds blocks but a Markdown
    /// table row is one line.
    fn inline_of_blocks(&self, node: &Node) -> String {
        node.content()
            .iter()
            .map(|block| self.inline(block))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn inline_node(&self, node: &Node, out: &mut String) {
        use nib_model::basic::nodes;

        if let Some(text) = node.text() {
            let in_code = node
                .marks()
                .iter()
                .any(|m| m.name() == nib_model::basic::marks::CODE);
            if in_code {
                out.push_str(text);
            } else {
                escape(text, out);
            }
            return;
        }
        match node.type_name() {
            // The backslash form: trailing spaces are invisible, and a line of
            // nothing but them is a blank line that ends the paragraph.
            nodes::HARD_BREAK => out.push_str("\\\n"),
            nodes::IMAGE => {
                out.push_str("![");
                escape(node.attrs().get_str("alt").unwrap_or(""), out);
                out.push(']');
                target(
                    node.attrs().get_str("src").unwrap_or(""),
                    node.attrs().get_str("title"),
                    out,
                );
            }
            name if name == COMPONENT_INLINE => {
                let component = node.attrs().get_str(NAME).unwrap_or("component");
                let props = write_props(node, &[NAME], self.dialect);
                if self.dialect == Dialect::Mdx {
                    let _ = write!(out, "<{component}{props} />");
                } else {
                    let _ = write!(out, ":{component}{props}");
                }
            }
            _ => out.push_str(&node.text_content()),
        }
    }
}

/// The delimiter a mark is written with, or `None` for marks written another
/// way (a link) or not at all.
fn delimiter(name: &str) -> Option<&'static str> {
    use nib_model::basic::marks;
    match name {
        marks::STRONG => Some("**"),
        marks::EM => Some("*"),
        marks::STRIKETHROUGH => Some("~~"),
        // Inline code is written whole by `code_span`; a link by `target`.
        _ => None,
    }
}

/// How emphasis is written: as Markdown's delimiters, or as the HTML tags
/// Markdown lets stand in for them.
#[derive(Clone, Copy)]
enum Spelling {
    Delimiters,
    Tags,
}

impl Spelling {
    /// A mark's opening and closing text, when it has one.
    fn of(self, name: &str) -> Option<(&'static str, &'static str)> {
        use nib_model::basic::marks;
        match self {
            Self::Delimiters => delimiter(name).map(|d| (d, d)),
            Self::Tags => match name {
                marks::STRONG => Some(("<strong>", "</strong>")),
                marks::EM => Some(("<em>", "</em>")),
                marks::STRIKETHROUGH => Some(("<del>", "</del>")),
                _ => None,
            },
        }
    }
}

/// The longest run of `c` in `text`.
fn longest_run(text: &str, c: char) -> usize {
    text.split(|x| x != c).map(str::len).max().unwrap_or(0)
}

/// A code span, delimited by one more backtick than the longest run inside
/// it, and padded with a space where the content would otherwise touch the
/// delimiter or lose a space to the parser's stripping of one from each end.
fn code_span(text: &str, out: &mut String) {
    let fence = "`".repeat(longest_run(text, '`') + 1);
    let pad = text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty());
    let pad = if pad { " " } else { "" };
    let _ = write!(out, "{fence}{pad}{text}{pad}{fence}");
}

/// Text read with its backslash escapes and entity references decoded — a
/// link destination, a code block's info string — written so that decoding
/// gives it back.
fn escape_destination(text: &str, out: &mut String) {
    for (i, ch) in text.char_indices() {
        if ch == '\\' || entity_at(text, i) {
            out.push('\\');
        }
        out.push(ch);
    }
}

/// Whether an entity or character reference starts at byte `at`: `&`, a name
/// or `#` and digits, then `;`.
fn entity_at(text: &str, at: usize) -> bool {
    let Some(rest) = text[at..].strip_prefix('&') else {
        return false;
    };
    let name = rest.split(';').next().unwrap_or("");
    rest.len() > name.len()
        && !name.is_empty()
        && name
            .strip_prefix('#')
            .unwrap_or(name)
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
}

/// A link or image target, with its title, in parentheses.
///
/// A destination with a space or a parenthesis in it goes in angle brackets,
/// where neither ends it; a title is quoted with its quotes escaped.
fn target(href: &str, title: Option<&str>, out: &mut String) {
    out.push('(');
    if href.is_empty() || href.contains([' ', '(', ')', '<', '>', '\\']) {
        out.push('<');
        for (i, ch) in href.char_indices() {
            if matches!(ch, '<' | '>' | '\\') || entity_at(href, i) {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('>');
    } else {
        escape_destination(href, out);
    }
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        out.push_str(" \"");
        for (i, ch) in title.char_indices() {
            if matches!(ch, '"' | '\\') || entity_at(title, i) {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    }
    out.push(')');
}

/// A component's props, in the dialect's spelling.
///
/// MDC brackets the whole list and writes values bare; MDX writes JSX
/// attributes, quoting strings and bracing everything else.
fn write_props(node: &Node, skip: &[&str], dialect: Dialect) -> String {
    let jsx = dialect == Dialect::Mdx;
    let mut parts = Vec::new();
    for (name, value) in node.attrs().iter() {
        if skip.contains(&name) || value.is_null() {
            continue;
        }
        match value {
            nib_model::Value::Bool(true) => parts.push(name.to_owned()),
            nib_model::Value::Str(s) => parts.push(format!("{name}=\"{s}\"")),
            other if jsx => parts.push(format!("{name}={{{other}}}")),
            other => parts.push(format!("{name}={other}")),
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    if jsx {
        format!(" {}", parts.join(" "))
    } else {
        format!("{{{}}}", parts.join(" "))
    }
}

/// Escapes what would otherwise be read as syntax, and nothing else.
fn escape(text: &str, out: &mut String) {
    let mut at_line_start = out.is_empty() || out.ends_with('\n');
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        let next = chars.peek().map(|(_, c)| *c);
        match ch {
            // Always ambiguous inside a line. A tilde is strikethrough on its
            // own, and a fence in a run of three.
            '*' | '_' | '`' | '[' | ']' | '\\' | '~' => {
                out.push('\\');
                out.push(ch);
            }
            // The start of a tag, a comment or an autolink, which the parser
            // would take and the document would lose.
            '<' if next
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?')) =>
            {
                out.push_str("\\<");
            }
            // The start of an entity or a character reference.
            '&' if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '#') => {
                out.push_str("\\&");
            }
            // Only ambiguous where a block marker could start: a heading, a
            // list, a quote, or the underline that turns the line above into
            // a heading.
            '#' | '-' | '+' | '>' | '=' if at_line_start => {
                out.push('\\');
                out.push(ch);
            }
            // `1986. A year` and `1) one` open an ordered list.
            '0'..='9' if at_line_start => {
                let digits = text[i..].bytes().take_while(u8::is_ascii_digit).count();
                let after = &text[i + digits..];
                out.push_str(&text[i..i + digits]);
                for _ in 1..digits {
                    chars.next();
                }
                if digits <= 9
                    && let Some(punct) = after.chars().next().filter(|c| matches!(c, '.' | ')'))
                    && after[1..]
                        .chars()
                        .next()
                        .is_none_or(|c| c == ' ' || c == '\t')
                {
                    out.push('\\');
                    out.push(punct);
                    chars.next();
                }
                at_line_start = false;
                continue;
            }
            _ => out.push(ch),
        }
        at_line_start = ch == '\n';
    }
}
