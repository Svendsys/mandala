// SPDX-License-Identifier: MPL-2.0

//! View-only glyph mutators (design §12.5): effects drawn over the
//! projected runs that mean nothing in the document — the selection
//! tint, the rubber band's preview of what it covers, the reparent and
//! connect sources ([`SOURCE_HIGHLIGHT`]) and target
//! ([`TARGET_HIGHLIGHT`]), NodeEdit's dim ([`Dim`]) and a text editor's
//! selected range ([`RangeTint`]).
//!
//! **One mechanism.** An [`Overlay`] is a run override
//! ([`RunOverrideOp`]) and the document elements it is stamped on. It is
//! applied as a baumhard glyph mutator, by the walker, journaled, to the
//! runs those elements project to, and its journals go to the layer
//! renderer beside the projector's (`MapView::sync`). Two moves keep it
//! right, and each costs what changed:
//!
//! - **The targets change** (a click, a rubber-band frame): the runs of
//!   the elements that left are cleared and the runs of those that came
//!   are stamped. No other run is visited.
//! - **The projector rewrites runs** (an update): the projector writes a
//!   run's whole state, its overrides included (`GfxMutation::RunSet`),
//!   so a stamp it rewrote is gone. Each run the update's `LayerChanges`
//!   name is looked up once — its element, then the element it belongs
//!   to — and stamped again when that is a target. Runs the update did
//!   not name keep their stamp, untouched (§12.5: "re-stamped on exactly
//!   those elements").
//!
//! **Unstamping is `RunOverrideOp::Clear`.** The projector writes no
//! override of its own in V1, so a cleared run is the projector's run
//! exactly. A glyph-layer presentation (V1-D2, #251) writes overrides
//! too, on top of an overlay's: before an overlay stamps or clears a
//! run a presentation plays on, the run is put back to what the
//! projector and the overlays made it, and the presentation is written
//! again once the overlay has written (`view::present`). That is why an
//! update is two steps — [`Overlay::plan`], which names the runs it will
//! write, and [`Overlay::write`], which writes them.
//!
//! An element's runs are those it *owns*: a node owns its sections' and
//! its frame's rings' runs; a connector or a link owns its body, its caps
//! and its label; a link owns its portal ends and their labels. A
//! selected node is tinted where its text and its frame are drawn, and a
//! selected path where it is. A target may also be a part of one that
//! owns runs of its own — a section, a ring: while a contextual color
//! picker is open, the selection tint stands down where the picker's axis
//! colors its targets and stays on the rest (`MapView::follow_picker`),
//! so a picked node keeps it on its ring and not on its text. A run the
//! projector rewrites is stamped again for the nearest target on its way
//! up to its owner.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::document::values::Rgba;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::journal::{Journal, JournalBuilder, Touch};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::run::RunOverrideOp;
use baumhard::gfx_structs::tree::GfxMutatorTree;
use baumhard::projection::{GlyphLayers, Layer, LayerChanges, Projector, RUN_CHANNEL};
use rustc_hash::FxHashMap;

use crate::application::document::selection::kind_of;

use super::inspect::runs_of;

/// The selection tint: the one "this is active" color, shared with the
/// mode-status line (`renderer::MODE_STATUS_COLOR`), `#00E5FF`.
pub(crate) const SELECTION_HIGHLIGHT: Rgba = Rgba::opaque(0x00, 0xE5, 0xFF);

/// The tint of the node a reparent or a connect would land on, under the
/// pointer in either mode: `main`'s target green, `#33FF66`.
pub(crate) const TARGET_HIGHLIGHT: Rgba = Rgba::opaque(0x33, 0xFF, 0x66);

/// The tint of the nodes a reparent or a connect moves or links, in
/// place of the selection's while either mode is active: `main`'s source
/// orange, `#FF8C00`.
pub(crate) const SOURCE_HIGHLIGHT: Rgba = Rgba::opaque(0xFF, 0x8C, 0x00);

