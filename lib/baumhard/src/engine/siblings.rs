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
//! other sibling stays where it is. Finding a node's slot (`slot`,
//! `reposition_slot`, `last_slot`) is separate from attaching it
//! (`attach_at`), so that a [journal](crate::engine::journal::Journal) can note
//! each move between the two. Nothing here allocates beyond the arena
//! slot a new node takes.

use crate::arena::{Arena, NodeId};
use crate::engine::tree::{Placement, TreeError};

/// Insert `value` as a child of `parent` at the position `at` names
/// within the run of children whose key equals `value`'s — the slot
/// [`slot`] finds.
///
/// Errors: those of [`slot`]. Nothing is inserted on an error.
///
/// Cost: [`slot`]'s scan plus one arena slot. The new node is fresh,
/// so the arena's O(1) value-inserting paths apply — there is no
/// ancestor walk.
pub(crate) fn insert_sorted<T, K: Ord>(
    arena: &mut Arena<T>,
    parent: NodeId,
    value: T,
    at: Placement,
    key_of: impl Fn(&T) -> K,
) -> Result<NodeId, TreeError> {
    let prev = slot(arena, parent, &key_of(&value), at, &key_of)?;
    Ok(insert_value_at(arena, parent, value, prev))
}

/// Insert a new node holding `value` under `parent`, right after `prev`
/// (`None`: at the front), and return its id. The slot is the caller's,
/// found by [`slot`] or [`last_slot`] in a sorted row; `prev`, when
/// given, must be a live child of `parent`.
///
/// Cost: O(1) — the node is new, so the arena's value inserts check no
/// ancestry, however deep `parent` sits.
pub(crate) fn insert_value_at<T>(
    arena: &mut Arena<T>,
    parent: NodeId,
    value: T,
    prev: Option<NodeId>,
) -> NodeId {
    match prev {
        Some(prev) => prev.insert_after_value(value, arena),
        None => parent.prepend_value(value, arena),
    }
}

/// Insert a new node holding `value` under `parent` at the end of the
/// run of its key ([`Placement::Last`]) and return its id — how a
/// detached subtree is built row by row. `parent` must be in `arena`.
///
/// Cost: a scan from the end of the row, O(1) for children appended in
/// ascending key order, and an O(1) value insert.
pub(crate) fn append_value_sorted<T, K: Ord>(
    arena: &mut Arena<T>,
    parent: NodeId,
    value: T,
    key_of: impl Fn(&T) -> K,
) -> NodeId {
    let last = arena.get(parent).and_then(|node| node.last_child());
    let prev = last_slot_for(arena, last, &key_of(&value), &key_of);
    insert_value_at(arena, parent, value, prev)
}

