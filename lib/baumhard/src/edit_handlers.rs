// SPDX-License-Identifier: MPL-2.0

//! Edit handlers: layouts the mutator language cannot spell (design
//! §10.1), each a function that **reads** the document and the drawn
//! geometry and **returns** its walks: a mutator tree and its anchors
//! per write it needs.
//!
//! A handler is given `&Document`, so it cannot write it — the borrow
//! checker refuses:
//!
//! ```compile_fail
//! use baumhard::document::ids::Document;
//! use baumhard::document::writer::{ChangeSource, EditRequest};
//! fn sneaky(doc: &Document) {
//!     let _ = doc.apply(EditRequest::new(Vec::new(), "write"), ChangeSource::Edit);
//! }
//! ```
//!
//! What it returns goes through `Document::apply` (or an animated
//! transaction) like every other change, which is what gives every
//! handler undo, animation and change events with nothing of its own.
//!
//! Sizes are derived (D5), so a layout reads them where they are drawn:
//! [`Geometry`](crate::projection::Geometry), the projector's state.
//! That is reading derived geometry, not reading glyph layers to learn
//! what the document is. A node the projection does not draw (hidden,
//! under a fold) has no rectangle, and a layout leaves it where it is.
//!
//! `flower` and `cascade` start from `main`'s
//! `document/mutations/flower_layout.rs` and `tree_cascade.rs` —
//! angles, radius, rows and gaps, breadth-first order — and place
//! differently in two ways:
//!
//! - **A placed node never overlaps another drawn node** (owner, on
//!   #250): not a sibling, not the parent, not anything else on the map.
//!   A flower keeps its shape: one radius and one spacing for every
//!   petal, on the full circle where that is clear and else on the
//!   widest clear fan, its lines clear of other nodes too, and no petal
//!   on its parent's own connector or on a line or connector the run
//!   keeps for the families laid before it (the order, what is kept and
//!   the bounds are on [`flower`](crate::edit_handlers::flower)); only
//!   when no such arrangement is clear does each petal move outward
//!   along its own ray. A cascade row moves down as a whole, keeping its
//!   spacing, trying at each depth straight below, then right and left
//!   by growing steps
//!   ([`MAX_SIDE_STEPS`](crate::edit_handlers::MAX_SIDE_STEPS)). The
//!   distances tried are [`probes`](crate::edit_handlers::probes):
//!   [`MAX_CLEAR_STEPS`](crate::edit_handlers::MAX_CLEAR_STEPS) steps of
//!   [`CLEAR_STEP`](crate::edit_handlers::CLEAR_STEP), then doubling
//!   steps out to twice the canvas bound, where every probe past the
//!   drawn nodes meets nothing. What is drawn is asked of the projector's
//!   body index (`Geometry::nodes_meeting`), never scanned: a run keeps
//!   only the set of nodes it moves, the rectangles it has placed and
//!   the segments it keeps, so its cost follows the families it moves,
//!   not the map. Only a layout keeps clear: a node moved by hand goes
//!   where it is put.
//! - A child with no rectangle has no size to lay out by (D5), so it
//!   takes no slot: one hidden child of twelve makes an eleven-petal
//!   flower, and a cascade row without it. `main` gave it one.
//!
//! Each placement is emitted as one `NodeOp::SetPosition` per node, each
//! its own walk: a walk applies one mutator per sibling key, and sibling
//! nodes commonly share one, so one walk could not place them apart
//! (ruling on #250).
//!
//! A handler that only reads compiles, the control for the refusal
//! above:
//!
//! ```
//! use baumhard::document::ids::Document;
//! fn reads(doc: &Document) -> usize {
//!     doc.children(doc.root()).count()
//! }
//! ```

use crate::arena::NodeId;
use crate::document::bounds::{clamp_canvas_coord, MAX_CANVAS_COORD};
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::Document;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use crate::engine::anchor::{in_tree_order, Anchors};
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tree::MutatorTree;
use crate::flow::Rect;
use crate::projection::Geometry;
use glam::Vec2;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{HashMap, VecDeque};

/// What a handler returns: its walks, each a mutator and its anchors,
/// in order. They are the steps of the edit's one request, so they are
/// one journal and one history entry (ruling on #250).
pub type Walks = Vec<(DocMutatorTree, Anchors)>;

