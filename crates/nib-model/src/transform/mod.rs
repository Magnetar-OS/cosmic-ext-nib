// SPDX-License-Identifier: MPL-2.0

//! Changes as data.
//!
//! A [`Step`] is one atomic change, and it has three properties that together
//! carry the whole engine:
//!
//! - It **applies** to a document, producing a new one or an error.
//! - It **inverts** against the document it applied to, producing the step
//!   that undoes it.
//! - It **maps** through other steps, producing the equivalent change in a
//!   document those other steps have already altered.
//!
//! Undo is the second property; collaborative editing is the third; and
//! because they are the same two functions on the same values, an engine that
//! has one does not need a second mechanism for the other.
//!
//! A [`Transform`] is a document plus the steps taken to get there, with a
//! [`Mapping`] that carries positions across the lot.

pub mod fit;
pub mod map;
pub mod range;
pub mod step;
pub mod structure;

pub use fit::replace_step;
pub use map::{MapResult, Mapping, Recover, StepMap};
pub use step::{Step, StepError};
pub use structure::{
    MarkFilter, TypeAndAttrs, can_join, can_split, find_wrapping, insert_point, join_point,
    lift_target,
};

use crate::node::Node;
use crate::schema::Schema;
use crate::slice::Slice;

/// A document with a record of how it got that way.
///
/// Carries its schema: every structural method needs it, and a transform only
/// ever works on documents of one schema.
#[derive(Debug, Clone)]
pub struct Transform {
    schema: Schema,
    doc: Node,
    steps: Vec<Step>,
    /// The document before each step, so a step can be inverted after the
    /// fact. Kept because inversion needs the *old* document, and an undo
    /// stack that re-derives it would have to replay from the beginning.
    docs: Vec<Node>,
    mapping: Mapping,
}

impl Transform {
    #[must_use]
    pub fn new(schema: Schema, doc: Node) -> Self {
        Self {
            schema,
            doc,
            steps: Vec::new(),
            docs: Vec::new(),
            mapping: Mapping::new(),
        }
    }

    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The document as it now stands.
    #[must_use]
    pub fn doc(&self) -> &Node {
        &self.doc
    }

    /// The document before any of these steps.
    #[must_use]
    pub fn before(&self) -> &Node {
        self.docs.first().unwrap_or(&self.doc)
    }

    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The document each step applied to, in order.
    #[must_use]
    pub fn docs(&self) -> &[Node] {
        &self.docs
    }

    #[must_use]
    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    #[must_use]
    pub fn doc_changed(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Applies a step.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the step does not apply, leaving the transform
    /// untouched. A failed step is a normal outcome — a command asking whether
    /// something is possible does so by trying it — not an exceptional one.
    pub fn step(&mut self, step: Step) -> Result<&mut Self, StepError> {
        let doc = step.apply(&self.doc)?;
        self.add_step(step, doc);
        Ok(self)
    }

    /// Applies a step, ignoring it if it does not apply.
    pub fn maybe_step(&mut self, step: Step) -> &mut Self {
        let _ = self.step(step);
        self
    }

    fn add_step(&mut self, step: Step, doc: Node) {
        self.docs.push(std::mem::replace(&mut self.doc, doc));
        self.mapping.append_map(step.step_map(), None);
        self.steps.push(step);
    }

    /// Replaces a range with a slice, exactly: the slice must already fit.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the replacement does not apply.
    pub fn replace(&mut self, from: usize, to: usize, slice: Slice) -> Result<&mut Self, StepError> {
        self.step(Step::replace(from, to, slice))
    }

    /// Deletes a range.
    ///
    /// # Errors
    ///
    /// [`StepError`] when the deletion does not apply.
    pub fn delete(&mut self, from: usize, to: usize) -> Result<&mut Self, StepError> {
        self.replace(from, to, Slice::empty())
    }

    /// The steps of this transform, inverted and reversed — the transform that
    /// takes the document back.
    #[must_use]
    pub fn invert(&self) -> Vec<Step> {
        self.steps
            .iter()
            .enumerate()
            .rev()
            .map(|(i, step)| step.invert(&self.docs[i]))
            .collect()
    }
}
