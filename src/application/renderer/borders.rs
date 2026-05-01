// SPDX-License-Identifier: MPL-2.0

//! Border-buffer creators and glyph-advance measurement. Every
//! helper returns a [`MindMapTextBuffer`] with
//! [`ZoomVisibility::unbounded`]; scene-builder routes overwrite it
//! to gate on zoom, overlay routes leave it at default.
//!
//! Per CODE_CONVENTIONS §1, styled-region → cosmic-text spans go
//! through `baumhard::font::attrs` — never inlined here. Hex-colour
//! parsing into `cosmic_text::Color` goes through
//! `baumhard::font::hex::hex_to_cosmic_color` (§B5: cosmic-text
//! usage stays inside `font/`).

use cosmic_text::{Attrs, FontSystem};

use baumhard::font::metrics::monospace_advance;
use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

use super::MindMapTextBuffer;

/// Widest shaped advance across `glyphs` at `font_size`, via
/// cosmic-text. Falls back to `monospace_advance(font_size)` if
/// every glyph shapes to zero (tofu + missing fallback).
pub fn measure_max_glyph_advance(
    font_system: &mut cosmic_text::FontSystem,
    glyphs: &[&str],
    font_size: f32,
) -> f32 {
    let mut buffer = cosmic_text::Buffer::new(font_system, cosmic_text::Metrics::new(font_size, font_size));
    let attrs = Attrs::new();
    let mut max_w: f32 = 0.0;
    for g in glyphs {
        buffer.set_text(font_system, g, &attrs, cosmic_text::Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                if glyph.w > max_w {
                    max_w = glyph.w;
                }
            }
        }
    }
    if max_w <= 0.0 {
        monospace_advance(font_size)
    } else {
        max_w
    }
}

pub(super) fn create_border_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    create_border_buffer_lh(font_system, text, attrs, font_size, font_size, pos, bounds)
}

/// Like [`create_border_buffer`] but sets an explicit line-height on
/// the buffer metrics. Needed for multi-line console side columns,
/// where the vertical stack of `│` glyphs has to advance at the
/// content's `row_height` (font_size + 2px breathing room) — not the
/// default `font_size`, which would drift the side column short by
/// 2px per row.
pub(super) fn create_border_buffer_lh(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    line_height: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(font_system, cosmic_text::Metrics::new(font_size, line_height));
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer {
        buffer: buf,
        pos,
        bounds,
        zoom_visibility: ZoomVisibility::unbounded(),
    }
}

/// Multi-span variant of [`create_border_buffer`] — hands cosmic-text
/// a sequence of `(text, attrs)` pairs in one buffer so adjacent
/// spans with different colors (e.g. accent-colored prompt glyph +
/// text-colored input) lay out as one line without the caller having
/// to position them separately.
pub(super) fn create_border_buffer_spans(
    font_system: &mut FontSystem,
    spans: &[(&str, Attrs)],
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(font_system, cosmic_text::Metrics::new(font_size, font_size));
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    let span_refs: Vec<(&str, Attrs)> = spans.iter().map(|(t, a)| (*t, a.clone())).collect();
    buf.set_rich_text(
        font_system,
        span_refs,
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer {
        buffer: buf,
        pos,
        bounds,
        zoom_visibility: ZoomVisibility::unbounded(),
    }
}

/// Like `create_border_buffer` but center-aligns the text within its
/// box via `cosmic_text::Align::Center`. Used for the color picker's
/// crosshair-arm glyphs and hue-ring glyphs: with sacred-script
/// glyphs varying significantly in shaped width (~5 px for Hebrew,
/// ~20 px for Egyptian hieroglyphs at base `font_size`), flush-left
/// positioning would produce a visibly drifting cross and a ring
/// thrown out of round. Center alignment pins each glyph's visual
/// center to the middle of its box, independent of advance width.
pub(super) fn create_centered_cell_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(font_system, cosmic_text::Metrics::new(font_size, font_size));
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        Some(cosmic_text::Align::Center),
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer {
        buffer: buf,
        pos,
        bounds,
        zoom_visibility: ZoomVisibility::unbounded(),
    }
}
