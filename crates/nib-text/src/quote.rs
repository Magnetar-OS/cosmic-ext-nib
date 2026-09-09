// SPDX-License-Identifier: MPL-2.0

//! Mail quoting.
//!
//! A `text/plain` mail body is not a paragraph of text; it is a small stack —
//! what was written now, what was written before, and who wrote it. Flattening
//! that into one string is what makes a twentieth reply unreadable: the two
//! lines that are new sit above forty that are not, in the same colour, at the
//! same size.
//!
//! These are pure functions over `&str`, so the decisions they encode — what a
//! quote *is*, where a signature starts — are testable rather than only
//! visible on screen.

/// One run of a plain-text body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Written by the sender of this message.
    Prose(String),
    /// Quoted history, with one level of `>` removed.
    Quoted {
        /// The shallowest depth in the run, so a reader can say how deep the
        /// history goes without re-scanning.
        depth: usize,
        text: String,
    },
    /// Everything after the sender's `-- ` line.
    Signature(String),
}

/// The signature separator, exactly as RFC 3676 §4.3 spells it — two hyphens,
/// a space, end of line. Emitted without the space by enough clients that the
/// bare form is recognised too, but never *written* that way.
const SIGNATURE_MARKER: &str = "--";

/// Splits a plain-text body into prose, quoted history, and a signature.
///
/// The rules, in the order they are applied:
///
/// - A line's quote depth is its leading run of `>`, ignoring spaces between
///   them, so `>>`, `> >` and `> > ` are all depth two.
/// - Consecutive lines at depth ≥ 1 are one [`Block::Quoted`]. They are *not*
///   split by depth changes: a reply that dips into a deeper quote and comes
///   back is one piece of history, and three blocks where the user sees one
///   run would fold into three separate controls.
/// - A blank line inside a quoted run stays in the run when quoted text
///   resumes after it. Senders leave real blank lines between quoted
///   paragraphs, and treating each as a boundary shatters the run.
/// - The first unquoted `--` or `-- ` line ends the message; the rest is the
///   signature. Quoted signatures are at depth ≥ 1 and so cannot trigger this.
#[must_use]
pub fn blocks(body: &str) -> Vec<Block> {
    let lines: Vec<&str> = body.lines().collect();
    let mut blocks = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let mut run_depth: Option<usize> = None;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let depth = quote_depth(line);

        if depth == 0 && is_signature_marker(line) {
            flush(&mut blocks, &mut run, run_depth);
            let signature = trimmed_join(&lines[index + 1..]);
            if !signature.is_empty() {
                blocks.push(Block::Signature(signature));
            }
            return blocks;
        }

        let same_kind = match run_depth {
            None => depth == 0,
            Some(_) => depth > 0 || blank_inside_quote(&lines, index),
        };
        if !same_kind && !run.is_empty() {
            flush(&mut blocks, &mut run, run_depth);
        }
        if run.is_empty() {
            run_depth = (depth > 0).then_some(depth);
        } else if let Some(current) = run_depth
            && depth > 0
        {
            run_depth = Some(current.min(depth));
        }
        run.push(line);
        index += 1;
    }
    flush(&mut blocks, &mut run, run_depth);
    blocks
}

/// True when a blank line is inside a quoted run rather than ending it —
/// quoted text resumes after it.
fn blank_inside_quote(lines: &[&str], index: usize) -> bool {
    if !lines[index].trim().is_empty() {
        return false;
    }
    lines[index + 1..]
        .iter()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| quote_depth(line) > 0)
}

fn flush(blocks: &mut Vec<Block>, run: &mut Vec<&str>, depth: Option<usize>) {
    if run.iter().all(|line| line.trim().is_empty()) {
        run.clear();
        return;
    }
    blocks.push(match depth {
        Some(depth) => {
            let stripped: Vec<&str> = run.iter().map(|line| unquote_one(line)).collect();
            Block::Quoted {
                depth,
                text: trimmed_join(&stripped),
            }
        }
        None => Block::Prose(trimmed_join(run)),
    });
    run.clear();
}

fn trimmed_join(lines: &[&str]) -> String {
    let mut joined = lines.join("\n");
    while joined.ends_with('\n') {
        joined.pop();
    }
    joined.trim_start_matches('\n').to_owned()
}

/// A line's quote depth: its leading run of `>`, ignoring spaces between them.
#[must_use]
pub fn quote_depth(line: &str) -> usize {
    let mut depth = 0;
    for ch in line.chars() {
        match ch {
            '>' => depth += 1,
            ' ' | '\t' => {}
            _ => break,
        }
    }
    depth
}

/// Removes one level of quoting from a line.
#[must_use]
pub fn unquote_one(line: &str) -> &str {
    let trimmed = line.trim_start_matches([' ', '\t']);
    match trimmed.strip_prefix('>') {
        Some(rest) => rest.strip_prefix(' ').unwrap_or(rest),
        None => line,
    }
}

fn is_signature_marker(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed == SIGNATURE_MARKER
}

/// Adds one level of quoting to every line.
#[must_use]
pub fn quote(body: &str) -> String {
    body.lines()
        .map(|line| {
            // A blank line gets a bare `>` rather than `> `: trailing
            // whitespace on a quote line is what makes a diff of two replies
            // noisy, and some clients strip it anyway.
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