/// A handler: given the document, where it is drawn and the nodes the
/// edit reached, the walks to run. `Err` says why it declined — the
/// refusal names the edit ([`EditError::Handler`](crate::document::edit::EditError::Handler))
/// — and nothing is applied.
pub type EditHandler = fn(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String>;

/// The handler registry, by name.
pub type Handlers = HashMap<&'static str, EditHandler>;

/// Every handler this build has, by the name an edit's `handler` gives.
pub const HANDLERS: [(&str, EditHandler); 2] = [("flower", flower), ("cascade", cascade)];

/// The registry: every handler in [`HANDLERS`]. What the app fills at
/// startup, on both targets.
pub fn registry() -> Handlers {
    HANDLERS.into_iter().collect()
}

/// Whether this build has a handler called `name`. O(handlers).
pub fn exists(name: &str) -> bool {
    HANDLERS.iter().any(|(known, _)| *known == name)
}

/// The space between a parent's outline and its children's, round a
/// flower.
pub const RADIAL_PADDING: f32 = 40.0;

/// Where a flower's first child goes: straight up (`0` is right, `π/2`
/// down).
pub const START_ANGLE: f32 = -std::f32::consts::FRAC_PI_2;

/// The space between a cascade's rows.
pub const ROW_GAP: f32 = 80.0;

/// The space between siblings in a cascade's row.
pub const SIBLING_GAP: f32 = 40.0;

/// How far a layout moves a node at a time while it looks for a place
/// clear of every other drawn node: a petal outward along its ray, a
/// cascade row downward, and a row sideways.
pub const CLEAR_STEP: f32 = 40.0;

/// How many [`CLEAR_STEP`]s a layout tries before its steps double.
pub const MAX_CLEAR_STEPS: u32 = 64;

/// The distances a layout tries a node or a row at, from where it
/// belongs, in order: [`MAX_CLEAR_STEPS`] steps of [`CLEAR_STEP`] from
/// `0`, then doubling from twice the last, out to twice the canvas bound
/// (about nineteen more). Past the drawn nodes a probe meets nothing, so
/// a layout finds a clear place unless a drawn node reaches across the
/// whole of it; then the node stays where it belongs. O(1) per item.
pub fn probes() -> impl Iterator<Item = f32> {
    let linear = MAX_CLEAR_STEPS as f32 * CLEAR_STEP;
    (0..MAX_CLEAR_STEPS)
        .map(|k| k as f32 * CLEAR_STEP)
        .chain(std::iter::successors(Some(2.0 * linear), |d| {
            (*d * 2.0 <= 2.0 * MAX_CANVAS_COORD).then_some(*d * 2.0)
        }))
}

/// How many [`CLEAR_STEP`]s a cascade row tries to each side, at each
/// depth, before it steps down again.
pub const MAX_SIDE_STEPS: u32 = 16;

/// How many radii a regular flower tries, from the base radius out,
/// [`FLOWER_RADIUS_STEP`] apart.
pub const FLOWER_RADII: u32 = 32;

/// The step between the radii a regular flower tries.
pub const FLOWER_RADIUS_STEP: f32 = 20.0;

/// How many rotations of the full circle a flower tries at a radius:
/// the first petal at the top, then turned by an eighth of the spacing
/// between petals at a time.
pub const FLOWER_ROTATIONS: u32 = 8;

/// The directions a fan of petals is centered on, from the top
/// clockwise, evenly round the parent: twelve, one every 30°.
pub const FAN_DIRECTIONS: u32 = 12;

/// The parts of a full turn a fan narrows by: each fan a flower tries is
/// one part ([`FAN_NARROWING`]) narrower than the one before, from the
/// span of the full circle down, so there are fewer than this many fans
/// per direction — at most `FAN_SPANS - 1`.
pub const FAN_SPANS: u32 = 12;

/// How much narrower each fan a flower tries is than the one before: a
/// turn over [`FAN_SPANS`], 30°.
pub const FAN_NARROWING: f32 = std::f32::consts::TAU / FAN_SPANS as f32;

/// The longest piece of a petal's line read from the projector's body
/// index at once: one of the **index's** cells (256 units; the grid of
/// this run's placements in `Clear` has 512-unit cells of its own). A
/// piece's box then covers at most four cells of either grid, so no read
/// falls to the index's scan of every body.
pub const LINE_PIECE: f32 = 256.0;

/// The most pieces a petal's line is read in. A line longer than this
/// many [`LINE_PIECE`]s outside the parent and the petal (16,384 units)
/// is not read, and an arrangement with one is not taken.
pub const MAX_LINE_PIECES: u32 = 64;

/// `flower`: each node's child nodes round it at one radius, evenly
/// spaced, clockwise in sibling order — on the full circle, the first at
/// the top, where that is clear, else on the widest fan that is, the
/// first at its counter-clockwise end (owner, on #250). The base
/// radius clears the parent's half-span and the largest child's, plus
/// [`RADIAL_PADDING`].
///
/// A **regular** arrangement is one radius and one spacing for every
/// petal. It is taken when no petal overlaps a drawn node, a petal
/// already placed or another of its own, and no petal's line — the
/// straight line from the parent's center to the petal's, where it runs
/// outside the two — crosses a drawn node, a placed petal or another of
/// its own (see `Clear`). Nor does a petal meet its parent's own
/// connector from the grandparent (owner, on #331) — the straight line
/// from the grandparent's center to the parent's, where it runs outside
/// the two, the grandparent where this run placed it or else where it is
/// drawn — which is held whole while the family is laid, however long;
/// nor any **segment the run keeps**. Once a family is laid, the run
/// keeps its lines and its parent's connector for every family after it,
/// so a later family keeps clear of every earlier family's parent's
/// connector too, not only of its own. Only these are kept: a connector
/// neither of whose ends the run flowers is never avoided. A kept
/// segment longer than [`MAX_LINE_PIECES`] × [`LINE_PIECE`] (16,384
/// units) is kept only in its two end stretches of that length, beside
/// the nodes it joins; its middle is not kept. The arrangements are
/// tried in this order:
///
/// 1. each radius from the base out, [`FLOWER_RADII`] of them,
///    [`FLOWER_RADIUS_STEP`] apart;
/// 2. at each radius, the full circle at [`FLOWER_ROTATIONS`] rotations;
/// 3. then fans: the full circle's span less [`FAN_NARROWING`], less
///    twice that, and so on while a span is left, each centered in turn
///    on [`FAN_DIRECTIONS`] directions from the top, clockwise, its
///    petals evenly from end to end.
///
/// So the smallest radius that has a clear arrangement wins, and at
/// that radius the full circle, then the widest fan. When none is
/// clear, each petal moves outward along its own ray, by [`probes`],
/// until it is clear of the drawn nodes, the petals before it, its
/// parent's connector and the segments the run keeps.
///
/// Parents are placed before their children, in tree order, and a node
/// one flower moves is the center of its own where it is moved to, so
/// nested nodes compose.
///
/// Cost, for a family of `n` children: at most [`FLOWER_RADII`] ×
/// ([`FLOWER_ROTATIONS`] + [`FAN_DIRECTIONS`] × (`FAN_SPANS` − 1))
/// arrangements, 4,480 with the constants as they are. One arrangement
/// (`arrangement`) costs, at most:
///
/// - O(n²) comparisons of its petals and lines with each other;
/// - per petal, one query of the projector's body index over the
///   petal's cells, a test of its parent's connector, and a lookup of
///   this run's placements and kept segment pieces there
///   (`Clear::is_clear`). A kept segment is at most 2 ×
///   [`MAX_LINE_PIECES`] pieces of at most [`LINE_PIECE`], each filed
///   under at most four of `Clear`'s 512-unit cells, so a petal reads
///   only the pieces kept near it — though segments, unlike placed
///   rectangles, may overlap, so those converging on one node crowd the
///   cells beside it;
/// - per line, at most [`MAX_LINE_PIECES`] queries of the body index,
///   each over at most four cells, a `node_rect` for each node one finds,
///   and as many lookups of the placements.
///
/// It stops at the first petal or line that fails. So a family of twelve
/// whose every arrangement fails at its last line costs at most about
/// 4,480 × 12 × 65 ≈ 3.5 million queries, none of them a scan of every
/// body. Then, only if none is clear, at most the [`probes`] per petal.
/// One exception is the petal itself: a petal larger than the index's
/// cell limit (64 cells of 256 units) is checked by the index's scan of
/// every body (`BodyIndex::query`), and one larger than `Clear`'s (64
/// cells of 512 units) reads every placement of this run, and the kept
/// pieces along its outline or through every cell that holds one,
/// whichever is fewer (`Clear::meets_segment`). Nothing else is
/// proportional to the map.
pub fn flower(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String> {
    let families: Vec<Family> = in_tree_order(doc.tree(), nodes, false)
        .into_iter()
        .filter_map(|parent| {
            let rect = geometry.node_rect(parent)?;
            let children = drawn_children(doc, geometry, parent);
            (!children.is_empty()).then_some((parent, rect, children))
        })
        .collect();
    let mut clear = Clear::new(geometry, families.iter().flat_map(|(_, _, row)| row));
    let mut planned: Vec<(NodeId, Vec2)> = Vec::new();
    // Where each node is drawn once placed: its new rectangle's origin.
    let mut placed: FxHashMap<NodeId, Vec2> = FxHashMap::default();
    for (parent, rect, children) in families {
        let origin = placed.get(&parent).copied().unwrap_or(rect.origin);
        let parent_span = rect.size.max_element() / 2.0;
        let child_span = children
            .iter()
            .map(|(_, r)| r.size.max_element() / 2.0)
            .fold(0.0_f32, f32::max);
        let radius = parent_span + child_span + RADIAL_PADDING;
        let center = origin + rect.size / 2.0;
        let outline = Rect::new(origin, rect.size);
        // The parent's own connector from the grandparent, the
        // grandparent as placed in this run or as drawn: a segment none of
        // its petals may meet, held whole while they are laid.
        let grandparent = doc
            .parent(parent)
            .filter(|&up| doc.get(up).is_some_and(|e| e.kind() == DocKind::Node));
        clear.own = grandparent.and_then(|up| {
            let drawn = geometry.node_rect(up)?;
            let grand = Rect::new(placed.get(&up).copied().unwrap_or(drawn.origin), drawn.size);
            line(grand.center(), &grand, &outline)
        });
        let laid: Vec<Rect> = match regular_flower(&clear, (parent, outline), center, radius, &children) {
            Some(laid) => {
                for ((child, _), petal) in children.iter().zip(&laid) {
                    clear.add(*child, *petal);
                }
                laid
            }
            None => {
                let n = children.len() as f32;
                let mut laid = Vec::with_capacity(children.len());
                for (i, (child, r)) in children.iter().enumerate() {
                    let ray = Vec2::from_angle(START_ANGLE + i as f32 * std::f32::consts::TAU / n);
                    let along = |d: f32| center + d * ray - r.size / 2.0;
                    let at = probes()
                        .map(|d| along(radius + d))
                        .find(|&o| clear.is_clear(Rect::new(o, r.size)))
                        .unwrap_or_else(|| along(radius));
                    let petal = Rect::new(at, r.size);
                    clear.add(*child, petal);
                    laid.push(petal);
                }
                laid
            }
        };
        // For the families after it: the parent's connector — unless this
        // run placed the parent, when its grandparent's family's lines
        // already hold it — and this family's lines.
        if let Some((a, b)) = clear.own.take() {
            if !placed.contains_key(&parent) {
                clear.keep_segment(a, b);
            }
        }
        for petal in &laid {
            if let Some((a, b)) = line(center, &outline, petal) {
                clear.keep_segment(a, b);
            }
        }
        for ((child, r), petal) in children.iter().zip(laid) {
            placed.insert(*child, petal.origin);
            planned.push((*child, position_for(doc, *child, *r, petal.origin)));
        }
    }
    Ok(positions(planned))
}

/// The first regular arrangement of `children` round `center` that is
/// clear, in [`flower`]'s order: the petals' rectangles, in sibling
/// order, or `None`. `parent` is the parent and where it is drawn.
///
/// Cost: at most [`FLOWER_RADII`] × ([`FLOWER_ROTATIONS`] +
/// [`FAN_DIRECTIONS`] × (`FAN_SPANS` − 1)) calls of `arrangement`, and
/// a list of the arrangements, built once.
pub(crate) fn regular_flower(
    clear: &Clear<'_>,
    parent: (NodeId, Rect),
    center: Vec2,
    radius: f32,
    children: &[(NodeId, Rect)],
) -> Option<Vec<Rect>> {
    use std::f32::consts::TAU;
    let n = children.len();
    let spacing = TAU / n as f32;
    // A fan's span, from end petal to end petal: the full circle's, less
    // a narrowing at a time, fewer than `FAN_SPANS` of them. One petal has
    // no fan but the circle.
    let full = spacing * (n.saturating_sub(1)) as f32;
    let spans = (1..FAN_SPANS)
        .map(move |m| full - m as f32 * FAN_NARROWING)
        .take_while(move |span| n > 1 && *span > 0.0);
    // Each arrangement as its first petal's angle and the step to the next.
    let arrangements = (0..FLOWER_ROTATIONS)
        .map(move |j| {
            (
                START_ANGLE + j as f32 * spacing / FLOWER_ROTATIONS as f32,
                spacing,
            )
        })
        .chain(spans.flat_map(move |span| {
            (0..FAN_DIRECTIONS).map(move |d| {
                let middle = START_ANGLE + d as f32 * TAU / FAN_DIRECTIONS as f32;
                (middle - span / 2.0, span / (n - 1) as f32)
            })
        }));
    let arrangements: Vec<(f32, f32)> = arrangements.collect();
    (0..FLOWER_RADII)
        .map(|k| radius + k as f32 * FLOWER_RADIUS_STEP)
        .find_map(|r| {
            arrangements
                .iter()
                .find_map(|&(first, step)| arrangement(clear, parent, center, r, first, step, children))
        })
}

/// `children` at radius `r` round `center`, the first at angle `first`,
/// each next `step` on (a positive step runs clockwise on the canvas,
/// whose y points down): their rectangles, when no petal meets a drawn
/// node, a placed one or another of these, and no petal's line (`line`)
/// crosses one; else `None`, at the first that fails. `parent` is the
/// parent and where it is drawn.
///
/// Cost: O(n²) comparisons among the `n` petals and their lines; per
/// petal a `Clear::is_clear`, per line a `Clear::line_clear`. It stops
/// at the first that fails.
pub(crate) fn arrangement(
    clear: &Clear<'_>,
    parent: (NodeId, Rect),
    center: Vec2,
    r: f32,
    first: f32,
    step: f32,
    children: &[(NodeId, Rect)],
) -> Option<Vec<Rect>> {
    let mut laid: Vec<Rect> = Vec::with_capacity(children.len());
    for (i, (_, drawn)) in children.iter().enumerate() {
        let at = center + r * Vec2::from_angle(first + i as f32 * step) - drawn.size / 2.0;
        let petal = Rect::new(at, drawn.size);
        if !clear.is_clear(petal) || laid.iter().any(|other| meets(other, &petal)) {
            return None;
        }
        laid.push(petal);
    }
    for (i, petal) in laid.iter().enumerate() {
        let Some((a, b)) = line(center, &parent.1, petal) else {
            continue;
        };
        let crosses_own = laid
            .iter()
            .enumerate()
            .any(|(j, other)| j != i && segment_meets(a, b, other));
        if crosses_own || !clear.line_clear(a, b, parent.0) {
            return None;
        }
    }
    Some(laid)
}

/// A petal's line: the straight line from `center`, inside `parent`, to
/// the center of `petal`, where it runs outside the two — from where it
/// leaves the parent to where it enters the petal — or `None` when none
/// of it does. What is inside the parent or the petal is theirs, and
/// reading only the rest keeps a line as short as the gap between them,
/// however large either is. O(1).
fn line(center: Vec2, parent: &Rect, petal: &Rect) -> Option<(Vec2, Vec2)> {
    let to = petal.center();
    let d = to - center;
    let t0 = leaves(center, d, parent);
    let t1 = 1.0 - leaves(to, -d, petal);
    (t0 < t1).then(|| (center + d * t0, center + d * t1))
}

/// How far along `d`, as a fraction of it, a walk from `p` stays inside
/// `rect` (edges included): `0` when `p` is outside, `1` when the walk
/// never leaves within `d`. O(1).
fn leaves(p: Vec2, d: Vec2, rect: &Rect) -> f32 {
    let inside = rect.origin.x <= p.x && p.x <= rect.max().x && rect.origin.y <= p.y && p.y <= rect.max().y;
    if !inside {
        return 0.0;
    }
    let axis = |p: f32, d: f32, lo: f32, hi: f32| {
        if d > 0.0 {
            (hi - p) / d
        } else if d < 0.0 {
            (lo - p) / d
        } else {
            f32::INFINITY
        }
    };
    axis(p.x, d.x, rect.origin.x, rect.max().x)
        .min(axis(p.y, d.y, rect.origin.y, rect.max().y))
        .clamp(0.0, 1.0)
}

/// Whether the segment from `a` to `b` passes through the inside of
/// `rect`: touching its edge or corner does not count. O(1).
fn segment_meets(a: Vec2, b: Vec2, rect: &Rect) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, dp, lo, hi) in [
        (a.x, d.x, rect.origin.x, rect.max().x),
        (a.y, d.y, rect.origin.y, rect.max().y),
    ] {
        if dp == 0.0 {
            if !(lo < p && p < hi) {
                return false;
            }
            continue;
        }
        let (u, v) = ((lo - p) / dp, (hi - p) / dp);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
    }
    t0 < t1
}

