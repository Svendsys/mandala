// SPDX-License-Identifier: MPL-2.0

//! The arena tree every tree in Baumhard is stored in: the engine's
//! [`Tree`](crate::engine::tree::Tree) and
//! [`MutatorTree`](crate::engine::tree::MutatorTree), and through them
//! the document and the glyph trees.
//!
//! An [`Arena<T>`](crate::arena::Arena) holds nodes in one `Vec`; a [`NodeId`](crate::arena::NodeId) names a node
//! by its slot and the slot's **generation**. Each node links to its
//! parent, its first and last child and its previous and next sibling,
//! so every one-step move, every child iteration and every structural
//! edit of one node is O(1) and none of them allocates beyond the slot.
//!
//! # Generational ids
//!
//! Removing a node frees its slot for a later node, and moves the slot
//! to its next generation. A [`NodeId`](crate::arena::NodeId) matches its slot only while the
//! generation it was handed out with is the slot's, so an id kept past
//! its node's removal is **stale**: [`Arena::get`](crate::arena::Arena::get) answers `None` for
//! it, [`NodeId::is_removed`](crate::arena::NodeId::is_removed) `true`, and indexing with it panics
//! naming the id — it never silently reads the node that took the slot
//! since. A slot whose generation has reached `u32::MAX` is retired
//! rather than reused, so no generation is ever handed out twice and no
//! id can ever alias another node, however often a slot is recycled.
//!
//! # Inserting an existing node
//!
//! A node created by the insert (`append_value`, `prepend_value`,
//! `insert_after_value`, `insert_before_value`) cannot close a cycle,
//! and goes in in O(1). A node that already exists is moved:
//!
//! - **under another parent** by the checked inserts
//!   ([`NodeId::checked_append`](crate::arena::NodeId::checked_append), [`NodeId::checked_prepend`](crate::arena::NodeId::checked_prepend),
//!   [`NodeId::checked_insert_after`](crate::arena::NodeId::checked_insert_after), [`NodeId::checked_insert_before`](crate::arena::NodeId::checked_insert_before)),
//!   which refuse to put a node under itself or under one of its own
//!   descendants. Telling that is a climb from the destination to the
//!   top of its tree, O(depth), paid **once per moved subtree**, never
//!   once per node of it;
//! - **within the row it was detached from** by
//!   `NodeId::reattach_in_row`, crate-private and O(1): a node put back
//!   under the parent it came from cannot close a cycle, so nothing is
//!   climbed (ruling R16 on #282, V1-A6 #290). The engine calls it from
//!   one function, `engine::siblings::reattach_in_row`.
//!
//! # Depth
//!
//! Nothing here recurses, and nothing spends work per node that grows
//! with the depth (design §20 rule 11): the iterators carry their
//! position, [`NodeId::remove_subtree`](crate::arena::NodeId::remove_subtree) frees a subtree with a cursor,
//! and `Clone`, `Drop`, `PartialEq` and `Debug` of an arena are those
//! of its flat `Vec` of slots. [`NodeId::ancestors`](crate::arena::NodeId::ancestors) and the checked
//! inserts are the climbs, and they are O(depth) by what they answer.
//!
//! No `unsafe` (the crate forbids it, CONVENTIONS §B7).

#[cfg(test)]
mod internal_tests;
mod iter;
mod link;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;

pub use iter::{Ancestors, ChildrenOf, Descendants, NodeEdge, Traverse};

use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{Index, IndexMut};

/// A node's identity in an [`Arena`]: its slot and the slot's
/// generation when the node was created (see the module docs). `Copy`,
/// and ordered by slot, then generation, so ids of an arena that has
/// removed nothing order as their nodes were created.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    /// The slot, counted from 1 so that `Option<NodeId>` is no larger
    /// than a `NodeId`.
    index1: NonZeroUsize,
    /// The slot's generation when this node was created.
    generation: u32,
}

impl NodeId {
    /// The slot's index in the arena's `Vec`.
    fn index0(self) -> usize {
        self.index1.get() - 1
    }
}

/// The slot number, counted from 1 — what a refusal names a node by.
impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.index1)
    }
}

/// `NodeId(slot@generation)`, the slot counted from 1.
impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({}@{})", self.index1, self.generation)
    }
}

/// One node of an [`Arena`]: its value and its links. Borrowed through
/// [`Arena::get`] or by indexing; its links are read-only — a node is
/// moved through [`NodeId`]'s structural operations, which keep both
/// ends of every link in step.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node<T> {
    parent: Option<NodeId>,
    previous_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    data: T,
}

impl<T> Node<T> {
    /// The node's value. O(1).
    pub fn get(&self) -> &T {
        &self.data
    }

