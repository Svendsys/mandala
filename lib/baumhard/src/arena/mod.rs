// SPDX-License-Identifier: MPL-2.0

//! The arena tree every tree in Baumhard is stored in: the engine's
//! [`Tree`](crate::engine::tree::Tree) and
//! [`MutatorTree`](crate::engine::tree::MutatorTree), and through them
//! the document and the glyph trees.
//!
//! An [`Arena<T>`](crate::arena::Arena) holds nodes in one `Vec`; a
//! [`NodeId`](crate::arena::NodeId) names a node by its slot and the
//! slot's **generation**. Each node links to its parent, its first and
//! last child and its previous and next sibling, so every one-step
//! move, every child iteration and every structural edit of one node is
//! O(1) and none of them allocates beyond the slot. The links are the
//! arena's alone: a node's value can be borrowed mutably
//! ([`Arena::get_mut`](crate::arena::Arena::get_mut)), the node itself
//! never, so no code outside this module can rewrite a link.
//!
//! # Generational ids
//!
//! Removing a node frees its slot for a later node, and moves the slot
//! to its next generation. A [`NodeId`](crate::arena::NodeId) matches
//! its slot only while the generation it was handed out with is the
//! slot's, so an id kept past its node's removal is **stale**:
//! [`Arena::get`](crate::arena::Arena::get) answers `None` for it,
//! [`NodeId::is_removed`](crate::arena::NodeId::is_removed) `true`, a
//! structural operation refuses it (`ArenaError::Stale`), and indexing
//! with it panics naming the id — it never silently reads the node that
//! took the slot since. Free slots are reused oldest first. A slot
//! whose generation has reached `u32::MAX` is retired rather than
//! reused, so no generation is ever handed out twice and no id can ever
//! alias another node, however often a slot is recycled.
//!
//! # Compact links (V1-A7, #301)
//!
//! A link is a bare slot number, a `NonZeroU32`, not a `NodeId`: a
//! node's five links take 20 bytes on every target. A link written by
//! the arena names a live node by construction — both ends of every
//! link change together, and a node is unlinked before its slot is
//! freed — so a link carries no generation and is never checked
//! against one. The generation is checked where an id **enters** the
//! arena: [`Arena::get`](crate::arena::Arena::get), indexing,
//! [`Arena::get_mut`](crate::arena::Arena::get_mut), and every
//! [`NodeId`](crate::arena::NodeId) method's `self` and arguments —
//! once each per call. A link leaves the arena as a `NodeId` again
//! carrying its slot's current generation, read beside the link. For
//! that reason a node's links are read through `NodeId`
//! (`id.parent(&arena)`), never from a bare
//! [`Node`](crate::arena::Node), which has no arena to read a
//! generation from. A [`NodeRef`](crate::arena::NodeRef)
//! ([`Arena::node`](crate::arena::Arena::node),
//! [`Arena::at`](crate::arena::Arena::at)) reads a node's value and
//! all its links behind one check.
//!
//! An arena holds at most [`MAX_SLOTS`](crate::arena::MAX_SLOTS) slots,
//! `u32::MAX - 1`, so that every slot number fits a link; the slot past
//! them is refused with a panic naming the bound, never a slot number
//! that wraps.
//!
//! # Inserting an existing node
//!
//! A node created by the insert (`append_value`, `prepend_value`,
//! `insert_after_value`, `insert_before_value`) cannot close a cycle,
//! and goes in in O(1). A node that already exists is moved:
//!
//! - **under another parent** by the checked inserts
//!   ([`checked_append`](crate::arena::NodeId::checked_append),
//!   [`checked_prepend`](crate::arena::NodeId::checked_prepend),
//!   [`checked_insert_after`](crate::arena::NodeId::checked_insert_after),
//!   [`checked_insert_before`](crate::arena::NodeId::checked_insert_before)),
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
//! position,
//! [`NodeId::remove_subtree`](crate::arena::NodeId::remove_subtree)
//! frees a subtree with a cursor — each node after its subtree, the
//! node itself last — and `Clone`, `Drop`, `PartialEq` and `Debug` of
//! an arena are those of its flat `Vec` of slots.
//! [`NodeId::ancestors`](crate::arena::NodeId::ancestors) and the
//! checked inserts are the climbs, and they are O(depth) by what they
//! answer.
//!
//! # Allocation
//!
//! The arena's one allocation is its `Vec` of slots. No structural
//! operation allocates, and neither does any iterator; a value insert
//! does only when no slot is free and the `Vec` is full.
//! [`Arena::with_capacity`](crate::arena::Arena::with_capacity) and
//! [`Arena::reserve`](crate::arena::Arena::reserve) make that room up
//! front, for a caller that knows how many nodes are coming.
//!
//! No `unsafe` (the crate forbids it, CONVENTIONS §B7).

