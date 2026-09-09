// SPDX-License-Identifier: MPL-2.0

//! Marks: the formatting carried *by* inline content rather than *around* it.
//!
//! # Why marks are not nodes
//!
//! Because `<b><i>x</i></b>` and `<i><b>x</b></i>` are the same document, and
//! a tree says they are not. Modelling bold as a node forces an arbitrary
//! nesting order, and then every edit has to preserve or repair it: toggling
//! italic off in the middle of a bold run becomes a tree surgery instead of a
//! set operation. As a set attached to each inline node, it is a set
//! operation, and the two spellings above cannot be distinguished because
//! there is nothing to distinguish them with.
//!
//! # Order, exclusion, inclusivity
//!
//! Three properties do the work, and all three come from the schema:
//!
//! - **Rank.** A mark set is kept sorted by its types' order in the schema, so
//!   two equal sets are equal as slices and serialisation is stable.
//! - **Exclusion.** Some marks cannot coexist — `code` typically excludes
//!   `em`, and every mark excludes another of its own type with different
//!   attributes, which is what makes re-linking a link replace the old `href`
//!   instead of nesting a second one.
//! - **Inclusivity.** Whether typing at the very end of a marked run continues
//!   the mark. Bold is inclusive, a link is not: the whole reason you can type
//!   after a link without the new text joining it.

use std::sync::Arc;

use crate::attrs::Attrs;
use crate::schema::MarkType;

/// One mark: a type and its attributes.
#[derive(Debug, Clone)]
pub struct Mark {
    typ: Arc<MarkType>,
    attrs: Attrs,
}

impl Mark {
    #[must_use]
    pub fn new(typ: Arc<MarkType>, attrs: Attrs) -> Self {
        Self { typ, attrs }
    }

    #[must_use]
    pub fn typ(&self) -> &Arc<MarkType> {
        &self.typ
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.typ.name()
    }

    #[must_use]
    pub fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    /// A copy with one attribute changed — how "edit this link's target"
    /// works.
    ///
    /// # Panics
    ///
    /// Never for a well-formed mark; the attribute map is already owned.
    #[must_use]
    pub fn with_attr(&self, name: &str, value: impl Into<crate::attrs::Value>) -> Self {
        Self {
            typ: Arc::clone(&self.typ),
            attrs: self.attrs.set(name, value),
        }
    }

    /// Adds this mark to `set`, returning the new set.
    ///
    /// Follows the three schema properties: a mark that excludes a member
    /// replaces it, a mark excluded by a member is refused (the set comes back
    /// unchanged), and what survives is inserted in rank order.
    ///
    /// # Panics
    ///
    /// Never: the copy it builds is always non-empty by the time it is read.
    #[must_use]
    pub fn add_to_set(&self, set: &Marks) -> Marks {
        let existing = set.as_slice();
        let mut copy: Option<Vec<Self>> = None;
        let mut placed = false;

        for (i, other) in existing.iter().enumerate() {
            if self == other {
                return set.clone();
            }
            if self.typ.excludes(other.typ.id()) {
                // Drop `other`. Everything before it is already known good.
                if copy.is_none() {
                    copy = Some(existing[..i].to_vec());
                }
            } else if other.typ.excludes(self.typ.id()) {
                return set.clone();
            } else {
                if !placed && other.typ.rank() > self.typ.rank() {
                    if copy.is_none() {
                        copy = Some(existing[..i].to_vec());
                    }
                    copy.as_mut().expect("just set").push(self.clone());
                    placed = true;
                }
                if let Some(copy) = copy.as_mut() {
                    copy.push(other.clone());
                }
            }
        }

        let mut copy = copy.unwrap_or_else(|| existing.to_vec());
        if !placed {
            copy.push(self.clone());
        }
        Marks::from_vec(copy)
    }

    /// Removes this exact mark — type *and* attributes — from `set`.
    #[must_use]
    pub fn remove_from_set(&self, set: &Marks) -> Marks {
        match set.as_slice().iter().position(|m| m == self) {
            None => set.clone(),
            Some(i) => {
                let mut copy = set.as_slice().to_vec();
                copy.remove(i);
                Marks::from_vec(copy)
            }
        }
    }

    /// True when this exact mark is in `set`.
    #[must_use]
    pub fn is_in_set(&self, set: &Marks) -> bool {
        set.as_slice().contains(self)
    }
}

impl PartialEq for Mark {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.typ, &other.typ) && self.attrs == other.attrs
    }
}

impl Eq for Mark {}

impl std::hash::Hash for Mark {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.typ.id().hash(state);
        self.attrs.hash(state);
    }
}

/// A set of marks, kept in schema rank order.
///
/// Shared like [`Attrs`], and for the same reason: every inline node in a
/// document carries one, and a bold run of a hundred text nodes should carry
/// one allocation between them, not a hundred.
#[derive(Debug, Clone, Default)]
pub struct Marks(Option<Arc<[Mark]>>);

impl Marks {
    /// The empty set. Allocates nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    #[must_use]
    pub fn from_vec(marks: Vec<Mark>) -> Self {
        if marks.is_empty() {
            Self(None)
        } else {
            Self(Some(Arc::from(marks)))
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Mark] {
        self.0.as_deref().unwrap_or(&[])
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Mark> {
        self.as_slice().iter()
    }

    /// The mark of this type, if the set has one.
    #[must_use]
    pub fn find(&self, typ: &MarkType) -> Option<&Mark> {
        self.as_slice().iter().find(|m| m.typ.id() == typ.id())
    }

    /// True when the set carries a mark of this type, whatever its attributes.
    #[must_use]
    pub fn has_type(&self, typ: &MarkType) -> bool {
        self.find(typ).is_some()
    }

    /// Every mark of this set whose type is in `allowed`, with the rest
    /// dropped — how content is coerced when it moves into a node whose
    /// schema is stricter (pasting a link into a code block).
    #[must_use]
    pub fn retain_allowed(&self, allows: impl Fn(&MarkType) -> bool) -> Self {
        if self.as_slice().iter().all(|m| allows(&m.typ)) {
            return self.clone();
        }
        Self::from_vec(
            self.as_slice()
                .iter()
                .filter(|m| allows(&m.typ))
                .cloned()
                .collect(),
        )
    }
}

impl PartialEq for Marks {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || **a == **b,
            _ => false,
        }
    }
}

impl Eq for Marks {}

impl std::hash::Hash for Marks {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl FromIterator<Mark> for Marks {
    fn from_iter<I: IntoIterator<Item = Mark>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a Marks {
    type Item = &'a Mark;
    type IntoIter = std::slice::Iter<'a, Mark>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
