// SPDX-License-Identifier: MPL-2.0

//! `TestElement` — a deliberately un-glyph-like instantiation of the
//! engine. An integer `value`, a kind, a channel and nothing else: no
//! geometry (`SPATIAL = false`), and an inherited state that is real
//! (`Inherited = i64`, each element handing its children
//! `parent + value`). If the walker, the predicate or the builder
//! leaned on anything glyph-shaped, this type could not instantiate
//! them.
//!
//! [`SpatialTestElement`] is the same element with a box, for the one
//! walker arm `TestElement` cannot reach: `SpatialDescend`.
//!
//! Plus the fixture helpers the engine tests and the walker bench
//! build trees with.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::interpolate::{Interpolable, Interpolate, Snap};
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::predicate::{Comparator, Inspect};
use crate::engine::tree::{MutatorTree, Placement, Tree};
use glam::Vec2;
use indextree::NodeId;
use serde::{Deserialize, Serialize};
use std::cell::Cell;

/// The kinds a [`TestElement`] can have. The walker routes on kind and
/// channel together, so two kinds are what it takes to show a mutator
/// for one kind missing the other; predicates can filter on it too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TestKind {
    /// The kind of every element the fixtures build unless told
    /// otherwise, and the kind of [`MutatorTree::new`]'s `Void` root.
    #[default]
    Plain,
    /// A second kind. Keys of this kind sort after every `Plain` key.
    Marked,
}

/// The key of a [`TestKind::Plain`] element on `channel`.
pub fn plain(channel: usize) -> BranchKey<TestKind> {
    BranchKey::new(TestKind::Plain, channel)
}

/// The key of a [`TestKind::Marked`] element on `channel`.
pub fn marked(channel: usize) -> BranchKey<TestKind> {
    BranchKey::new(TestKind::Marked, channel)
}

/// An element that is one integer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestElement {
    /// The element's kind.
    pub kind: TestKind,
    /// The channel the walker aligns it on.
    pub channel: usize,
    /// The element's own value — what mutations change, and what it
    /// adds to the state it hands its children.
    pub value: i64,
}

impl TestElement {
    /// A [`TestKind::Plain`] element.
    pub fn new(channel: usize, value: i64) -> Self {
        TestElement {
            kind: TestKind::Plain,
            channel,
            value,
        }
    }

    /// A [`TestKind::Marked`] element.
    pub fn marked(channel: usize, value: i64) -> Self {
        TestElement {
            kind: TestKind::Marked,
            channel,
            value,
        }
    }
}

/// The derived state of a `TestElement` tree: how many times the
/// walker has invalidated it. A counter rather than a flag so a test
/// can tell one invalidation from several.
#[derive(Clone, Debug, Default)]
pub struct TestCache {
    /// Number of [`TreeCache::invalidate`] calls so far.
    pub invalidations: Cell<usize>,
}

impl TreeCache for TestCache {
    fn invalidate(&self) {
        self.invalidations.set(self.invalidations.get() + 1);
    }
}

/// The facets a predicate can name on a [`TestElement`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestField {
    /// The element's value, compared numerically.
    Value(i64),
}

impl Inspect for TestElement {
    type Field = TestField;

    fn test_field(&self, field: &TestField, comparator: &Comparator) -> Option<bool> {
        match (field, comparator) {
            (TestField::Value(v), Comparator::Equals(neg)) => Some((self.value == *v) != *neg),
            (TestField::Value(v), Comparator::GreaterThan(neg)) => Some((self.value > *v) != *neg),
            (TestField::Value(v), Comparator::LessThan(neg)) => Some((self.value < *v) != *neg),
            (TestField::Value(_), Comparator::Exists(neg)) => Some(!neg),
        }
    }
}

impl TreeElement for TestElement {
    type Kind = TestKind;
    type Cache = TestCache;
    type Inherited = i64;
    /// The value the first part keeps.
    type SplitPoint = i64;
    const SPATIAL: bool = false;

