// SPDX-License-Identifier: MPL-2.0

//! [`Mutator`](crate::engine::mutator::Mutator) — one node of a
//! [`MutatorTree`](crate::engine::tree::MutatorTree) — and the
//! [`Instruction`](crate::engine::mutator::Instruction) vocabulary that changes *how* the walker treats a
//! node's children rather than *what* it changes.

use crate::engine::element::TreeElement;
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::predicate::Predicate;
use crate::util::ordered_vec2::OrderedVec2;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use strum_macros::EnumDiscriminants;

/// A control-flow directive carried by a [`Mutator::Instruction`]
/// node. Instructions govern *how* the walker processes the mutator's
/// children against the target tree, rather than *what* field to
/// change. Evaluated once per matching target node during
/// [`walk_tree_from`](crate::engine::walker::walk_tree_from); cost is
/// proportional to the number of matching descendants.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound = "")]
#[non_exhaustive]
pub enum Instruction<E: TreeElement> {
    /// Recursively apply the child mutator nodes of this instruction
    /// on every target descendant for which the [`Predicate`] returns
    /// true. When the predicate fails on a node the branch terminates
    /// and the walker resumes, at that node, with the repeated
    /// mutator's own children aligned by key. O(n) in the number of
    /// descendants tested.
    ///
    /// Routed by kind at every level. The instruction's children reach
    /// the target's children by [key](BranchKey) — kind and channel —
    /// and below that a repeated child descends into the children of
    /// its own **kind** on every channel: a descendant of another kind
    /// is not tested, not mutated and not descended into. For
    /// per-index targeting that sidesteps keys entirely, see
    /// [`Instruction::MapChildren`].
    RepeatWhile(Predicate<E>),
    /// Find the descendant of the current target whose own box
    /// ([`TreeElement::spatial_bounds`]) contains the point — the
    /// smallest such box, the first in pre-order on a tie — and apply
    /// the instruction's mutation to it; no-op if no box contains the
    /// point.
    ///
    /// Bypasses key alignment — like [`Instruction::MapChildren`]
    /// but routing on a spatial hit-test rather than sibling position.
    /// Only meaningful for an element type with
    /// [`SPATIAL`](TreeElement::SPATIAL) set; the builder refuses it
    /// for any other.
    ///
    /// Cost: O(descendants of the target), no allocation on a miss
    /// and one path-length `Vec` on a hit (the hit's inherited state
    /// is folded down that path).
    SpatialDescend(OrderedVec2),
    /// Pair mutator children with target children **by sibling
    /// position** (zip), ignoring the keys of the paired children.
    /// The opt-in alternative to [`Instruction::RepeatWhile`]'s
    /// key-broadcast semantics, used when each child needs a
    /// distinct mutation (per-index layout). Excess children on
    /// either side are dropped with one `debug!` at termination.
    /// The attached `mutation` field is applied to the current
    /// target before the body runs, same as
    /// [`Instruction::RepeatWhile`]. Composes with the AST `Repeat`
    /// wrapper for runtime-count expansion.
    ///
    /// Cost: O(min(mutator_children, target_children)); no
    /// allocation inside the zip.
    MapChildren,
}

/// A node in a [`MutatorTree`](crate::engine::tree::MutatorTree).
///
/// Each variant pairs a [`BranchKey`] (used by the walker to align
/// mutator nodes with target nodes: a mutator reaches a target only
/// when the two keys are equal) with what happens when the walker
/// reaches the matching target. A node that carries no payload says so
/// structurally: [`Mutator::Void`], or an [`Mutator::Instruction`]
/// whose `mutation` is `None`.
///
/// Cost of applying: O(1) for `Single`/`Void`, O(k) for `Macro` where
/// k is the number of inner mutations.
///
/// The payload-free `MutatorType` tag is derived from this enum and
/// reached through
/// [`crate::core::primitives::Discriminated::variant`].
#[derive(Clone, Debug, Serialize, Deserialize, EnumDiscriminants)]
#[serde(bound(serialize = "M: Serialize", deserialize = "M: DeserializeOwned"))]
#[strum_discriminants(name(MutatorType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize))]
#[strum_discriminants(doc = "Payload-free tag for [`Mutator`], derived by strum. Lets the")]
#[strum_discriminants(doc = "walker match a mutator kind without destructuring the full")]
#[strum_discriminants(doc = "enum. No heap cost — `Copy`, and comparison is a single-byte")]
#[strum_discriminants(doc = "test.")]
pub enum Mutator<E: TreeElement, M: Mutation<E>> {
    /// A placeholder node that occupies a position in the mutator
    /// tree without carrying any mutation. Used to carry its children
    /// down to the target its key matches, and to group children.
    Void {
        /// The key the walker aligns this node on.
        key: BranchKey<E::Kind>,
    },
    /// A single mutation targeting the element with the matching key.
    Single {
        /// The key the walker aligns this node on.
        key: BranchKey<E::Kind>,
        /// The mutation payload to apply.
        mutation: M,
    },
    /// A batch of mutations applied to the same target element in
    /// sequence, in the order of the `Vec`.
    Macro {
        /// The key the walker aligns this node on.
        key: BranchKey<E::Kind>,
        /// The mutations to apply, in order.
        mutations: Vec<M>,
    },
    /// A control-flow node: the [`Instruction`] governs how the walker
    /// processes this node's children against the target tree. The
    /// optional `mutation` is applied to the matched target before the
    /// instruction body runs.
    Instruction {
        /// The key the walker aligns this node on.
        key: BranchKey<E::Kind>,
        /// The control-flow directive.
        instruction: Instruction<E>,
        /// An optional direct mutation applied before the instruction
        /// body. `None` when the node only steers the walk.
        mutation: Option<M>,
    },
}

