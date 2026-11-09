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
//! `flower` and `cascade` are ported from `main`'s
//! `document/mutations/flower_layout.rs` and `tree_cascade.rs`: the same
//! placement, now emitted as one `NodeOp::SetPosition` per node, each
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

/// `flower`: each node's child nodes on a circle round it, the first at
/// the top, clockwise. The radius clears the parent's half-span and the
/// largest child's, plus [`RADIAL_PADDING`], so no child's outline
/// crosses the parent's. Parents are placed before their children, in
/// tree order, and a node one flower moves is the center of its own
/// where it is moved to, so nested nodes compose. O(the nodes'
/// children, plus the tree order of `nodes`).
pub fn flower(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String> {
    let mut planned: Vec<(NodeId, Vec2)> = Vec::new();
    // Where each node is drawn once placed: its new rectangle's origin.
    let mut placed: FxHashMap<NodeId, Vec2> = FxHashMap::default();
    for parent in in_tree_order(doc.tree(), nodes, false) {
        let Some(rect) = geometry.node_rect(parent) else {
            continue;
        };
        let origin = placed.get(&parent).copied().unwrap_or(rect.origin);
        let children: Vec<(NodeId, Rect)> = child_nodes(doc, parent)
            .filter_map(|child| geometry.node_rect(child).map(|r| (child, r)))
            .collect();
        if children.is_empty() {
            continue;
        }
        let parent_span = rect.size.max_element() / 2.0;
        let child_span = children
            .iter()
            .map(|(_, r)| r.size.max_element() / 2.0)
            .fold(0.0_f32, f32::max);
        let radius = parent_span + child_span + RADIAL_PADDING;
        let center = origin + rect.size / 2.0;
        let n = children.len() as f32;
        for (i, (child, r)) in children.iter().enumerate() {
            let theta = START_ANGLE + i as f32 * std::f32::consts::TAU / n;
            let at = center + radius * Vec2::new(theta.cos(), theta.sin()) - r.size / 2.0;
            placed.insert(*child, at);
            planned.push((*child, position_for(doc, *child, *r, at)));
        }
    }
    Ok(positions(planned))
}

/// `cascade`: each node's descendants as a top-down tree — each row
/// [`ROW_GAP`] below its parent, siblings [`SIBLING_GAP`] apart and
/// centered under it, each level placed from its parent's new place.
/// A node nested under another of `nodes` is laid out as part of the
/// outer one's tree, so the walk starts from the outermost.
/// Breadth first and iterative. O(the subtrees).
pub fn cascade(doc: &Document, geometry: &dyn Geometry, nodes: &[NodeId]) -> Result<Walks, String> {
    let mut planned: Vec<(NodeId, Vec2)> = Vec::new();
    // Where each node is drawn once placed: its new rectangle's origin.
    let mut placed: FxHashMap<NodeId, Vec2> = FxHashMap::default();
    let outermost = in_tree_order(doc.tree(), nodes, true);
    let mut seen: FxHashSet<NodeId> = outermost.iter().copied().collect();
    let mut queue: VecDeque<NodeId> = outermost.into();
    while let Some(parent) = queue.pop_front() {
        let children: Vec<NodeId> = child_nodes(doc, parent).collect();
        for &child in &children {
            if seen.insert(child) {
                queue.push_back(child);
            }
        }
        let Some(rect) = geometry.node_rect(parent) else {
            continue;
        };
        let origin = placed.get(&parent).copied().unwrap_or(rect.origin);
        let row: Vec<(NodeId, Rect)> = children
            .iter()
            .filter_map(|&child| geometry.node_rect(child).map(|r| (child, r)))
            .collect();
        if row.is_empty() {
            continue;
        }
        let width: f32 =
            row.iter().map(|(_, r)| r.size.x).sum::<f32>() + SIBLING_GAP * (row.len() as f32 - 1.0);
        let row_y = origin.y + rect.size.y + ROW_GAP;
        let mut x = origin.x + rect.size.x / 2.0 - width / 2.0;
        for (child, r) in row {
            let at = Vec2::new(x, row_y);
            placed.insert(child, at);
            planned.push((child, position_for(doc, child, r, at)));
            x += r.size.x + SIBLING_GAP;
        }
    }
    Ok(positions(planned))
}

/// The child nodes of `id`, in sibling order.
fn child_nodes(doc: &Document, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(id)
        .filter(move |&child| doc.get(child).is_some_and(|e| e.kind() == DocKind::Node))
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
/// tell them apart (ruling on #250). O(the planned nodes).
fn positions(planned: Vec<(NodeId, Vec2)>) -> Walks {
    planned
        .into_iter()
        .map(|(node, at)| {
            let set = Mutator::new(
                DocMutation::Node(NodeOp::SetPosition(at)),
                BranchKey::new(DocKind::Node, 0),
            );
            (MutatorTree::new_with(set), Anchors::Nodes(vec![node]))
        })
        .collect()
}
