// SPDX-License-Identifier: MPL-2.0

//! The structural step of an update (#241 §5.1) and its seeds (§5.2):
//! which nodes are visible, which elements are units, and what each
//! change marks.

use rustc_hash::FxHashSet;

use crate::arena::NodeId;
use crate::document::deps::InputKind;
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::palettes;
use crate::document::resolve::Input as DocInput;
use crate::document::values::{LinkKind, PaletteId};
use crate::document::writer::feed::ChangeRecord;
use crate::engine::journal::Touch;
use crate::engine::tree::Tree;
use crate::projection::input::{Dependent, Input, Stage};
use crate::projection::layers::{GlyphLayers, Layer, LayerBuilders};
use crate::projection::stages::Work;
use crate::projection::state::{
    LineState, NodeState, Owner, PortalState, RingState, SectionState, Unit, UnitBody,
};
use crate::projection::Projector;

/// Every element a batch of records touched, each once, in the order
/// first named, and which of them a record moved under another parent.
pub(crate) struct Touched {
    /// The elements.
    pub(crate) ids: Vec<NodeId>,
    /// The elements some record re-parented.
    pub(crate) moved: FxHashSet<NodeId>,
}

impl Touched {
    /// The touches of `changes`. O(their touches).
    pub(crate) fn of(changes: &[ChangeRecord]) -> Self {
        let mut seen = FxHashSet::default();
        let mut ids = Vec::new();
        let mut moved = FxHashSet::default();
        for record in changes {
            for &(id, touch) in &record.touched {
                if seen.insert(id) {
                    ids.push(id);
                }
                if touch == Touch::Moved {
                    moved.insert(id);
                }
            }
        }
        Touched { ids, moved }
    }
}

/// The element at `id`, if it is in the tree. O(1).
pub(crate) fn element(tree: &Tree<DocElement>, id: NodeId) -> Option<&DocElement> {
    if !tree.contains(id) {
        return None;
    }
    tree.get(id).map(|node| node.get())
}

/// `id`'s parent when it is a node; `None` for a root node, whose parent
/// is the map. O(1).
pub(crate) fn parent_node(tree: &Tree<DocElement>, id: NodeId) -> Option<NodeId> {
    let up = tree.node(id)?.parent()?;
    (element(tree, up)?.kind() == DocKind::Node).then_some(up)
}

/// The units under `node` in document order: its sections, its frame's
/// rings, its connector and its links. Stops at its first child node,
/// since nodes are the last of a node's children. O(the node's own
/// elements).
pub(crate) fn units_in_order(tree: &Tree<DocElement>, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    let arena = tree.arena();
    node.children(arena)
        .take_while(move |&c| arena[c].get().kind() != DocKind::Node)
        .flat_map(move |c| {
            let rings = (arena[c].get().kind() == DocKind::Frame)
                .then(|| c.children(arena).filter(move |&r| arena[r].get().kind() == DocKind::Ring))
                .into_iter()
                .flatten();
            let own = matches!(
                arena[c].get().kind(),
                DocKind::Section | DocKind::Connector | DocKind::Link
            )
            .then_some(c);
            own.into_iter().chain(rings)
        })
}

/// What a unit element is, freshly projected: `None` for an element
/// that draws through another (a span, a cap, a label, a portal end, a
/// frame) or nothing.
fn body_for(tree: &Tree<DocElement>, id: NodeId) -> Option<UnitBody> {
    Some(match element(tree, id)? {
        DocElement::Section(_) => UnitBody::Section(SectionState::default()),
        DocElement::Ring(_) => UnitBody::Ring(RingState {
            frame: tree.node(id)?.parent(),
            ..RingState::default()
        }),
        DocElement::Connector(_) => UnitBody::Line(Box::default()),
        DocElement::Link(link) => match link.kind {
            LinkKind::Line => UnitBody::Line(Box::<LineState>::default()),
            LinkKind::Portal => UnitBody::Portal(Box::<PortalState>::default()),
        },
        _ => return None,
    })
}

/// The palette id of the palette element `id`: its rank among the map's
/// palettes. O(palettes).
fn palette_id_of(tree: &Tree<DocElement>, id: NodeId) -> Option<PaletteId> {
    let arena = tree.arena();
    let rank = tree
        .root()
        .children(arena)
        .filter(|&c| arena[c].get().kind() == DocKind::Palette)
        .position(|c| c == id)?;
    palettes(tree).nth(rank).map(|(palette, _)| palette)
}

