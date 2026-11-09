// SPDX-License-Identifier: MPL-2.0

//! The map's view: the projector, the glyph layers it keeps, the chrome
//! layer and the view-only overlays — and the **one pipeline** that
//! keeps them and the renderer in step with the document (design §3,
//! §12, §13; V1-C3 #244).
//!
//! ```text
//! MindMapDocument ──take_changes()──▶ Projector::update ──▶ GlyphLayers
//!        │ (once per change)                │ LayerChanges
//!        └──▶ the IPC event stream          ▼
//!                               overlays re-stamped on what changed
//!                                           │ their journals
//!                                           ▼
//!                       LayerRenderer::apply(projector's ++ overlays')
//!                                           │ LayerChanges
//!                                           ▼
//!                       chrome builders ──▶ LayerRenderer::apply_chrome
//!                                           │
//!                                           ▼ discard every journal
//! ```
//!
//! [`MapView::load`] is the only full pass: one projection of the whole
//! document, one load of the renderer's state. After it, every change
//! goes through [`MapView::sync`], which drains the document's change
//! feed **once**, hands the records to the projector, re-stamps the
//! overlays on exactly the runs the projector wrote, applies both sets of
//! journals to the renderer, discards them, and keeps the records for
//! the IPC event stream, which reads them at its next look at the app
//! ([`MapView::pending_feed`]). There is no rebuild function. A camera change
//! calls none of this: the renderer applies the camera when it draws
//! (§12.4).
//!
//! **Chrome that follows the document** registers a
//! [`chrome::ChromeBuilder`] at load ([`MapView::load_with_chrome`]):
//! the hook V1-C7's handles and V1-C8's caret register on, so neither
//! edits this pass. A pass that changed the layers hands its
//! `LayerChanges` to every builder before it discards them, and applies
//! what they return to the chrome layer under one journal in the same
//! pass ([module docs](chrome)).
//!
//! **One font lease per pass** (#241, B7's note): a load and a sync each
//! hold one `FontSystemLease`, taken at the first font read and released
//! when the pass ends, never across frames.
//!
//! **What is read here, and what is not.** Selection and hit testing ask
//! the projector — [`MapView::hit`], and natively `MapView::nodes_in` for
//! the rubber band — which keeps
//! the projection map; nothing reads a glyph layer to learn what the
//! document is.

#[cfg(not(target_arch = "wasm32"))]
mod band;
pub(crate) mod chrome;
pub(crate) mod inspect;
pub(crate) mod overlay;
#[cfg(test)]
mod tests;

use baumhard::arena::NodeId;
use baumhard::document::resolve::resolve_background;
use baumhard::document::writer::feed::ChangeRecord;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::journal::{Journal, JournalBuilder};
use baumhard::engine::tree::TreeError;
#[cfg(not(target_arch = "wasm32"))]
use baumhard::flow::Rect;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
#[cfg(not(target_arch = "wasm32"))]
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
#[cfg(not(target_arch = "wasm32"))]
use baumhard::projection::RUN_CHANNEL;
use baumhard::projection::{GlyphLayers, Hit, LayerChanges, Projector};
use glam::Vec2;

use crate::application::document::MindMapDocument;
use crate::application::renderer::{LayerRenderer, Renderer};

use self::chrome::{ChromeBuilder, PassChanges};
use self::overlay::{Overlay, OverlayJournals, SELECTION_HIGHLIGHT, SOURCE_HIGHLIGHT, TARGET_HIGHLIGHT};

