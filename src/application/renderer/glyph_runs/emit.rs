// SPDX-License-Identifier: MPL-2.0

//! From a [`GlyphRunData`] and a camera to the glyphs a frame draws:
//! for each glyph of the run, which rasterization it needs (a
//! cosmic-text [`CacheKey`]), the surface pixel its pen sits on, and
//! its color with the run's opacity folded in.
//!
//! Pure: values in, values out, no device and no font lock, so every
//! decision about *where* and *how* a glyph is drawn is pinned without
//! a GPU (TEST_CONVENTIONS §T8's renderer-free tier). The atlas and the
//! pipeline only look up and draw what this emits.
//!
//! **A path run** (frames, connections, markers): glyph *i* of the
//! placement is the `placed.glyphs[i].glyph`-th item of
//! [`ShapedRun::layout_glyphs`](baumhard::flow::ShapedRun::layout_glyphs),
//! which is collected once per run. Its box's top-left is
//! `origin + placed.pos`, its baseline `run.baseline()` below that. The
//! placement replaces the laid-out glyph's own pen position (`x`, `y`)
//! and line, and keeps its shaper offsets: `x_offset` and `y_offset`,
//! in units of the glyph's font size, `y` subtracted — exactly as
//! cosmic-text's `LayoutGlyph::physical` applies them, which is where
//! mark stacking lives.
//!
//! **A box run** (sections, labels) is its buffer drawn whole at
//! `origin`: each laid-out glyph at its own pen position on its line's
//! baseline, as `LayoutGlyph::physical` places it.
//!
//! **Overrides** are indexed by drawn glyph (placed glyph for a path,
//! layout order for a box) and are empty or exactly as long as the
//! run's glyph count (`GlyphRunData::new` checks it). An offset moves
//! the glyph in canvas units; a scale multiplies its font size about
//! the center of its box, so a glyph grows in place rather than away
//! from its baseline's left end; a color replaces its own.
//! **Opacity** multiplies the alpha of every glyph.
//!
//! **Zoom is applied here, at render time**: the glyph is rasterized at
//! `font size × scale × pixels per unit`, and nothing about the run
//! changes with the camera. A pan or a zoom touches no run.
//!
//! **Nothing is ever rotated** (D9): a sprite is a pen position and a
//! rasterization key, and the key carries no angle.
//!
//! Hostile values degrade, never panic: a run's override values can be
//! authored by a map (`RunOverrides`, D29), so a glyph whose position
//! or scale is not finite, whose scale is not positive, or whose
//! rasterized size would pass [`MAX_RASTER_FONT_PX`] is skipped.

use baumhard::document::values::Rgba;
use baumhard::font::{Color, LayoutGlyph, COLOR_WHITE};
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::run::{GlyphRunData, RunShape};
use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;
use glam::Vec2;
use glyphon::CacheKey;

/// The largest font size, in surface pixels, a glyph is rasterized at.
///
/// A glyph's rasterized size is its font size times its override scale
/// times the camera's pixels per unit. The camera bounds the last
/// (`Camera2D::MAX_ZOOM`), but an override scale comes from the map and
/// has no bound of its own, and the rasterizer allocates the bitmap the
/// size asks for: a scale of `1e6` would ask for terabytes. 2048 px is
/// far past any glyph the app draws (a 72 pt heading at the camera's
/// maximum zoom is 360 px) and its bitmap, a few megabytes at most, is
/// one the atlas can still decline to hold.
pub(crate) const MAX_RASTER_FONT_PX: f32 = 2048.0;

/// One glyph to draw: the rasterization it needs, the surface pixel its
/// pen sits on, and its color.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct GlyphSprite {
    /// What to rasterize: face, glyph, font size in surface pixels,
    /// weight, and the subpixel bins of the pen position.
    pub(crate) key: CacheKey,
    /// The pen's surface pixel, after [`CacheKey::new`] moved its
    /// fractional part into the key's bins. The rasterized bitmap's own
    /// placement (left and top bearing) is applied from here.
    pub(crate) pen: (i32, i32),
    /// sRGB color, opacity folded into alpha. A color glyph (an emoji)
    /// keeps its own colors and takes only the alpha.
    pub(crate) color: [u8; 4],
}

/// What [`emit_run`] did with one run, for the frame's counts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EmitCounts {
    /// Whether the run was drawn at all: inside its zoom window, on
    /// screen and not fully transparent.
    pub(crate) drawn: bool,
    /// Glyphs emitted.
    pub(crate) sprites: usize,
    /// Glyphs skipped: off screen, or a hostile position or scale.
    pub(crate) skipped: usize,
}