/// One view-only glyph mutator: `op` stamped on the runs of `targets`.
/// See the [module docs](self).
pub(crate) struct Overlay {
    /// What a stamp writes on a run.
    op: RunOverrideOp,
    /// The document elements stamped, in no order that matters.
    targets: Vec<NodeId>,
    /// Every run stamped now, by layer and glyph element, with the target
    /// it was stamped for.
    stamped: FxHashMap<(Layer, NodeId), NodeId>,
}

/// What one [`Overlay::plan`] decided to write: the runs to clear, then
/// the runs to stamp, per layer.
#[derive(Default)]
pub(crate) struct Planned {
    /// The runs to clear, indexed by `Layer::index`.
    clear: [Vec<NodeId>; 5],
    /// The runs to stamp, indexed by `Layer::index`.
    stamp: [Vec<NodeId>; 5],
}

impl Planned {
    /// Every run this will write, cleared or stamped. No allocation.
    pub(crate) fn runs(&self) -> impl Iterator<Item = (Layer, NodeId)> + '_ {
        Layer::ALL.into_iter().flat_map(move |layer| {
            self.clear[layer.index()]
                .iter()
                .chain(&self.stamp[layer.index()])
                .map(move |&glyph| (layer, glyph))
        })
    }
}

/// The journals one [`Overlay::update`] wrote, one per layer: what the
/// layer renderer applies after the projector's, then discards.
pub(crate) struct OverlayJournals {
    /// One journal per layer, indexed by `Layer::index`.
    per_layer: [Journal<GfxElement>; 5],
}

impl OverlayJournals {
    /// Each layer's journal, as `LayerRenderer::journals_of` hands the
    /// projector's. No allocation.
    pub(crate) fn journals(&self) -> impl Iterator<Item = (Layer, &Journal<GfxElement>)> {
        Layer::ALL.into_iter().map(|l| (l, &self.per_layer[l.index()]))
    }

    /// Free what the journals hold, once the renderer has read them. A
    /// stamp removes nothing, so this holds no subtree; it is discarded
    /// as every consumed journal is (#241 amendment 4).
    pub(crate) fn discard(self, layers: &mut GlyphLayers) {
        for (layer, journal) in Layer::ALL.into_iter().zip(self.per_layer) {
            journal.discard(layers.tree_mut(layer));
        }
    }
}

impl Overlay {
    /// The tint of `color` on whatever it is pointed at. Stamps nothing
    /// until [`Self::update`].
    pub(crate) fn tint(color: Rgba) -> Self {
        Overlay {
            op: RunOverrideOp::Color {
                range: 0..usize::MAX,
                value: Some(color),
            },
            targets: Vec::new(),
            stamped: FxHashMap::default(),
        }
    }

    /// The elements stamped now. No allocation.
    pub(crate) fn targets(&self) -> &[NodeId] {
        &self.targets
    }

    /// Every run stamped now, by layer and glyph element. Test builds
    /// only.
    #[cfg(test)]
    pub(crate) fn stamped_runs(&self) -> Vec<(Layer, NodeId)> {
        self.stamped.keys().copied().collect()
    }

    /// Bring the stamps up to date: with `targets` as the elements to
    /// stamp from now on, and `changes`, when an update was made, as what
    /// the projector just wrote. Returns the journals of what was written.
    /// [`Self::plan`], then [`Self::write`].
    ///
    /// Cost: O(the runs of the targets that came or left, the runs
    /// `changes` names, and the depth from each named run's element to
    /// its owner — at most three); no walk of a layer or of the
    /// document.
    pub(crate) fn update(
        &mut self,
        targets: &[NodeId],
        changes: Option<&LayerChanges>,
        doc: &Document,
        projector: &Projector,
        layers: &mut GlyphLayers,
    ) -> OverlayJournals {
        let planned = self.plan(targets, changes, doc, projector, layers);
        self.write(planned, layers)
    }

