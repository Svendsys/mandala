// SPDX-License-Identifier: MPL-2.0

//! [`Mutation`](crate::engine::mutation::Mutation) — the payload a [`Mutator`](crate::engine::mutator::Mutator)
//! carries to one element.

use crate::engine::element::TreeElement;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;

/// A change that can be applied to one element of type `E`.
///
/// The walker decides *which* elements a mutation reaches; the
/// mutation decides *what* happens to each one. A mutator that carries
/// no payload says so with `Option::None` (an
/// [`Instruction`](crate::engine::mutator::Mutator::Instruction)'s
/// `mutation`) or with a [`Void`](crate::engine::mutator::Mutator::Void)
/// node — a mutation type has no no-op variant of its own.
pub trait Mutation<E: TreeElement>: Clone + Debug {
    /// What a builder template for this payload looks like:
    /// [`MutationSrc::Template`](crate::engine::builder::MutationSrc::Template)
    /// carries one, and the
    /// [`BuildContext`](crate::engine::builder::BuildContext) resolves
    /// it into a concrete mutation per `Repeat` iteration.
    type Template: Clone + Debug + Serialize + DeserializeOwned;

    /// Apply this mutation to `target`. `inherited` is the inherited
    /// state of the target's **parent** — the value its parent handed
    /// down, computed after the parent's own mutations in the same
    /// walk — so a mutation can act on a value the target inherits
    /// rather than stores.
    fn apply_to(&self, target: &mut E, inherited: &E::Inherited);
}