    fn kind(&self) -> TestKind {
        self.kind
    }

    fn channel(&self) -> usize {
        self.channel
    }

    fn void() -> Self {
        TestElement::new(0, 0)
    }

    /// Every field is state.
    fn same_state(&self, other: &Self) -> bool {
        self == other
    }

    fn inherit(&self, parent: &i64) -> i64 {
        parent + self.value
    }

    /// Split at a value threshold: the first part keeps `at`, the
    /// second the rest, `value - at`, both on this element's key. Only
    /// a threshold strictly between 0 and the value cuts anything, so
    /// an element whose value is 1 or less is never splittable.
    fn split(&self, at: &i64) -> Option<(Self, Self)> {
        if *at <= 0 || *at >= self.value {
            return None;
        }
        let part = |value| TestElement {
            kind: self.kind,
            channel: self.channel,
            value,
        };
        Some((part(*at), part(self.value - at)))
    }

    /// Two elements of the same kind merge into one holding the sum of
    /// their values, on this element's key; a sum that overflows does
    /// not merge.
    fn merge(&self, next: &Self) -> Option<Self> {
        if self.kind != next.kind {
            return None;
        }
        Some(TestElement {
            kind: self.kind,
            channel: self.channel,
            value: self.value.checked_add(next.value)?,
        })
    }
}

/// `value` interpolates — as an `f64`, rounded to the nearest integer —
/// and the kind and channel snap.
impl Interpolable for TestElement {
    fn interpolate(before: &Self, after: &Self, t: f32, snap: Snap) -> Self {
        let base = snap.pick(before, after, t);
        TestElement {
            kind: base.kind,
            channel: base.channel,
            value: f64::lerp(&(before.value as f64), &(after.value as f64), t).round() as i64,
        }
    }
}

/// What a mutator can do to a [`TestElement`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestMutation {
    /// `value += n`.
    Add(i64),
    /// `value = n`.
    Set(i64),
    /// `value = inherited` — the state the target's parent handed
    /// down, which is what makes inherited state observable.
    SetToInherited,
    /// `channel = n` — a mutation that changes its target's key, which
    /// is what moves the target among its siblings once the walk ends.
    SetChannel(usize),
}

impl Mutation<TestElement> for TestMutation {
    /// A template is a mutation whose operand the build context offsets
    /// by the iteration index (see `IndexContext` in `builder_tests`).
    type Template = TestMutation;

    fn apply_to(&self, target: &mut TestElement, inherited: &i64) {
        match self {
            TestMutation::Add(n) => target.value += n,
            TestMutation::Set(n) => target.value = *n,
            TestMutation::SetToInherited => target.value = *inherited,
            TestMutation::SetChannel(n) => target.channel = *n,
        }
    }
}

/// The engine's mutator over the test instantiation.
pub type TestMutator = Mutator<TestElement, TestMutation>;

/// Add `element` under `parent` in `tree`, at the end of the run of
/// its key ([`Placement::Last`]); returns its id.
pub fn add(tree: &mut Tree<TestElement>, parent: NodeId, element: TestElement) -> NodeId {
    tree.insert_child(parent, element, Placement::Last).unwrap()
}

/// Add `mutator` under `parent` in `mutators`, at the end of the run
/// of its key ([`Placement::Last`]); returns its id.
pub fn add_mutator(
    mutators: &mut MutatorTree<TestElement, TestMutation>,
    parent: NodeId,
    mutator: TestMutator,
) -> NodeId {
    mutators.insert_child(parent, mutator, Placement::Last).unwrap()
}

/// The values of `parent`'s children, in sibling order.
pub fn row(tree: &Tree<TestElement>, parent: NodeId) -> Vec<i64> {
    parent.children(tree.arena()).map(|id| value(tree, id)).collect()
}

/// The value of the element at `id`.
pub fn value(tree: &Tree<TestElement>, id: NodeId) -> i64 {
    tree.arena()
        .get(id)
        .expect("fixture id belongs to the tree")
        .get()
        .value
}