/// `cascade`: each node's descendants as a top-down tree — each row
/// [`ROW_GAP`] below its parent, siblings [`SIBLING_GAP`] apart and
/// centered under it, each level placed from its parent's new place. A
/// row that would overlap another drawn node — a row already placed,
/// the anchor, anything else on the map — moves as a whole, keeping its
/// spacing, until it is clear (see `Clear`). It tries, in this order,
/// each depth from where it belongs down by [`CLEAR_STEP`]s, at most
/// [`MAX_CLEAR_STEPS`] of them, and at each depth first straight below,
/// then shifted right and then left by one [`CLEAR_STEP`], by two, and
/// so on to [`MAX_SIDE_STEPS`]; the depths beyond are the doubling
/// [`probes`], each tried the same way. A single node is never moved
/// out of its row. A node nested under another of `nodes` is laid out
/// as part of the outer one's tree, so the walk starts from the
/// outermost. Breadth first and iterative.
///
/// Cost: the subtrees, then per row at most the [`probes`] × (2 ×
/// [`MAX_SIDE_STEPS`] + 1) tries, each a query of the projector's body
/// index and a lookup of this run's placements per node of the row.
/// Nothing is proportional to the map.
pub fn cascade(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String> {
    // The rows, breadth first: which does not depend on where they go.
    let mut rows: Vec<Family> = Vec::new();
    let outermost = in_tree_order(doc.tree(), nodes, true);
    let mut seen: FxHashSet<NodeId> = outermost.iter().copied().collect();
    let mut queue: VecDeque<NodeId> = outermost.into();
    while let Some(parent) = queue.pop_front() {
        for child in child_nodes(doc, parent) {
            if seen.insert(child) {
                queue.push_back(child);
            }
        }
        if let Some(rect) = geometry.node_rect(parent) {
            let row = drawn_children(doc, geometry, parent);
            if !row.is_empty() {
                rows.push((parent, rect, row));
            }
        }
    }
    let mut clear = Clear::new(geometry, rows.iter().flat_map(|(_, _, row)| row));
    let mut planned: Vec<(NodeId, Vec2)> = Vec::new();
    // Where each node is drawn once placed: its new rectangle's origin.
    let mut placed: FxHashMap<NodeId, Vec2> = FxHashMap::default();
    for (parent, rect, row) in rows {
        let origin = placed.get(&parent).copied().unwrap_or(rect.origin);
        let width: f32 =
            row.iter().map(|(_, r)| r.size.x).sum::<f32>() + SIBLING_GAP * (row.len() as f32 - 1.0);
        let left = origin.x + rect.size.x / 2.0 - width / 2.0;
        // The row's rectangles with its top at `y`, shifted right by `dx`.
        let at = |dx: f32, y: f32| {
            let mut x = left + dx;
            row.iter()
                .map(|(_, r)| {
                    let laid = Rect::new(Vec2::new(x, y), r.size);
                    x += r.size.x + SIBLING_GAP;
                    laid
                })
                .collect::<Vec<Rect>>()
        };
        let top = origin.y + rect.size.y + ROW_GAP;
        // Straight below first, then right and left, one step further each.
        let sideways = || {
            std::iter::once(0.0).chain((1..=MAX_SIDE_STEPS).flat_map(|j| {
                let d = j as f32 * CLEAR_STEP;
                [d, -d]
            }))
        };
        let (dx, y) = probes()
            .flat_map(|d| sideways().map(move |dx| (dx, top + d)))
            .find(|&(dx, y)| at(dx, y).iter().all(|&laid| clear.is_clear(laid)))
            .unwrap_or((0.0, top));
        for ((child, r), laid) in row.iter().zip(at(dx, y)) {
            clear.add(*child, laid);
            placed.insert(*child, laid.origin);
            planned.push((*child, position_for(doc, *child, *r, laid.origin)));
        }
    }
    Ok(positions(planned))
}

/// A drawn node and where, with its drawn child nodes and where: what a
/// layout places round, or under, one node.
type Family = (NodeId, Rect, Vec<(NodeId, Rect)>);

/// The child nodes of `id`, in sibling order.
fn child_nodes(doc: &Document, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(id)
        .filter(move |&child| doc.get(child).is_some_and(|e| e.kind() == DocKind::Node))
}

/// The child nodes of `id` that are drawn, with where, in sibling order.
fn drawn_children(doc: &Document, geometry: &dyn Geometry, id: NodeId) -> Vec<(NodeId, Rect)> {
    child_nodes(doc, id)
        .filter_map(|child| geometry.node_rect(child).map(|r| (child, r)))
        .collect()
}

/// The side of a cell of a run's placements, in canvas units.
const CELL: f32 = 512.0;

/// The most cells a placed rectangle is filed under; a larger one is
/// kept apart and tested against every query.
const MAX_CELLS: i64 = 64;

/// What a layout keeps its nodes clear of: every drawn node it does not
/// move, as the projector's body index has it
/// ([`Geometry::nodes_meeting`]); every rectangle this run has placed;
/// the connector of the family being laid, whole; and the segments a
/// flower keeps for the families after — each laid family's parent's
/// connector from its own parent, and its lines, so a later family
/// avoids every one of them — a segment past 16,384 units in its two end
/// stretches only. The placements and the kept segments are filed in a
/// small grid of its own. Two rectangles are clear when their insides
/// do not meet, and a segment and a rectangle when the segment does not
/// pass through the rectangle's inside; a shared edge is clear. Holds
/// only the moved set, the placements and the kept segments: nothing of
/// the map is copied.
pub(crate) struct Clear<'g> {
    /// Where the map is drawn.
    geometry: &'g dyn Geometry,
    /// The nodes this run moves, whose drawn rectangles are where they
    /// were, not where they go.
    moved: FxHashSet<NodeId>,
    /// The placed rectangles, with their nodes, filed under each cell
    /// they cover.
    cells: FxHashMap<(i32, i32), Vec<(NodeId, Rect)>>,
    /// Every placed rectangle, for a query too large for the cells.
    placed: Vec<(NodeId, Rect)>,
    /// The kept segments, in pieces of at most [`LINE_PIECE`], each piece
    /// filed under the cells its box covers (at most four).
    segment_cells: FxHashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
    /// The connector of the family being laid, from its grandparent to its
    /// parent, held whole: its petals meet no part of it, however long.
    own: Option<(Vec2, Vec2)>,
}

