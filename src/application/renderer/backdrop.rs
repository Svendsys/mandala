// SPDX-License-Identifier: MPL-2.0

//! The opaque backdrops behind the console and the glyph-wheel color
//! picker, drawn as glyphs: a grid of full blocks (`█`, U+2588) in the
//! backdrop color, at the bottom of the console's own glyphon pass, under
//! the modal's text (V1-B12; the owner's ruling on #298). With them the
//! app draws nothing but the frame's clear and glyphs, so it has no
//! other pipeline.
//!
//! **Why the cells cover every pixel exactly once, opaque.** The block
//! is Public Pixel's, the one bundled face whose `█` is a real block:
//! its outline is the whole em square, `(0, 0)–(1024, 1024)` on a
//! 1024-unit em, with an advance of one em and an ascent of one em over a
//! descent of zero in all three of its metric tables. Shaped at
//! [`CELL_PX`] — an integer — with a line height of [`CELL_PX`], its ink
//! is exactly the cell: `CELL_PX × CELL_PX` pixels, every one fully
//! covered, with the top-left at the text area's top-left. A text area
//! placed at whole target pixels puts it there (glyphon rounds a line's
//! baseline to a whole pixel, and a whole-pixel pen position leaves the
//! glyph at its first subpixel bin), so a grid of cells on the pixel grid
//! abuts without seams: no pixel is shared between two partly covered
//! edges, which would let the canvas through where they meet. That is
//! also why the cells are laid out here, in the target's own pixels,
//! rather than in the overlay tree's window pixels: a capture at a
//! fractional `pixel_ratio` would put them between pixels.
//! `test_a_backdrop_cell_is_one_opaque_square_at_its_corner` pins the
//! cell against glyphon's own placement arithmetic.
//!
//! **Why the extent is the old rectangle's, to the pixel.** A backdrop's
//! window-pixel rectangle covers, in a target at `pixel_ratio`, the
//! pixels whose centers it holds — the rule a GPU rasterizes a
//! rectangle by, so the pixels the rect pipeline filled
//! ([`PixelRect::covered_by`]). Every cell is clipped to those pixels by
//! its text area's bounds, which glyphon applies in whole pixels, so the
//! grid's last row and column end exactly at the rectangle's edge.
//!
//! The one shaped cell ([`BackdropCell`]) is drawn by every text area of
//! every backdrop, so a backdrop costs one text area per cell and no
//! shaping: O(cells) per frame, `⌈w / 64⌉ × ⌈h / 64⌉` in target pixels.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::attrs::{rich_text_spans_into, RegionFamilies};
use baumhard::font::fonts::AppFont;
use baumhard::font::{buffer, Attrs, Buffer, FontSystem, COLOR_BLACK, SHAPING_ADVANCED};
use glyphon::{TextArea, TextBounds};

/// The glyph a backdrop is made of: the full block.
pub(super) const FILL: &str = "\u{2588}";

/// The side of one cell, in target pixels: the block's font size and
/// line height. A whole number, so the block's edges fall on pixel
/// edges ([module docs](self)).
pub(super) const CELL_PX: i32 = 64;

/// The one cell every backdrop draws: a single `█` of Public Pixel at
/// [`CELL_PX`], opaque black — the color both backdrops have always
/// been, against which the console's frame and the picker's hue ring
/// read over any canvas.
pub(super) struct BackdropCell {
    buffer: Buffer,
}

impl BackdropCell {
    /// Shape the cell, under the caller's font-system guard. Once per
    /// renderer: it never changes, whatever the backdrops' extents or
    /// the target's ratio.
    pub(super) fn shape(font_system: &mut FontSystem) -> Self {
        BackdropCell {
            buffer: shape_block(font_system, CELL_PX as f32, Some(AppFont::PublicPixel)),
        }
    }

    /// The shaped cell, for the tests that pin its geometry.
    #[cfg(test)]
    pub(super) fn buffer(&self) -> &Buffer {
        &self.buffer
    }
}