    /// The first half of [`Self::update`]: decide which runs to clear
    /// and which to stamp, and take `targets` as the elements stamped
    /// from now on. Writes no run.
    pub(crate) fn plan(
        &mut self,
        targets: &[NodeId],
        changes: Option<&LayerChanges>,
        doc: &Document,
        projector: &Projector,
        layers: &GlyphLayers,
    ) -> Planned {
        let mut stamp: [Vec<NodeId>; 5] = Default::default();
        let mut clear: [Vec<NodeId>; 5] = Default::default();

        if let Some(changes) = changes {
            for layer in Layer::ALL {
                for (glyph, touch) in changes.journal(layer).touched() {
                    if touch == Touch::Removed {
                        self.stamped.remove(&(layer, glyph));
                        continue;
                    }
                    if let Some(target) = target_of(doc, projector, layer, glyph, targets) {
                        self.stamped.insert((layer, glyph), target);
                        stamp[layer.index()].push(glyph);
                    }
                }
            }
        }

        if targets != self.targets.as_slice() {
            self.stamped.retain(|&(layer, glyph), owner| {
                if targets.contains(owner) {
                    return true;
                }
                clear[layer.index()].push(glyph);
                false
            });
            for &target in targets {
                if self.targets.contains(&target) {
                    continue;
                }
                for (layer, glyph) in runs_of(doc, projector, layers, target) {
                    if self.stamped.insert((layer, glyph), target).is_none() {
                        stamp[layer.index()].push(glyph);
                    }
                }
            }
            self.targets.clear();
            self.targets.extend_from_slice(targets);
        }
        Planned { clear, stamp }
    }

    /// The second half of [`Self::update`]: clear and stamp the runs
    /// `planned` names, each by a glyph mutator the walker applies, and
    /// return the journals.
    pub(crate) fn write(&self, planned: Planned, layers: &mut GlyphLayers) -> OverlayJournals {
        write_planned(planned, self.op.clone(), layers)
    }
}

/// Write `op` on every run of `glyphs` in `tree`: one mutator, anchored at
/// all of them, applied by the walker into `journal`. Nothing for an
/// empty `glyphs`. A refusal — which a run the overlay found in the layer
/// cannot meet — is logged and leaves the stamps of that layer unwritten
/// for this update.
fn write(
    tree: &mut baumhard::gfx_structs::tree::GfxTree,
    journal: &mut JournalBuilder<GfxElement>,
    glyphs: &[NodeId],
    op: RunOverrideOp,
) {
    if glyphs.is_empty() {
        return;
    }
    let mutator = GfxMutatorTree::new_with(GfxMutator::new(GfxMutation::RunOverrides(op), RUN_CHANNEL));
    if let Err(error) = mutator.apply_at_journaled(tree, &Anchors::Nodes(glyphs.to_vec()), |_, _| (), journal)
    {
        log::warn!("view::overlay: a stamp was refused: {error}");
    }
}

/// The target of `targets` a run belongs to: its element, or the first
/// element above it up to its owner ([`owner_of`]) that is one — so a
/// target may be a node or a path, or a part of one that owns runs of its
/// own (a section, a ring: what the selection tint stands on while the
/// color picker stands it down on the rest, `MapView::follow_picker`).
/// `None` for a group, and for a run no target holds. O(the depth to the
/// owner, at most three, times the targets).
fn target_of(
    doc: &Document,
    projector: &Projector,
    layer: Layer,
    glyph: NodeId,
    targets: &[NodeId],
) -> Option<NodeId> {
    let mut at = projector.element_of(layer, glyph)?;
    if kind_of(doc, at)? == DocKind::Node {
        return None;
    }
    loop {
        if targets.contains(&at) {
            return Some(at);
        }
        match kind_of(doc, at)? {
            DocKind::Node | DocKind::Connector | DocKind::Link => return None,
            _ => at = doc.parent(at)?,
        }
    }
}

