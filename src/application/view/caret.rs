// SPDX-License-Identifier: MPL-2.0

//! **A text editor's caret and selected range** (V1-C8, #249; design
//! §12.1, §12.5; ruling K1 on #249): view-only chrome placed from where
//! the projector drew the edited owner. Nothing here reads the document
//! back from glyphs: what an editor holds is the document (it stages
//! every change), and this places a caret over its projection.
//!
//! The view keeps it itself, as it keeps the handles, because it follows
//! the editor as well as the document: the editor tells the view where
//! its caret is ([`EditorFocus`], `MapView::follow_editor`), and a sync
//! pass rewrites the caret only when the focus changed — the caret or the
//! anchor moved, or another owner is edited — or when the pass touched
//! the owner ([`touches`]) or moved its geometry or its node's
//! (`Projector::geometry_moved`). A pan or a zoom does none of that and
//! writes nothing (§12.4).
//!
//! **Where the caret goes** is read from the projection:
//!
//! - a **section**: the projector's shaping of it (`Projector::section_box`)
//!   — its laid-out lines, broken at `\n` only (D26), so the caret's line
//!   is the count of line breaks before it, and its place on that line the
//!   first glyph at or after it (the end of the line past the last);
//! - a **label** (a box run) the same way, from the run the labels or
//!   portals layer holds for it;
//! - a **ring, a cap, a portal end, a path's content** (path runs): the
//!   placed glyphs, in text order. The caret sits between the boxes of the
//!   glyphs before and after it — the midpoint of their centers — so it
//!   follows the path whichever way a stretch is laid (D33's readable
//!   rule places a stretch reversed); at either end it is extrapolated
//!   half a step out. A repeated run's first copy holds it.
//!
//! Each glyph is mapped to its grapheme of the owner through the span and
//! the bytes shaping recorded for it, against the owner's spans counted
//! the way the editor counts them (span by span); a filler or a `content`
//! glyph maps to none. An owner that draws no glyph of its own text — an
//! empty label, a path with no spans — puts the caret where the owner is
//! drawn: a label's center, a path's middle, a portal end's marker.
//!
//! **The selected range** is the same mapping run the other way: the
//! placed glyphs whose graphemes fall in it, which the view's
//! [`RangeTint`](super::overlay::RangeTint) colors.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind, SpanData};
use baumhard::document::ids::Document;
use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{Align, FontSize, Weight};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::journal::Touch;
use baumhard::engine::tree::Placement;
use baumhard::flow::{shape_box, FlowSpan, ShapedBox, FILLER_SPAN};
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp, RunShape};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::{GlyphLayers, Layer, LayerChanges, Projector};
use baumhard::util::grapheme_chad;
use glam::Vec2;
use std::sync::Arc;

use super::overlay::{owner_of, GlyphRange, SELECTION_HIGHLIGHT};
use crate::application::document::selection::kind_of;

/// The glyph the caret draws: ASCII `|`, which every text face the app
/// ships has, so the caret matches the face's metrics (`main`'s reason).
pub(crate) const CARET_GLYPH: &str = "|";

/// The size the caret is shaped at; it is scaled to the edited line's
/// height when shown.
const CARET_SIZE_PT: f32 = 16.0;

/// The channel of the caret's group under the chrome's root: after the
/// handle sets' ([`super::handles::SET_GROUP_CHANNELS`]).
pub(crate) const CARET_GROUP_CHANNEL: usize = 4;

/// The channel of the caret's run in its group.
const CARET_CHANNEL: usize = 0;

/// What an open text editor tells the view: which owner it edits, the
/// element the selection names for it, and where its caret and its
/// shift-select anchor are, in grapheme clusters of the owner's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EditorFocus {
    /// The element whose spans are edited: a section, a ring, a cap, a
    /// label, a portal end, a connector or a line link.
    pub(crate) owner: NodeId,
    /// The node or path the selection names for it, on which the
    /// selection tint stands down while the editor is open: what is
    /// edited is shown by its caret.
    pub(crate) host: NodeId,
    /// The caret.
    pub(crate) caret: usize,
    /// Where a live shift-select began.
    pub(crate) anchor: Option<usize>,
}

