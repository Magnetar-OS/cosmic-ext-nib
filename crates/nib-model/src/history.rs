// SPDX-License-Identifier: MPL-2.0

//! Undo and redo.
//!
//! # There is no undo mechanism
//!
//! There is only [`Step::invert`](crate::transform::Step::invert), which
//! already exists because collaborative editing needs it. The history is
//! bookkeeping over that: a list of inverted steps, cut into events, with the
//! selection each event started from.
//!
//! # What makes it feel right
//!
//! Two things, and both are about *grouping*, because a history that undoes
//! one keystroke at a time is technically correct and unusable:
//!
//! - **Time.** Changes arriving within [`Options::new_group_delay`] of one
//!   another join the current event. The clock comes from the transaction, not
//!   from this module — see [`Transaction::at`](crate::Transaction::at).
//! - **Adjacency.** Even within the delay, a change somewhere else in the
//!   document starts a new event. Typing a word, then clicking elsewhere and
//!   typing another, is two undos however fast the typing was.
//!
//! # Changes that are not the user's
//!
//! A transaction marked `addToHistory: false` — a remote edit, a
//! decoration-only change — is not added to either branch, but its position
//! map *is*, so the inverted steps already on the stack still point at the
//! right places afterwards. That is why an [`Item`] can hold a map with no
//! step.

use std::sync::Arc;

use crate::node::Node;
use crate::state::plugin::{FieldValue, StateField};
use crate::state::{EditorState, Plugin, PluginKey, Selection, Transaction};
use crate::transform::{Mapping, Step, StepMap};

/// The key the history's state is stored under.
pub const KEY: PluginKey = PluginKey::new("history");

/// Set on a transaction to keep it out of the history, while still letting the
/// history follow the positions it moved.
pub const ADD_TO_HISTORY: &str = "addToHistory";

/// Set on a transaction to end the current undo event, so the next change
/// starts a new one.
pub const CLOSE_HISTORY: &str = "closeHistory";

/// Set by [`undo`] and [`redo`], carrying the history state the transaction
/// produces.
///
/// Undo is the one change the history cannot work out from the transaction
/// alone: both branches move at once, and only the command that popped the
/// event knows what is left. So it computes the whole new state and attaches
/// it, and [`apply`] takes it as given.
const HISTORY_STATE: &str = "historyState";

/// How the history groups and how much it keeps.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// How many events to keep. Older ones are dropped.
    pub depth: usize,
    /// Changes further apart than this many milliseconds start a new event.
    pub new_group_delay: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            depth: 100,
            new_group_delay: 500,
        }
    }
}

/// One entry: how a step moved positions, the step that undoes it, and — for
/// the entry that begins an event — the selection to restore.
#[derive(Debug, Clone)]
struct Item {
    map: StepMap,
    /// `None` for an entry that only records a change the history did not
    /// make, kept so earlier steps still map correctly.
    step: Option<Step>,
    /// `Some` marks the start of an undo event.
    selection: Option<Selection>,
}

impl Item {
    /// Combines this entry with one that follows it, when the two are a
    /// continuation of the same edit.
    fn merge(&self, next: &Self) -> Option<Self> {
        let (Some(mine), Some(theirs)) = (&self.step, &next.step) else {
            return None;
        };
        if next.selection.is_some() {
            return None;
        }
        // The entries hold *inverted* steps, so the later one comes first.
        let merged = theirs.merge(mine)?;
        Some(Self {
            map: merged.step_map().invert(),
            step: Some(merged),
            selection: self.selection.clone(),
        })
    }
}

/// One direction of the history.
#[derive(Debug, Clone, Default)]
struct Branch {
    items: Vec<Item>,
    event_count: usize,
}

impl Branch {
    fn is_empty(&self) -> bool {
        self.event_count == 0
    }

