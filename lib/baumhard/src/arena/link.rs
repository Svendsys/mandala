// SPDX-License-Identifier: MPL-2.0

//! What a [`NodeId`] does in its [`Arena`]: read its links, iterate
//! around it, and change the structure — detach, remove a subtree,
//! insert a new node beside or below it, and move an existing node,
//! checked against cycles under another parent and unchecked within the
//! row it left. Every operation here is O(1) but the iterators' full
//! runs, [`NodeId::remove_subtree`] (O(subtree)) and the checked
//! inserts' cycle check (O(depth of the destination)).

use crate::arena::iter::{Ancestors, ChildrenOf, Descendants, Traverse};
use crate::arena::{Arena, ArenaError, NodeId};

impl NodeId {
    /// Whether this id names no node of `arena`: its node was removed,
    /// or its slot has been reused since. O(1).
    pub fn is_removed<T>(self, arena: &Arena<T>) -> bool {
        arena.get(self).is_none()
    }

    /// The node's parent; `None` for the top of a tree, a detached node
    /// or a stale id. O(1).
    pub fn parent<T>(self, arena: &Arena<T>) -> Option<NodeId> {
        arena.get(self)?.parent
    }

    /// The node's first child. O(1).
    pub fn first_child<T>(self, arena: &Arena<T>) -> Option<NodeId> {
        arena.get(self)?.first_child
    }

    /// The node's last child. O(1).
    pub fn last_child<T>(self, arena: &Arena<T>) -> Option<NodeId> {
        arena.get(self)?.last_child
    }

    /// The sibling before the node. O(1).
    pub fn previous_sibling<T>(self, arena: &Arena<T>) -> Option<NodeId> {
        arena.get(self)?.previous_sibling
    }

    /// The sibling after the node. O(1).
    pub fn next_sibling<T>(self, arena: &Arena<T>) -> Option<NodeId> {
        arena.get(self)?.next_sibling
    }

