// SPDX-License-Identifier: MPL-2.0

//! Nodes hide the paths behind them (#332): a node's contents take
//! priority over every connector and line link that does not end at it.
//!
//! A glyph of a path's body or caps is left out of its run where its box
//! — its ink, or its advance by its line height where it has none —
//! grown by the path's own gap meets the **region** of a visible node the
//! path does not end at ([`Projector::region`]): what the node draws, in
//! two parts — the box round its sections' glyph cells, a rectangle, and
//! the box round its rings' inks, shaped by its outline — not its drawn
//! rectangle (the owner's answer on #334). So a path shown on either side
//! of a node stops
//! at least one gap short of it, as it stops one gap short of its own
//! frames. A glyph is drawn whole or not at all, never clipped. Its label
//! is hidden whole where the box round its text meets such a region (the
//! owner's answer on #334), its own two nodes sparing it as they spare its
//! glyphs; a portal end is never hidden.
//!
//! A node hides a path only where it is drawn wherever the path is: at an
//! opacity above 0, in a zoom window that holds the path's
//! ([`drawn_wherever`](crate::projection::behind::drawn_wherever); the
//! owner's answer on #334). Zoom windows are
//! evaluated when drawing (§12.4) and a run's glyphs share one window, so
//! a glyph cannot be hidden only while the node shows, and a gap with
//! nothing over it reads as a break in the path. A path whose two nodes
//! lie inside a third node's ringed region is hidden whole.
//!
//! Hiding is a view decision: the document keeps the path, the
//! projection keeps every glyph as laid (`LineState::laid`), and a glyph
//! comes back when the node moves off it.
//!
//! # What runs again
//!
//! Two indices on the body index's grid keep this proportional to what
//! moved: every visible node's region
//! ([`Regions`](crate::projection::behind::Regions), a [`BodyIndex`] for
//! each part), and where each drawn path's glyphs and label are
//! ([`PathCells`](crate::projection::behind::PathCells)). A path's hide
//! stage, run once every path is laid and every ring and section placed,
//! tries each of its glyphs, and its label's text, against the regions in
//! their cells, and records reading the region, opacity and zoom window of
//! each node it meets. Each node whose region may have moved — its
//! geometry or reach marked, a section of it placed elsewhere or dropped —
//! has its region found again; if it changed, it is set again in the
//! region index, its [`ProjectionInput::Region`] is marked, which marks
//! exactly the paths with a glyph or a label its old region met, and it
//! finds the paths with a glyph or a label its new region meets in the
//! paths' index, in the cells that region covers. A path laid again is
//! hidden again.
//!
//! A path is hidden again **whole**: the paths a moved node meets are the
//! unit of work, not the glyphs near it. So each frame of a drag across a
//! long edge tries and indexes every glyph of that edge, and its label,
//! again — O(the edge's length), not O(the glyphs under the node).

use rustc_hash::FxHashMap;

use crate::arena::NodeId;
use crate::document::values::ZoomWindow;
use crate::flow::{PlacedRun, Rect};
use crate::gfx_structs::run::RunShape;
use crate::gfx_structs::shape::NodeShape;
use crate::projection::geometry::node_shape;
use crate::projection::hit::{cell_count, cells, span, BodyIndex};
use crate::projection::input::{Dependent, ProjectionInput, Stage};
use crate::projection::stages::{gap_and_size, glyph_ink, run_data, PhaseWork};
use crate::projection::state::UnitBody;
use crate::projection::Projector;

/// A grid cell, as the body index numbers them.
type Cell = (i32, i32);

