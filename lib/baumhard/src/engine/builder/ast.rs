// SPDX-License-Identifier: MPL-2.0

//! Typed AST for the mutator-tree DSL. The variants here are what
//! [`build`](super::build()) walks to produce a
//! [`MutatorTree<E, M>`](crate::engine::tree::MutatorTree) ready for
//! `apply_to`. The AST is generic over the element type `E`, which fixes
//! the predicate vocabulary, and the payload `M`, which fixes the
//! literal mutations and the template shape — so one JSON authoring
//! surface serves every instantiation, and the element type is fixed by
//! where the AST is used, never guessed. See `super` for the high-level
//! tour and CONVENTIONS §B2 for why the tree mutates rather than
//! rebuilds.

use crate::engine::element::TreeElement;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Instruction;
use crate::engine::predicate::Predicate;
use crate::util::ordered_vec2::OrderedVec2;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// One node in the mutator-tree DSL. Variants map 1:1 to
/// [`Mutator`](crate::engine::mutator::Mutator) constructors; `Repeat`
/// is a compact sugar for "expand to N children at consecutive
/// channels".
///
/// Every variant carries the `kind` half of the
/// [key](crate::engine::key::BranchKey) its mutator routes on, beside
/// the channel half: a node's `kind` and `channel` together are its
/// key. Both are required in JSON; for an element type whose kind is
/// `()` — the glyph trees — `kind` is written `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "M: Serialize", deserialize = "M: DeserializeOwned"))]
pub enum MutatorNode<E: TreeElement, M: Mutation<E>> {
    /// `Mutator::Void` — no mutation, just structural grouping.
    /// Children are inserted in key order, equal keys in declaration
    /// order.
    Void {
        /// The kind half of the routing key.
        kind: E::Kind,
        /// The channel half of the routing key. The walker descends
        /// only where a target's key matches.
        channel: usize,
        /// Inner mutator nodes inserted in key order under
        /// this grouping. Empty by default so a bare structural
        /// `Void` need not declare an empty list.
        #[serde(default)]
        children: Vec<MutatorNode<E, M>>,
    },
    /// `Mutator::Single` — one mutation on one channel. With
    /// [`MutationSrc::None`] it builds a `Mutator::Void` on the same
    /// channel: a single mutation that applies nothing *is* a void
    /// node, because a mutator's "no payload" is structural.
    Single {
        /// The kind half of the routing key. Inside a `Repeat`
        /// template it must equal the `Repeat`'s own `kind`
        /// ([`BuildError::TemplateKindDiffers`](super::BuildError::TemplateKindDiffers)).
        kind: E::Kind,
        /// Channel source — literal index or per-iteration index
        /// resolved by an enclosing [`MutatorNode::Repeat`].
        channel: ChannelSrc,
        /// Mutation payload — a template resolved per `Repeat`
        /// iteration, runtime-fetched from a
        /// [`BuildContext`](super::context::BuildContext), or nothing.
        mutation: MutationSrc<E, M>,
    },
    /// `Mutator::Macro` — flat batch of mutations on one channel.
    /// Macros can't nest their own mutation list — that's
    /// mutation-level nesting, which this AST doesn't model.
    /// `children` lets a Macro carry child mutator nodes in the
    /// arena (for the `SelfAndDescendants` scope shape: Macro at root
    /// applies to the anchor, with an `Instruction(RepeatWhile)`
    /// child walking descendants). Defaults to empty so the overwhelming
    /// "flat Macro" case stays terse.
    Macro {
        /// The kind half of the routing key.
        kind: E::Kind,
        /// The channel half of the routing key, for the whole macro
        /// batch.
        channel: usize,
        /// Source of the flat `Vec<M>` — literal payload or
        /// runtime-fetched by label.
        mutations: MutationListSrc<M>,
        /// Optional inner nodes expanded after the macro. The
        /// `SelfAndDescendants` shape uses one
        /// `Instruction(RepeatWhile)` child to walk descendants.
        /// Empty default keeps the common flat-Macro case terse.
        #[serde(default)]
        children: Vec<MutatorNode<E, M>>,
    },
    /// `Mutator::Instruction` — recursive evaluation driver
    /// (`RepeatWhile` etc.) wrapping inner children.
    Instruction {
        /// The kind half of the routing key.
        kind: E::Kind,
        /// The channel half of the routing key. The walker descends
        /// only where a target's key matches.
        channel: usize,
        /// Which `Instruction` variant drives the recursive walk
        /// (`RepeatWhile`, `SpatialDescend`, `MapChildren`).
        instruction: InstructionSpec<E>,
        /// Optional per-step mutation applied to the current
        /// target before descending. Defaults to
        /// [`MutationSrc::None`] so `Instruction` nodes that only
        /// drive walking need not carry a payload.
        #[serde(default = "MutationSrc::none_default")]
        mutation: MutationSrc<E, M>,
        /// Inner mutator nodes evaluated against each visited
        /// target during the walk. Empty default for terse JSON.
        #[serde(default)]
        children: Vec<MutatorNode<E, M>>,
    },
    /// Compact "N consecutive children with the same template" — the
    /// "24 children of X" idiom. Expands at build time into
    /// `count - skip_indices.len()` children keyed
    /// `(kind, channel_base + i)` for every `i in 0..count` not in
    /// `skip_indices`. The `template`'s `ChannelSrc` should be
    /// `SectionIndex` so the builder threads the per-iteration channel
    /// through, and the template's own `kind` must equal this one
    /// ([`BuildError::TemplateKindDiffers`](super::BuildError::TemplateKindDiffers)).
    /// Only valid as a child: a `Repeat` at the root of a build is
    /// [`BuildError::RootRepeat`](super::BuildError::RootRepeat).
    Repeat {
        /// The kind of every key this `Repeat` emits.
        kind: E::Kind,
        /// Free-form label the [`BuildContext`](super::context::BuildContext)
        /// keys per-iteration runtime data on (runtime counts and
        /// template resolution).
        section: String,
        /// First channel in the iterated range. Iteration `i`
        /// (zero-indexed) maps to channel `channel_base + i`.
        channel_base: usize,
        /// How many iterations to expand — literal at AST time or
        /// runtime-fetched by section label.
        count: CountSrc,
        /// Iteration indices to skip. The expanded child set is
        /// `count - skip_indices.len()` nodes. Empty by default.
        #[serde(default)]
        skip_indices: Vec<usize>,
        /// One template node cloned per iteration; its
        /// [`ChannelSrc::SectionIndex`] entries resolve to
        /// `channel_base + iter_index` at build time.
        template: Box<MutatorNode<E, M>>,
    },
}