    /// The node's children, first to last (double-ended). Empty for a
    /// stale id. O(1) per step; no allocation.
    pub fn children<T>(self, arena: &Arena<T>) -> ChildrenOf<'_, T> {
        ChildrenOf::new(arena, self)
    }

    /// The node and every node above it, nearest first. Empty for a
    /// stale id. **A climb:** O(depth) to run to the end — never once
    /// per node of a walk (design §20 rule 11). No allocation.
    pub fn ancestors<T>(self, arena: &Arena<T>) -> Ancestors<'_, T> {
        Ancestors::new(arena, self)
    }

    /// The node and every node below it, in depth-first pre-order,
    /// children first to last. Empty for a stale id. O(subtree) to run
    /// to the end, no allocation and no recursion.
    pub fn descendants<T>(self, arena: &Arena<T>) -> Descendants<'_, T> {
        Descendants::new(arena, self)
    }

    /// The start and end of the node and of every node below it, in
    /// depth-first order (see [`Traverse`]). Empty for a stale id.
    /// O(subtree) to run to the end, no allocation and no recursion.
    pub fn traverse<T>(self, arena: &Arena<T>) -> Traverse<'_, T> {
        Traverse::new(arena, self)
    }

    /// Take the node, with its subtree, out of the row it hangs in: it
    /// keeps its children and loses its parent and siblings. Its
    /// siblings close the gap. A detached node or a stale id is left as
    /// it is. O(1).
    pub fn detach<T>(self, arena: &mut Arena<T>) {
        let Some(node) = arena.live_node_mut(self) else {
            return;
        };
        let (parent, previous, next) = (node.parent, node.previous_sibling, node.next_sibling);
        node.parent = None;
        node.previous_sibling = None;
        node.next_sibling = None;
        match previous {
            Some(previous) => arena.node_mut(previous).next_sibling = next,
            None => {
                if let Some(parent) = parent {
                    arena.node_mut(parent).first_child = next;
                }
            }
        }
        match next {
            Some(next) => arena.node_mut(next).previous_sibling = previous,
            None => {
                if let Some(parent) = parent {
                    arena.node_mut(parent).last_child = previous;
                }
            }
        }
    }

    /// Detach the node and free it and every node below it: their
    /// values are dropped and every id among them becomes stale. A
    /// stale id is ignored.
    ///
    /// O(subtree): a cursor runs down first children and frees each
    /// leaf it reaches, so no stack is kept and nothing recurses. Slots
    /// join the free list in the order freed — each node after its
    /// subtree, the node itself last.
    pub fn remove_subtree<T>(self, arena: &mut Arena<T>) {
        if self.is_removed(arena) {
            return;
        }
        self.detach(arena);
        let mut cursor = self;
        loop {
            if let Some(child) = arena[cursor].first_child {
                cursor = child;
                continue;
            }
            if cursor == self {
                break;
            }
            let (parent, next) = {
                let node = &arena[cursor];
                (node.parent, node.next_sibling)
            };
            // A leaf below `self`: unlink it from the front of its row
            // and free it, then go on with its next sibling — or, when
            // it was the last, its parent, now a leaf in its turn.
            let Some(parent) = parent else {
                break;
            };
            let row = arena.node_mut(parent);
            row.first_child = next;
            if next.is_none() {
                row.last_child = None;
            }
            if let Some(next) = next {
                arena.node_mut(next).previous_sibling = None;
            }
            arena.free(cursor);
            cursor = next.unwrap_or(parent);
        }
        arena.free(self);
    }

    /// A new node holding `value`, appended as the node's last child.
    /// O(1): the node is new, so nothing is climbed.
    ///
    /// **Panics** when `self` is stale.
    #[track_caller]
    pub fn append_value<T>(self, value: T, arena: &mut Arena<T>) -> NodeId {
        let last = arena[self].last_child;
        let new = arena.new_node(value);
        link(arena, new, self, last, None);
        new
    }

    /// A new node holding `value`, prepended as the node's first child.
    /// O(1): the node is new, so nothing is climbed.
    ///
    /// **Panics** when `self` is stale.
    #[track_caller]
    pub fn prepend_value<T>(self, value: T, arena: &mut Arena<T>) -> NodeId {
        let first = arena[self].first_child;
        let new = arena.new_node(value);
        link(arena, new, self, None, first);
        new
    }

    /// A new node holding `value`, inserted as the node's next sibling.
    /// O(1): the node is new, so nothing is climbed.
    ///
    /// **Panics** when `self` is stale or has no parent: only a node
    /// under a parent is in a row.
    #[track_caller]
    pub fn insert_after_value<T>(self, value: T, arena: &mut Arena<T>) -> NodeId {
        let (parent, next) = {
            let node = &arena[self];
            (row_of(self, node.parent), node.next_sibling)
        };
        let new = arena.new_node(value);
        link(arena, new, parent, Some(self), next);
        new
    }

    /// A new node holding `value`, inserted as the node's previous
    /// sibling. O(1): the node is new, so nothing is climbed.
    ///
    /// **Panics** when `self` is stale or has no parent: only a node
    /// under a parent is in a row.
    #[track_caller]
    pub fn insert_before_value<T>(self, value: T, arena: &mut Arena<T>) -> NodeId {
        let (parent, previous) = {
            let node = &arena[self];
            (row_of(self, node.parent), node.previous_sibling)
        };
        let new = arena.new_node(value);
        link(arena, new, parent, previous, Some(self));
        new
    }

    /// Move the existing node `child`, with its subtree, to be the
    /// node's last child — detaching it from wherever it hangs.
    ///
    /// Errors (nothing changes), the first that applies:
    /// [`ArenaError::Stale`] for a stale `self`, then for a stale
    /// `child`; [`ArenaError::Cycle`] when `child` is the node itself or
    /// one of its ancestors.
    ///
    /// Cost: the cycle check, a climb from the node to the top of its
    /// tree — O(depth), once for the whole moved subtree — then O(1).
    pub fn checked_append<T>(self, child: NodeId, arena: &mut Arena<T>) -> Result<(), ArenaError> {
        check_move(arena, self, child)?;
        child.detach(arena);
        let last = arena[self].last_child;
        link(arena, child, self, last, None);
        Ok(())
    }

    /// Move the existing node `child`, with its subtree, to be the
    /// node's first child. The errors and cost of
    /// [`Self::checked_append`].
    pub fn checked_prepend<T>(self, child: NodeId, arena: &mut Arena<T>) -> Result<(), ArenaError> {
        check_move(arena, self, child)?;
        child.detach(arena);
        let first = arena[self].first_child;
        link(arena, child, self, None, first);
        Ok(())
    }

    /// Move the existing node `sibling`, with its subtree, to be the
    /// node's next sibling, under the node's parent.
    ///
    /// Errors (nothing changes), the first that applies:
    /// [`ArenaError::Stale`] for a stale `self`, then for a stale
    /// `sibling`; [`ArenaError::NoParent`] when the node has no parent,
    /// and so no row; [`ArenaError::Cycle`] when `sibling` is the node
    /// itself (`to` is the node), or is the node's parent or one of its
    /// ancestors (`to` is the parent).
    ///
    /// Cost: the cycle check, a climb from the node's parent to the top
    /// of its tree — O(depth), once for the whole moved subtree — then
    /// O(1).
    pub fn checked_insert_after<T>(self, sibling: NodeId, arena: &mut Arena<T>) -> Result<(), ArenaError> {
        let parent = check_beside(arena, self, sibling)?;
        sibling.detach(arena);
        let next = arena[self].next_sibling;
        link(arena, sibling, parent, Some(self), next);
        Ok(())
    }

    /// Move the existing node `sibling`, with its subtree, to be the
    /// node's previous sibling, under the node's parent. The errors and
    /// cost of [`Self::checked_insert_after`].
    pub fn checked_insert_before<T>(self, sibling: NodeId, arena: &mut Arena<T>) -> Result<(), ArenaError> {
        let parent = check_beside(arena, self, sibling)?;
        sibling.detach(arena);
        let previous = arena[self].previous_sibling;
        link(arena, sibling, parent, previous, Some(self));
        Ok(())
    }

    /// Put the detached node back into `parent`'s row right after
    /// `previous` (`None`: at the front) — **the row it was detached
    /// from**, which is the caller's to guarantee and is not checked:
    /// a node returned under the parent it came from cannot close a
    /// cycle, so nothing is climbed (ruling R16 on #282). Crate-private
    /// for that reason: under any other parent, an unchecked insert
    /// could hang a node below itself. The engine's one caller is
    /// `engine::siblings::reattach_in_row`.
    ///
    /// Errors (nothing changes), each O(1) to tell, the first that
    /// applies: [`ArenaError::Stale`] for a stale `self`;
    /// [`ArenaError::Attached`] for a node that still hangs in a row;
    /// `Stale` for a stale `parent`; [`ArenaError::Cycle`] for `parent`
    /// being the node itself; `Stale` for a stale `previous`;
    /// [`ArenaError::NotAChild`] for a `previous` that is not a child of
    /// `parent`.
    ///
    /// Cost: O(1), however deep `parent` sits.
    pub(crate) fn reattach_in_row<T>(
        self,
        parent: NodeId,
        previous: Option<NodeId>,
        arena: &mut Arena<T>,
    ) -> Result<(), ArenaError> {
        let node = arena.get(self).ok_or(ArenaError::Stale(self))?;
        if !node.is_detached() {
            return Err(ArenaError::Attached(self));
        }
        let parent_node = arena.get(parent).ok_or(ArenaError::Stale(parent))?;
        if parent == self {
            // A node never left itself: the one cycle telling costs O(1).
            return Err(ArenaError::Cycle { node: self, to: self });
        }
        let next = match previous {
            Some(previous) => {
                let sibling = arena.get(previous).ok_or(ArenaError::Stale(previous))?;
                if sibling.parent != Some(parent) {
                    return Err(ArenaError::NotAChild {
                        parent,
                        node: previous,
                    });
                }
                sibling.next_sibling
            }
            None => parent_node.first_child,
        };
        #[cfg(test)]
        crate::arena::tests_support::count_insert_steps(1);
        link(arena, self, parent, previous, next);
        Ok(())
    }
}

