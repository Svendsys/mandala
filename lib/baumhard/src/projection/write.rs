// SPDX-License-Identifier: MPL-2.0

//! Writing the layers (#241 §5.6): a full projection appends every
//! element in pre-order; an update emits, journaled per layer, only what
//! differs from what the projector last wrote — a `RunSet` where a run's
//! data changed, a structural command where an element appeared, left or
//! moved.

use rustc_hash::FxHashSet;

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::{Placement, Tree};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::run::GlyphRunData;
use crate::gfx_structs::tree::GfxMutatorTree;
use crate::projection::layers::{GlyphLayers, Layer, LayerBuilders, CHILD_GROUP_CHANNEL, RUN_CHANNEL};
use crate::projection::stages::PhaseWork;
use crate::projection::state::{Owner, ProjectedUnit, UnitBody};
use crate::projection::structure::{parent_node, units_in_order};
use crate::projection::Projector;

/// Apply `mutator` at `target` in `layer`, journaled. Logs and reports a
/// refusal, which nothing the projector builds should meet.
fn apply(
    layers: &mut GlyphLayers,
    out: &mut LayerBuilders,
    layer: Layer,
    target: NodeId,
    mutator: GfxMutatorTree,
) -> bool {
    let tree = layers.tree_mut(layer);
    match mutator.apply_at_journaled(tree, &Anchors::Nodes(vec![target]), |_, _| (), out.of(layer)) {
        Ok(()) => true,
        Err(error) => {
            log::error!("projection: a write to the {layer:?} layer was refused: {error}");
            debug_assert!(
                false,
                "projection: a write to the {layer:?} layer was refused: {error}"
            );
            false
        }
    }
}

/// A structural command on one target, as a mutator tree.
fn command(op: StructOp<GfxElement>) -> GfxMutatorTree {
    GfxMutatorTree::new_with(GfxMutator::new_structure(op, RUN_CHANNEL))
}

/// The element a run of the unit `unit_id` is drawn for, as a reader of
/// the update asks it ([`LayerChanges::removed_owner`]): a section's or a
/// ring's node, and a connector's or a link's own element. Exhaustive
/// over [`UnitBody`], so a new kind of unit decides it. O(1).
///
/// [`LayerChanges::removed_owner`]: crate::projection::LayerChanges::removed_owner
pub(crate) fn run_owner(unit_id: NodeId, unit: &ProjectedUnit) -> NodeId {
    match unit.body {
        UnitBody::Section(_) | UnitBody::Ring(_) => unit.node,
        UnitBody::Line(_) | UnitBody::Portal(_) => unit_id,
    }
}

/// Remove `glyph` and its subtree from `layer`, journaled: detached, the
/// layer's journal owning it until discarded. O(1).
pub(crate) fn remove(layers: &mut GlyphLayers, out: &mut LayerBuilders, layer: Layer, glyph: NodeId) {
    apply(layers, out, layer, glyph, command(StructOp::Remove));
}

/// Insert `subtree` under `parent` in `layer` at `at`, journaled, and
/// return the id of its top. O(the subtree).
fn insert(
    layers: &mut GlyphLayers,
    out: &mut LayerBuilders,
    layer: Layer,
    parent: NodeId,
    subtree: Subtree<GfxElement>,
    at: Placement,
) -> Option<NodeId> {
    if !apply(
        layers,
        out,
        layer,
        parent,
        command(StructOp::Insert { subtree, at }),
    ) {
        return None;
    }
    out.of(layer).created_nodes().last()
}

/// Move `glyph` under `to` in `layer` at `at`, journaled.
fn move_to(
    layers: &mut GlyphLayers,
    out: &mut LayerBuilders,
    layer: Layer,
    glyph: NodeId,
    to: NodeId,
    at: Placement,
) {
    apply(layers, out, layer, glyph, command(StructOp::Move { to, at }));
}

/// Set the run `glyph` of `layer` to `data`, journaled.
fn set_run(
    layers: &mut GlyphLayers,
    out: &mut LayerBuilders,
    layer: Layer,
    glyph: NodeId,
    data: GlyphRunData,
) {
    let mutator = GfxMutatorTree::new_with(GfxMutator::new(GfxMutation::RunSet(Box::new(data)), RUN_CHANNEL));
    apply(layers, out, layer, glyph, mutator);
}

