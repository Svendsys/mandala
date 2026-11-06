// SPDX-License-Identifier: MPL-2.0

//! TextFlow on spans built from choices, as flow_structured, with the
//! faces drawn from every face a map can pin: flow_structured's list and
//! the ornament fallback's two (#297), `Mandala Ornaments` and the
//! outline-free `Mandala Emoji`. A list of its own, with a corpus and
//! seeds of its own, so that flow_structured's committed seeds and
//! findings keep the faces they chose, and this target's replay in
//! theirs: every property of
//! `baumhard::flow::tests::flow_fuzz::check_flow_structured_every_face`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::flow::tests::flow_fuzz::check_flow_structured_every_face(bytes) {
        panic!("{why}");
    }
});