impl EditorFocus {
    /// The selected range, start before end, when the anchor and the
    /// caret differ. O(1).
    pub(crate) fn range(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.caret).then(|| (anchor.min(self.caret), anchor.max(self.caret)))
    }
}

/// Whether a pass's `changes` wrote a run `element` owns — the element's
/// own runs, its node's when it is a node, or a removal of what it drew —
/// in any layer: the question the caret asks of its owner (the handles
/// ask of geometry instead, `Projector::geometry_moved`). A removal is
/// read from the owner the projector recorded when it wrote it
/// (`LayerChanges::removed_owner`), since by now it has forgotten the
/// glyph.
///
/// Cost: O(the elements the pass wrote × the depth to each one's owner,
/// at most three); no walk of a layer or of the document.
pub(crate) fn touches(
    changes: &LayerChanges,
    doc: &Document,
    projector: &Projector,
    element: NodeId,
) -> bool {
    Layer::ALL.into_iter().any(|layer| {
        changes.journal(layer).touched().any(|(glyph, touch)| {
            if touch == Touch::Removed {
                return changes.removed_owner(layer, glyph) == Some(element);
            }
            projector.element_of(layer, glyph) == Some(element)
                || owner_of(doc, projector, layer, glyph) == Some(element)
        })
    })
}

/// The grapheme of the owner each glyph's (span, byte) source falls in:
/// the owner's spans counted span by span, as the editor counts them.
struct Graphemes {
    /// The first grapheme index of each span.
    bases: Vec<usize>,
    /// Each span's grapheme byte starts.
    starts: Vec<Vec<usize>>,
    /// Each span's text, for the line breaks.
    texts: Vec<String>,
}

impl Graphemes {
    /// The owner's spans, read from the document. O(their text).
    fn of(doc: &Document, owner: NodeId) -> Self {
        let spans: Vec<&SpanData> = doc
            .children(owner)
            .filter_map(|id| match doc.get(id) {
                Some(DocElement::Span(span)) => Some(span),
                _ => None,
            })
            .collect();
        let mut base = 0;
        let mut bases = Vec::with_capacity(spans.len());
        let mut starts = Vec::with_capacity(spans.len());
        for span in &spans {
            bases.push(base);
            let s: Vec<usize> = grapheme_chad::grapheme_byte_starts(&span.text).collect();
            base += s.len();
            starts.push(s);
        }
        Graphemes {
            bases,
            starts,
            texts: spans.iter().map(|s| s.text.clone()).collect(),
        }
    }

    /// Whether the owner holds no text.
    fn is_empty(&self) -> bool {
        self.starts.iter().all(Vec::is_empty)
    }

    /// The grapheme a glyph from byte `byte` of span `span` is part of;
    /// `None` for a filler, a `content` glyph, or a span the owner does
    /// not hold. O(log of the span's clusters).
    fn index(&self, span: usize, byte: usize) -> Option<usize> {
        if span == FILLER_SPAN {
            return None;
        }
        let starts = self.starts.get(span)?;
        let within = starts.partition_point(|&s| s <= byte).checked_sub(1)?;
        Some(self.bases[span] + within)
    }

    /// The line `caret` is on: how many line breaks come before it. O(the
    /// text before it).
    fn line_of(&self, caret: usize) -> usize {
        let mut seen = 0;
        let mut lines = 0;
        for (text, starts) in self.texts.iter().zip(&self.starts) {
            for (i, &from) in starts.iter().enumerate() {
                if seen >= caret {
                    return lines;
                }
                let to = starts.get(i + 1).copied().unwrap_or(text.len());
                if matches!(&text[from..to], "\n" | "\r\n") {
                    lines += 1;
                }
                seen += 1;
            }
        }
        lines
    }
}