impl<E: TreeElement, M: Mutation<E>> Mutator<E, M> {
    /// A `Single` mutator on `key`. Takes anything that converts into
    /// a key, so a single-kind element type can name a channel alone.
    /// No allocation beyond what `mutation` already owns.
    pub fn new(mutation: M, key: impl Into<BranchKey<E::Kind>>) -> Self {
        Mutator::Single {
            key: key.into(),
            mutation,
        }
    }

    /// A `Macro` mutator carrying several mutations on `key`. The
    /// `Vec` is moved, not cloned.
    pub fn new_macro(mutations: Vec<M>, key: impl Into<BranchKey<E::Kind>>) -> Self {
        Mutator::Macro {
            key: key.into(),
            mutations,
        }
    }

    /// A `Void` placeholder on `key`. No payload, no allocation.
    pub fn new_void(key: impl Into<BranchKey<E::Kind>>) -> Self {
        Mutator::Void { key: key.into() }
    }

    /// An `Instruction` mutator on `key` with no mutation of its own.
    /// The instruction governs child traversal during the walk.
    pub fn new_instruction(instruction: Instruction<E>, key: impl Into<BranchKey<E::Kind>>) -> Self {
        Mutator::Instruction {
            key: key.into(),
            instruction,
            mutation: None,
        }
    }

    /// The key the walker aligns this node on. O(1).
    pub fn key(&self) -> BranchKey<E::Kind> {
        match self {
            Mutator::Void { key }
            | Mutator::Single { key, .. }
            | Mutator::Macro { key, .. }
            | Mutator::Instruction { key, .. } => *key,
        }
    }

    /// Whether this mutator is of the given [`MutatorType`]. O(1), no
    /// allocation.
    pub fn is(&self, mutator_type: MutatorType) -> bool {
        MutatorType::from(self) == mutator_type
    }

    /// Whether [`Self::apply_to`] would apply any mutation: a `Single`,
    /// a non-empty `Macro`, or an `Instruction` carrying a mutation.
    /// The walker applies — and journals — only a mutator that does.
    /// O(1).
    pub fn has_payload(&self) -> bool {
        match self {
            Mutator::Single { .. } => true,
            Mutator::Macro { mutations, .. } => !mutations.is_empty(),
            Mutator::Instruction { mutation, .. } => mutation.is_some(),
            Mutator::Void { .. } => false,
        }
    }

    /// Apply this node's payload to `target`: the one mutation of a
    /// `Single`, every mutation of a `Macro` in order, an
    /// `Instruction`'s mutation when it has one, nothing for `Void`.
    /// `inherited` is the target's parent's inherited state, passed
    /// through to every [`Mutation::apply_to`]. Does not consult
    /// keys — the walker decides whether this node reaches
    /// `target`. O(k) in the number of mutations applied.
    pub fn apply_to(&self, target: &mut E, inherited: &E::Inherited) {
        match self {
            Mutator::Single { mutation, .. }
            | Mutator::Instruction {
                mutation: Some(mutation),
                ..
            } => mutation.apply_to(target, inherited),
            Mutator::Macro { mutations, .. } => {
                for mutation in mutations {
                    mutation.apply_to(target, inherited);
                }
            }
            Mutator::Void { .. } | Mutator::Instruction { mutation: None, .. } => {}
        }
    }
}
