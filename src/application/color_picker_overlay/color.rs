// SPDX-License-Identifier: MPL-2.0

//! RGB → cosmic-text conversion and highlight mixes shared by the
//! picker's tree / mutator / area builders.

use baumhard::font::hex::cosmic_color_from_rgba;

/// Convert a normalized `[0, 1]` opaque RGB triple into a
/// [`baumhard::font::Color`]. Used by the glyph-wheel color picker
/// render path to paint each hue-ring slot, sat/val cell, and
/// preview glyph at its own HSV coordinate without per-frame closure
/// allocation. Routes through [`cosmic_color_from_rgba`] so quantisation
/// stays single-sourced in the font wrapper.
#[inline]
pub(super) fn rgb_to_cosmic_color(rgb: [f32; 3]) -> baumhard::font::Color {
    cosmic_color_from_rgba([rgb[0], rgb[1], rgb[2], 1.0])
}

/// Linear mix of `rgb` toward white by `t` ∈ `[0, 1]`. `t = 0` is the
/// input untouched; `t = 1` is pure white. Shared by the picker's
/// hover / selected highlight mixes so the two differ only in the
/// mix constant — the UI choice — not in the math.
#[inline]
fn mix_toward_white(rgb: [f32; 3], t: f32) -> [f32; 3] {
    let mix = |c: f32| (c + (1.0 - c) * t).clamp(0.0, 1.0);
    [mix(rgb[0]), mix(rgb[1]), mix(rgb[2])]
}

/// Highlight a crosshair-arm cell's color to mark it as "currently
/// selected". The picker used to swap glyphs (■ → ◆) to indicate
/// selection, but with sacred-script glyphs that approach would lose
/// the per-cell script identity. Instead we brighten the cell 60%
/// toward white, which reads as a subtle glow on top of the
/// hue-saturated base color.
#[inline]
pub(super) fn highlight_selected_cell_color(rgb: [f32; 3]) -> baumhard::font::Color {
    rgb_to_cosmic_color(mix_toward_white(rgb, 0.6))
}

/// Highlight a cell under the cursor. Distinct from the selected-
/// cell mix (which marks the HSV-current cell) so the hovered + the
/// already-selected cell can both be visually distinguishable — the
/// hovered one reads "whitest" because of the scale bump AND a
/// deeper mix, while the selected one stays subtly glowing behind
/// the hover cursor. A 40% mix toward white is enough to pop against
/// the hue-saturated background but not so saturated that the glyph
/// character becomes hard to read.
#[inline]
pub(super) fn highlight_hovered_cell_color(rgb: [f32; 3]) -> baumhard::font::Color {
    rgb_to_cosmic_color(mix_toward_white(rgb, 0.4))
}
