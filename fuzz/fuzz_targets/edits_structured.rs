// SPDX-License-Identifier: MPL-2.0

//! Authored edits built from choices — every body, nested past the depth
//! bound, names a map may lack — run on a map, with every property of
//! `baumhard::document::tests::edit_fuzz::check_edits_structured`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::edit_fuzz::check_edits_structured(bytes) {
        panic!("{why}");
    }
});
