// SPDX-License-Identifier: MPL-2.0

//! Hex-string → [`cosmic_text::Color`] bridge. Lives in `font/`
//! per §B5 ("cosmic-text usage is concentrated in
//! `lib/baumhard/src/font/`") so app code outside the renderer
//! reaches a `cosmic_text::Color` through this single entry point
//! instead of importing `cosmic_text` directly. The underlying
//! length-and-nibble parsing lives in
//! [`crate::util::color_conversion::hex_to_rgba`] — that primitive
//! has no cosmic-text dependency and stays usable from non-font
//! callers (e.g. background-fill resolution in
//! `mindmap::tree_builder::node`).

use crate::util::color_conversion::{convert_f32_to_u8, convert_u8_to_f32, hex_to_rgba};

/// Parse a hex color string into a [`cosmic_text::Color`], returning
/// `None` on any parse failure. Accepts 3, 4, 6, or 8 hex chars with
/// an optional leading `#`. Used by render-time paths
/// (`renderer/borders.rs` etc.) where a typo in a theme variable
/// must not crash but must also not silently substitute a fallback —
/// the caller picks the per-element default (cyan handles, light-grey
/// labels) rather than baking it into the parser.
///
/// **Cost.** O(len) over the input string for the underlying
/// `hex_to_rgba` walk plus a single [`cosmic_color_from_rgba`]
/// quantisation; no heap allocation.
pub fn hex_to_cosmic_color(color: &str) -> Option<cosmic_text::Color> {
    Some(cosmic_color_from_rgba(hex_to_rgba(color)?))
}

/// Quantise a `[f32; 4]` RGBA in `[0, 1]` into a [`cosmic_text::Color`].
/// Single boundary helper for code on the float side (the picker's
/// HSV math, the renderer's pre-quantised palette resolves) needing
/// to hand a colour to cosmic-text.
#[inline]
pub fn cosmic_color_from_rgba(rgba: [f32; 4]) -> cosmic_text::Color {
    let u = convert_f32_to_u8(&rgba);
    cosmic_text::Color::rgba(u[0], u[1], u[2], u[3])
}

/// Read a [`cosmic_text::Color`]'s byte channels back into a
/// `[f32; 4]` RGBA in `[0, 1]`. Inverse of [`cosmic_color_from_rgba`]
/// within rounding slack; routes through
/// [`crate::util::color_conversion::convert_u8_to_f32`] so every
/// byte→float quantisation in the project lands on the same arithmetic.
#[inline]
pub fn cosmic_color_to_rgba(color: cosmic_text::Color) -> [f32; 4] {
    convert_u8_to_f32(&[color.r(), color.g(), color.b(), color.a()])
}
