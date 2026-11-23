// SPDX-License-Identifier: MPL-2.0

//! A rectangle outlined in box-drawing glyphs, as four box runs — the
//! top and bottom edges, the left and right sides — in the selection's
//! cyan: what the rubber band draws (`view::band`, native) and what
//! NodeEdit's section frames draw ([`super::frames`]), each in its own
//! glyphs.
//!
//! **Placed by its strokes, not its cells** ([`StrokeModel`],
//! [`Strokes`]): where each glyph draws its line in its cell is read
//! from the glyph, so a stroke goes exactly where it is placed. The
//! edges hold a whole number of cells and the sides a whole number of
//! rows, each run laid at the size that makes them span exactly from
//! stroke to stroke — rows counted by the run's line height, not its
//! point size, which left a side a fifth longer than its edges — so every
//! corner meets its edge and its side.

use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{Align, FontSize, Weight};
use baumhard::flow::{shape_box, FlowSpan, Rect};
use baumhard::font::fonts::FontSystemLease;
use baumhard::font::metrics::monospace_advance;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use glam::Vec2;
use std::sync::Arc;

use super::overlay::SELECTION_HIGHLIGHT;

/// The glyph size outlines are drawn at, in points.
pub(crate) const OUTLINE_SIZE_PT: f32 = 14.0;

/// The glyphs and the alpha one outline is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutlineStyle {
    /// Its corners: top-left, top-right, bottom-left, bottom-right.
    pub(crate) corners: [char; 4],
    /// Its top and bottom edges, one per cell.
    pub(crate) horizontal: char,
    /// Its sides, one per row.
    pub(crate) vertical: &'static str,
    /// The alpha its cyan is drawn at.
    pub(crate) alpha: u8,
}

/// Where an outline's glyphs draw their strokes in their cells, read
/// from the glyphs themselves as a box run shapes them: what lets the
/// strokes be placed, rather than the cells, and the corners meet the
/// edges and the sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StrokeModel {
    /// One cell's width: the horizontal glyph's advance.
    pub(crate) advance: f32,
    /// One row's height: the run's line height, which a side's rows are
    /// laid at — not the point size.
    pub(crate) pitch: f32,
    /// The horizontal stroke's center below its run's top.
    pub(crate) h_center: f32,
    /// Half the horizontal stroke's thickness.
    pub(crate) h_half: f32,
    /// The vertical stroke's center right of its run's left edge.
    pub(crate) v_center: f32,
    /// Half the vertical stroke's thickness.
    pub(crate) v_half: f32,
}

impl StrokeModel {
    /// Measure `style`: one box run of its horizontal glyph and one of
    /// its vertical glyph, shaped as the outline's runs are, and each
    /// glyph's ink as the renderer draws it
    /// (`baumhard::font::fonts::glyph_outline_ink`). A glyph whose ink
    /// is not read falls back to its cell's middle and a hairline.
    ///
    /// Cost: two box shapings and two outline-cache lookups, under
    /// `fonts`; done once per style and view, never per frame.
    pub(crate) fn of(style: &OutlineStyle, fonts: &mut FontSystemLease) -> StrokeModel {
        let mut swash = glyphon::SwashCache::new();
        let mut measure = |text: &str| {
            let run = run_of(text, Vec2::ZERO, 1.0, style.alpha, fonts);
            let RunShape::Box(shaped) = run.shaped() else { unreachable!("an outline run is a box run") };
            let shaped = Arc::clone(shaped);
            let line = shaped.buffer.layout_runs().next().map(|line| {
                let glyph = line.glyphs.first().cloned();
                (line.line_top, line.line_y, line.line_height, glyph)
            });
            let (top, baseline, height, glyph) =
                line.unwrap_or((0.0, OUTLINE_SIZE_PT, OUTLINE_SIZE_PT, None));
            let ink = glyph
                .as_ref()
                .and_then(|glyph| baumhard::font::fonts::glyph_outline_ink(fonts.fonts(), &mut swash, glyph));
            let (x, advance) = glyph
                .as_ref()
                .map_or((0.0, monospace_advance(OUTLINE_SIZE_PT)), |g| (g.x, g.w));
            (baseline - top, height, x, advance, ink)
        };
        let (baseline, pitch, _, advance, ink) = measure(&style.horizontal.to_string());
        let (h_center, h_half) = ink.map_or((pitch * 0.5, 0.5), |(min, max)| {
            (baseline + (min.y + max.y) * 0.5, ((max.y - min.y) * 0.5).max(0.5))
        });
        let advance = if advance > 0.0 {
            advance
        } else {
            monospace_advance(OUTLINE_SIZE_PT)
        };
        let (_, _, vx, v_advance, v_ink) = measure(style.vertical);
        let (v_center, v_half) = v_ink.map_or((v_advance * 0.5, 0.5), |(min, max)| {
            (vx + (min.x + max.x) * 0.5, ((max.x - min.x) * 0.5).max(0.5))
        });
        StrokeModel {
            advance,
            pitch,
            h_center,
            h_half,
            v_center,
            v_half,
        }
    }