#[cfg(test)]
mod internal_tests;
mod iter;
mod link;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;

pub use iter::{Ancestors, ChildrenOf, Descendants, LiveNodes, NodeEdge, Traverse};

use std::fmt;
use std::num::NonZeroU32;
use std::ops::Index;

/// The most slots an [`Arena`] holds — its live nodes and its free and
/// retired slots together: `u32::MAX - 1`. Every slot number, counted
/// from 1, then fits the `NonZeroU32` a link is, with `u32::MAX` itself
/// never used. A new node that would need one more slot panics (see
/// [`Arena::new_node`]).
pub const MAX_SLOTS: usize = (u32::MAX - 1) as usize;

/// A link: the slot number, counted from 1, of a node the arena has
/// linked to — live by construction, so carrying no generation.
type Link = NonZeroU32;

/// A node's identity in an [`Arena`]: its slot and the slot's
/// generation when the node was created (see the module docs). `Copy`,
/// 8 bytes on every target, and ordered by slot, then generation, so
/// ids of an arena that has removed nothing order as their nodes were
/// created.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    /// The slot, counted from 1 so that `Option<NodeId>` is no larger
    /// than a `NodeId`.
    slot: Link,
    /// The slot's generation when this node was created.
    generation: u32,
}

impl NodeId {
    /// The slot's index in the arena's `Vec`.
    fn index0(self) -> usize {
        index0(self.slot)
    }
}

/// The slot number, counted from 1 — what a refusal names a node by.
impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.slot)
    }
}

/// `NodeId(slot@generation)`, the slot counted from 1.
impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({}@{})", self.slot, self.generation)
    }
}

/// One node of an [`Arena`]: its value and its links. Borrowed only
/// shared, through [`Arena::get`] or by indexing, and what it gives is
/// its value: its links are slot numbers that mean something only
/// beside the arena's generations, so they are read through
/// [`NodeId`]'s methods (`id.parent(&arena)`) or a [`NodeRef`]
/// ([`Arena::node`]), which hand each back as a `NodeId`. A node is
/// moved through [`NodeId`]'s structural operations, which keep both
/// ends of every link in step, and its value is changed through
/// [`Arena::get_mut`]. Not `Clone`: a node's links mean something only
/// in the slot it sits in.
#[derive(PartialEq, Eq, Debug)]
pub struct Node<T> {
    /// The slot's generation, which is this node's. Kept here, beside
    /// the five 4-byte links, rather than in the [`Slot`]: the six fill
    /// 24 bytes, so a slot of a value aligned to 8 spends no padding
    /// on its generation.
    generation: u32,
    parent: Option<Link>,
    previous_sibling: Option<Link>,
    next_sibling: Option<Link>,
    first_child: Option<Link>,
    last_child: Option<Link>,
    data: T,
}

impl<T> Node<T> {
    /// The node's value. O(1).
    pub fn get(&self) -> &T {
        &self.data
    }

    /// Whether this node hangs in nothing: no parent and no sibling.
    fn is_detached(&self) -> bool {
        self.parent.is_none() && self.previous_sibling.is_none() && self.next_sibling.is_none()
    }

    /// A node holding `data` at `generation`, linked to nothing.
    fn new(generation: u32, data: T) -> Self {
        Node {
            generation,
            parent: None,
            previous_sibling: None,
            next_sibling: None,
            first_child: None,
            last_child: None,
            data,
        }
    }
}

/// A live node read beside its arena: its id, its value, and its links
/// as [`NodeId`]s. From [`Arena::node`], which checks the id's
/// generation once; every link read here follows a link the arena
/// wrote, reading the far slot's generation and checking none. `Copy`:
/// two references and an id.
pub struct NodeRef<'a, T> {
    arena: &'a Arena<T>,
    node: &'a Node<T>,
    id: NodeId,
}

/// Not derived: a derive would ask `T: Clone`. O(1).
impl<T> Clone for NodeRef<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NodeRef<'_, T> {}

