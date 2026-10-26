// SPDX-License-Identifier: MPL-2.0

//! [`BranchKey`](crate::engine::key::BranchKey) — what the walker
//! routes on.
//!
//! A key is an element's **kind** and its **channel**. A mutator
//! reaches a target only when the two keys are equal, kind included,
//! so one parent can hold children of several kinds whose channel
//! spaces are independent: a mutator addressed to one kind's channel 0
//! cannot land on another kind's channel 0 under the same parent.
//!
//! Keys are also what a parent's children are ordered by. Every
//! [`Tree`](crate::engine::tree::Tree) and
//! [`MutatorTree`](crate::engine::tree::MutatorTree) keeps each
//! parent's children in ascending key order — kind first, then channel
//! — which is what lets the walker merge a mutator row against a target
//! row in one pass, in step, without collecting or sorting either.

use serde::{Deserialize, Serialize};

/// The routing key of an element or a mutator: its kind, then its
/// channel.
///
/// The derived `Ord` compares `kind` first and `channel` second, which
/// is the order every parent's children are kept in. Both fields are
/// `Copy`, so comparing two keys is a couple of integer compares — no
/// string compare and no hashing on the walk.
///
/// Serialized as an object with two fields, `"kind"` and `"channel"`,
/// both required. An element type with a single kind uses `()`, whose
/// `"kind"` is written `null`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BranchKey<K> {
    /// The element kind this key routes to. Compared first.
    pub kind: K,
    /// The channel within that kind. Compared second.
    pub channel: usize,
}

impl<K> BranchKey<K> {
    /// The key of `kind` on `channel`. O(1).
    pub const fn new(kind: K, channel: usize) -> Self {
        BranchKey { kind, channel }
    }
}

/// The key of a single-kind element type on `channel`: an element type
/// whose `Kind` is `()` routes on its channel alone, so a channel is
/// all it takes to name one of its keys. O(1).
impl From<usize> for BranchKey<()> {
    fn from(channel: usize) -> Self {
        BranchKey { kind: (), channel }
    }
}