/// Where each drawn connector's and line link's glyphs and label are
/// (#332): the cells of the body index's grid that its glyphs' boxes,
/// grown by its gap, and its label's text box, not grown, cover, and in
/// each cell the box round those of them there. A path is set whole each
/// time it is hidden again, and a node's region finds it through the
/// cells the region covers.
///
/// Each cell's entry names its place in its path's list, and each place
/// in a path's list names its entry's index in the cell — the dependency
/// index's shape (`DepIndex`) — so a path leaves a cell in O(1) however
/// many paths share it, as every path leaving one hub does.
#[derive(Debug, Default)]
pub(crate) struct PathCells {
    /// Each path's cells, each with its entry's index in that cell's list.
    of: FxHashMap<NodeId, Vec<(Cell, u32)>>,
    /// The paths with a glyph in each cell: each path, the box round its
    /// glyphs there, and the entry's place in the path's list.
    cells: FxHashMap<Cell, Vec<(NodeId, Rect, u32)>>,
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
    /// new cells).
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
            let list = self.cells.entry(cell).or_default();
            of.push((cell, list.len() as u32));
            list.push((path, rect, (of.len() - 1) as u32));
        }
        if !of.is_empty() {
            self.of.insert(path, of);
        }
    }

    /// Take `path` out: its entry swapped out of each of its cells, the
    /// entry that takes its place pointed at its new index. O(its cells).
    pub(crate) fn remove(&mut self, path: NodeId) {
        self.large.remove(&path);
        for (cell, at) in self.of.remove(&path).into_iter().flatten() {
            let Some(list) = self.cells.get_mut(&cell) else {
                continue;
            };
            let at = at as usize;
            list.swap_remove(at);
            if let Some(&(moved, _, place)) = list.get(at) {
                if let Some(entry) = self.of.get_mut(&moved).and_then(|o| o.get_mut(place as usize)) {
                    entry.1 = at as u32;
                }
            }
            if list.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    /// Each path with glyphs whose box, in one of `rect`'s cells, `rect`
    /// meets, passed to `f` — once for each such cell. `rect`'s own cells
    /// are walked when they are fewer than the cells that hold a path, and
    /// those are scanned otherwise. O(the lesser of the two, and the
    /// entries tried).
    pub(crate) fn each_meeting(&self, rect: Rect, mut f: impl FnMut(NodeId)) {
        let mut each = |list: &[(NodeId, Rect, u32)]| {
            for (path, b, _) in list {
                if meet(b, &rect) {
                    f(*path);
                }
            }
        };
        for (&path, b) in &self.large {
            each(&[(path, *b, 0)]);
        }
        match span(rect) {
            Some(((x0, y0), (x1, y1))) if cell_count((x0, y0), (x1, y1)) <= self.cells.len() as i64 => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        each(self.cells.get(&(x, y)).map_or(&[], Vec::as_slice));
                    }
                }
            }
            Some(_) => self.cells.values().for_each(|list| each(list)),
            None => {}
        }
    }

    /// Whether `self` and `other` hold the same paths in the same cells,
    /// each with the same box, whatever order they were set in: what the
    /// oracle compares with a full projection's. `Err` names the first
    /// difference. Test builds only.
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &PathCells) -> Result<(), String> {
        fn sets(p: &PathCells) -> Vec<(NodeId, Cell, [u32; 4])> {
            let mut out: Vec<_> = p
                .cells
                .iter()
                .flat_map(|(&cell, list)| list.iter().map(move |&(path, r, _)| (path, cell, bits(r))))
                .collect();
            out.sort_unstable();
            out
        }
        let large = |p: &PathCells| {
            let mut v: Vec<_> = p.large.iter().map(|(&path, &r)| (path, bits(r))).collect();
            v.sort_unstable();
            v
        };
        first_difference("cells (path, cell, box bits)", &sets(self), &sets(other))?;
        first_difference("large paths (path, box bits)", &large(self), &large(other))?;
        for (path, of) in &self.of {
            for (k, &(cell, at)) in of.iter().enumerate() {
                let entry = self.cells.get(&cell).and_then(|l| l.get(at as usize));
                if entry.map(|e| (e.0, e.2)) != Some((*path, k as u32)) {
                    return Err(format!("{path:?}'s entry {k} in {cell:?} points at {entry:?}"));
                }
            }
        }
        Ok(())
    }
}

/// `Ok` when `kept` and `full`, two sorted lists of what an index holds,
/// are equal; else `Err` naming `what` and the first entry they differ at,
/// as each holds it there. Test builds only.
#[cfg(test)]
pub(crate) fn first_difference<T: PartialEq + std::fmt::Debug>(
    what: &str,
    kept: &[T],
    full: &[T],
) -> Result<(), String> {
    if kept == full {
        return Ok(());
    }
    let at = kept
        .iter()
        .zip(full)
        .position(|(a, b)| a != b)
        .unwrap_or(kept.len().min(full.len()));
    Err(format!(
        "{what} differ first at entry {at}: kept {:?}, in full {:?} ({} kept, {} in full)",
        kept.get(at),
        full.get(at),
        kept.len(),
        full.len()
    ))
}