    /// The outward distances, top, bottom, left and right, the outline
    /// stood at before its strokes were placed: its top edge's run a
    /// point size above the rectangle, its left side's a cell left of
    /// it, its bottom edge's and right side's on its far edges. What the
    /// rubber band keeps, and the most a section's frame stands out.
    pub(crate) fn standing(&self) -> [f32; 4] {
        [
            OUTLINE_SIZE_PT - self.h_center,
            self.h_center,
            self.advance - self.v_center,
            self.v_center,
        ]
    }
}

/// Where an outline's four strokes run: the center lines of its top and
/// bottom edges and of its left and right sides. [`runs_for`] lays its
/// runs so the corner glyphs sit where the strokes meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Strokes {
    /// The top edge's center line.
    pub(crate) top: f32,
    /// The bottom edge's center line.
    pub(crate) bottom: f32,
    /// The left side's center line.
    pub(crate) left: f32,
    /// The right side's center line.
    pub(crate) right: f32,
}

impl Strokes {
    /// The strokes round `rect` at the outward distances `out` — top,
    /// bottom, left, right. What the rubber band draws. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the rubber band draws with it, which is native")
    )]
    pub(crate) fn around(rect: Rect, out: [f32; 4]) -> Strokes {
        let (min, max) = (rect.origin, rect.max());
        let [top_out, bottom_out, left_out, right_out] = out;
        Strokes {
            top: min.y - top_out,
            bottom: max.y + bottom_out,
            left: min.x - left_out,
            right: max.x + right_out,
        }
    }

    /// The four strokes as the boxes their ink covers — top, bottom,
    /// left, right — each running between the corners' centers. O(1).
    pub(crate) fn segments(&self, model: &StrokeModel) -> [Rect; 4] {
        let horizontal = |y: f32| {
            Rect::new(
                Vec2::new(self.left, y - model.h_half),
                Vec2::new(self.right - self.left, 2.0 * model.h_half),
            )
        };
        let vertical = |x: f32| {
            Rect::new(
                Vec2::new(x - model.v_half, self.top),
                Vec2::new(2.0 * model.v_half, self.bottom - self.top),
            )
        };
        [
            horizontal(self.top),
            horizontal(self.bottom),
            vertical(self.left),
            vertical(self.right),
        ]
    }
}