impl<'a, T> NodeRef<'a, T> {
    /// The node's id. O(1).
    pub fn id(self) -> NodeId {
        self.id
    }

    /// The node's value. O(1).
    pub fn get(self) -> &'a T {
        &self.node.data
    }

    /// The node's parent, `None` for the top of a tree or a detached
    /// node. O(1).
    pub fn parent(self) -> Option<NodeId> {
        self.node.parent.map(|link| self.arena.id_of(link))
    }

    /// The node's first child. O(1).
    pub fn first_child(self) -> Option<NodeId> {
        self.node.first_child.map(|link| self.arena.id_of(link))
    }

    /// The node's last child. O(1).
    pub fn last_child(self) -> Option<NodeId> {
        self.node.last_child.map(|link| self.arena.id_of(link))
    }

    /// The sibling before this node. O(1).
    pub fn previous_sibling(self) -> Option<NodeId> {
        self.node.previous_sibling.map(|link| self.arena.id_of(link))
    }

    /// The sibling after this node. O(1).
    pub fn next_sibling(self) -> Option<NodeId> {
        self.node.next_sibling.map(|link| self.arena.id_of(link))
    }
}

/// What a slot holds, with its generation: the live node's (in the
/// [`Node`]), or for a free slot the generation the next node to take
/// it will have.
#[derive(PartialEq, Eq, Debug)]
enum SlotEntry<T> {
    /// A node.
    Live(Node<T>),
    /// Nothing.
    Free {
        /// The generation the next node in this slot will have; for a
        /// retired slot, the last one it had.
        generation: u32,
        /// The slot number of the next free slot, oldest first, when
        /// this one is on the free list.
        next: Option<Link>,
    },
}

/// One slot of an [`Arena`]: what it holds. Its size is pinned on both
/// targets (`arena::tests::layout_tests`).
#[derive(PartialEq, Eq, Debug)]
struct Slot<T> {
    entry: SlotEntry<T>,
}

impl<T> Slot<T> {
    /// The slot's generation, live or free.
    fn generation(&self) -> u32 {
        match &self.entry {
            SlotEntry::Live(node) => node.generation,
            SlotEntry::Free { generation, .. } => *generation,
        }
    }
}

/// A copy of the slot, links and all — which is why a whole arena
/// clones and a lone [`Node`] does not. O(1) beyond the value's clone.
impl<T: Clone> Clone for Slot<T> {
    fn clone(&self) -> Self {
        let entry = match &self.entry {
            SlotEntry::Live(node) => SlotEntry::Live(Node {
                generation: node.generation,
                parent: node.parent,
                previous_sibling: node.previous_sibling,
                next_sibling: node.next_sibling,
                first_child: node.first_child,
                last_child: node.last_child,
                data: node.data.clone(),
            }),
            SlotEntry::Free { generation, next } => SlotEntry::Free {
                generation: *generation,
                next: *next,
            },
        };
        Slot { entry }
    }
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
    first_free: Option<Link>,
    /// The newest free slot: where a freed slot joins the list.
    last_free: Option<Link>,
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

    /// An empty arena with room for at least `capacity` nodes before
    /// its `Vec` of slots grows. Allocates that room, once, up front.
    pub fn with_capacity(capacity: usize) -> Self {
        Arena {
            slots: Vec::with_capacity(capacity),
            first_free: None,
            last_free: None,
            live: 0,
        }
    }

    /// Make room for at least `additional` more slots than the arena
    /// holds, so that many new nodes go in without the `Vec` of slots
    /// growing — free slots are reused first, so it may be more room
    /// than they need. At most one allocation, of the whole new room;
    /// none when there is room already.
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// How many slots the arena has room for before its `Vec` of slots
    /// grows. O(1).
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// A new detached node holding `data`, and its id. It takes the
    /// oldest free slot, or a new one at the end. O(1) amortized: a
    /// `Vec` push when no slot is free, which allocates only when the
    /// `Vec` is full.
    ///
    /// **Panics** when no slot is free and the arena already holds
    /// [`MAX_SLOTS`] slots.
    #[track_caller]
    pub fn new_node(&mut self, data: T) -> NodeId {
        if let Some(link) = self.first_free {
            let slot = &mut self.slots[index0(link)];
            if let SlotEntry::Free { generation, next } = slot.entry {
                slot.entry = SlotEntry::Live(Node::new(generation, data));
                self.first_free = next;
                if next.is_none() {
                    self.last_free = None;
                }
                self.live += 1;
                return NodeId {
                    slot: link,
                    generation,
                };
            }
        }
        let link = next_slot(self.slots.len());
        self.slots.push(Slot {
            entry: SlotEntry::Live(Node::new(0, data)),
        });
        self.live += 1;
        NodeId {
            slot: link,
            generation: 0,
        }
    }

