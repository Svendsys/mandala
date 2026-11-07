// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` loader on maps built from choices: deep chains
//! with a link on every node, wide fans, dense rows, random trees, with
//! at most one injected fault, judged against the differential oracle
//! and the reference model of
//! `baumhard::document::tests::format_fuzz::check_structured`
//! (TEST_CONVENTIONS §T14). A map that loads is projected too, inside the
//! projection's work bound; at every power of two and every 256 inputs the target reports how many
//! it projected and how many it skipped (ruling B7.3 on #241).

#![no_main]

use std::sync::atomic::{AtomicUsize, Ordering};

use libfuzzer_sys::fuzz_target;

static INPUTS: AtomicUsize = AtomicUsize::new(0);

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::format_fuzz::check_structured(bytes) {
        panic!("{why}");
    }
    let inputs = INPUTS.fetch_add(1, Ordering::Relaxed) + 1;
    if inputs.is_power_of_two() || inputs % 256 == 0 {
        let (projected, skipped) = baumhard::projection::tests::projection_fuzz::projection_counts();
        eprintln!("projection: {projected} loaded maps projected, {skipped} outside the bound and skipped");
    }
});
