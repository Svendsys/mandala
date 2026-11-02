// SPDX-License-Identifier: MPL-2.0

//! What the arena's `pub mod tests` tree cannot hold as benchmark
//! bodies: a slot at its last generation, the step counts of the
//! inserts of existing nodes, and the generation checks and slot reads
//! of every operation and walk (V1-A7, #301), which read crate-private,
//! test-only state; the panics the arena's API documents; and what the
//! iterators say of their length.

use crate::arena::tests_support::{generation_checks, insert_steps, slot_reads};
use crate::arena::{next_slot, Arena, ArenaError, NodeId, MAX_SLOTS};

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

/// How many ids `$body` checks against their generation; it must be
/// `$expected`.
macro_rules! checks {
    ($expected:expr, $what:expr, $body:expr) => {{
        let before = generation_checks();
        let _ = $body;
        assert_eq!(
            generation_checks() - before,
            $expected,
            "{}: each id it is handed is checked once",
            $what
        );
    }};
}

/// Every operation checks the generation of each id it is handed —
/// `self`, then each argument — once, and of nothing else: what it does
/// after follows links (V1-A7, #301). A walk checks its start once,
/// however far it goes: a 1,000-deep chain's descendants, traversal and
/// climb are one check each. The rows control each other: an operation
/// handed one more id checks one more.
#[test]
fn test_each_id_is_checked_once_per_operation() {
    let (mut arena, ids) = chain(1_000);
    let (top, leaf) = (ids[0], ids[999]);
    let a = leaf.append_value(0, &mut arena);
    let b = leaf.append_value(1, &mut arena);
    let c = leaf.append_value(2, &mut arena);

    checks!(1, "get", arena.get(a));
    checks!(1, "indexing", arena[a].get());
    checks!(1, "node, and its five link reads", {
        let n = arena.node(a).expect("live");
        (
            n.parent(),
            n.first_child(),
            n.last_child(),
            n.previous_sibling(),
            n.next_sibling(),
        )
    });
    checks!(1, "at, and a link read", arena.at(a).next_sibling());
    checks!(1, "get_mut", arena.get_mut(a).map(|v| *v += 0));
    checks!(1, "is_removed", a.is_removed(&arena));
    checks!(1, "parent", a.parent(&arena));
    checks!(1, "first_child", leaf.first_child(&arena));
    checks!(1, "last_child", leaf.last_child(&arena));
    checks!(1, "previous_sibling", b.previous_sibling(&arena));
    checks!(1, "next_sibling", b.next_sibling(&arena));
    checks!(1, "children, to the end", leaf.children(&arena).count());
    checks!(1, "children, from the back", leaf.children(&arena).rev().count());
    checks!(1, "descendants of the chain", top.descendants(&arena).count());
    checks!(1, "the chain's traversal", top.traverse(&arena).count());
    checks!(1, "ancestors from below the leaf", a.ancestors(&arena).count());
    checks!(0, "iter, which is handed no id", arena.iter().count());
    checks!(0, "new_node, which is handed no id", arena.new_node(9));

    checks!(1, "detach", b.detach(&mut arena));
    checks!(
        3,
        "a re-attach after a sibling",
        b.reattach_in_row(leaf, Some(a), &mut arena)
    );
    b.detach(&mut arena);
    checks!(
        2,
        "a re-attach at the front",
        b.reattach_in_row(leaf, None, &mut arena)
    );
    let other = arena.new_node(3);
    let d = arena.new_node(4);
    checks!(2, "checked_append", other.checked_append(c, &mut arena));
    checks!(2, "checked_prepend", other.checked_prepend(b, &mut arena));
    checks!(2, "checked_insert_after", c.checked_insert_after(a, &mut arena));
    checks!(2, "checked_insert_before", c.checked_insert_before(d, &mut arena));
    checks!(
        2,
        "a refused cycle, 1,000 deep",
        leaf.checked_append(top, &mut arena)
    );
    checks!(1, "append_value", other.append_value(5, &mut arena));
    checks!(1, "prepend_value", other.prepend_value(6, &mut arena));
    checks!(1, "insert_after_value", c.insert_after_value(7, &mut arena));
    checks!(1, "insert_before_value", c.insert_before_value(8, &mut arena));
    assert_eq!(
        other.children(&arena).count(),
        8,
        "precondition: every move landed"
    );
    checks!(1, "remove_subtree", other.remove_subtree(&mut arena));
    checks!(1, "a stale id, refused", c.detach(&mut arena));
    assert!(
        c.is_removed(&arena),
        "precondition: the last row checked a stale id"
    );
}