/// Where the caret for `caret` in `owner` centers, and the height of the
/// line it stands in, read from the projection. `None` when the owner is
/// not drawn. Cost: the owner's spans and its run's glyphs, once each.
pub(crate) fn caret_place(
    doc: &Document,
    projector: &Projector,
    layers: &GlyphLayers,
    owner: NodeId,
    caret: usize,
) -> Option<(Vec2, f32)> {
    let graphemes = Graphemes::of(doc, owner);
    if kind_of(doc, owner) == Some(DocKind::Section) {
        let (origin, shaped) = projector.section_box(owner)?;
        let align = match doc.get(owner) {
            Some(DocElement::Section(section)) => section.align.unwrap_or(Align::Left),
            _ => Align::Left,
        };
        return Some(box_caret(shaped, origin, align, &graphemes, caret));
    }
    let run = owner_run(doc, projector, layers, owner).map(|(_, _, run)| run);
    let drawn = run.and_then(|run| match run.shaped() {
        RunShape::Box(shaped) => Some(box_caret(shaped, run.origin(), Align::Center, &graphemes, caret)),
        RunShape::Path { run: shaped, placed } => {
            let steep = |at: Vec2| steep_at(doc, projector, owner, at);
            path_caret(shaped, placed, run.origin(), &graphemes, caret, &steep)
        }
    });
    drawn.or_else(|| fallback_place(doc, projector, owner))
}

/// The run `owner` draws, as its layer holds it, with where it is. A
/// section, a ring and a path are units of their own, but a label, a cap
/// and a portal end's text and label are slots of their path's or their
/// portal's unit, which `Projector::glyphs_of` answers for the unit
/// alone, every slot of it — so the run is looked for among the slots of
/// the owner and of its ancestors, and is the one the projector says
/// draws the owner (`Projector::element_of`), never a sibling slot's.
///
/// Cost: O(the depth to the unit, at most three × its slots).
fn owner_run<'l>(
    doc: &Document,
    projector: &Projector,
    layers: &'l GlyphLayers,
    owner: NodeId,
) -> Option<(Layer, NodeId, &'l GlyphRunData)> {
    std::iter::successors(Some(owner), |&at| doc.parent(at))
        .take(4)
        .find_map(|unit| {
            projector.glyphs_of(unit).into_iter().find_map(|(layer, glyph)| {
                if projector.element_of(layer, glyph) != Some(owner) {
                    return None;
                }
                let run = layers.tree(layer).get(glyph)?.get().glyph_run()?;
                Some((layer, glyph, run))
            })
        })
}

/// Where an owner that draws no glyph of its own text puts the caret:
/// where it is drawn — a label's center, a path's middle, a portal end's
/// marker. `None` for an owner not drawn.
fn fallback_place(doc: &Document, projector: &Projector, owner: NodeId) -> Option<(Vec2, f32)> {
    let at = match kind_of(doc, owner)? {
        DocKind::Label => projector.label_pos(owner)?,
        DocKind::Connector | DocKind::Link => projector.point_along(owner, 0.5)?,
        DocKind::CapStart => projector.point_along(doc.parent(owner)?, 0.0)?,
        DocKind::CapEnd => projector.point_along(doc.parent(owner)?, 1.0)?,
        DocKind::PortalEnd => projector.portal_end_pos(doc, owner)?,
        DocKind::Ring => projector.node_rect(doc.parent(doc.parent(owner)?)?)?.origin,
        _ => return None,
    };
    Some((at, CARET_SIZE_PT))
}

