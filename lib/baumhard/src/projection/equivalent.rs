// SPDX-License-Identifier: MPL-2.0

//! The oracle's comparison (#241, Tests): whether two sets of glyph
//! layers draw the same thing, so that layers kept up to date by
//! [`Projector::update`](crate::projection::Projector::update) can be held
//! to a [full projection](crate::projection::Projector::project_full) of
//! the same document.
//!
//! Two sets are equivalent when every layer has the same tree shape and
//! every element the same payload: the same variant, channel and flags,
//! and for a run the same origin, opacity, zoom window and overrides,
//! and the same shape **by value** — a run shaped twice from the same
//! text is never the same `Arc`. Shapes compare by their glyph metrics
//! and glyph ids, placements and positions within
//! [`TOLERANCE`](crate::projection::equivalent::TOLERANCE). Unique ids
//! are ignored.

use glam::Vec2;

use crate::arena::{Arena, NodeId};
use crate::engine::element::TreeElement;
use crate::flow::{ShapedBox, ShapedRun};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::run::{GlyphRunData, RunShape};
use crate::projection::layers::{GlyphLayers, Layer};

/// How far two positions may differ and still be the same placement, in
/// canvas units.
pub const TOLERANCE: f32 = 1e-3;

/// Where two sets of layers first differ: the layer, the pre-order index
/// of the element in it, and what differs.
#[derive(Clone, Debug, PartialEq)]
pub struct Difference {
    /// The layer.
    pub layer: Layer,
    /// The element's index in the layer's pre-order.
    pub index: usize,
    /// What differs.
    pub what: String,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} layer, element {}: {}", self.layer, self.index, self.what)
    }
}

/// Whether `a` and `b` are equivalent (see the module docs): `Ok`, or the
/// first difference found.
///
/// Cost: one pre-order walk of every layer of each, side by side, with no
/// recursion; O(placed glyphs) per run compared.
pub fn layers_equivalent(a: &GlyphLayers, b: &GlyphLayers) -> Result<(), Difference> {
    for layer in Layer::ALL {
        let (ta, tb) = (a.tree(layer), b.tree(layer));
        let fail = |index: usize, what: String| Difference { layer, index, what };
        let mut walk_a = ta.root().descendants(ta.arena());
        let mut walk_b = tb.root().descendants(tb.arena());
        let mut index = 0;
        loop {
            match (walk_a.next(), walk_b.next()) {
                (None, None) => break,
                (Some(_), None) => return Err(fail(index, "the second layer ends first".into())),
                (None, Some(_)) => return Err(fail(index, "the first layer ends first".into())),
                (Some(x), Some(y)) => {
                    if let Some(what) = shape_differs(ta.arena(), x, tb.arena(), y) {
                        return Err(fail(index, what));
                    }
                    if let Some(what) = element_differs(ta.arena()[x].get(), tb.arena()[y].get()) {
                        return Err(fail(index, what));
                    }
                }
            }
            index += 1;
        }
    }
    Ok(())
}

/// Whether the two nodes stand differently in their trees: their child
/// counts, which with the same pre-order is the same tree shape.
fn shape_differs(a: &Arena<GfxElement>, x: NodeId, b: &Arena<GfxElement>, y: NodeId) -> Option<String> {
    let (ca, cb) = (x.children(a).count(), y.children(b).count());
    (ca != cb).then(|| format!("{ca} children against {cb}"))
}

/// What differs between two elements' payloads, unique ids aside.
fn element_differs(a: &GfxElement, b: &GfxElement) -> Option<String> {
    if a.channel() != b.channel() {
        return Some(format!("channel {} against {}", a.channel(), b.channel()));
    }
    match (a, b) {
        (GfxElement::Void { flags: fa, .. }, GfxElement::Void { flags: fb, .. }) => {
            (fa != fb).then(|| "flags".to_string())
        }
        (GfxElement::GlyphRun { run: ra, flags: fa, .. }, GfxElement::GlyphRun { run: rb, flags: fb, .. }) => {
            if fa != fb {
                return Some("flags".into());
            }
            run_differs(ra, rb)
        }
        (GfxElement::GlyphArea { .. }, GfxElement::GlyphArea { .. })
        | (GfxElement::GlyphModel { .. }, GfxElement::GlyphModel { .. }) => {
            // No projection writes these; compared by state, ids aside.
            let mut b = b.clone();
            set_id(&mut b, a.unique_id());
            (!a.same_state(&b)).then(|| "area or model state".into())
        }
        _ => Some("a different kind of element".into()),
    }
}

