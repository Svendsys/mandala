// SPDX-License-Identifier: MPL-2.0

//! Nodes hide the paths behind them (#332): a node's contents take
//! priority over every connector and line link that does not end at it.
//!
//! A glyph of a path's body or caps is left out of its run where its box
//! — its ink, or its advance by its line height where it has none —
//! grown by the path's own gap meets the **region** of a visible node the
//! path does not end at: the box round the node's drawn rectangle and its
//! rings' inks (the reach of #296, with the rectangle always in it),
//! shaped by its outline. So a path shown on either side of a node stops
//! one gap short of it, as it stops one gap short of its own frames. A
//! glyph is drawn whole or not at all, never clipped. Its label and
//! portal ends are not paths' glyphs and are not hidden. A node hides
//! whatever its zoom window and opacity: zoom windows are evaluated when
//! drawing (§12.4), and a run's glyphs share one window.
//!
//! Hiding is a view decision: the document keeps the path, the
//! projection keeps every glyph as laid (`LineState::laid`), and a glyph
//! comes back when the node moves off it.
//!
//! # What runs again
//!
//! Two indices on the body index's grid keep this proportional to what
//! moved: every visible node's region (a [`BodyIndex`]), and where each
//! drawn path's glyphs are ([`PathCells`]). A path's hide stage, run once
//! every path is laid and every ring placed, tries each of its glyphs
//! against the regions in that glyph's cells, and records reading the
//! geometry and reach of each node it meets. So a node that moves, is
//! reshaped or leaves marks exactly the paths with a glyph its old region
//! met, through the dependency index. Each node whose geometry or reach
//! was marked is then set again in the region index, and finds the paths
//! with a glyph its new region meets in the paths' index, in the cells
//! that region covers. A path laid again is hidden again.

use rustc_hash::FxHashMap;

use crate::arena::NodeId;
use crate::flow::{PlacedRun, Rect};
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use crate::projection::geometry::node_shape;
use crate::projection::hit::cells;
use crate::projection::input::{Dependent, ProjectionInput, Stage};
use crate::projection::stages::{gap_and_size, glyph_ink, PhaseWork};
use crate::projection::state::UnitBody;
use crate::projection::Projector;

/// A grid cell, as the body index numbers them.
type Cell = (i32, i32);

/// Where each drawn connector's and line link's glyphs are (#332): the
/// cells of the body index's grid that its glyphs' boxes, grown by its
/// gap, cover, and in each cell the box round those of its glyphs there.
/// A path is set whole each time it is hidden again, and a node's region
/// finds it through the cells the region covers.
#[derive(Debug, Default)]
pub(crate) struct PathCells {
    /// Each path's cells.
    of: FxHashMap<NodeId, Vec<Cell>>,
    /// The paths with a glyph in each cell, each with the box round its
    /// glyphs there.
    cells: FxHashMap<Cell, Vec<(NodeId, Rect)>>,
    /// The paths with a glyph too large for the cells, each with the box
    /// round those glyphs: tried by every query.
    large: FxHashMap<NodeId, Rect>,
}

/// Whether `a` and `b` meet, edges included.
fn meet(a: &Rect, b: &Rect) -> bool {
    a.origin.x <= b.max().x && b.origin.x <= a.max().x && a.origin.y <= b.max().y && b.origin.y <= a.max().y
}

impl PathCells {
    /// Set where `path`'s glyphs are: `boxes`, each a glyph's box grown by
    /// the path's gap. A box that is not finite is left out. O(its old and
    /// new cells, and the paths sharing them).
    pub(crate) fn set(&mut self, path: NodeId, boxes: &[Rect]) {
        self.remove(path);
        let mut here: FxHashMap<Cell, Rect> = FxHashMap::default();
        for b in boxes {
            if !b.origin.is_finite() || !b.size.is_finite() {
                continue;
            }
            match cells(*b) {
                Some(((x0, y0), (x1, y1))) => {
                    for x in x0..=x1 {
                        for y in y0..=y1 {
                            here.entry((x, y)).and_modify(|r| *r = r.union(b)).or_insert(*b);
                        }
                    }
                }
                None => {
                    self.large
                        .entry(path)
                        .and_modify(|r| *r = r.union(b))
                        .or_insert(*b);
                }
            }
        }
        let mut of = Vec::with_capacity(here.len());
        for (cell, rect) in here {
            self.cells.entry(cell).or_default().push((path, rect));
            of.push(cell);
        }
        if !of.is_empty() {
            self.of.insert(path, of);
        }
    }

    /// Take `path` out. O(its cells, and the paths sharing them).
    pub(crate) fn remove(&mut self, path: NodeId) {
        self.large.remove(&path);
        for cell in self.of.remove(&path).into_iter().flatten() {
            if let Some(list) = self.cells.get_mut(&cell) {
                list.retain(|&(p, _)| p != path);
                if list.is_empty() {
                    self.cells.remove(&cell);
                }
            }
        }
    }