/// Where a `Single`'s channel comes from — the channel half of its key,
/// beside the node's `kind`. Inside a `Repeat`, `SectionIndex` resolves
/// to `channel_base + iter_index`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChannelSrc {
    /// A baked-in channel index.
    Literal(usize),
    /// The iteration's channel (`channel_base + iter_index`). Only
    /// meaningful inside a [`MutatorNode::Repeat`] template; anywhere
    /// else it is
    /// [`BuildError::SectionIndexOutsideRepeat`](super::BuildError::SectionIndexOutsideRepeat).
    SectionIndex,
}

/// Static or runtime-supplied cell count.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CountSrc {
    /// Count baked into the AST at deserialize time.
    Literal(usize),
    /// Count fetched from
    /// [`BuildContext::count`](crate::engine::builder::BuildContext::count)
    /// at build time under the given label.
    Runtime(String),
}

/// Where a single mutation comes from.
///
/// Generic over the element type as well as the payload because the
/// template shape is the payload's
/// [`Mutation::Template`],
/// and a payload type names its template only together with the
/// element type it mutates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "")]
#[non_exhaustive]
pub enum MutationSrc<E: TreeElement, M: Mutation<E>> {
    /// A per-iteration template: the
    /// [`BuildContext`](super::context::BuildContext) resolves it into
    /// a mutation for each `Repeat` iteration, given the section name
    /// and the iteration index. Only meaningful inside a
    /// [`MutatorNode::Repeat`] template; anywhere else it is
    /// [`BuildError::TemplateOutsideRepeat`](super::BuildError::TemplateOutsideRepeat).
    Template(M::Template),
    /// Entirely runtime-supplied single mutation. The context is asked
    /// for it by the label carried here — the same keying as
    /// [`MutationListSrc::Runtime`].
    Runtime(String),
    /// No mutation. A `Single` carrying it builds a `Mutator::Void`;
    /// an `Instruction` carrying it builds with `mutation: None`.
    None,
}

