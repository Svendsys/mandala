// SPDX-License-Identifier: MPL-2.0

//! What a map costs in memory while it loads and is saved, measured
//! natively, and wasm32's two limits held to it (design §C.8, R17.2).
//!
//! **Why.** On wasm32 the address space is 4 GiB, and a failed
//! allocation traps and kills the page. [`MAX_MAP_BYTES`] and
//! [`MAX_ELEMENTS`] are what refuse a map the browser cannot hold
//! before it tries, so each must leave room for what a map becomes. The
//! model (`baumhard::document::tests::format_memory_shapes`) says a
//! map's peak is at most `text × WASM32_BYTES_PER_TEXT_BYTE + elements ×
//! WASM32_BYTES_PER_ELEMENT`; each limit is half the budget over its
//! cost, so a map under both peaks under the budget whatever its shape.
//!
//! **What this measures.** The densest maps the format allows of each
//! kind of content, at their worst point of growth, loaded and then
//! saved, under a counting allocator: the requested bytes at the peak.
//! It derives the two costs from them — the cost per byte from the text
//! shapes, the cost per element from what the element shapes cost
//! beyond their text — and asserts that the pinned constants are those
//! costs rounded up, so a change that makes a map costlier fails here.
//!
//! **What it does not.** Requested bytes are not the allocator's
//! footprint: wasm32's `dlmalloc` adds a header and rounding to each
//! allocation, copies on a `realloc` it cannot do in place, and leaves
//! freed chunks it cannot reuse for a larger request. The wasm32 probe
//! (`tests/wasm_font_lock.rs`, the `memory_*` scenarios) measures the
//! same maps by the module's own memory, which counts all of that, and
//! holds them to the same model.
//!
//! **One process per map.** The allocator's peak cannot be reset, so
//! the test runs itself again for each map, and each child reports one
//! line.

use baumhard::document::format::{
    MAX_ELEMENTS, MAX_MAP_BYTES, WASM32_BYTES_PER_ELEMENT, WASM32_BYTES_PER_TEXT_BYTE, WASM32_MAX_ELEMENTS,
    WASM32_MAX_MAP_BYTES, WASM32_MEMORY_BUDGET,
};
use baumhard::document::tests::format_memory_shapes::{
    derive, measure, measured_text, Measured, ARENA_POINT, ELEMENT_SHAPES, TEXT_SHAPES,
};
use cap::Cap;
use std::alloc::System;

#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// Set in a child: the map it measures.
const SHAPE: &str = "MANDALA_FORMAT_MEMORY_SHAPE";

/// Run this test again for `shape`, and read what it measured.
fn measured_in_a_child(shape: &str) -> Measured {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(&exe)
        .args([
            "--exact",
            "test_wasm32_limits_are_derived_from_what_a_map_costs",
            "--nocapture",
        ])
        .env(SHAPE, shape)
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{shape}: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .find_map(Measured::parse)
        .unwrap_or_else(|| panic!("{shape} reported nothing:\n{stdout}"))
}

#[test]
fn test_wasm32_limits_are_derived_from_what_a_map_costs() {
    if let Ok(shape) = std::env::var(SHAPE) {
        let base = ALLOCATOR.allocated() as u64;
        let text = measured_text(&shape);
        let measured = measure(&shape, text, base, || ALLOCATOR.max_allocated() as u64);
        println!("{}", measured.line());
        return;
    }

    // The limits are each half the budget over their cost.
    assert_eq!(
        WASM32_MAX_MAP_BYTES,
        WASM32_MEMORY_BUDGET / 2 / WASM32_BYTES_PER_TEXT_BYTE
    );
    assert_eq!(
        WASM32_MAX_ELEMENTS,
        WASM32_MEMORY_BUDGET / 2 / WASM32_BYTES_PER_ELEMENT
    );
    #[cfg(target_arch = "wasm32")]
    assert_eq!(
        (MAX_MAP_BYTES, MAX_ELEMENTS),
        (Some(WASM32_MAX_MAP_BYTES), Some(WASM32_MAX_ELEMENTS))
    );
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!((MAX_MAP_BYTES, MAX_ELEMENTS), (None, None));

    let mut all = Vec::new();
    for shape in TEXT_SHAPES.iter().chain(&ELEMENT_SHAPES) {
        let m = measured_in_a_child(shape);
        println!(
            "{shape}: {} bytes read, {} written, {} elements, peaks {} loading and {} saving",
            m.text_in, m.text_out, m.elements, m.load, m.both
        );
        all.push(m);
    }
    // Preconditions: each map is as big as it was meant to be, so the
    // process's own baseline is noise beside it. Verbs stop at
    // `MAX_VERBS`, short of the others' count.
    for m in &all {
        if m.shape == "verbs" {
            assert!(
                m.elements > 1 << 16,
                "precondition: verbs has {} elements",
                m.elements
            );
        } else if ELEMENT_SHAPES.contains(&m.shape.as_str()) {
            assert!(
                m.elements >= ARENA_POINT,
                "precondition: {} has {} elements",
                m.shape,
                m.elements
            );
        } else {
            assert!(
                m.text_in > 1_000_000,
                "precondition: {} is {} bytes",
                m.shape,
                m.text_in
            );
        }
    }
    let (per_byte, per_element) = derive(&all);
    println!("native: {per_byte:.2} bytes per byte of text, {per_element:.1} bytes per element beyond it");
    // The pins come from wasm32's measurement (the probe), which is the
    // higher: here, every map must simply be within the model.
    for m in &all {
        if let Err(why) = m.check() {
            panic!("{why}");
        }
    }
}
