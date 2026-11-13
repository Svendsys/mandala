// SPDX-License-Identifier: MPL-2.0

//! No structural operation of the arena allocates (V1-A7, #301): every
//! operation and walk, run in an arena that already has room for the
//! nodes it creates, allocates zero bytes, counted by a global
//! allocator.
//!
//! The count is the measuring thread's own. A global allocator sees
//! every thread's allocations, and a test is never alone in its
//! process: libtest runs it on a thread it spawned while its main thread
//! keeps working, so a process-wide count charged the arena with
//! whatever that thread allocated meanwhile, and flaked (#341).
//! `allocation_counter::measure` counts only the allocations made on the
//! thread that runs it, while it runs, so another thread's cannot land
//! in the count, whatever it does and whenever.
//!
//! Two controls hold the count. A value insert into an arena with no
//! room left allocates, and the count sees it — so a zero is the
//! arena's, not a counter that saw nothing. And another thread's
//! allocation made inside the measured window is not counted, while a
//! count on that thread sees it — so a zero is not libtest's luck.
//!
//! The arena's one allocation is its `Vec` of slots, which
//! `arena::tests::in_place_tests::do_arena_operations_in_place` also
//! holds to its capacity in the unit suite; this binary is the stronger
//! form, counting every allocation the operations could make, wherever.

use allocation_counter::measure;
use baumhard::arena::tests::in_place_tests::{every_operation_in_place, in_place_fixture};
use std::sync::Barrier;

#[test]
fn test_no_structural_operation_allocates() {
    let mut fixture = in_place_fixture();
    let mut sum = 0;
    let allocated = measure(|| sum = every_operation_in_place(&mut fixture)).bytes_total;
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
    let grown = measure(|| {
        leaf.append_value(0, arena);
    });
    assert!(
        grown.bytes_total > 0,
        "precondition: an insert past the arena's room allocates, and the count sees it"
    );
}

#[test]
fn test_another_threads_allocation_is_not_counted() {
    // The other thread allocates after `start` and before `done`, and the
    // measuring thread passes both inside its window: the allocation
    // falls in the window, whatever the scheduler does.
    let (start, done) = (Barrier::new(2), Barrier::new(2));
    std::thread::scope(|s| {
        let other = s.spawn(|| {
            start.wait();
            let own = measure(|| {
                std::hint::black_box(vec![0_u8; 900]);
            });
            done.wait();
            own.bytes_total
        });
        let counted = measure(|| {
            start.wait();
            done.wait();
        })
        .bytes_total;
        let own = other.join().expect("the other thread allocates and returns");
        assert!(
            own >= 900,
            "precondition: the other thread allocated, and a count on it sees {own} bytes"
        );
        assert_eq!(
            counted, 0,
            "another thread's allocation inside the window was counted: {counted} bytes"
        );
    });
}
