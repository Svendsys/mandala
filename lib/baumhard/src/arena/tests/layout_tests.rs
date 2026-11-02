// SPDX-License-Identifier: MPL-2.0

//! The arena's sizes, pinned on both targets (V1-A7, #301): a
//! [`NodeId`](crate::arena::NodeId), an optional one, a node's links, and a slot for the
//! element types the trees hold. A change that makes any of them larger
//! — a link back to a full `NodeId`, a free-list link back to a
//! `usize`, a slot that loses its niche — fails here natively and, in
//! the wasm32 probe (`tests/wasm_font_lock.rs`), on wasm32.

use crate::arena::{Link, Node, NodeId, Slot};
use crate::document::element::DocElement;
use crate::gfx_structs::element::GfxElement;
use std::mem::size_of;

/// Every size pinned: what it is, and its size in bytes.
pub fn arena_sizes() -> [(&'static str, usize); 9] {
    [
        ("NodeId", size_of::<NodeId>()),
        ("Option<NodeId>", size_of::<Option<NodeId>>()),
        ("a link", size_of::<Option<Link>>()),
        ("Node<()>: its links and generation", size_of::<Node<()>>()),
        ("Slot<()>", size_of::<Slot<()>>()),
        ("Slot<u32>", size_of::<Slot<u32>>()),
        ("Slot<u64>", size_of::<Slot<u64>>()),
        (
            "Slot<DocElement> beyond the element",
            size_of::<Slot<DocElement>>() - size_of::<DocElement>(),
        ),
        (
            "Slot<GfxElement> beyond the element",
            size_of::<Slot<GfxElement>>() - size_of::<GfxElement>(),
        ),
    ]
}

/// The sizes [`arena_sizes`] must report on this target.
#[cfg(target_pointer_width = "64")]
pub const PINNED: [usize; 9] = [8, 8, 4, 24, 28, 32, 40, 24, 24];

/// The sizes [`arena_sizes`] must report on this target.
#[cfg(target_pointer_width = "32")]
pub const PINNED: [usize; 9] = [8, 8, 4, 24, 28, 32, 40, 24, 24];

#[test]
fn test_arena_slot_and_id_sizes() {
    do_arena_slot_and_id_sizes();
}

/// Every size [`arena_sizes`] reports is the one [`PINNED`] for this
/// target; the message names each that is not, with both numbers.
pub fn do_arena_slot_and_id_sizes() {
    let wrong: Vec<String> = arena_sizes()
        .iter()
        .zip(PINNED)
        .filter(|((_, size), pinned)| size != pinned)
        .map(|((what, size), pinned)| format!("{what}: {size} bytes, pinned at {pinned}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "the arena's sizes changed on this target: {}",
        wrong.join("; ")
    );
}
