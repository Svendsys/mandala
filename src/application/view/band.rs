// SPDX-License-Identifier: MPL-2.0

//! The rubber band, drawn on the chrome layer: the chrome's first user
//! (design §12.1). Four box runs — the top and bottom edges, the left
//! and right sides — in dashed box-drawing glyphs, sitting in the chrome
//! from the view's load on, hidden (opacity 0) while no band is out.
//! Moving the band replaces those four runs' payloads through the view's
//! journaled chrome write (`MapView::apply_chrome`); nothing else in the
//! chrome, and nothing in the five glyph layers, is touched. Native,
//! with the rubber band's gesture.

use baumhard::arena::NodeId;
use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{Align, FontSize, Weight};
use baumhard::engine::tree::Placement;
use baumhard::flow::{shape_box, FlowSpan, Rect};
use baumhard::font::fonts::{AppFont, FontSystemLease};
use baumhard::font::metrics::monospace_advance;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::RUN_CHANNEL;
use glam::Vec2;
use std::sync::Arc;

use super::overlay::SELECTION_HIGHLIGHT;

/// The band's corners, in the order top-left, top-right, bottom-left,
/// bottom-right: the console frame's rounded arcs.
pub(crate) const BAND_CORNERS: [char; 4] = ['\u{256D}', '\u{256E}', '\u{2570}', '\u{256F}'];

/// The band's top and bottom edges, one per cell: a light triple dash.
pub(crate) const BAND_HORIZONTAL: char = '\u{2504}'; // ┄

/// The band's sides, one per row: a light triple dash, upright.
pub(crate) const BAND_VERTICAL: &str = "\u{2506}"; // ┆

/// The band's glyph size, in points.
const BAND_SIZE_PT: f32 = 14.0;

/// The alpha the band's cyan is drawn at: below opaque, so what the band
/// sweeps over stays readable underneath it.
const BAND_ALPHA: u8 = 200;

/// The four runs of the band, in the chrome layer: top, bottom, left,
/// right.
pub(super) type BandRuns = [NodeId; 4];

/// Put the band's four runs in `chrome`, hidden, under its root, and
/// return them. Called once, when the chrome is built.
pub(super) fn plant(chrome: &mut GfxTree) -> BandRuns {
    let root = chrome.root();
    let mut fonts = FontSystemLease::new("view::band::plant");
    std::array::from_fn(|_| {
        let run = run_of("", Vec2::ZERO, 0.0, &mut fonts);
        chrome
            .insert_child(root, GfxElement::new_run(run, RUN_CHANNEL, 0), Placement::Last)
            .expect("the chrome's root takes a child")
    })
}

/// The four runs a band over `rect` draws, in [`BandRuns`] order, or
/// the four hidden ones with `None`.
pub(super) fn runs_for(rect: Option<Rect>) -> [GlyphRunData; 4] {
    let mut fonts = FontSystemLease::new("view::band");
    let Some(rect) = rect else {
        return std::array::from_fn(|_| run_of("", Vec2::ZERO, 0.0, &mut fonts));
    };
    let (min, max) = (rect.origin, rect.max());
    let advance = monospace_advance(BAND_SIZE_PT);
    let cells = ((max.x - min.x) / advance).max(1.0) as usize;
    let rows = ((max.y - min.y) / BAND_SIZE_PT).max(1.0) as usize;
    let [top_left, top_right, bottom_left, bottom_right] = BAND_CORNERS;
    let edge: String = std::iter::repeat_n(BAND_HORIZONTAL, cells).collect();
    let top = format!("{top_left}{edge}{top_right}");
    let bottom = format!("{bottom_left}{edge}{bottom_right}");
    let side = vec![BAND_VERTICAL; rows].join("\n");
    [
        run_of(
            &top,
            Vec2::new(min.x - advance, min.y - BAND_SIZE_PT),
            1.0,
            &mut fonts,
        ),
        run_of(&bottom, Vec2::new(min.x - advance, max.y), 1.0, &mut fonts),
        run_of(&side, Vec2::new(min.x - advance, min.y), 1.0, &mut fonts),
        run_of(&side, Vec2::new(max.x, min.y), 1.0, &mut fonts),
    ]
}

/// One box run of `text` at `origin`, in the band's style. The view's
/// tests build the chrome hook's test builder's run with it too.
pub(super) fn run_of(text: &str, origin: Vec2, opacity: f32, fonts: &mut FontSystemLease) -> GlyphRunData {
    let [r, g, b, _] = SELECTION_HIGHLIGHT.0;
    let style = ResolvedText {
        font: AppFont::Any,
        size: FontSize::Pt(BAND_SIZE_PT),
        color: baumhard::document::values::Rgba::new(r, g, b, BAND_ALPHA),
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
