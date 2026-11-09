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
//!   A petal moves outward along its ray, a row downward,
//!   [`CLEAR_STEP`](crate::edit_handlers::CLEAR_STEP) at a time, and after
//!   [`MAX_CLEAR_STEPS`](crate::edit_handlers::MAX_CLEAR_STEPS) tries it
//!   goes past everything drawn, where it is clear by construction.
//!   Only a layout keeps clear: a node moved by hand goes where it is
//!   put.
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
use crate::document::bounds::clamp_canvas_coord;
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
/// cascade row downward.
pub const CLEAR_STEP: f32 = 40.0;

/// How many [`CLEAR_STEP`]s a layout tries before it places the node
/// past everything drawn, where it is clear by construction.
pub const MAX_CLEAR_STEPS: u32 = 64;

/// `flower`: each node's child nodes on a circle round it, the first at
/// the top, clockwise. The radius clears the parent's half-span and the
/// largest child's, plus [`RADIAL_PADDING`]. A petal that would overlap
/// another drawn node — a sibling already placed, the parent, anything
/// else on the map — moves outward along its ray, [`CLEAR_STEP`] at a
/// time, until it is clear (see `Clear`). Parents are placed before
/// their children, in tree order, and a node one flower moves is the
/// center of its own where it is moved to, so nested nodes compose.
///
/// Cost: one pass over the map's nodes for what is drawn, then per
/// petal at most [`MAX_CLEAR_STEPS`] lookups, each of the grid cells
/// the petal covers.
pub fn flower(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String> {
    let families: Vec<Family> = in_tree_order(doc.tree(), nodes, false)
        .into_iter()
        .filter_map(|parent| {
            let rect = geometry.node_rect(parent)?;
            let children = drawn_children(doc, geometry, parent);
            (!children.is_empty()).then_some((parent, rect, children))
        })
        .collect();
    let mut clear = Clear::around(doc, geometry, families.iter().flat_map(|(_, _, row)| row));
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
        let n = children.len() as f32;
        for (i, (child, r)) in children.iter().enumerate() {
            let ray = Vec2::from_angle(START_ANGLE + i as f32 * std::f32::consts::TAU / n);
            let along = |d: f32| center + d * ray - r.size / 2.0;
            let tried = (0..MAX_CLEAR_STEPS)
                .map(|k| along(radius + k as f32 * CLEAR_STEP))
                .find(|&o| clear.is_clear(Rect::new(o, r.size)));
            // Past every drawn rectangle: its center farther from `center`
            // than any of them reaches, by more than its own half-diagonal.
            let at =
                tried.unwrap_or_else(|| along(radius.max(clear.reach(center) + r.size.length() / 2.0 + 1.0)));
            clear.add(Rect::new(at, r.size));
            placed.insert(*child, at);
            planned.push((*child, position_for(doc, *child, *r, at)));
        }
    }
    Ok(positions(planned))
}

