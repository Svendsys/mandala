// SPDX-License-Identifier: MPL-2.0

//! No structural operation of the arena allocates (V1-A7, #301): every
//! operation and walk, run in an arena that already has room for the
//! nodes it creates, allocates zero bytes, counted by a global
//! allocator. Its own binary, with one test, because a global allocator
//! counts every thread: nothing else runs beside the count.
//!
//! The control the count is held against is in the same test: a value
//! insert into an arena with no room left allocates, and the count sees
//! it — so a zero is the arena's, not a counter that saw nothing.
//!
//! The arena's one allocation is its `Vec` of slots, which
//! `arena::tests::in_place_tests::do_arena_operations_in_place` also
//! holds to its capacity in the unit suite; this binary is the stronger
//! form, counting every allocation the operations could make, wherever.

use baumhard::arena::tests::in_place_tests::{every_operation_in_place, in_place_fixture};
use cap::Cap;
use std::alloc::System;

#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

#[test]
fn test_no_structural_operation_allocates() {
    let mut fixture = in_place_fixture();
    let before = ALLOCATOR.total_allocated();
    let sum = every_operation_in_place(&mut fixture);
    let allocated = ALLOCATOR.total_allocated() - before;
    assert!(sum > 0, "precondition: the operations ran");
    assert_eq!(
        allocated, 0,
        "every operation and walk ran in the room the arena had, and allocated {allocated} bytes"
    );

    // The control: with no room left — no free slot, and the `Vec` full —
    // a value insert grows the arena's `Vec`, and the count sees it.
    let arena = &mut fixture.arena;
    let leaf = fixture.chain[fixture.chain.len() - 1];
    while arena.live_count() < arena.capacity() {
        leaf.append_value(0, arena);
    }
    let before = ALLOCATOR.total_allocated();
    leaf.append_value(0, arena);
    assert!(
        ALLOCATOR.total_allocated() > before,
        "precondition: an insert past the arena's room allocates, and the count sees it"
    );
}
