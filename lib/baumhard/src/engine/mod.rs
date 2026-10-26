// SPDX-License-Identifier: MPL-2.0

//! The mutation engine, generic over **the element type** and **the
//! mutation payload**.
//!
//! A [`Tree<E>`](crate::engine::tree::Tree) is an arena of elements; a
//! [`MutatorTree<E, M>`](crate::engine::tree::MutatorTree) is an arena of
//! [`Mutator`](crate::engine::mutator::Mutator)s that carry payloads `M`. The
//! [walk](crate::engine::walker) aligns the two by
//! [key](crate::engine::key::BranchKey) — an element's kind and channel —
//! and applies each payload through
//! [`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to),
//! carrying the element type's [inherited state](crate::engine::element::TreeElement::Inherited)
//! down the walk on its own call stack. Both arenas keep every parent's
//! children sorted by key, so the walk merges sibling rows in step, and a
//! walk can start at [anchors](crate::engine::anchor::Anchors) — the
//! selection — rather than at the root. [`Predicate`](crate::engine::predicate::Predicate)s
//! steer conditional walks through an element's
//! [`Inspect`](crate::engine::predicate::Inspect) surface, and the
//! [builder DSL](crate::engine::builder) compiles a serde-friendly
//! [`MutatorNode`](crate::engine::builder::MutatorNode) AST into a mutator tree.
//!
//! A walk can be **journaled**: the walker records every element it
//! applies to, once, in a [`JournalBuilder`](crate::engine::journal::JournalBuilder),
//! and the closed [`Journal`](crate::engine::journal::Journal) can
//! [revert](crate::engine::journal::Journal::revert) or
//! [replay](crate::engine::journal::Journal::replay) the change exactly,
//! or [show it part-way](crate::engine::journal::Journal::write_interpolated)
//! for an element type that is [`Interpolable`](crate::engine::interpolate::Interpolable).
//!
//! Nothing here knows what an element *is*. The glyph trees
//! (`gfx_structs`) are one instantiation — `GfxElement` with the
//! `GfxMutation` payload — and the document is meant to be another;
//! neither is special-cased here. That is a rule of this module, not a
//! habit: it imports nothing from `gfx_structs`, `mindmap` or `font`,
//! and `util::engine_isolation` holds it to that.
//!
//! Prescriptive rules for the walker (mutation, not rebuild; no
//! allocation per visited node) live in `lib/baumhard/CONVENTIONS.md`
//! §B2 and §B7.

/// [`Anchors`](anchor::Anchors) — where an anchored walk
/// ([`MutatorTree::apply_at`](tree::MutatorTree::apply_at)) starts.
pub mod anchor;
/// Builder DSL: the [`MutatorNode`](builder::MutatorNode) AST, the
/// [`BuildContext`](builder::BuildContext) runtime look-up and
/// [`build`](builder::build()).
pub mod builder;
/// [`TreeElement`](element::TreeElement) — what an element type
/// provides to the engine — and its per-tree derived state,
/// [`TreeCache`](element::TreeCache).
pub mod element;
/// [`Interpolate`](interpolate::Interpolate),
/// [`Snap`](interpolate::Snap) and [`Interpolable`](interpolate::Interpolable):
/// the state between a journal's before and after.
pub mod interpolate;
/// [`JournalBuilder`](journal::JournalBuilder) and
/// [`Journal`](journal::Journal): the exact record of what a walk
/// changed, with [`revert`](journal::Journal::revert) and
/// [`replay`](journal::Journal::replay).
pub mod journal;
/// [`BranchKey`](key::BranchKey) — the kind and channel the walker
/// routes on, and every parent's children are sorted by.
pub mod key;
/// [`Mutation`](mutation::Mutation) — the payload trait a mutator
/// carries to an element.
pub mod mutation;
/// [`Mutator`](mutator::Mutator) — one node of a mutator tree — and
/// the [`Instruction`](mutator::Instruction) control-flow vocabulary.
pub mod mutator;
/// [`Predicate`](predicate::Predicate) and
/// [`Comparator`](predicate::Comparator): the conditions that steer
/// [`Instruction::RepeatWhile`](mutator::Instruction::RepeatWhile),
/// evaluated through an element's [`Inspect`](predicate::Inspect)
/// surface.
pub mod predicate;
/// The sorted-children invariant as arena operations: sorted insert,
/// single-node repair, batch re-attach and the check.
mod siblings;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8), plus
/// the `TestElement` instantiation that proves the engine is not
/// glyph-bound. `#[allow(clippy::unwrap_used)]` because this is a
/// tests tree (§T2.2): it carries no `cfg(test)` gate, so the
/// crate-root lint sees it as shipped code; `util::unwrap_posture`
/// excludes these trees by path.
#[allow(clippy::unwrap_used)]
pub mod tests;
/// [`Tree`](tree::Tree) and [`MutatorTree`](tree::MutatorTree), the
/// two arenas a walk aligns.
pub mod tree;
/// The walker behind [`MutatorTree::apply_to`](tree::MutatorTree::apply_to).
pub mod walker;