impl Projector {
    /// Whether the node `id` is drawn: in the tree, not hidden, and a root
    /// node or the child of a drawn node that is not folded (D17). Reads
    /// the parent's projection state, so a parent is decided before its
    /// children. O(1).
    pub(crate) fn shows(&self, tree: &Tree<DocElement>, id: NodeId) -> bool {
        let Some(DocElement::Node(node)) = element(tree, id) else {
            return false;
        };
        if node.hidden {
            return false;
        }
        let Some(up) = tree.node(id).and_then(|n| n.parent()) else {
            return false;
        };
        if up == tree.root() {
            return true;
        }
        match element(tree, up) {
            Some(DocElement::Node(parent)) => !parent.folded && self.nodes.contains_key(&up),
            _ => false,
        }
    }

    /// Project every visible root node and its subtree: what a full
    /// projection starts from. O(visible document).
    pub(crate) fn project_roots(&mut self, tree: &Tree<DocElement>, work: &mut Work) {
        let arena = tree.arena();
        let roots: Vec<NodeId> = tree
            .root()
            .children(arena)
            .filter(|&c| arena[c].get().kind() == DocKind::Node)
            .collect();
        for root in roots {
            if self.shows(tree, root) {
                self.create_subtree(tree, root, work);
            }
        }
    }

    /// Create projection state for `top`, a node that is drawn and has
    /// none, and for every node below it that is drawn and has none —
    /// a node that already has state was moved here and keeps it — and
    /// for all their units, and mark all their work. Iterates with a heap
    /// stack (§20 rule 11); the parent's depth is carried down. O(the
    /// subtree).
    pub(crate) fn create_subtree(&mut self, tree: &Tree<DocElement>, top: NodeId, work: &mut Work) {
        let parent = parent_node(tree, top);
        let depth = parent
            .and_then(|p| self.nodes.get(&p))
            .map_or(0, |p| p.depth + 1);
        let arena = tree.arena();
        let mut stack = vec![(top, parent, depth)];
        work.pending_roots.push(top);
        while let Some((id, parent, depth)) = stack.pop() {
            let Some(DocElement::Node(data)) = element(tree, id) else {
                continue;
            };
            let mut state = NodeState::new(parent, depth);
            state.folded = data.folded;
            let folded = data.folded;
            self.nodes.insert(id, state);
            work.created.insert(id);
            self.queue(work, Dependent::new(id, Stage::Carry));
            self.queue(work, Dependent::new(id, Stage::Geometry));
            // Whatever waited for it to be drawn — a link into it — now
            // draws.
            self.mark(work, Input::Visibility(id));
            let units: Vec<NodeId> = units_in_order(tree, id).collect();
            for unit in units {
                self.create_unit(tree, unit, id, work);
            }
            if folded {
                continue;
            }
            for child in id.children(arena) {
                if let DocElement::Node(node) = arena[child].get() {
                    if !node.hidden && !self.nodes.contains_key(&child) {
                        stack.push((child, Some(id), depth + 1));
                    }
                }
            }
        }
    }

    /// Create the unit `id` of `node`, if it is one and has no state, and
    /// mark its resolution. O(1).
    pub(crate) fn create_unit(&mut self, tree: &Tree<DocElement>, id: NodeId, node: NodeId, work: &mut Work) {
        if self.units.contains_key(&id) {
            return;
        }
        let Some(body) = body_for(tree, id) else {
            return;
        };
        self.units.insert(id, Unit::new(node, body));
        if let Some(state) = self.nodes.get_mut(&node) {
            state.units.push(id);
        }
        self.queue(work, Dependent::new(id, Stage::Resolve));
    }