/// `cascade`: each node's descendants as a top-down tree — each row
/// [`ROW_GAP`] below its parent, siblings [`SIBLING_GAP`] apart and
/// centered under it, each level placed from its parent's new place. A
/// row that would overlap another drawn node — a row already placed,
/// the anchor, anything else on the map — moves down, [`CLEAR_STEP`] at
/// a time, until it is clear (see `Clear`). A node nested under
/// another of `nodes` is laid out as part of the outer one's tree, so
/// the walk starts from the outermost. Breadth first and iterative.
///
/// Cost: the subtrees, one pass over the map's nodes for what is drawn,
/// then per row at most [`MAX_CLEAR_STEPS`] lookups of its nodes.
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
    let mut clear = Clear::around(doc, geometry, rows.iter().flat_map(|(_, _, row)| row));
    let mut planned: Vec<(NodeId, Vec2)> = Vec::new();
    // Where each node is drawn once placed: its new rectangle's origin.
    let mut placed: FxHashMap<NodeId, Vec2> = FxHashMap::default();
    for (parent, rect, row) in rows {
        let origin = placed.get(&parent).copied().unwrap_or(rect.origin);
        let width: f32 =
            row.iter().map(|(_, r)| r.size.x).sum::<f32>() + SIBLING_GAP * (row.len() as f32 - 1.0);
        let left = origin.x + rect.size.x / 2.0 - width / 2.0;
        // The row's rectangles with its top at `y`.
        let at = |y: f32| {
            let mut x = left;
            row.iter()
                .map(|(_, r)| {
                    let laid = Rect::new(Vec2::new(x, y), r.size);
                    x += r.size.x + SIBLING_GAP;
                    laid
                })
                .collect::<Vec<Rect>>()
        };
        let top = origin.y + rect.size.y + ROW_GAP;
        let tried = (0..MAX_CLEAR_STEPS)
            .map(|k| top + k as f32 * CLEAR_STEP)
            .find(|&y| at(y).iter().all(|&laid| clear.is_clear(laid)));
        // Below every drawn rectangle.
        let y = tried.unwrap_or_else(|| top.max(clear.bottom() + ROW_GAP));
        for ((child, r), laid) in row.iter().zip(at(y)) {
            clear.add(laid);
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

/// The side of a [`Clear`] grid cell, in canvas units.
const CELL: f32 = 512.0;

/// The most grid cells a rectangle is filed under; a larger one is kept
/// apart and tested against every query.
const MAX_CELLS: i64 = 64;

/// The drawn rectangles a layout keeps its nodes clear of: every drawn
/// node it does not move, as drawn, and every node it has placed, as
/// placed. Two rectangles are clear when their insides do not meet; a
/// shared edge is clear. Filed in a uniform grid so a lookup reads only
/// the rectangles near it.
struct Clear {
    /// The rectangles filed under each cell they cover.
    cells: FxHashMap<(i32, i32), Vec<Rect>>,
    /// The rectangles that cover more than [`MAX_CELLS`] cells.
    wide: Vec<Rect>,
    /// Every rectangle, for [`Self::reach`] and [`Self::bottom`].
    all: Vec<Rect>,
}

impl Clear {
    /// Every drawn node of `doc` but those in `moved`. One pass over the
    /// map's nodes, iterative.
    fn around<'a>(
        doc: &Document,
        geometry: &dyn Geometry,
        moved: impl Iterator<Item = &'a (NodeId, Rect)>,
    ) -> Self {
        let moved: FxHashSet<NodeId> = moved.map(|(node, _)| *node).collect();
        let mut clear = Clear {
            cells: FxHashMap::default(),
            wide: Vec::new(),
            all: Vec::new(),
        };
        let mut pending = vec![doc.root()];
        while let Some(id) = pending.pop() {
            for child in child_nodes(doc, id) {
                if !moved.contains(&child) {
                    if let Some(rect) = geometry.node_rect(child) {
                        clear.add(rect);
                    }
                }
                pending.push(child);
            }
        }
        clear
    }

    /// The cells `rect` covers, or `None` past [`MAX_CELLS`].
    fn cells_of(rect: Rect) -> Option<impl Iterator<Item = (i32, i32)>> {
        let cell = |v: f32| (v / CELL).floor() as i32;
        let (x0, y0, x1, y1) = (
            cell(rect.origin.x),
            cell(rect.origin.y),
            cell(rect.max().x),
            cell(rect.max().y),
        );
        let count = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1);
        (count <= MAX_CELLS).then(|| (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y))))
    }

    /// File `rect`.
    fn add(&mut self, rect: Rect) {
        match Self::cells_of(rect) {
            Some(cells) => {
                for cell in cells {
                    self.cells.entry(cell).or_default().push(rect);
                }
            }
            None => self.wide.push(rect),
        }
        self.all.push(rect);
    }

    /// Whether `rect` is clear of every filed rectangle.
    fn is_clear(&self, rect: Rect) -> bool {
        let meets = |other: &Rect| {
            rect.origin.x < other.max().x
                && other.origin.x < rect.max().x
                && rect.origin.y < other.max().y
                && other.origin.y < rect.max().y
        };
        let near = match Self::cells_of(rect) {
            Some(cells) => cells
                .filter_map(|cell| self.cells.get(&cell))
                .any(|filed| filed.iter().any(meets)),
            None => self.all.iter().any(meets),
        };
        !near && !self.wide.iter().any(meets)
    }

    /// How far from `center` the farthest filed rectangle reaches: past
    /// it, a disk round `center` holds every one.
    fn reach(&self, center: Vec2) -> f32 {
        self.all
            .iter()
            .flat_map(|r| {
                [
                    r.origin,
                    r.max(),
                    Vec2::new(r.origin.x, r.max().y),
                    Vec2::new(r.max().x, r.origin.y),
                ]
            })
            .map(|corner| (corner - center).length())
            .fold(0.0, f32::max)
    }

    /// The lowest edge of any filed rectangle.
    fn bottom(&self) -> f32 {
        self.all.iter().map(|r| r.max().y).fold(f32::MIN, f32::max)
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
