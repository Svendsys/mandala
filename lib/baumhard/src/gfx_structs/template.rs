// SPDX-License-Identifier: MPL-2.0

//! The glyph payload's builder template: a `Vec<CellField>` is
//! [`GfxMutation`](crate::gfx_structs::mutator::GfxMutation)'s [`Mutation::Template`](crate::engine::mutation::Mutation::Template).
//! Inside a `Repeat`, a
//! [`MutationSrc::Template`](crate::engine::builder::MutationSrc::Template)
//! carrying one resolves, per iteration, into one
//! [`GfxMutation::AreaDelta`](crate::gfx_structs::mutator::GfxMutation::AreaDelta) whose fields are pulled from that cell's
//! `GlyphArea` (bare variants) or baked in (tagged variants). A
//! [`BuildContext`](crate::engine::builder::BuildContext) over glyph
//! trees resolves with [`area_delta_from_template`](crate::gfx_structs::template::area_delta_from_template()), either straight
//! from a pre-built area ([`resolve_from_area`](crate::gfx_structs::template::resolve_from_area())) or from a slimmer
//! per-field source on hot paths.

use crate::core::primitives::ApplyOperation;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};
use crate::gfx_structs::mutator::GfxMutation;
use serde::{Deserialize, Serialize};

/// Per-cell `AreaDelta` field slot. Bare variants = "supplied at
/// runtime by the area lookup"; tagged variants = baked-in literals
/// reused for every cell.
//
// The lowercase variants (`position`, `bounds`, `scale`, `line_height`)
// match the snake_case JSON tags consumers emit for runtime-supplied
// fields; renaming them would break the on-the-wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
#[non_exhaustive]
pub enum CellField {
    /// Pull the cell's text payload from the per-section area
    /// lookup at build time.
    Text,
    /// Pull the cell's canvas-space position
    /// (`OrderedVec2`) from the area lookup.
    position,
    /// Pull the cell's render bounds (width, height in canvas
    /// pixels) from the area lookup.
    bounds,
    /// Pull the cell's font-size scalar from the area lookup.
    scale,
    /// Pull the cell's line-height multiplier from the area lookup.
    line_height,
    /// Pull the cell's [`ColorFontRegions`](crate::core::primitives::ColorFontRegions)
    /// styled-span set from the area lookup.
    ColorFontRegions,
    /// Pull the cell's optional halo outline payload from the
    /// area lookup.
    Outline,
    /// Bake an [`ApplyOperation`] literal into the delta — every
    /// cell built from this template uses the same arithmetic
    /// (`Add`, `Assign`, `Subtract`, `Delete`, ...).
    Operation(ApplyOperation),
}

/// The [`GfxMutation::AreaDelta`] a template stands for, with every
/// slot materialized by `field`. One `Vec` of `template.len()` fields
/// plus the delta's box.
pub fn area_delta_from_template(
    template: &[CellField],
    field: impl FnMut(&CellField) -> GlyphAreaField,
) -> GfxMutation {
    GfxMutation::area_delta(DeltaGlyphArea::new(template.iter().map(field).collect()))
}

/// [`area_delta_from_template`] with every bare slot pulled out of a
/// pre-built `area` by [`field_from_area`]. Clones the text and the
/// region set when the template names them.
pub fn resolve_from_area(area: &GlyphArea, template: &[CellField]) -> GfxMutation {
    area_delta_from_template(template, |cell| field_from_area(area, cell))
}

/// One template slot, pulled out of a pre-built `area`: the bare
/// variants copy the area's field, `Operation` bakes its literal in.
/// Clones the text for `Text` and the region set for
/// `ColorFontRegions`; O(1) otherwise.
pub fn field_from_area(area: &GlyphArea, cell: &CellField) -> GlyphAreaField {
    match cell {
        CellField::Text => GlyphAreaField::Text(area.text.clone()),
        CellField::position => GlyphAreaField::position(area.position.x.0, area.position.y.0),
        CellField::bounds => GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
        CellField::scale => GlyphAreaField::scale(area.scale.0),
        CellField::line_height => GlyphAreaField::line_height(area.line_height.0),
        CellField::ColorFontRegions => GlyphAreaField::ColorFontRegions(area.regions.clone()),
        CellField::Outline => GlyphAreaField::Outline(area.outline),
        CellField::Operation(op) => GlyphAreaField::Operation(*op),
    }
}