impl<E: TreeElement, M: Mutation<E>> MutationSrc<E, M> {
    pub(super) fn none_default() -> Self {
        MutationSrc::None
    }
}

/// Where a `Macro`'s `Vec<M>` comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MutationListSrc<M> {
    /// A baked-in `Vec<M>` serialized alongside the AST. The
    /// overwhelming case for entries that ship pure data from a JSON
    /// file — no runtime context is consulted.
    Literal(Vec<M>),
    /// Entirely runtime-supplied — the build context returns the list
    /// keyed by the label (a free-form name the consumer disambiguates
    /// on). Used by consumers whose list depends on scene state (e.g.
    /// size-aware layouts).
    Runtime(String),
}

/// Serializable shadow of [`Instruction`].
///
/// The variants are intentionally 1:1 with [`Instruction`] except for
/// `RepeatWhileAlwaysTrue`, which is sugar: it serializes as a plain
/// named variant instead of forcing every JSON author to write out a
/// full always-true [`Predicate`]. The seam is preserved — a later
/// format revision can collapse the sugar into a serde alias without
/// breaking existing documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "")]
#[non_exhaustive]
pub enum InstructionSpec<E: TreeElement> {
    /// `Instruction::RepeatWhile(Predicate::always_true())`.
    RepeatWhileAlwaysTrue,
    /// `Instruction::RepeatWhile(predicate)`.
    RepeatWhile(Predicate<E>),
    /// `Instruction::SpatialDescend(point)`. Refused by
    /// [`build`](super::build()) for an element type that is not
    /// [`SPATIAL`](crate::engine::element::TreeElement::SPATIAL).
    SpatialDescend(OrderedVec2),
    /// `Instruction::MapChildren` — unit variant, no payload. Pairs
    /// this instruction node's mutator children with the current
    /// target's children by sibling position (zip), independent of
    /// key. The opt-in alternative to key-based alignment for
    /// per-index targeting.
    MapChildren,
}

impl<E: TreeElement, M: Mutation<E>> MutatorNode<E, M> {
    /// The kind half of this node's routing key. O(1).
    pub fn kind(&self) -> E::Kind {
        match self {
            MutatorNode::Void { kind, .. }
            | MutatorNode::Single { kind, .. }
            | MutatorNode::Macro { kind, .. }
            | MutatorNode::Instruction { kind, .. }
            | MutatorNode::Repeat { kind, .. } => *kind,
        }
    }
}

impl<E: TreeElement> InstructionSpec<E> {
    /// The [`Instruction`] this spec stands for. O(1); moves the
    /// predicate of a `RepeatWhile`.
    pub fn into_instruction(self) -> Instruction<E> {
        match self {
            InstructionSpec::RepeatWhileAlwaysTrue => Instruction::RepeatWhile(Predicate::always_true()),
            InstructionSpec::RepeatWhile(p) => Instruction::RepeatWhile(p),
            InstructionSpec::SpatialDescend(point) => Instruction::SpatialDescend(point),
            InstructionSpec::MapChildren => Instruction::MapChildren,
        }
    }
}
