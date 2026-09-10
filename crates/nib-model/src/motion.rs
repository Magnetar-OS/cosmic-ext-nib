// SPDX-License-Identifier: MPL-2.0

//! Moving a position through a document.
//!
//! # What is here and what is not
//!
//! Everything that depends only on the document: a grapheme, a word, the ends
//! of a block, the block before or after. Not *lines* — a line is a thing the
//! shaper made when it wrapped the text, and it changes when the window does,
//! so only the view can say where one ends.
//!
//! Both the view and the Vim keymap ask these questions, and before this
//! module they asked them separately. Two answers to "where is the previous
//! word" is one answer too many.

use unicode_segmentation::UnicodeSegmentation;

use crate::node::Node;
use crate::state::Selection;

/// One grapheme cluster left or right.
///
/// Not one byte, and not one character: an emoji with a skin tone is one thing
/// to the user however many code points it is.
#[must_use]
pub fn by_grapheme(doc: &Node, from: usize, forward: bool) -> Option<usize> {
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let start = at.start(at.depth());
    let offset = from.saturating_sub(start).min(text.len());

    if forward {
        if offset >= text.len() {
            // Out of this block: the next place a caret can be.
            let next = (from + 1).min(doc.content_size());
            return Selection::find_from(doc, &doc.resolve(next), 1, true).map(|s| s.head());
        }
        let step = text[offset..].graphemes(true).next()?.len();
        Some(from + step)
    } else {
        if offset == 0 {
            let previous = from.saturating_sub(1);
            return Selection::find_from(doc, &doc.resolve(previous), -1, true).map(|s| s.head());
        }
        let step = text[..offset].graphemes(true).next_back()?.len();
        Some(from - step)
    }
}

/// To the start of the next word, or the start of the word before.
///
/// Vim's `w` and `b`: `w` lands on the first character of the next word rather
/// than on the space in front of it, and at the end of a block it carries on
/// into the next one.
#[must_use]
pub fn by_word(doc: &Node, from: usize, forward: bool) -> Option<usize> {
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let start = at.start(at.depth());
    let offset = from.saturating_sub(start).min(text.len());

    let boundary = if forward {
        text.split_word_bound_indices()
            .find(|(i, word)| *i > offset && !word.chars().all(char::is_whitespace))
            .map(|(i, _)| i)
    } else {
        text[..offset]
            .split_word_bound_indices()
            .rfind(|(_, word)| !word.chars().all(char::is_whitespace))
            .map(|(i, _)| i)
    };
    if let Some(offset) = boundary {
        return Some(start + offset);
    }
    // Out of words in this block. Vim keeps going into the next one.
    if forward {
        next_block(doc, from)
            .map(|to| first_non_blank(doc, to))
            .or_else(|| Some(block_end(doc, from)))
    } else {
        previous_block(doc, from)
            .map(|to| block_end(doc, to))
            .or_else(|| Some(block_start(doc, from)))
    }
}

/// To the end of the word the position is in or before.
#[must_use]
pub fn word_end(doc: &Node, from: usize) -> Option<usize> {
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let start = at.start(at.depth());
    let offset = from.saturating_sub(start).min(text.len());

    text[offset..]
        .split_word_bound_indices()
        .find(|(i, word)| !word.trim().is_empty() && offset + i + word.len() > from - start)
        .map(|(i, word)| start + offset + i + word.len())
        .or_else(|| by_word(doc, from, true))
}

/// The start of the block the position is in.
#[must_use]
pub fn block_start(doc: &Node, from: usize) -> usize {
    let at = doc.resolve(from);
    at.start(at.depth())
}

/// The end of the block the position is in.
#[must_use]
pub fn block_end(doc: &Node, from: usize) -> usize {
    let at = doc.resolve(from);
    at.end(at.depth())
}

/// The start of the line the position is in.
///
/// A line is what the text says it is: a run between newlines. A paragraph
/// holds one and a code block holds many, which is why this is not the same
/// question as [`block_start`] — and why neither is the same question as where
/// the window happened to wrap, which only the view can answer.
#[must_use]
pub fn line_start(doc: &Node, from: usize) -> usize {
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let start = at.start(at.depth());
    let offset = from.saturating_sub(start).min(text.len());
    match text[..offset].rfind('\n') {
        Some(newline) => start + newline + 1,
        None => start,
    }
}

/// The end of the line the position is in.
#[must_use]
pub fn line_end(doc: &Node, from: usize) -> usize {
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let start = at.start(at.depth());
    let offset = from.saturating_sub(start).min(text.len());
    match text[offset..].find('\n') {
        Some(newline) => start + offset + newline,
        None => start + text.len(),
    }
}

