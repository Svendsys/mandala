// SPDX-License-Identifier: MPL-2.0

//! The sorted-children invariant, as arena operations.
//!
//! **Invariant:** every parent's children are in ascending
//! [`BranchKey`](crate::engine::key::BranchKey) order, and children with
//! equal keys keep the order they were placed in. Both arenas a walk
//! aligns hold it — [`Tree`](crate::engine::tree::Tree) over elements and
//! [`MutatorTree`](crate::engine::tree::MutatorTree) over mutators — so
//! the functions here are generic over the arena's value type and take
//! the key as a function of the value.
//!
//! The invariant is kept by **placing** a node, never by sorting a
//! row: a node goes in at the one position its key allows, found by
//! scanning from the end of the row its placement names, and every
//! other sibling stays where it is. Finding a moved node's slot
//! (`reposition_slot`, `last_slot`) is separate from attaching it
//! (`attach_at`), so that a [journal](crate::engine::journal::Journal) can note
//! each move between the two. Nothing here allocates beyond the arena
//! slot a new node takes.

use crate::engine::tree::{Placement, TreeError};
use indextree::{Arena, NodeId};

/// Insert `value` as a child of `parent` at the position `at` names
/// within the run of children whose key equals `value`'s.
///
/// - [`Placement::Last`]: after every child whose key is `<=` the new
///   key — the end of the equal-key run. Scans backward from the last
///   child, so appending in ascending key order is O(1).
/// - [`Placement::First`]: before every child whose key is `>=` the
///   new key — the start of the equal-key run. Scans forward from the
///   first child.
/// - [`Placement::Before`] / [`Placement::After`]: next to the named
///   sibling, which must be a child of `parent` with an equal key.
///
/// Errors: [`TreeError::StaleNode`] when `parent` or the named sibling
/// is not in the arena, [`TreeError::NotAChild`] when the named sibling
/// is not `parent`'s child, [`TreeError::PlacementCrossesKey`] when its
/// key differs from the new one. Nothing is inserted on an error.
///
/// Cost: O(siblings scanned) key reads plus one arena slot. The new
/// node is fresh, so indextree's O(1) value-inserting paths apply —
/// there is no ancestor walk.
pub(crate) fn insert_sorted<T, K: Ord>(
    arena: &mut Arena<T>,
    parent: NodeId,
    value: T,
    at: Placement,
    key_of: impl Fn(&T) -> K,
) -> Result<NodeId, TreeError> {
    let Some(parent_node) = arena.get(parent) else {
        return Err(TreeError::StaleNode(parent));
    };
    let key = key_of(&value);
    match at {
        Placement::Last => {
            let mut cursor = parent_node.last_child();
            while let Some(id) = cursor {
                let node = &arena[id];
                if key_of(node.get()) <= key {
                    return Ok(id.insert_after_value(value, arena));
                }
                cursor = node.previous_sibling();
            }
            Ok(parent.prepend_value(value, arena))
        }
        Placement::First => {
            let mut cursor = parent_node.first_child();
            while let Some(id) = cursor {
                let node = &arena[id];
                if key_of(node.get()) >= key {
                    return Ok(id.insert_before_value(value, arena));
                }
                cursor = node.next_sibling();
            }
            Ok(parent.append_value(value, arena))
        }
        Placement::Before(sibling) => {
            check_sibling(arena, parent, sibling, &key, &key_of)?;
            Ok(sibling.insert_before_value(value, arena))
        }
        Placement::After(sibling) => {
            check_sibling(arena, parent, sibling, &key, &key_of)?;
            Ok(sibling.insert_after_value(value, arena))
        }
    }
}

/// Whether `sibling` may anchor a placement of a node keyed `key`
/// under `parent`: it exists, it is `parent`'s child, and its key is
/// equal. O(1).
fn check_sibling<T, K: Ord>(
    arena: &Arena<T>,
    parent: NodeId,
    sibling: NodeId,
    key: &K,
    key_of: &impl Fn(&T) -> K,
) -> Result<(), TreeError> {
    let Some(node) = arena.get(sibling) else {
        return Err(TreeError::StaleNode(sibling));
    };
    if node.parent() != Some(parent) {
        return Err(TreeError::NotAChild {
            parent,
            node: sibling,
        });
    }
    if key_of(node.get()) != *key {
        return Err(TreeError::PlacementCrossesKey);
    }
    Ok(())
}

