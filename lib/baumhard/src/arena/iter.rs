// SPDX-License-Identifier: MPL-2.0

//! The arena's iterators. Each carries its position and reads one link
//! per step, so none of them recurses or allocates; the walks below a
//! node ([`Descendants`], [`Traverse`]) go through each node of the
//! subtree a bounded number of times, whatever its depth.

use crate::arena::{Arena, NodeId};

/// `start` and every node above it, nearest first, up to the top of
/// whatever it hangs in. From [`NodeId::ancestors`]. O(1) per step, so
/// a whole climb is O(depth).
#[derive(Clone)]
pub struct Ancestors<'a, T> {
    arena: &'a Arena<T>,
    next: Option<NodeId>,
}

impl<'a, T> Ancestors<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, start: NodeId) -> Self {
        Ancestors {
            arena,
            next: arena.get(start).map(|_| start),
        }
    }
}

impl<T> Iterator for Ancestors<'_, T> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next.take()?;
        self.next = self.arena.get(current).and_then(|node| node.parent);
        Some(current)
    }
}

impl<T> std::iter::FusedIterator for Ancestors<'_, T> {}

/// A node's children, first to last, and from the back last to first.
/// From [`NodeId::children`]. O(1) per step.
#[derive(Clone)]
pub struct ChildrenOf<'a, T> {
    arena: &'a Arena<T>,
    /// The next child from the front; `None` once the two ends meet.
    head: Option<NodeId>,
    /// The next child from the back.
    tail: Option<NodeId>,
}

impl<'a, T> ChildrenOf<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, parent: NodeId) -> Self {
        let node = arena.get(parent);
        ChildrenOf {
            arena,
            head: node.and_then(|node| node.first_child),
            tail: node.and_then(|node| node.last_child),
        }
    }
}

impl<T> Iterator for ChildrenOf<'_, T> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.head?;
        if Some(current) == self.tail {
            self.head = None;
            self.tail = None;
        } else {
            self.head = self.arena.get(current).and_then(|node| node.next_sibling);
            if self.head.is_none() {
                self.tail = None;
            }
        }
        Some(current)
    }
}

impl<T> DoubleEndedIterator for ChildrenOf<'_, T> {
    fn next_back(&mut self) -> Option<NodeId> {
        let current = self.tail?;
        if Some(current) == self.head {
            self.head = None;
            self.tail = None;
        } else {
            self.tail = self.arena.get(current).and_then(|node| node.previous_sibling);
            if self.tail.is_none() {
                self.head = None;
            }
        }
        Some(current)
    }
}

impl<T> std::iter::FusedIterator for ChildrenOf<'_, T> {}

/// A node and every node below it, in depth-first pre-order with
/// children first to last. From [`NodeId::descendants`].
///
/// A step goes down to a first child, across to a next sibling, or up
/// through the ancestors that have no next sibling to the first that
/// does; each link is followed at most once down and once up over the
/// whole walk, so a full walk is O(subtree), and no stack is kept.
#[derive(Clone)]
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
}

impl<T> std::iter::FusedIterator for Descendants<'_, T> {}

/// One side of a node in a depth-first walk: where its subtree starts,
/// or where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeEdge {
    /// Before the node's descendants.
    Start(NodeId),
    /// After the node's descendants.
    End(NodeId),
}

/// The two sides of a node and of every node below it, in depth-first
/// order with children first to last: each node's
/// [`Start`](NodeEdge::Start), then its children's edges, then its
/// [`End`](NodeEdge::End). From [`NodeId::traverse`]. O(1) per step,
/// O(subtree) in all; no stack is kept.
#[derive(Clone)]
pub struct Traverse<'a, T> {
    arena: &'a Arena<T>,
    top: NodeId,
    next: Option<NodeEdge>,
}

impl<'a, T> Traverse<'a, T> {
    pub(super) fn new(arena: &'a Arena<T>, top: NodeId) -> Self {
        Traverse {
            arena,
            top,
            next: arena.get(top).map(|_| NodeEdge::Start(top)),
        }
    }

    /// The edge after `edge`, or `None` once the top's end is reached.
    fn after(&self, edge: NodeEdge) -> Option<NodeEdge> {
        match edge {
            NodeEdge::Start(id) => {
                let node = self.arena.get(id)?;
                Some(match node.first_child {
                    Some(child) => NodeEdge::Start(child),
                    None => NodeEdge::End(id),
                })
            }
            NodeEdge::End(id) if id == self.top => None,
            NodeEdge::End(id) => {
                let node = self.arena.get(id)?;
                match node.next_sibling {
                    Some(next) => Some(NodeEdge::Start(next)),
                    None => node.parent.map(NodeEdge::End),
                }
            }
        }
    }
}

impl<T> Iterator for Traverse<'_, T> {
    type Item = NodeEdge;

    fn next(&mut self) -> Option<NodeEdge> {
        let current = self.next.take()?;
        self.next = self.after(current);
        Some(current)
    }
}

impl<T> std::iter::FusedIterator for Traverse<'_, T> {}
