// SPDX-License-Identifier: MPL-2.0

//! Authored glyph-layer mutations built from choices, `NaN` included,
//! applied to a tree of box and path runs and fills built the same way,
//! with every property of
//! `baumhard::gfx_structs::tests::glyph_layer_fuzz::check_glyph_mutations_structured`
//! (TEST_CONVENTIONS §T14).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(why) = baumhard::gfx_structs::tests::glyph_layer_fuzz::check_glyph_mutations_structured(bytes)
    {
        panic!("{why}");
    }
});
