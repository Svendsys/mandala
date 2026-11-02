// SPDX-License-Identifier: MPL-2.0

//! The arena's iterators. Each carries its position and reads one slot
//! per step, so none of them recurses or allocates; the walks below a
//! node ([`Descendants`], [`Traverse`]) go through each node of the
//! subtree a bounded number of times, whatever its depth.
//!
//! The id an iterator starts from is checked against its generation
//! once, when the iterator is made; every step after follows a link the
//! arena wrote, which needs no check (see the module docs of
//! [`crate::arena`]). Every one is a `FusedIterator`, [`ChildrenOf`] is
//! double-ended, [`LiveNodes`] knows its length exactly, and the others
//! give the bounds they know: at least one more while they have a next,
//! at most the arena's live nodes (twice that for [`Traverse`]).

use crate::arena::{Arena, Link, Node, NodeId, Slot, SlotEntry};
use std::iter::FusedIterator;

/// The `size_hint` of a walk with `next` still to come, over an arena
/// of `live` nodes, each node yielded at most `per_node` times.
fn walk_hint<L>(next: Option<L>, live: usize, per_node: usize) -> (usize, Option<usize>) {
    match next {
        Some(_) => (1, Some(live.saturating_mul(per_node))),
        None => (0, Some(0)),
    }
}

/// `start` and every node above it, nearest first, up to the top of
/// whatever it hangs in. From [`NodeId::ancestors`]. O(1) per step, so
/// a whole climb is O(depth).
pub struct Ancestors<'a, T> {
    arena: &'a Arena<T>,
    next: Option<Link>,
}

impl<'a, T> Ancestors<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, start: NodeId) -> Self {
        Ancestors {
            arena,
            next: arena.resolve(start).map(|_| start.slot),
        }
    }
}

/// Not derived: a derive would ask `T: Clone`, and the iterator holds
/// only a shared borrow of the arena. O(1).
impl<T> Clone for Ancestors<'_, T> {
    fn clone(&self) -> Self {
        Ancestors {
            arena: self.arena,
            next: self.next,
        }
    }
}

impl<T> Iterator for Ancestors<'_, T> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next.take()?;
        let node = self.arena.linked(current);
        let generation = node.generation;
        self.next = node.parent;
        Some(NodeId {
            slot: current,
            generation,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        walk_hint(self.next, self.arena.live, 1)
    }
}

impl<T> FusedIterator for Ancestors<'_, T> {}

/// A node's children, first to last, and from the back last to first.
/// From [`NodeId::children`]. O(1) per step.
pub struct ChildrenOf<'a, T> {
    arena: &'a Arena<T>,
    /// The next child from the front; `None` once the two ends meet.
    head: Option<Link>,
    /// The next child from the back.
    tail: Option<Link>,
}

impl<'a, T> ChildrenOf<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, parent: NodeId) -> Self {
        let node = arena.resolve(parent);
        ChildrenOf {
            arena,
            head: node.and_then(|node| node.first_child),
            tail: node.and_then(|node| node.last_child),
        }
    }
}

/// Not derived: a derive would ask `T: Clone`, and the iterator holds
/// only a shared borrow of the arena. O(1).
impl<T> Clone for ChildrenOf<'_, T> {
    fn clone(&self) -> Self {
        ChildrenOf {
            arena: self.arena,
            head: self.head,
            tail: self.tail,
        }
    }
}

impl<T> Iterator for ChildrenOf<'_, T> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.head?;
        let node = self.arena.linked(current);
        let generation = node.generation;
        if Some(current) == self.tail {
            self.head = None;
            self.tail = None;
        } else {
            self.head = node.next_sibling;
            if self.head.is_none() {
                self.tail = None;
            }
        }
        Some(NodeId {
            slot: current,
            generation,
        })
    }

    /// Exact once the two ends meet on the last child; otherwise at
    /// least one, and at most every live node but the parent.
    fn size_hint(&self) -> (usize, Option<usize>) {
        match (self.head, self.tail) {
            (Some(head), Some(tail)) if head == tail => (1, Some(1)),
            (head, _) => walk_hint(head, self.arena.live.saturating_sub(1), 1),
        }
    }
}

impl<T> DoubleEndedIterator for ChildrenOf<'_, T> {
    fn next_back(&mut self) -> Option<NodeId> {
        let current = self.tail?;
        let node = self.arena.linked(current);
        let generation = node.generation;
        if Some(current) == self.head {
            self.head = None;
            self.tail = None;
        } else {
            self.tail = node.previous_sibling;
            if self.tail.is_none() {
                self.head = None;
            }
        }
        Some(NodeId {
            slot: current,
            generation,
        })
    }
}

impl<T> FusedIterator for ChildrenOf<'_, T> {}

/// A node and every node below it, in depth-first pre-order with
/// children first to last. From [`NodeId::descendants`].
///
/// A step goes down to a first child, across to a next sibling, or up
/// through the ancestors that have no next sibling to the first that
/// does; each link is followed at most once down and once up over the
/// whole walk, so a full walk is O(subtree), and no stack is kept.
pub struct Descendants<'a, T> {
    traverse: Traverse<'a, T>,
}

