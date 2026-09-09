// SPDX-License-Identifier: MPL-2.0

//! The atomic changes.
//!
//! # Why a closed set
//!
//! ProseMirror lets applications define their own step types. This does not,
//! and the reason is serialisation and rebasing: every peer in a collaborative
//! session, and every reader of a stored history, must be able to apply and
//! invert every step it receives. A step type only one client knows is a step
//! the others must refuse, and a refusal in the middle of a rebase is not
//! recoverable.
//!
//! The set below is closed and complete for the same reason ProseMirror's core
//! set is: [`Step::ReplaceAround`] can express any structural change — wrap,
//! lift, split, join, change a block's type — as one atomic, invertible
//! operation. An extension that wants a new *edit* writes a command that emits
//! these; it does not need a new step.

use std::sync::Arc;

use crate::attrs::Value;
use crate::fragment::Fragment;
use crate::mark::Mark;
use crate::node::Node;
use crate::replace::ReplaceError;
use crate::slice::Slice;
use crate::transform::map::{Mapping, StepMap};

/// Why a step did not apply.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum StepError {
    #[error(transparent)]
    Replace(#[from] ReplaceError),
    #[error("a structural replace would overwrite content between {from} and {to}")]
    WouldOverwrite { from: usize, to: usize },
    #[error("no node at position {0}")]
    NoNodeAt(usize),
    #[error("the gap between {from} and {to} is not a flat range")]
    GapNotFlat { from: usize, to: usize },
    #[error("the surrounding content does not fit around the gap")]
    GapDoesNotFit,
    #[error("position {pos} is outside a document of {size}")]
    OutOfRange { pos: usize, size: usize },
}

/// One atomic change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Replaces `from..to` with a slice.
    Replace {
        from: usize,
        to: usize,
        slice: Slice,
        /// When set, the step refuses rather than deleting content it did not
        /// expect to be there. Structural steps — lift, join, split — set it,
        /// because a lift that silently swallowed a paragraph someone else
        /// inserted is worse than a lift that does not happen.
        structure: bool,
    },
    /// Replaces `from..to` while keeping `gap_from..gap_to` and re-inserting
    /// it at offset `insert` within the slice.
    ///
    /// The one step that can change what surrounds a range without touching
    /// the range: wrapping paragraphs in a blockquote, lifting a list item
    /// out, turning a paragraph into a heading. Doing it as one step is what
    /// makes it invert cleanly and rebase against a concurrent edit *inside*
    /// the gap.
    ReplaceAround {
        from: usize,
        to: usize,
        gap_from: usize,
        gap_to: usize,
        slice: Slice,
        insert: usize,
        structure: bool,
    },
    /// Adds a mark to the inline content in `from..to`.
    AddMark { from: usize, to: usize, mark: Mark },
    /// Removes a mark from the inline content in `from..to`.
    RemoveMark { from: usize, to: usize, mark: Mark },
    /// Adds a mark to the single node at `pos` — for marks on block or atom
    /// nodes rather than on text.
    AddNodeMark { pos: usize, mark: Mark },
    /// Removes a mark from the single node at `pos`.
    RemoveNodeMark { pos: usize, mark: Mark },
    /// Sets one attribute on the node at `pos`.
    SetAttr {
        pos: usize,
        attr: Arc<str>,
        value: Value,
    },
    /// Sets one attribute on the document node itself.
    SetDocAttr { attr: Arc<str>, value: Value },
}

impl Step {
    /// A plain replacement.
    #[must_use]
    pub fn replace(from: usize, to: usize, slice: Slice) -> Self {
        Self::Replace {
            from,
            to,
            slice,
            structure: false,
        }
    }

    /// A replacement that refuses rather than destroying content between its
    /// ends.
    #[must_use]
    pub fn replace_structure(from: usize, to: usize, slice: Slice) -> Self {
        Self::Replace {
            from,
            to,
            slice,
            structure: true,
        }
    }