    /// Adds a transform's steps, inverted. `selection` being `Some` starts a
    /// new event.
    fn add(
        &self,
        tr: &Transaction,
        selection: Option<Selection>,
        options: Options,
    ) -> Self {
        let mut items = self.items.clone();
        let mut event_count = self.event_count;
        let mut selection = selection;

        for (i, step) in tr.steps().iter().enumerate() {
            let item = Item {
                map: tr.mapping().maps()[i].clone(),
                step: Some(step.invert(&tr.docs()[i])),
                selection: selection.take(),
            };
            let starts_event = item.selection.is_some();
            match items.last().and_then(|last| last.merge(&item)) {
                Some(merged) => {
                    items.pop();
                    items.push(merged);
                }
                None => items.push(item),
            }
            if starts_event {
                event_count += 1;
            }
        }

        let mut branch = Self { items, event_count };
        branch.trim(options.depth);
        branch
    }

    /// Adds maps with no steps: the history did not make these changes, but it
    /// must follow them.
    fn add_maps(&self, maps: &[StepMap]) -> Self {
        if self.is_empty() {
            return self.clone();
        }
        let mut items = self.items.clone();
        items.extend(maps.iter().map(|map| Item {
            map: map.clone(),
            step: None,
            selection: None,
        }));
        Self {
            items,
            event_count: self.event_count,
        }
    }

    /// Drops the oldest events until only `depth` remain.
    fn trim(&mut self, depth: usize) {
        if self.event_count <= depth {
            return;
        }
        let excess = self.event_count - depth;
        let mut seen = 0;
        let mut cut = 0;
        for (i, item) in self.items.iter().enumerate() {
            if item.selection.is_some() {
                seen += 1;
                if seen > excess {
                    cut = i;
                    break;
                }
            }
        }
        self.items.drain(..cut);
        self.event_count -= excess;
    }

    /// Takes the most recent event off, as a transaction that undoes it.
    ///
    /// Returns the remaining branch, the transaction, and the selection the
    /// event started from.
    fn pop_event(&self, state: &EditorState) -> Option<(Self, Transaction, Option<Selection>)> {
        if self.is_empty() {
            return None;
        }
        let mut tr = state.tr();
        let mut selection = None;
        let mut cut = 0;
        // Steps not in the history sit between ours and have to be mapped
        // through; `remap` accumulates them as we walk back.
        let mut remap = Mapping::new();
        let mut remapping = false;

        for i in (0..self.items.len()).rev() {
            let item = &self.items[i];
            match &item.step {
                None => {
                    remap.append_map(item.map.invert(), None);
                    remapping = true;
                }
                Some(step) => {
                    let step = if remapping {
                        if let Some(mapped) = step.map(&remap) { mapped } else {
                            if item.selection.is_some() {
                                selection.clone_from(&item.selection);
                                cut = i;
                                break;
                            }
                            continue;
                        }
                    } else {
                        step.clone()
                    };
                    let _ = tr.step(step);
                }
            }
            if item.selection.is_some() {
                selection.clone_from(&item.selection);
                cut = i;
                break;
            }
        }

        let remaining = Self {
            items: self.items[..cut].to_vec(),
            event_count: self.event_count - 1,
        };
        Some((remaining, tr, selection))
    }
}

/// The history's state.
#[derive(Debug, Clone, Default)]
pub struct History {
    done: Branch,
    undone: Branch,
    /// The ranges the previous change touched, for the adjacency test.
    prev_ranges: Option<Vec<(usize, usize)>>,
    prev_time: u64,
    options: Options,
}

impl History {
    /// How many undo steps are available.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.done.event_count
    }

    /// How many redo steps are available.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.undone.event_count
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}

/// Which branch a history transaction is coming from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

struct HistoryField {
    options: Options,
}

impl StateField for HistoryField {
    fn init(&self, _doc: &Node, _selection: &Selection) -> FieldValue {
        Arc::new(History {
            options: self.options,
            ..History::default()
        })
    }

    fn apply(
        &self,
        tr: &Transaction,
        value: &FieldValue,
        old: &EditorState,
        _new_doc: &Node,
        _new_selection: &Selection,
    ) -> FieldValue {
        let history = value
            .downcast_ref::<History>()
            .cloned()
            .unwrap_or_default();
        Arc::new(apply(history, tr, old))
    }
}

