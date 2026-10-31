// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` loader on any bytes: every property of
//! `baumhard::document::tests::format_fuzz::check_load_bytes` — no panic,
//! a well-formed refusal, and a valid, savable, round-tripping document
//! for what loads (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::format_fuzz::check_load_bytes(bytes) {
        panic!("{why}");
    }
});