#[cfg(test)]
thread_local! {
    /// How many kept segment pieces `Clear` has tested against a
    /// rectangle, on this thread: what the test that holds a family's
    /// work to its own neighborhood reads. Test builds only.
    pub(crate) static SEGMENT_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The pieces of the segment from `a` to `b`, in order, each at most
/// [`LINE_PIECE`] long, so each piece's box covers at most four cells of
/// the body index's grid or of `Clear`'s. O(1) per piece; the caller
/// bounds how many by the segment's length.
fn pieces(a: Vec2, b: Vec2) -> impl Iterator<Item = (Vec2, Vec2)> {
    let count = ((b - a).length() / LINE_PIECE).ceil().max(1.0) as u32;
    (0..count).map(move |k| {
        (
            a.lerp(b, k as f32 / count as f32),
            a.lerp(b, (k + 1) as f32 / count as f32),
        )
    })
}

/// The box of the segment from `a` to `b`. O(1).
fn box_of(a: Vec2, b: Vec2) -> Rect {
    Rect::new(a.min(b), (b - a).abs())
}

/// Whether the insides of `a` and `b` meet. O(1).
fn meets(a: &Rect, b: &Rect) -> bool {
    a.origin.x < b.max().x && b.origin.x < a.max().x && a.origin.y < b.max().y && b.origin.y < a.max().y
}

/// The cells `rect` covers, or `None` past [`MAX_CELLS`] or for a
/// rectangle that is not finite. Each axis is bounded before the count
/// is taken, so no rectangle, however large, overflows it.
fn cells_of(rect: Rect) -> Option<impl Iterator<Item = (i32, i32)>> {
    if !rect.origin.is_finite() || !rect.size.is_finite() {
        return None;
    }
    let cell = |v: f32| i64::from((v / CELL).floor().clamp(i32::MIN as f32, i32::MAX as f32) as i32);
    let (x0, y0, x1, y1) = (
        cell(rect.origin.x),
        cell(rect.origin.y),
        cell(rect.max().x),
        cell(rect.max().y),
    );
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    if w > MAX_CELLS || h > MAX_CELLS || w * h > MAX_CELLS {
        return None;
    }
    // Each is within an `i32` cell coordinate: they came from one.
    let (x0, y0, x1, y1) = (x0 as i32, y0 as i32, x1 as i32, y1 as i32);
    Some((x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y))))
}