    /// Drop the unit `id`: forget what it read, take its runs out of the
    /// reverse map — and, when `remove` says so, out of their layers,
    /// journaled — and mark what read its geometry. O(its slots and what
    /// it read).
    pub(crate) fn drop_unit(
        &mut self,
        id: NodeId,
        remove: bool,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
        work: &mut Work,
    ) {
        let Some(unit) = self.units.remove(&id) else {
            return;
        };
        for slot in unit.slots.iter().chain(unit.retired.iter()) {
            if let Some(glyph) = slot.glyph {
                self.owners.remove(&(slot.layer, glyph));
                if remove {
                    write::remove(layers, out, slot.layer, glyph);
                }
            }
        }
        for stage in [Stage::Resolve, Stage::Shape, Stage::Place] {
            self.deps.clear(Dependent::new(id, stage));
        }
        self.label_hosts.retain(|_, host| *host != id);
        if let Some(node) = self.nodes.get_mut(&unit.node) {
            if let Some(at) = node.units.iter().position(|&u| u == id) {
                node.units.swap_remove(at);
            }
        }
        self.mark(work, Input::Geometry(id));
    }

    /// Take the node `top` and every projected node below it out of the
    /// projection: their groups out of every layer, journaled, and their
    /// state, their units' and what they read out of the projector; mark
    /// what read their visibility — the links that lead to them. The
    /// nodes below are found from `top`'s group in the sections layer,
    /// which holds a group for each of them: what the projector wrote,
    /// read back to find its own state, never to learn the document.
    /// O(the removed groups and runs).
    pub(crate) fn remove_subtree(
        &mut self,
        top: NodeId,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
        work: &mut Work,
    ) {
        let Some(state) = self.nodes.get(&top) else {
            return;
        };
        let groups = state.groups;
        let mut gone = vec![top];
        if let Some(group) = groups[Layer::Sections.index()] {
            let arena = layers.sections.arena();
            gone.extend(group.descendants(arena).skip(1).filter_map(|g| {
                match self.owners.get(&(Layer::Sections, g)) {
                    Some(Owner::Group(node)) => Some(*node),
                    _ => None,
                }
            }));
        }
        for layer in Layer::ALL {
            if let Some(group) = groups[layer.index()] {
                write::remove(layers, out, layer, group);
            }
        }
        for node in gone {
            let Some(state) = self.nodes.remove(&node) else {
                continue;
            };
            for unit in state.units {
                self.drop_unit(unit, false, layers, out, work);
            }
            for layer in Layer::ALL {
                if let Some(group) = state.groups[layer.index()] {
                    self.owners.remove(&(layer, group));
                }
            }
            self.deps.clear(Dependent::new(node, Stage::Carry));
            self.deps.clear(Dependent::new(node, Stage::Geometry));
            self.bodies.remove(node);
            self.mark(work, Input::Visibility(node));
            self.mark(work, Input::Geometry(node));
        }
    }

    /// The structural step of an update (#241 §5.1): take out what stopped
    /// being drawn, fold and unfold, project what started, re-parent what
    /// moved, and create and drop units as their elements come and go.
    /// O(the touches, and the subtrees that appeared or disappeared).
    pub(crate) fn restructure(
        &mut self,
        tree: &Tree<DocElement>,
        touched: &Touched,
        work: &mut Work,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
    ) {
        // 1. Nodes no longer drawn: removed, hidden, or under a node that
        //    is.
        for &id in &touched.ids {
            if self.nodes.contains_key(&id) && !self.shows(tree, id) {
                self.remove_subtree(id, layers, out, work);
            }
        }
        // 2. Folding: a node that folds takes its children's subtrees out;
        //    one that unfolds puts the drawn ones back.
        let mut candidates: Vec<NodeId> = Vec::new();
        for &id in &touched.ids {
            let Some(DocElement::Node(data)) = element(tree, id) else {
                continue;
            };
            let Some(state) = self.nodes.get_mut(&id) else {
                continue;
            };
            if state.folded == data.folded {
                continue;
            }
            state.folded = data.folded;
            let children: Vec<NodeId> = id
                .children(tree.arena())
                .filter(|&c| tree.arena()[c].get().kind() == DocKind::Node)
                .collect();
            if data.folded {
                for child in children {
                    self.remove_subtree(child, layers, out, work);
                }
            } else {
                candidates.extend(children);
            }
        }
        // 3. Nodes now drawn and not yet projected, with their subtrees.
        for &id in touched.ids.iter().chain(candidates.iter()) {
            if !self.nodes.contains_key(&id) && self.shows(tree, id) {
                self.create_subtree(tree, id, work);
            }
        }
        // 4. Nodes kept that changed parent, or moved within their row.
        for &id in &touched.ids {
            if work.created.contains(&id) || !self.nodes.contains_key(&id) {
                continue;
            }
            let parent = parent_node(tree, id);
            let state_parent = self.nodes.get(&id).and_then(|s| s.parent);
            if parent != state_parent || touched.moved.contains(&id) {
                self.reparent(tree, id, parent, work);
                work.moves.push(id);
            } else {
                work.reorders.push(id);
            }
        }
        // 5. Units that came or went, or moved to another node.
        for &id in &touched.ids {
            if self.nodes.contains_key(&id) {
                continue;
            }
            let owner = element(tree, id).and_then(|_| crate::document::ids::owning_node(tree, id));
            if let Some(unit) = self.units.get(&id) {
                if owner != Some(unit.node) {
                    self.drop_unit(id, true, layers, out, work);
                } else {
                    work.unit_reorders.push(id);
                }
            }
            if let Some(owner) = owner {
                if self.nodes.contains_key(&owner) && !self.units.contains_key(&id) {
                    self.create_unit(tree, id, owner, work);
                }
            }
        }
    }