    /// The node `id` names, or `None` when it is stale. O(1): the one
    /// generation check of the id.
    pub fn get(&self, id: NodeId) -> Option<&Node<T>> {
        self.resolve(id)
    }

    /// The node `id` names, read with its links, or `None` when it is
    /// stale. O(1): the one generation check of the id; the
    /// [`NodeRef`]'s link reads check nothing more.
    pub fn node(&self, id: NodeId) -> Option<NodeRef<'_, T>> {
        self.resolve(id).map(|node| NodeRef {
            arena: self,
            node,
            id,
        })
    }

    /// [`Self::node`] for an id that must be live: indexing's read, with
    /// the links. O(1), one generation check.
    ///
    /// **Panics** when `id` is stale, as indexing does.
    #[track_caller]
    pub fn at(&self, id: NodeId) -> NodeRef<'_, T> {
        NodeRef {
            arena: self,
            node: self.expect(id),
            id,
        }
    }

    /// The value of the node `id` names, mutably, or `None` when it is
    /// stale. The value only: the node's links are not reachable
    /// mutably outside the arena. O(1).
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        self.resolve_mut(id).map(|node| &mut node.data)
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
    /// tree or detached alike; free and retired slots are skipped. Its
    /// length is known exactly ([`Arena::live_count`]). O(slots) to run
    /// to the end; no allocation.
    pub fn iter(&self) -> LiveNodes<'_, T> {
        LiveNodes::new(self)
    }

    /// Whether the arena holds no slot at all. O(1).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The node `id` names, or `None` when it is stale: **the**
    /// generation check, which every id from outside goes through once
    /// (counted in test builds).
    fn resolve(&self, id: NodeId) -> Option<&Node<T>> {
        #[cfg(test)]
        tests_support::count_generation_check();
        match self.slots.get(id.index0()) {
            Some(Slot {
                entry: SlotEntry::Live(node),
            }) if node.generation == id.generation => Some(node),
            _ => None,
        }
    }

    /// [`Self::resolve`], mutably. Private: a link written through it
    /// must be written at both its ends.
    fn resolve_mut(&mut self, id: NodeId) -> Option<&mut Node<T>> {
        #[cfg(test)]
        tests_support::count_generation_check();
        match self.slots.get_mut(id.index0()) {
            Some(Slot {
                entry: SlotEntry::Live(node),
            }) if node.generation == id.generation => Some(node),
            _ => None,
        }
    }

    /// The live node `id` names; panics naming it when it is stale.
    #[track_caller]
    fn expect(&self, id: NodeId) -> &Node<T> {
        match self.resolve(id) {
            Some(node) => node,
            None => stale(id),
        }
    }

    /// The node at the far end of a link the arena wrote, its generation
    /// beside its links: one bounds-checked read of the slot and one
    /// match of what it holds, which a link always finds live — no
    /// generation is compared (counted in test builds).
    fn linked(&self, link: Link) -> &Node<T> {
        #[cfg(test)]
        tests_support::count_slot_read();
        match &self.slots[index0(link)].entry {
            SlotEntry::Live(node) => node,
            SlotEntry::Free { .. } => dangling(link),
        }
    }

    /// [`Self::linked`], mutably.
    fn linked_mut(&mut self, link: Link) -> &mut Node<T> {
        #[cfg(test)]
        tests_support::count_slot_read();
        match &mut self.slots[index0(link)].entry {
            SlotEntry::Live(node) => node,
            SlotEntry::Free { .. } => dangling(link),
        }
    }

    /// The id of the node at the far end of a link the arena wrote: the
    /// link and its node's generation, read beside it.
    fn id_of(&self, link: Link) -> NodeId {
        NodeId {
            slot: link,
            generation: self.linked(link).generation,
        }
    }

    /// Free the slot `link` names: drop its value, and move the slot to
    /// its next generation and onto the free list — or, at the last
    /// generation, retire it. The caller has checked the id the link
    /// came from and unlinked the node.
    fn free_link(&mut self, link: Link) {
        let slot = &mut self.slots[index0(link)];
        let generation = slot.generation();
        self.live -= 1;
        if generation == u32::MAX {
            // Retired: a further generation would repeat one already
            // handed out, so the slot is never reused.
            slot.entry = SlotEntry::Free {
                generation,
                next: None,
            };
            return;
        }
        slot.entry = SlotEntry::Free {
            generation: generation + 1,
            next: None,
        };
        match self.last_free {
            Some(last) => {
                if let SlotEntry::Free { next, .. } = &mut self.slots[index0(last)].entry {
                    *next = Some(link);
                }
            }
            None => self.first_free = Some(link),
        }
        self.last_free = Some(link);
    }

    /// Free `id`'s slot, as [`Self::free_link`] does; a stale `id` is
    /// ignored. The caller has unlinked the node.
    #[cfg(test)]
    fn free(&mut self, id: NodeId) {
        if self.resolve(id).is_some() {
            self.free_link(id.slot);
        }
    }

    /// Set the generation of a fresh slot, so a test can reach the last
    /// generation without recycling a slot four billion times.
    #[cfg(test)]
    fn set_next_generation(&mut self, index0: usize, generation: u32) {
        match &mut self.slots[index0].entry {
            SlotEntry::Live(node) => node.generation = generation,
            SlotEntry::Free { generation: next, .. } => *next = generation,
        }
    }
}