impl<'g> Clear<'g> {
    /// Nothing placed yet; the nodes of `moved` are skipped where the
    /// projector has them. O(moved).
    pub(crate) fn new<'a>(
        geometry: &'g dyn Geometry,
        moved: impl Iterator<Item = &'a (NodeId, Rect)>,
    ) -> Self {
        Clear {
            geometry,
            moved: moved.map(|(node, _)| *node).collect(),
            cells: FxHashMap::default(),
            placed: Vec::new(),
            segment_cells: FxHashMap::default(),
            own: None,
        }
    }

    /// Keep the segment from `a` to `b` for the petals placed after: walked
    /// in [`pieces`], each filed under the cells its box covers. A segment
    /// longer than [`MAX_LINE_PIECES`] × [`LINE_PIECE`] (16,384 units) is
    /// kept only in its two end stretches of that length, next to the two
    /// nodes it joins; its middle is not kept. O(its pieces): at most
    /// 2 × [`MAX_LINE_PIECES`], each filed under at most four cells.
    pub(crate) fn keep_segment(&mut self, a: Vec2, b: Vec2) {
        let length = (b - a).length();
        if !length.is_finite() {
            return;
        }
        let reach = MAX_LINE_PIECES as f32 * LINE_PIECE;
        let stretches = if length <= reach {
            [Some((a, b)), None]
        } else {
            let end = (b - a) * (reach / length);
            [Some((a, a + end)), Some((b - end, b))]
        };
        for (from, to) in stretches.into_iter().flatten() {
            for piece in pieces(from, to) {
                for cell in cells_of(box_of(piece.0, piece.1)).into_iter().flatten() {
                    self.segment_cells.entry(cell).or_default().push(piece);
                }
            }
        }
    }

    /// Whether `rect` meets the family's own connector, or a kept segment
    /// piece filed under one of its cells. A rectangle too large for the
    /// cells (a petal past [`MAX_CELLS`] of them) is read along its outline
    /// instead, a [`LINE_PIECE`] at a time — a kept segment that meets its
    /// inside without crossing its outline would lie wholly inside it,
    /// with an end on a node's outline that the rectangle then covers,
    /// which `is_clear` refuses anyway — or, when that outline has more
    /// pieces than there are cells holding kept pieces, through those cells
    /// that lie under it. O(the pieces filed under `rect`'s cells), and for
    /// a rectangle too large for them O(the fewer of its outline's pieces
    /// and the cells holding kept pieces), stopping at the first.
    fn meets_segment(&self, rect: Rect) -> bool {
        if self.own.is_some_and(|(a, b)| segment_meets(a, b, &rect)) {
            return true;
        }
        let through = |&(a, b): &(Vec2, Vec2)| {
            #[cfg(test)]
            SEGMENT_TESTS.with(|tests| tests.set(tests.get() + 1));
            segment_meets(a, b, &rect)
        };
        let filed = |cell: (i32, i32)| {
            self.segment_cells
                .get(&cell)
                .is_some_and(|near| near.iter().any(through))
        };
        match cells_of(rect) {
            Some(mut cells) => cells.any(filed),
            None if rect.origin.is_finite() && rect.size.is_finite() => {
                // Whichever is fewer: the pieces of the outline, or the
                // cells that hold kept pieces at all, those under `rect`.
                let outline = 2.0 * (rect.size.x + rect.size.y) / LINE_PIECE;
                if outline <= self.segment_cells.len() as f32 {
                    let (min, max) = (rect.origin, rect.max());
                    let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y), min];
                    corners.windows(2).any(|edge| {
                        pieces(edge[0], edge[1])
                            .any(|(p, q)| cells_of(box_of(p, q)).into_iter().flatten().any(filed))
                    })
                } else {
                    let cell = |v: f32| (v / CELL).floor();
                    let (lo, hi) = (rect.origin, rect.max());
                    self.segment_cells.iter().any(|(&(x, y), near)| {
                        let (x, y) = (x as f32, y as f32);
                        cell(lo.x) <= x
                            && x <= cell(hi.x)
                            && cell(lo.y) <= y
                            && y <= cell(hi.y)
                            && near.iter().any(through)
                    })
                }
            }
            None => false,
        }
    }

    /// File `node`, placed at `rect`. O(its cells).
    fn add(&mut self, node: NodeId, rect: Rect) {
        if let Some(cells) = cells_of(rect) {
            for cell in cells {
                self.cells.entry(cell).or_default().push((node, rect));
            }
        }
        self.placed.push((node, rect));
    }

    /// Whether `hit` holds for any placed rectangle that may meet
    /// `area`: those filed under its cells, or every one for an area too
    /// large for them (an area past [`MAX_CELLS`] of 512-unit cells, which
    /// a petal larger than that is). O(the placements filed under `area`'s
    /// cells, or every placement for such an area), stopping at the first
    /// hit.
    fn any_placed(&self, area: Rect, mut hit: impl FnMut(NodeId, &Rect) -> bool) -> bool {
        match cells_of(area) {
            Some(cells) => {
                for cell in cells {
                    if let Some(filed) = self.cells.get(&cell) {
                        if filed.iter().any(|(node, rect)| hit(*node, rect)) {
                            return true;
                        }
                    }
                }
                false
            }
            None => self.placed.iter().any(|(node, rect)| hit(*node, rect)),
        }
    }

    /// Whether `rect` is clear of every drawn node this run does not
    /// move, of every rectangle it has placed, of the family's own
    /// connector and of every segment it keeps. O(the body index's cells
    /// under `rect`, and this run's placements and kept segment pieces in
    /// them; see `meets_segment` for a rectangle too large for the cells).
    fn is_clear(&self, rect: Rect) -> bool {
        !self.any_placed(rect, |_, other| meets(&rect, other))
            && !self.meets_segment(rect)
            && self
                .geometry
                .nodes_meeting(rect)
                .iter()
                .all(|node| self.moved.contains(node))
    }

    /// Whether the segment from `a` to `b` — a petal's line, outside its
    /// parent and itself — crosses no drawn node this run does not move
    /// and no rectangle it has placed, `parent` apart. Read a
    /// [`LINE_PIECE`] at a time, so each read covers at most four cells;
    /// a segment of more than [`MAX_LINE_PIECES`] pieces is not read, and
    /// counts as crossing. O(the segment's length over [`LINE_PIECE`],
    /// at most [`MAX_LINE_PIECES`] pieces), each a query of the body index
    /// over at most four cells, a `node_rect` per node it finds, and a
    /// lookup of the placements.
    fn line_clear(&self, a: Vec2, b: Vec2, parent: NodeId) -> bool {
        let count = ((b - a).length() / LINE_PIECE).ceil();
        if !count.is_finite() || count > MAX_LINE_PIECES as f32 {
            return false;
        }
        pieces(a, b).all(|(from, to)| {
            let area = box_of(from, to);
            let crosses_placed = self.any_placed(area, |node, other| {
                node != parent && segment_meets(from, to, other)
            });
            let crosses_drawn = self.geometry.nodes_meeting(area).into_iter().any(|node| {
                node != parent
                    && !self.moved.contains(&node)
                    && self
                        .geometry
                        .node_rect(node)
                        .is_some_and(|drawn| segment_meets(from, to, &drawn))
            });
            !crosses_placed && !crosses_drawn
        })
    }
}

