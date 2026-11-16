// SPDX-License-Identifier: MPL-2.0

//! Glyph-layer presentations on the glyph layers (#251 §4, ruling D2.1
//! §1; design §12.5): the run mutations of every glyph-layer instance
//! playing on the document, written on the runs its elements project
//! to, at the instance's progress. They are view-only: nothing here
//! touches the document, and the document's layer
//! (`baumhard::document::presentation::layer`) only lists the instances
//! and moves their progress.
//!
//! **What is kept, per run an instance is on, is the run's base**: the
//! values of exactly what a run mutation can change — its opacity, its
//! origin and its overrides — as the projector and the overlays left
//! them ([`RunBase`]). Nothing else of a run is copied, a run no
//! instance is on keeps none, and when the last instance on a run ends
//! the run is put back to its base and the base is dropped: the run is
//! then what the projector and the overlays alone make it, to the bit.
//!
//! **No journal outlives a pass.** A journal can be written at a
//! progress only while its tree stands at one of the journal's two
//! stamps, and the projector and the overlays write these same trees
//! every pass. So each pass, for exactly the runs that
//!
//! 1. the projector's `LayerChanges` name and an instance targets,
//! 2. an overlay is about to stamp or clear, or
//! 3. carry an instance that started, ended or moved,
//!
//! the run is put back to its base ([`Stage::lift`]), the overlays
//! write, and then each instance on it, in start order, applies its run
//! mutations by the walker under a journal that is written at the
//! instance's progress with `Journal::write_interpolated`
//! ([`Stage::stamp`]). Putting a run back is itself a mutation applied
//! by the walker: the three run mutations a presentation may write,
//! spelling the base. Every journal goes to the layer renderer with the
//! projector's and the overlays', and is discarded with them.
//!
//! **Work is proportional to those runs**, never to a layer: a run is
//! found from its element through the projector's map, an element's
//! instances through an index kept here, and no layer is walked.
//!
//! **Offsets and scales blend from a run that has none.** A run's
//! override vectors interpolate only between two of the same length
//! (`GfxElement`'s interpolation), and a projected run has none. So
//! before an instance that writes overrides is applied on such a run,
//! the run is given neutral overrides — drawn exactly as none — by one
//! more walk. The base, taken before, still says "none", and is what
//! the run goes back to. A glyph's color has no neutral value to blend
//! from: it appears whole, where the instance's `snap` puts it.

use baumhard::arena::NodeId;
use baumhard::document::ids::Document;
use baumhard::document::presentation::{InstanceId, InstanceInfo, PresentationLayer};
use baumhard::document::values::Rgba;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::element::TreeElement;
use baumhard::engine::interpolate::Snap;
use baumhard::engine::journal::{Journal, JournalBuilder, Touch};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::{glyph_mutations, GlyphLayers, Layer, LayerChanges, Projector, RUN_CHANNEL};
use glam::Vec2;
use rustc_hash::{FxHashMap, FxHashSet};

/// A run of a layer.
type Run = (Layer, NodeId);

/// What of a run a run mutation can change, as the projector and the
/// overlays left it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunBase {
    opacity: f32,
    origin: Vec2,
    overrides: GlyphOverrides,
}

impl RunBase {
    /// The base `run` stands at now. One copy of its override vectors.
    fn of(run: &GlyphRunData) -> Self {
        RunBase {
            opacity: run.opacity(),
            origin: run.origin(),
            overrides: run.overrides().clone(),
        }
    }