/// One [`FILL`] in `font` (the shaper's default face for `None`), opaque
/// black, at font size and line height `size`. The face is pinned the
/// way every glyph area pins one, through `baumhard::font::attrs`, which
/// resolves it from the guard it is handed (CODE_CONVENTIONS §1).
pub(super) fn shape_block(font_system: &mut FontSystem, size: f32, font: Option<AppFont>) -> Buffer {
    let regions = ColorFontRegions::single_span(1, Some([0.0, 0.0, 0.0, 1.0]), font);
    let families = RegionFamilies::resolve(FILL, &regions, font_system);
    let mut spans = Vec::new();
    rich_text_spans_into(&families, size, size, None, &mut spans);
    let mut buffer = buffer::create(font_system, size, size);
    buffer.set_size(Some(size), Some(size));
    buffer.set_rich_text(spans, &Attrs::new(), SHAPING_ADVANCED, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

/// A rectangle of a target's whole pixels: `left..right` by
/// `top..bottom`, the right and bottom edges excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PixelRect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

impl PixelRect {
    /// The pixels of a target at `ratio` target pixels per window pixel
    /// that the window-pixel rectangle `(left, top, width, height)`
    /// covers: those whose centers it holds, its left and top edges in
    /// and its right and bottom edges out — the rule a GPU rasterizes a
    /// rectangle by, so exactly the pixels the rect pipeline filled.
    /// Pixel `i` is covered when `x0 ≤ i + 0.5 < x1`, that is for
    /// `⌈x0 − 0.5⌉ ≤ i < ⌈x1 − 0.5⌉`. O(1).
    pub(super) fn covered_by((left, top, width, height): (f32, f32, f32, f32), ratio: f32) -> Self {
        let edge = |at: f32| (f64::from(at) - 0.5).ceil() as i32;
        PixelRect {
            left: edge(left * ratio),
            top: edge(top * ratio),
            right: edge((left + width) * ratio),
            bottom: edge((top + height) * ratio),
        }
    }

    /// Whether the rectangle holds no pixel.
    pub(super) fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// How many cells [`Self::cells`] yields: `⌈w / CELL_PX⌉ × ⌈h /
    /// CELL_PX⌉`, or none for an empty rectangle. O(1).
    pub(super) fn cell_count(&self) -> usize {
        if self.is_empty() {
            return 0;
        }
        let across = |from: i32, to: i32| (to - from).unsigned_abs().div_ceil(CELL_PX as u32) as usize;
        across(self.left, self.right) * across(self.top, self.bottom)
    }

    /// The top-left corner of every cell that covers the rectangle, row
    /// by row: a grid of [`CELL_PX`] from its own top-left corner, as many
    /// as reach its right and bottom edges. The last row and column
    /// overhang it, and are clipped to it by their bounds
    /// ([`text_areas`]). Nothing for an empty rectangle.
    pub(super) fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        let PixelRect {
            left,
            top,
            right,
            bottom,
        } = self;
        let step = CELL_PX as usize;
        (top..bottom)
            .step_by(step)
            .flat_map(move |y| (left..right).step_by(step).map(move |x| (x, y)))
    }
}

/// The text areas that draw the backdrop `rect` from `cell`: one per
/// cell of [`PixelRect::cells`], at its corner in target pixels, at
/// scale `1` (the cell is shaped in target pixels), clipped to `rect`.
pub(super) fn text_areas(cell: &BackdropCell, rect: PixelRect) -> impl Iterator<Item = TextArea<'_>> {
    let bounds = TextBounds {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    rect.cells().map(move |(x, y)| TextArea {
        buffer: &cell.buffer,
        left: x as f32,
        top: y as f32,
        scale: 1.0,
        bounds,
        default_color: COLOR_BLACK,
        custom_glyphs: &[],
    })
}

/// How many fill blocks the last prepared frame handed each half of
/// the frame: the canvas (the main glyphon pass and the glyph-run pass)
/// and the screen-space chrome (the console's pass: the modal, the FPS
/// readout, the mode-status line, and the backdrops at the bottom of
/// it). A fill block is the backdrop's own glyph, Public Pixel's `█`
/// ([`FillGlyph`]) — the one glyph of a bundled face that is a solid
/// block, so the one a glyph fill can be made of. Under V1-B12 a map
/// draws no fill, so a frame's `canvas` count is what a content fill
/// would come back as, through whichever glyph pass it took. Kept by
/// the test build only; read by the GPU-backed tests
/// (`Renderer::fill_census_of_last_frame`).
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FillCensus {
    /// Fill blocks the canvas passes were handed to draw.
    pub(crate) canvas: usize,
    /// Fill blocks the chrome pass was handed to draw.
    pub(crate) chrome: usize,
}

/// A glyph's identity: its face and its index in the face.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FillGlyph {
    font_id: glyphon::fontdb::ID,
    glyph_id: u16,
}

#[cfg(test)]
impl FillGlyph {
    /// The glyph `cell` draws: its one laid-out glyph.
    pub(super) fn of(cell: &BackdropCell) -> Option<Self> {
        let run = cell.buffer.layout_runs().next()?;
        let glyph = run.glyphs.first()?;
        Some(FillGlyph {
            font_id: glyph.font_id,
            glyph_id: glyph.glyph_id,
        })
    }

    fn is(self, glyph: &baumhard::font::LayoutGlyph) -> bool {
        glyph.font_id == self.font_id && glyph.glyph_id == self.glyph_id
    }

    /// The fill glyphs `buffer` lays out: what glyphon draws of it.
    /// O(glyphs).
    pub(super) fn in_buffer(self, buffer: &Buffer) -> usize {
        buffer
            .layout_runs()
            .map(|run| run.glyphs.iter().filter(|glyph| self.is(glyph)).count())
            .sum()
    }

    /// The fill glyphs the glyph-run pass draws of `run`: each laid-out
    /// glyph of a box run, and each placement of a path run — a filler
    /// places one shaped glyph many times — as `glyph_runs::emit` reads
    /// them. O(glyphs).
    pub(super) fn in_run(self, run: &baumhard::gfx_structs::run::GlyphRunData) -> usize {
        use baumhard::gfx_structs::run::RunShape;
        match run.shaped() {
            RunShape::Box(shaped) => self.in_buffer(&shaped.buffer),
            RunShape::Path { run: shaped, placed } => {
                let laid: Vec<&baumhard::font::LayoutGlyph> = shaped.layout_glyphs().collect();
                placed
                    .glyphs
                    .iter()
                    .filter(|glyph| laid.get(glyph.glyph as usize).is_some_and(|laid| self.is(laid)))
                    .count()
            }
        }
    }
}
