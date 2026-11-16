// SPDX-License-Identifier: MPL-2.0

//! Presentations as a file spells them: any bytes read as a
//! presentations file, and each presentation that reads played on a
//! map — started, brought through frames, with edits made under it,
//! stopped — with every property of
//! `baumhard::document::tests::presentation_fuzz::check_presentations_json`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::document::tests::presentation_fuzz::check_presentations_json(bytes) {
        panic!("{why}");
    }
});