/// The element a run of `layer` belongs to for an overlay: from the
/// element it draws, the nearest of itself and its owners that is a
/// connector, a link or a node. `None` for a group, and for a glyph the
/// projector names no element for. O(the depth to the owner, at most
/// three).
pub(super) fn owner_of(doc: &Document, projector: &Projector, layer: Layer, glyph: NodeId) -> Option<NodeId> {
    let mut at = projector.element_of(layer, glyph)?;
    // A node draws nothing of its own, only groups: a glyph whose element
    // is a node is its group, and only runs are stamped.
    if kind_of(doc, at)? == DocKind::Node {
        return None;
    }
    loop {
        match kind_of(doc, at)? {
            DocKind::Node | DocKind::Connector | DocKind::Link => return Some(at),
            _ => at = doc.parent(at)?,
        }
    }
}

/// How much NodeEdit's dim keeps of a glyph's alpha: half, `main`'s
/// `INACTIVE_NODE_ALPHA_MULTIPLIER`.
pub(crate) const DIM_FADE: f32 = 0.5;

/// **NodeEdit's dim** (design §12.5; V1-C8, ruling K2 on #249): every
/// node but the one being edited drawn at [`DIM_FADE`] of its alpha —
/// its sections' and its frame's runs, as `main` dimmed them — by a
/// [`RunOverrideOp::Fade`] stamped as every overlay is stamped.
///
/// **Its targets are a rule, not a list**: a run is dimmed when its owner
/// is a node other than the active one and other than every node another
/// overlay is stamped on (`excluded`), so no two overlays ever stamp one
/// run and one's `Clear` never takes another's stamp. A list of every
/// other node would make each pass's membership test as long as the map;
/// the rule is O(the depth to the run's owner, at most three) per run.
///
/// **Each move costs what it changes.** Entering NodeEdit stamps the
/// sections' and frames' runs once — every node but one changes how it
/// looks — and leaving it clears exactly what it stamped. In between, a
/// projector rewrite re-stamps the runs it named, and a node that leaves
/// or joins `excluded` has its own runs cleared or stamped; no other run
/// is visited.
#[derive(Default)]
pub(crate) struct Dim {
    /// The node being edited, while NodeEdit is active.
    active: Option<NodeId>,
    /// The nodes another overlay is stamped on, which the dim leaves.
    excluded: Vec<NodeId>,
    /// Every run stamped now, by layer and glyph element.
    stamped: rustc_hash::FxHashSet<(Layer, NodeId)>,
    /// Every run the last plan cleared or stamped: what the tests that
    /// hold a pass to its changes read, since a stamp a run already wore
    /// writes no journal entry. Test builds only.
    #[cfg(test)]
    last_planned: Vec<(Layer, NodeId)>,
}

impl Dim {
    /// Whether the dim already stands as `active` and `excluded` want.
    /// O(the excluded nodes).
    pub(crate) fn is_current(&self, active: Option<NodeId>, excluded: &[NodeId]) -> bool {
        self.active == active && (active.is_none() || self.excluded == excluded)
    }

    /// The runs stamped now. Test builds only.
    #[cfg(test)]
    pub(crate) fn stamped_runs(&self) -> Vec<(Layer, NodeId)> {
        self.stamped.iter().copied().collect()
    }

    /// Every run the last plan cleared or stamped. Test builds only.
    #[cfg(test)]
    pub(crate) fn last_planned(&self) -> &[(Layer, NodeId)] {
        &self.last_planned
    }

    /// Whether the run `glyph` of `layer` is one the dim stamps for
    /// `active` and `excluded`: a section's or a ring's run whose node is
    /// neither. O(the depth to its owner, at most three).
    fn dims(
        doc: &Document,
        projector: &Projector,
        layer: Layer,
        glyph: NodeId,
        active: NodeId,
        excluded: &[NodeId],
    ) -> bool {
        matches!(layer, Layer::Sections | Layer::Frames)
            && owner_of(doc, projector, layer, glyph)
                .is_some_and(|node| node != active && !excluded.contains(&node))
    }