/// The view of one open map. See the [module docs](self).
pub(crate) struct MapView {
    /// The projection state: what every glyph element was projected from.
    projector: Projector,
    /// The five glyph layers the projector writes.
    layers: GlyphLayers,
    /// The chrome layer (design §12.1): view-only glyphs built from app
    /// state rather than the document — the rubber band
    /// (`view::band`, native), and V1-C7's handles and V1-C8's section frames to
    /// come. Drawn above the five layers; written only under a journal
    /// the renderer applies — [`Self::apply_chrome`]'s, or a sync
    /// pass's for its chrome builders.
    chrome: GfxTree,
    /// The chrome that follows the document ([`chrome::ChromeBuilder`]),
    /// in the order they were registered at load: handed every sync
    /// pass's `LayerChanges`.
    chrome_builders: Vec<Box<dyn ChromeBuilder>>,
    /// The rubber band's four runs in [`Self::chrome`]. Native, with
    /// the rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    band: band::BandRuns,
    /// The rectangle the band is drawn over, while one is out. Native,
    /// with the rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    band_rect: Option<Rect>,
    /// The selection tint, stamped on the selection — or, while a rubber
    /// band is out, on what it covers (`band_targets`).
    highlight: Overlay,
    /// The reparent and connect target tint, stamped on
    /// [`Self::hover_target`].
    hover: Overlay,
    /// The node a reparent or a connect would land on, while the pointer
    /// is over a valid one in either mode.
    hover_target: Option<NodeId>,
    /// The tint of a reparent's or a connect's sources, stamped on
    /// [`Self::source_targets`] in place of the selection's.
    sources: Overlay,
    /// The nodes a reparent or a connect moves or links, while either
    /// mode is active.
    source_targets: Vec<NodeId>,
    /// The mode last followed ([`Self::follow_mode`]). Native, with the
    /// two modes.
    #[cfg(not(target_arch = "wasm32"))]
    mode: crate::application::app::InteractionMode,
    /// The active mode's status line, drawn over the document's.
    mode_line: Option<String>,
    /// What a rubber band covers while one is out — the highlight's
    /// targets, not a preview of any document change: the highlight
    /// follows them instead of the selection, whose highlight stands down
    /// until the band's release replaces the selection.
    band_targets: Option<Vec<NodeId>>,
    /// The change records drained since the event loop last cleared
    /// them ([`Self::clear_feed`]): the second consumer of the one drain
    /// a sync makes (#244 amendment 3; V1-E2 builds events from them).
    /// Cleared every iteration, so it holds at most an iteration's
    /// changes.
    feed: Vec<ChangeRecord>,
    /// The document's status line ([`MindMapDocument::status`]) as the
    /// renderer was last told it, so a sync hands it over only when it
    /// changed.
    shown_status: Option<String>,
}

impl MapView {
    /// Project `doc` whole and load the renderer with it: what installing
    /// a document does, and the only full pass in its life (§3 "Load").
    /// Any change records the document holds are dropped: the projection
    /// is of the document as it now stands.
    ///
    /// Cost: one walk of the visible document, every flow shaped and
    /// placed once, under one font lease; one walk of each layer by the
    /// renderer.
    pub(crate) fn load(doc: &mut MindMapDocument, renderer: &mut Renderer) -> MapView {
        Self::load_with_chrome(doc, renderer, Vec::new())
    }

    /// [`Self::load`], with `builders` registered for the chrome that
    /// follows the document ([`chrome::ChromeBuilder`]): each is planted
    /// in the chrome layer before the renderer loads it, and from then on
    /// handed every sync pass's changes. Cost: [`Self::load`]'s, and each
    /// builder's planting.
    pub(crate) fn load_with_chrome(
        doc: &mut MindMapDocument,
        renderer: &mut Renderer,
        builders: Vec<Box<dyn ChromeBuilder>>,
    ) -> MapView {
        let (mut view, stamps) = Self::project(doc, builders);
        renderer.load_view(&view.layers, &view.chrome);
        renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
        stamps.discard(&mut view.layers);
        view
    }

    /// The projection half of [`Self::load_with_chrome`]: the view of
    /// `doc`, its chrome builders planted, its highlight stamped, and the
    /// stamps' journals still to discard.
    fn project(
        doc: &mut MindMapDocument,
        mut builders: Vec<Box<dyn ChromeBuilder>>,
    ) -> (MapView, OverlayJournals) {
        doc.take_changes();
        let mut layers = GlyphLayers::new();
        let projector = {
            let mut fonts = FontSystemLease::new("view::load");
            Projector::project_full(doc.doc(), &mut layers, &mut fonts)
        };
        let mut chrome = chrome_layer();
        #[cfg(not(target_arch = "wasm32"))]
        let band = band::plant(&mut chrome);
        for builder in &mut builders {
            builder.plant(&mut chrome);
        }
        let mut view = MapView {
            projector,
            layers,
            chrome,
            chrome_builders: builders,
            #[cfg(not(target_arch = "wasm32"))]
            band,
            #[cfg(not(target_arch = "wasm32"))]
            band_rect: None,
            highlight: Overlay::tint(SELECTION_HIGHLIGHT),
            hover: Overlay::tint(TARGET_HIGHLIGHT),
            hover_target: None,
            sources: Overlay::tint(SOURCE_HIGHLIGHT),
            source_targets: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            mode: Default::default(),
            mode_line: None,
            band_targets: None,
            feed: Vec::new(),
            shown_status: None,
        };
        let targets: Vec<NodeId> = doc.selection.iter().collect();
        let stamps = view
            .highlight
            .update(&targets, None, doc.doc(), &view.projector, &mut view.layers);
        (view, stamps)
    }

