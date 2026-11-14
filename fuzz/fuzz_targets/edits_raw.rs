// SPDX-License-Identifier: MPL-2.0

//! Authored edits as a file spells them: any bytes read as an edits
//! file, and each edit that reads run on a map, with every property of
//! `baumhard::document::tests::edit_fuzz::check_edits_json`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::edit_fuzz::check_edits_json(bytes) {
        panic!("{why}");
    }
});
