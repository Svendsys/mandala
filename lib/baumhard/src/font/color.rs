// SPDX-License-Identifier: MPL-2.0

//! Float-RGBA ↔ [`cosmic_text::Color`] bridge. Single source of
//! truth for byte ↔ float quantisation at the cosmic-text wall;
//! every renderer / picker call site that needs to hand cosmic-text
//! a colour or read one back goes through these two helpers, which
//! delegate to [`crate::util::color_conversion::convert_f32_to_u8`]
//! / [`convert_u8_to_f32`] so the arithmetic stays single-sourced.

use crate::util::color_conversion::{convert_f32_to_u8, convert_u8_to_f32};

/// Quantise a `[f32; 4]` RGBA in `[0, 1]` into a [`cosmic_text::Color`].
/// Used on the float side (the picker's HSV math, the renderer's
/// pre-quantised palette resolves) when handing a colour to
/// cosmic-text.
#[inline]
pub fn cosmic_color_from_rgba(rgba: [f32; 4]) -> cosmic_text::Color {
    let u = convert_f32_to_u8(&rgba);
    cosmic_text::Color::rgba(u[0], u[1], u[2], u[3])
}

/// Read a [`cosmic_text::Color`]'s byte channels back into a
/// `[f32; 4]` RGBA in `[0, 1]`. Inverse of [`cosmic_color_from_rgba`]
/// within rounding slack; routes through [`convert_u8_to_f32`] so
/// every byte→float quantisation in the project lands on the same
/// arithmetic.
#[inline]
pub fn cosmic_color_to_rgba(color: cosmic_text::Color) -> [f32; 4] {
    convert_u8_to_f32(&[color.r(), color.g(), color.b(), color.a()])
}
