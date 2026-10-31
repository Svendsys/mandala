// SPDX-License-Identifier: MPL-2.0

//! TextFlow on spans built from choices: hostile text in every face,
//! size, weight and slant, shaped and placed in every layout and fit
//! mode on hostile geometry, with every property of
//! `baumhard::flow::tests::flow_fuzz::check_flow_structured`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::flow::tests::flow_fuzz::check_flow_structured(bytes) {
        panic!("{why}");
    }
});