/// The position that draws `node`'s rectangle, now `drawn`, at
/// `origin`: its own position moved as far as its rectangle moves.
fn position_for(doc: &Document, node: NodeId, drawn: Rect, origin: Vec2) -> Vec2 {
    let position = match doc.get(node) {
        Some(DocElement::Node(data)) => data.position,
        _ => drawn.origin,
    };
    position + (origin - drawn.origin)
}

/// The walks that set each planned node's position: one `SetPosition`
/// anchored at each node, in the order they were placed. One walk per
/// node, because a walk applies one mutator per sibling key and sibling
/// nodes commonly share one (all on channel 0), so no single walk could
/// tell them apart (ruling on #250). Each position is clamped to the
/// canvas bound ([`clamp_canvas_coord`]), as `main`'s layouts clamped
/// theirs: a layout near the edge of the canvas computes a position
/// past it from positions inside it, and has no author to refuse, so
/// the node is pinned at the edge rather than the whole edit refused.
/// O(the planned nodes).
fn positions(planned: Vec<(NodeId, Vec2)>) -> Walks {
    planned
        .into_iter()
        .map(|(node, at)| {
            let at = Vec2::new(clamp_canvas_coord(at.x), clamp_canvas_coord(at.y));
            let set = Mutator::new(
                DocMutation::Node(NodeOp::SetPosition(at)),
                BranchKey::new(DocKind::Node, 0),
            );
            (MutatorTree::new_with(set), Anchors::Nodes(vec![node]))
        })
        .collect()
}