/// Whether the line at `from` is the whole of its block.
///
/// What tells a paragraph from one line of a code block, and so what tells a
/// linewise operator whether it is taking a block away or emptying a line.
#[must_use]
pub fn line_is_block(doc: &Node, from: usize) -> bool {
    line_start(doc, from) == block_start(doc, from) && line_end(doc, from) == block_end(doc, from)
}

/// The first position at which the line's text is not whitespace.
///
/// Vim's `^`, and what Home does in most editors on the second press.
#[must_use]
pub fn first_non_blank(doc: &Node, from: usize) -> usize {
    let start = line_start(doc, from);
    let end = line_end(doc, from);
    let at = doc.resolve(from);
    let text = at.parent().text_content();
    let block = at.start(at.depth());
    let line = &text[start - block..end - block];
    let indent = line
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map_or(line.len(), |(i, _)| i);
    start + indent
}

/// The start of the next block, or `None` at the last one.
#[must_use]
pub fn next_block(doc: &Node, from: usize) -> Option<usize> {
    let end = block_end(doc, from);
    let after = (end + 1).min(doc.content_size());
    let found = Selection::find_from(doc, &doc.resolve(after), 1, true)?.head();
    (found > end).then_some(found)
}

/// The start of the previous block, or `None` at the first one.
#[must_use]
pub fn previous_block(doc: &Node, from: usize) -> Option<usize> {
    let start = block_start(doc, from);
    let before = start.checked_sub(1)?;
    let found = Selection::find_from(doc, &doc.resolve(before), -1, true)?.head();
    (found < start).then_some(found)
}

/// The same column one line down or up.
///
/// Within a code block that is the next of its own lines; at the last of them
/// it is the next block. What this is *not* is one row of a wrapped paragraph:
/// the shaper decided where those go, so only the view can say.
#[must_use]
pub fn by_line(doc: &Node, from: usize, forward: bool) -> Option<usize> {
    let column = from - line_start(doc, from);

    // Another line inside this block, or the first line of the next.
    let target = if forward {
        let end = line_end(doc, from);
        if end < block_end(doc, from) {
            end + 1
        } else {
            next_block(doc, from)?
        }
    } else {
        let start = line_start(doc, from);
        if start > block_start(doc, from) {
            line_start(doc, start - 1)
        } else {
            let previous = previous_block(doc, from)?;
            line_start(doc, block_end(doc, previous))
        }
    };

    let start = line_start(doc, target);
    let end = line_end(doc, target);
    // The column, clipped to the line, and never inside a character.
    let mut wanted = (start + column).min(end);
    let at = doc.resolve(target);
    let text = at.parent().text_content();
    let block = at.start(at.depth());
    while wanted > start && !text.is_char_boundary(wanted - block) {
        wanted -= 1;
    }
    Some(wanted)
}

/// The very first place a caret can be.
#[must_use]
pub fn document_start(doc: &Node) -> usize {
    Selection::at_start(doc).head()
}

/// The very last place a caret can be.
#[must_use]
pub fn document_end(doc: &Node) -> usize {
    Selection::at_end(doc).head()
}

/// The word around a position, as a range.
///
/// With `trailing`, the run of spaces after the word joins it — the difference
/// between Vim's `iw` and `aw`, and between "select this word" and "delete
/// this word and close the gap".
#[must_use]
pub fn word_range(doc: &Node, at: usize, trailing: bool) -> Option<(usize, usize)> {
    let resolved = doc.resolve(at);
    let text = resolved.parent().text_content();
    let start = resolved.start(resolved.depth());
    let offset = at.saturating_sub(start).min(text.len());

    let (mut from, mut to) = text
        .split_word_bound_indices()
        .find(|(i, word)| *i <= offset && i + word.len() > offset)
        .map(|(i, word)| (i, i + word.len()))?;

    if trailing {
        // Vim takes the space after the word, or — at the end of a line —
        // the space before it, so `daw` never leaves a doubled space.
        let after = text[to..]
            .split_word_bound_indices()
            .next()
            .filter(|(_, word)| !word.is_empty() && word.chars().all(char::is_whitespace));
        match after {
            Some((_, word)) => to += word.len(),
            None => {
                if let Some((i, word)) = text[..from]
                    .split_word_bound_indices()
                    .next_back()
                    .filter(|(_, word)| word.chars().all(char::is_whitespace))
                {
                    let _ = word;
                    from = i;
                }
            }
        }
    }
    Some((start + from, start + to))
}