    /// Applies the step.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the step does not apply to this document.
    pub fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        match self {
            Self::Replace {
                from,
                to,
                slice,
                structure,
            } => {
                if *structure && content_between(doc, *from, *to) {
                    return Err(StepError::WouldOverwrite {
                        from: *from,
                        to: *to,
                    });
                }
                Ok(doc.replace(*from, *to, slice)?)
            }

            Self::ReplaceAround {
                from,
                to,
                gap_from,
                gap_to,
                slice,
                insert,
                structure,
            } => {
                if *structure
                    && (content_between(doc, *from, *gap_from)
                        || content_between(doc, *gap_to, *to))
                {
                    return Err(StepError::WouldOverwrite {
                        from: *from,
                        to: *to,
                    });
                }
                let gap = doc.slice(*gap_from, *gap_to, false);
                if gap.open_start() > 0 || gap.open_end() > 0 {
                    return Err(StepError::GapNotFlat {
                        from: *gap_from,
                        to: *gap_to,
                    });
                }
                let inserted = slice
                    .insert_at(*insert, gap.content().clone())
                    .ok_or(StepError::GapDoesNotFit)?;
                Ok(doc.replace(*from, *to, &inserted)?)
            }

            Self::AddMark { from, to, mark } => {
                Ok(map_marks(doc, *from, *to, &|marks| mark.add_to_set(marks), mark)?)
            }
            Self::RemoveMark { from, to, mark } => Ok(map_marks(
                doc,
                *from,
                *to,
                &|marks| mark.remove_from_set(marks),
                mark,
            )?),

            Self::AddNodeMark { pos, mark } => {
                let node = doc.node_at(*pos).ok_or(StepError::NoNodeAt(*pos))?;
                Ok(replace_node_shell(
                    doc,
                    *pos,
                    node.with_marks(mark.add_to_set(node.marks())),
                )?)
            }
            Self::RemoveNodeMark { pos, mark } => {
                let node = doc.node_at(*pos).ok_or(StepError::NoNodeAt(*pos))?;
                Ok(replace_node_shell(
                    doc,
                    *pos,
                    node.with_marks(mark.remove_from_set(node.marks())),
                )?)
            }

            Self::SetAttr { pos, attr, value } => {
                let node = doc.node_at(*pos).ok_or(StepError::NoNodeAt(*pos))?;
                Ok(replace_node_shell(
                    doc,
                    *pos,
                    node.with_attrs(node.attrs().set(Arc::clone(attr), value.clone())),
                )?)
            }
            Self::SetDocAttr { attr, value } => {
                Ok(doc.with_attrs(doc.attrs().set(Arc::clone(attr), value.clone())))
            }
        }
    }

    /// How this step moves every position in the document.
    #[must_use]
    pub fn step_map(&self) -> StepMap {
        match self {
            Self::Replace {
                from, to, slice, ..
            } => StepMap::single(*from, to - from, slice.size()),
            Self::ReplaceAround {
                from,
                to,
                gap_from,
                gap_to,
                slice,
                insert,
                ..
            } => StepMap::pair(
                (*from, gap_from - from, *insert),
                (*gap_to, to - gap_to, slice.size() - insert),
            ),
            // Marks and attributes change no position.
            _ => StepMap::empty(),
        }
    }

    /// The step that undoes this one, against the document it applied to.
    ///
    /// # Panics
    ///
    /// If `doc` is not the document this step applied to — the node the step
    /// referred to must still be where it was.
    #[must_use]
    pub fn invert(&self, doc: &Node) -> Self {
        match self {
            Self::Replace {
                from,
                to,
                slice,
                structure,
            } => Self::Replace {
                from: *from,
                to: from + slice.size(),
                slice: doc.slice(*from, *to, false),
                structure: *structure,
            },

            Self::ReplaceAround {
                from,
                to,
                gap_from,
                gap_to,
                slice,
                insert,
                structure,
            } => {
                let gap = gap_to - gap_from;
                let removed = doc
                    .slice(*from, *to, false)
                    .remove_between(gap_from - from, gap_to - from)
                    .expect("the gap of an applied gap-replace is flat by construction");
                Self::ReplaceAround {
                    from: *from,
                    to: from + slice.size() + gap,
                    gap_from: from + insert,
                    gap_to: from + insert + gap,
                    slice: removed,
                    insert: gap_from - from,
                    structure: *structure,
                }
            }

            Self::AddMark { from, to, mark } => Self::RemoveMark {
                from: *from,
                to: *to,
                mark: mark.clone(),
            },
            Self::RemoveMark { from, to, mark } => Self::AddMark {
                from: *from,
                to: *to,
                mark: mark.clone(),
            },

            Self::AddNodeMark { pos, mark } => {
                let node = doc
                    .node_at(*pos)
                    .expect("inverting against the document the step applied to");
                let after = mark.add_to_set(node.marks());
                // Adding a mark can *replace* one — a second link on the same
                // text. Then the undo is not a removal but a restoration of
                // the one that was displaced.
                if after.len() == node.marks().len() {
                    let displaced = node
                        .marks()
                        .iter()
                        .find(|m| !m.is_in_set(&after))
                        .unwrap_or(mark);
                    return Self::AddNodeMark {
                        pos: *pos,
                        mark: displaced.clone(),
                    };
                }
                Self::RemoveNodeMark {
                    pos: *pos,
                    mark: mark.clone(),
                }
            }
            Self::RemoveNodeMark { pos, mark } => Self::AddNodeMark {
                pos: *pos,
                mark: mark.clone(),
            },

            Self::SetAttr { pos, attr, .. } => {
                let node = doc
                    .node_at(*pos)
                    .expect("inverting against the document the step applied to");
                Self::SetAttr {
                    pos: *pos,
                    attr: Arc::clone(attr),
                    value: node.attrs().get(attr).cloned().unwrap_or(Value::Null),
                }
            }
            Self::SetDocAttr { attr, .. } => Self::SetDocAttr {
                attr: Arc::clone(attr),
                value: doc.attrs().get(attr).cloned().unwrap_or(Value::Null),
            },
        }
    }

    /// The equivalent step in a document that `mapping`'s changes have already
    /// been applied to, or `None` when the step no longer has anything to act
    /// on.
    #[must_use]
    pub fn map(&self, mapping: &Mapping) -> Option<Self> {
        match self {
            Self::Replace {
                from,
                to,
                slice,
                structure,
            } => {
                let start = mapping.map_result(*from, 1);
                let end = mapping.map_result(*to, -1);
                if start.deleted_across() && end.deleted_across() {
                    return None;
                }
                Some(Self::Replace {
                    from: start.pos(),
                    to: end.pos().max(start.pos()),
                    slice: slice.clone(),
                    structure: *structure,
                })
            }

            Self::ReplaceAround {
                from,
                to,
                gap_from,
                gap_to,
                slice,
                insert,
                structure,
            } => {
                let start = mapping.map_result(*from, 1);
                let end = mapping.map_result(*to, -1);
                let mapped_gap_from = if from == gap_from {
                    start.pos()
                } else {
                    mapping.map(*gap_from, -1)
                };
                let mapped_gap_to = if to == gap_to {
                    end.pos()
                } else {
                    mapping.map(*gap_to, 1)
                };
                if (start.deleted_across() && end.deleted_across())
                    || mapped_gap_from < start.pos()
                    || mapped_gap_to > end.pos()
                {
                    return None;
                }
                Some(Self::ReplaceAround {
                    from: start.pos(),
                    to: end.pos(),
                    gap_from: mapped_gap_from,
                    gap_to: mapped_gap_to,
                    slice: slice.clone(),
                    insert: *insert,
                    structure: *structure,
                })
            }

            Self::AddMark { from, to, mark } | Self::RemoveMark { from, to, mark } => {
                let start = mapping.map_result(*from, 1);
                let end = mapping.map_result(*to, -1);
                // The range survives only if something is left of it.
                if (start.deleted() && end.deleted()) || start.pos() >= end.pos() {
                    return None;
                }
                let (from, to, mark) = (start.pos(), end.pos(), mark.clone());
                Some(match self {
                    Self::AddMark { .. } => Self::AddMark { from, to, mark },
                    _ => Self::RemoveMark { from, to, mark },
                })
            }

            Self::AddNodeMark { pos, mark } | Self::RemoveNodeMark { pos, mark } => {
                let mapped = mapping.map_result(*pos, 1);
                if mapped.deleted_after() {
                    return None;
                }
                let (pos, mark) = (mapped.pos(), mark.clone());
                Some(match self {
                    Self::AddNodeMark { .. } => Self::AddNodeMark { pos, mark },
                    _ => Self::RemoveNodeMark { pos, mark },
                })
            }

            Self::SetAttr { pos, attr, value } => {
                let mapped = mapping.map_result(*pos, 1);
                if mapped.deleted_after() {
                    return None;
                }
                Some(Self::SetAttr {
                    pos: mapped.pos(),
                    attr: Arc::clone(attr),
                    value: value.clone(),
                })
            }
            // The document node cannot move.
            Self::SetDocAttr { .. } => Some(self.clone()),
        }
    }

    /// Combines two adjacent steps into one, when they are the kind that
    /// combine.
    ///
    /// This is what keeps a burst of typing from becoming one history entry
    /// per character *in the steps themselves*, independently of how the
    /// history groups events.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Option<Self> {
        let (
            Self::Replace {
                from: a_from,
                to: a_to,
                slice: a,
                structure: false,
            },
            Self::Replace {
                from: b_from,
                to: b_to,
                slice: b,
                structure: false,
            },
        ) = (self, other)
        else {
            return None;
        };

        // `other` continues where `self` left off.
        if a_from + a.size() == *b_from && a.open_end() == 0 && b.open_start() == 0 {
            let slice = if a.size() + b.size() == 0 {
                Slice::empty()
            } else {
                Slice::new(a.content().append(b.content()), a.open_start(), b.open_end())
            };
            return Some(Self::replace(*a_from, a_to + (b_to - b_from), slice));
        }
        // `other` ends where `self` begins — backspacing.
        if b_to == a_from && a.open_start() == 0 && b.open_end() == 0 {
            let slice = if a.size() + b.size() == 0 {
                Slice::empty()
            } else {
                Slice::new(b.content().append(a.content()), b.open_start(), a.open_end())
            };
            return Some(Self::replace(*b_from, *a_to, slice));
        }
        None
    }
}