/// The parent of a node a sibling is inserted beside. **Panics** when
/// it has none: a node without a parent is in no row.
#[track_caller]
fn row_of(id: NodeId, parent: Option<NodeId>) -> NodeId {
    match parent {
        Some(parent) => parent,
        None => panic!("arena: node {id:?} has no parent, so no row to insert a sibling in"),
    }
}

/// Whether the existing node `node` may move to hang below `parent`:
/// both are live (`parent` checked first), and `node` is neither
/// `parent` nor above it. The one climb of the arena's inserts, from
/// `parent` to the top of its tree, counted in test builds.
fn check_move<T>(arena: &Arena<T>, parent: NodeId, node: NodeId) -> Result<(), ArenaError> {
    if arena.get(parent).is_none() {
        return Err(ArenaError::Stale(parent));
    }
    if arena.get(node).is_none() {
        return Err(ArenaError::Stale(node));
    }
    let mut climbed = 0;
    let cycle = parent.ancestors(arena).any(|above| {
        climbed += 1;
        above == node
    });
    #[cfg(test)]
    crate::arena::tests_support::count_insert_steps(1 + climbed);
    let _ = climbed;
    if cycle {
        return Err(ArenaError::Cycle { node, to: parent });
    }
    Ok(())
}

/// Whether the existing node `sibling` may move into `anchor`'s row,
/// beside it — `anchor` live and under a parent, `sibling` live and
/// neither `anchor` itself nor above `anchor`'s parent — and that
/// parent. See [`NodeId::checked_insert_after`] for the order the
/// refusals are tried in.
fn check_beside<T>(arena: &Arena<T>, anchor: NodeId, sibling: NodeId) -> Result<NodeId, ArenaError> {
    let Some(node) = arena.get(anchor) else {
        return Err(ArenaError::Stale(anchor));
    };
    if arena.get(sibling).is_none() {
        return Err(ArenaError::Stale(sibling));
    }
    let Some(parent) = node.parent else {
        return Err(ArenaError::NoParent(anchor));
    };
    if sibling == anchor {
        return Err(ArenaError::Cycle {
            node: sibling,
            to: anchor,
        });
    }
    check_move(arena, parent, sibling)?;
    Ok(parent)
}

/// Link the detached node `new` — its subtree comes along — under
/// `parent` between `previous` and `next`, which must be adjacent in
/// that row (either may be `None`, at an end of it). O(1).
fn link<T>(
    arena: &mut Arena<T>,
    new: NodeId,
    parent: NodeId,
    previous: Option<NodeId>,
    next: Option<NodeId>,
) {
    {
        let node = arena.node_mut(new);
        node.parent = Some(parent);
        node.previous_sibling = previous;
        node.next_sibling = next;
    }
    match previous {
        Some(previous) => arena.node_mut(previous).next_sibling = Some(new),
        None => arena.node_mut(parent).first_child = Some(new),
    }
    match next {
        Some(next) => arena.node_mut(next).previous_sibling = Some(new),
        None => arena.node_mut(parent).last_child = Some(new),
    }
}
