// SPDX-License-Identifier: MPL-2.0

//! A glyph-layer presentation's body, held to the fuzz properties of
//! authored run mutations: what the presentations fuzz properties
//! (`document::tests::presentation_fuzz`) check of every glyph-layer
//! definition that reads. The document names no glyph type, so the
//! check goes through here, where a body becomes run mutations
//! ([`glyph_mutations`]).

use crate::document::presentation::GlyphOps;
use crate::gfx_structs::tests::glyph_layer_fuzz;
use crate::projection::glyph_mutations;

/// Every property of [`glyph_layer_fuzz::check_mutations`], of the run
/// mutations `ops` spell.
pub fn check_glyph_body(ops: &GlyphOps) -> Result<(), String> {
    glyph_layer_fuzz::check_mutations(&glyph_mutations(ops))
}