    /// The run mutations that put a run back to this base, in order:
    /// its opacity, its origin, no overrides, then the base's own — the
    /// vectors made neutral, and every stretch of glyphs that share a
    /// value other than the neutral one written over them. Values are
    /// compared by their bits, so what is written is the base exactly.
    ///
    /// Cost: O(the base's glyphs); three mutations for a base with no
    /// overrides, and for one tinted whole, five.
    fn restore(&self) -> Vec<GfxMutation> {
        let mut ops = vec![
            GfxMutation::RunOpacity(self.opacity),
            GfxMutation::RunOrigin(self.origin),
            GfxMutation::RunOverrides(RunOverrideOp::Clear),
        ];
        let len = self.overrides.len();
        if len == 0 {
            return ops;
        }
        ops.push(neutral_overrides());
        let offsets = self.overrides.offset();
        stretches(
            len,
            |i| (offsets[i].x.to_bits(), offsets[i].y.to_bits()),
            (0f32.to_bits(), 0f32.to_bits()),
            |range| {
                ops.push(GfxMutation::RunOverrides(RunOverrideOp::Offset {
                    value: offsets[range.start],
                    range,
                }))
            },
        );
        let colors = self.overrides.color();
        stretches(
            len,
            |i| colors[i],
            None::<Rgba>,
            |range| {
                ops.push(GfxMutation::RunOverrides(RunOverrideOp::Color {
                    value: colors[range.start],
                    range,
                }))
            },
        );
        let scales = self.overrides.scale();
        stretches(
            len,
            |i| scales[i].to_bits(),
            1f32.to_bits(),
            |range| {
                ops.push(GfxMutation::RunOverrides(RunOverrideOp::Scale {
                    value: scales[range.start],
                    range,
                }))
            },
        );
        ops
    }
}

/// Call `each` with every maximal stretch of `0..len` over which `key`
/// is one value other than `neutral`. O(len).
fn stretches<K: PartialEq>(
    len: usize,
    key: impl Fn(usize) -> K,
    neutral: K,
    mut each: impl FnMut(std::ops::Range<usize>),
) {
    let mut start = 0;
    while start < len {
        let value = key(start);
        let mut end = start + 1;
        while end < len && key(end) == value {
            end += 1;
        }
        if value != neutral {
            each(start..end);
        }
        start = end;
    }
}

/// The run mutation that gives a run with no overrides neutral ones —
/// a zero offset on every glyph — and leaves a run that has overrides
/// with every offset at zero.
fn neutral_overrides() -> GfxMutation {
    GfxMutation::RunOverrides(RunOverrideOp::Offset {
        range: 0..usize::MAX,
        value: Vec2::ZERO,
    })
}

/// Whether `ops` write a run's overrides, other than dropping them.
fn writes_overrides(ops: &[GfxMutation]) -> bool {
    ops.iter().any(|op| {
        matches!(
            op,
            GfxMutation::RunOverrides(
                RunOverrideOp::Offset { .. } | RunOverrideOp::Color { .. } | RunOverrideOp::Scale { .. }
            )
        )
    })
}

/// What was last written of one instance.
#[derive(Clone, Debug)]
struct Shown {
    /// The elements it plays on.
    elements: Vec<NodeId>,
    /// Its run mutations: what its body spells
    /// (`projection::glyph_mutations`), read once, when it is first seen.
    mutations: Vec<GfxMutation>,
    /// The progress it was written at.
    t: f32,
    /// The snap it was written with.
    snap: Snap,
}

/// The journals one pass of the stage wrote, in the order it wrote
/// them: what the layer renderer applies with the projector's and the
/// overlays', then discards.
#[derive(Default)]
pub(crate) struct StageJournals {
    journals: Vec<(Layer, Journal<GfxElement>)>,
}

impl StageJournals {
    /// Each journal with its layer. No allocation.
    pub(crate) fn journals(&self) -> impl Iterator<Item = (Layer, &Journal<GfxElement>)> {
        self.journals.iter().map(|(layer, journal)| (*layer, journal))
    }

    /// Free what the journals hold, once the renderer has read them. A
    /// run mutation removes nothing, so they hold no subtree; they are
    /// discarded as every consumed journal is.
    pub(crate) fn discard(self, layers: &mut GlyphLayers) {
        for (layer, journal) in self.journals {
            journal.discard(layers.tree_mut(layer));
        }
    }
}

/// What [`Stage::lift`] found to rewrite, handed to [`Stage::stamp`]
/// once the overlays have written.
pub(crate) struct Lifted {
    /// The runs to rewrite, in a fixed order.
    runs: Vec<Run>,
    /// The journals of the runs put back.
    journals: StageJournals,
}