    /// Plan this pass's clears and stamps, in two halves — what it clears
    /// and re-stamps, written with the other overlays' clears; what it
    /// stamps anew, written with their stamps — and take `active` and
    /// `excluded` as what it stands by from now on. Writes no run.
    ///
    /// Cost: entering or leaving NodeEdit, the runs of the sections and
    /// frames layers once; otherwise the runs `changes` names and the
    /// runs of the nodes that left or joined `excluded`.
    pub(crate) fn plan(
        &mut self,
        active: Option<NodeId>,
        excluded: &[NodeId],
        changes: Option<&LayerChanges>,
        doc: &Document,
        projector: &Projector,
        layers: &GlyphLayers,
    ) -> (Planned, Planned) {
        let mut first = Planned::default();
        let mut second = Planned::default();
        // What the projector removed is no run to clear or stamp.
        if let Some(changes) = changes {
            for layer in Layer::ALL {
                for (glyph, touch) in changes.journal(layer).touched() {
                    if touch == Touch::Removed {
                        self.stamped.remove(&(layer, glyph));
                    }
                }
            }
        }
        // The transition first, to the `active` and `excluded` this pass
        // stands by, so what the changes re-stamp below is read against
        // them: a run rewritten in the pass that leaves NodeEdit, moves it
        // to another node, or puts its node in `excluded` is not stamped
        // under the state the pass leaves behind.
        let mut stamped_now = rustc_hash::FxHashSet::default();
        if self.active != active {
            for &(layer, glyph) in &self.stamped {
                first.clear[layer.index()].push(glyph);
            }
            self.stamped.clear();
            if let Some(active) = active {
                for layer in [Layer::Sections, Layer::Frames] {
                    let tree = layers.tree(layer);
                    for glyph in tree.descendants() {
                        let is_run = tree.get(glyph).is_some_and(|n| n.get().glyph_run().is_some());
                        if is_run && Self::dims(doc, projector, layer, glyph, active, excluded) {
                            self.stamped.insert((layer, glyph));
                            stamped_now.insert((layer, glyph));
                            second.stamp[layer.index()].push(glyph);
                        }
                    }
                }
            }
        } else if let Some(active) = active.filter(|_| self.excluded != excluded) {
            // Nodes that joined `excluded` lose their dim; nodes that left
            // it are dimmed again.
            self.stamped.retain(|&(layer, glyph)| {
                if Self::dims(doc, projector, layer, glyph, active, excluded) {
                    return true;
                }
                first.clear[layer.index()].push(glyph);
                false
            });
            for &node in &self.excluded {
                if excluded.contains(&node) {
                    continue;
                }
                for (layer, glyph) in runs_of(doc, projector, layers, node) {
                    if Self::dims(doc, projector, layer, glyph, active, excluded)
                        && self.stamped.insert((layer, glyph))
                    {
                        stamped_now.insert((layer, glyph));
                        second.stamp[layer.index()].push(glyph);
                    }
                }
            }
        }
        // Then the runs the projector rewrote, which lost their stamp:
        // stamped again when the state this pass stands by dims them.
        if let (Some(changes), Some(active)) = (changes, active) {
            for layer in Layer::ALL {
                for (glyph, touch) in changes.journal(layer).touched() {
                    if touch == Touch::Removed || stamped_now.contains(&(layer, glyph)) {
                        continue;
                    }
                    if Self::dims(doc, projector, layer, glyph, active, excluded) {
                        self.stamped.insert((layer, glyph));
                        first.stamp[layer.index()].push(glyph);
                    }
                }
            }
        }
        self.active = active;
        self.excluded.clear();
        self.excluded.extend_from_slice(excluded);
        #[cfg(test)]
        {
            self.last_planned = first.runs().chain(second.runs()).collect();
        }
        (first, second)
    }

    /// Write what [`Self::plan`] decided, as [`Overlay::write`] writes:
    /// the clears, then a fade on every stamped run.
    pub(crate) fn write(&self, planned: Planned, layers: &mut GlyphLayers) -> OverlayJournals {
        write_planned(
            planned,
            RunOverrideOp::Fade {
                range: 0..usize::MAX,
                value: DIM_FADE,
            },
            layers,
        )
    }
}

