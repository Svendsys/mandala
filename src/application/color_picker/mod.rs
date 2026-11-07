// SPDX-License-Identifier: MPL-2.0

//! Magical glyph-wheel color picker — a custom modal overlay for choosing
//! colors that fits Mandala's "everything is a positioned glyph" aesthetic.
//!
//! Layout: a hue ring of `HUE_SLOT_COUNT` glyphs forming a mandala, a
//! crosshair sat/value selector formed by two perpendicular glyph bars
//! meeting at the wheel center, and a central preview glyph showing the
//! currently-previewed color. (A row of theme-variable quick-pick chips
//! used to sit below; it has been retired — see `compute_sizing.rs`,
//! where its width reservation was removed — and nothing cycles chips
//! any more.) Mouse hover updates the preview live; click commits,
//! click outside cancels. Keyboard fallback: h/H nudges hue, s/S sat,
//! v/V value, Enter commits, Esc cancels.
//!
//! **What it writes to returns with V1-C7 (#248).** The picker is a
//! transaction there: opening it is `Document::begin`, each hover
//! change stages the color op for its target, a click commits and a
//! close or cancel cancels. This module is the widget — layout,
//! hit-testing, glyph tables, its modal state — and nothing in it reads
//! or writes the document.
//!
//! Pure-function layout (`compute_color_picker_layout`) and hit-testing
//! (`hit_test_picker`) are extracted so unit tests don't need a GPU.

mod clipboard;
mod compute;
mod compute_positions;
mod compute_sizing;
mod geometry;
mod glyph_tables;
mod hit;
mod layout;
mod state;

#[cfg(test)]
pub(in crate::application) mod tests;

// Cross-platform surface — consumed by the picker widget on both
// native and WASM (the spec-driven dynamic-context tree builder is
// cross-platform).
pub use geometry::ColorPickerOverlayGeometry;
pub use glyph_tables::{
    arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
    center_preview_glyph, hue_ring_glyphs, hue_slot_to_degrees, picker_channel, sat_cell_to_value,
    sat_value_to_cell, val_cell_to_value, val_value_to_cell, CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT,
    SAT_CELL_COUNT, VAL_CELL_COUNT,
};
pub use hit::PickerHit;
pub use layout::ColorPickerLayout;

// Native-only surface — consumed exclusively by `app/color_picker_flow/`
// (mouse / open / commit / rebuild) and `run_native.rs`. WASM has
// no inline color-picker modal yet, so the symbols below have no
// reachable consumer there.
#[cfg(not(target_arch = "wasm32"))]
pub use compute::compute_color_picker_layout;
#[cfg(not(target_arch = "wasm32"))]
pub use glyph_tables::hue_ring_font_scale;
#[cfg(not(target_arch = "wasm32"))]
pub use hit::hit_test_picker;
#[cfg(not(target_arch = "wasm32"))]
pub use state::{
    request_error_flash, ColorPickerState, FlashKind, OpenPicker, PickerDynamicApplyKey, PickerGesture,
    PickerMode,
};
