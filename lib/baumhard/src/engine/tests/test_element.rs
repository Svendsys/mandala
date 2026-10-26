// SPDX-License-Identifier: MPL-2.0

//! `TestElement` — a deliberately un-glyph-like instantiation of the
//! engine. An integer `value`, a kind, a channel and nothing else: no
//! geometry (`SPATIAL = false`), and an inherited state that is real
//! (`Inherited = i64`, each element handing its children
//! `parent + value`). If the walker, the predicate or the builder
//! leaned on anything glyph-shaped, this type could not instantiate
//! them.
//!
//! Plus the fixture helpers the engine tests and the walker bench
//! build trees with.

use crate::engine::element::{TreeCache, TreeElement};
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::predicate::{Comparator, Inspect};
use crate::engine::tree::{MutatorTree, Tree};
use indextree::NodeId;
use serde::{Deserialize, Serialize};
use std::cell::Cell;

/// The kinds a [`TestElement`] can have. Carried and inspectable;
/// the walker does not route on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TestKind {
    /// The kind of every element the fixtures build unless told
    /// otherwise.
    Plain,
    /// A second kind, for predicates that filter on kind.
    Marked,
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
    /// The element's kind; only `Equals` and `Exists` are meaningful.
    Kind(TestKind),
}

impl Inspect for TestElement {
    type Field = TestField;

    fn test_field(&self, field: &TestField, comparator: &Comparator) -> Option<bool> {
        match (field, comparator) {
            (TestField::Value(v), Comparator::Equals(neg)) => Some((self.value == *v) != *neg),
            (TestField::Value(v), Comparator::GreaterThan(neg)) => Some((self.value > *v) != *neg),
            (TestField::Value(v), Comparator::LessThan(neg)) => Some((self.value < *v) != *neg),
            (TestField::Kind(k), Comparator::Equals(neg)) => Some((self.kind == *k) != *neg),
            (_, Comparator::Exists(neg)) => Some(!neg),
            (TestField::Kind(_), _) => None,
        }
    }
}

impl TreeElement for TestElement {
    type Kind = TestKind;
    type Cache = TestCache;
    type Inherited = i64;
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

    fn inherit(&self, parent: &i64) -> i64 {
        parent + self.value
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
        }
    }
}

/// The engine's mutator over the test instantiation.
pub type TestMutator = Mutator<TestElement, TestMutation>;

/// Append `element` under `parent` in `tree`; returns its id.
pub fn add(tree: &mut Tree<TestElement>, parent: NodeId, element: TestElement) -> NodeId {
    let id = tree.arena.new_node(element);
    parent.append(id, &mut tree.arena);
    id
}

/// Append `mutator` under `parent` in `mutators`; returns its id.
pub fn add_mutator(
    mutators: &mut MutatorTree<TestElement, TestMutation>,
    parent: NodeId,
    mutator: TestMutator,
) -> NodeId {
    let id = mutators.arena.new_node(mutator);
    parent.append(id, &mut mutators.arena);
    id
}

/// The value of the element at `id`.
pub fn value(tree: &Tree<TestElement>, id: NodeId) -> i64 {
    tree.arena
        .get(id)
        .expect("fixture id belongs to the tree")
        .get()
        .value
}
