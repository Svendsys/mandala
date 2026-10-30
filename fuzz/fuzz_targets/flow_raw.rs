// SPDX-License-Identifier: MPL-2.0

//! TextFlow on any bytes, read as text: shaped as a box and as a run,
//! placed around an outline, along a line and at a point, with every
//! property of `baumhard::flow::tests::flow_fuzz::check_flow_bytes`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::flow::tests::flow_fuzz::check_flow_bytes(bytes) {
        panic!("{why}");
    }
});