/// The caret in a box flow drawn at `origin`: on the line holding it, at
/// the first glyph at or after it, or past the line's last glyph; on an
/// empty line, where `align` starts one. O(the box's glyphs).
fn box_caret(
    shaped: &ShapedBox,
    origin: Vec2,
    align: Align,
    graphemes: &Graphemes,
    caret: usize,
) -> (Vec2, f32) {
    let line = graphemes.line_of(caret);
    let mut index = 0;
    let mut last = None;
    for run in shaped.buffer.layout_runs() {
        let first = index;
        index += run.glyphs.len();
        last = Some((run.line_top, run.line_height));
        if run.line_i != line {
            continue;
        }
        let at = |x: f32| {
            (
                origin + Vec2::new(x, run.line_top + run.line_height * 0.5),
                run.line_height,
            )
        };
        let mut end = None;
        for (k, glyph) in run.glyphs.iter().enumerate() {
            let source = shaped.glyph_spans.get(first + k);
            if let Some(g) = source.and_then(|(span, bytes)| graphemes.index(*span, bytes.start)) {
                if g >= caret {
                    return at(glyph.x);
                }
            }
            end = Some(glyph.x + glyph.w);
        }
        let x = end.unwrap_or(match align {
            Align::Left => 0.0,
            Align::Center => shaped.size.x * 0.5,
            Align::Right => shaped.size.x,
        });
        return at(x);
    }
    // A line past the last laid-out one: a caret after a final line break.
    let (top, height) = last.map_or((0.0, CARET_SIZE_PT), |(top, h)| (top + h, h));
    let x = match align {
        Align::Left => 0.0,
        Align::Center => shaped.size.x * 0.5,
        Align::Right => shaped.size.x,
    };
    (origin + Vec2::new(x, top + height * 0.5), height)
}

/// Whether the path `owner`'s text is laid along runs steep at `at`, a
/// placed glyph's center: whether its tangent there is at least 45° from
/// level, as the readable rule classes a step (D33, `flow::place`), so
/// text there reads top to bottom. A connector's or a link's content and
/// its caps read their curve, near `at`; a ring reads its node's outline,
/// whose tangent is square to the line from the node's center; a portal
/// end's text is laid at a point, level. O(the curve samples, 64) for a
/// path; O(1) otherwise.
fn steep_at(doc: &Document, projector: &Projector, owner: NodeId, at: Vec2) -> bool {
    let steep = |tangent: Vec2| {
        let length = tangent.length();
        length > 0.0 && (tangent.x / length).abs() <= std::f32::consts::FRAC_1_SQRT_2
    };
    let path = match kind_of(doc, owner) {
        Some(DocKind::Connector | DocKind::Link) => Some(owner),
        Some(DocKind::CapStart | DocKind::CapEnd) => doc.parent(owner),
        Some(DocKind::Ring) => {
            let center = doc
                .parent(owner)
                .and_then(|frame| doc.parent(frame))
                .and_then(|node| projector.node_rect(node))
                .map(|rect| rect.center());
            return center.is_some_and(|center| {
                let radial = at - center;
                steep(Vec2::new(-radial.y, radial.x))
            });
        }
        _ => None,
    };
    let Some(path) = path else {
        return false;
    };
    const SAMPLES: usize = 64;
    let points: Vec<Vec2> = (0..=SAMPLES)
        .filter_map(|k| projector.point_along(path, k as f32 / SAMPLES as f32))
        .collect();
    let Some(nearest) = (0..points.len()).min_by(|&a, &b| {
        points[a]
            .distance_squared(at)
            .total_cmp(&points[b].distance_squared(at))
    }) else {
        return false;
    };
    let before = points[nearest.saturating_sub(1)];
    let after = points[(nearest + 1).min(points.len() - 1)];
    steep(after - before)
}

