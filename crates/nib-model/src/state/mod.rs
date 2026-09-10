// SPDX-License-Identifier: MPL-2.0

//! The editor's state, and the transactions that change it.
//!
//! [`EditorState`] is a value: a document, a selection, the marks pending
//! typing will carry, and each plugin's state. Applying a [`Transaction`]
//! produces a *new* state rather than mutating one, which is what makes undo a
//! matter of keeping old values and makes a view's redraw check a comparison
//! rather than a traversal.

pub mod plugin;
pub mod selection;
pub mod transaction;

pub use plugin::{FieldValue, Plugin, PluginKey, StateField};
pub use selection::{Kind as SelectionKind, Selection};
pub use transaction::Transaction;

use std::sync::Arc;

use crate::mark::Marks;
use crate::node::Node;
use crate::schema::Schema;

/// How many rounds of `append_transaction` are allowed before the state gives
/// up.
///
/// Plugins that respond to each other's transactions can in principle ping-pong
/// forever. `ProseMirror` relies on them not doing so; a bound turns a hang into
/// a visible, debuggable stop.
const MAX_APPEND_ROUNDS: usize = 32;

/// The editor's whole state.
#[derive(Clone)]
pub struct EditorState {
    schema: Schema,
    doc: Node,
    selection: Selection,
    stored_marks: Option<Marks>,
    plugins: Arc<Vec<Plugin>>,
    /// One slot per plugin, in the same order.
    fields: Vec<Option<FieldValue>>,
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("doc", &self.doc.to_string())
            .field("selection", &self.selection)
            .field("stored_marks", &self.stored_marks)
            .field("plugins", &self.plugins.len())
            .finish_non_exhaustive()
    }
}

impl EditorState {
    /// A state over `doc`, with the caret at the first place one can be.
    #[must_use]
    pub fn new(schema: Schema, doc: Node) -> Self {
        let selection = Selection::at_start(&doc);
        Self::with_selection(schema, doc, selection, Vec::new())
    }

    /// A state over an empty document of `schema`.
    #[must_use]
    pub fn empty(schema: Schema) -> Self {
        let doc = schema.empty_doc();
        Self::new(schema, doc)
    }

    /// A state with an explicit selection and a set of plugins.
    #[must_use]
    pub fn with_selection(
        schema: Schema,
        doc: Node,
        selection: Selection,
        plugins: Vec<Plugin>,
    ) -> Self {
        let mut state = Self {
            schema,
            doc,
            selection,
            stored_marks: None,
            plugins: Arc::new(plugins),
            fields: Vec::new(),
        };
        state.fields = state
            .plugins
            .iter()
            .map(|p| {
                p.state_field()
                    .map(|f| f.init(&state.doc, &state.selection))
            })
            .collect();
        state
    }

    /// The same state with `plugins` installed, their fields initialised.
    #[must_use]
    pub fn with_plugins(&self, plugins: Vec<Plugin>) -> Self {
        Self::with_selection(
            self.schema.clone(),
            self.doc.clone(),
            self.selection.clone(),
            plugins,
        )
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    #[must_use]
    pub fn doc(&self) -> &Node {
        &self.doc
    }

    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    #[must_use]
    pub fn stored_marks(&self) -> Option<&Marks> {
        self.stored_marks.as_ref()
    }

    #[must_use]
    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// The marks a character typed now would carry: the pending set if there
    /// is one, otherwise whatever the selection sits in.
    #[must_use]
    pub fn marks(&self) -> Marks {
        self.stored_marks
            .clone()
            .unwrap_or_else(|| self.selection.marks(&self.doc))
    }

    /// A plugin's state, if the plugin is installed and its value is a `T`.
    #[must_use]
    pub fn plugin_state<T: std::any::Any + Send + Sync>(&self, key: PluginKey) -> Option<&T> {
        let index = self.plugins.iter().position(|p| p.key() == key)?;
        self.fields.get(index)?.as_ref()?.downcast_ref::<T>()
    }

    /// A fresh transaction over this state.
    #[must_use]
    pub fn tr(&self) -> Transaction {
        Transaction::new(
            self.schema.clone(),
            self.doc.clone(),
            self.selection.clone(),
            self.stored_marks.clone(),
        )
    }

    /// Applies a transaction, giving the new state and every transaction that
    /// went into it — the one given, plus whatever plugins appended.
    ///
    /// Returns `None` when a plugin refused the transaction.
    #[must_use]
    pub fn apply(&self, tr: Transaction) -> Option<(Self, Vec<Transaction>)> {
        if !self.allows(&tr, self.plugins.len()) {
            return None;
        }
        let mut applied = vec![tr];
        let mut state = self.apply_inner(&applied[0]);

        for _ in 0..MAX_APPEND_ROUNDS {
            let mut appended = false;
            for (i, plugin) in self.plugins.iter().enumerate() {
                let Some(append) = plugin.append() else {
                    continue;
                };
                let Some(extra) = append(&applied, self, &state) else {
                    continue;
                };
                if !state.allows(&extra, i) {
                    continue;
                }
                state = state.apply_inner(&extra);
                applied.push(extra);
                appended = true;
            }
            if !appended {
                break;
            }
        }
        Some((state, applied))
    }

    /// Applies a transaction, discarding the record of what plugins appended.
    ///
    /// Returns the state unchanged when a plugin refused it.
    #[must_use]
    pub fn applied(&self, tr: Transaction) -> Self {
        self.apply(tr).map_or_else(|| self.clone(), |(s, _)| s)
    }

    /// True when no plugin before index `limit` refuses the transaction.
    fn allows(&self, tr: &Transaction, limit: usize) -> bool {
        self.plugins
            .iter()
            .take(limit)
            .all(|p| p.filter().is_none_or(|f| f(tr, self)))
    }

    /// The state after one transaction, before any plugin appends to it.
    fn apply_inner(&self, tr: &Transaction) -> Self {
        let doc = tr.doc().clone();
        // A transaction that set the selection deliberately has already put it
        // where it belongs; one that only changed the document has had it
        // carried along.
        let selection = tr.selection().clone();
        let stored_marks = if tr.stored_marks_were_set() {
            tr.stored_marks().cloned()
        } else if tr.doc_changed() {
            None
        } else {
            self.stored_marks.clone()
        };

        let fields = self
            .plugins
            .iter()
            .zip(&self.fields)
            .map(|(plugin, value)| match (plugin.state_field(), value) {
                (Some(field), Some(value)) => Some(field.apply(tr, value, self, &doc, &selection)),
                (Some(field), None) => Some(field.init(&doc, &selection)),
                (None, _) => None,
            })
            .collect();

        Self {
            schema: self.schema.clone(),
            doc,
            selection,
            stored_marks,
            plugins: Arc::clone(&self.plugins),
            fields,
        }
    }
}