/// A rectangle's four floats, by their bits: what the oracle's set
/// comparisons order and compare. Test builds only.
#[cfg(test)]
pub(crate) fn bits(r: Rect) -> [u32; 4] {
    [
        r.origin.x.to_bits(),
        r.origin.y.to_bits(),
        r.size.x.to_bits(),
        r.size.y.to_bits(),
    ]
}

/// A node's region (#332), in its two parts: the box round its text, a
/// rectangle, and the box round its rings' inks with the shape of its
/// outline, which they run round. Either is `None` where the node draws
/// none of it, or none of any area.
pub(crate) type Region = (Option<Rect>, Option<(Rect, NodeShape)>);

/// Every visible node's region, each part in a body index of its own: the
/// text part as a rectangle, the rings part by the node's outline — so
/// the corners of an ellipse's text, outside the ellipse, hide as the rest
/// of its text does, and the empty corners of its rings' box do not.
#[derive(Debug, Default)]
pub(crate) struct Regions {
    /// Each node's text part.
    text: BodyIndex,
    /// Each node's rings part.
    rings: BodyIndex,
}

impl Regions {
    /// `node`'s region as the index holds it. O(1).
    pub(crate) fn get(&self, node: NodeId) -> Region {
        (self.text.get(node).map(|(r, _)| r), self.rings.get(node))
    }

    /// Set `node`'s region: each part set, or taken out where it is
    /// `None`. O(the cells of the old and new parts).
    pub(crate) fn set(&mut self, node: NodeId, (text, rings): Region) {
        match text {
            Some(rect) => self.text.set(node, rect, NodeShape::Rectangle),
            None => self.text.remove(node),
        }
        match rings {
            Some((rect, shape)) => self.rings.set(node, rect, shape),
            None => self.rings.remove(node),
        }
    }

    /// Each node with a part of its region `rect` overlaps by that part's
    /// shape, passed to `f` — once for each part and each cell it is found
    /// in ([`BodyIndex::each_within`]). O(the bodies tried in both parts).
    pub(crate) fn each_within(&self, rect: Rect, mut f: impl FnMut(NodeId)) {
        self.text.each_within(rect, &mut f);
        self.rings.each_within(rect, f);
    }

    /// Whether `self` and `other` hold the same regions, part by part:
    /// what the oracle compares with a full projection's. `Err` names the
    /// part and the first entry they differ at. Test builds only.
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &Regions) -> Result<(), String> {
        self.text
            .same_as(&other.text)
            .map_err(|d| format!("text part: {d}"))?;
        self.rings
            .same_as(&other.rings)
            .map_err(|d| format!("rings part: {d}"))
    }
}

impl Projector {
    /// The region `node` hides paths' glyphs by (#332), in its two parts:
    /// the box round its sections' text, each glyph's cell, as a rectangle;
    /// and the box round its rings' inks, shaped by its outline, which the
    /// rings run round. A part the node draws nothing of, or nothing of any
    /// area, is `None`; a node with neither hides nothing. O(its units).
    pub(crate) fn region(&self, node: NodeId) -> Region {
        let Some(n) = self.nodes.get(&node) else {
            return (None, None);
        };
        let (mut text, mut inks): (Option<Rect>, Option<Rect>) = (None, None);
        let join = |a: Option<Rect>, b: Option<Rect>| match (a, b) {
            (Some(a), Some(b)) => Some(a.union(&b)),
            (a, b) => a.or(b),
        };
        for u in &n.units {
            match self.units.get(u).map(|u| &u.body) {
                Some(UnitBody::Ring(ring)) => inks = join(inks, ring.ink),
                Some(UnitBody::Section(section)) => text = join(text, section.drawn),
                _ => {}
            }
        }
        let some = |r: Option<Rect>| r.filter(|r| r.size.x > 0.0 && r.size.y > 0.0);
        (some(text), some(inks).map(|r| (r, node_shape(n.shape))))
    }

