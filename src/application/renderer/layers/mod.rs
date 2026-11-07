// SPDX-License-Identifier: MPL-2.0

//! The layer renderer: the projection's glyph layers (`GlyphLayers`,
//! V1-B7) drawn through the positioned-glyph pass, with render state kept
//! **per glyph element** and updated **only where a journal says the
//! layers changed** (design §13.1, V1-C2 #243).
//!
//! # What it keeps
//!
//! For each of the five layers, a mirror of the layer's tree
//! ([`mirror`]): one slot per element, linked as the elements are, so the
//! layer's pre-order — the order its runs are drawn in, each over the
//! ones before it — is a walk of the links. A `GlyphRun`'s slot holds the
//! run as the layer holds it, its shaping and placement by `Arc`: keeping
//! it copies pointers, and the renderer never shapes. A group (`Void`)
//! holds nothing but its place.
//!
//! # Three entry points
//!
//! - [`LayerRenderer::load`]: the whole layers, once per document load —
//!   one walk of each, the only one.
//! - [`LayerRenderer::apply`]: after every projector update, the journals
//!   of what changed — the projector's `LayerChanges`, merged with those
//!   of view-only glyph mutators and glyph-layer presentations (#243
//!   amendment 3) — and nothing else. Each element a journal names is
//!   visited once however many name it, the last touch deciding whether
//!   it is still in its layer:
//!   - an element that left is dropped, with every slot under it;
//!   - an element that entered gets a slot, read from the layer;
//!   - an element that changed is compared with what the slot holds, and
//!     only a difference is written: a new placement or origin is the
//!     same shaping re-placed, an opacity, override or zoom window a new
//!     value — nothing is reshaped, and an unchanged run keeps its state
//!     and its stamp;
//!   - an element whose place changed — moved, or a sibling reordered
//!     into it — is spliced back in after its previous sibling, under its
//!     parent, as the layer now holds it.
//!
//!   No element outside the journals and their subtrees is visited, and
//!   no layer is walked: the cost is the touches, the subtrees removals
//!   take out, and O(1) per splice.
//! - [`LayerRenderer::encode`]: every frame, the runs in draw order —
//!   sections, frames, connections, labels, portals (`Layer::ALL`), each
//!   layer in tree order — handed to the frame's positioned-glyph pass,
//!   which applies the camera and evaluates each run's zoom window and
//!   cull **at draw time** (`glyph_runs::emit`). It takes `&self`: a pan
//!   or a zoom never calls `apply` and cannot write render state.
//!
//! The GPU side is the frame's: a glyph's rasterization depends on the
//! zoom and on the pen's subpixel position, so instances are built per
//! frame from the state kept here, as the decision record says (design
//! §13.2), into the one instanced draw the pass makes.
//!
//! The layers are drawn after the canvas trees' runs and under the
//! screen-space chrome (the console or the color picker over its
//! backdrop, the FPS readout, the mode line).

mod mirror;
#[cfg(test)]
mod tests;

use baumhard::arena::{Arena, NodeId};
use baumhard::engine::journal::{Journal, Touch};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::{GlyphLayers, Layer, LayerChanges};
use glam::Vec2;
use log::warn;
use rustc_hash::FxHashMap;

use self::mirror::{Mirror, SlotId};
use super::glyph_runs::GlyphRunFrame;

pub(crate) use self::mirror::RunState;

/// The render state of the five glyph layers. See the [module
/// docs](self).
#[derive(Default)]
pub(crate) struct LayerRenderer {
    /// One mirror per layer, indexed by `Layer::index`.
    mirrors: [Mirror; 5],
    /// The count every write of a run's state takes the next value of:
    /// its [`RunState`] stamp.
    stamp: u64,
    /// What [`Self::apply`] collects and works through, kept so a steady
    /// stream of updates allocates nothing once it has grown.
    scratch: Scratch,
    /// What the last [`Self::apply`] did, and how many calls there were.
    /// Test builds only: the product keeps no log of its work.
    #[cfg(test)]
    log: ApplyLog,
}

/// [`LayerRenderer::apply`]'s working space.
#[derive(Default)]
struct Scratch {
    /// Per layer, the last touch of each element the journals name.
    touches: [FxHashMap<NodeId, Touch>; 5],
    /// Per layer, those elements in the order they were first named.
    order: [Vec<NodeId>; 5],
    /// Slots waiting to be put in their row.
    pending: Vec<SlotId>,
    /// The walk of a removed subtree, and the chain of rows a splice
    /// waits on.
    stack: Vec<SlotId>,
}

