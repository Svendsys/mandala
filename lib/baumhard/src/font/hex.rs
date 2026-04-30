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

use crate::util::color_conversion::hex_to_rgba;

/// Parse a hex color string into a [`cosmic_text::Color`], returning
/// `None` on any parse failure. Accepts 3, 4, 6, or 8 hex chars with
/// an optional leading `#`. Used by render-time paths
/// (`renderer/borders.rs` etc.) where a typo in a theme variable
/// must not crash but must also not silently substitute a fallback —
/// the caller picks the per-element default (cyan handles, light-grey
/// labels) rather than baking it into the parser.
///
/// **Cost.** O(len) over the input string for the underlying
/// `hex_to_rgba` walk plus four `(f32 * 255.0).round() as u8` casts;
/// no heap allocation.
pub fn hex_to_cosmic_color(color: &str) -> Option<cosmic_text::Color> {
    let [r, g, b, a] = hex_to_rgba(color)?;
    Some(cosmic_text::Color::rgba(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        (a * 255.0).round() as u8,
    ))
}
