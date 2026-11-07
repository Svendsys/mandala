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

/// The cells `rect` covers, when they are few enough to list.
fn cells(rect: Rect) -> Option<((i32, i32), (i32, i32))> {
    if !rect.origin.is_finite() || !rect.size.is_finite() {
        return None;
    }
    let (x0, y0) = (cell_of(rect.origin.x), cell_of(rect.origin.y));
    let (x1, y1) = (cell_of(rect.max().x), cell_of(rect.max().y));
    let count = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1);
    (count <= MAX_CELLS).then_some(((x0, y0), (x1, y1)))
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
}

impl Projector {
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
