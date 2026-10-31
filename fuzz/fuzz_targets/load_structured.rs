// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` loader on maps built from choices: deep chains
//! with a link on every node, wide fans, dense rows, random trees, with
//! at most one injected fault, judged against the differential oracle
//! and the reference model of
//! `baumhard::document::tests::format_fuzz::check_structured`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::format_fuzz::check_structured(bytes) {
        panic!("{why}");
    }
});