/// A [`TestElement`] with a box: the same integer, channel and
/// inherited state (`Inherited = i64`, each element handing its
/// children `parent + value`), plus an optional `(top_left, size)` that
/// makes it reachable by `SpatialDescend` (`SPATIAL = true`). It exists
/// so the walker's spatial arm, which folds the hit's inherited state
/// down a path instead of descending one level at a time, has an
/// element type whose state is observable. The glyph instantiation's
/// `Inherited = ()` cannot show a wrong fold.
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialTestElement {
    /// The channel the walker aligns it on.
    pub channel: usize,
    /// The element's own value, as on [`TestElement`].
    pub value: i64,
    /// The element's box as `(top_left, size)`, or `None` for an
    /// element `SpatialDescend` cannot hit.
    pub bounds: Option<(Vec2, Vec2)>,
}

impl SpatialTestElement {
    /// An element with no box.
    pub fn unboxed(channel: usize, value: i64) -> Self {
        SpatialTestElement {
            channel,
            value,
            bounds: None,
        }
    }

    /// An element whose box has its top-left corner at `(x, y)` and
    /// measures `w` by `h`.
    pub fn boxed(channel: usize, value: i64, (x, y, w, h): (f32, f32, f32, f32)) -> Self {
        SpatialTestElement {
            channel,
            value,
            bounds: Some((Vec2::new(x, y), Vec2::new(w, h))),
        }
    }
}

impl Inspect for SpatialTestElement {
    type Field = TestField;

    /// The same answers as [`TestElement`]'s, from this element's value.
    fn test_field(&self, field: &TestField, comparator: &Comparator) -> Option<bool> {
        match (field, comparator) {
            (TestField::Value(v), Comparator::Equals(neg)) => Some((self.value == *v) != *neg),
            (TestField::Value(v), Comparator::GreaterThan(neg)) => Some((self.value > *v) != *neg),
            (TestField::Value(v), Comparator::LessThan(neg)) => Some((self.value < *v) != *neg),
            (TestField::Value(_), Comparator::Exists(neg)) => Some(!neg),
        }
    }
}

impl TreeElement for SpatialTestElement {
    type Kind = ();
    type Cache = TestCache;
    type Inherited = i64;
    type SplitPoint = ();
    const SPATIAL: bool = true;

    fn kind(&self) {}

    fn channel(&self) -> usize {
        self.channel
    }

    fn void() -> Self {
        SpatialTestElement::unboxed(0, 0)
    }

    /// Every field is state.
    fn same_state(&self, other: &Self) -> bool {
        self == other
    }

    fn inherit(&self, parent: &i64) -> i64 {
        parent + self.value
    }

    fn spatial_bounds(&self) -> Option<(Vec2, Vec2)> {
        self.bounds
    }

    /// Breaks `TreeElement::split`'s contract on purpose: the second
    /// part lands on the next channel. The engine must refuse it, which
    /// `structure_tests` checks with this element.
    fn split(&self, _at: &()) -> Option<(Self, Self)> {
        let mut second = self.clone();
        second.channel += 1;
        Some((self.clone(), second))
    }

    /// Breaks `TreeElement::merge`'s contract on purpose: the merged
    /// element is on the next channel. See [`Self::split`].
    fn merge(&self, next: &Self) -> Option<Self> {
        Some(SpatialTestElement {
            channel: self.channel + 1,
            value: self.value + next.value,
            bounds: self.bounds,
        })
    }
}

impl Mutation<SpatialTestElement> for TestMutation {
    type Template = TestMutation;

    /// The same operations as on [`TestElement`].
    fn apply_to(&self, target: &mut SpatialTestElement, inherited: &i64) {
        match self {
            TestMutation::Add(n) => target.value += n,
            TestMutation::Set(n) => target.value = *n,
            TestMutation::SetToInherited => target.value = *inherited,
            TestMutation::SetChannel(n) => target.channel = *n,
        }
    }
}
