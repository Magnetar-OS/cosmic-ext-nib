// SPDX-License-Identifier: MPL-2.0

//! Plugins: state that is not the document, and a say in what happens.
//!
//! Three things a plugin can do, and no more:
//!
//! - **Keep a value alongside the document**, updated by every transaction.
//!   The undo history is one; a table's cell selection is another; so is "the
//!   syntax tree of every code block", which is what makes highlighting a
//!   plugin rather than a special case in the view.
//! - **Refuse a transaction** it does not want applied.
//! - **Append a transaction** in response to one — how a rule that
//!   auto-numbers list items, or one that keeps a table rectangular, works
//!   without every command having to remember to do it.
//!
//! What a plugin deliberately *cannot* do here is draw. Handlers for key
//! presses and pointer events, and the decorations that style a range, belong
//! to the view, which is a different crate with a different vocabulary. A
//! plugin that needs both is two objects sharing a [`PluginKey`].

use std::any::Any;
use std::sync::Arc;

use crate::node::Node;
use crate::state::selection::Selection;
use crate::state::{EditorState, Transaction};

/// The name a plugin's state is looked up by.
///
/// A `&'static str` rather than a generated id, because plugin state is read
/// by code that did not create the plugin — a command asking the history
/// whether there is anything to undo — and a name is what such code can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginKey(pub &'static str);

impl PluginKey {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for PluginKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// A plugin's state value, type-erased.
///
/// Erased because the state holds a heterogeneous list of them and cannot be
/// generic over every plugin an application installs. Recovered by
/// [`EditorState::plugin_state`], which downcasts.
pub type FieldValue = Arc<dyn Any + Send + Sync>;

/// How a plugin's value is created and kept up to date.
pub trait StateField: Send + Sync {
    /// The initial value, for a fresh state.
    fn init(&self, doc: &Node, selection: &Selection) -> FieldValue;

    /// The value after a transaction.
    ///
    /// `new_doc` and `new_selection` are what the state is becoming; the old
    /// state is available whole. Both are given because the new state does not
    /// exist yet — it is being built out of these calls.
    fn apply(
        &self,
        tr: &Transaction,
        value: &FieldValue,
        old: &EditorState,
        new_doc: &Node,
        new_selection: &Selection,
    ) -> FieldValue;
}

/// Decides whether a transaction may be applied.
pub type FilterTransaction = Box<dyn Fn(&Transaction, &EditorState) -> bool + Send + Sync>;

/// Produces a follow-up transaction in response to ones just applied.
pub type AppendTransaction =
    Box<dyn Fn(&[Transaction], &EditorState, &EditorState) -> Option<Transaction> + Send + Sync>;

/// One plugin.
pub struct Plugin {
    key: PluginKey,
    state: Option<Box<dyn StateField>>,
    filter: Option<FilterTransaction>,
    append: Option<AppendTransaction>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("key", &self.key)
            .field("has_state", &self.state.is_some())
            .field("filters", &self.filter.is_some())
            .field("appends", &self.append.is_some())
            .finish()
    }
}

impl Plugin {
    #[must_use]
    pub fn new(key: PluginKey) -> Self {
        Self {
            key,
            state: None,
            filter: None,
            append: None,
        }
    }

    #[must_use]
    pub fn with_state(mut self, field: impl StateField + 'static) -> Self {
        self.state = Some(Box::new(field));
        self
    }

    #[must_use]
    pub fn filtering(
        mut self,
        f: impl Fn(&Transaction, &EditorState) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.filter = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn appending(
        mut self,
        f: impl Fn(&[Transaction], &EditorState, &EditorState) -> Option<Transaction>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.append = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn key(&self) -> PluginKey {
        self.key
    }

    #[must_use]
    pub fn state_field(&self) -> Option<&dyn StateField> {
        self.state.as_deref()
    }

    #[must_use]
    pub fn filter(&self) -> Option<&FilterTransaction> {
        self.filter.as_ref()
    }

    #[must_use]
    pub fn append(&self) -> Option<&AppendTransaction> {
        self.append.as_ref()
    }
}
