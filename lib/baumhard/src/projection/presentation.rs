// SPDX-License-Identifier: MPL-2.0

//! A glyph-layer presentation's body as run mutations (#251 §4, ruling
//! D2.1 §1). The document holds a glyph-layer body in its own words —
//! [`GlyphOps`], what a presentations file spells — and lists the
//! instances that play it; it names no glyph type. What those words
//! mean on a run is said here, beside the rest of what the document
//! becomes on the glyph layers, and whoever writes the glyph layers
//! reads it.

use crate::document::format::presentations::{GlyphOpDto, RunOverrideDto};
use crate::document::presentation::GlyphOps;
use crate::document::values::Rgba;
use crate::gfx_structs::mutator::GfxMutation;
use crate::gfx_structs::run::RunOverrideOp;
use glam::Vec2;

/// The run mutations `ops` spell, in order: `run_opacity` is
/// [`GfxMutation::RunOpacity`], `run_origin` [`GfxMutation::RunOrigin`],
/// and each `run_overrides` edit the [`GfxMutation::RunOverrides`] of the
/// same name, its `[start, end]` the half-open range `start..end`.
///
/// Cost: O(the ops), one allocation.
pub fn glyph_mutations(ops: &GlyphOps) -> Vec<GfxMutation> {
    ops.0.iter().map(glyph_mutation).collect()
}

/// The run mutation `dto` spells.
fn glyph_mutation(dto: &GlyphOpDto) -> GfxMutation {
    let range = |[start, end]: [usize; 2]| start..end;
    match dto {
        GlyphOpDto::Opacity(opacity) => GfxMutation::RunOpacity(*opacity),
        GlyphOpDto::Origin([x, y]) => GfxMutation::RunOrigin(Vec2::new(*x, *y)),
        GlyphOpDto::Overrides(op) => GfxMutation::RunOverrides(match op {
            RunOverrideDto::Offset { range: r, value } => RunOverrideOp::Offset {
                range: range(*r),
                value: Vec2::new(value[0], value[1]),
            },
            RunOverrideDto::Color { range: r, value } => RunOverrideOp::Color {
                range: range(*r),
                value: value.map(|hex| Rgba(hex.0)),
            },
            RunOverrideDto::Scale { range: r, value } => RunOverrideOp::Scale {
                range: range(*r),
                value: *value,
            },
            RunOverrideDto::Clear => RunOverrideOp::Clear,
        }),
    }
}
