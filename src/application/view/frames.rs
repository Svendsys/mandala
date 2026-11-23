// SPDX-License-Identifier: MPL-2.0

//! **NodeEdit's section frames** (V1-C8, #249; design §12.5): while
//! NodeEdit is active on a node of more than one section, each of its
//! sections is outlined in the chrome layer ([`super::outline`]) — thin,
//! and heavy for the focused one, the section being edited or else the
//! one selected — as `main` outlined them: the cue that tells the user
//! what there is to pick.
//!
//! **A pool of slots, grown in place.** The frames' group is planted
//! empty when the view is built; a node with more sections than the pool
//! has slots grows it by structural inserts under the group, journaled
//! like every chrome write (`MapView::sync`), and a slot it no longer
//! needs is hidden, never removed. Each slot is four runs, replaced
//! (`RunSet`) only when what it outlines moves.
//!
//! **Updated only when what it is built from changes**: the active node
//! or the focused section, or a sync pass that may have moved the node's
//! geometry, its sections' or its rings' (`Projector::geometry_moved`). A
//! pan or a zoom changes none of them and writes nothing.
//!
//! **Inside the rings' inks** (ruling on #249): a frame never crosses the
//! text of its node's rings. Each side's stroke goes halfway between the
//! section's box and the nearest ring ink beyond that side, never further
//! out than the outline stood before ([`StrokeModel::standing`]); with no
//! room it hugs the box, going inside it if it must but never onto the
//! section's own letters ([`place`]). Corners sit where the strokes meet:
//! the runs are laid to the strokes (`outline::runs_for`). That a ring's
//! ink stays off its node's section boxes is a premise no projection rule
//! holds — a ring's letters reach into a section's line box, and a frame
//! laid inward centers its ink on the node's outline (`format/schema.md`,
//! "A frame's text sits on one curve") — so it is checked, not assumed: a
//! stroke that would still cross ink, where the letters leave it no room,
//! is not drawn.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::structure::{StructOp, Subtree};
use baumhard::engine::tree::Placement;
use baumhard::flow::Rect;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::run::RunShape;
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::{GlyphLayers, Projector, RUN_CHANNEL};
use glam::Vec2;

use super::outline::{self, OutlineStyle, StrokeModel, Strokes};
use crate::application::document::selection::kind_of;

/// The channel of the frames' group under the chrome's root: after the
/// caret's ([`super::caret::CARET_GROUP_CHANNEL`]).
pub(crate) const FRAMES_GROUP_CHANNEL: usize = 5;

/// A section's frame: light box-drawing lines, half-transparent.
pub(crate) const THIN: OutlineStyle = OutlineStyle {
    corners: ['\u{250C}', '\u{2510}', '\u{2514}', '\u{2518}'],
    horizontal: '\u{2500}',
    vertical: "\u{2502}",
    alpha: 150,
};

/// The focused section's frame: heavy lines, opaque.
pub(crate) const HEAVY: OutlineStyle = OutlineStyle {
    corners: ['\u{250F}', '\u{2513}', '\u{2517}', '\u{251B}'],
    horizontal: '\u{2501}',
    vertical: "\u{2503}",
    alpha: 255,
};

/// One frame as it is wanted or shown: where its strokes run, which of
/// its sides — top, bottom, left, right — are drawn, and whether it is
/// the focused section's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    /// Where its four strokes run.
    pub(crate) strokes: Strokes,
    /// How far down its left and right sides run, from and to: between
    /// its top and bottom strokes, or less, where a side stops short of a
    /// ring's ink.
    pub(crate) reach: [(f32, f32); 2],
    /// Which sides are drawn: a side whose stroke would cross a ring's
    /// ink is not.
    pub(crate) shown: [bool; 4],
    /// Whether it is the focused section's, drawn heavy.
    pub(crate) focused: bool,
}