/// A group element. No allocation.
fn group() -> GfxElement {
    GfxElement::new_void(CHILD_GROUP_CHANNEL)
}

/// A run element of `data`. One allocation, for the box.
fn run(data: GlyphRunData) -> GfxElement {
    GfxElement::new_run(data, RUN_CHANNEL, 0)
}

/// Where `element` goes among its parent's children with an equal key:
/// right after `pred`, or first.
fn after(pred: Option<NodeId>) -> Placement {
    pred.map_or(Placement::First, Placement::After)
}

/// One element of a subtree to write: its depth below the subtree's top,
/// the element, and whose it is.
type Planned = (usize, GfxElement, Owner);

impl Projector {
    /// The elements of `top`'s subtree in `layer`, in pre-order: each node
    /// that has no group yet — `top`, and every projected node below it
    /// not moved there from elsewhere — then its runs in document order,
    /// then its child nodes'. Iterates with a heap stack. O(the subtree).
    fn plan(&self, tree: &Tree<DocElement>, top: NodeId, layer: Layer) -> Vec<Planned> {
        let arena = tree.arena();
        let mut out = Vec::new();
        let mut stack = vec![(top, 0usize)];
        while let Some((node, depth)) = stack.pop() {
            if !self.nodes.contains_key(&node) {
                continue;
            }
            out.push((depth, group(), Owner::Group(node)));
            for unit_id in units_in_order(tree, node) {
                let Some(unit) = self.units.get(&unit_id) else {
                    continue;
                };
                for (slot, s) in unit.slots.iter().enumerate() {
                    if s.layer == layer {
                        if let Some(want) = &s.want {
                            out.push((depth + 1, run(want.clone()), Owner::Run { unit: unit_id, slot }));
                        }
                    }
                }
            }
            let first = stack.len();
            for child in node.children(arena) {
                if arena[child].get().kind() != DocKind::Node {
                    continue;
                }
                if self
                    .nodes
                    .get(&child)
                    .is_some_and(|c| c.groups[layer.index()].is_none())
                {
                    stack.push((child, depth + 1));
                }
            }
            stack[first..].reverse();
        }
        out
    }

    /// Note that `glyph` of `layer` is `owner`'s: the reverse map, and the
    /// state that names it.
    fn adopt(&mut self, layer: Layer, glyph: NodeId, owner: Owner) {
        self.owners.insert((layer, glyph), owner);
        match owner {
            Owner::Group(node) => {
                if let Some(state) = self.nodes.get_mut(&node) {
                    state.groups[layer.index()] = Some(glyph);
                }
            }
            Owner::Run { unit, slot } => {
                if let Some(s) = self.units.get_mut(&unit).and_then(|u| u.slots.get_mut(slot)) {
                    s.glyph = Some(glyph);
                    s.written = s.want.clone();
                }
            }
        }
    }

    /// Write every projected node into fresh `layers`, in pre-order, each
    /// element appended where it goes: a group, its runs, then its child
    /// groups. O(the projection); every insert is an append (§B4).
    pub(crate) fn write_full(&mut self, tree: &Tree<DocElement>, layers: &mut GlyphLayers) {
        let arena = tree.arena();
        let roots: Vec<NodeId> = tree
            .root()
            .children(arena)
            .filter(|&c| self.nodes.contains_key(&c))
            .collect();
        for layer in Layer::ALL {
            for &root in &roots {
                let planned = self.plan(tree, root, layer);
                let mut parents: Vec<NodeId> = Vec::new();
                for (depth, element, owner) in planned {
                    parents.truncate(depth);
                    let parent = parents
                        .last()
                        .copied()
                        .unwrap_or_else(|| layers.tree(layer).root());
                    let Ok(glyph) = layers
                        .tree_mut(layer)
                        .insert_child(parent, element, Placement::Last)
                    else {
                        continue;
                    };
                    if matches!(owner, Owner::Group(_)) {
                        parents.push(glyph);
                    }
                    self.adopt(layer, glyph, owner);
                }
            }
        }
    }