/// `element` with `id` for its unique id.
fn set_id(element: &mut GfxElement, id: usize) {
    element.set_unique_id(id);
}

/// Whether two positions are the same placement.
fn near(a: Vec2, b: Vec2) -> bool {
    (a - b).abs().max_element() <= TOLERANCE || (a.is_nan() && b.is_nan())
}

/// What differs between two runs.
fn run_differs(a: &GlyphRunData, b: &GlyphRunData) -> Option<String> {
    if !near(a.origin(), b.origin()) {
        return Some(format!("origin {} against {}", a.origin(), b.origin()));
    }
    if a.opacity() != b.opacity() && !(a.opacity().is_nan() && b.opacity().is_nan()) {
        return Some(format!("opacity {} against {}", a.opacity(), b.opacity()));
    }
    if a.zoom() != b.zoom() {
        return Some(format!("zoom {:?} against {:?}", a.zoom(), b.zoom()));
    }
    if a.overrides() != b.overrides() {
        return Some("overrides".into());
    }
    match (a.shaped(), b.shaped()) {
        (RunShape::Box(x), RunShape::Box(y)) => box_differs(x, y),
        (RunShape::Path { run: rx, placed: px }, RunShape::Path { run: ry, placed: py }) => {
            if let Some(what) = shaped_differs(rx, ry) {
                return Some(what);
            }
            if px.truncated != py.truncated {
                return Some("truncation".into());
            }
            if px.glyphs.len() != py.glyphs.len() {
                return Some(format!("{} placed glyphs against {}", px.glyphs.len(), py.glyphs.len()));
            }
            for (i, (g, h)) in px.glyphs.iter().zip(&py.glyphs).enumerate() {
                if g.glyph != h.glyph {
                    return Some(format!("placed glyph {i} is glyph {} against {}", g.glyph, h.glyph));
                }
                if !near(g.pos, h.pos) {
                    return Some(format!("placed glyph {i} at {} against {}", g.pos, h.pos));
                }
            }
            None
        }
        _ => Some("a box run against a path run".into()),
    }
}

/// What differs between two shaped runs: their glyph metrics, line height
/// and laid-out glyphs.
fn shaped_differs(a: &ShapedRun, b: &ShapedRun) -> Option<String> {
    if a.line_height != b.line_height {
        return Some(format!("line height {} against {}", a.line_height, b.line_height));
    }
    if a.glyphs != b.glyphs {
        return Some("glyph metrics".into());
    }
    let ga = a.layout_glyphs().map(|g| (g.glyph_id, g.font_id, g.color_opt));
    let gb = b.layout_glyphs().map(|g| (g.glyph_id, g.font_id, g.color_opt));
    (!ga.eq(gb)).then(|| "laid-out glyphs".into())
}

/// What differs between two shaped boxes: their size, their glyphs' spans
/// and their laid-out glyphs, positions within the tolerance.
fn box_differs(a: &ShapedBox, b: &ShapedBox) -> Option<String> {
    if !near(a.size, b.size) {
        return Some(format!("box size {} against {}", a.size, b.size));
    }
    if a.glyph_spans != b.glyph_spans {
        return Some("box glyph spans".into());
    }
    let glyphs = |s: &ShapedBox| {
        s.buffer
            .layout_runs()
            .flat_map(|run| {
                let top = run.line_top;
                run.glyphs
                    .iter()
                    .map(move |g| (g.glyph_id, g.font_id, g.color_opt, Vec2::new(g.x, g.y + top), g.w))
            })
            .collect::<Vec<_>>()
    };
    let (ga, gb) = (glyphs(a), glyphs(b));
    if ga.len() != gb.len() {
        return Some(format!("{} box glyphs against {}", ga.len(), gb.len()));
    }
    for (i, (x, y)) in ga.iter().zip(&gb).enumerate() {
        if x.0 != y.0 || x.1 != y.1 || x.2 != y.2 || !near(x.3, y.3) || (x.4 - y.4).abs() > TOLERANCE {
            return Some(format!("box glyph {i} differs"));
        }
    }
    None
}