/// Where a frame of `rect` goes among `inks` — the boxes its node's
/// rings' glyphs ink — around `text`, the box its own letters ink, for an
/// outline drawn as `model` says, and which of its sides it draws.
///
/// Each side's clearance is the distance from the box's edge to the
/// nearest ring ink beyond that edge — the side a box whose center lies
/// outside the box lies furthest beyond, which is less than nothing where
/// the ink reaches into the box, as a ring's letters reach into a
/// section's line box — among the boxes within the outline's reach along
/// it. Its stroke stands at half of it, at most where the outline
/// [stood](StrokeModel::standing), and at least a stroke's width off the
/// box. With less room than that it goes inside the box — hugs it — but
/// never nearer the section's own letters than a stroke's width. Each
/// stroke is placed on its own: the runs are laid to meet the strokes
/// ([`outline::runs_for`]), not the strokes to fit the runs. A side whose
/// stroke then still crosses ring ink — ink reaching into the box past
/// its letters, or a box lying over it — is not drawn; a left or right
/// side that crosses ink near an end runs only as far as a stroke's width
/// short of it, and is drawn if what is left clears all ink.
///
/// Returns the strokes, how far down the left and the right sides run,
/// and which sides are drawn.
///
/// Cost: O(the ink boxes), twice.
pub(crate) fn place(
    rect: Rect,
    text: Option<Rect>,
    inks: &[Rect],
    model: &StrokeModel,
) -> (Strokes, [(f32, f32); 2], [bool; 4]) {
    let standing = model.standing();
    let reach = standing.iter().copied().fold(0.0_f32, f32::max);
    let (min, max) = (rect.origin, rect.max());
    let mut clearance = [f32::INFINITY; 4];
    for ink in inks {
        let (lo, hi) = (ink.origin, ink.max());
        let seps = [min.y - hi.y, lo.y - max.y, min.x - hi.x, lo.x - max.x];
        let along_x = lo.x < max.x + reach && hi.x > min.x - reach;
        let along_y = lo.y < max.y + reach && hi.y > min.y - reach;
        let reaches = [along_x, along_x, along_y, along_y];
        let center = (lo + hi) * 0.5;
        if center.x >= min.x && center.x <= max.x && center.y >= min.y && center.y <= max.y {
            // Over the box itself: no side's clearance keeps clear of it;
            // the check below hides a side that crosses it.
            continue;
        }
        let outside = seps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        for side in 0..4 {
            if reaches[side] && seps[side] == outside {
                clearance[side] = clearance[side].min(seps[side]);
            }
        }
    }
    // How far inside each edge the section's own letters begin: the
    // nearest a hugging stroke may come is a stroke's width off them.
    let inner = text.map_or([f32::INFINITY; 4], |ink| {
        [
            ink.origin.y - min.y,
            max.y - ink.max().y,
            ink.origin.x - min.x,
            max.x - ink.max().x,
        ]
    });
    let halves = [model.h_half, model.h_half, model.v_half, model.v_half];
    let [top, bottom, left, right]: [f32; 4] = std::array::from_fn(|side| {
        let (c, half, stood) = (clearance[side], halves[side], standing[side]);
        // A stroke's width off the box, or inside it, down to a stroke's
        // width off the letters; never past the ink, unless the letters
        // leave no room, when the check below hides it.
        let off = 2.0 * half;
        let floor = off - inner[side].max(0.0);
        let least = off.min(c - half).max(floor);
        let most = stood.min(c - half).max(least);
        (c * 0.5).clamp(least, most)
    });
    let strokes = Strokes {
        top: min.y - top,
        bottom: max.y + bottom,
        left: min.x - left,
        right: max.x + right,
    };
    let full = [(strokes.top, strokes.bottom); 2];
    let crosses = |segment: Rect| inks.iter().any(|ink| overlaps(segment, *ink));
    let mut shown = segments(&strokes, full, model).map(|segment| !crosses(segment));
    let mut reach = full;
    for (k, side) in [2_usize, 3].into_iter().enumerate() {
        if shown[side] {
            continue;
        }
        let band = segments(&strokes, full, model)[side];
        let middle = (strokes.top + strokes.bottom) * 0.5;
        let (mut from, mut to) = (strokes.top, strokes.bottom);
        for ink in inks {
            let across = ink.origin.x < band.max().x && ink.max().x > band.origin.x;
            if !across || ink.max().y <= strokes.top || ink.origin.y >= strokes.bottom {
                continue;
            }
            if (ink.origin.y + ink.max().y) * 0.5 < middle {
                from = from.max(ink.max().y + model.v_half);
            } else {
                to = to.min(ink.origin.y - model.v_half);
            }
        }
        let mut cut = reach;
        cut[k] = (from, to);
        if to - from > outline::MIN_SIDE && !crosses(segments(&strokes, cut, model)[side]) {
            reach = cut;
            shown[side] = true;
        }
    }
    (strokes, reach, shown)
}