/// What one [`LayerRenderer::apply`] did: the counts the tests that hold
/// it to the journals assert (TEST_CONVENTIONS §T8; never timings).
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ApplyLog {
    /// How many times [`LayerRenderer::apply`] has been called.
    pub(crate) applies: u64,
    /// Every element the last call visited, each once, by layer, in the
    /// order the journals first named it.
    pub(crate) visited: Vec<(Layer, NodeId)>,
    /// Slots made for elements that entered.
    pub(crate) inserted: usize,
    /// Slots freed for elements that left, their subtrees included.
    pub(crate) freed: usize,
    /// Existing slots spliced into another place in their layer.
    pub(crate) relinked: usize,
    /// Elements still in their layer whose state already matched it.
    pub(crate) unchanged: usize,
    /// Runs whose shaping the projector replaced: adopted, never shaped
    /// here.
    pub(crate) reshaped: usize,
    /// Runs re-placed: the same shaped run, a new placement.
    pub(crate) replaced: usize,
    /// Runs whose origin moved.
    pub(crate) moved: usize,
    /// Runs whose opacity, overrides or zoom window changed.
    pub(crate) restyled: usize,
}

impl LayerRenderer {
    /// The render state of `layers` as a whole: what a document load
    /// does, replacing whatever was kept. Every run gets a fresh state.
    ///
    /// Cost: one walk of each layer — O(its elements) — one slot and one
    /// index entry per element, one clone of each run (its `Arc`s, and its
    /// override vectors when it has any).
    pub(crate) fn load(&mut self, layers: &GlyphLayers) {
        for layer in Layer::ALL {
            let tree = layers.tree(layer);
            let stamp = &mut self.stamp;
            self.mirrors[layer.index()] = Mirror::of(tree, |id| run_state(tree, id, stamp));
        }
    }

    /// Bring the render state up to date with `layers` after the changes
    /// `journals` record, each naming the layer it is of: the projector's
    /// ([`Self::journals_of`]), and those of the glyph mutators applied
    /// over them, in the order they were applied. See the [module
    /// docs](self) for what each touch does.
    ///
    /// The journals must be those of every change made to `layers` since
    /// the last `load` or `apply`, read before they are discarded
    /// (`LayerChanges::discard`): an element a journal does not name is
    /// not looked at, so a change no journal records is not drawn.
    ///
    /// Cost: O(the elements the journals name, and the slots under the
    /// removed ones); no walk of a layer and no comparison of an element
    /// no journal names.
    pub(crate) fn apply<'j>(
        &mut self,
        layers: &GlyphLayers,
        journals: impl IntoIterator<Item = (Layer, &'j Journal<GfxElement>)>,
    ) {
        #[cfg(test)]
        {
            self.log = ApplyLog {
                applies: self.log.applies + 1,
                ..ApplyLog::default()
            };
        }
        for (layer, journal) in journals {
            let i = layer.index();
            let (touches, order) = (&mut self.scratch.touches[i], &mut self.scratch.order[i]);
            for (id, touch) in journal.touched() {
                if touches.insert(id, touch).is_none() {
                    order.push(id);
                }
            }
        }
        for layer in Layer::ALL {
            self.apply_layer(layer, layers.tree(layer));
        }
    }