/// The caret in a path flow: between the centers of the placed glyphs
/// before and after it, the first copy's; at an end, half a step out —
/// the step between the last two centers, or for an owner that places
/// one cluster, half its cell along the way its text reads there: down
/// where the path is steep (`steep`), else right. `None` when no placed
/// glyph maps to the owner's text. O(the placed glyphs).
fn path_caret(
    shaped: &baumhard::flow::ShapedRun,
    placed: &baumhard::flow::PlacedRun,
    origin: Vec2,
    graphemes: &Graphemes,
    caret: usize,
    steep: &dyn Fn(Vec2) -> bool,
) -> Option<(Vec2, f32)> {
    let height = shaped.line_height;
    // Each cluster's center, in text order, with its grapheme: the first
    // copy only, so a repeated run's caret stays on its first.
    let mut clusters: Vec<(usize, Vec2)> = Vec::new();
    for glyph in &placed.glyphs {
        let Some(metrics) = shaped.glyphs.get(glyph.glyph as usize) else {
            continue;
        };
        if !metrics.grapheme_start {
            continue;
        }
        let Some(g) = graphemes.index(metrics.span, metrics.bytes.start) else {
            continue;
        };
        if clusters.last().is_some_and(|&(prev, _)| g <= prev) {
            break;
        }
        let center = origin + glyph.pos + Vec2::new(metrics.advance * 0.5, height * 0.5);
        clusters.push((g, center));
    }
    if clusters.is_empty() || graphemes.is_empty() {
        return None;
    }
    let after = clusters.iter().position(|&(g, _)| g >= caret);
    let center = |i: usize| clusters[i].1;
    // One cluster: half its cell along the way the text reads there.
    let half_cell = || {
        if steep(center(0)) {
            Vec2::new(0.0, height * 0.5)
        } else {
            Vec2::new(shaped.glyphs.first().map_or(0.0, |g| g.advance) * 0.5, 0.0)
        }
    };
    let place = match after {
        Some(0) if clusters.len() > 1 => center(0) - (center(1) - center(0)) * 0.5,
        Some(0) => center(0) - half_cell(),
        Some(i) => (center(i - 1) + center(i)) * 0.5,
        None => {
            let n = clusters.len();
            if n > 1 {
                center(n - 1) + (center(n - 1) - center(n - 2)) * 0.5
            } else {
                center(0) + half_cell()
            }
        }
    };
    Some((place, height))
}

/// The run `owner` draws and the glyphs of it that `start..end` of its
/// text covers: what the editor's [`RangeTint`](super::overlay::RangeTint)
/// colors. `None` for an owner not drawn, or a range that covers no glyph.
/// O(the run's glyphs).
pub(crate) fn range_glyphs(
    doc: &Document,
    projector: &Projector,
    layers: &GlyphLayers,
    owner: NodeId,
    start: usize,
    end: usize,
) -> Option<GlyphRange> {
    let graphemes = Graphemes::of(doc, owner);
    let (layer, glyph, run) = owner_run(doc, projector, layers, owner)?;
    let inside = |source: Option<usize>| source.is_some_and(|g| start <= g && g < end);
    let mut first = None;
    let mut last = None;
    match run.shaped() {
        RunShape::Box(shaped) => {
            for (i, (span, bytes)) in shaped.glyph_spans.iter().enumerate() {
                if inside(graphemes.index(*span, bytes.start)) {
                    first.get_or_insert(i);
                    last = Some(i);
                }
            }
        }
        RunShape::Path { run: shaped, placed } => {
            for (i, glyph) in placed.glyphs.iter().enumerate() {
                let source = shaped
                    .glyphs
                    .get(glyph.glyph as usize)
                    .and_then(|m| graphemes.index(m.span, m.bytes.start));
                if inside(source) {
                    first.get_or_insert(i);
                    last = Some(i);
                } else if first.is_some() && source.is_some_and(|g| g < start) {
                    // A repeated run's second copy: the first one holds it.
                    break;
                }
            }
        }
    }
    Some((layer, glyph, first?..last? + 1))
}

/// The caret in the chrome layer: one run of [`CARET_GLYPH`] in its own
/// group, planted hidden and moved in place.
pub(crate) struct Caret {
    /// The caret's group under the chrome's root.
    group: NodeId,
    /// The caret's run, as shaped, and half its size.
    half: Vec2,
    /// The height of the shaped caret's line, which a shown caret is
    /// scaled from.
    height: f32,
    /// What it follows.
    focus: Option<EditorFocus>,
    /// Where it centers and how tall it stands while shown.
    shown: Option<(Vec2, f32)>,
    /// How many times it was placed again: what the tests that hold a pan
    /// to placing nothing count. Test builds only.
    #[cfg(test)]
    placed: u64,
}