    /// The group in `layer` the node `node` goes right after: its nearest
    /// earlier sibling node whose group is already under `under`. O(the
    /// siblings walked back over).
    fn group_pred(
        &self,
        tree: &Tree<DocElement>,
        node: NodeId,
        layer: Layer,
        under: NodeId,
        layers: &GlyphLayers,
    ) -> Option<NodeId> {
        let arena = tree.arena();
        let glyphs = layers.tree(layer).arena();
        let mut at = arena.node(node)?.previous_sibling();
        while let Some(prev) = at {
            if arena[prev].get().kind() != DocKind::Node {
                return None;
            }
            if let Some(g) = self.nodes.get(&prev).and_then(|s| s.groups[layer.index()]) {
                if glyphs.node(g).and_then(|n| n.parent()) == Some(under) {
                    return Some(g);
                }
            }
            at = arena.at(prev).previous_sibling();
        }
        None
    }

    /// The group `node`'s groups hang under in `layer`: its parent's, or
    /// the layer's root.
    fn group_parent(
        &self,
        tree: &Tree<DocElement>,
        node: NodeId,
        layer: Layer,
        layers: &GlyphLayers,
    ) -> Option<NodeId> {
        match parent_node(tree, node) {
            Some(parent) => self.nodes.get(&parent)?.groups[layer.index()],
            None => Some(layers.tree(layer).root()),
        }
    }

    /// The run in `layer` slot `slot` of unit `unit` goes right after,
    /// under its node's group `under`: the unit's own earlier slot in the
    /// layer, else the last run in the layer of the nearest earlier
    /// sibling unit. O(the siblings walked back over).
    fn slot_pred(
        &self,
        tree: &Tree<DocElement>,
        unit_id: NodeId,
        slot: usize,
        under: NodeId,
        layers: &GlyphLayers,
    ) -> Option<NodeId> {
        let unit = self.units.get(&unit_id)?;
        let layer = unit.slots[slot].layer;
        let glyphs = layers.tree(layer).arena();
        let in_place = |g: NodeId| glyphs.node(g).and_then(|n| n.parent()) == Some(under);
        if let Some(g) = unit.slots[..slot]
            .iter()
            .rev()
            .filter(|s| s.layer == layer)
            .find_map(|s| s.glyph.filter(|&g| in_place(g)))
        {
            return Some(g);
        }
        let arena = tree.arena();
        let mut at = arena.node(unit_id)?.previous_sibling();
        while let Some(prev) = at {
            if let Some(other) = self.units.get(&prev) {
                if let Some(g) = other
                    .slots
                    .iter()
                    .rev()
                    .filter(|s| s.layer == layer)
                    .find_map(|s| s.glyph.filter(|&g| in_place(g)))
                {
                    return Some(g);
                }
            }
            at = arena.at(prev).previous_sibling();
        }
        None
    }

    /// Whether `glyph` already stands right after `pred` among the
    /// children of its key — first among them when `pred` is `None`. O(1).
    fn placed_after(layers: &GlyphLayers, layer: Layer, glyph: NodeId, pred: Option<NodeId>) -> bool {
        let arena = layers.tree(layer).arena();
        let Some(prev) = arena.node(glyph).map(|n| n.previous_sibling()) else {
            return false;
        };
        match pred {
            Some(p) => prev == Some(p),
            None => prev.is_none_or(|s| arena[s].get().channel() != arena[glyph].get().channel()),
        }
    }