/// Emit every glyph of `run` that `camera` shows, in drawing order, to
/// `sink`. `offset` is the registered offset of the tree the run is in,
/// added to its origin as the tree walker adds it to an area's position.
///
/// `layout` is scratch space the caller keeps across runs so a frame
/// allocates it once: a path run collects its laid-out glyphs into it
/// once, and glyph *i* of the placement is then an index, not a walk.
///
/// Cost: O(glyphs of the run) — the layout collected once, one
/// [`CacheKey::new`] per glyph; no allocation once `layout` has grown
/// to the largest run's length.
pub(crate) fn emit_run<'a>(
    run: &'a GlyphRunData,
    offset: Vec2,
    camera: &Camera2D,
    layout: &mut Vec<&'a LayoutGlyph>,
    sink: &mut impl FnMut(GlyphSprite),
) -> EmitCounts {
    let mut counts = EmitCounts::default();
    let opacity = run.opacity();
    // `!(x > 0)` rather than `x <= 0` so a NaN opacity draws nothing.
    if !(opacity > 0.0) {
        return counts;
    }
    let opacity = opacity.min(1.0);
    let (top_left, size) = run.bounds();
    let top_left = top_left + offset;
    let window = run.zoom().map_or(ZoomVisibility::unbounded(), |w| {
        ZoomVisibility::from_pair(w.min, w.max)
    });
    if !camera.visibility(top_left, size, window).is_visible() {
        return counts;
    }
    counts.drawn = true;
    let overrides = run.overrides();
    let style = GlyphStyle {
        camera,
        opacity,
        offsets: overrides.offset(),
        colors: overrides.color(),
        scales: overrides.scale(),
    };
    let origin = run.origin() + offset;
    match run.shaped() {
        RunShape::Path { run: shaped, placed } => {
            layout.clear();
            layout.extend(shaped.layout_glyphs());
            let baseline = shaped.baseline();
            for (index, glyph) in placed.glyphs.iter().enumerate() {
                let (Some(laid), Some(metrics)) = (
                    layout.get(glyph.glyph as usize),
                    shaped.glyphs.get(glyph.glyph as usize),
                ) else {
                    counts.skipped += 1;
                    continue;
                };
                let corner = origin + glyph.pos;
                let at = GlyphAt {
                    laid,
                    pen: corner + Vec2::new(0.0, baseline),
                    center: corner + Vec2::new(metrics.advance, shaped.line_height) * 0.5,
                    index,
                };
                style.emit(at, &mut counts, sink);
            }
        }
        RunShape::Box(shaped) => {
            let mut index = 0;
            for line in shaped.buffer.layout_runs() {
                for laid in line.glyphs {
                    let pen = origin + Vec2::new(laid.x, line.line_y + laid.y);
                    let center =
                        origin + Vec2::new(laid.x + laid.w * 0.5, line.line_top + line.line_height * 0.5);
                    style.emit(
                        GlyphAt {
                            laid,
                            pen,
                            center,
                            index,
                        },
                        &mut counts,
                        sink,
                    );
                    index += 1;
                }
            }
        }
    }
    counts
}

/// One drawn glyph before its overrides: the laid-out glyph, its pen
/// on the canvas, the center of its box (what a scale override scales
/// about) and its index among the run's drawn glyphs.
struct GlyphAt<'g> {
    laid: &'g LayoutGlyph,
    pen: Vec2,
    center: Vec2,
    index: usize,
}

/// What every glyph of one run is drawn through: the camera, the run's
/// opacity and its override vectors (each empty, or one entry per
/// drawn glyph).
struct GlyphStyle<'r> {
    camera: &'r Camera2D,
    opacity: f32,
    offsets: &'r [Vec2],
    colors: &'r [Option<Rgba>],
    scales: &'r [f32],
}

impl GlyphStyle<'_> {
    /// Apply the overrides to one glyph, project it through the camera
    /// and emit it — or count it skipped.
    fn emit(&self, at: GlyphAt<'_>, counts: &mut EmitCounts, sink: &mut impl FnMut(GlyphSprite)) {
        let offset = self.offsets.get(at.index).copied().unwrap_or(Vec2::ZERO);
        let scale = self.scales.get(at.index).copied().unwrap_or(1.0);
        // `!(x > 0)` rather than `x <= 0` so a NaN scale is skipped too.
        if !(scale > 0.0) || !scale.is_finite() || !offset.is_finite() {
            counts.skipped += 1;
            return;
        }
        let laid = at.laid;
        let font_size = laid.font_size * scale;
        let font_px = font_size * self.camera.pixels_per_unit();
        if !(font_px > 0.0) || font_px > MAX_RASTER_FONT_PX {
            counts.skipped += 1;
            return;
        }
        // Scaling about the box's center: the pen moves away from the
        // center by the same factor the glyph grows by.
        let pen = at.center + (at.pen - at.center) * scale + offset;
        // The shaper's offsets, in units of the font size, `y`
        // subtracted: `LayoutGlyph::physical`'s arithmetic.
        let canvas = pen + Vec2::new(font_size * laid.x_offset, -font_size * laid.y_offset);
        let screen = self.camera.canvas_to_screen(canvas);
        if !screen.is_finite() || self.off_screen(screen, font_px) {
            counts.skipped += 1;
            return;
        }
        // The baseline lands on a whole pixel, as `physical`'s
        // truncation lands it (vertical hinting); `floor` rather than a
        // cast so a pen above the surface rounds the same way.
        let (key, x, y) = CacheKey::new(
            laid.font_id,
            laid.glyph_id,
            font_px,
            (screen.x, screen.y.floor()),
            laid.font_weight,
            laid.cache_key_flags,
        );
        let color = match self.colors.get(at.index).copied().flatten() {
            Some(Rgba(rgba)) => rgba,
            None => color_bytes(laid.color_opt.unwrap_or(COLOR_WHITE)),
        };
        let alpha = (f32::from(color[3]) * self.opacity).round() as u8;
        sink(GlyphSprite {
            key,
            pen: (x, y),
            color: [color[0], color[1], color[2], alpha],
        });
        counts.sprites += 1;
    }

    /// Whether a glyph whose pen is at `screen` cannot reach the
    /// surface: its ink stays within two ems of the pen for any face
    /// the app ships, so a pen farther than that past an edge draws
    /// nothing there. Culling here keeps such a glyph out of the atlas
    /// as well as out of the frame.
    fn off_screen(&self, screen: Vec2, font_px: f32) -> bool {
        let margin = 2.0 * font_px;
        let (w, h) = self.camera.viewport_size;
        screen.x < -margin
            || screen.y < -margin
            || screen.x > w as f32 + margin
            || screen.y > h as f32 + margin
    }
}

/// A cosmic-text color as `[r, g, b, a]` bytes.
fn color_bytes(color: Color) -> [u8; 4] {
    [color.r(), color.g(), color.b(), color.a()]
}