/// The four strokes of an outline on `strokes`, as the boxes their ink
/// covers — top, bottom, left, right — its sides running as far as
/// `reach` says. O(1).
pub(crate) fn segments(strokes: &Strokes, reach: [(f32, f32); 2], model: &StrokeModel) -> [Rect; 4] {
    let [top, bottom, _, _] = strokes.segments(model);
    let side = |x: f32, (from, to): (f32, f32)| {
        Rect::new(
            Vec2::new(x - model.v_half, from),
            Vec2::new(2.0 * model.v_half, (to - from).max(0.0)),
        )
    };
    [
        top,
        bottom,
        side(strokes.left, reach[0]),
        side(strokes.right, reach[1]),
    ]
}

/// The box the ink of `section`'s own letters covers, on the canvas: its
/// laid glyphs' outline inks as the renderer draws them
/// (`baumhard::font::fonts::glyph_outline_ink`), together. `None` for a
/// section that is not drawn or draws no ink.
///
/// Cost: O(its glyphs) outline-cache lookups, under `fonts`.
pub(crate) fn text_ink(
    projector: &Projector,
    section: NodeId,
    fonts: &mut FontSystemLease,
    swash: &mut glyphon::SwashCache,
) -> Option<Rect> {
    let (origin, shaped) = projector.section_box(section)?;
    let mut ink: Option<(Vec2, Vec2)> = None;
    for line in shaped.buffer.layout_runs() {
        for glyph in line.glyphs {
            let Some((lo, hi)) = baumhard::font::fonts::glyph_outline_ink(fonts.fonts(), swash, glyph) else {
                continue;
            };
            let at = origin + Vec2::new(glyph.x, line.line_y);
            let (lo, hi) = (at + lo, at + hi);
            ink = Some(ink.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
    }
    ink.map(|(lo, hi)| Rect::new(lo, hi - lo))
}

/// Whether two boxes share more than a touch. O(1).
fn overlaps(a: Rect, b: Rect) -> bool {
    const TOUCH: f32 = 1e-3;
    a.origin.x < b.max().x - TOUCH
        && b.origin.x < a.max().x - TOUCH
        && a.origin.y < b.max().y - TOUCH
        && b.origin.y < a.max().y - TOUCH
}

/// The boxes the glyphs of `node`'s rings ink, on the canvas: each placed
/// glyph's outline ink where its run read it, else its advance by the
/// line height, as the projection boxes a glyph it reads no ink of; a
/// blank glyph inks nothing.
///
/// Cost: O(the node's ring glyphs).
pub(crate) fn ring_inks(
    doc: &Document,
    projector: &Projector,
    layers: &GlyphLayers,
    node: NodeId,
) -> Vec<Rect> {
    let mut inks = Vec::new();
    let rings = doc
        .children(node)
        .filter(|&child| kind_of(doc, child) == Some(DocKind::Frame))
        .flat_map(|frame| doc.children(frame))
        .filter(|&ring| kind_of(doc, ring) == Some(DocKind::Ring));
    for ring in rings {
        for (layer, glyph) in projector.glyphs_of(ring) {
            let Some(run) = layers.tree(layer).get(glyph).and_then(|n| n.get().glyph_run()) else {
                continue;
            };
            let RunShape::Path { run: shaped, placed } = run.shaped() else {
                continue;
            };
            for g in &placed.glyphs {
                let Some(metrics) = shaped.glyphs.get(g.glyph as usize) else {
                    continue;
                };
                if metrics.blank {
                    continue;
                }
                let at = run.origin() + g.pos;
                inks.push(match metrics.ink {
                    Some(ink) => Rect::new(
                        at + Vec2::new(0.0, shaped.baseline()) + ink.min,
                        ink.max - ink.min,
                    ),
                    None => Rect::new(at, Vec2::new(metrics.advance, shaped.line_height)),
                });
            }
        }
    }
    inks
}

/// The section frames in the chrome. See the [module docs](self).
pub(crate) struct SectionFrames {
    /// The frames' group under the chrome's root.
    group: NodeId,
    /// Each slot's four runs: top, bottom, left, right.
    slots: Vec<[NodeId; 4]>,
    /// What each slot shows.
    shown: Vec<Option<Frame>>,
    /// The node it outlines the sections of.
    node: Option<NodeId>,
    /// The focused section.
    focus: Option<NodeId>,
    /// Where the thin and the heavy outlines draw their strokes: measured
    /// at the first frame, once.
    models: Option<[StrokeModel; 2]>,
}

impl SectionFrames {
    /// Put the frames' group in `chrome`, empty, under the root. Called
    /// once, when the chrome is built.
    pub(crate) fn plant(chrome: &mut GfxTree) -> SectionFrames {
        let group = chrome
            .insert_child(
                chrome.root(),
                GfxElement::new_void(FRAMES_GROUP_CHANNEL),
                Placement::Last,
            )
            .expect("the chrome's root takes a group");
        SectionFrames {
            group,
            slots: Vec::new(),
            shown: Vec::new(),
            node: None,
            focus: None,
            models: None,
        }
    }

    /// Where the thin and the heavy outlines draw their strokes, measured
    /// the first time they are asked for. O(1) after that.
    fn models(&mut self) -> [StrokeModel; 2] {
        *self.models.get_or_insert_with(|| {
            let mut fonts = FontSystemLease::new("view::frames::models");
            [
                StrokeModel::of(&THIN, &mut fonts),
                StrokeModel::of(&HEAVY, &mut fonts),
            ]
        })
    }

    /// The frames `node` wants, focused on `focus`: one per drawn section,
    /// in sibling order, when it has more than one — a node of one section
    /// has nothing to pick, and its frame would only repeat its own, as on
    /// `main` — each [placed](place) inside the node's rings' inks.
    ///
    /// Cost: O(its sections × its ring glyphs), and its sections' glyphs'
    /// outline-cache lookups; asked only when
    /// [`Self::due`]: on entering NodeEdit, on a change of focus, and on a
    /// pass that may have moved the node, its sections or its rings.
    pub(crate) fn wanted(
        &mut self,
        doc: &Document,
        projector: &Projector,
        layers: &GlyphLayers,
        node: Option<NodeId>,
        focus: Option<NodeId>,
    ) -> Vec<Frame> {
        let Some(node) = node else {
            return Vec::new();
        };
        let sections: Vec<NodeId> = doc
            .children(node)
            .filter(|&child| kind_of(doc, child) == Some(DocKind::Section))
            .collect();
        if sections.len() < 2 {
            return Vec::new();
        }
        let models = self.models();
        let inks = ring_inks(doc, projector, layers, node);
        let mut fonts = FontSystemLease::new("view::frames::wanted");
        let mut swash = glyphon::SwashCache::new();
        sections
            .into_iter()
            .filter_map(|section| {
                let rect = projector
                    .section_rect(section)
                    .filter(|r| r.size.x > 0.0 && r.size.y > 0.0 && r.size.is_finite())?;
                let focused = focus == Some(section);
                let text = text_ink(projector, section, &mut fonts, &mut swash);
                let (strokes, reach, shown) = place(rect, text, &inks, &models[usize::from(focused)]);
                Some(Frame {
                    strokes,
                    reach,
                    shown,
                    focused,
                })
            })
            .collect()
    }

    /// Whether a pass must look at the frames again: the node or the focus
    /// changed, or `follow` says the node's geometry may have moved. O(1).
    pub(crate) fn due(&self, node: Option<NodeId>, focus: Option<NodeId>, follow: bool) -> bool {
        node != self.node || focus != self.focus || (follow && node.is_some())
    }

    /// The structural inserts that give the pool `count` slots, each four
    /// hidden runs under the group; nothing when it has them. Applied, the
    /// new runs are read back with [`Self::adopt`].
    pub(crate) fn grow(&self, count: usize) -> Vec<(GfxMutatorTree, Anchors)> {
        let missing = count.saturating_sub(self.slots.len());
        if missing == 0 {
            return Vec::new();
        }
        let mut fonts = FontSystemLease::new("view::frames::grow");
        (0..missing * 4)
            .map(|_| {
                let run = outline::run_of("", Vec2::ZERO, 0.0, THIN.alpha, &mut fonts);
                let insert = StructOp::Insert {
                    subtree: Subtree::leaf(GfxElement::new_run(run, RUN_CHANNEL, 0)),
                    at: Placement::Last,
                };
                (
                    GfxMutatorTree::new_with(GfxMutator::new_structure(insert, RUN_CHANNEL)),
                    Anchors::Nodes(vec![self.group]),
                )
            })
            .collect()
    }

    /// Take the runs [`Self::grow`]'s inserts put under the group as the
    /// pool's new slots, four by four, in order. O(the group's runs).
    pub(crate) fn adopt(&mut self, chrome: &GfxTree) {
        let runs: Vec<NodeId> = self.group.children(chrome.arena()).collect();
        for &chunk in runs.as_chunks::<4>().0.iter().skip(self.slots.len()) {
            self.slots.push(chunk);
            self.shown.push(None);
        }
    }

    /// The chrome mutators that show `want` — slot by slot, replacing the
    /// four runs of every slot whose frame changed and hiding the slots no
    /// frame needs — and take `node` and `focus` as what it shows. The
    /// pool must hold a slot per frame ([`Self::grow`]); a frame past it is
    /// left out.
    ///
    /// Cost: four box shapings per slot that changed.
    pub(crate) fn update(
        &mut self,
        node: Option<NodeId>,
        focus: Option<NodeId>,
        want: &[Frame],
    ) -> Vec<(GfxMutatorTree, Anchors)> {
        self.node = node;
        self.focus = focus;
        let models = self.models();
        let mut fonts = FontSystemLease::new("view::frames");
        let mut out = Vec::new();
        for (slot, runs) in self.slots.iter().enumerate() {
            let next = want.get(slot).copied();
            if self.shown[slot] == next {
                continue;
            }
            self.shown[slot] = next;
            let focused = next.is_some_and(|frame| frame.focused);
            let style = if focused { &HEAVY } else { &THIN };
            let built = outline::runs_for(
                next.map(|frame| frame.strokes),
                next.map_or([(0.0, 0.0); 2], |frame| frame.reach),
                next.map_or([false; 4], |frame| frame.shown),
                &models[usize::from(focused)],
                style,
                &mut fonts,
            );
            for (run, &id) in built.into_iter().zip(runs) {
                out.push((
                    GfxMutatorTree::new_with(GfxMutator::new(
                        GfxMutation::RunSet(Box::new(run)),
                        RUN_CHANNEL,
                    )),
                    Anchors::Nodes(vec![id]),
                ));
            }
        }
        out
    }

    /// How many slots the pool holds. Test builds only.
    #[cfg(test)]
    pub(crate) fn slots(&self) -> usize {
        self.slots.len()
    }

    /// What each slot shows. Test builds only.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> &[Option<Frame>] {
        &self.shown
    }
}
