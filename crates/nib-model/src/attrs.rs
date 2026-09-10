// SPDX-License-Identifier: MPL-2.0

//! Node and mark attributes.
//!
//! # Why a value type at all
//!
//! A schema is data. `heading` has a `level`, `link` has an `href`, `image`
//! has a `src` and an `alt` — and the set of them is decided by whoever builds
//! the schema, not by this crate. A generic bag is therefore unavoidable, and
//! the only question is how wide the values may be.
//!
//! [`Value`] is deliberately narrow: null, bool, integer, float, string. It is
//! not JSON — there are no nested objects or arrays. An attribute that wants
//! structure is an attribute that wanted to be a child node, and letting it
//! hide inside a value is how documents grow contents the schema cannot see,
//! positions cannot address, and steps cannot map.
//!
//! # Equality
//!
//! Attributes take part in node equality, which takes part in deciding whether
//! two adjacent text nodes may merge and whether a redraw is needed. So
//! [`Value`] is `Eq` and `Hash`, including for floats, where both are defined
//! over `f64::to_bits`. That makes `NaN == NaN` and `0.0 != -0.0`, neither of
//! which matches IEEE 754 — but attributes are compared to answer "is this the
//! same document?", and by that question two `NaN`s written the same way are.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// One attribute value.
#[derive(Debug, Clone)]
pub enum Value {
    /// An attribute that is present and empty — distinct from absent, which is
    /// how a schema default is spelled.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
}

impl Value {
    /// The string, if this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The integer, if this is one. Floats do not narrow: a `level` that
    /// arrived as `1.0` is a schema that let a float in where an int belongs,
    /// and silently rounding it hides that.
    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            // Bitwise, per the module note: this answers "same document?",
            // not "same number?".
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Str(a), Self::Str(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Null => {}
            Self::Bool(b) => b.hash(state),
            Self::Int(i) => i.hash(state),
            Self::Float(f) => f.to_bits().hash(state),
            Self::Str(s) => s.hash(state),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str(""),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(x) => write!(f, "{x}"),
            Self::Str(s) => f.write_str(s),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<usize> for Value {
    #[allow(clippy::cast_possible_wrap)]
    fn from(v: usize) -> Self {
        Self::Int(v as i64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Str(Arc::from(v))
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Str(Arc::from(v))
    }
}

impl From<Arc<str>> for Value {
    fn from(v: Arc<str>) -> Self {
        Self::Str(v)
    }
}

/// A node's or mark's attributes.
///
/// Shared, not copied. A transaction that rewrites one paragraph in a thousand
/// clones a thousand `Arc`s and one map; making this a plain `BTreeMap` would
/// make it clone a thousand maps. The document model is persistent everywhere
/// for the same reason, and this is where it starts.
///
/// Ordered rather than hashed, so serialising a document twice writes the same
/// bytes — which is what makes a round-trip test a test rather than a coin
/// toss.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs(Option<Arc<BTreeMap<Arc<str>, Value>>>);

impl Attrs {
    /// The empty attribute set. Allocates nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_none_or(|m| m.is_empty())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |m| m.len())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.as_ref()?.get(name)
    }

    /// The value as a string, for the common case of a `href` or a `src`.
    #[must_use]
    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name)?.as_str()
    }

    #[must_use]
    pub fn get_int(&self, name: &str) -> Option<i64> {
        self.get(name)?.as_int()
    }

    #[must_use]
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.get(name)?.as_bool()
    }

    /// Returns a copy with `name` set to `value`. Copy-on-write: the map is
    /// cloned only here, and only when something actually changes.
    #[must_use]
    pub fn set(&self, name: impl Into<Arc<str>>, value: impl Into<Value>) -> Self {
        let name = name.into();
        let value = value.into();
        if self.get(&name) == Some(&value) {
            return self.clone();
        }
        let mut map = self
            .0
            .as_ref()
            .map_or_else(BTreeMap::new, |m| (**m).clone());
        map.insert(name, value);
        Self(Some(Arc::new(map)))
    }

    /// Returns a copy without `name`.
    #[must_use]
    pub fn remove(&self, name: &str) -> Self {
        if self.get(name).is_none() {
            return self.clone();
        }
        let mut map = self
            .0
            .as_ref()
            .map_or_else(BTreeMap::new, |m| (**m).clone());
        map.remove(name);
        if map.is_empty() {
            return Self::none();
        }
        Self(Some(Arc::new(map)))
    }

    /// Iterates in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().flat_map(|m| m.iter().map(|(k, v)| (&**k, v)))
    }
}

impl<K, V> FromIterator<(K, V)> for Attrs
where
    K: Into<Arc<str>>,
    V: Into<Value>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let map: BTreeMap<Arc<str>, Value> = iter
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        if map.is_empty() {
            Self::none()
        } else {
            Self(Some(Arc::new(map)))
        }
    }
}

impl std::hash::Hash for Attrs {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for (name, value) in self.iter() {
            name.hash(state);
            value.hash(state);
        }
    }
}

/// Builds an [`Attrs`] from `name => value` pairs.
///
/// ```
/// # use nib_model::attrs;
/// let a = attrs! { "level" => 2_i64, "id" => "intro" };
/// assert_eq!(a.get_int("level"), Some(2));
/// ```
#[macro_export]
macro_rules! attrs {
    () => { $crate::attrs::Attrs::none() };
    ($($name:expr => $value:expr),+ $(,)?) => {{
        <$crate::attrs::Attrs as ::core::iter::FromIterator<(&str, $crate::attrs::Value)>>::from_iter(
            [$(($name, $crate::attrs::Value::from($value))),+]
        )
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_attrs_allocate_nothing_and_compare_equal() {
        assert_eq!(Attrs::none(), Attrs::none());
        assert!(Attrs::none().is_empty());
        assert_eq!(Attrs::none().len(), 0);
    }

    #[test]
    fn set_is_copy_on_write_and_leaves_the_original_alone() {
        let base = attrs! { "level" => 1_i64 };
        let raised = base.set("level", 2_i64);
        assert_eq!(base.get_int("level"), Some(1));
        assert_eq!(raised.get_int("level"), Some(2));
    }

    #[test]
    fn setting_the_value_it_already_has_is_not_a_change() {
        let base = attrs! { "level" => 1_i64 };
        assert_eq!(base, base.set("level", 1_i64));
    }

    #[test]
    fn removing_the_last_attribute_returns_to_the_empty_set() {
        let one = attrs! { "id" => "x" };
        assert_eq!(one.remove("id"), Attrs::none());
        // Removing what is not there is not a change.
        assert_eq!(one.remove("nope"), one);
    }

    #[test]
    fn iteration_is_in_name_order_so_serialisation_is_stable() {
        let a = attrs! { "z" => 1_i64, "a" => 2_i64, "m" => 3_i64 };
        let names: Vec<&str> = a.iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["a", "m", "z"]);
    }

    #[test]
    fn nan_equals_nan_because_the_question_is_sameness_not_arithmetic() {
        assert_eq!(Value::Float(f64::NAN), Value::Float(f64::NAN));
        assert_ne!(Value::Float(0.0), Value::Float(-0.0));
    }

    #[test]
    fn null_is_present_and_distinct_from_absent() {
        let a = attrs! { "alt" => Value::Null };
        assert_eq!(a.get("alt"), Some(&Value::Null));
        assert_eq!(a.get("missing"), None);
        assert!(!a.is_empty());
    }
}
