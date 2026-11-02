// SPDX-License-Identifier: MPL-2.0

//! Every structural operation and walk of the arena, run in an arena
//! that already has room for the nodes they create (V1-A7, #301): none
//! of them allocates. `tests/arena_alloc.rs` runs
//! [`every_operation_in_place`](crate::arena::tests::in_place_tests::every_operation_in_place)
//! under a counting allocator and holds it to zero bytes;
//! [`do_arena_operations_in_place`](crate::arena::tests::in_place_tests::do_arena_operations_in_place)
//! holds the arena's own `Vec` to its capacity, the one allocation the
//! arena owns.

use crate::arena::{Arena, NodeEdge, NodeId};

/// How deep the fixture's chain is.
pub const IN_PLACE_DEPTH: usize = 1_000;

/// How many children the fixture's leaf holds.
pub const IN_PLACE_ROW: usize = 8;

/// How many nodes [`every_operation_in_place`] creates: the room the
/// fixture leaves for them.
pub const IN_PLACE_CREATED: usize = 5;

/// An arena to operate on in place: a chain [`IN_PLACE_DEPTH`] deep, a
/// row of [`IN_PLACE_ROW`] children under its leaf, one detached node,
/// and room for [`IN_PLACE_CREATED`] more nodes.
pub struct InPlace {
    /// The arena.
    pub arena: Arena<usize>,
    /// The chain, top to leaf.
    pub chain: Vec<NodeId>,
    /// The leaf's children, first to last.
    pub row: Vec<NodeId>,
    /// A node in no tree.
    pub loose: NodeId,
}

/// The fixture, built in an arena made with exactly the room
/// [`every_operation_in_place`] needs. Allocates.
pub fn in_place_fixture() -> InPlace {
    let mut arena = Arena::with_capacity(IN_PLACE_DEPTH + IN_PLACE_ROW + 1 + IN_PLACE_CREATED);
    let mut chain = Vec::with_capacity(IN_PLACE_DEPTH);
    let mut at = arena.new_node(0);
    chain.push(at);
    for level in 1..IN_PLACE_DEPTH {
        at = at.append_value(level, &mut arena);
        chain.push(at);
    }
    let row = (0..IN_PLACE_ROW)
        .map(|i| at.append_value(10_000 + i, &mut arena))
        .collect();
    let loose = arena.new_node(usize::MAX);
    InPlace {
        arena,
        chain,
        row,
        loose,
    }
}

/// Every structural operation and walk on `f`, each checked by what it
/// leaves, with no allocation of its own: reads, every walk run to its
/// end from both ends, the four value inserts and [`Arena::new_node`]
/// into the room the fixture left, a detach and a same-row re-attach,
/// the four checked moves, a refused cycle, a removal that frees a slot
/// and a value insert that takes it again. Returns a sum of what the
/// walks yielded, so nothing is optimized away.
pub fn every_operation_in_place(f: &mut InPlace) -> usize {
    let arena = &mut f.arena;
    let (top, leaf) = (f.chain[0], f.chain[IN_PLACE_DEPTH - 1]);
    let (first, second, last) = (f.row[0], f.row[1], f.row[IN_PLACE_ROW - 1]);
    let mut sum = 0;

    // Reads and walks.
    assert_eq!(first.parent(arena), Some(leaf));
    assert_eq!(arena.at(leaf).last_child(), Some(last));
    assert_eq!(arena.node(second).and_then(|n| n.previous_sibling()), Some(first));
    sum += *arena[leaf].get() + arena.get(top).map_or(0, |n| *n.get());
    sum += leaf.children(arena).map(|id| *arena[id].get()).sum::<usize>();
    sum += leaf.children(arena).rev().count();
    sum += top.descendants(arena).count();
    sum += top
        .traverse(arena)
        .filter(|edge| matches!(edge, NodeEdge::End(_)))
        .count();
    sum += last.ancestors(arena).count();
    sum += arena.iter().len();
    assert_eq!(sum, 85_061, "precondition: the walks ran");

    // Value inserts and a new node, into the room left for them.
    let appended = leaf.append_value(1, arena);
    let prepended = leaf.prepend_value(2, arena);
    let after = first.insert_after_value(3, arena);
    let before = last.insert_before_value(4, arena);
    let fresh = arena.new_node(5);
    assert_eq!(leaf.first_child(arena), Some(prepended));
    assert_eq!(leaf.last_child(arena), Some(appended));
    assert_eq!(first.next_sibling(arena), Some(after));
    assert_eq!(last.previous_sibling(arena), Some(before));
    *arena.get_mut(fresh).expect("the new node is live") += 1;

    // A detach and a same-row re-attach.
    second.detach(arena);
    second
        .reattach_in_row(leaf, Some(last), arena)
        .expect("back into its own row");
    assert_eq!(leaf.last_child(arena), Some(appended));
    assert_eq!(last.next_sibling(arena), Some(second));

    // The checked moves, and a refused cycle.
    f.loose
        .checked_append(fresh, arena)
        .expect("under the loose node");
    f.loose
        .checked_prepend(appended, arena)
        .expect("under the loose node");
    fresh
        .checked_insert_after(prepended, arena)
        .expect("beside the new node");
    fresh
        .checked_insert_before(after, arena)
        .expect("beside the new node");
    assert!(
        leaf.checked_append(top, arena).is_err(),
        "the chain cannot hang below its leaf"
    );
    assert_eq!(f.loose.children(arena).count(), 4);

    // A removal, and a value insert that takes the freed slot again.
    let live = arena.live_count();
    f.loose.remove_subtree(arena);
    assert_eq!(arena.live_count(), live - 5);
    let again = leaf.append_value(6, arena);
    assert!(f.loose.is_removed(arena) && !again.is_removed(arena));
    sum
}

#[test]
fn test_arena_operations_in_place() {
    do_arena_operations_in_place();
}

/// [`every_operation_in_place`] on a fresh fixture leaves the arena's
/// `Vec` of slots at the capacity the fixture gave it: the room it
/// left was enough, and nothing grew.
pub fn do_arena_operations_in_place() {
    let mut f = in_place_fixture();
    let capacity = f.arena.capacity();
    assert_eq!(
        capacity,
        f.arena.len() + IN_PLACE_CREATED,
        "precondition: the fixture left exactly the room it needs"
    );
    every_operation_in_place(&mut f);
    assert_eq!(f.arena.capacity(), capacity, "an operation grew the arena");
}