/// The glyph-layer instances as they stand on the glyph layers. See the
/// [module docs](self).
#[derive(Default)]
pub(crate) struct Stage {
    /// The base of every run an instance is on.
    bases: FxHashMap<Run, RunBase>,
    /// What was last written of each instance.
    shown: FxHashMap<InstanceId, Shown>,
    /// The instances that play on each element, in start order.
    targets: FxHashMap<NodeId, Vec<InstanceId>>,
    /// The document's presentation epoch as of the last pass; `None`
    /// before the first.
    epoch: Option<u64>,
    /// What the stage has done, counted. Test builds only.
    #[cfg(test)]
    pub(crate) work: Work,
}

/// What a [`Stage`] has done, counted, for the tests that hold its work
/// to the runs a pass names. Test builds only.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Work {
    /// Runs put back to their base.
    pub(crate) put_back: usize,
    /// Bases taken.
    pub(crate) based: usize,
    /// Applications of one instance on one run.
    pub(crate) applied: usize,
}

impl Stage {
    /// The stage of a view just projected from `doc`: nothing written
    /// yet. With no glyph-layer instance playing it is up to date, so
    /// the first pass has nothing to do for it; with one — a view made
    /// of a document already performing — the first pass writes it.
    /// O(the instances).
    pub(crate) fn of(doc: &Document) -> Self {
        Stage {
            epoch: glyph_instances(doc)
                .next()
                .is_none()
                .then(|| doc.presentation_epoch()),
            ..Stage::default()
        }
    }

    /// Whether a pass has nothing to do for the stage on its own: no
    /// instance started, ended or moved since the last one. A pass that
    /// the projector or an overlay gives work still calls
    /// [`Self::lift`]. O(1).
    pub(crate) fn is_current(&self, doc: &Document) -> bool {
        self.epoch == Some(doc.presentation_epoch())
    }

    /// The first half of a pass, after the projector's update and before
    /// the overlays write: find the runs to rewrite — the three kinds
    /// the [module docs](self) list — and put each that carries an
    /// instance's writes back to its base, unless the projector has just
    /// written it whole.
    ///
    /// `planned` is every run an overlay is about to stamp or clear.
    ///
    /// Cost: O(the runs `changes` names, the runs `planned` names, and —
    /// when an instance started, ended or moved — the glyph-layer
    /// instances and the runs of those that did); one walk per run put
    /// back.
    pub(crate) fn lift(
        &mut self,
        doc: &Document,
        projector: &Projector,
        changes: Option<&LayerChanges>,
        planned: impl IntoIterator<Item = Run>,
        layers: &mut GlyphLayers,
    ) -> Lifted {
        let mut dirty: FxHashSet<Run> = FxHashSet::default();
        // The runs the projector has just written whole: what stood on
        // them is gone, and there is nothing to put back.
        let mut fresh: FxHashSet<Run> = FxHashSet::default();

        let moved = !self.is_current(doc);
        if moved {
            self.follow_instances(doc, projector, &mut dirty);
        }

        if let Some(changes) = changes {
            for layer in Layer::ALL {
                let journal = changes.journal(layer);
                for (glyph, touch) in journal.touched() {
                    let run = (layer, glyph);
                    if touch == Touch::Removed {
                        self.bases.remove(&run);
                        dirty.remove(&run);
                        continue;
                    }
                    // A run the projector only moved among its siblings
                    // keeps what stands on it.
                    let rewritten = match (journal.before(glyph), journal.after(glyph)) {
                        (Some(before), Some(after)) => !before.same_state(after),
                        _ => true,
                    };
                    if !rewritten {
                        continue;
                    }
                    self.bases.remove(&run);
                    let targeted = projector
                        .element_of(layer, glyph)
                        .is_some_and(|element| self.targets.contains_key(&element));
                    if targeted {
                        dirty.insert(run);
                        fresh.insert(run);
                    } else {
                        dirty.remove(&run);
                    }
                }
            }
        }

        for run in planned {
            if self.bases.contains_key(&run) {
                dirty.insert(run);
            }
        }

        let mut runs: Vec<Run> = dirty.into_iter().collect();
        runs.sort_unstable_by_key(|&(layer, glyph)| (layer.index(), glyph));

        let mut builders: [JournalBuilder<GfxElement>; 5] = std::array::from_fn(|_| JournalBuilder::new());
        for &(layer, glyph) in &runs {
            if fresh.contains(&(layer, glyph)) {
                continue;
            }
            let Some(base) = self.bases.get(&(layer, glyph)) else {
                continue;
            };
            #[cfg(test)]
            {
                self.work.put_back += 1;
            }
            write(
                layers.tree_mut(layer),
                &mut builders[layer.index()],
                &[glyph],
                base.restore(),
            );
        }
        let mut journals = StageJournals::default();
        for (layer, builder) in Layer::ALL.into_iter().zip(builders) {
            journals.journals.push((layer, builder.close(layers.tree(layer))));
        }
        Lifted { runs, journals }
    }