    /// **The pipeline** ([module docs](self)): bring the layers, the
    /// overlays and the renderer up to date with `doc` — its changes since
    /// the last pass, and its selection — and keep the change records,
    /// drained once, for the IPC event stream ([`Self::pending_feed`]).
    ///
    /// Does nothing to any tree, and calls the renderer not at all, when
    /// the document did not change, the highlight's targets did not
    /// either, and neither did its status line: what a frame with nothing
    /// to do, a pan and a zoom cost. Returns whether it did anything —
    /// whether the frame owes a redraw.
    ///
    /// The status line is the document's ([`MindMapDocument::status`],
    /// D28): what a refused change says, handed to the renderer's status
    /// line when it changes — the one place every family's refusal is
    /// drawn, on both targets.
    ///
    /// When the layers changed, the chrome builders are handed the
    /// changes ([`chrome::PassChanges`]) and what they return is applied
    /// to the chrome, and by the renderer, before the pass discards its
    /// journals.
    ///
    /// Cost: the projector's update (proportional to the changes and their
    /// readers), the overlay's re-stamp (the runs named), each chrome
    /// builder's reading of the changes, the renderer's apply (the
    /// elements the journals name); O(1) for the status line.
    pub(crate) fn sync(&mut self, doc: &mut MindMapDocument, renderer: &mut Renderer) -> bool {
        let status_changed = self.show_status(doc, renderer);
        let Some(pass) = self.advance(doc) else {
            return status_changed;
        };
        let projected = pass
            .changes
            .as_ref()
            .into_iter()
            .flat_map(LayerRenderer::journals_of);
        let stamped = pass.stamps.iter().flat_map(OverlayJournals::journals);
        renderer.apply_layer_journals(&self.layers, projected.chain(stamped));
        if let Some(changes) = &pass.changes {
            renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
            if let Some(journal) = self.follow_chrome(changes, doc) {
                renderer.apply_chrome_journal(&self.chrome, &journal);
                journal.discard(&mut self.chrome);
            }
        }
        self.finish(pass);
        true
    }

    /// Hand a pass's `changes` to every chrome builder and apply what
    /// they return to the chrome layer, under one journal: the chrome
    /// half of a sync pass. `None` when no builder asked for anything,
    /// or when a mutator was refused — which reverts everything the
    /// builders applied in this pass, and is logged.
    ///
    /// Cost: each builder's reading of `changes`; the walks the
    /// mutators make.
    fn follow_chrome(
        &mut self,
        changes: &LayerChanges,
        doc: &MindMapDocument,
    ) -> Option<Journal<GfxElement>> {
        if self.chrome_builders.is_empty() {
            return None;
        }
        let pass = PassChanges {
            changes,
            doc: doc.doc(),
            projector: &self.projector,
        };
        let mutators: Vec<(GfxMutatorTree, Anchors)> = self
            .chrome_builders
            .iter_mut()
            .flat_map(|builder| builder.follow(&pass))
            .collect();
        if mutators.is_empty() {
            return None;
        }
        match self.journal_chrome(&mutators) {
            Ok(journal) => Some(journal),
            Err(error) => {
                log::warn!("view: a chrome builder's mutator was refused: {error}");
                None
            }
        }
    }

    /// Apply `mutators` to the chrome layer under one journal and close
    /// it: the one way the chrome is written. A refusal reverts
    /// everything applied before it and is returned.
    fn journal_chrome(
        &mut self,
        mutators: &[(GfxMutatorTree, Anchors)],
    ) -> Result<Journal<GfxElement>, TreeError> {
        let mut journal = JournalBuilder::new();
        for (mutator, anchors) in mutators {
            mutator.apply_at_journaled(&mut self.chrome, anchors, |_, _| (), &mut journal)?;
        }
        Ok(journal.close(&self.chrome))
    }

