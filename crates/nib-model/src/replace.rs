// SPDX-License-Identifier: MPL-2.0

//! Replacing a range with a slice.
//!
//! One operation, and the only one that changes a document's shape. Every
//! other edit — typing, deleting, splitting a block, lifting a list item,
//! pasting — is a range and a slice handed to this.
//!
//! # The three-way join
//!
//! Replacing `from..to` with an open slice means joining three things that
//! were never siblings: what is left of the tree before `from`, the slice, and
//! what is left after `to`. Each of the three has its own chain of ancestors,
//! and the open depths say how many of those chains meet.
//!
//! The algorithm walks down all three chains at once. At every depth it asks
//! whether the left edge continues into the slice (`open_start` says so, and
//! the two node types must be compatible), whether the slice continues into
//! the right edge, and whether both — the case where a single node is being
//! rebuilt from three pieces. It refuses rather than guesses: joining a
//! `list_item` onto a `paragraph` is an error, not something to paper over,
//! and the caller — [`fit`](crate::fit) — is the one that knows how to retry
//! with the slice opened differently.

use crate::fragment::Fragment;
use crate::node::Node;
use crate::resolve::ResolvedPos;
use crate::slice::Slice;

/// Why a replacement could not be performed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ReplaceError {
    #[error("inserted content is deeper ({open}) than the position it is inserted at ({depth})")]
    TooDeep { open: usize, depth: usize },
    #[error("the slice's open depths do not match the range's: {start} against {end}")]
    InconsistentDepths { start: usize, end: usize },
    #[error("cannot join {sub} onto {main}")]
    CannotJoin { sub: String, main: String },
    #[error("the result would not be valid content for {node}: {found}")]
    InvalidContent { node: String, found: String },
}

/// Replaces `from..to` in `doc` with `slice`.
///
/// # Errors
///
/// [`ReplaceError`] when the slice cannot be joined into the range — an open
/// depth that does not match, or node types that cannot follow one another.
pub fn replace(
    from: &ResolvedPos,
    to: &ResolvedPos,
    slice: &Slice,
) -> Result<Node, ReplaceError> {
    if slice.open_start() > from.depth() {
        return Err(ReplaceError::TooDeep {
            open: slice.open_start(),
            depth: from.depth(),
        });
    }
    if from.depth() - slice.open_start() != to.depth() - slice.open_end() {
        return Err(ReplaceError::InconsistentDepths {
            start: from.depth() - slice.open_start(),
            end: to.depth().saturating_sub(slice.open_end()),
        });
    }
    replace_outer(from, to, slice, 0)
}

fn replace_outer(
    from: &ResolvedPos,
    to: &ResolvedPos,
    slice: &Slice,
    depth: usize,
) -> Result<Node, ReplaceError> {
    let index = from.index(depth);
    let node = from.node(depth);

    if index == to.index(depth) && depth < from.depth() - slice.open_start() {
        // Both ends are inside the same child: recurse without touching this
        // level at all.
        let inner = replace_outer(from, to, slice, depth + 1)?;
        return Ok(node.copy(node.content().replace_child(index, inner)));
    }

    if slice.is_empty() {
        return close(node, replace_two_way(from, to, depth)?);
    }

    if slice.open_start() == 0
        && slice.open_end() == 0
        && from.depth() == depth
        && to.depth() == depth
    {
        // Flat: a run of complete nodes going into one parent.
        let parent = from.parent();
        let content = parent.content();
        let joined = content
            .cut(0, from.parent_offset())
            .append(slice.content())
            .append(&content.cut(to.parent_offset(), content.size()));
        return close(parent, joined);
    }

    let (start, end) = prepare_slice(slice, from);
    close(node, replace_three_way(from, &start, &end, to, depth)?)
}

/// Rebuilds the slice inside a copy of the target's ancestor chain, so its
/// edges can be resolved as positions and walked the same way the document's
/// are.
fn prepare_slice(slice: &Slice, along: &ResolvedPos) -> (ResolvedPos, ResolvedPos) {
    let extra = along.depth() - slice.open_start();
    let parent = along.node(extra);
    let mut node = parent.copy(slice.content().clone());
    for i in (0..extra).rev() {
        node = along.node(i).copy(Fragment::from(node));
    }
    let start = node.resolve(slice.open_start() + extra);
    let end = node.resolve(node.content_size() - slice.open_end() - extra);
    (start, end)
}

fn check_join(main: &Node, sub: &Node) -> Result<(), ReplaceError> {
    if sub.typ().is_compatible_content(main.typ()) {
        Ok(())
    } else {
        Err(ReplaceError::CannotJoin {
            sub: sub.type_name().to_owned(),
            main: main.type_name().to_owned(),
        })
    }
}

fn joinable(before: &ResolvedPos, after: &ResolvedPos, depth: usize) -> Result<Node, ReplaceError> {
    let node = before.node(depth);
    check_join(node, after.node(depth))?;
    Ok(node.clone())
}

