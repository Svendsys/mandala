// SPDX-License-Identifier: MPL-2.0

//! [`Membership`](crate::document::mutation::membership::Membership):
//! whether elements are in the tree — rather than in a subtree a
//! journal holds detached — for many elements at once, at a cost that
//! does not grow with depth per element (design §20 rule 11).
//!
//! `Tree::contains` climbs from one element to the root. Asked for every
//! element of a selection, or every link of a request, that is the
//! depth paid once per element. A `Membership` remembers the answer for
//! every element it climbs through, and a later climb stops at the
//! first element already answered, so each ancestor is climbed through
//! once over all the questions it is asked.

use crate::document::element::DocElement;
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
}

impl Membership {
    /// Whether `id` is in `tree`: in its arena, and under its root.
    ///
    /// Cost: amortized O(1) per element over every question this memo
    /// answers — the climbs stop at the first answered element, and each
    /// element is answered once — plus a hash insert per element climbed
    /// through.
    pub(crate) fn contains(&mut self, tree: &Tree<DocElement>, id: NodeId) -> bool {
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
            at = arena[node].parent();
        };
        for node in self.climb.drain(..) {
            self.known.insert(node, answer);
        }
        answer
    }
}