    /// The write of an update: the elements of each subtree that became
    /// visible, the moves of nodes re-parented or reordered, then every
    /// unit whose stages ran — a run removed, inserted, moved or set only
    /// where what it wants differs from what was written.
    pub(crate) fn write_update(
        &mut self,
        tree: &Tree<DocElement>,
        work: &mut PhaseWork,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
    ) {
        // Runs of a kind a link left.
        let retiring: Vec<NodeId> = work.units_run.iter().copied().collect();
        for unit_id in &retiring {
            let (retired, owner) = match self.units.get_mut(unit_id) {
                Some(unit) => (std::mem::take(&mut unit.retired), run_owner(*unit_id, unit)),
                None => continue,
            };
            for slot in retired {
                if let Some(glyph) = slot.glyph {
                    out.removed(slot.layer, glyph, owner);
                    self.owners.remove(&(slot.layer, glyph));
                    remove(layers, out, slot.layer, glyph);
                }
            }
        }
        // Subtrees that became visible, whole.
        let roots = std::mem::take(&mut work.pending_roots);
        for root in roots {
            for layer in Layer::ALL {
                if self
                    .nodes
                    .get(&root)
                    .is_none_or(|s| s.groups[layer.index()].is_some())
                {
                    continue;
                }
                let Some(under) = self.group_parent(tree, root, layer, layers) else {
                    continue;
                };
                let pred = self.group_pred(tree, root, layer, under, layers);
                let planned = self.plan(tree, root, layer);
                let owners: Vec<Owner> = planned.iter().map(|(_, _, o)| *o).collect();
                let subtree = build_subtree(planned);
                let Some(top) = insert(layers, out, layer, under, subtree, after(pred)) else {
                    continue;
                };
                let glyphs: Vec<NodeId> = top.descendants(layers.tree(layer).arena()).collect();
                for (glyph, owner) in glyphs.into_iter().zip(owners) {
                    self.adopt(layer, glyph, owner);
                }
            }
        }
        // Nodes re-parented, then nodes touched in place: each placed
        // after its nearest earlier sibling, the earlier ones first.
        let moves = std::mem::take(&mut work.moves);
        let reorders = std::mem::take(&mut work.reorders);
        let pending: FxHashSet<NodeId> = moves.iter().chain(reorders.iter()).copied().collect();
        let mut settled: FxHashSet<NodeId> = FxHashSet::default();
        for &node in moves.iter().chain(reorders.iter()) {
            self.settle_node(tree, node, &pending, &mut settled, layers, out);
        }
        // Units: what each wants against what was written.
        let mut units: Vec<NodeId> = work.units_run.iter().copied().collect();
        units.extend(work.unit_reorders.iter().copied());
        let reordering: FxHashSet<NodeId> = units.iter().copied().collect();
        let mut done: FxHashSet<NodeId> = FxHashSet::default();
        for unit in units {
            self.settle_unit(tree, unit, &reordering, &mut done, layers, out);
        }
    }

    /// Put `node`'s groups where they go, after first settling any earlier
    /// sibling also waiting in `pending`: placed in document order, each
    /// right after the one before it.
    fn settle_node(
        &mut self,
        tree: &Tree<DocElement>,
        node: NodeId,
        pending: &FxHashSet<NodeId>,
        settled: &mut FxHashSet<NodeId>,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
    ) {
        let mut stack = vec![node];
        while let Some(&at) = stack.last() {
            if settled.contains(&at) {
                stack.pop();
                continue;
            }
            // An earlier sibling still waiting goes first.
            let arena = tree.arena();
            let mut prev = arena.node(at).and_then(|n| n.previous_sibling());
            let mut waiting = None;
            while let Some(p) = prev {
                if arena[p].get().kind() != DocKind::Node {
                    break;
                }
                if pending.contains(&p) && !settled.contains(&p) {
                    waiting = Some(p);
                    break;
                }
                if self.nodes.contains_key(&p) {
                    break;
                }
                prev = arena.at(p).previous_sibling();
            }
            if let Some(p) = waiting {
                stack.push(p);
                continue;
            }
            settled.insert(at);
            stack.pop();
            for layer in Layer::ALL {
                let Some(glyph) = self.nodes.get(&at).and_then(|s| s.groups[layer.index()]) else {
                    continue;
                };
                let Some(under) = self.group_parent(tree, at, layer, layers) else {
                    continue;
                };
                let pred = self.group_pred(tree, at, layer, under, layers);
                let parent = layers.tree(layer).arena().node(glyph).and_then(|n| n.parent());
                if parent == Some(under) && Self::placed_after(layers, layer, glyph, pred) {
                    continue;
                }
                move_to(layers, out, layer, glyph, under, after(pred));
            }
        }
    }