/// Where a node keyed `key` goes among `parent`'s children, at the
/// position `at` names within the run of children whose key equals
/// `key`: the sibling it must follow, or `None` for the front of the
/// row. The row must be in order and must not hold the node being
/// placed. Reads only; [`attach_at`] or a value insert places the node.
///
/// - [`Placement::Last`]: after every child whose key is `<=` `key` —
///   the end of the equal-key run. Scans backward from the last child,
///   so appending in ascending key order is O(1).
/// - [`Placement::First`]: before every child whose key is `>=` `key`
///   — the start of the equal-key run. Scans forward from the first
///   child.
/// - [`Placement::Before`] / [`Placement::After`]: next to the named
///   sibling, which must be a child of `parent` with an equal key.
///
/// Errors: [`TreeError::StaleNode`] when `parent` or the named sibling
/// is not in the arena, [`TreeError::NotAChild`] when the named sibling
/// is not `parent`'s child, [`TreeError::PlacementCrossesKey`] when its
/// key differs from `key`.
///
/// Cost: O(siblings scanned) key reads; no allocation.
pub(crate) fn slot<T, K: Ord>(
    arena: &Arena<T>,
    parent: NodeId,
    key: &K,
    at: Placement,
    key_of: impl Fn(&T) -> K,
) -> Result<Option<NodeId>, TreeError> {
    let Some(parent_node) = arena.get(parent) else {
        return Err(TreeError::StaleNode(parent));
    };
    match at {
        Placement::Last => Ok(last_slot_for(arena, parent_node.last_child(), key, &key_of)),
        Placement::First => {
            let mut cursor = parent_node.first_child();
            while let Some(id) = cursor {
                let node = &arena[id];
                if key_of(node.get()) >= *key {
                    return Ok(node.previous_sibling());
                }
                cursor = node.next_sibling();
            }
            Ok(parent_node.last_child())
        }
        Placement::Before(sibling) => {
            check_sibling(arena, parent, sibling, key, &key_of)?;
            Ok(arena[sibling].previous_sibling())
        }
        Placement::After(sibling) => {
            check_sibling(arena, parent, sibling, key, &key_of)?;
            Ok(Some(sibling))
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
    last_slot_for(arena, parent_node.last_child(), &key_of(moved.get()), &key_of)
}

/// The nearest of the siblings from `last` backward whose key is `<=`
/// `key` — the end of `key`'s run in a sorted row whose last child is
/// `last` — or `None` when every one of them is keyed above it.
///
/// Cost: O(siblings scanned) key reads.
fn last_slot_for<T, K: Ord>(
    arena: &Arena<T>,
    last: Option<NodeId>,
    key: &K,
    key_of: &impl Fn(&T) -> K,
) -> Option<NodeId> {
    let mut cursor = last;
    while let Some(id) = cursor {
        let sibling = &arena[id];
        if key_of(sibling.get()) <= *key {
            return Some(id);
        }
        cursor = sibling.previous_sibling();
    }
    None
}

/// Re-attach `node`, which the caller has just detached from
/// `parent`'s row, into **that same row**, right after `prev` (`None`:
/// at the front) — a re-arrangement among siblings, never a change of
/// parent. The walker's settle, [`Tree::update`](crate::engine::tree::Tree::update)
/// and a [journal](crate::engine::journal::Journal) restoring an element
/// to the row it never left all re-arrange through here, and nothing
/// else does: the one place their insert is made.
///
/// A node put back under the parent it came from cannot close a cycle,
/// so it goes in through the arena's unchecked same-row re-attach,
/// which climbs nothing (ruling R16 on #282, V1-A6 #290). That `parent`
/// is the one `node` left is this function's callers' to guarantee,
/// since a detached node no longer names it; every other precondition
/// the arena checks in O(1) — a stale id, a node still attached, a
/// `prev` under another parent, the node as its own parent — and
/// refuses. No caller can be refused, so a refusal is an engine bug: a
/// debug build stops on it, and a release build logs it and leaves the
/// node detached rather than aborting the caller (an interactive path,
/// CODE_CONVENTIONS §9).
///
/// Cost: O(1), however deep `parent` sits.
pub(crate) fn reattach_in_row<T>(arena: &mut Arena<T>, parent: NodeId, node: NodeId, prev: Option<NodeId>) {
    if let Err(e) = node.reattach_in_row(parent, prev, arena) {
        debug_assert!(false, "engine invariant: a same-row re-attach was refused: {e}");
        log::error!("engine: could not re-attach node {node} in the row of {parent}: {e}");
    }
}

/// Attach the detached `node` under `parent`, right after `prev`, or
/// as the first child when `prev` is `None`. Places exactly where it
/// is told — keys are not read — so the caller owns the
/// sorted-children invariant: the structural commands pass a slot found
/// by [`slot`], and a [journal](crate::engine::journal::Journal) passes
/// a position it recorded from a sorted row. `parent` may itself be
/// detached: the node then joins that detached subtree.
///
/// For a node that changes parent — a move, or a node entering the
/// tree. A node re-arranged within its own row goes through
/// [`reattach_in_row`]; a node created for the insert goes in through
/// the arena's value inserts, which need no check ([`insert_sorted`]).
/// This is the engine's one checked insert of an existing node:
/// `test_the_engine_climbs_only_where_it_may` (over `util::source_scan`)
/// holds every other engine file to not making one.
///
/// The checked insert refuses only a stale node or a cycle, neither of
/// which a detached node placed by a command that checked for one can
/// be. A refusal is therefore an engine bug: a debug build stops on
/// it, and a release build logs it and leaves the node detached rather
/// than aborting the caller (an interactive path, CODE_CONVENTIONS §9).
///
/// Cost: the arena's cycle check, a climb from `parent` to the top —
/// O(depth), the price a real re-parent pays once per node moved.
pub(crate) fn attach_at<T>(arena: &mut Arena<T>, parent: NodeId, node: NodeId, prev: Option<NodeId>) {
    #[cfg(test)]
    tests_support::count_checked_attach();
    let attached = match prev {
        Some(anchor) => anchor.checked_insert_after(node, arena),
        None => parent.checked_prepend(node, arena),
    };
    if let Err(e) = attached {
        debug_assert!(false, "engine invariant: a re-parenting attach was refused: {e}");
        log::error!("engine: could not attach node {node} under {parent}: {e}");
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

/// What the tests read of [`attach_at`]: how many nodes it has put in
/// place on this thread. Compiled for tests only.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::cell::Cell;

    thread_local! {
        static CHECKED_ATTACHES: Cell<usize> = const { Cell::new(0) };
    }

    /// One more node put in place by [`attach_at`](super::attach_at) —
    /// an insert that climbs to the root to check for a cycle.
    pub(super) fn count_checked_attach() {
        CHECKED_ATTACHES.with(|cell| cell.set(cell.get() + 1));
    }

    /// How many nodes [`attach_at`](super::attach_at) has put in place on
    /// this thread so far: each one a climb to the root.
    pub(crate) fn checked_attaches() -> usize {
        CHECKED_ATTACHES.with(Cell::get)
    }
}