    /// Hand the status line to the renderer when it is not the one last
    /// shown: the active mode's (`follow_mode`), or else the document's.
    /// Returns whether it changed.
    fn show_status(&mut self, doc: &MindMapDocument, renderer: &mut Renderer) -> bool {
        let status = self.mode_line.as_deref().or(doc.status());
        if status == self.shown_status.as_deref() {
            return false;
        }
        let status = status.map(str::to_string);
        renderer.set_mode_status_text(status.clone());
        self.shown_status = status;
        true
    }

    /// The renderer-free half of [`Self::sync`]: drain the records once,
    /// update the projection, re-stamp the overlay, keep the records for
    /// the event stream. `None` when there is nothing to do.
    fn advance(&mut self, doc: &mut MindMapDocument) -> Option<Pass> {
        let records = doc.take_changes();
        let changes: Option<LayerChanges> = (!records.is_empty()).then(|| {
            doc.retain_live_selection();
            let mut fonts = FontSystemLease::new("view::sync");
            self.projector
                .update(doc.doc(), &records, &mut self.layers, &mut fonts)
        });
        // A mode's sources wear their own tint instead of the selection's,
        // and the hover target is neither (`set_hover_target`), so no two
        // overlays are stamped on one run.
        let MapView {
            highlight,
            hover,
            hover_target,
            sources,
            source_targets,
            band_targets,
            projector,
            layers,
            ..
        } = self;
        let mut targets: Vec<NodeId> = match band_targets {
            Some(covered) => covered.clone(),
            None => doc.selection.iter().collect(),
        };
        targets.retain(|target| !source_targets.contains(target));
        let mut overlays: [(&mut Overlay, &[NodeId]); 3] = [
            (hover, hover_target.as_slice()),
            (sources, source_targets.as_slice()),
            (highlight, &targets),
        ];
        if changes.is_none() && overlays.iter().all(|(overlay, want)| overlay.targets() == *want) {
            return None;
        }
        // Every overlay's targets that leave are cleared before any
        // overlay stamps the ones that came, so a node that passes from
        // one overlay to another in this pass ends with the second's tint.
        let mut stamps = Vec::with_capacity(2 * overlays.len());
        for (overlay, want) in &mut overlays {
            let kept: Vec<NodeId> = overlay
                .targets()
                .iter()
                .copied()
                .filter(|t| want.contains(t))
                .collect();
            stamps.push(overlay.update(&kept, changes.as_ref(), doc.doc(), projector, layers));
        }
        for (overlay, want) in &mut overlays {
            stamps.push(overlay.update(want, None, doc.doc(), projector, layers));
        }
        self.feed.extend(records);
        Some(Pass { changes, stamps })
    }

