// SPDX-License-Identifier: MPL-2.0

//! `compute_color_picker_layout` — pure-function layout pass.
//! Orchestrates the two-step derivation in `compute_sizing` (font
//! size, ring radius, cell step) and `compute_positions` (per-cell
//! anchors, backdrop, title / hint / hex positions).
//!
//! No GPU access, no font system — unit tests construct a layout
//! from nothing but a geometry struct + screen dimensions.

use super::compute_positions::compute_positions;
use super::compute_sizing::derive_sizing;
use super::geometry::ColorPickerOverlayGeometry;
use super::layout::ColorPickerLayout;
use crate::application::widgets::color_picker_widget::load_spec;

/// Three-stage pure-function layout: sizing in [`super::compute_sizing`],
/// then per-cell positions in [`super::compute_positions`].
pub fn compute_color_picker_layout(
    geometry: &ColorPickerOverlayGeometry,
    screen_w: f32,
    screen_h: f32,
) -> ColorPickerLayout {
    let spec = load_spec();
    let g = &spec.geometry;
    let sizing = derive_sizing(geometry, g, screen_w, screen_h);
    compute_positions(geometry, g, screen_w, screen_h, sizing)
}