    /// Each path with glyphs whose box, in one of `rect`'s cells, `rect`
    /// meets, passed to `f` — once for each such cell. A `rect` spanning
    /// more cells than the body index lists tries every cell. O(the
    /// entries tried).
    pub(crate) fn each_meeting(&self, rect: Rect, mut f: impl FnMut(NodeId)) {
        let mut each = |list: &[(NodeId, Rect)]| {
            for (path, b) in list {
                if meet(b, &rect) {
                    f(*path);
                }
            }
        };
        for (&path, b) in &self.large {
            each(&[(path, *b)]);
        }
        match cells(rect) {
            Some(((x0, y0), (x1, y1))) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        each(self.cells.get(&(x, y)).map_or(&[], Vec::as_slice));
                    }
                }
            }
            None => self.cells.values().for_each(|list| each(list)),
        }
    }
}

impl Projector {
    /// Phase D for an element whose geometry or reach was marked: a
    /// visible node's region set again in the region index, and the
    /// paths with a glyph the new region meets queued to be hidden again
    /// — those its old region met read its geometry, and were marked with
    /// it. Anything else leaves the index: a node no longer visible, or
    /// an element that is no node, which was never in it. O(the region's
    /// cells in both indices).
    pub(crate) fn rehide(&mut self, id: NodeId, work: &mut PhaseWork) {
        let Some((rect, shape)) = self.nodes.get(&id).map(|n| (n.rect, n.shape)) else {
            self.regions.remove(id);
            return;
        };
        let region = self.reach(id).map_or(rect, |reach| reach.union(&rect));
        self.regions.set(id, region, node_shape(shape));
        self.path_cells.each_meeting(region, |path| {
            self.queue(work, Dependent::new(path, Stage::Hide));
        });
    }

    /// Phase D for a drawn connector or line link: its body's and caps'
    /// runs as laid, less every glyph whose box, grown by the path's gap,
    /// meets the region of a node it does not end at. Records reading the
    /// geometry and reach of each such node, and sets where its glyphs
    /// are. A path not drawn has nothing hidden, and leaves the paths'
    /// index.
    ///
    /// Cost: O(its glyphs, and the regions in each glyph's cells).
    pub(crate) fn hide(&mut self, id: NodeId) {
        let Some(UnitBody::Line(line)) = self.units.get(&id).map(|u| &u.body) else {
            return;
        };
        let Some((from, to)) = line.ends.filter(|_| line.drawn) else {
            self.unhide(id);
            return;
        };
        let gap = gap_and_size(line).0.max(0.0);
        let mut reads = Vec::new();
        let mut boxes = Vec::new();
        let wants = line.laid.clone().map(|laid| {
            let run = laid?;
            let RunShape::Path { run: shaped, placed } = run.shaped() else {
                return Some(run);
            };
            let mut shown = Vec::with_capacity(placed.glyphs.len());
            for g in &placed.glyphs {
                let Some(ink) = glyph_ink(shaped, g) else {
                    shown.push(*g);
                    continue;
                };
                let grown = ink.grown(gap);
                boxes.push(grown);
                let mut hidden = false;
                self.regions.each_within(grown, |node| {
                    if node != from && node != to {
                        reads.push(ProjectionInput::Geometry(node));
                        reads.push(ProjectionInput::Reach(node));
                        hidden = true;
                    }
                });
                if !hidden {
                    shown.push(*g);
                }
            }
            if shown.len() == placed.glyphs.len() {
                return Some(run);
            }
            let placed = std::sync::Arc::new(PlacedRun {
                glyphs: shown,
                truncated: placed.truncated,
            });
            GlyphRunData::new(
                RunShape::Path {
                    run: shaped.clone(),
                    placed,
                },
                run.origin(),
                GlyphOverrides::none(),
                run.opacity(),
                run.zoom(),
            )
            .ok()
        });
        self.deps.record(Dependent::new(id, Stage::Hide), &reads);
        self.path_cells.set(id, &boxes);
        if let Some(unit) = self.units.get_mut(&id) {
            for (slot, want) in unit.slots.iter_mut().zip(wants) {
                slot.want = want;
            }
        }
    }

    /// Forget what the path `id` hid: what its hide stage read, where its
    /// glyphs were, and its runs as laid — for a path no longer drawn as a
    /// line. O(its reads and cells).
    pub(crate) fn unhide(&mut self, id: NodeId) {
        self.deps.clear(Dependent::new(id, Stage::Hide));
        self.path_cells.remove(id);
        if let Some(UnitBody::Line(line)) = self.units.get_mut(&id).map(|u| &mut u.body) {
            line.laid = Default::default();
        }
    }
}
