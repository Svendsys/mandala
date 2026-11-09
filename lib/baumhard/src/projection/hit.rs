// SPDX-License-Identifier: MPL-2.0

//! Hit testing (#241 §6, ruling B7.1): the layers top-down — portals,
//! labels, connections, frames, sections — through each layer's glyph
//! BVH, then the visible nodes' bodies by their shape. Only a point
//! outside every node and every glyph is no hit.

use glam::Vec2;
use rustc_hash::FxHashMap;

use crate::arena::NodeId;
use crate::flow::{Rect, FILLER_SPAN};
use crate::gfx_structs::run::RunShape;
use crate::gfx_structs::shape::NodeShape;
use crate::projection::layers::{GlyphLayers, Layer};
use crate::projection::state::Owner;
use crate::projection::Projector;

/// What a point hits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The node whose element was hit: the node that owns it, or the node
    /// whose body holds the point.
    pub node: NodeId,
    /// The element hit: a section, ring, connector, link, cap, label or
    /// portal end — or the node itself, for its body.
    pub element: NodeId,
    /// The span of the glyph hit, when it has one: a filler glyph and a
    /// `content` text have none.
    pub span: Option<NodeId>,
    /// The glyph hit, by its index among the run's glyphs: a path run's
    /// placed glyphs, a box run's laid-out ones.
    pub glyph: Option<usize>,
}

/// The side of a cell of the body index, canvas units.
const CELL: f32 = 256.0;

/// A body spanning more cells than this is kept in a list of its own,
/// tried on every query, rather than in every cell it covers.
const MAX_CELLS: i64 = 64;

/// Where the visible nodes' bodies are, for the hit test: each node's
/// drawn rectangle and hit shape, in a grid of cells, so a query tries
/// only the bodies of one cell (and the few large ones). Kept by the
/// projector as nodes are measured: a node whose rectangle changes
/// leaves exactly its old cells and joins its new ones, and no other
/// node's entry is visited (ruling B7.1's approval).
#[derive(Debug, Default)]
pub(crate) struct BodyIndex {
    /// Each body's rectangle and shape, and whether it is large.
    bodies: FxHashMap<NodeId, (Rect, NodeShape, bool)>,
    /// The bodies in each cell.
    cells: FxHashMap<(i32, i32), Vec<NodeId>>,
    /// The bodies too large for the cells.
    large: Vec<NodeId>,
    /// How many cell entries the index has added and removed: what the
    /// test that holds a move to its own cells reads. Test builds only.
    #[cfg(test)]
    pub(crate) cell_writes: usize,
    /// How many cells `within` has visited: what the test that holds a
    /// rubber band to its own cells reads. Test builds only.
    #[cfg(test)]
    pub(crate) within_cells: std::cell::Cell<usize>,
    /// How many bodies `within` has tried against its rectangle. Test
    /// builds only.
    #[cfg(test)]
    pub(crate) within_tried: std::cell::Cell<usize>,
}

/// The cell coordinate of `v`, clamped to the grid.
fn cell_of(v: f32) -> i32 {
    let c = (v / CELL).floor();
    if c.is_nan() {
        0
    } else {
        c.clamp(i32::MIN as f32, i32::MAX as f32) as i32
    }
}

/// The first and last cells `rect` covers, however many: `None` for a
/// rectangle that is not finite.
pub(crate) fn span(rect: Rect) -> Option<((i32, i32), (i32, i32))> {
    if !rect.origin.is_finite() || !rect.size.is_finite() {
        return None;
    }
    let (x0, y0) = (cell_of(rect.origin.x), cell_of(rect.origin.y));
    let (x1, y1) = (cell_of(rect.max().x), cell_of(rect.max().y));
    Some(((x0, y0), (x1, y1)))
}

/// How many cells the span from `(x0, y0)` to `(x1, y1)` holds.
pub(crate) fn cell_count((x0, y0): (i32, i32), (x1, y1): (i32, i32)) -> i64 {
    (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1)
}

/// The cells `rect` covers, when they are few enough to list.
pub(crate) fn cells(rect: Rect) -> Option<((i32, i32), (i32, i32))> {
    span(rect).filter(|&(a, b)| cell_count(a, b) <= MAX_CELLS)
}

