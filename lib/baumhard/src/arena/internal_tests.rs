// SPDX-License-Identifier: MPL-2.0

//! What the arena's `pub mod tests` tree cannot hold as benchmark
//! bodies: a slot at its last generation and the step counts of the
//! inserts of existing nodes, which read crate-private, test-only
//! state, and the two panics the arena's API documents.

use crate::arena::tests_support::insert_steps;
use crate::arena::{Arena, ArenaError, NodeId};

/// A slot freed at its last generation is retired: the next node takes
/// a new slot, and the old id stays stale. The control: a slot one
/// generation short of the last is reused, by a node with the last
/// generation, which is told from the id before it.
#[test]
fn test_a_slot_at_its_last_generation_is_retired() {
    let mut arena = Arena::new();
    let short = arena.new_node(1);
    arena.free(short);
    arena.set_next_generation(0, u32::MAX - 1);
    let before = arena.new_node(2);
    assert_eq!(before.to_string(), "1", "precondition: the slot is reused");
    assert_eq!(
        before.generation,
        u32::MAX - 1,
        "precondition: at the generation set"
    );
    arena.free(before);
    let last = arena.new_node(3);
    assert_eq!(
        last.to_string(),
        "1",
        "the control: one generation short of the last is reused"
    );
    assert_eq!(last.generation, u32::MAX);
    assert!(before.is_removed(&arena) && arena.get(last).is_some());

    arena.free(last);
    let next = arena.new_node(4);
    assert_eq!(
        next.to_string(),
        "2",
        "a slot at its last generation is not reused"
    );
    assert!(last.is_removed(&arena), "its last id stays stale");
    assert_eq!(arena.len(), 2);
    assert_eq!(arena.live_count(), 1);
}

/// A chain `depth` deep built by value inserts, and its ids top to leaf.
fn chain(depth: usize) -> (Arena<usize>, Vec<NodeId>) {
    let mut arena = Arena::new();
    let mut at = arena.new_node(0);
    let mut ids = vec![at];
    for level in 1..depth {
        at = at.append_value(level, &mut arena);
        ids.push(at);
    }
    (arena, ids)
}

/// A same-row re-attach at the bottom of a deep chain is one step; a
/// checked insert at the same place is one plus every node its cycle
/// check climbs — the depth. Value inserts are none.
#[test]
fn test_insert_steps_count_the_climb_and_only_the_climb() {
    const DEPTH: usize = 1_000;
    let before = insert_steps();
    let (mut arena, ids) = chain(DEPTH);
    assert_eq!(insert_steps() - before, 0, "value inserts climb nothing");
    let parent = ids[DEPTH - 1];
    let first = parent.append_value(0, &mut arena);
    let second = parent.append_value(1, &mut arena);

    first.detach(&mut arena);
    let before = insert_steps();
    first.reattach_in_row(parent, Some(second), &mut arena).unwrap();
    assert_eq!(insert_steps() - before, 1, "a same-row re-attach is one step");

    first.detach(&mut arena);
    let before = insert_steps();
    second.checked_insert_after(first, &mut arena).unwrap();
    assert_eq!(
        insert_steps() - before,
        1 + DEPTH,
        "a checked insert climbs from the parent, {DEPTH} deep, to the top"
    );
    assert_eq!(parent.children(&arena).collect::<Vec<_>>(), vec![second, first]);
}

/// The O(1) refusals of a same-row re-attach count no step and change
/// nothing.
#[test]
fn test_a_refused_reattach_counts_nothing() {
    let (mut arena, ids) = chain(3);
    let before = insert_steps();
    assert_eq!(
        ids[2].reattach_in_row(ids[1], None, &mut arena),
        Err(ArenaError::Attached(ids[2]))
    );
    assert_eq!(insert_steps() - before, 0);
    assert_eq!(ids[2].parent(&arena), Some(ids[1]));
}

/// Indexing with the id of a removed node panics, naming it, even once
/// its slot holds another node — it never reads that node. The control
/// is the precondition: `get` with the old id is `None` while the slot
/// is live under the new one.
#[test]
#[should_panic(expected = "is not in the arena (removed, or its slot reused since)")]
fn test_indexing_with_a_stale_id_panics() {
    let mut arena = Arena::new();
    let gone = arena.new_node(1);
    gone.remove_subtree(&mut arena);
    let taken = arena.new_node(2);
    assert_eq!(
        taken.to_string(),
        gone.to_string(),
        "precondition: the slot is reused"
    );
    assert!(arena.get(gone).is_none() && arena.get(taken).is_some());
    let _ = arena[gone].get();
}

/// A sibling inserted beside a node with no parent panics: a node is in
/// a row only under a parent. The control: beside a child, it is
/// inserted.
#[test]
#[should_panic(expected = "has no parent, so no row to insert a sibling in")]
fn test_a_sibling_beside_a_parentless_node_panics() {
    let mut arena = Arena::new();
    let top = arena.new_node(0);
    let child = top.append_value(1, &mut arena);
    let sibling = child.insert_after_value(2, &mut arena);
    assert_eq!(top.children(&arena).collect::<Vec<_>>(), vec![child, sibling]);
    top.insert_before_value(3, &mut arena);
}

/// The iterators clone over an arena whose values do not: they hold a
/// shared borrow of the arena and a position, nothing of `T`. A clone
/// walks on from the same place.
#[test]
fn test_the_iterators_clone_without_cloning_values() {
    struct Opaque;
    let mut arena = Arena::new();
    let top = arena.new_node(Opaque);
    let child = top.append_value(Opaque, &mut arena);
    let mut children = top.children(&arena);
    assert_eq!(children.clone().count(), 1);
    assert_eq!(children.next(), Some(child));
    assert_eq!(child.ancestors(&arena).clone().count(), 2);
    assert_eq!(top.descendants(&arena).clone().count(), 2);
    assert_eq!(top.traverse(&arena).clone().count(), 4);
}