/// The slot's index in the arena's `Vec`.
fn index0(link: Link) -> usize {
    link.get() as usize - 1
}

/// The slot number of a new slot pushed after `len` slots: `len + 1`.
/// **Panics** when the arena already holds [`MAX_SLOTS`] slots, naming
/// the bound, rather than handing out a slot number that wraps.
#[track_caller]
fn next_slot(len: usize) -> Link {
    match u32::try_from(len) {
        Ok(len) if (len as usize) < MAX_SLOTS => {
            Link::new(len + 1).expect("a slot number past a u32 length is never zero")
        }
        _ => panic!("arena: full: an arena holds at most MAX_SLOTS ({MAX_SLOTS}) slots"),
    }
}

/// The panic of indexing with a stale id.
#[track_caller]
fn stale(id: NodeId) -> ! {
    panic!("arena: node {id:?} is not in the arena (removed, or its slot reused since)")
}

/// A link the arena wrote names a free slot: a broken invariant of this
/// module, never a caller's mistake.
#[track_caller]
fn dangling(link: Link) -> ! {
    panic!("arena: a link names slot {link}, which is free: a link was left behind when its node was freed")
}

/// The node `id` names. **Panics** when `id` is stale — a removed
/// node's id never reads the node that took its slot since. Use
/// [`Arena::get`] where the id may be stale.
impl<T> Index<NodeId> for Arena<T> {
    type Output = Node<T>;

    #[track_caller]
    fn index(&self, id: NodeId) -> &Node<T> {
        self.expect(id)
    }
}

/// What the tests read of the arena's work, counted on this thread:
///
/// - **insert steps**: how many nodes the inserts of an existing node
///   have gone through — one for each node a same-row re-attach links,
///   and for a checked insert one plus every node its cycle check
///   climbs through;
/// - **generation checks**: how many ids from outside the arena have
///   been checked against their slot's generation;
/// - **slot reads**: how many links have been followed to the slot
///   they name.
///
/// Compiled for tests only.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::cell::Cell;

    thread_local! {
        static INSERT_STEPS: Cell<usize> = const { Cell::new(0) };
        static GENERATION_CHECKS: Cell<usize> = const { Cell::new(0) };
        static SLOT_READS: Cell<usize> = const { Cell::new(0) };
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

    /// One more id checked against its slot's generation.
    pub(super) fn count_generation_check() {
        GENERATION_CHECKS.with(|cell| cell.set(cell.get() + 1));
    }

    /// The ids checked against their slot's generation on this thread so
    /// far.
    pub(crate) fn generation_checks() -> usize {
        GENERATION_CHECKS.with(Cell::get)
    }

    /// One more link followed to its slot.
    pub(super) fn count_slot_read() {
        SLOT_READS.with(|cell| cell.set(cell.get() + 1));
    }

    /// The links followed to their slot on this thread so far.
    pub(crate) fn slot_reads() -> usize {
        SLOT_READS.with(Cell::get)
    }
}
