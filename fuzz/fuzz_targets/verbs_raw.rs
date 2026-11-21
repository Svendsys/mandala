// SPDX-License-Identifier: MPL-2.0

//! A map's verbs as a file spells them: any bytes read as the `verbs`
//! value of a fixed map, and each verb that loads invoked, framed
//! through its waits and released, with every property of
//! `baumhard::document::tests::verb_fuzz::check_verbs_json`
//! (TEST_CONVENTIONS §T14; design D29, #252).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::verb_fuzz::check_verbs_json(bytes) {
        panic!("{why}");
    }
});
