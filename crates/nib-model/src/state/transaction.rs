// SPDX-License-Identifier: MPL-2.0

//! A proposed change to the editor's whole state.
//!
//! A [`Transform`] changes a document. A [`Transaction`] changes a document
//! *and* the selection, the marks that pending typing will carry, and whatever
//! plugins keep alongside — as one indivisible thing.
//!
//! That the two are separate matters. Undo groups by transaction, not by step,
//! so "make this bold" is one entry even though it emits a `RemoveMark` and an
//! `AddMark`. Plugins see transactions, so a plugin can veto or answer one
//! without having to reason about half-applied steps. And the selection is
//! carried *through* the change rather than recomputed after it, which is what
//! keeps the caret where the user left it.
//!
//! # Meta
//!
//! A transaction carries arbitrary tagged values that no step represents:
//! "this is an undo", "do not add this to the history", "this came from
//! pasting". Plugins read them to decide what a change *means*, which the
//! steps alone never say.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::attrs::Attrs;
use crate::fragment::Fragment;
use crate::mark::{Mark, Marks};
use crate::node::Node;
use crate::resolve::NodeRange;
use crate::schema::{NodeTypeId, Schema};
use crate::slice::Slice;
use crate::state::selection::{Kind, Selection};
use crate::transform::structure::{MarkFilter, TypeAndAttrs};
use crate::transform::{Mapping, Step, StepError, Transform};

/// A value attached to a transaction under a name.
pub type MetaValue = Arc<dyn Any + Send + Sync>;

/// A change to the editor state, not yet applied.
#[derive(Clone)]
pub struct Transaction {
    transform: Transform,
    selection: Selection,
    /// Set when the selection was chosen deliberately rather than carried
    /// along — so a plugin can tell "the user moved the caret" from "the caret
    /// moved because text was inserted before it".
    selection_set: bool,
    stored_marks: Option<Marks>,
    stored_marks_set: bool,
    meta: BTreeMap<&'static str, MetaValue>,
    scroll_into_view: bool,
    /// When this change happened, in milliseconds. Supplied by the caller
    /// rather than read from a clock here: the model has no business knowing
    /// what time it is, and a test that cannot control the clock cannot test
    /// how the history groups.
    time: u64,
}

