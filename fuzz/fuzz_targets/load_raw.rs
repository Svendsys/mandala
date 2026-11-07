// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` loader on any bytes: every property of
//! `baumhard::document::tests::format_fuzz::check_load_bytes` — no panic,
//! a well-formed refusal, and a valid, savable, round-tripping document
//! for what loads, projected inside the projection's work bound
//! (TEST_CONVENTIONS §T14). At every power of two and every 256 inputs
//! the target reports how many loaded maps it projected and how many it
//! skipped (ruling B7.3 on #241).

#![no_main]

use std::sync::atomic::{AtomicUsize, Ordering};

use libfuzzer_sys::fuzz_target;

static INPUTS: AtomicUsize = AtomicUsize::new(0);

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::format_fuzz::check_load_bytes(bytes) {
        panic!("{why}");
    }
    let inputs = INPUTS.fetch_add(1, Ordering::Relaxed) + 1;
    if inputs.is_power_of_two() || inputs % 256 == 0 {
        let (projected, skipped) = baumhard::projection::tests::projection_fuzz::projection_counts();
        eprintln!("projection: {projected} loaded maps projected, {skipped} outside the bound and skipped");
    }
});
