// SPDX-License-Identifier: MPL-2.0

//! Verbs built from choices — every step kind and target, names the map
//! has and lacks, waits at and past their bound, cycles, chains past the
//! depth and fans past the parts — held to a reference model's verdict
//! and run, and span links of drawn schemes at load and at apply, with
//! every property of
//! `baumhard::document::tests::verb_fuzz::check_verbs_structured`
//! (TEST_CONVENTIONS §T14; design D29, #252).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::verb_fuzz::check_verbs_structured(bytes) {
        panic!("{why}");
    }
});