    /// The node's value, mutably. O(1).
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.data
    }

    /// The node's parent, `None` for the top of a tree or a detached
    /// node. O(1).
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// The node's first child. O(1).
    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    /// The node's last child. O(1).
    pub fn last_child(&self) -> Option<NodeId> {
        self.last_child
    }

    /// The sibling before this node. O(1).
    pub fn previous_sibling(&self) -> Option<NodeId> {
        self.previous_sibling
    }

    /// The sibling after this node. O(1).
    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling
    }

    /// Whether this node hangs in nothing: no parent and no sibling.
    fn is_detached(&self) -> bool {
        self.parent.is_none() && self.previous_sibling.is_none() && self.next_sibling.is_none()
    }

    /// A node holding `data`, linked to nothing.
    fn new(data: T) -> Self {
        Node {
            parent: None,
            previous_sibling: None,
            next_sibling: None,
            first_child: None,
            last_child: None,
            data,
        }
    }
}

/// What a slot holds.
#[derive(Clone, PartialEq, Eq, Debug)]
enum SlotEntry<T> {
    /// A node.
    Live(Node<T>),
    /// Nothing; the next free slot, oldest first, when this one is on
    /// the free list.
    Free(Option<usize>),
}

/// One slot of an [`Arena`]: what it holds, and its generation — the
/// live node's, or for a free slot the generation the next node to
/// take it will have.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Slot<T> {
    generation: u32,
    entry: SlotEntry<T>,
}

/// Why a structural operation on an [`Arena`] was refused. Every
/// refusal leaves the arena as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// The id names no node of the arena: it was removed, or its slot
    /// has been reused since.
    Stale(NodeId),
    /// A checked insert would have put `node` under itself or under one
    /// of its own descendants: `to` is `node` or in its subtree.
    Cycle {
        /// The node being moved.
        node: NodeId,
        /// The node it would have hung below.
        to: NodeId,
    },
    /// A sibling insert beside a node with no parent: a node is in a
    /// row only under a parent.
    NoParent(NodeId),
    /// A same-row re-attach was handed a node that still hangs in a
    /// row: it must be detached first.
    Attached(NodeId),
    /// A same-row re-attach named, as the sibling to follow, a node
    /// that is not a child of the given parent.
    NotAChild {
        /// The parent the node was to be re-attached under.
        parent: NodeId,
        /// The sibling named.
        node: NodeId,
    },
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArenaError::Stale(id) => write!(f, "node {id} is not in the arena"),
            ArenaError::Cycle { node, to } => {
                write!(
                    f,
                    "node {node} cannot hang below {to}, which is itself or in its subtree"
                )
            }
            ArenaError::NoParent(id) => {
                write!(f, "node {id} has no parent, so no row to insert beside it in")
            }
            ArenaError::Attached(id) => write!(f, "node {id} is still attached"),
            ArenaError::NotAChild { parent, node } => write!(f, "node {node} is not a child of {parent}"),
        }
    }
}

impl std::error::Error for ArenaError {}