fn apply(mut history: History, tr: &Transaction, state: &EditorState) -> History {
    // An undo or redo brings its own answer.
    if let Some(carried) = tr.meta::<History>(HISTORY_STATE) {
        return carried.clone();
    }

    if tr.has_meta(CLOSE_HISTORY) {
        history.prev_time = 0;
        history.prev_ranges = None;
    }

    if !tr.doc_changed() {
        return history;
    }

    // A change the history should follow but not own.
    if tr.meta::<bool>(ADD_TO_HISTORY) == Some(&false) {
        let maps = tr.mapping().maps();
        return History {
            done: history.done.add_maps(maps),
            undone: history.undone.add_maps(maps),
            prev_ranges: history
                .prev_ranges
                .map(|r| map_ranges(&r, tr.mapping())),
            ..history
        };
    }

    let new_group = history.prev_time == 0
        || tr.time() > history.prev_time.saturating_add(history.options.new_group_delay)
        || !is_adjacent_to(tr, history.prev_ranges.as_deref());

    let ranges = ranges_for(tr.mapping().maps().last());
    History {
        done: history.done.add(
            tr,
            new_group.then(|| state.selection().clone()),
            history.options,
        ),
        // Any new change abandons the redo branch: there is no longer one
        // future to redo into.
        undone: Branch::default(),
        prev_ranges: Some(ranges),
        prev_time: tr.time(),
        options: history.options,
    }
}

fn ranges_for(map: Option<&StepMap>) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    if let Some(map) = map {
        map.for_each(|_, _, new_from, new_to| ranges.push((new_from, new_to)));
    }
    ranges
}

fn map_ranges(ranges: &[(usize, usize)], mapping: &Mapping) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .map(|(from, to)| (mapping.map(*from, 1), mapping.map(*to, -1)))
        .collect()
}

/// True when this change touches the same part of the document as the last
/// one — the test that keeps typing here and typing there from becoming one
/// undo.
fn is_adjacent_to(tr: &Transaction, prev: Option<&[(usize, usize)]>) -> bool {
    let Some(prev) = prev else { return false };
    if !tr.doc_changed() {
        return true;
    }
    let Some(first) = tr.mapping().maps().first() else {
        return true;
    };
    let mut adjacent = false;
    first.for_each(|start, end, _, _| {
        if prev.iter().any(|(a, b)| start <= *b && end >= *a) {
            adjacent = true;
        }
    });
    adjacent
}

/// The history plugin.
#[must_use]
pub fn history(options: Options) -> Plugin {
    Plugin::new(KEY).with_state(HistoryField { options })
}

/// The transaction that undoes the most recent event, or `None` when there is
/// nothing to undo.
#[must_use]
pub fn undo(state: &EditorState) -> Option<Transaction> {
    step_history(state, Direction::Undo)
}

/// The transaction that redoes the most recently undone event, or `None`.
#[must_use]
pub fn redo(state: &EditorState) -> Option<Transaction> {
    step_history(state, Direction::Redo)
}

fn step_history(state: &EditorState, direction: Direction) -> Option<Transaction> {
    let history = state.plugin_state::<History>(KEY)?;
    let (source, target) = match direction {
        Direction::Undo => (&history.done, &history.undone),
        Direction::Redo => (&history.undone, &history.done),
    };
    let (remaining, mut tr, selection) = source.pop_event(state)?;
    if !tr.doc_changed() {
        return None;
    }

    // The event goes on the opposite branch, starting there — which is what
    // makes an undone change redoable, and a redone one undoable again.
    let added = target.add(&tr, Some(state.selection().clone()), history.options);
    let (done, undone) = match direction {
        Direction::Undo => (remaining, added),
        Direction::Redo => (added, remaining),
    };

    if let Some(selection) = selection {
        let doc = tr.doc().clone();
        let mapped = selection.map(&doc, &Mapping::new());
        tr.set_selection(mapped);
    }
    let new_history = History {
        done,
        undone,
        prev_ranges: None,
        prev_time: 0,
        options: history.options,
    };
    Some(tr.set_meta(HISTORY_STATE, new_history).scroll_into_view())
}
