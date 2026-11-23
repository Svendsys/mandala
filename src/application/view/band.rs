// SPDX-License-Identifier: MPL-2.0

//! The rubber band, drawn on the chrome layer: the chrome's first user
//! (design §12.1). Four box runs — the top and bottom edges, the left
//! and right sides — in dashed box-drawing glyphs ([`super::outline`]),
//! sitting in the chrome from the view's load on, hidden (opacity 0)
//! while no band is out. Moving the band replaces those four runs'
//! payloads through the view's journaled chrome write
//! (`MapView::apply_chrome`); nothing else in the chrome, and nothing in
//! the five glyph layers, is touched. Native, with the rubber band's
//! gesture.

use baumhard::arena::NodeId;
use baumhard::engine::tree::Placement;
use baumhard::flow::Rect;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::RUN_CHANNEL;
use glam::Vec2;

use super::outline::{self, OutlineStyle};

/// The band's corners, in the order top-left, top-right, bottom-left,
/// bottom-right: the console frame's rounded arcs.
pub(crate) const BAND_CORNERS: [char; 4] = ['\u{256D}', '\u{256E}', '\u{2570}', '\u{256F}'];

/// The band's top and bottom edges, one per cell: a light triple dash.
pub(crate) const BAND_HORIZONTAL: char = '\u{2504}'; // ┄

/// The band's sides, one per row: a light triple dash, upright.
pub(crate) const BAND_VERTICAL: &str = "\u{2506}"; // ┆

/// The alpha the band's cyan is drawn at: below opaque, so what the band
/// sweeps over stays readable underneath it.
const BAND_ALPHA: u8 = 200;

/// The band's outline.
const BAND: OutlineStyle = OutlineStyle {
    corners: BAND_CORNERS,
    horizontal: BAND_HORIZONTAL,
    vertical: BAND_VERTICAL,
    alpha: BAND_ALPHA,
};

/// The four runs of the band, in the chrome layer: top, bottom, left,
/// right.
pub(super) type BandRuns = [NodeId; 4];

/// Put the band's four runs in `chrome`, hidden, under its root, and
/// return them. Called once, when the chrome is built.
pub(super) fn plant(chrome: &mut GfxTree) -> BandRuns {
    let root = chrome.root();
    let mut fonts = FontSystemLease::new("view::band::plant");
    std::array::from_fn(|_| {
        let run = outline::run_of("", Vec2::ZERO, 0.0, BAND_ALPHA, &mut fonts);
        chrome
            .insert_child(root, GfxElement::new_run(run, RUN_CHANNEL, 0), Placement::Last)
            .expect("the chrome's root takes a child")
    })
}

/// The four runs a band over `rect` draws, in [`BandRuns`] order, or
/// the four hidden ones with `None`.
pub(super) fn runs_for(rect: Option<Rect>) -> [GlyphRunData; 4] {
    let mut fonts = FontSystemLease::new("view::band");
    outline::runs_for(rect, &BAND, &mut fonts)
}
