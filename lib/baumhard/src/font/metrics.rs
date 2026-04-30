// SPDX-License-Identifier: MPL-2.0

//! Font-metric approximations that don't need a `FontSystem` to
//! compute. Renderer + scene-builder paths sometimes need a quick
//! "approximately how wide is one character?" answer at frame-build
//! time, before any glyph has been shaped — those callers used to
//! open-code `font_size * 0.6` with the same explanatory comment
//! at five sites. Single-source the constant + helper here so a
//! font-face calibration update lands in one place.
//!
//! Real glyph measurement (per-face advance, ink bounds) lives in
//! `super::fonts` via `measure_glyph_ink_bounds`; reach for
//! those when a `FontSystem` is in scope. The helper here is the
//! coarse estimate used when one isn't.

/// Approximate ratio of a glyph's horizontal advance to its
/// nominal `font_size_pt` in the box-drawing-friendly monospace
/// faces Mandala ships. Calibrated against LiberationSans-style
/// monospace; consumers that need per-face precision should
/// measure through cosmic-text instead.
///
/// Used by both crates: renderer hot paths (border layout,
/// connection-label placement, console glyph sizing) and the
/// color-picker layout in the app crate.
pub const MONOSPACE_ADVANCE_RATIO: f32 = 0.6;

/// Approximate per-glyph horizontal advance for a monospace face
/// at the given `font_size_pt`. Multiplies by
/// [`MONOSPACE_ADVANCE_RATIO`].
///
/// **Cost.** O(1); no allocation, no font-system access. Safe to
/// call from per-frame layout paths.
#[inline]
pub fn monospace_advance(font_size_pt: f32) -> f32 {
    font_size_pt * MONOSPACE_ADVANCE_RATIO
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::geometry::almost_equal;

    #[test]
    fn test_monospace_advance_zero_is_zero() {
        assert_eq!(monospace_advance(0.0), 0.0);
    }

    #[test]
    fn test_monospace_advance_scales_linearly() {
        assert!(almost_equal(monospace_advance(10.0), 6.0));
        assert!(almost_equal(monospace_advance(20.0), 12.0));
        assert!(almost_equal(
            monospace_advance(33.3),
            33.3 * MONOSPACE_ADVANCE_RATIO,
        ));
    }

    #[test]
    fn test_monospace_advance_ratio_is_zero_point_six() {
        assert_eq!(MONOSPACE_ADVANCE_RATIO, 0.6);
    }
}
