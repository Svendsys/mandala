// SPDX-License-Identifier: MPL-2.0

//! Presentations built from choices — every play, both layers, timings
//! at their bounds and past them, names a map may lack — played on a
//! map with edits built from choices made under them, with every
//! property of
//! `baumhard::document::tests::presentation_fuzz::check_presentations_structured`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::presentation_fuzz::check_presentations_structured(bytes) {
        panic!("{why}");
    }
});
