// SPDX-License-Identifier: MPL-2.0

//! A rectangle outlined in box-drawing glyphs, as four box runs — the
//! top and bottom edges, the left and right sides — in the selection's
//! cyan: what the rubber band draws (`view::band`, native) and what
//! NodeEdit's section frames draw ([`super::frames`]), each in its own
//! glyphs.

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

/// The four runs an outline of `rect` in `style` draws — top, bottom,
/// left, right — just outside it, or four hidden ones with `None`.
///
/// Cost: four box shapings, under `fonts`.
pub(crate) fn runs_for(
    rect: Option<Rect>,
    style: &OutlineStyle,
    fonts: &mut FontSystemLease,
) -> [GlyphRunData; 4] {
    let Some(rect) = rect else {
        return std::array::from_fn(|_| run_of("", Vec2::ZERO, 0.0, style.alpha, fonts));
    };
    let (min, max) = (rect.origin, rect.max());
    let advance = monospace_advance(OUTLINE_SIZE_PT);
    let cells = ((max.x - min.x) / advance).max(1.0) as usize;
    let rows = ((max.y - min.y) / OUTLINE_SIZE_PT).max(1.0) as usize;
    let [top_left, top_right, bottom_left, bottom_right] = style.corners;
    let edge: String = std::iter::repeat_n(style.horizontal, cells).collect();
    let top = format!("{top_left}{edge}{top_right}");
    let bottom = format!("{bottom_left}{edge}{bottom_right}");
    let side = vec![style.vertical; rows].join("\n");
    let alpha = style.alpha;
    [
        run_of(
            &top,
            Vec2::new(min.x - advance, min.y - OUTLINE_SIZE_PT),
            1.0,
            alpha,
            fonts,
        ),
        run_of(&bottom, Vec2::new(min.x - advance, max.y), 1.0, alpha, fonts),
        run_of(&side, Vec2::new(min.x - advance, min.y), 1.0, alpha, fonts),
        run_of(&side, Vec2::new(max.x, min.y), 1.0, alpha, fonts),
    ]
}

/// One box run of `text` at `origin`, in the selection's cyan at `alpha`.
pub(crate) fn run_of(
    text: &str,
    origin: Vec2,
    opacity: f32,
    alpha: u8,
    fonts: &mut FontSystemLease,
) -> GlyphRunData {
    let [r, g, b, _] = SELECTION_HIGHLIGHT.0;
    let style = ResolvedText {
        font: baumhard::document::values::FontRef::DEFAULT,
        size: FontSize::Pt(OUTLINE_SIZE_PT),
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