    /// The projector's journals, each with its layer: what
    /// [`Self::apply`] takes for an update with nothing applied over it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the app's projector is wired to the layer renderer by V1-C3 (#244)"
        )
    )]
    pub(crate) fn journals_of(changes: &LayerChanges) -> impl Iterator<Item = (Layer, &Journal<GfxElement>)> {
        Layer::ALL
            .into_iter()
            .map(|layer| (layer, changes.journal(layer)))
    }

    /// Hand every run to `frame`, in draw order: the five layers in
    /// `Layer::ALL`'s order, each in its pre-order. The frame's pass
    /// applies its camera, evaluates each run's zoom window and culls it
    /// (`glyph_runs::emit`); nothing here depends on the camera.
    ///
    /// Cost: O(the layers' elements) to walk, and the pass's O(glyphs) for
    /// each run it draws; no allocation.
    pub(crate) fn encode<'r>(&'r self, frame: &mut GlyphRunFrame<'_, 'r>) {
        for run in self.runs() {
            frame.draw(run, Vec2::ZERO);
        }
    }

    /// Every run kept, in draw order ([`Self::encode`]'s).
    pub(crate) fn runs(&self) -> impl Iterator<Item = &GlyphRunData> {
        self.mirrors.iter().flat_map(Mirror::runs).map(RunState::run)
    }

    /// What the last [`Self::apply`] did. Test builds only.
    #[cfg(test)]
    pub(crate) fn log(&self) -> &ApplyLog {
        &self.log
    }

    /// The state kept for `glyph` of `layer`, if it is a run the renderer
    /// holds. Test builds only.
    #[cfg(test)]
    pub(crate) fn state(&self, layer: Layer, glyph: NodeId) -> Option<&RunState> {
        let mirror = &self.mirrors[layer.index()];
        mirror.slot(mirror.slot_of(glyph)?).state.as_ref()
    }

    /// Whether the render state is exactly `layers`: each mirror holds
    /// its layer's elements in the layer's pre-order, and each run's state
    /// is the run its element holds (`GlyphRunData::same_state`). `Err`
    /// names the first difference. Test builds only: what the tests that
    /// hold `apply` to a fresh `load` compare by, reading the layers
    /// themselves and not their journals.
    ///
    /// Cost: one walk of each layer and of each mirror.
    #[cfg(test)]
    pub(crate) fn matches(&self, layers: &GlyphLayers) -> Result<(), String> {
        for layer in Layer::ALL {
            let tree = layers.tree(layer);
            let mirror = &self.mirrors[layer.index()];
            let want: Vec<NodeId> = tree.root().descendants(tree.arena()).collect();
            let have = mirror.ids();
            if want != have {
                return Err(format!(
                    "{layer:?}: the layer's pre-order is {want:?}, the renderer's {have:?}"
                ));
            }
            if mirror.len() != want.len() {
                return Err(format!(
                    "{layer:?}: {} elements in the layer, {} kept",
                    want.len(),
                    mirror.len()
                ));
            }
            for id in want {
                let element = tree.get(id).map(|node| node.get().glyph_run());
                let kept = self.state(layer, id).map(RunState::run);
                match (element.flatten(), kept) {
                    (None, None) => {}
                    (Some(a), Some(b)) if a.same_state(b) => {}
                    (a, b) => {
                        return Err(format!(
                            "{layer:?} {id:?}: the layer holds {a:?}, the renderer {b:?}"
                        ))
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply the touches collected for `layer` against `tree`, the layer
    /// as it now stands: drop what left, write what entered or changed,
    /// then splice everything whose place changed into its row.
    fn apply_layer(&mut self, layer: Layer, tree: &GfxTree) {
        let i = layer.index();
        let mut order = std::mem::take(&mut self.scratch.order[i]);
        let mut touches = std::mem::take(&mut self.scratch.touches[i]);
        let mirror = &mut self.mirrors[i];
        let (pending, stack) = (&mut self.scratch.pending, &mut self.scratch.stack);
        pending.clear();
        #[cfg(test)]
        let log = &mut self.log;

        for id in &order {
            #[cfg(test)]
            log.visited.push((layer, *id));
            if touches.get(id) == Some(&Touch::Removed) {
                if let Some(slot) = mirror.slot_of(*id) {
                    let _freed = mirror.free_subtree(slot, stack);
                    #[cfg(test)]
                    {
                        log.freed += _freed;
                    }
                }
            }
        }
        let arena = tree.arena();
        for &id in &order {
            if touches.get(&id) == Some(&Touch::Removed) {
                continue;
            }
            let Some(node) = tree.get(id) else {
                warn!("renderer::layers: {layer:?} names {id:?}, which the layer does not hold");
                continue;
            };
            let run = node.get().glyph_run();
            let Some(slot) = mirror.slot_of(id) else {
                let state = run.map(|run| new_state(run, &mut self.stamp));
                pending.push(mirror.alloc(id, state));
                #[cfg(test)]
                {
                    log.inserted += 1;
                }
                continue;
            };
            let s = mirror.slot_mut(slot);
            match (&mut s.state, run) {
                (Some(state), Some(run)) if state.run.same_state(run) => {
                    #[cfg(test)]
                    {
                        log.unchanged += 1;
                    }
                }
                (Some(state), Some(run)) => {
                    #[cfg(test)]
                    log.classify(&state.run, run);
                    state.run = run.clone();
                    state.stamp = next_stamp(&mut self.stamp);
                }
                (state, run) => *state = run.map(|run| new_state(run, &mut self.stamp)),
            }
            if !in_place(mirror, arena, id, slot) {
                mirror.unlink(slot);
                pending.push(slot);
                #[cfg(test)]
                {
                    log.relinked += 1;
                }
            }
        }
        for k in 0..pending.len() {
            splice(mirror, arena, pending[k], stack);
        }
        order.clear();
        touches.clear();
        self.scratch.order[i] = order;
        self.scratch.touches[i] = touches;
    }
}

/// Whether `slot`, the slot of `id`, stands where `arena` — its layer as
/// it now stands — has `id`: under the slot of its parent, right after
/// the slot of its previous sibling (first in the row when it has none).
/// A previous sibling the renderer holds no slot for yet is one that
/// entered, so `id` is not where it was. The root stands in place while
/// it is the mirror's root. O(1).
fn in_place(mirror: &Mirror, arena: &Arena<GfxElement>, id: NodeId, slot: SlotId) -> bool {
    let s = mirror.slot(slot);
    let Some(parent) = id.parent(arena) else {
        return s.parent.is_none() && mirror.is_attached(slot);
    };
    let prev = match id.previous_sibling(arena) {
        None => None,
        Some(prev) => match mirror.slot_of(prev) {
            Some(prev) => Some(prev),
            None => return false,
        },
    };
    s.parent.is_some() && s.parent == mirror.slot_of(parent) && s.prev == prev
}

/// Put `slot`, which hangs under no parent, into its row as `arena` —
/// its layer as it now stands — has it: under its element's parent,
/// right after its previous sibling. A previous sibling still waiting for
/// its own row is put in first, and so on back along the row, so a row
/// of entering siblings goes in in one pass whatever order they were
/// named in. `stack` is the caller's scratch.
///
/// Cost: O(1) per slot put in, each once.
fn splice(mirror: &mut Mirror, arena: &Arena<GfxElement>, slot: SlotId, stack: &mut Vec<SlotId>) {
    stack.clear();
    stack.push(slot);
    while let Some(&top) = stack.last() {
        if mirror.is_attached(top) {
            stack.pop();
            continue;
        }
        let Some(id) = mirror.slot(top).id else {
            stack.pop();
            continue;
        };
        let Some(parent) = id.parent(arena).and_then(|p| mirror.slot_of(p)) else {
            warn!("renderer::layers: {id:?} enters under a parent the renderer does not hold");
            stack.pop();
            continue;
        };
        let prev = match id.previous_sibling(arena) {
            None => None,
            Some(p) => match mirror.slot_of(p) {
                Some(p) => Some(p),
                None => {
                    warn!("renderer::layers: {id:?} enters after a sibling the renderer does not hold");
                    stack.pop();
                    continue;
                }
            },
        };
        if let Some(p) = prev.filter(|&p| !mirror.is_attached(p)) {
            stack.push(p);
            continue;
        }
        mirror.link(top, parent, prev);
        stack.pop();
    }
}

/// The state of `id`'s run in `tree`, or `None` for an element that is
/// not a run.
fn run_state(tree: &GfxTree, id: NodeId, stamp: &mut u64) -> Option<RunState> {
    let run = tree.get(id)?.get().glyph_run()?;
    Some(new_state(run, stamp))
}

/// A fresh state of `run`, with the next stamp.
fn new_state(run: &GlyphRunData, stamp: &mut u64) -> RunState {
    RunState {
        run: run.clone(),
        stamp: next_stamp(stamp),
    }
}

/// The next stamp.
fn next_stamp(stamp: &mut u64) -> u64 {
    *stamp += 1;
    *stamp
}

#[cfg(test)]
impl ApplyLog {
    /// Count how `new` differs from `old`, a run's state it replaces.
    fn classify(&mut self, old: &GlyphRunData, new: &GlyphRunData) {
        use baumhard::gfx_structs::run::RunShape;
        use std::sync::Arc;
        match (old.shaped(), new.shaped()) {
            (RunShape::Path { run: a, placed: pa }, RunShape::Path { run: b, placed: pb })
                if Arc::ptr_eq(a, b) =>
            {
                self.replaced += usize::from(!Arc::ptr_eq(pa, pb));
            }
            (RunShape::Box(a), RunShape::Box(b)) if Arc::ptr_eq(a, b) => {}
            _ => self.reshaped += 1,
        }
        self.moved += usize::from(old.origin() != new.origin());
        self.restyled += usize::from(
            old.opacity() != new.opacity() || old.overrides() != new.overrides() || old.zoom() != new.zoom(),
        );
    }
}