/// A run of one layer and the glyphs of it an overlay covers.
pub(crate) type GlyphRange = (Layer, NodeId, std::ops::Range<usize>);

/// **The editor's selected range** (V1-C8): the glyphs of the edited
/// owner's run that a shift-select covers, in the selection's cyan — a
/// [`RunOverrideOp::Color`] over just that glyph range, stamped and
/// re-stamped as every overlay is. The selection tint stands down on what
/// an editor edits (`MapView::sync`), so this is the only overlay on that
/// run while it is open.
#[derive(Default)]
pub(crate) struct RangeTint {
    /// The run stamped now, and the glyphs it covers.
    stamped: Option<GlyphRange>,
}

impl RangeTint {
    /// Whether it already stamps `want`. O(1).
    pub(crate) fn is_current(&self, want: Option<&GlyphRange>) -> bool {
        self.stamped.as_ref() == want
    }

    /// The run and range stamped now. Test builds only.
    #[cfg(test)]
    pub(crate) fn stamped(&self) -> Option<&GlyphRange> {
        self.stamped.as_ref()
    }

    /// Plan this pass's clear and stamp: the run stamped now is cleared
    /// when the range moved, and `want` is stamped when it moved or the
    /// projector rewrote its run in `changes` (which wiped the stamp).
    /// Returns the plan and the range to stamp. O(the runs `changes`
    /// names).
    pub(crate) fn plan(
        &mut self,
        want: Option<GlyphRange>,
        changes: Option<&LayerChanges>,
    ) -> (Planned, Option<std::ops::Range<usize>>) {
        let mut planned = Planned::default();
        let rewritten = |layer: Layer, glyph: NodeId| {
            changes.is_some_and(|c| c.journal(layer).touched().any(|(id, _)| id == glyph))
        };
        let moved = self.stamped != want;
        if let Some((layer, glyph, _)) = &self.stamped {
            if moved && !rewritten(*layer, *glyph) {
                planned.clear[layer.index()].push(*glyph);
            }
        }
        let range = match &want {
            Some((layer, glyph, range)) if moved || rewritten(*layer, *glyph) => {
                planned.stamp[layer.index()].push(*glyph);
                Some(range.clone())
            }
            _ => None,
        };
        self.stamped = want;
        (planned, range)
    }

    /// Write what [`Self::plan`] decided: the clear, then the cyan over
    /// `range`.
    pub(crate) fn write(
        &self,
        planned: Planned,
        range: Option<std::ops::Range<usize>>,
        layers: &mut GlyphLayers,
    ) -> OverlayJournals {
        write_planned(
            planned,
            RunOverrideOp::Color {
                range: range.unwrap_or(0..0),
                value: Some(SELECTION_HIGHLIGHT),
            },
            layers,
        )
    }
}

/// Clear and stamp the runs `planned` names — every clear before any
/// stamp, layer by layer — the stamps with `op`, each by a glyph mutator
/// the walker applies; return the journals.
fn write_planned(planned: Planned, op: RunOverrideOp, layers: &mut GlyphLayers) -> OverlayJournals {
    let Planned { clear, stamp } = planned;
    let mut out: [JournalBuilder<GfxElement>; 5] = std::array::from_fn(|_| JournalBuilder::new());
    for layer in Layer::ALL {
        let i = layer.index();
        let tree = layers.tree_mut(layer);
        write(tree, &mut out[i], &clear[i], RunOverrideOp::Clear);
        write(tree, &mut out[i], &stamp[i], op.clone());
    }
    let [sections, frames, connections, labels, portals] = out;
    OverlayJournals {
        per_layer: [
            sections.close(&layers.sections),
            frames.close(&layers.frames),
            connections.close(&layers.connections),
            labels.close(&layers.labels),
            portals.close(&layers.portals),
        ],
    }
}