    /// Write unit `unit`'s slots: remove, insert, move or set each run
    /// where what it wants differs from what was written, after first
    /// settling an earlier sibling unit also waiting in `pending`.
    fn settle_unit(
        &mut self,
        tree: &Tree<DocElement>,
        unit: NodeId,
        pending: &FxHashSet<NodeId>,
        done: &mut FxHashSet<NodeId>,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
    ) {
        let mut stack = vec![unit];
        while let Some(&at) = stack.last() {
            if done.contains(&at) || !self.units.contains_key(&at) {
                done.insert(at);
                stack.pop();
                continue;
            }
            let arena = tree.arena();
            let mut prev = arena.node(at).and_then(|n| n.previous_sibling());
            let mut waiting = None;
            while let Some(p) = prev {
                if pending.contains(&p) && !done.contains(&p) && self.units.contains_key(&p) {
                    waiting = Some(p);
                    break;
                }
                prev = arena.at(p).previous_sibling();
            }
            if let Some(p) = waiting {
                stack.push(p);
                continue;
            }
            done.insert(at);
            stack.pop();
            self.write_slots(tree, at, layers, out);
        }
    }

    /// [`Self::settle_unit`]'s write of one unit's slots.
    fn write_slots(
        &mut self,
        tree: &Tree<DocElement>,
        unit_id: NodeId,
        layers: &mut GlyphLayers,
        out: &mut LayerBuilders,
    ) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let node = unit.node;
        let owner = run_owner(unit_id, unit);
        for slot in 0..unit_slot_count(self, unit_id) {
            let Some(s) = self.units.get(&unit_id).map(|u| &u.slots[slot]) else {
                return;
            };
            let layer = s.layer;
            let Some(under) = self.nodes.get(&node).and_then(|n| n.groups[layer.index()]) else {
                continue;
            };
            match (s.glyph, s.want.clone()) {
                (Some(glyph), None) => {
                    out.removed(layer, glyph, owner);
                    self.owners.remove(&(layer, glyph));
                    remove(layers, out, layer, glyph);
                    if let Some(s) = self.units.get_mut(&unit_id).map(|u| &mut u.slots[slot]) {
                        s.glyph = None;
                        s.written = None;
                    }
                }
                (None, Some(want)) => {
                    let pred = self.slot_pred(tree, unit_id, slot, under, layers);
                    let Some(glyph) =
                        insert(layers, out, layer, under, Subtree::leaf(run(want)), after(pred))
                    else {
                        continue;
                    };
                    self.adopt(layer, glyph, Owner::Run { unit: unit_id, slot });
                }
                (Some(glyph), Some(want)) => {
                    let pred = self.slot_pred(tree, unit_id, slot, under, layers);
                    if !Self::placed_after(layers, layer, glyph, pred) {
                        move_to(layers, out, layer, glyph, under, after(pred));
                    }
                    let same = s.written.as_ref().is_some_and(|w| w.same_state(&want));
                    if !same {
                        set_run(layers, out, layer, glyph, want.clone());
                        if let Some(s) = self.units.get_mut(&unit_id).map(|u| &mut u.slots[slot]) {
                            s.written = Some(want);
                        }
                    }
                }
                (None, None) => {}
            }
        }
    }
}

/// How many slots unit `id` has.
fn unit_slot_count(projector: &Projector, id: NodeId) -> usize {
    projector.units.get(&id).map_or(0, |u| u.slots.len())
}

/// The subtree a pre-order plan describes, built with a heap stack: each
/// element's parent is the nearest element before it one level up.
fn build_subtree(planned: Vec<Planned>) -> Subtree<GfxElement> {
    let mut stack: Vec<Subtree<GfxElement>> = Vec::new();
    for (depth, element, _) in planned {
        while stack.len() > depth {
            let done = stack.pop().expect("a stack longer than a depth is not empty");
            if let Some(parent) = stack.last_mut() {
                parent.children.push(done);
            }
        }
        stack.push(Subtree::leaf(element));
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("a stack of two is not empty");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(done);
        }
    }
    stack
        .pop()
        .unwrap_or_else(|| Subtree::leaf(GfxElement::new_void(CHILD_GROUP_CHANNEL)))
}
