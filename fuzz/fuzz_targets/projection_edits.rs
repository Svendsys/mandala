// SPDX-License-Identifier: MPL-2.0

//! The projection under edit sequences (V1-B7, #241; ruling B7.3): a map
//! of fifty nodes or more built from choices, then up to sixteen edits
//! from the input — field edits, structural edits, a rebinding near the
//! root, theme and palette changes, transaction stages, undos and redos —
//! the layers the updates keep held to a full projection after each,
//! by `baumhard::projection::tests::projection_fuzz::check_projection_edits`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::projection::tests::projection_fuzz::check_projection_edits(bytes) {
        panic!("{why}");
    }
});