/// A walk reads one slot per node per step, and a link read one slot:
/// the far end's, for its generation. The counts are exact on a chain
/// 1,000 deep with three children under its leaf, 1,003 nodes in all.
#[test]
fn test_a_walk_reads_one_slot_per_step() {
    let (mut arena, ids) = chain(1_000);
    let (top, leaf) = (ids[0], ids[999]);
    let a = leaf.append_value(0, &mut arena);
    leaf.append_value(1, &mut arena);
    leaf.append_value(2, &mut arena);
    let reads = |f: &dyn Fn() -> usize| {
        let before = slot_reads();
        let yielded = f();
        (yielded, slot_reads() - before)
    };
    assert_eq!(reads(&|| leaf.children(&arena).count()), (3, 3), "children");
    assert_eq!(
        reads(&|| leaf.children(&arena).rev().count()),
        (3, 3),
        "children, from the back"
    );
    assert_eq!(
        reads(&|| top.descendants(&arena).count()),
        (1_003, 2 * 1_003),
        "descendants: each node's start and end"
    );
    assert_eq!(
        reads(&|| top.traverse(&arena).count()),
        (2 * 1_003, 2 * 1_003),
        "traversal"
    );
    assert_eq!(
        reads(&|| a.ancestors(&arena).count()),
        (1_001, 1_001),
        "ancestors"
    );
    assert_eq!(
        reads(&|| usize::from(a.parent(&arena).is_some())),
        (1, 1),
        "a link read"
    );
    assert_eq!(
        reads(&|| usize::from(arena.get(a).is_some())),
        (1, 0),
        "a get, which checks and follows no link"
    );
}

/// Every iterator's `size_hint` bounds what it goes on to yield, at
/// every step and from either end; the children's is exact once one is
/// left, and an arena's nodes' is exact throughout, across a freed
/// slot.
#[test]
fn test_the_iterators_size_hints_bound_what_they_yield() {
    fn bounded<I: DoubleEndedIterator + Clone>(what: &str, mut walk: I, from_back: bool) {
        loop {
            let (low, high) = walk.size_hint();
            let left = walk.clone().count();
            assert!(
                low <= left && high.is_none_or(|high| left <= high),
                "{what}: size_hint ({low}, {high:?}) with {left} left"
            );
            let step = if from_back { walk.next_back() } else { walk.next() };
            if step.is_none() {
                break;
            }
        }
    }
    fn bounded_forward<I: Iterator + Clone>(what: &str, mut walk: I) {
        loop {
            let (low, high) = walk.size_hint();
            let left = walk.clone().count();
            assert!(
                low <= left && high.is_none_or(|high| left <= high),
                "{what}: size_hint ({low}, {high:?}) with {left} left"
            );
            if walk.next().is_none() {
                break;
            }
        }
    }
    let mut arena = Arena::new();
    let top = arena.new_node(0);
    let a = top.append_value(1, &mut arena);
    let b = top.append_value(2, &mut arena);
    top.append_value(3, &mut arena);
    a.append_value(4, &mut arena);
    let e = a.append_value(5, &mut arena);
    let f = e.append_value(6, &mut arena);
    let loose = arena.new_node(7);
    let gone = b.append_value(8, &mut arena);
    gone.remove_subtree(&mut arena);
    for id in [top, a, b, e, f, loose, gone] {
        bounded("children", id.children(&arena), false);
        bounded("children, from the back", id.children(&arena), true);
        bounded_forward("ancestors", id.ancestors(&arena));
        bounded_forward("descendants", id.descendants(&arena));
        bounded_forward("traversal", id.traverse(&arena));
    }

    let live = arena.live_count();
    assert_eq!(live, 8, "precondition: eight nodes, one slot freed");
    assert_eq!(top.children(&arena).size_hint(), (1, Some(live - 1)));
    let mut children = top.children(&arena);
    children.next();
    children.next_back();
    assert_eq!(
        children.size_hint(),
        (1, Some(1)),
        "one child left between the ends"
    );
    assert_eq!(gone.children(&arena).size_hint(), (0, Some(0)), "a stale id's");
    assert_eq!(top.traverse(&arena).size_hint(), (1, Some(2 * live)));

    let mut nodes = arena.iter();
    for left in (0..=live).rev() {
        assert_eq!(nodes.len(), left, "an arena's nodes know how many are left");
        nodes.next();
    }
    assert!(nodes.next().is_none() && nodes.len() == 0, "and stay done");
}

/// The last slot an arena holds has slot number `u32::MAX - 1`, and the
/// one past [`MAX_SLOTS`] panics naming the bound rather than wrapping.
/// The control is the slot before it.
#[test]
fn test_the_last_slot_is_numbered_below_u32_max() {
    assert_eq!(next_slot(0).get(), 1);
    assert_eq!(next_slot(MAX_SLOTS - 1).get(), u32::MAX - 1);
}

/// See [`test_the_last_slot_is_numbered_below_u32_max`].
#[test]
#[should_panic(expected = "arena: full: an arena holds at most MAX_SLOTS (4294967294) slots")]
fn test_a_slot_past_max_slots_panics() {
    next_slot(MAX_SLOTS);
}

/// [`Arena::with_capacity`] and [`Arena::reserve`] make room once: that
/// many new nodes then go in without the `Vec` of slots growing, and
/// one more past the room grows it.
#[test]
fn test_with_capacity_and_reserve_make_room_once() {
    let mut arena = Arena::with_capacity(10);
    let room = arena.capacity();
    assert!(room >= 10, "with_capacity made room for 10: {room}");
    for i in 0..10 {
        arena.new_node(i);
    }
    assert_eq!(
        arena.capacity(),
        room,
        "ten nodes went into the room made for ten"
    );
    arena.reserve(100);
    let room = arena.capacity();
    assert!(room >= 110, "reserve made room for 100 more: {room}");
    while arena.len() < room {
        arena.new_node(0);
    }
    assert_eq!(arena.capacity(), room, "the reserved room filled without growing");
    arena.new_node(0);
    assert!(arena.capacity() > room, "the control: one past the room grows it");
}