impl Caret {
    /// Put the caret in `chrome`, hidden, in its group under the root.
    /// Called once, when the chrome is built.
    pub(crate) fn plant(chrome: &mut GfxTree) -> Caret {
        let mut fonts = FontSystemLease::new("view::caret::plant");
        let group = chrome
            .insert_child(
                chrome.root(),
                GfxElement::new_void(CARET_GROUP_CHANNEL),
                Placement::Last,
            )
            .expect("the chrome's root takes a group");
        let (run, size) = caret_run(&mut fonts);
        chrome
            .insert_child(group, GfxElement::new_run(run, CARET_CHANNEL, 0), Placement::Last)
            .expect("the caret's group takes its run");
        Caret {
            group,
            half: size * 0.5,
            height: size.y,
            focus: None,
            shown: None,
            #[cfg(test)]
            placed: 0,
        }
    }

    /// The chrome mutator that shows the caret for `focus`, when the focus
    /// changed or `follow` says the pass touched its owner: one mutator
    /// tree anchored at its group, which moves the run, scales it to the
    /// line and shows it — or hides it with no focus. Nothing when it
    /// would draw what it draws now.
    ///
    /// Cost: O(1) when nothing changed; otherwise one [`caret_place`].
    pub(crate) fn update(
        &mut self,
        focus: Option<EditorFocus>,
        follow: bool,
        doc: &Document,
        projector: &Projector,
        layers: &GlyphLayers,
    ) -> Option<(GfxMutatorTree, Anchors)> {
        if focus == self.focus && !follow {
            return None;
        }
        self.focus = focus;
        #[cfg(test)]
        {
            self.placed += 1;
        }
        let want = focus.and_then(|f| caret_place(doc, projector, layers, f.owner, f.caret));
        if want == self.shown {
            return None;
        }
        self.shown = want;
        let mutations = match want {
            Some((center, height)) => {
                let scale = if self.height > 0.0 {
                    height / self.height
                } else {
                    1.0
                };
                vec![
                    GfxMutation::RunOrigin(center - self.half),
                    GfxMutation::RunOverrides(RunOverrideOp::Scale {
                        range: 0..1,
                        value: scale,
                    }),
                    GfxMutation::RunOpacity(1.0),
                ]
            }
            None => vec![GfxMutation::RunOpacity(0.0)],
        };
        let mut tree = GfxMutatorTree::new_with(GfxMutator::new_void(CARET_GROUP_CHANNEL));
        let root = tree.root();
        tree.insert_child(
            root,
            GfxMutator::new_macro(mutations, CARET_CHANNEL),
            Placement::Last,
        )
        .expect("one child");
        Some((tree, Anchors::Nodes(vec![self.group])))
    }

    /// What the caret follows. O(1).
    pub(crate) fn focus(&self) -> Option<EditorFocus> {
        self.focus
    }

    /// Where the caret centers and how tall it stands, while shown. O(1).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<(Vec2, f32)> {
        self.shown
    }

    /// How many times the caret was placed again since the view was
    /// built. Test builds only.
    #[cfg(test)]
    pub(crate) fn placed(&self) -> u64 {
        self.placed
    }
}

/// The caret's run, hidden, at the origin, in the selection's cyan, and
/// its size.
fn caret_run(fonts: &mut FontSystemLease) -> (GlyphRunData, Vec2) {
    let [r, g, b, _] = SELECTION_HIGHLIGHT.0;
    let style = ResolvedText {
        font: baumhard::document::values::FontRef::DEFAULT,
        size: FontSize::Pt(CARET_SIZE_PT),
        color: baumhard::document::values::Rgba::new(r, g, b, 0xFF),
        weight: Weight::NORMAL,
        italic: false,
    };
    let spans = [FlowSpan {
        text: CARET_GLYPH,
        style: &style,
        link: None,
    }];
    let shaped = shape_box(&spans, None, Align::Left, fonts);
    let size = shaped.size;
    let run = GlyphRunData::new(
        RunShape::Box(Arc::new(shaped)),
        Vec2::ZERO,
        GlyphOverrides::none(),
        0.0,
        None,
    )
    .expect("a run with no overrides is always built");
    (run, size)
}