    /// Follow the interaction mode: a reparent's or a connect's sources
    /// wear the source tint in place of the selection's, and its status
    /// line (`InteractionMode::status_line`) is drawn over the
    /// document's; any other mode has neither, and no target tint. Both
    /// are derived from the mode, so no document change can erase them
    /// while it lasts. Takes effect at the next [`Self::sync`]; the frame
    /// calls it before its sync, whatever changed the mode. O(1) when the
    /// mode is the one last followed. Native, with the two modes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn follow_mode(&mut self, mode: &crate::application::app::InteractionMode) {
        if self.mode == *mode {
            return;
        }
        self.mode = mode.clone();
        self.source_targets = mode.sources().to_vec();
        self.mode_line = mode.status_line();
        if self.source_targets.is_empty() {
            self.hover_target = None;
        }
    }

    /// The nodes tinted as a mode's sources.
    #[cfg(test)]
    pub(crate) fn sources(&self) -> &[NodeId] {
        &self.source_targets
    }

    /// Discard a pass's journals once the renderer has applied them.
    fn finish(&mut self, pass: Pass) {
        if let Some(changes) = pass.changes {
            changes.discard(&mut self.layers);
        }
        for stamps in pass.stamps {
            stamps.discard(&mut self.layers);
        }
    }

    /// The node whose body holds `point`, on the canvas — a path or a
    /// label drawn over it does not stand in for it
    /// (`Projector::node_at`): what a reparent or a connect lands on.
    /// Native, with the two modes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn node_at(&self, point: Vec2) -> Option<NodeId> {
        self.projector.node_at(point)
    }

    /// Tint `target` as the node a reparent or a connect would land on,
    /// or tint none with `None`; the caller passes only a valid target
    /// that is not selected. Takes effect at the next [`Self::sync`].
    /// Returns whether it changed. Native, with the two modes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_hover_target(&mut self, target: Option<NodeId>) -> bool {
        if self.hover_target == target {
            return false;
        }
        self.hover_target = target;
        true
    }

    /// The node tinted as a reparent's or a connect's target, if any.
    #[cfg(test)]
    pub(crate) fn hover_target(&self) -> Option<NodeId> {
        self.hover_target
    }

    /// A view of `doc` with no renderer behind it, for the tests that
    /// read what the pipeline wrote ([`Self::sync_for_test`]).
    #[cfg(test)]
    pub(crate) fn for_test(doc: &MindMapDocument) -> MapView {
        Self::for_test_with_chrome(doc, Vec::new())
    }

    /// [`Self::for_test`], with `builders` registered as
    /// [`Self::load_with_chrome`] registers them.
    #[cfg(test)]
    pub(crate) fn for_test_with_chrome(
        doc: &MindMapDocument,
        builders: Vec<Box<dyn ChromeBuilder>>,
    ) -> MapView {
        let mut doc = MindMapDocument::new(doc.doc().clone(), None, doc.history_limit());
        let (mut view, stamps) = Self::project(&mut doc, builders);
        stamps.discard(&mut view.layers);
        view
    }

    /// [`Self::sync`] without a renderer: what the pass wrote, or `None`
    /// when it found nothing to do.
    #[cfg(test)]
    pub(crate) fn sync_for_test(&mut self, doc: &mut MindMapDocument) -> Option<SyncReport> {
        let pass = self.advance(doc)?;
        let projected = pass.changes.as_ref().map_or(0, |changes| {
            LayerRenderer::journals_of(changes)
                .map(|(_, j)| j.touched().count())
                .sum()
        });
        let stamped = pass
            .stamps
            .iter()
            .flat_map(OverlayJournals::journals)
            .flat_map(|(layer, journal)| journal.touched().map(move |(id, _)| (layer, id)))
            .collect();
        let chrome = match &pass.changes {
            Some(changes) => match self.follow_chrome(changes, doc) {
                Some(journal) => {
                    let touched = journal.touched().map(|(id, _)| id).collect();
                    journal.discard(&mut self.chrome);
                    touched
                }
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        self.finish(pass);
        Some(SyncReport {
            projected,
            stamped,
            chrome,
        })
    }

    /// The change records synced since the event loop last cleared them
    /// ([`Self::clear_feed`]), oldest first: what the IPC event stream
    /// reads to report document changes and their sources. O(1).
    pub(crate) fn pending_feed(&self) -> &[ChangeRecord] {
        &self.feed
    }

    /// Drop the records the event stream has seen: called once per
    /// event-loop iteration, after its last observation. O(the records).
    pub(crate) fn clear_feed(&mut self) {
        self.feed.clear();
    }

    /// What `point`, on the canvas, hits: the projector's hit test, top
    /// layer down, then the nodes' bodies. `&mut` for the layers' hit
    /// memo, which the first query after a change fills.
    pub(crate) fn hit(&mut self, point: Vec2) -> Option<Hit> {
        self.projector.hit(&mut self.layers, point)
    }

    /// The visible nodes `rect`, on the canvas, covers: what a rubber band
    /// over it selects (`Projector::nodes_in`). Native with the rubber
    /// band's gesture (`CLAUDE.md` "Dual-target status"); the highlight
    /// that reads the band targets is both targets'.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn nodes_in(&self, rect: Rect) -> Vec<NodeId> {
        self.projector.nodes_in(rect)
    }

    /// Point the highlight at `covered` while a rubber band is out, or
    /// back at the selection with `None`. Takes effect at the next
    /// [`Self::sync`]. Returns whether it changed. Native, with the
    /// rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_band_targets(&mut self, covered: Option<Vec<NodeId>>) -> bool {
        if self.band_targets == covered {
            return false;
        }
        self.band_targets = covered;
        true
    }

    /// The rubber band's covered set, while one is out. Native, with the
    /// rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn band_targets(&self) -> Option<&[NodeId]> {
        self.band_targets.as_deref()
    }

    /// Apply `mutators` — each a glyph mutator and the chrome elements it
    /// is anchored at — to the chrome layer, under one journal, and hand
    /// the journal to the renderer before it is discarded: the chrome's
    /// one write path, as the projector's journals are the glyph layers'.
    /// A refusal reverts everything this call applied and is returned;
    /// the renderer is then not called.
    ///
    /// Cost: the walks the mutators make, and the renderer's apply of
    /// the elements the journal names.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the chrome's one user so far, the rubber band, is native; V1-C7's and V1-C8's chrome is both targets'"
        )
    )]
    pub(crate) fn apply_chrome(
        &mut self,
        mutators: &[(GfxMutatorTree, Anchors)],
        renderer: &mut Renderer,
    ) -> Result<(), TreeError> {
        let journal = self.journal_chrome(mutators)?;
        renderer.apply_chrome_journal(&self.chrome, &journal);
        journal.discard(&mut self.chrome);
        Ok(())
    }

    /// Draw the rubber band over `rect`, or hide it with `None`, through
    /// [`Self::apply_chrome`]: the band's four runs get new payloads, and
    /// nothing else is touched. Nothing at all when the band is already
    /// so. Returns whether it changed. Native, with the rubber band's
    /// gesture (`CLAUDE.md` "Dual-target status"); the chrome's write
    /// path is both targets'.
    ///
    /// Cost: four box shapings, under one font lease, and the renderer's
    /// apply of four runs.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_band(&mut self, rect: Option<Rect>, renderer: &mut Renderer) -> bool {
        if self.band_rect == rect {
            return false;
        }
        let mutators: Vec<(GfxMutatorTree, Anchors)> = band::runs_for(rect)
            .into_iter()
            .zip(self.band)
            .map(|(run, id)| {
                (
                    GfxMutatorTree::new_with(GfxMutator::new(
                        GfxMutation::RunSet(Box::new(run)),
                        RUN_CHANNEL,
                    )),
                    Anchors::Nodes(vec![id]),
                )
            })
            .collect();
        match self.apply_chrome(&mutators, renderer) {
            Ok(()) => {
                self.band_rect = rect;
                true
            }
            Err(error) => {
                log::warn!("view: the rubber band was refused: {error}");
                false
            }
        }
    }

    /// The chrome layer, for the tests that read what the band wrote.
    #[cfg(test)]
    pub(crate) fn chrome(&self) -> &GfxTree {
        &self.chrome
    }

    /// The rubber band's four runs in the chrome layer. Test builds only.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn band_runs(&self) -> [NodeId; 4] {
        self.band
    }

    /// The projection state, to read geometry from (`Projector`'s
    /// `Geometry`: node rectangles, section boxes, curves, label places).
    pub(crate) fn projector(&self) -> &Projector {
        &self.projector
    }

    /// The glyph layers, for the tests and the IPC scene dump, which
    /// report what is drawn — never to learn what the document is.
    pub(crate) fn layers(&self) -> &GlyphLayers {
        &self.layers
    }

    /// The elements the highlight is stamped on now.
    #[cfg(test)]
    pub(crate) fn highlighted(&self) -> &[NodeId] {
        self.highlight.targets()
    }
}

/// One pass of the pipeline, between its update and its discard.
struct Pass {
    /// What the projector wrote, when the document changed.
    changes: Option<LayerChanges>,
    /// What the overlays cleared, then what they stamped.
    stamps: Vec<OverlayJournals>,
}

/// What [`MapView::sync_for_test`] saw a pass write.
#[cfg(test)]
pub(crate) struct SyncReport {
    /// How many element touches the projector's journals named.
    pub(crate) projected: usize,
    /// The glyphs the overlay stamped or cleared, by layer.
    pub(crate) stamped: Vec<(baumhard::projection::Layer, NodeId)>,
    /// The chrome elements the chrome builders' journal touched.
    pub(crate) chrome: Vec<NodeId>,
}

/// An empty chrome layer: a `Void` root, drawn after the five glyph
/// layers.
fn chrome_layer() -> GfxTree {
    let mut tree = GfxTree::new_non_indexed();
    tree.layer = baumhard::projection::Layer::ALL.len();
    tree
}