/// Replaces just the opening boundary of the node at `pos`, leaving its
/// content in place.
///
/// The trick that lets a node's marks or attributes change without rewriting
/// its subtree: replace the single position the node's opening occupies with a
/// new, empty node whose right edge is open, and the original content flows
/// back in.
fn replace_node_shell(doc: &Node, pos: usize, updated: Node) -> Result<Node, ReplaceError> {
    let open_end = usize::from(!updated.is_leaf());
    let shell = Node::new(
        Arc::clone(updated.typ()),
        updated.attrs().clone(),
        Fragment::empty(),
        updated.marks().clone(),
    );
    doc.replace(pos, pos + 1, &Slice::new(Fragment::from(shell), 0, open_end))
}

/// Applies a mark change to every inline node in a range whose parent allows
/// that mark type.
fn map_marks(
    doc: &Node,
    from: usize,
    to: usize,
    f: &dyn Fn(&crate::mark::Marks) -> crate::mark::Marks,
    mark: &Mark,
) -> Result<Node, ReplaceError> {
    let old = doc.slice(from, to, false);
    let at = doc.resolve(from);
    let parent = at.node(at.shared_depth(to)).clone();
    let content = map_fragment(old.content(), &parent, &mut |node, parent| {
        if !node.is_inline() || !parent.typ().allows_mark_type(mark.typ().id()) {
            return node.clone();
        }
        node.with_marks(f(node.marks()))
    });
    doc.replace(
        from,
        to,
        &Slice::new(content, old.open_start(), old.open_end()),
    )
}

