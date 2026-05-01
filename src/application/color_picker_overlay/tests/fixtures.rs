// SPDX-License-Identifier: MPL-2.0

//! Shared test fixtures for the color picker overlay tests. The
//! geometry-stub builder is the same as the one in
//! `color_picker::tests::fixtures::sample_geometry` — same struct,
//! same field values — so it lives once under the conceptual
//! owner (the picker, where `ColorPickerOverlayGeometry` is
//! defined) and is re-exported here under the historical
//! `picker_sample_geometry` name to keep call sites unchanged.

use baumhard::gfx_structs::area::GlyphArea;

use crate::application::color_picker::{compute_color_picker_layout, ColorPickerOverlayGeometry};
use crate::application::color_picker_overlay::picker_glyph_areas::picker_glyph_areas;

pub(super) use crate::application::color_picker::tests::fixtures::sample_geometry as picker_sample_geometry;

/// Compute the picker's channel-ordered `(channel, GlyphArea)` list
/// at the canonical 1280×720 viewport — what every test that reasons
/// about emitted areas wants.
pub(super) fn picker_glyph_areas_for(geometry: &ColorPickerOverlayGeometry) -> Vec<(usize, GlyphArea)> {
    let layout = compute_color_picker_layout(geometry, 1280.0, 720.0);
    picker_glyph_areas(geometry, &layout)
}