/// An arena of nodes holding values `T`, linked into trees (see the
/// module docs). Any number of trees and detached nodes share one
/// arena; a [`NodeId`] reaches its node in O(1).
///
/// `Clone`, `PartialEq` and `Debug` are those of the slots, compared
/// and printed in slot order — a flat walk, however deep the trees, so
/// none of them recurses. Two arenas are equal when every slot holds
/// the same value under the same links and generation, and their free
/// lists agree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// The oldest free slot: the next a new node takes.
    first_free: Option<usize>,
    /// The newest free slot: where a freed slot joins the list.
    last_free: Option<usize>,
    /// How many slots hold a node.
    live: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    /// An empty arena. O(1); allocates nothing.
    pub const fn new() -> Self {
        Arena {
            slots: Vec::new(),
            first_free: None,
            last_free: None,
            live: 0,
        }
    }

    /// A new detached node holding `data`, and its id. It takes the
    /// oldest free slot, or a new one at the end. O(1) amortized: a
    /// `Vec` push when no slot is free.
    pub fn new_node(&mut self, data: T) -> NodeId {
        self.live += 1;
        if let Some(index0) = self.first_free {
            let slot = &mut self.slots[index0];
            if let SlotEntry::Free(next) = slot.entry {
                slot.entry = SlotEntry::Live(Node::new(data));
                self.first_free = next;
                if next.is_none() {
                    self.last_free = None;
                }
                return NodeId {
                    index1: one_based(index0),
                    generation: slot.generation,
                };
            }
        }
        self.slots.push(Slot {
            generation: 0,
            entry: SlotEntry::Live(Node::new(data)),
        });
        NodeId {
            index1: one_based(self.slots.len() - 1),
            generation: 0,
        }
    }

    /// The node `id` names, or `None` when it is stale. O(1).
    pub fn get(&self, id: NodeId) -> Option<&Node<T>> {
        match self.slots.get(id.index0()) {
            Some(Slot {
                generation,
                entry: SlotEntry::Live(node),
            }) if *generation == id.generation => Some(node),
            _ => None,
        }
    }

    /// The node `id` names, mutably, or `None` when it is stale. Only
    /// its value can be changed through it. O(1).
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node<T>> {
        match self.slots.get_mut(id.index0()) {
            Some(Slot {
                generation,
                entry: SlotEntry::Live(node),
            }) if *generation == id.generation => Some(node),
            _ => None,
        }
    }

    /// How many slots the arena holds: its live nodes and its free and
    /// retired slots. O(1).
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// How many nodes the arena holds — in a tree or detached — its free
    /// and retired slots not counted. O(1).
    pub fn live_count(&self) -> usize {
        self.live
    }

    /// Every node of the arena with its id, in slot order — the order
    /// nodes were created in, for an arena that has freed nothing. In a
    /// tree or detached alike; free and retired slots are skipped.
    /// O(slots) to run to the end; no allocation.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node<T>)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index0, slot)| match &slot.entry {
                SlotEntry::Live(node) => Some((
                    NodeId {
                        index1: one_based(index0),
                        generation: slot.generation,
                    },
                    node,
                )),
                SlotEntry::Free(_) => None,
            })
    }

    /// Whether the arena holds no slot at all. O(1).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Free `id`'s slot: drop its value, and move the slot to its next
    /// generation and onto the free list — or, at the last generation,
    /// retire it. The caller has unlinked the node. A stale `id` is
    /// ignored.
    fn free(&mut self, id: NodeId) {
        let Some(slot) = self.slots.get_mut(id.index0()) else {
            return;
        };
        if slot.generation != id.generation || !matches!(slot.entry, SlotEntry::Live(_)) {
            return;
        }
        slot.entry = SlotEntry::Free(None);
        self.live -= 1;
        if slot.generation == u32::MAX {
            // Retired: a further generation would repeat one already
            // handed out, so the slot is never reused.
            return;
        }
        slot.generation += 1;
        let index0 = id.index0();
        match self.last_free {
            Some(last) => {
                if let Some(Slot {
                    entry: SlotEntry::Free(next),
                    ..
                }) = self.slots.get_mut(last)
                {
                    *next = Some(index0);
                }
            }
            None => self.first_free = Some(index0),
        }
        self.last_free = Some(index0);
    }

    /// The live node `id` names; panics naming it when it is stale.
    #[track_caller]
    fn node(&self, id: NodeId) -> &Node<T> {
        match self.get(id) {
            Some(node) => node,
            None => stale(id),
        }
    }

    /// The live node `id` names, mutably; panics naming it when it is
    /// stale.
    #[track_caller]
    fn node_mut(&mut self, id: NodeId) -> &mut Node<T> {
        match self.get_mut(id) {
            Some(node) => node,
            None => stale(id),
        }
    }

    /// Set the generation of a fresh slot, so a test can reach the last
    /// generation without recycling a slot four billion times.
    #[cfg(test)]
    fn set_next_generation(&mut self, index0: usize, generation: u32) {
        self.slots[index0].generation = generation;
    }
}

/// `index0 + 1` as a slot number. A `Vec` of nodes — never zero-sized,
/// since each carries its links — cannot hold `usize::MAX` of them, so
/// the sum cannot wrap.
fn one_based(index0: usize) -> NonZeroUsize {
    NonZeroUsize::new(index0.wrapping_add(1)).expect("a Vec of nodes never holds usize::MAX slots")
}

/// The panic of indexing with a stale id.
#[track_caller]
fn stale(id: NodeId) -> ! {
    panic!("arena: node {id:?} is not in the arena (removed, or its slot reused since)")
}

/// The node `id` names. **Panics** when `id` is stale — a removed
/// node's id never reads the node that took its slot since. Use
/// [`Arena::get`] where the id may be stale.
impl<T> Index<NodeId> for Arena<T> {
    type Output = Node<T>;

    #[track_caller]
    fn index(&self, id: NodeId) -> &Node<T> {
        self.node(id)
    }
}

/// The node `id` names, mutably; panics as [`Index`] does.
impl<T> IndexMut<NodeId> for Arena<T> {
    #[track_caller]
    fn index_mut(&mut self, id: NodeId) -> &mut Node<T> {
        self.node_mut(id)
    }
}

/// What the tests read of the arena's inserts of existing nodes: how
/// many nodes they have gone through on this thread — one for each node
/// a same-row re-attach links, and for a checked insert one plus every
/// node its cycle check climbs through. Compiled for tests only.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::cell::Cell;

    thread_local! {
        static INSERT_STEPS: Cell<usize> = const { Cell::new(0) };
    }

    /// `steps` more nodes gone through by an insert of an existing node.
    pub(super) fn count_insert_steps(steps: usize) {
        INSERT_STEPS.with(|cell| cell.set(cell.get() + steps));
    }

    /// The nodes every insert of an existing node on this thread has
    /// gone through so far.
    pub(crate) fn insert_steps() -> usize {
        INSERT_STEPS.with(Cell::get)
    }
}
