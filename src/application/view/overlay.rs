// SPDX-License-Identifier: MPL-2.0

//! View-only glyph mutators (design §12.5): effects drawn over the
//! projected runs that mean nothing in the document — the selection
//! tint, the rubber band's preview of what it covers, the reparent and
//! connect target ([`TARGET_HIGHLIGHT`]), and in a later issue
//! NodeEdit's dimming (V1-C8).
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
//! override of its own in V1 (glyph-layer presentations arrive with
//! V1-D2, #251, which stacks its own overrides on this mechanism), so a
//! cleared run is the projector's run exactly.
//!
//! An element's runs are those it *owns*: a node owns its sections' and
//! its frame's rings' runs; a connector or a link owns its body, its caps
//! and its label; a link owns its portal ends and their labels. A
//! selected node is tinted where its text and its frame are drawn, and a
//! selected path where it is.

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

    /// Bring the stamps up to date: with `targets` as the elements to
    /// stamp from now on, and `changes`, when an update was made, as what
    /// the projector just wrote. Returns the journals of what was written.
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
        let mut stamp: [Vec<NodeId>; 5] = Default::default();
        let mut clear: [Vec<NodeId>; 5] = Default::default();

        if let Some(changes) = changes {
            for layer in Layer::ALL {
                for (glyph, touch) in changes.journal(layer).touched() {
                    if touch == Touch::Removed {
                        self.stamped.remove(&(layer, glyph));
                        continue;
                    }
                    let Some(owner) = owner_of(doc, projector, layer, glyph) else {
                        continue;
                    };
                    if targets.contains(&owner) {
                        self.stamped.insert((layer, glyph), owner);
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

        let mut out: [JournalBuilder<GfxElement>; 5] = std::array::from_fn(|_| JournalBuilder::new());
        for layer in Layer::ALL {
            let i = layer.index();
            let tree = layers.tree_mut(layer);
            write(tree, &mut out[i], &clear[i], RunOverrideOp::Clear);
            write(tree, &mut out[i], &stamp[i], self.op.clone());
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
