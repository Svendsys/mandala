// SPDX-License-Identifier: MPL-2.0

//! Predicate language that steers walker traversal — the condition
//! inside [`Instruction::RepeatWhile`](crate::engine::mutator::Instruction::RepeatWhile).
//! A [`Predicate`](crate::engine::predicate::Predicate) pairs field selectors with [`Comparator`](crate::engine::predicate::Comparator)s; what a
//! field selector *is*, and how an element answers one, belongs to the
//! element type through [`Inspect`](crate::engine::predicate::Inspect). Pure data and O(1) evaluations,
//! serde-serializable so the mutator DSL can persist predicates
//! verbatim.

use crate::engine::predicate::Comparator::{Equals, Exists, GreaterThan, LessThan};
use crate::util::geometry::almost_equal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// How an element answers a [`Predicate`]'s field tests.
///
/// `Field` is the element type's field selector — the vocabulary a
/// predicate names facets of the element in — and
/// [`test_field`](Self::test_field) evaluates one `(field, comparator)`
/// pair against the element.
pub trait Inspect {
    /// The field selector a predicate names a facet of this element
    /// with. Serialized verbatim inside a predicate.
    type Field: Clone + Debug + Serialize + DeserializeOwned;

    /// Evaluate one `(field, comparator)` pair against `self`.
    ///
    /// Returns `Some(verdict)` when the pair decides the predicate, or
    /// `None` when the pair does not apply to this element — a facet
    /// the predicate language does not expose, or one this element
    /// does not carry — in which case [`Predicate::test`] moves on to
    /// the next pair. O(1) for every implementation in this crate; no
    /// allocation.
    fn test_field(&self, field: &Self::Field, comparator: &Comparator) -> Option<bool>;
}

/// A comparison operator used by [`Predicate`] to test a single field of an
/// element against a reference value.
///
/// Each variant wraps a `bool` *negation flag*: when `false` the comparison
/// is applied as-is; when `true` the result is inverted. This lets a single
/// enum express both a comparator and its logical complement (e.g. `==` and
/// `!=`) without doubling the variant count.
///
/// Costs: all comparisons are O(1); floating-point equality delegates to
/// [`crate::util::geometry::almost_equal`] to absorb rounding.
///
/// **Negation flag worked example.** A pair naming a presence flag
/// with `Equals(false)` matches every element whose flag is **set** —
/// `Equals(false)` is the un-negated `==`, so the pair passes when
/// `is_set == true`. The negation form `Equals(true)` reads as `!=`,
/// so it passes when `is_set == false` (every element *without* the
/// flag). The two interpretations are easy to flip in your head — keep
/// this example pinned at the call site if you write a new field arm
/// in an [`Inspect`] implementation.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum Comparator {
    /// Equality test. `Equals(false)` means `==`; `Equals(true)` means `!=`.
    Equals(bool),
    /// Existence test. `Exists(false)` returns `true` unconditionally (the
    /// field is present); `Exists(true)` returns `false` (the field must
    /// *not* exist). Primarily used for optional sub-fields like region
    /// font/color.
    Exists(bool),
    /// Strict greater-than. `GreaterThan(false)` means `a > b`;
    /// `GreaterThan(true)` means `a <= b` (the negation).
    GreaterThan(bool),
    /// Strict less-than. `LessThan(false)` means `a < b`;
    /// `LessThan(true)` means `a >= b` (the negation).
    LessThan(bool),
}

impl Comparator {
    /// Construct an equality comparator (`==`). O(1), no allocation.
    pub fn equals() -> Self {
        Equals(false)
    }

    /// Construct a not-equal comparator (`!=`). O(1), no allocation.
    pub fn not_equals() -> Self {
        Equals(true)
    }

    /// Construct an existence comparator — always returns `true`. O(1).
    pub fn exists() -> Self {
        Exists(false)
    }

    /// Construct a non-existence comparator — always returns `false`. O(1).
    pub fn not_exists() -> Self {
        Exists(true)
    }

    /// Construct a strict greater-than comparator (`>`). O(1).
    pub fn greater() -> Self {
        GreaterThan(false)
    }

    /// Construct a less-or-equal comparator (`<=`), the negation of
    /// greater-than. O(1).
    pub fn less_or_equal() -> Self {
        GreaterThan(true)
    }

    /// Construct a strict less-than comparator (`<`). O(1).
    pub fn less() -> Self {
        LessThan(false)
    }

    /// Construct a greater-or-equal comparator (`>=`), the negation of
    /// less-than. O(1).
    pub fn greater_or_equal() -> Self {
        LessThan(true)
    }

    /// Compare two `f32` values using this comparator's semantics.
    ///
    /// * `a` — the element-side value (left operand).
    /// * `b` — the reference value from the predicate field (right operand).
    ///
    /// Equality uses [`crate::util::geometry::almost_equal`] to absorb
    /// floating-point rounding. `Exists` ignores both inputs and returns
    /// the existence flag directly. O(1), no allocation.
    pub fn compare_f32(&self, a: f32, b: f32) -> bool {
        match self {
            Equals(negation) => almost_equal(a, b) != *negation,
            GreaterThan(negation) => (a > b) != *negation,
            LessThan(negation) => (a < b) != *negation,
            Exists(negation) => !negation,
        }
    }
}

/// A condition tested against an element `E` to decide whether a
/// conditional walk continues through it.
///
/// A `Predicate` holds a list of `(E::Field, Comparator)` pairs.
/// [`Predicate::test`] walks the list and returns the verdict of the
/// first pair the element answers ([`Inspect::test_field`] returns
/// `Some`), or `false` when no pair applies. The `always_match` flag
/// short-circuits the walk and unconditionally returns `true` — used
/// by `TargetScope::Descendants` to blanket-apply mutations.
///
/// Costs: `test()` is O(n) in `fields.len()`, but typical predicates
/// carry one or two fields so the cost is effectively O(1). No
/// allocation on the test path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Predicate<E: Inspect> {
    /// The field/comparator pairs to evaluate against a candidate
    /// element. Evaluation stops at the first pair the element
    /// answers — remaining pairs are not consulted.
    pub fields: Vec<(E::Field, Comparator)>,
    /// When true, this predicate matches any element regardless of
    /// fields. Used by `TargetScope::Descendants` to apply mutations to
    /// all descendants.
    #[serde(default)]
    pub always_match: bool,
}

/// Not derived: `derive(Default)` would demand `E: Default`, which a
/// predicate does not need.
impl<E: Inspect> Default for Predicate<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Inspect> Predicate<E> {
    /// An empty predicate that matches nothing (no fields, no
    /// `always_match`). O(1); an empty `Vec` does not allocate.
    pub fn new() -> Self {
        Predicate {
            fields: vec![],
            always_match: false,
        }
    }

    /// A predicate that matches every element unconditionally. O(1);
    /// an empty `Vec` does not allocate.
    pub fn always_true() -> Self {
        Predicate {
            fields: vec![],
            always_match: true,
        }
    }

    /// Test whether `element` satisfies this predicate.
    ///
    /// * Returns `true` immediately if `always_match` is set.
    /// * Otherwise walks `fields` and returns the verdict of the first
    ///   pair `element` answers.
    /// * Returns `false` if no pair applies.
    ///
    /// Costs: O(n) in `fields.len()`, no allocation.
    pub fn test(&self, element: &E) -> bool {
        if self.always_match {
            return true;
        }
        for (field, comparator) in &self.fields {
            if let Some(verdict) = element.test_field(field, comparator) {
                return verdict;
            }
        }
        false
    }
}