    /// Bring what is known of the instances up to the document's: the
    /// index of who plays on what, and — into `dirty` — the runs of
    /// every instance that started, ended or moved.
    fn follow_instances(&mut self, doc: &Document, projector: &Projector, dirty: &mut FxHashSet<Run>) {
        self.epoch = Some(doc.presentation_epoch());
        let mut live: FxHashSet<InstanceId> = FxHashSet::default();
        let mut changed: Vec<Vec<NodeId>> = Vec::new();
        let mut index_changed = false;
        for instance in glyph_instances(doc) {
            live.insert(instance.id);
            match self.shown.get_mut(&instance.id) {
                Some(shown) if shown.t == instance.t && shown.snap == instance.snap => {}
                Some(shown) => {
                    shown.t = instance.t;
                    shown.snap = instance.snap;
                    changed.push(shown.elements.clone());
                }
                None => {
                    index_changed = true;
                    changed.push(instance.elements.to_vec());
                    self.shown.insert(
                        instance.id,
                        Shown {
                            elements: instance.elements.to_vec(),
                            mutations: instance.glyph.map(glyph_mutations).unwrap_or_default(),
                            t: instance.t,
                            snap: instance.snap,
                        },
                    );
                }
            }
        }
        self.shown.retain(|id, shown| {
            if live.contains(id) {
                return true;
            }
            index_changed = true;
            changed.push(std::mem::take(&mut shown.elements));
            false
        });
        if index_changed {
            self.targets.clear();
            let mut ids: Vec<InstanceId> = self.shown.keys().copied().collect();
            ids.sort_unstable();
            for id in ids {
                for &element in &self.shown[&id].elements {
                    self.targets.entry(element).or_default().push(id);
                }
            }
        }
        for elements in changed {
            for element in elements {
                dirty.extend(runs_drawn_for(doc, projector, element));
            }
        }
    }