impl<'a, T> Descendants<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, top: NodeId) -> Self {
        Descendants {
            traverse: Traverse::new(arena, top),
        }
    }
}

/// Not derived: a derive would ask `T: Clone`, and the iterator holds
/// only a shared borrow of the arena. O(1).
impl<T> Clone for Descendants<'_, T> {
    fn clone(&self) -> Self {
        Descendants {
            traverse: self.traverse.clone(),
        }
    }
}

impl<T> Iterator for Descendants<'_, T> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        loop {
            match self.traverse.next()? {
                NodeEdge::Start(id) => return Some(id),
                NodeEdge::End(_) => continue,
            }
        }
    }

    /// At least one while a node's start is next, at most every live
    /// node.
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.traverse.next {
            Some(Edge::Start(_)) => (1, Some(self.traverse.arena.live)),
            _ => (0, Some(self.traverse.arena.live)),
        }
    }
}

impl<T> FusedIterator for Descendants<'_, T> {}

/// One side of a node in a depth-first walk: where its subtree starts,
/// or where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeEdge {
    /// Before the node's descendants.
    Start(NodeId),
    /// After the node's descendants.
    End(NodeId),
}

/// A [`NodeEdge`] by link: what [`Traverse`] holds between steps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start(Link),
    End(Link),
}

/// The two sides of a node and of every node below it, in depth-first
/// order with children first to last: each node's
/// [`Start`](NodeEdge::Start), then its children's edges, then its
/// [`End`](NodeEdge::End). From [`NodeId::traverse`]. O(1) per step,
/// O(subtree) in all; no stack is kept.
pub struct Traverse<'a, T> {
    arena: &'a Arena<T>,
    top: Link,
    next: Option<Edge>,
}

impl<'a, T> Traverse<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, top: NodeId) -> Self {
        Traverse {
            arena,
            top: top.slot,
            next: arena.resolve(top).map(|_| Edge::Start(top.slot)),
        }
    }
}

/// Not derived: a derive would ask `T: Clone`, and the iterator holds
/// only a shared borrow of the arena. O(1).
impl<T> Clone for Traverse<'_, T> {
    fn clone(&self) -> Self {
        Traverse {
            arena: self.arena,
            top: self.top,
            next: self.next,
        }
    }
}

impl<T> Iterator for Traverse<'_, T> {
    type Item = NodeEdge;

    /// The edge, and the one after it from the same slot read: after a
    /// start, the first child's start or the node's own end; after an
    /// end, the next sibling's start or the parent's end — or nothing,
    /// at the top's end.
    fn next(&mut self) -> Option<NodeEdge> {
        let current = self.next.take()?;
        match current {
            Edge::Start(link) => {
                let node = self.arena.linked(link);
                let generation = node.generation;
                self.next = Some(match node.first_child {
                    Some(child) => Edge::Start(child),
                    None => Edge::End(link),
                });
                Some(NodeEdge::Start(NodeId {
                    slot: link,
                    generation,
                }))
            }
            Edge::End(link) => {
                let node = self.arena.linked(link);
                let generation = node.generation;
                if link != self.top {
                    self.next = match node.next_sibling {
                        Some(next) => Some(Edge::Start(next)),
                        None => node.parent.map(Edge::End),
                    };
                }
                Some(NodeEdge::End(NodeId {
                    slot: link,
                    generation,
                }))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        walk_hint(self.next, self.arena.live, 2)
    }
}

impl<T> FusedIterator for Traverse<'_, T> {}

/// Every node of an arena with its id, in slot order. From
/// [`Arena::iter`]. Its length is exact: the arena's live nodes not yet
/// yielded. O(1) per slot passed; free and retired slots are skipped.
pub struct LiveNodes<'a, T> {
    slots: std::iter::Enumerate<std::slice::Iter<'a, Slot<T>>>,
    /// The live nodes not yet yielded.
    remaining: usize,
}

impl<'a, T> LiveNodes<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>) -> Self {
        LiveNodes {
            slots: arena.slots.iter().enumerate(),
            remaining: arena.live,
        }
    }
}

/// Not derived: a derive would ask `T: Clone`. O(1).
impl<T> Clone for LiveNodes<'_, T> {
    fn clone(&self) -> Self {
        LiveNodes {
            slots: self.slots.clone(),
            remaining: self.remaining,
        }
    }
}

impl<'a, T> Iterator for LiveNodes<'a, T> {
    type Item = (NodeId, &'a Node<T>);

    fn next(&mut self) -> Option<Self::Item> {
        for (index0, slot) in self.slots.by_ref() {
            if let SlotEntry::Live(node) = &slot.entry {
                self.remaining -= 1;
                let id = NodeId {
                    slot: Link::new(index0 as u32 + 1).expect("a slot number counted from 1 is never zero"),
                    generation: node.generation,
                };
                return Some((id, node));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for LiveNodes<'_, T> {}

impl<T> FusedIterator for LiveNodes<'_, T> {}