impl BodyIndex {
    /// Set `node`'s body: its old cells left, its new ones joined.
    /// O(the cells of the two rectangles).
    pub(crate) fn set(&mut self, node: NodeId, rect: Rect, shape: NodeShape) {
        self.remove(node);
        let large = match cells(rect) {
            Some(((x0, y0), (x1, y1))) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        self.cells.entry((x, y)).or_default().push(node);
                        #[cfg(test)]
                        {
                            self.cell_writes += 1;
                        }
                    }
                }
                false
            }
            None => {
                self.large.push(node);
                true
            }
        };
        self.bodies.insert(node, (rect, shape, large));
    }

    /// Take `node`'s body out. O(its cells, and the bodies sharing them).
    pub(crate) fn remove(&mut self, node: NodeId) {
        let Some((rect, _, large)) = self.bodies.remove(&node) else {
            return;
        };
        if large {
            self.large.retain(|&n| n != node);
            return;
        }
        if let Some(((x0, y0), (x1, y1))) = cells(rect) {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    if let Some(list) = self.cells.get_mut(&(x, y)) {
                        list.retain(|&n| n != node);
                        #[cfg(test)]
                        {
                            self.cell_writes += 1;
                        }
                        if list.is_empty() {
                            self.cells.remove(&(x, y));
                        }
                    }
                }
            }
        }
    }

    /// The smallest body holding `point` — the innermost, as the glyph
    /// BVH breaks ties — and on equal areas the least node id, so the
    /// answer does not depend on the order bodies were added. O(the
    /// bodies of the point's cell, and the large ones).
    pub(crate) fn at(&self, point: Vec2) -> Option<NodeId> {
        let cell = (cell_of(point.x), cell_of(point.y));
        let candidates = self
            .cells
            .get(&cell)
            .into_iter()
            .flatten()
            .chain(self.large.iter());
        let mut best: Option<(f32, NodeId)> = None;
        for &node in candidates {
            let Some(&(rect, shape, _)) = self.bodies.get(&node) else {
                continue;
            };
            let min = rect.origin;
            let max = rect.max();
            if !(point.x >= min.x && point.y >= min.y && point.x <= max.x && point.y <= max.y) {
                continue;
            }
            if !shape.contains_local(point - min, max - min) {
                continue;
            }
            let area = rect.size.x * rect.size.y;
            let better = match best {
                None => true,
                Some((a, n)) => area < a || (area == a && node < n),
            };
            if better {
                best = Some((area, node));
            }
        }
        best.map(|(_, node)| node)
    }

    /// Every body that `rect` overlaps by its shape, in ascending node
    /// id. **Inclusive**: a body whose edge `rect` only touches is in,
    /// and a zero-area `rect` is a point query that finds every body
    /// holding the point ([`NodeShape::intersects_local_aabb`]); a body
    /// of zero area is never in, as it is never hit. A query covering at
    /// most [`MAX_CELLS`] cells tries only the bodies of those cells and
    /// the large ones; a larger one tries every body. O(the bodies
    /// tried), plus a sort of what is found.
    pub(crate) fn within(&self, rect: Rect) -> Vec<NodeId> {
        let mut found = Vec::new();
        self.candidates(rect, |node| found.push(node));
        found.sort_unstable();
        found.dedup();
        found.retain(|&node| self.overlaps(node, rect));
        found
    }

    /// Each body [`Self::within`] finds, passed to `f` as it is found and
    /// nothing collected: a body in more than one of the cells tried is
    /// passed once for each, in no set order. For a caller that asks of
    /// one small box after another (#332). O(the bodies tried).
    pub(crate) fn each_within(&self, rect: Rect, mut f: impl FnMut(NodeId)) {
        self.candidates(rect, |node| {
            if self.overlaps(node, rect) {
                f(node);
            }
        });
    }

    /// The bodies `rect`'s cells hold, and the large ones — or every
    /// body, for a `rect` spanning more than [`MAX_CELLS`] cells — passed
    /// to `f`, a body in more than one of those cells once for each.
    fn candidates(&self, rect: Rect, mut f: impl FnMut(NodeId)) {
        match cells(rect) {
            Some(((x0, y0), (x1, y1))) => {
                self.large.iter().copied().for_each(&mut f);
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        #[cfg(test)]
                        self.within_cells.set(self.within_cells.get() + 1);
                        self.cells
                            .get(&(x, y))
                            .into_iter()
                            .flatten()
                            .copied()
                            .for_each(&mut f);
                    }
                }
            }
            None => self.bodies.keys().copied().for_each(f),
        }
    }

    /// Whether `self` and `other` hold the same bodies — each rectangle,
    /// shape and cell — whatever order they were set in: what the oracle
    /// compares with a full projection's regions (#332). `Err` names the
    /// first difference. Test builds only.
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &BodyIndex) -> Result<(), String> {
        use crate::projection::behind::bits;
        let bodies = |b: &BodyIndex| {
            let mut v: Vec<_> = b
                .bodies
                .iter()
                .map(|(&n, &(r, shape, large))| (n, bits(r), shape == NodeShape::Ellipse, large))
                .collect();
            v.sort_unstable();
            v
        };
        let cells = |b: &BodyIndex| {
            let mut v: Vec<_> = b
                .cells
                .iter()
                .flat_map(|(&c, list)| list.iter().map(move |&n| (c, n)))
                .chain(b.large.iter().map(|&n| ((i32::MIN, i32::MIN), n)))
                .collect();
            v.sort_unstable();
            v
        };
        if bodies(self) != bodies(other) {
            return Err(format!(
                "the bodies differ: kept {:?}, in full {:?}",
                bodies(self).len(),
                bodies(other).len()
            ));
        }
        if cells(self) != cells(other) {
            return Err("the bodies' cells differ".to_string());
        }
        Ok(())
    }

    /// Whether `rect` overlaps `node`'s body by its shape, inclusively.
    fn overlaps(&self, node: NodeId, rect: Rect) -> bool {
        #[cfg(test)]
        self.within_tried.set(self.within_tried.get() + 1);
        self.bodies.get(&node).is_some_and(|&(body, shape, _)| {
            let min = body.origin;
            shape.intersects_local_aabb(rect.origin - min, rect.max() - min, body.max() - min)
        })
    }
}

