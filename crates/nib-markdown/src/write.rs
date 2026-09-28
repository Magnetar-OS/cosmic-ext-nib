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
        for (i, child) in parent.content().into_iter().enumerate() {
            if i > 0 && !Self::tight_after(parent, i) {
                // The blank line keeps the prefix's markers: a bare blank
                // line inside a quote ends it, and the next paragraph would
                // come back as a second quote.
                self.line("", out);
            }
            self.block(child, out);
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
        parent.type_name() == nodes::LIST_ITEM
            && matches!(node.type_name(), nodes::BULLET_LIST | nodes::ORDERED_LIST)
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
                self.line(&format!("{hashes} {}", self.inline(node)), out);
            }
            nodes::HORIZONTAL_RULE => self.line("---", out),
            nodes::CODE_BLOCK => {
                let language = node.attrs().get_str("language").unwrap_or("");
                let text = node.text_content();
                // Longer than any run of backticks the code holds, so no line
                // of it can close the block early.
                let fence = "`".repeat(longest_run(&text, '`').max(2) + 1);
                self.line(&format!("{fence}{language}"), out);
                // A fenced block's content conventionally ends with a newline;
                // splitting on it would add a line that was never there.
                for line in text.strip_suffix('\n').unwrap_or(&text).split('\n') {
                    self.line(line, out);
                }
                self.line(&fence, out);
            }
            nodes::BLOCKQUOTE => {
                let inner = self.nested("> ");
                inner.blocks(node, out);
            }
            nodes::BULLET_LIST => self.list(node, None, out),
            nodes::ORDERED_LIST => {
                let start = node.attrs().get_int("start").unwrap_or(1);
                self.list(node, Some(start), out);
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

    fn list(&self, node: &Node, start: Option<i64>, out: &mut String) {
        // No blank line between items: a tight list is what the author almost
        // always wrote, and a loose one re-parses as tight anyway.
        for (i, item) in node.content().into_iter().enumerate() {
            let marker = match start {
                Some(start) => format!("{}. ", start + i64::try_from(i).unwrap_or(0)),
                None => "- ".to_owned(),
            };
            let marker = match item.attrs().get_bool("checked") {
                Some(true) => format!("{marker}[x] "),
                Some(false) => format!("{marker}[ ] "),
                None => marker,
            };
            // The first line carries the marker; the rest line up under it.
            let indent = " ".repeat(marker.chars().count());
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
                out.push_str(
                    first
                        .strip_prefix(nested.prefix.as_str())
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
    fn line(&self, text: &str, out: &mut String) {
        let line = format!("{}{text}", self.prefix);
        out.push_str(line.trim_end());
        out.push('\n');
    }

    /// A textblock's inline content.
    fn inline(&self, node: &Node) -> String {
        let mut out = String::new();
        let mut open: Vec<String> = Vec::new();
        for child in node.content() {
            let marks = if child.is_inline() {
                child.marks().clone()
            } else {
                Marks::none()
            };
            let wanted: Vec<String> = marks
                .iter()
                .filter_map(|m| delimiter(m.name()).map(str::to_owned))
                .collect();

            let shared = open.iter().zip(&wanted).take_while(|(a, b)| a == b).count();
            while open.len() > shared {
                if let Some(delim) = open.pop() {
                    out.push_str(&delim);
                }
            }
            for delim in wanted.iter().skip(shared) {
                out.push_str(delim);
                open.push(delim.clone());
            }

            // Inline code excludes every other mark, so a code run stands
            // alone and is written whole: its delimiter depends on what it
            // holds, which a shared delimiter cannot.
            if let Some(text) = child.text()
                && marks
                    .iter()
                    .any(|m| m.name() == nib_model::basic::marks::CODE)
            {
                code_span(text, &mut out);
                continue;
            }

            // A link's brackets sit outside its delimiters and carry a target,
            // so they are written around the run rather than as a delimiter.
            let link = marks
                .iter()
                .find(|m| m.name() == nib_model::basic::marks::LINK);
            if let Some(link) = link {
                out.push('[');
                self.inline_node(child, &mut out);
                out.push(']');
                target(
                    link.attrs().get_str("href").unwrap_or(""),
                    link.attrs().get_str("title"),
                    &mut out,
                );
            } else {
                self.inline_node(child, &mut out);
            }
        }
        while let Some(delim) = open.pop() {
            out.push_str(&delim);
        }
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
            nodes::HARD_BREAK => out.push_str("  \n"),
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

/// A link or image target, with its title, in parentheses.
///
/// A destination with a space or a parenthesis in it goes in angle brackets,
/// where neither ends it; a title is quoted with its quotes escaped.
fn target(href: &str, title: Option<&str>, out: &mut String) {
    out.push('(');
    if href.is_empty() || href.contains([' ', '(', ')', '<', '>', '\\']) {
        out.push('<');
        for ch in href.chars() {
            if matches!(ch, '<' | '>' | '\\') {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('>');
    } else {
        out.push_str(href);
    }
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        out.push_str(" \"");
        for ch in title.chars() {
            if matches!(ch, '"' | '\\') {
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
