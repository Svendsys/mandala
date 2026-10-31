// SPDX-License-Identifier: MPL-2.0

//! Authored glyph-layer mutations as a file spells them: any bytes read
//! as JSON mutations and applied by the journaled walker to a tree of
//! runs and fills, with every property of
//! `baumhard::gfx_structs::tests::glyph_layer_fuzz::check_glyph_mutations_json`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::gfx_structs::tests::glyph_layer_fuzz::check_glyph_mutations_json(bytes) {
        panic!("{why}");
    }
});
