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
///
/// **Forward-compat (§B10).** This is currently a single
/// face-calibrated constant — exposing it as `pub` honours
/// today's "in-tree app needs it" need but isn't a stable
/// plugin contract. When the plugin font-metrics API lands
/// it will own per-face calibration; this constant becomes
/// either internal scaffolding or moves under that surface.
/// Don't reach for it from plugin code as if it were a
/// shipped API.
pub const MONOSPACE_ADVANCE_RATIO: f32 = 0.6;

/// Approximate per-glyph horizontal advance for a monospace face
/// at the given `font_size_pt`. Multiplies by
/// [`MONOSPACE_ADVANCE_RATIO`].
///
/// **Cost.** O(1); no allocation, no font-system access. Safe to
/// call from per-frame layout paths.
///
/// **NaN / non-finite inputs.** Returns NaN / ±∞ unchanged —
/// this is a multiplicative shape, not a sanitiser. Every
/// in-tree caller pre-clamps its `font_size_pt` to a finite
/// positive value (via [`is_positive_finite`] at the parse
/// boundary, or [`f32::clamp`] inside `effective_font_size_pt`).
/// New callers MUST do the same; threading NaN through layout
/// math propagates into every downstream pixel.
///
/// **Forward-compat (§B10).** See the note on
/// [`MONOSPACE_ADVANCE_RATIO`] — this helper inherits the same
/// "single-face calibration" caveat. Per-face calibration will
/// belong to the plugin font-metrics API when it lands; treat
/// this `pub` exposure as in-tree scaffolding, not a plugin
/// contract.
///
/// [`is_positive_finite`]: crate::util::geometry::is_positive_finite
#[inline]
pub fn monospace_advance(font_size_pt: f32) -> f32 {
    font_size_pt * MONOSPACE_ADVANCE_RATIO
}
