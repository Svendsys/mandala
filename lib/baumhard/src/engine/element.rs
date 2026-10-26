// SPDX-License-Identifier: MPL-2.0

//! What an element type gives the engine.
//!
//! The walker needs four things from an element and nothing else: the
//! [key](crate::engine::key::BranchKey) it routes on and is sorted by
//! among its siblings, a placeholder to pad structure with, the
//! state it hands its children ([`TreeElement::inherit`](crate::engine::element::TreeElement::inherit)), and —
//! only for element types with geometry — a box to hit-test. A
//! [journal](crate::engine::journal::Journal) needs one more: whether two values
//! of the element hold the same state
//! ([`TreeElement::same_state`](crate::engine::element::TreeElement::same_state)). Every
//! other fact about an element is the business of its
//! [`Mutation`](crate::engine::mutation::Mutation)s and its
//! [`Inspect`](crate::engine::predicate::Inspect) surface.

use crate::engine::key::BranchKey;
use crate::engine::predicate::Inspect;
use glam::Vec2;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::hash::Hash;

/// An element a [`Tree`](crate::engine::tree::Tree) can hold and the
/// walker can mutate.
///
/// Implementations are expected to be small values: a plain walk clones
/// none of them, but a journaled walk clones each element it applies
/// to once, and a journal clones it again to restore it.
///
/// `Clone` must copy the element's whole state — a journal's
/// before-image is a clone, and `revert` writes it back.
pub trait TreeElement: Clone + Debug + Inspect {
    /// The element's kind: the first half of its
    /// [`key`](Self::key). The walker routes on kind and channel
    /// together, so one parent can hold children of several kinds
    /// whose channel spaces are independent. An element type with one
    /// kind uses `()`, and then routes on its channel alone.
    type Kind: Copy + Ord + Eq + Hash + Debug + Serialize + DeserializeOwned;

    /// Per-tree derived state, owned by the
    /// [`Tree`](crate::engine::tree::Tree) and dropped by
    /// [`TreeCache::invalidate`] whenever the walker applies a
    /// mutator to one of its elements. `()` for an element type that derives nothing.
    type Cache: TreeCache;

    /// State the walker carries **down** a walk: what an element
    /// inherits rather than stores. A mutation receives its target's
    /// parent's value (see [`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to)),
    /// and the walker computes the target's own value with
    /// [`inherit`](Self::inherit) — after the target's own mutations —
    /// before descending. The value lives on the walker's call stack
    /// and is never stored on an element or in a tree, so it should be
    /// a small value type. `()` for an element type that inherits
    /// nothing.
    type Inherited: Clone + Debug;

    /// Whether [`spatial_bounds`](Self::spatial_bounds) can ever return
    /// `Some`. The builder refuses a
    /// [`SpatialDescend`](crate::engine::mutator::Instruction::SpatialDescend)
    /// for an element type where this is `false`
    /// ([`BuildError::SpatialOnNonSpatial`](crate::engine::builder::BuildError::SpatialOnNonSpatial)).
    const SPATIAL: bool;

    /// This element's kind. O(1).
    fn kind(&self) -> Self::Kind;

    /// This element's channel within its kind: the second half of its
    /// [`key`](Self::key). O(1).
    fn channel(&self) -> usize;

    /// The key the walker routes this element on, and the key its
    /// parent keeps it sorted by among its siblings: its
    /// [`kind`](Self::kind), then its [`channel`](Self::channel).
    ///
    /// A change to either is a change of position: a tree moves an
    /// element whose key changed to its sorted place
    /// ([`Tree::update`](crate::engine::tree::Tree::update), and the
    /// walker after a walk). O(1).
    fn key(&self) -> BranchKey<Self::Kind> {
        BranchKey::new(self.kind(), self.channel())
    }

    /// A neutral placeholder, used as the root of an empty tree.
    fn void() -> Self;

    /// Whether `self` and `other` hold the same **state**: every field
    /// that is the element's own value — what a mutation can change,
    /// what a journal restores, and what is drawn or saved from it.
    /// Bookkeeping that is not state (callbacks, memoized derived
    /// values) is left out.
    ///
    /// A [`Journal`](crate::engine::journal::Journal) asks this when it
    /// closes: an element whose state is the same before and after,
    /// and whose position did not change, did not change at all, and
    /// its entry is dropped. An answer of `true` for two different
    /// states therefore loses that change from the journal — its
    /// `revert` would not undo it — so an implementation compares
    /// every state field, never a coarser identity. `false` for equal
    /// states costs only a redundant entry.
    ///
    /// Required, with no default, so that a new element type states
    /// what its state is. For a plain value type it is `==`. Cost: an
    /// implementation's field compares; no allocation.
    fn same_state(&self, other: &Self) -> bool;

    /// The state this element hands its children, given the state
    /// its own parent handed it. Called once per element the walker
    /// descends through, after that element's own mutations.
    fn inherit(&self, parent: &Self::Inherited) -> Self::Inherited;

    /// This element's own box as `(top_left, size)`, or `None` when it
    /// has no geometry to hit-test. Read by
    /// [`SpatialDescend`](crate::engine::mutator::Instruction::SpatialDescend):
    /// the point is inside when `top_left <= point <= top_left + size`
    /// on both axes, and among several boxes the smallest
    /// `size.x * size.y` wins. Returned as an origin and a size rather
    /// than two corners so that the area is the element's own size,
    /// not a difference of two rounded sums. Defaults to `None`; an
    /// implementation that overrides it sets
    /// [`SPATIAL`](Self::SPATIAL). O(1).
    fn spatial_bounds(&self) -> Option<(Vec2, Vec2)> {
        None
    }
}

/// Per-tree derived state a [`Tree`](crate::engine::tree::Tree) owns
/// beside its arena — the glyph trees keep their AABB memo and region
/// index here.
///
/// [`invalidate`](Self::invalidate) takes `&self`, because the walker
/// calls it while the tree's arena is mutably borrowed; an
/// implementation keeps its state in `Cell`s. It must be O(1): it runs
/// every time the walker applies a mutator to a target.
pub trait TreeCache: Default + Clone + Debug {
    /// Mark everything derived as stale. The next reader recomputes.
    fn invalidate(&self);
}

/// The cache of an element type that derives nothing.
impl TreeCache for () {
    fn invalidate(&self) {}
}