impl Projector {
    /// The visible nodes whose bodies `rect`, on the canvas, overlaps:
    /// what a rubber-band selection covers. A body is the node's drawn
    /// rectangle (D5) shaped by its outline — the area [`Self::hit`]
    /// falls back to — and not its frame's rings, which are glyphs set
    /// outside the outline. Inclusive of edges, in ascending node id; see
    /// `BodyIndex::within` for the zero-area and zero-size cases. Cost:
    /// the body index's cells under `rect`, or every body for a
    /// rectangle spanning more cells than the index lists per body.
    pub fn nodes_in(&self, rect: Rect) -> Vec<NodeId> {
        self.bodies.within(rect)
    }

    /// The smallest visible node whose body holds `point` by its shape —
    /// what [`Self::hit`] falls back to, asked directly, so a path or a
    /// label drawn over a node does not stand in for it: the node a
    /// reparent or a connect lands on. Cost: a cell of the body index.
    pub fn node_at(&self, point: Vec2) -> Option<NodeId> {
        self.bodies.at(point)
    }

    /// What `point`, on the canvas, hits (#241 §6, ruling B7.1): the first
    /// layer, top-down, whose glyph BVH finds a glyph element there — a
    /// path run only where one of its glyphs is drawn — mapped back
    /// through the reverse map to its element and, through the run's
    /// glyph-to-span mapping, to its span; else the smallest visible
    /// node whose body holds the point by its shape; else nothing.
    ///
    /// `layers` is `&mut` because each layer's BVH memo is filled on the
    /// first query after a change. Cost: a BVH descent per layer tried,
    /// O(the glyphs of the run hit), and a cell of the body index.
    pub fn hit(&self, layers: &mut GlyphLayers, point: Vec2) -> Option<Hit> {
        for layer in Layer::HIT_ORDER {
            let Some(glyph) = layers.tree_mut(layer).descendant_at(point) else {
                continue;
            };
            let Some(Owner::Run { unit, slot }) = self.owners.get(&(layer, glyph)).copied() else {
                continue;
            };
            let Some(u) = self.units.get(&unit) else {
                continue;
            };
            let Some(s) = u.slots.get(slot) else {
                continue;
            };
            let element = s.element.unwrap_or(unit);
            let (index, span) = match s.written.as_ref() {
                Some(run) => glyph_and_span(run, point),
                None => (None, None),
            };
            return Some(Hit {
                node: u.node,
                element,
                span: span.and_then(|i| s.spans.get(i).copied()),
                glyph: index,
            });
        }
        self.bodies.at(point).map(|node| Hit {
            node,
            element: node,
            span: None,
            glyph: None,
        })
    }
}

/// The glyph of `run` under `point`, and the index of its span in the
/// run's flow: a path run's placed glyph whose box holds it, a box run's
/// laid-out glyph whose cell of its line holds it. A filler names no span.
fn glyph_and_span(
    run: &crate::gfx_structs::run::GlyphRunData,
    point: Vec2,
) -> (Option<usize>, Option<usize>) {
    match run.shaped() {
        RunShape::Path { run: shaped, placed } => {
            let Some(index) = run.placed_glyph_at(point, 0.0) else {
                return (None, None);
            };
            let span = placed
                .glyphs
                .get(index)
                .and_then(|g| shaped.glyphs.get(g.glyph as usize))
                .map(|m| m.span)
                .filter(|&s| s != FILLER_SPAN);
            (Some(index), span)
        }
        RunShape::Box(shaped) => {
            let local = point - run.origin();
            let mut index = 0;
            for line in shaped.buffer.layout_runs() {
                let top = line.line_top;
                let inside = local.y >= top && local.y <= top + line.line_height;
                for glyph in line.glyphs.iter() {
                    if inside && local.x >= glyph.x && local.x <= glyph.x + glyph.w {
                        let span = shaped
                            .glyph_spans
                            .get(index)
                            .map(|(s, _)| *s)
                            .filter(|&s| s != FILLER_SPAN);
                        return (Some(index), span);
                    }
                    index += 1;
                }
            }
            (None, None)
        }
    }
}