impl std::fmt::Debug for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("steps", &self.transform.steps().len())
            .field("selection", &self.selection)
            .field("stored_marks", &self.stored_marks)
            .field("meta", &self.meta.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Transaction {
    #[must_use]
    pub fn new(
        schema: Schema,
        doc: Node,
        selection: Selection,
        stored_marks: Option<Marks>,
    ) -> Self {
        Self {
            transform: Transform::new(schema, doc),
            selection,
            selection_set: false,
            stored_marks,
            stored_marks_set: false,
            meta: BTreeMap::new(),
            scroll_into_view: false,
            time: 0,
        }
    }

    /// Stamps the transaction with a time in milliseconds. The history groups
    /// changes that arrive close together into one undo step, and this is what
    /// it goes by.
    #[must_use]
    pub fn at(mut self, time_ms: u64) -> Self {
        self.time = time_ms;
        self
    }

    /// Stamps the transaction with the system clock. A convenience for callers
    /// with no clock of their own; a view that already has a frame time should
    /// pass it to [`Transaction::at`] instead.
    #[must_use]
    pub fn now(self) -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        self.at(ms)
    }

    /// When this change happened, in milliseconds.
    #[must_use]
    pub fn time(&self) -> u64 {
        self.time
    }

    // -- reading -----------------------------------------------------------

    #[must_use]
    pub fn doc(&self) -> &Node {
        self.transform.doc()
    }

    #[must_use]
    pub fn before(&self) -> &Node {
        self.transform.before()
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        self.transform.schema()
    }

    #[must_use]
    pub fn steps(&self) -> &[Step] {
        self.transform.steps()
    }

    #[must_use]
    pub fn mapping(&self) -> &Mapping {
        self.transform.mapping()
    }

    #[must_use]
    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    #[must_use]
    pub fn doc_changed(&self) -> bool {
        self.transform.doc_changed()
    }

    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    #[must_use]
    pub fn selection_was_set(&self) -> bool {
        self.selection_set
    }

    #[must_use]
    pub fn stored_marks(&self) -> Option<&Marks> {
        self.stored_marks.as_ref()
    }

    #[must_use]
    pub fn stored_marks_were_set(&self) -> bool {
        self.stored_marks_set
    }

    #[must_use]
    pub fn wants_scroll_into_view(&self) -> bool {
        self.scroll_into_view
    }

    /// The steps of this transaction, inverted and reversed.
    #[must_use]
    pub fn invert_steps(&self) -> Vec<Step> {
        self.transform.invert()
    }

    /// The document each step applied to.
    #[must_use]
    pub fn docs(&self) -> &[Node] {
        self.transform.docs()
    }

    // -- meta --------------------------------------------------------------

    /// Attaches a value under `name`.
    #[must_use]
    pub fn set_meta<T: Any + Send + Sync>(mut self, name: &'static str, value: T) -> Self {
        self.meta.insert(name, Arc::new(value));
        self
    }

    /// Attaches a value under `name`, in place.
    pub fn put_meta<T: Any + Send + Sync>(&mut self, name: &'static str, value: T) {
        self.meta.insert(name, Arc::new(value));
    }

    /// The value under `name`, if it is of type `T`.
    #[must_use]
    pub fn meta<T: Any + Send + Sync>(&self, name: &str) -> Option<&T> {
        self.meta.get(name)?.downcast_ref::<T>()
    }

    #[must_use]
    pub fn has_meta(&self, name: &str) -> bool {
        self.meta.contains_key(name)
    }

    /// Asks the view to bring the selection into view when this applies.
    #[must_use]
    pub fn scroll_into_view(mut self) -> Self {
        self.scroll_into_view = true;
        self
    }

    // -- selection and marks -----------------------------------------------

    /// Sets the selection, and clears any pending marks — moving the caret
    /// abandons "the next character will be bold".
    pub fn set_selection(&mut self, selection: Selection) -> &mut Self {
        self.selection = selection;
        self.selection_set = true;
        self.stored_marks = None;
        self.stored_marks_set = true;
        self
    }

    /// Sets the marks the next typed character will carry.
    pub fn set_stored_marks(&mut self, marks: Option<Marks>) -> &mut Self {
        self.stored_marks = marks;
        self.stored_marks_set = true;
        self
    }

    /// Adds a mark to the pending set.
    pub fn add_stored_mark(&mut self, mark: &Mark) -> &mut Self {
        let base = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.selection.marks(self.doc()));
        self.set_stored_marks(Some(mark.add_to_set(&base)))
    }

    /// Removes a mark from the pending set.
    pub fn remove_stored_mark(&mut self, mark: &Mark) -> &mut Self {
        let base = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.selection.marks(self.doc()));
        self.set_stored_marks(Some(mark.remove_from_set(&base)))
    }

    // -- the document ------------------------------------------------------

    /// Runs `f` on the underlying transform and carries the selection across
    /// whatever steps it added.
    fn with<F>(&mut self, f: F) -> Result<&mut Self, StepError>
    where
        F: FnOnce(&mut Transform) -> Result<(), StepError>,
    {
        let before = self.transform.steps().len();
        f(&mut self.transform)?;
        self.carry_selection(before);
        Ok(self)
    }

    /// Maps the selection through the steps added since `from`, and clears
    /// pending marks — text was inserted, so "the next character is bold" has
    /// either happened or been overtaken.
    fn carry_selection(&mut self, from: usize) {
        let len = self.transform.steps().len();
        if len == from {
            return;
        }
        let mapping = self.transform.mapping().slice(from, len);
        self.selection = self.selection.map(self.transform.doc(), &mapping);
        if !self.stored_marks_set {
            self.stored_marks = None;
        }
    }

    /// Applies a step.
    ///
    /// # Errors
    ///
    /// [`StepError`] when it does not apply.
    pub fn step(&mut self, step: Step) -> Result<&mut Self, StepError> {
        self.with(|t| t.step(step).map(|_| ()))
    }

    /// Replaces a range exactly.
    ///
    /// # Errors
    ///
    /// [`StepError`] when it does not apply.
    pub fn replace(&mut self, from: usize, to: usize, slice: Slice) -> Result<&mut Self, StepError> {
        self.with(|t| t.replace(from, to, slice).map(|_| ()))
    }

    /// Replaces a range, fitting the slice to it.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the fitted step does not apply.
    pub fn replace_fitted(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.replace_fitted(from, to, slice).map(|_| ()))
    }

    /// Replaces a range, widening it where the content would rather replace a
    /// node than sit in it.
    ///
    /// # Errors
    ///
    /// [`StepError`] when no attempt applies.
    pub fn replace_range(
        &mut self,
        from: usize,
        to: usize,
        slice: Slice,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.replace_range(from, to, slice).map(|_| ()))
    }

    /// Replaces a range with a single node.
    ///
    /// # Errors
    ///
    /// [`StepError`] when no attempt applies.
    pub fn replace_range_with(
        &mut self,
        from: usize,
        to: usize,
        node: Node,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.replace_range_with(from, to, node).map(|_| ()))
    }

    /// Deletes a range exactly.
    ///
    /// # Errors
    ///
    /// [`StepError`] when it does not apply.
    pub fn delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        self.with(|t| t.delete(from, to).map(|_| ()))
    }

    /// Deletes a range, widening to whole nodes it fully covers.
    ///
    /// # Errors
    ///
    /// [`StepError`] when it does not apply.
    pub fn delete_range(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        self.with(|t| t.delete_range(from, to).map(|_| ()))
    }

    /// Inserts a node.
    ///
    /// # Errors
    ///
    /// [`StepError`] when it does not apply.
    pub fn insert(&mut self, pos: usize, node: Node) -> Result<&mut Self, StepError> {
        self.with(|t| t.insert(pos, node).map(|_| ()))
    }

    /// Splits the node at `pos`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the split does not apply.
    pub fn split(
        &mut self,
        pos: usize,
        depth: usize,
        types_after: Option<&[Option<TypeAndAttrs>]>,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.split(pos, depth, types_after).map(|_| ()))
    }

    /// Joins the nodes either side of `pos`.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the join does not apply.
    pub fn join(&mut self, pos: usize, depth: usize) -> Result<&mut Self, StepError> {
        self.with(|t| t.join(pos, depth).map(|_| ()))
    }

    /// Lifts a range out to `target` depth.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the lift does not apply.
    pub fn lift(&mut self, range: &NodeRange, target: usize) -> Result<&mut Self, StepError> {
        self.with(|t| t.lift(range, target).map(|_| ()))
    }

    /// Wraps a range in a chain of node types.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the wrapping does not apply.
    pub fn wrap(
        &mut self,
        range: &NodeRange,
        wrappers: &[TypeAndAttrs],
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.wrap(range, wrappers).map(|_| ()))
    }

    /// Changes the type of the textblocks in a range.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a change does not apply.
    pub fn set_block_type(
        &mut self,
        from: usize,
        to: usize,
        typ: NodeTypeId,
        attrs: Option<&Attrs>,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.set_block_type(from, to, typ, attrs).map(|_| ()))
    }

    /// Changes the type, attributes or marks of one node.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the change does not apply.
    pub fn set_node_markup(
        &mut self,
        pos: usize,
        typ: Option<NodeTypeId>,
        attrs: Option<&Attrs>,
        marks: Option<Marks>,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.set_node_markup(pos, typ, attrs, marks).map(|_| ()))
    }

    /// Adds a mark across a range.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a step does not apply.
    pub fn add_mark(&mut self, from: usize, to: usize, mark: &Mark) -> Result<&mut Self, StepError> {
        self.with(|t| t.add_mark(from, to, mark).map(|_| ()))
    }

    /// Removes marks across a range.
    ///
    /// # Errors
    ///
    /// [`StepError`] when a step does not apply.
    pub fn remove_mark(
        &mut self,
        from: usize,
        to: usize,
        which: MarkFilter<'_>,
    ) -> Result<&mut Self, StepError> {
        self.with(|t| t.remove_mark(from, to, which).map(|_| ()))
    }

    // -- the selection as a target -----------------------------------------

    /// Replaces the selection with a slice, and puts the caret after what was
    /// inserted.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the replacement does not apply.
    pub fn replace_selection(&mut self, slice: Slice) -> Result<&mut Self, StepError> {
        let (from, to) = (self.selection.from(), self.selection.to());
        let before = self.transform.steps().len();
        // The caret goes inside the inserted content when it ended in inline
        // content, and after it when it ended in a block.
        let ends_inline = last_open_node(&slice).is_none_or(|n| n.is_inline());
        self.with(|t| t.replace_range(from, to, slice).map(|_| ()))?;
        self.selection_to_insertion_end(before, if ends_inline { -1 } else { 1 });
        Ok(self)
    }

    /// Replaces the selection with a node.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the replacement does not apply.
    pub fn replace_selection_with(&mut self, node: Node) -> Result<&mut Self, StepError> {
        let (from, to) = (self.selection.from(), self.selection.to());
        let before = self.transform.steps().len();
        let inline = node.is_inline();
        self.with(|t| t.replace_range_with(from, to, node).map(|_| ()))?;
        self.selection_to_insertion_end(before, if inline { -1 } else { 1 });
        Ok(self)
    }

    /// Deletes the selection.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the deletion does not apply.
    pub fn delete_selection(&mut self) -> Result<&mut Self, StepError> {
        let (from, to) = (self.selection.from(), self.selection.to());
        self.delete_range(from, to)
    }

    /// Replaces the selection with text, carrying the pending marks.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the insertion does not apply.
    pub fn insert_text(&mut self, text: &str) -> Result<&mut Self, StepError> {
        if text.is_empty() {
            return self.delete_selection();
        }
        let marks = self
            .stored_marks
            .clone()
            .unwrap_or_else(|| self.selection.marks(self.doc()));
        let node = self.schema().text(text, marks);
        let (from, to) = (self.selection.from(), self.selection.to());
        let before = self.transform.steps().len();
        self.with(|t| {
            t.replace_range(from, to, Slice::new(Fragment::from(node), 0, 0))
                .map(|_| ())
        })?;
        self.selection_to_insertion_end(before, -1);
        Ok(self)
    }

    /// Puts the caret at the end of what the steps since `from` inserted.
    fn selection_to_insertion_end(&mut self, from: usize, bias: i32) {
        let steps = self.transform.steps();
        if steps.len() <= from {
            return;
        }
        let last = steps.len() - 1;
        if !matches!(
            steps[last],
            Step::Replace { .. } | Step::ReplaceAround { .. }
        ) {
            return;
        }
        let mut end = None;
        self.transform.mapping().maps()[last].for_each(|_, _, _, new_to| {
            if end.is_none() {
                end = Some(new_to);
            }
        });
        let Some(end) = end else { return };
        let doc = self.transform.doc().clone();
        let at = doc.resolve(end.min(doc.content_size()));
        self.selection = Selection::near(&doc, &at, bias);
        self.selection_set = true;
    }
}

/// The deepest last child of a slice's open right edge — what a caret lands
/// in after the slice is inserted.
fn last_open_node(slice: &Slice) -> Option<Node> {
    let mut node = slice.content().last_child().cloned()?;
    for _ in 0..slice.open_end() {
        let Some(next) = node.last_child().cloned() else {
            break;
        };
        node = next;
    }
    Some(node)
}

/// True when a selection covers whole nodes rather than inline content — used
/// by commands that must behave differently for the two.
#[must_use]
pub fn is_structural(selection: &Selection) -> bool {
    matches!(selection.kind(), Kind::Node | Kind::All)
}
