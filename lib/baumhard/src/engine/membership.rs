// SPDX-License-Identifier: MPL-2.0

//! [`Membership`](crate::engine::membership::Membership): whether
//! elements are in the tree — rather than in a subtree a journal holds
//! detached — for many elements at once, at a cost that does not grow
//! with depth per element (design §20 rule 11).
//!
//! [`Tree::contains`](crate::engine::tree::Tree::contains) answers in
//! O(1) from the tree's set of detached nodes; a `Membership` answers by
//! climbing, and is what the engine uses where it needs the climb
//! itself — the union of the paths above a set of anchors, which orders
//! them. A `Membership` remembers the answer for every element it
//! climbs through, and a later climb stops at the first element already
//! answered, so each ancestor is climbed through once over all the
//! questions it is asked.

use crate::engine::element::TreeElement;
use crate::engine::tree::Tree;
use indextree::NodeId;
use rustc_hash::FxHashMap;

/// A memo of which elements are in a tree. Holds no borrow of the tree:
/// every question passes the tree it is about, which must be the same
/// tree, unchanged, for the memo's whole life.
#[derive(Debug, Default)]
pub(crate) struct Membership {
    known: FxHashMap<NodeId, bool>,
    /// The elements of the climb in progress, reused between climbs.
    climb: Vec<NodeId>,
    /// How many elements every climb so far went through: what the
    /// tests hold to the number of distinct elements (§20 rule 11).
    climbed: usize,
}

impl Membership {
    /// Whether `id` is in `tree`: in its arena, and under its root.
    ///
    /// Cost: amortized O(1) per element over every question this memo
    /// answers — the climbs stop at the first answered element, and each
    /// element is answered once — plus a hash insert per element climbed
    /// through.
    pub(crate) fn contains<E: TreeElement>(&mut self, tree: &Tree<E>, id: NodeId) -> bool {
        let arena = tree.arena();
        if arena.get(id).is_none() {
            return false;
        }
        let mut at = Some(id);
        let answer = loop {
            let Some(node) = at else {
                // Climbed past a top that is not the root: detached.
                break false;
            };
            if let Some(&known) = self.known.get(&node) {
                break known;
            }
            if node == tree.root() {
                break true;
            }
            self.climb.push(node);
            self.climbed += 1;
            #[cfg(test)]
            tests_support::count_climb();
            at = arena[node].parent();
        };
        for node in self.climb.drain(..) {
            self.known.insert(node, answer);
        }
        answer
    }

    /// Every element found in the tree so far, the root excepted: the
    /// union of the paths up to the root from every element asked about
    /// that is in it. In no particular order. No allocation.
    pub(crate) fn members(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.known.iter().filter(|(_, &inside)| inside).map(|(&id, _)| id)
    }

    /// How many elements the climbs so far went through, each counted
    /// once per climb. O(1).
    pub(crate) fn climbed(&self) -> usize {
        self.climbed
    }
}

/// What the tests read of every [`Membership`]: how many elements the
/// climbs of all of them have gone through on this thread — including
/// the ones an engine call makes and drops internally, which
/// [`Membership::climbed`] cannot reach. Compiled for tests only.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::cell::Cell;

    thread_local! {
        static CLIMBED: Cell<usize> = const { Cell::new(0) };
    }

    /// One more element climbed through.
    pub(super) fn count_climb() {
        CLIMBED.with(|cell| cell.set(cell.get() + 1));
    }

    /// The elements every climb on this thread has gone through so far.
    pub(crate) fn climbed_steps() -> usize {
        CLIMBED.with(Cell::get)
    }
}