/// Appends a node, merging it into the previous one when both are text with
/// the same markup.
fn add_node(child: Node, target: &mut Vec<Node>) {
    if let Some(last) = target.last()
        && child.is_text()
        && last.is_text()
        && child.same_markup(last)
    {
        let joined = format!(
            "{}{}",
            last.text().unwrap_or(""),
            child.text().unwrap_or("")
        );
        let index = target.len() - 1;
        target[index] = last.with_text(joined);
        return;
    }
    target.push(child);
}

/// Copies the children of the node at `depth` that fall between the two
/// positions, either of which may be absent to mean "from the start" or "to
/// the end".
fn add_range(
    start: Option<&ResolvedPos>,
    end: Option<&ResolvedPos>,
    depth: usize,
    target: &mut Vec<Node>,
) {
    let node = end.or(start).expect("at least one end is given").node(depth);
    let mut start_index = 0;
    let end_index = end.map_or_else(|| node.child_count(), |e| e.index(depth));
    if let Some(start) = start {
        start_index = start.index(depth);
        if start.depth() > depth {
            start_index += 1;
        } else if start.text_offset() > 0 {
            if let Some(after) = start.node_after() {
                add_node(after, target);
            }
            start_index += 1;
        }
    }
    for i in start_index..end_index {
        if let Some(child) = node.child(i) {
            add_node(child.clone(), target);
        }
    }
    if let Some(end) = end
        && end.depth() == depth
        && end.text_offset() > 0
        && let Some(before) = end.node_before()
    {
        add_node(before, target);
    }
}

fn close(node: &Node, content: Fragment) -> Result<Node, ReplaceError> {
    if !node
        .typ()
        .content_match()
        .match_fragment(&content)
        .is_some_and(|m| m.valid_end())
    {
        return Err(ReplaceError::InvalidContent {
            node: node.type_name().to_owned(),
            found: content
                .iter()
                .map(Node::type_name)
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    Ok(node.copy(content))
}

fn replace_three_way(
    from: &ResolvedPos,
    start: &ResolvedPos,
    end: &ResolvedPos,
    to: &ResolvedPos,
    depth: usize,
) -> Result<Fragment, ReplaceError> {
    let open_start = if from.depth() > depth {
        Some(joinable(from, start, depth + 1)?)
    } else {
        None
    };
    let open_end = if to.depth() > depth {
        Some(joinable(end, to, depth + 1)?)
    } else {
        None
    };

    let mut content: Vec<Node> = Vec::new();
    add_range(None, Some(from), depth, &mut content);

    match (&open_start, &open_end) {
        // One node rebuilt from all three pieces.
        (Some(a), Some(b)) if start.index(depth) == end.index(depth) => {
            check_join(a, b)?;
            let inner = replace_three_way(from, start, end, to, depth + 1)?;
            add_node(close(a, inner)?, &mut content);
        }
        _ => {
            if let Some(a) = &open_start {
                let inner = replace_two_way(from, start, depth + 1)?;
                add_node(close(a, inner)?, &mut content);
            }
            add_range(Some(start), Some(end), depth, &mut content);
            if let Some(b) = &open_end {
                let inner = replace_two_way(end, to, depth + 1)?;
                add_node(close(b, inner)?, &mut content);
            }
        }
    }

    add_range(Some(to), None, depth, &mut content);
    Ok(Fragment::from_vec(content))
}

fn replace_two_way(
    from: &ResolvedPos,
    to: &ResolvedPos,
    depth: usize,
) -> Result<Fragment, ReplaceError> {
    let mut content: Vec<Node> = Vec::new();
    add_range(None, Some(from), depth, &mut content);
    if from.depth() > depth {
        let typ = joinable(from, to, depth + 1)?;
        let inner = replace_two_way(from, to, depth + 1)?;
        add_node(close(&typ, inner)?, &mut content);
    }
    add_range(Some(to), None, depth, &mut content);
    Ok(Fragment::from_vec(content))
}

impl Node {
    /// Replaces `from..to` with `slice`.
    ///
    /// # Errors
    ///
    /// [`ReplaceError`] when the slice cannot be joined into the range.
    ///
    /// # Panics
    ///
    /// If either position is outside the node.
    pub fn replace(&self, from: usize, to: usize, slice: &Slice) -> Result<Self, ReplaceError> {
        replace(&self.resolve(from), &self.resolve(to), slice)
    }

    /// Deletes `from..to`.
    ///
    /// # Errors
    ///
    /// [`ReplaceError`] when the two sides cannot be joined.
    ///
    /// # Panics
    ///
    /// If either position is outside the node.
    pub fn delete(&self, from: usize, to: usize) -> Result<Self, ReplaceError> {
        self.replace(from, to, &Slice::empty())
    }
}