    /// Phase D for an element whose region may have moved — a node whose
    /// geometry or reach was marked, or one a section of which was placed
    /// elsewhere or dropped: its region set again in the region index if it
    /// changed, the paths that met the old one marked through
    /// [`ProjectionInput::Region`], which they read, and those with a glyph
    /// or a label the new one meets queued to be hidden again. A node no
    /// longer visible, or one that now draws nothing, leaves the index; an
    /// element that is no node was never in it. O(its units, and the new
    /// region's cells in the paths' index).
    pub(crate) fn rehide(&mut self, id: NodeId, work: &mut PhaseWork) {
        let region = self.region(id);
        if self.regions.get(id) == region {
            return;
        }
        self.regions.set(id, region);
        self.mark(work, ProjectionInput::Region(id));
        for rect in [region.0, region.1.map(|(r, _)| r)].into_iter().flatten() {
            self.path_cells.each_meeting(rect, |path| {
                self.queue(work, Dependent::new(path, Stage::Hide));
            });
        }
    }

    /// Phase D for a drawn connector or line link: its body's and caps'
    /// runs as laid, less every glyph whose box, grown by the path's gap,
    /// meets the region of a node it does not end at that is drawn wherever
    /// the path is — at an opacity above 0, in a zoom window that holds the
    /// path's ([`drawn_wherever`]) — and its label as laid, left out whole
    /// where the box round its text meets such a region. Records reading
    /// the region, opacity and zoom window of each node it meets, and sets
    /// where its glyphs and its label are. A path not drawn has nothing
    /// hidden, and leaves the paths' index.
    ///
    /// Cost: O(its glyphs and its label's, and the regions in each glyph's
    /// cells and its label's).
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
        // Whether `rect`, of a run in `zoom`, meets the region of a node the
        // path does not end at that is drawn wherever the run is; each node
        // met is read.
        let mut behind = |rect: Rect, zoom: Option<ZoomWindow>| {
            let mut hidden = false;
            self.regions.each_within(rect, |node| {
                if node == from || node == to {
                    return;
                }
                reads.push(ProjectionInput::Region(node));
                reads.push(ProjectionInput::Opacity(node));
                reads.push(ProjectionInput::Visibility(node));
                if self
                    .nodes
                    .get(&node)
                    .is_some_and(|n| n.opacity > 0.0 && drawn_wherever(n.zoom, zoom))
                {
                    hidden = true;
                }
            });
            hidden
        };
        let wants = line.laid.clone().map(|laid| {
            let run = laid?;
            let (shaped, placed) = match run.shaped() {
                RunShape::Path { run: shaped, placed } => (shaped, placed),
                // The label: hidden whole where the box round its text
                // meets a node's region (the owner's answer on #334).
                RunShape::Box(label) => {
                    let Some(text) = label.text_box() else {
                        return Some(run);
                    };
                    let drawn = Rect::new(run.origin() + text.origin, text.size);
                    boxes.push(drawn);
                    return (!behind(drawn, run.zoom())).then_some(run);
                }
            };
            let mut shown = Vec::with_capacity(placed.glyphs.len());
            for g in &placed.glyphs {
                let Some(ink) = glyph_ink(shaped, g) else {
                    shown.push(*g);
                    continue;
                };
                let grown = ink.grown(gap);
                boxes.push(grown);
                if !behind(grown, run.zoom()) {
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
            run_data(
                RunShape::Path {
                    run: shaped.clone(),
                    placed,
                },
                run.origin(),
                run.opacity(),
                run.zoom(),
            )
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

/// Whether a node in the zoom window `node` is drawn at every zoom a path
/// in `path` is: `node` holds `path`, an open bound holding any. A node
/// that is not hides nothing of the path (the owner's answer on #334): a
/// run has one zoom window, so a glyph cannot be hidden only while the
/// node shows, and a gap at a zoom where nothing is drawn over it reads
/// as a break in the path. O(1).
pub(crate) fn drawn_wherever(node: Option<ZoomWindow>, path: Option<ZoomWindow>) -> bool {
    let Some(node) = node else {
        return true;
    };
    let path = path.unwrap_or(ZoomWindow { min: None, max: None });
    let low = match (node.min, path.min) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(n), Some(p)) => n <= p,
    };
    let high = match (node.max, path.max) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(n), Some(p)) => n >= p,
    };
    low && high
}