/// The four runs an outline on `strokes` in `style` draws — top, bottom,
/// left, right — each hidden where `shown` says so, or four hidden ones
/// with `None`. `reach` is how far down the left and the right side run,
/// from and to (canvas `y`): a side runs from its top corner's cell to its
/// bottom corner's, or less of it, where its reach says so. The edges hold the corners and a whole number of cells
/// between them, laid at the size — no larger than `model`'s — at which
/// the corners' strokes fall on the left and the right strokes; each side
/// holds a whole number of rows, laid at the size at which they run from
/// the top corner's cell to the bottom corner's, so no side runs past a
/// corner and none falls short of it.
///
/// Cost: four box shapings, under `fonts`.
pub(crate) fn runs_for(
    strokes: Option<Strokes>,
    reach: [(f32, f32); 2],
    shown: [bool; 4],
    model: &StrokeModel,
    style: &OutlineStyle,
    fonts: &mut FontSystemLease,
) -> [GlyphRunData; 4] {
    let Some(strokes) = strokes else {
        return std::array::from_fn(|_| run_of("", Vec2::ZERO, 0.0, style.alpha, fonts));
    };
    let alpha = style.alpha;
    let opacity = |side: usize| if shown[side] { 1.0 } else { 0.0 };
    // A whole number of steps across `span`, at a scale of the model no
    // larger than one: (how many, the scale).
    let fit = |span: f32, step: f32| {
        let count = ((span / step) - 1e-3).ceil().max(1.0);
        (count, (span / (count * step)).max(0.0))
    };
    let (cells, fx) = fit(strokes.right - strokes.left, model.advance);
    let [top_left, top_right, bottom_left, bottom_right] = style.corners;
    let edge: String = std::iter::repeat_n(style.horizontal, cells as usize - 1).collect();
    let top = format!("{top_left}{edge}{top_right}");
    let bottom = format!("{bottom_left}{edge}{bottom_right}");
    let x0 = strokes.left - model.v_center * fx;
    let edge_size = OUTLINE_SIZE_PT * fx;
    // The sides run from the bottom of the top corner's cell to the top
    // of the bottom corner's.
    let side_top = strokes.top + (model.pitch - model.h_center) * fx;
    let side_bottom = strokes.bottom - model.h_center * fx;
    // One side over what its reach leaves of the run between the corners:
    // a whole number of rows laid at the size that spans it exactly.
    let side = |x: f32, (from, to): (f32, f32), opacity: f32, fonts: &mut FontSystemLease| {
        let (from, to) = (from.max(side_top), to.min(side_bottom));
        let length = to - from;
        let (rows, fy) = fit(length.max(0.0), model.pitch);
        let text = if length > MIN_SIDE {
            vec![style.vertical; rows as usize].join("\n")
        } else {
            String::new()
        };
        run_sized(
            &text,
            Vec2::new(x - model.v_center * fy, from),
            OUTLINE_SIZE_PT * fy,
            opacity,
            alpha,
            fonts,
        )
    };
    let left = side(strokes.left, reach[0], opacity(2), fonts);
    let right = side(strokes.right, reach[1], opacity(3), fonts);
    [
        run_sized(
            &top,
            Vec2::new(x0, strokes.top - model.h_center * fx),
            edge_size,
            opacity(0),
            alpha,
            fonts,
        ),
        run_sized(
            &bottom,
            Vec2::new(x0, side_bottom),
            edge_size,
            opacity(1),
            alpha,
            fonts,
        ),
        left,
        right,
    ]
}

/// The shortest side an outline draws, in canvas units: one row at the
/// smallest size a run is laid at
/// (`baumhard::font::fonts::MIN_FONT_SIZE_PT` by
/// `baumhard::flow::LINE_HEIGHT_FACTOR`); a shorter gap between two
/// corners is too small to see.
pub(crate) const MIN_SIDE: f32 = baumhard::font::fonts::MIN_FONT_SIZE_PT * baumhard::flow::LINE_HEIGHT_FACTOR;

/// One box run of `text` at `origin`, in the selection's cyan at `alpha`.
pub(crate) fn run_of(
    text: &str,
    origin: Vec2,
    opacity: f32,
    alpha: u8,
    fonts: &mut FontSystemLease,
) -> GlyphRunData {
    run_sized(text, origin, OUTLINE_SIZE_PT, opacity, alpha, fonts)
}

/// [`run_of`] at `size` points.
fn run_sized(
    text: &str,
    origin: Vec2,
    size: f32,
    opacity: f32,
    alpha: u8,
    fonts: &mut FontSystemLease,
) -> GlyphRunData {
    let [r, g, b, _] = SELECTION_HIGHLIGHT.0;
    let style = ResolvedText {
        font: baumhard::document::values::FontRef::DEFAULT,
        size: FontSize::Pt(baumhard::document::bounds::clamp_font_size_pt(size)),
        color: baumhard::document::values::Rgba::new(r, g, b, alpha),
        weight: Weight::NORMAL,
        italic: false,
    };
    let spans = [FlowSpan {
        text,
        style: &style,
        link: None,
    }];
    let shaped = shape_box(&spans, None, Align::Left, fonts);
    GlyphRunData::new(
        RunShape::Box(Arc::new(shaped)),
        origin,
        GlyphOverrides::none(),
        opacity,
        None,
    )
    .expect("a run with no overrides is always built")
}