    /// The second half of a pass, after the overlays have written: for
    /// each run [`Self::lift`] found, take its base as it now stands and
    /// apply every instance on it, in start order, at its progress — or,
    /// when no instance is on it any more, drop its base and leave it as
    /// it is.
    ///
    /// Cost: O(the runs found, and the instances on them): per instance
    /// and layer one walk over its runs there, and one interpolation per
    /// run.
    pub(crate) fn stamp(
        &mut self,
        lifted: Lifted,
        doc: &Document,
        projector: &Projector,
        layers: &mut GlyphLayers,
    ) -> StageJournals {
        let Lifted { runs, mut journals } = lifted;
        if runs.is_empty() {
            return journals;
        }
        // The runs each instance is applied on now, per layer.
        let mut on: FxHashMap<InstanceId, [Vec<NodeId>; 5]> = FxHashMap::default();
        for &(layer, glyph) in &runs {
            let run = (layer, glyph);
            let live = layers
                .tree(layer)
                .get(glyph)
                .filter(|_| layers.tree(layer).contains(glyph))
                .and_then(|node| node.get().glyph_run());
            let instances = projector
                .element_of(layer, glyph)
                .and_then(|element| self.targets.get(&element));
            match (live, instances) {
                (Some(live), Some(instances)) if !instances.is_empty() => {
                    #[cfg(test)]
                    {
                        self.work.based += 1;
                    }
                    self.bases.insert(run, RunBase::of(live));
                    for &id in instances {
                        on.entry(id).or_default()[layer.index()].push(glyph);
                    }
                }
                _ => {
                    self.bases.remove(&run);
                }
            }
        }
        if on.is_empty() {
            return journals;
        }
        // In start order, each over what the ones before it wrote.
        for instance in glyph_instances(doc) {
            let Some(per_layer) = on.get(&instance.id) else {
                continue;
            };
            let Some(ops) = self
                .shown
                .get(&instance.id)
                .map(|shown| shown.mutations.as_slice())
            else {
                continue;
            };
            for layer in Layer::ALL {
                let glyphs = &per_layer[layer.index()];
                if glyphs.is_empty() {
                    continue;
                }
                #[cfg(test)]
                {
                    self.work.applied += glyphs.len();
                }
                let tree = layers.tree_mut(layer);
                if writes_overrides(ops) {
                    let bare: Vec<NodeId> = glyphs
                        .iter()
                        .copied()
                        .filter(|&glyph| {
                            tree.get(glyph)
                                .and_then(|node| node.get().glyph_run())
                                .is_some_and(|run| run.overrides().is_empty())
                        })
                        .collect();
                    if !bare.is_empty() {
                        let mut builder = JournalBuilder::new();
                        write(tree, &mut builder, &bare, vec![neutral_overrides()]);
                        journals.journals.push((layer, builder.close(tree)));
                    }
                }
                let mut builder = JournalBuilder::new();
                write(tree, &mut builder, glyphs, ops.to_vec());
                let journal = builder.close(tree);
                if instance.t < 1.0 {
                    if let Err(error) = journal.write_interpolated(tree, instance.t, instance.snap) {
                        log::warn!(
                            "view::present: `{}` could not be written at {}: {error}",
                            instance.presentation,
                            instance.t
                        );
                    }
                }
                journals.journals.push((layer, journal));
            }
        }
        journals
    }

    /// The runs an instance's writes stand on now, in no order: for the
    /// tests. Test builds only.
    #[cfg(test)]
    pub(crate) fn based_runs(&self) -> Vec<Run> {
        let mut runs: Vec<Run> = self.bases.keys().copied().collect();
        runs.sort_unstable_by_key(|&(layer, glyph)| (layer.index(), glyph));
        runs
    }
}

/// The glyph-layer instances playing on `doc`, in start order.
fn glyph_instances(doc: &Document) -> impl Iterator<Item = InstanceInfo<'_>> + '_ {
    doc.presentations()
        .filter(|instance| instance.layer == PresentationLayer::Glyph)
}

/// The runs `element` itself is drawn as: a section's box, a ring's
/// run, a connector's or a link's body, a label. The projector keeps a
/// unit's runs under the unit, so a label's is found under its path, or
/// under the link its portal end belongs to. None for an element that
/// draws no run of its own, or is not drawn. O(the unit's slots, and at
/// most two steps up).
fn runs_drawn_for(doc: &Document, projector: &Projector, element: NodeId) -> Vec<Run> {
    let mut at = element;
    for _ in 0..3 {
        let own: Vec<Run> = projector
            .glyphs_of(at)
            .into_iter()
            .filter(|&(layer, glyph)| projector.element_of(layer, glyph) == Some(element))
            .collect();
        if !own.is_empty() {
            return own;
        }
        match doc.parent(at) {
            Some(parent) => at = parent,
            None => break,
        }
    }
    Vec::new()
}

/// Apply `mutations` to every run of `glyphs` in `tree`: one mutator,
/// anchored at all of them, applied by the walker into `journal`. A
/// refusal — which a run the stage found in the layer cannot meet — is
/// logged and leaves those runs unwritten for this pass.
fn write(
    tree: &mut GfxTree,
    journal: &mut JournalBuilder<GfxElement>,
    glyphs: &[NodeId],
    mutations: Vec<GfxMutation>,
) {
    if glyphs.is_empty() || mutations.is_empty() {
        return;
    }
    let mutator = GfxMutatorTree::new_with(GfxMutator::new_macro(mutations, RUN_CHANNEL));
    if let Err(error) = mutator.apply_at_journaled(tree, &Anchors::Nodes(glyphs.to_vec()), |_, _| (), journal)
    {
        log::warn!("view::present: a run mutation was refused: {error}");
    }
}
