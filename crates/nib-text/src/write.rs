// SPDX-License-Identifier: MPL-2.0

//! A document as plain text.

use std::fmt::Write as _;

use nib_model::basic::nodes;
use nib_model::node::Node;
use unicode_segmentation::UnicodeSegmentation;

/// Where an outgoing body is wrapped, and what it does with structure.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The column to wrap at. `None` leaves lines as long as they are, which
    /// is what a terminal that wraps for itself wants.
    pub wrap: Option<usize>,
    /// Whether a heading is underlined with `=` and `-`, the convention plain
    /// text has for the two levels that matter.
    pub underline_headings: bool,
    /// The bullet a list item gets.
    pub bullet: char,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            // RFC 5322 asks for under 78; 72 is the convention on top of it,
            // and the headroom is what survives being quoted twice.
            wrap: Some(72),
            underline_headings: true,
            bullet: '-',
        }
    }
}

/// Writes a document as plain text.
#[must_use]
pub fn to_text(doc: &Node, options: Options) -> String {
    let mut out = String::new();
    blocks(doc, options, "", &mut out);
    out.trim_end().to_owned()
}

fn blocks(parent: &Node, options: Options, prefix: &str, out: &mut String) {
    for (i, child) in parent.content().into_iter().enumerate() {
        if i > 0 {
            let _ = writeln!(out, "{}", prefix.trim_end());
        }
        block(child, options, prefix, out);
    }
}

#[allow(clippy::too_many_lines)]
fn block(node: &Node, options: Options, prefix: &str, out: &mut String) {
    match node.type_name() {
        nodes::HEADING => {
            let text = inline(node);
            emit(&text, options, prefix, out);
            if options.underline_headings {
                let level = node.attrs().get_int("level").unwrap_or(1);
                let rule = if level <= 1 { '=' } else { '-' };
                let width = text.graphemes(true).count();
                let _ = writeln!(out, "{prefix}{}", rule.to_string().repeat(width));
            }
        }
        nodes::HORIZONTAL_RULE => {
            let width = options.wrap.unwrap_or(72).saturating_sub(prefix.len());
            let _ = writeln!(out, "{prefix}{}", "-".repeat(width.min(72)));
        }
        nodes::CODE_BLOCK => {
            // Indented rather than fenced: backticks in plain text are
            // punctuation, and indentation is what a reader sees as code.
            let text = node.text_content();
            for line in text.strip_suffix('\n').unwrap_or(&text).split('\n') {
                let _ = writeln!(out, "{prefix}    {line}");
            }
        }
        nodes::BLOCKQUOTE => {
            let inner = format!("{prefix}> ");
            blocks(node, options, &inner, out);
        }
        nodes::BULLET_LIST => list(node, options, prefix, None, out),
        nodes::ORDERED_LIST => {
            let start = node.attrs().get_int("start").unwrap_or(1);
            list(node, options, prefix, Some(start), out);
        }
        nodes::TABLE => table(node, options, prefix, out),
        _ if node.is_textblock() => emit(&inline(node), options, prefix, out),
        _ => blocks(node, options, prefix, out),
    }
}

fn list(node: &Node, options: Options, prefix: &str, start: Option<i64>, out: &mut String) {
    for (i, item) in node.content().into_iter().enumerate() {
        let marker = match start {
            Some(start) => format!("{}. ", start + i64::try_from(i).unwrap_or(0)),
            None => format!("{} ", options.bullet),
        };
        let checkbox = match item.attrs().get_bool("checked") {
            Some(true) => "[x] ",
            Some(false) => "[ ] ",
            None => "",
        };
        let indent = " ".repeat(marker.chars().count());
        let mut body = String::new();
        blocks(item, options, &format!("{prefix}{indent}"), &mut body);

        let mut lines = body.lines();
        if let Some(first) = lines.next() {
            let _ = writeln!(out, "{prefix}{marker}{checkbox}{}", first.trim_start());
        }
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
    }
}

/// A table as aligned columns, which is the only rendering plain text has.
fn table(node: &Node, options: Options, prefix: &str, out: &mut String) {
    let rows: Vec<Vec<String>> = node
        .content()
        .into_iter()
        .map(|row| row.content().into_iter().map(inline_of_blocks).collect())
        .collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|cell| cell.graphemes(true).count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    for (i, row) in rows.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                let pad = widths[c].saturating_sub(cell.graphemes(true).count());
                format!("{cell}{}", " ".repeat(pad))
            })
            .collect();
        let _ = writeln!(out, "{prefix}{}", cells.join("  ").trim_end());
        if i == 0 {
            let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            let _ = writeln!(out, "{prefix}{}", rule.join("  "));
        }
    }
    let _ = options;
}

/// A textblock's text, with the inline nodes that have a plain-text spelling.
fn inline(node: &Node) -> String {
    let mut out = String::new();
    for child in node.content() {
        if let Some(text) = child.text() {
            out.push_str(text);
            continue;
        }
        match child.type_name() {
            nodes::HARD_BREAK => out.push('\n'),
            nodes::IMAGE => {
                let alt = child.attrs().get_str("alt").unwrap_or("");
                let src = child.attrs().get_str("src").unwrap_or("");
                if alt.is_empty() {
                    let _ = write!(out, "[{src}]");
                } else {
                    let _ = write!(out, "[{alt}: {src}]");
                }
            }
            _ => out.push_str(&child.text_content()),
        }
    }
    // A link's target is invisible in plain text unless it is written out.
    for child in node.content() {
        if let Some(link) = child
            .marks()
            .iter()
            .find(|m| m.name() == nib_model::basic::marks::LINK)
            && let Some(href) = link.attrs().get_str("href")
            && !out.contains(href)
        {
            let _ = write!(out, " <{href}>");
        }
    }
    out
}

fn inline_of_blocks(node: &Node) -> String {
    node.content()
        .into_iter()
        .map(inline)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Writes a paragraph's worth of text, wrapped and prefixed.
fn emit(text: &str, options: Options, prefix: &str, out: &mut String) {
    for line in text.split('\n') {
        match options.wrap {
            None => {
                let _ = writeln!(out, "{prefix}{line}");
            }
            Some(columns) => {
                for wrapped in wrap(line, columns.saturating_sub(prefix.len()).max(20)) {
                    let _ = writeln!(out, "{prefix}{wrapped}");
                }
            }
        }
    }
}

/// Breaks a line at word boundaries, at or under `columns`.
///
/// A word longer than the column is left long rather than broken: a URL cut in
/// half is worse than a line that runs over, and every mail client that breaks
/// them produces links nobody can click.
#[must_use]
pub fn wrap(line: &str, columns: usize) -> Vec<String> {
    if line.graphemes(true).count() <= columns {
        return vec![line.to_owned()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut width = 0;
    for word in line.split(' ') {
        let word_width = word.graphemes(true).count();
        if width > 0 && width + 1 + word_width > columns {
            out.push(std::mem::take(&mut current));
            width = 0;
        }
        if width > 0 {
            current.push(' ');
            width += 1;
        }
        current.push_str(word);
        width += word_width;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