fn map_fragment(
    fragment: &Fragment,
    parent: &Node,
    f: &mut dyn FnMut(&Node, &Node) -> Node,
) -> Fragment {
    let mut mapped = Vec::with_capacity(fragment.child_count());
    for child in fragment.iter() {
        let child = if child.content_size() > 0 {
            let inner = map_fragment(child.content(), child, f);
            child.copy(inner)
        } else {
            child.clone()
        };
        mapped.push(if child.is_inline() {
            f(&child, parent)
        } else {
            child
        });
    }
    Fragment::from_vec(mapped)
}

/// True when `from..to` spans anything but node boundaries — used by
/// structural steps to check that the range they are about to collapse really
/// is empty of content.
fn content_between(doc: &Node, from: usize, to: usize) -> bool {
    let at = doc.resolve(from);
    let mut dist = to - from;
    let mut depth = at.depth();
    while dist > 0 && depth > 0 && at.index_after(depth) == at.node(depth).child_count() {
        depth -= 1;
        dist -= 1;
    }
    if dist > 0 {
        let mut next = at.node(depth).child(at.index_after(depth)).cloned();
        while dist > 0 {
            let Some(node) = next else { return true };
            if node.is_leaf() {
                return true;
            }
            next = node.first_child().cloned();
            dist -= 1;
        }
    }
    false
}