/// Where `node` belongs among its siblings — after every sibling whose
/// key is `<=` its own — assuming every **other** sibling is already in
/// order. Returns `None` when the node is already in place (or has no
/// parent), and otherwise `Some(prev)`: the sibling it must follow,
/// `None` for the front of the row. Reads only; [`attach_at`] performs
/// the move once the node is detached.
///
/// This is the single-node repair behind
/// [`Tree::update`](crate::engine::tree::Tree::update): one key changed,
/// so the rest of the row is sorted and the node only has to travel
/// past the siblings its new key no longer sits beside.
///
/// Cost: O(siblings passed) key reads; no allocation.
pub(crate) fn reposition_slot<T, K: Ord>(
    arena: &Arena<T>,
    node: NodeId,
    key_of: impl Fn(&T) -> K,
) -> Option<Option<NodeId>> {
    let current = arena.get(node)?;
    current.parent()?;
    let key = key_of(current.get());

    // Moving earlier: walk back past every sibling keyed above the
    // node. The first one at or below it is where the node goes after.
    let mut back = current.previous_sibling();
    let mut moved_back = false;
    while let Some(id) = back {
        let sibling = &arena[id];
        if key_of(sibling.get()) <= key {
            break;
        }
        moved_back = true;
        back = sibling.previous_sibling();
    }
    if moved_back {
        return Some(back);
    }

    // Moving later: walk forward past every sibling keyed at or below
    // the node — the end of its new equal-key run.
    let mut last_passed = None;
    let mut forward = current.next_sibling();
    while let Some(id) = forward {
        let sibling = &arena[id];
        if key_of(sibling.get()) > key {
            break;
        }
        last_passed = Some(id);
        forward = sibling.next_sibling();
    }
    last_passed.map(Some)
}

/// Where the detached `node` joins `parent`'s row at the end of the
/// run of children whose key equals its own — [`Placement::Last`] for
/// a node that already exists: the sibling it must follow, or `None`
/// for the front of the row. The row must be in order. Reads only;
/// [`attach_at`] performs the attach.
///
/// This is the batch repair behind the walker: every child whose key a
/// walk changed is detached first, which leaves each row sorted, and
/// each is then placed at the slot this finds.
///
/// Cost: O(siblings scanned from the end of the row) key reads.
pub(crate) fn last_slot<T, K: Ord>(
    arena: &Arena<T>,
    parent: NodeId,
    node: NodeId,
    key_of: impl Fn(&T) -> K,
) -> Option<NodeId> {
    let (Some(parent_node), Some(moved)) = (arena.get(parent), arena.get(node)) else {
        return None;
    };
    let key = key_of(moved.get());
    let mut cursor = parent_node.last_child();
    while let Some(id) = cursor {
        let sibling = &arena[id];
        if key_of(sibling.get()) <= key {
            return Some(id);
        }
        cursor = sibling.previous_sibling();
    }
    None
}

/// Attach the detached `node` under `parent`, right after `prev`, or
/// as the first child when `prev` is `None`. Places exactly where it
/// is told — keys are not read — so the caller owns the
/// sorted-children invariant: the walker and [`Tree::update`](crate::engine::tree::Tree::update)
/// pass a slot found by [`last_slot`] or [`reposition_slot`], and a
/// [journal](crate::engine::journal::Journal) passes a position it recorded
/// from a sorted row.
///
/// Cost: an O(depth) indextree insert check.
pub(crate) fn attach_at<T>(arena: &mut Arena<T>, parent: NodeId, node: NodeId, prev: Option<NodeId>) {
    match prev {
        Some(anchor) => attach_after(arena, anchor, node),
        None => attach_first(arena, parent, node),
    }
}

/// Attach the detached `node` right after `anchor`. The checked
/// indextree insert refuses only a removed node or a cycle, neither of
/// which a detached sibling of a live row can be; a refusal is logged
/// and leaves the node detached rather than aborting the caller (an
/// interactive path, CODE_CONVENTIONS §9).
fn attach_after<T>(arena: &mut Arena<T>, anchor: NodeId, node: NodeId) {
    if let Err(e) = anchor.checked_insert_after(node, arena) {
        log::error!("engine: could not re-attach node {node} after {anchor}: {e}");
    }
}

/// Attach the detached `node` as the first child of `parent`. See
/// [`attach_after`] for why a refusal is logged rather than raised.
fn attach_first<T>(arena: &mut Arena<T>, parent: NodeId, node: NodeId) {
    if let Err(e) = parent.checked_prepend(node, arena) {
        log::error!("engine: could not re-attach node {node} under {parent}: {e}");
    }
}

/// Whether every parent at or below `root` has its children in
/// ascending key order. The first out-of-order child found, in
/// pre-order, is reported as [`TreeError::Unsorted`].
///
/// Cost: O(nodes below `root`) key reads; no allocation.
pub(crate) fn check_sorted<T, K: Ord>(
    arena: &Arena<T>,
    root: NodeId,
    key_of: impl Fn(&T) -> K,
) -> Result<(), TreeError> {
    if arena.get(root).is_none() {
        return Err(TreeError::StaleNode(root));
    }
    for parent in root.descendants(arena) {
        let mut children = parent.children(arena);
        let Some(mut previous) = children.next() else {
            continue;
        };
        for child in children {
            if key_of(arena[child].get()) < key_of(arena[previous].get()) {
                return Err(TreeError::Unsorted { parent, child });
            }
            previous = child;
        }
    }
    Ok(())
}