    /// Hang the projected node `id` under `parent` (`None`: the map): its
    /// depth and its projected descendants' follow, each marked to carry
    /// again — what a re-parent changes for a subtree (#241 §5.1). O(the
    /// projected subtree).
    fn reparent(&mut self, tree: &Tree<DocElement>, id: NodeId, parent: Option<NodeId>, work: &mut Work) {
        let depth = parent.and_then(|p| self.nodes.get(&p)).map_or(0, |p| p.depth + 1);
        if let Some(state) = self.nodes.get_mut(&id) {
            state.parent = parent;
        }
        let arena = tree.arena();
        let mut stack = vec![(id, depth)];
        while let Some((at, depth)) = stack.pop() {
            let Some(state) = self.nodes.get_mut(&at) else {
                continue;
            };
            state.depth = depth;
            self.queue(work, Dependent::new(at, Stage::Carry));
            for child in at.children(arena) {
                if self.nodes.contains_key(&child) {
                    stack.push((child, depth + 1));
                }
            }
        }
    }

    /// Mark the readers of what `changes` touched (#241 §5.2): the
    /// readers of every input kind a record names (ruling B7.2), of every
    /// rearranged row, and each touched element's own projection — a
    /// node's carry and geometry, the unit a span, section or path is.
    /// O(the records).
    pub(crate) fn seed(
        &mut self,
        tree: &Tree<DocElement>,
        changes: &[ChangeRecord],
        touched: &Touched,
        work: &mut Work,
    ) {
        for record in changes {
            for &(id, kind) in &record.inputs {
                let input = match kind {
                    InputKind::Style => Some(DocInput::Style(id)),
                    InputKind::Placement => Some(DocInput::Placement(id)),
                    InputKind::PaletteBinding => Some(DocInput::PaletteBinding(id)),
                    InputKind::ParentRow => tree.node(id).and_then(|n| n.parent()).map(DocInput::Row),
                    InputKind::Theme => Some(DocInput::Theme),
                    InputKind::Palette => palette_id_of(tree, id).map(DocInput::Palette),
                    InputKind::Defaults => Some(DocInput::Defaults),
                };
                if let Some(input) = input {
                    self.mark(work, Input::Doc(input));
                }
            }
            for &row in &record.rows {
                self.mark(work, Input::Doc(DocInput::Row(row)));
            }
        }
        for &id in &touched.ids {
            if element(tree, id).is_none() {
                continue;
            }
            if self.nodes.contains_key(&id) {
                self.queue(work, Dependent::new(id, Stage::Carry));
                self.queue(work, Dependent::new(id, Stage::Geometry));
            } else if let Some(unit) = self.own_unit(tree, id) {
                self.queue(work, Dependent::new(unit, Stage::Resolve));
            }
        }
    }

    /// The unit whose projection draws `id`: itself, or the nearest unit
    /// above it within its node — a span's section, ring, path or portal,
    /// a cap's path. A climb of at most the few levels a node's elements
    /// nest, never past its node. `None` for an element no unit draws.
    fn own_unit(&self, tree: &Tree<DocElement>, id: NodeId) -> Option<NodeId> {
        let mut at = id;
        loop {
            if self.units.contains_key(&at) {
                return Some(at);
            }
            if element(tree, at)?.kind() == DocKind::Node {
                return None;
            }
            at = tree.node(at)?.parent()?;
        }
    }
}

use crate::projection::write;
