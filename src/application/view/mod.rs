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
//!                  glyph-layer presentations put back where a run is
//!                  rewritten; overlays re-stamped on what changed;
//!                  the presentations written again on those runs
//!                                           │ their journals
//!                                           ▼
//!          LayerRenderer::apply(projector's ++ overlays' ++ presentations')
//!                                           │ LayerChanges
//!                                           ▼
//!      handles, caret, section frames ──▶ LayerRenderer::apply_chrome
//!                                           │
//!                                           ▼ discard every journal
//! ```
//!
//! [`MapView::load`] is the only full pass: one projection of the whole
//! document, one load of the renderer's state. After it, every change
//! goes through [`MapView::sync`], which drains the document's change
//! feed **once**, hands the records to the projector, re-stamps the
//! overlays on exactly the runs the projector wrote, writes the
//! glyph-layer presentations on exactly the runs that need them
//! ([`present`]), applies every set of journals to the renderer, discards
//! them, and keeps the records for
//! the IPC event stream, which reads them at its next look at the app
//! ([`MapView::pending_feed`]). There is no rebuild function. A camera change
//! calls none of this: the renderer applies the camera when it draws
//! (§12.4).
//!
//! **Chrome that follows app state as well as the document** — the
//! **handles** ([`handles`]: the selection and the mode), a text editor's
//! **caret** ([`caret`]: the editor's focus, `MapView::follow_editor`) and
//! NodeEdit's **section frames** ([`frames`]: the mode) — is kept by the
//! view itself (ruling K1 on #249): every pass compares what each wants
//! with what it shows and asks whether the pass's changes touched or
//! moved its target, and writes what changed under one chrome journal in
//! that pass — and a pass that changed none of it, a pan's or a zoom's,
//! writes no chrome. NodeEdit's **dim** and the editor's selected
//! **range** are overlays ([`overlay::Dim`], [`overlay::RangeTint`]),
//! planned and written with the others.
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
pub(crate) mod band;
pub(crate) mod caret;
pub(crate) mod frames;
pub(crate) mod handles;
pub(crate) mod inspect;
pub(crate) mod outline;
pub(crate) mod overlay;
pub(crate) mod present;
#[cfg(test)]
mod tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests_handles;
#[cfg(test)]
mod tests_present;

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
use crate::application::keybinds::ColorAxis;
use crate::application::renderer::{LayerRenderer, Renderer};
use baumhard::document::ids::Document;

use self::caret::{Caret, EditorFocus};
use self::frames::SectionFrames;
use self::overlay::{
    Dim, Overlay, OverlayJournals, RangeTint, SELECTION_HIGHLIGHT, SOURCE_HIGHLIGHT, TARGET_HIGHLIGHT,
};

/// The view of one open map. See the [module docs](self).
pub(crate) struct MapView {
    /// The projection state: what every glyph element was projected from.
    projector: Projector,
    /// The five glyph layers the projector writes.
    layers: GlyphLayers,
    /// The chrome layer (design §12.1): view-only glyphs built from app
    /// state rather than the document — the rubber band
    /// (`view::band`, native), the handles ([`handles`]), a text editor's
    /// caret ([`caret`]) and NodeEdit's section frames ([`frames`]). Drawn
    /// above the five layers; written only under a journal the renderer
    /// applies — [`Self::apply_chrome`]'s, or a sync pass's.
    chrome: GfxTree,
    /// A text editor's caret, in the chrome.
    caret: Caret,
    /// What the open text editor edits and where its caret is
    /// ([`Self::follow_editor`]); `None` with no editor open.
    editing: Option<EditorFocus>,
    /// NodeEdit's section frames, in the chrome.
    frames: SectionFrames,
    /// The node NodeEdit edits, while it is active
    /// ([`Self::follow_node_edit`]).
    node_edit: Option<NodeId>,
    /// NodeEdit's dim: every other node faded, stamped on their runs.
    dim: Dim,
    /// An open editor's selected range, stamped on its run.
    range: RangeTint,
    /// The rubber band's four runs in [`Self::chrome`]. Native, with
    /// the rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    band: band::BandRuns,
    /// The rectangle the band is drawn over, while one is out. Native,
    /// with the rubber band.
    #[cfg(not(target_arch = "wasm32"))]
    band_rect: Option<Rect>,
    /// The handle chrome ([`handles`]): the resize mode's handles and a
    /// selected path's, in the chrome layer, and where they center.
    handles: handles::Handles,
    /// The selection tint, stamped on the selection — or, while a rubber
    /// band is out, on what it covers (`band_targets`).
    highlight: Overlay,
    /// The reparent and connect target tint, stamped on
    /// [`Self::hover_target`].
    hover: Overlay,
    /// The node a reparent or a connect would land on, while the pointer
    /// is over a valid one in either mode.
    hover_target: Option<NodeId>,
    /// The glyph-layer presentation instances as they stand on the
    /// layers ([`present`]).
    stage: present::Stage,
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
    /// The contextual color picker's axis and targets while one is open
    /// (`MapView::follow_picker`): on those targets the selection tint
    /// stands down on what the axis colors, so the color the picker
    /// stages shows. Native, with the picker; always `None` in the
    /// browser.
    picking: Option<(ColorAxis, Vec<NodeId>)>,
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
    /// a document does, and the only full pass in its life (§3 "Load"). A
    /// rescan that finds a font does not come back here: it reshapes the
    /// units that drew in the family (`MapView::faces_added`, native).
    /// Any change records the document holds are dropped: the projection
    /// is of the document as it now stands.
    ///
    /// Cost: one walk of the visible document, every flow shaped and
    /// placed once, under one font lease; one walk of each layer by the
    /// renderer.
    pub(crate) fn load(doc: &mut MindMapDocument, renderer: &mut Renderer) -> MapView {
        let (mut view, stamps) = Self::project(doc);
        renderer.load_view(&view.layers, &view.chrome);
        renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
        stamps.discard(&mut view.layers);
        // The status line the map opens with: its missing-font notice,
        // when it names a font this machine lacks (#329).
        view.show_status(doc, renderer);
        view
    }

    /// The projection half of [`Self::load`]: the view of `doc`, its
    /// chrome planted, its highlight stamped, and the stamps' journals
    /// still to discard.
    fn project(doc: &mut MindMapDocument) -> (MapView, OverlayJournals) {
        doc.take_changes();
        let mut layers = GlyphLayers::new();
        let projector = {
            let mut fonts = FontSystemLease::new("view::load");
            Projector::project_full(doc.doc(), &mut layers, &mut fonts)
        };
        let mut chrome = chrome_layer();
        #[cfg(not(target_arch = "wasm32"))]
        let band = band::plant(&mut chrome);
        let handles = handles::Handles::plant(&mut chrome);
        let caret = Caret::plant(&mut chrome);
        let frames = SectionFrames::plant(&mut chrome);
        let mut view = MapView {
            projector,
            layers,
            chrome,
            caret,
            editing: None,
            frames,
            node_edit: None,
            dim: Dim::default(),
            range: RangeTint::default(),
            #[cfg(not(target_arch = "wasm32"))]
            band,
            #[cfg(not(target_arch = "wasm32"))]
            band_rect: None,
            handles,
            highlight: Overlay::tint(SELECTION_HIGHLIGHT),
            hover: Overlay::tint(TARGET_HIGHLIGHT),
            hover_target: None,
            stage: present::Stage::of(doc.doc()),
            sources: Overlay::tint(SOURCE_HIGHLIGHT),
            source_targets: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            mode: Default::default(),
            mode_line: None,
            picking: None,
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
    /// either, no presentation instance started, ended or moved, and the
    /// status line is the one shown: what a frame with nothing to do, a
    /// pan and a zoom cost. Returns whether it did anything —
    /// whether the frame owes a redraw.
    ///
    /// The status line is the document's ([`MindMapDocument::status`],
    /// D28): what a refused change says, handed to the renderer's status
    /// line when it changes — the one place every family's refusal is
    /// drawn, on both targets.
    ///
    /// The chrome — the handles, the caret, the section frames — is
    /// brought up to date in the same pass, under one journal the renderer
    /// applies before the pass discards its journals
    /// ([`Self::chrome_pass`]).
    ///
    /// Cost: the projector's update (proportional to the changes and their
    /// readers), the overlays' re-stamp (the runs named), the glyph-layer
    /// presentations' rewrite (the runs [`present`] names), the chrome's
    /// reading of the changes, the renderer's apply (the elements the
    /// journals name); O(1) for the status line.
    pub(crate) fn sync(&mut self, doc: &mut MindMapDocument, renderer: &mut Renderer) -> bool {
        self.sync_with_faces(doc, renderer, &[])
    }

    /// [`Self::sync`] after a rescan added the faces of the installed
    /// `families` (#329, `missing_fonts::rescan`): in the same pass, every
    /// unit that drew in one of them is resolved and shaped again in its
    /// new faces (`Projector::update_faces`), though its key did not
    /// change. Proportional to those units, as any pass is to what it
    /// reaches (design §20 rules 4–5): nothing is projected whole. The
    /// change records the document held are drained into the feed as any
    /// pass drains them, so none is lost, and the view's mode and status
    /// line are its own and stay. Returns whether it did anything.
    ///
    /// Cost: [`Self::sync`]'s, and the units that drew in the families.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn faces_added(
        &mut self,
        doc: &mut MindMapDocument,
        renderer: &mut Renderer,
        families: &[String],
    ) -> bool {
        self.sync_with_faces(doc, renderer, families)
    }

    /// The body of [`Self::sync`] and `MapView::faces_added` (native).
    fn sync_with_faces(
        &mut self,
        doc: &mut MindMapDocument,
        renderer: &mut Renderer,
        families: &[String],
    ) -> bool {
        let status_changed = self.show_status(doc, renderer);
        let pass = self.advance(doc, families);
        let Some(pass) = pass else {
            let drew = self.write_chrome(doc, None, renderer);
            return status_changed || drew;
        };
        let projected = pass
            .changes
            .as_ref()
            .into_iter()
            .flat_map(LayerRenderer::journals_of);
        let stamped = pass.stamps.iter().flat_map(OverlayJournals::journals);
        let presented = pass.presented.journals();
        renderer.apply_layer_journals(&self.layers, projected.chain(stamped).chain(presented));
        if pass.changes.is_some() {
            renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
        }
        self.write_chrome(doc, pass.changes.as_ref(), renderer);
        self.finish(pass);
        true
    }

    /// The chrome half of a sync pass ([`Self::chrome_pass`]), handed to
    /// the renderer and discarded. Returns whether it wrote any chrome; a
    /// refusal reverts all of it and is logged.
    fn write_chrome(
        &mut self,
        doc: &MindMapDocument,
        changes: Option<&LayerChanges>,
        renderer: &mut Renderer,
    ) -> bool {
        match self.chrome_pass(doc, changes) {
            Ok(Some(journal)) => {
                renderer.apply_chrome_journal(&self.chrome, &journal);
                journal.discard(&mut self.chrome);
                true
            }
            Ok(None) => false,
            Err(error) => {
                log::warn!("view: a chrome mutator was refused: {error}");
                false
            }
        }
    }

    /// Bring the chrome up to date for this pass, under one journal: the
    /// handles ([`Self::follow_handles`]), the caret ([`Self::follow_caret`])
    /// and the section frames ([`Self::follow_frames`]) — whose pool, when
    /// a node has more sections than it has slots, is grown first, in the
    /// same journal. `None` when nothing of it changed, which is a pan's
    /// and a zoom's pass. A refusal reverts everything applied before it.
    ///
    /// Cost: what each of the three costs; no chrome walk.
    fn chrome_pass(
        &mut self,
        doc: &MindMapDocument,
        changes: Option<&LayerChanges>,
    ) -> Result<Option<Journal<GfxElement>>, TreeError> {
        let mut journal = JournalBuilder::new();
        let mut any = false;
        let node = self.node_edit.filter(|&node| doc.doc().get(node).is_some());
        let focus = self.focused_section(doc, node);
        let follow_frames = changes.is_some()
            && node.is_some_and(|node| {
                self.projector.geometry_moved(node)
                    || doc
                        .doc()
                        .children(node)
                        .any(|child| self.projector.geometry_moved(child))
            });
        if self.frames.due(node, focus, follow_frames) {
            let want = SectionFrames::wanted(doc.doc(), &self.projector, node, focus);
            let grow = self.frames.grow(want.len());
            if !grow.is_empty() {
                for (mutator, anchors) in &grow {
                    mutator.apply_at_journaled(&mut self.chrome, anchors, |_, _| (), &mut journal)?;
                }
                self.frames.adopt(&self.chrome);
                any = true;
            }
            let frames = self.frames.update(node, focus, &want);
            for (mutator, anchors) in &frames {
                mutator.apply_at_journaled(&mut self.chrome, anchors, |_, _| (), &mut journal)?;
                any = true;
            }
        }
        let mut mutators = self.follow_handles(doc, changes);
        mutators.extend(self.follow_caret(doc, changes));
        for (mutator, anchors) in &mutators {
            mutator.apply_at_journaled(&mut self.chrome, anchors, |_, _| (), &mut journal)?;
            any = true;
        }
        Ok(any.then(|| journal.close(&self.chrome)))
    }

    /// The section NodeEdit focuses on `node`: the one an editor has open,
    /// or else the one selected. O(1).
    fn focused_section(&self, doc: &MindMapDocument, node: Option<NodeId>) -> Option<NodeId> {
        let node = node?;
        let in_node = |section: NodeId| {
            crate::application::document::selection::kind_of(doc.doc(), section)
                == Some(baumhard::document::element::DocKind::Section)
                && doc.doc().parent(section) == Some(node)
        };
        self.editing
            .map(|focus| focus.owner)
            .filter(|&owner| in_node(owner))
            .or_else(|| {
                let mut selected = doc.selection.iter();
                match (selected.next(), selected.next()) {
                    (Some(only), None) if in_node(only) => Some(only),
                    _ => None,
                }
            })
    }

    /// The caret's chrome mutator for this pass ([`caret::Caret::update`]):
    /// written again when the editor's focus changed, or when `changes`
    /// touched its owner ([`caret::touches`]) or may have moved its
    /// geometry or its node's (`Projector::geometry_moved`). Nothing for a
    /// pass that changed none of it.
    fn follow_caret(
        &mut self,
        doc: &MindMapDocument,
        changes: Option<&LayerChanges>,
    ) -> Option<(GfxMutatorTree, Anchors)> {
        let follow = match (changes, self.editing.or(self.caret.focus())) {
            (Some(changes), Some(focus)) => {
                let moved = |element: NodeId| self.projector.geometry_moved(element);
                caret::touches(changes, doc.doc(), &self.projector, focus.owner)
                    || moved(focus.owner)
                    || moved(focus.host)
                    || doc.doc().parent(focus.owner).is_some_and(moved)
            }
            _ => false,
        };
        self.caret
            .update(self.editing, follow, doc.doc(), &self.projector, &self.layers)
    }

    /// Follow the open text editor: what it edits and where its caret and
    /// its anchor are, or `None` once it closes. While it is open the
    /// selection tint stands down on what it edits, its selected range is
    /// tinted, and its caret is drawn; each takes effect at the next
    /// [`Self::sync`], which works out whether anything changed. O(1).
    pub(crate) fn follow_editor(&mut self, focus: Option<EditorFocus>) {
        self.editing = focus;
    }

    /// The caret, for the tests that read where it is drawn.
    #[cfg(test)]
    pub(crate) fn caret(&self) -> &Caret {
        &self.caret
    }

    /// The section frames, for the tests that read what they show.
    #[cfg(test)]
    pub(crate) fn frames(&self) -> &SectionFrames {
        &self.frames
    }

    /// NodeEdit's dim, for the tests that read what it stamps.
    #[cfg(test)]
    pub(crate) fn dim(&self) -> &Dim {
        &self.dim
    }

    /// The editor's range tint, for the tests that read what it stamps.
    #[cfg(test)]
    pub(crate) fn range_tint(&self) -> &RangeTint {
        &self.range
    }

    /// Follow NodeEdit on both targets: the node it edits while `mode` is
    /// NodeEdit — the other nodes dimmed, its sections framed — or none.
    /// In the browser, which follows no other mode, the mode's status line
    /// with it. Takes effect at the next [`Self::sync`]. O(1).
    pub(crate) fn follow_node_edit(&mut self, mode: &crate::application::app::InteractionMode) {
        self.node_edit = mode.node_edit();
        #[cfg(target_arch = "wasm32")]
        {
            self.mode_line = mode.status_line();
        }
    }

    /// The handle chrome's mutators for this pass ([`handles`]): a set is
    /// shown again where what it is shown on changed — the selection, the
    /// resize mode's target — or where the projection that produced
    /// `changes` may have moved its target's geometry
    /// ([`Projector::geometry_moved`]): a node's drawn rectangle, a
    /// section's box or its node's, a path's curve. On geometry, not on
    /// the runs a target owns: a node that draws no frame grows without
    /// writing a run of its own (#395's review). Nothing for a pass that
    /// projected nothing, which is a pan's and a zoom's.
    ///
    /// Cost: O(1) to compare the targets and to ask of each; each set
    /// shown again, its slots.
    fn follow_handles(
        &mut self,
        doc: &MindMapDocument,
        changes: Option<&LayerChanges>,
    ) -> Vec<(GfxMutatorTree, Anchors)> {
        let targets = handles::targets_for(doc.doc(), &doc.selection, self.resize_target());
        let follow = match changes {
            Some(_) => handles::HandleSet::ALL.map(|set| {
                targets[set.index()].is_some_and(|target| {
                    let moved = |element| self.projector.geometry_moved(element);
                    moved(target)
                        || (set == handles::HandleSet::Section && doc.doc().parent(target).is_some_and(moved))
                })
            }),
            None => [false; 3],
        };
        self.handles.update(targets, follow, doc.doc(), &self.projector)
    }

    /// The resize mode's target, as the view last followed the mode: the
    /// browser has no resize mode.
    fn resize_target(&self) -> Option<crate::application::app::ResizeTarget> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.mode.resize_target()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// The handle a press at `point` grabs, within `tolerance` canvas
    /// units of its center ([`handles::Handles::at`]): asked before the
    /// projector is, since the chrome is drawn over the map.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn handle_at(&self, point: Vec2, tolerance: f32) -> Option<handles::Handle> {
        self.handles.at(point, tolerance)
    }

    /// Where `handle` centers while it is shown.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn handle_center(&self, handle: handles::Handle) -> Option<Vec2> {
        self.handles.center_of(handle)
    }

    /// The handle chrome, for the tests that read what it holds.
    #[cfg(test)]
    pub(crate) fn handles(&self) -> &handles::Handles {
        &self.handles
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
    /// shown: the active mode's (`follow_mode`), or else the document's
    /// refusal, or else its missing-font notice (#329). Returns whether
    /// it changed.
    fn show_status(&mut self, doc: &MindMapDocument, renderer: &mut Renderer) -> bool {
        let status = self.mode_line.as_deref().or(doc.status()).or(doc.font_notice());
        if status == self.shown_status.as_deref() {
            return false;
        }
        let status = status.map(str::to_string);
        renderer.set_mode_status_text(status.clone());
        self.shown_status = status;
        true
    }

    /// The renderer-free half of [`Self::sync`]: drain the records once,
    /// update the projection — with the units that drew in `families`
    /// reshaped, after a rescan found them (`MapView::faces_added`, native) —
    /// re-stamp the overlay, keep the records for the event stream. `None`
    /// when there is nothing to do.
    fn advance(&mut self, doc: &mut MindMapDocument, families: &[String]) -> Option<Pass> {
        let records = doc.take_changes();
        let changes: Option<LayerChanges> = (!records.is_empty() || !families.is_empty()).then(|| {
            doc.retain_live_selection();
            let mut fonts = FontSystemLease::new("view::sync");
            let families: Vec<&str> = families.iter().map(String::as_str).collect();
            self.projector
                .update_faces(doc.doc(), &records, &families, &mut self.layers, &mut fonts)
        });
        // One tint per node, by construction: a mode's sources wear their
        // own instead of the selection's, and the target tint goes only on
        // a node that wears neither, so no two overlays stamp one run.
        let MapView {
            highlight,
            hover,
            hover_target,
            sources,
            source_targets,
            band_targets,
            picking,
            projector,
            layers,
            stage,
            editing,
            node_edit,
            dim,
            range,
            ..
        } = self;
        let mut targets: Vec<NodeId> = match band_targets {
            Some(covered) => covered.clone(),
            None => doc.selection.iter().collect(),
        };
        targets.retain(|target| !source_targets.contains(target));
        if let Some((axis, picked)) = picking {
            targets = stand_down(doc.doc(), targets, *axis, picked);
        }
        // What an open editor edits shows its caret and its selected
        // range, not the selection's tint.
        if let Some(focus) = editing {
            targets.retain(|&target| target != focus.host && target != focus.owner);
        }
        let hovered: Option<NodeId> =
            hover_target.filter(|target| !targets.contains(target) && !source_targets.contains(target));
        // NodeEdit's dim leaves every node another overlay stamps, so no
        // two overlays share a run.
        let active = node_edit.filter(|&node| doc.doc().get(node).is_some());
        let excluded: Vec<NodeId> = if active.is_some() {
            targets
                .iter()
                .chain(source_targets.iter())
                .chain(hovered.iter())
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        let want_range = editing.as_ref().and_then(|focus| {
            let (start, end) = focus.range()?;
            caret::range_glyphs(doc.doc(), projector, layers, focus.owner, start, end)
        });
        let mut overlays: [(&mut Overlay, &[NodeId]); 3] = [
            (hover, hovered.as_slice()),
            (sources, source_targets.as_slice()),
            (highlight, &targets),
        ];
        if changes.is_none()
            && overlays.iter().all(|(overlay, want)| overlay.targets() == *want)
            && dim.is_current(active, &excluded)
            && range.is_current(want_range.as_ref())
            && stage.is_current(doc.doc())
        {
            return None;
        }
        // Every overlay's targets that leave are cleared before any
        // overlay stamps the ones that came, so a node that passes from
        // one overlay to another in this pass ends with the second's tint.
        // All of it is planned before any of it is written: a run a
        // glyph-layer presentation plays on is put back to its base
        // before an overlay writes it, and played on again after.
        let mut plans = Vec::with_capacity(2 * overlays.len());
        for (index, (overlay, want)) in overlays.iter_mut().enumerate() {
            let kept: Vec<NodeId> = overlay
                .targets()
                .iter()
                .copied()
                .filter(|t| want.contains(t))
                .collect();
            plans.push((
                index,
                overlay.plan(&kept, changes.as_ref(), doc.doc(), projector, layers),
            ));
        }
        let (dim_first, dim_second) =
            dim.plan(active, &excluded, changes.as_ref(), doc.doc(), projector, layers);
        for (index, (overlay, want)) in overlays.iter_mut().enumerate() {
            plans.push((index, overlay.plan(want, None, doc.doc(), projector, layers)));
        }
        let (range_plan, range_glyphs) = range.plan(want_range, changes.as_ref());
        let lifted = stage.lift(
            doc.doc(),
            projector,
            changes.as_ref(),
            plans
                .iter()
                .map(|(_, planned)| planned)
                .chain([&dim_first, &dim_second, &range_plan])
                .flat_map(|planned| planned.runs()),
            layers,
        );
        // Every overlay's clears before any overlay's stamps: the overlays'
        // own leaving runs, then the dim's clears and re-stamps, then the
        // overlays' stamps, the dim's new stamps and the range's.
        let kept = overlays.len();
        let mut stamps: Vec<OverlayJournals> = Vec::with_capacity(plans.len() + 3);
        let mut plans = plans.into_iter();
        for (index, planned) in plans.by_ref().take(kept) {
            stamps.push(overlays[index].0.write(planned, layers));
        }
        stamps.push(dim.write(dim_first, layers));
        for (index, planned) in plans {
            stamps.push(overlays[index].0.write(planned, layers));
        }
        stamps.push(dim.write(dim_second, layers));
        stamps.push(range.write(range_plan, range_glyphs, layers));
        let presented = stage.stamp(lifted, doc.doc(), projector, layers);
        self.feed.extend(records);
        Some(Pass {
            changes,
            stamps,
            presented,
        })
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
        self.node_edit = mode.node_edit();
        if self.source_targets.is_empty() {
            self.hover_target = None;
        }
    }

    /// Follow the contextual color picker: while one is open on
    /// `picking`'s targets, the selection tint stands down on what its
    /// axis colors there ([`stand_down`]), so the staged color is what is
    /// drawn; with `None` — no picker, or the standalone palette — the
    /// tint is back. Takes effect at the next [`Self::sync`]. O(the
    /// targets) to compare. Native, with the picker.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn follow_picker(&mut self, picking: Option<(ColorAxis, &[NodeId])>) {
        let same = match (&self.picking, picking) {
            (None, None) => true,
            (Some((axis, targets)), Some((now, picked))) => *axis == now && targets.as_slice() == picked,
            _ => false,
        };
        if !same {
            self.picking = picking.map(|(axis, picked)| (axis, picked.to_vec()));
        }
    }

    /// The selection tint's targets as the overlay holds them. Test
    /// builds only.
    #[cfg(test)]
    pub(crate) fn tinted(&self) -> &[NodeId] {
        self.highlight.targets()
    }

    /// Every run the selection tint stamps now. Test builds only.
    #[cfg(test)]
    pub(crate) fn tinted_runs(&self) -> Vec<(baumhard::projection::Layer, NodeId)> {
        self.highlight.stamped_runs()
    }

    /// Whether the run `glyph` of `layer` is drawn in the selection tint:
    /// a color override of the tint on it, as the layers hold it — what a
    /// projector rewrite wipes and a re-stamp writes again. Test builds
    /// only.
    #[cfg(test)]
    pub(crate) fn run_is_tinted(&self, layer: baumhard::projection::Layer, glyph: NodeId) -> bool {
        self.layers
            .tree(layer)
            .get(glyph)
            .and_then(|node| node.get().glyph_run())
            .is_some_and(|run| run.overrides().color().contains(&Some(SELECTION_HIGHLIGHT)))
    }

    /// The runs `element` draws, by layer. Test builds only.
    #[cfg(test)]
    pub(crate) fn runs_of_for_test(
        &self,
        doc: &Document,
        element: NodeId,
    ) -> Vec<(baumhard::projection::Layer, NodeId)> {
        inspect::runs_of(doc, &self.projector, &self.layers, element)
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
        pass.presented.discard(&mut self.layers);
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
    /// or tint none with `None`; the caller passes only a valid target. A
    /// target that is selected or a source keeps that tint instead.
    /// Takes effect at the next [`Self::sync`], which works out whether
    /// it changed anything. Native, with the two modes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_hover_target(&mut self, target: Option<NodeId>) {
        self.hover_target = target;
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
        let mut doc = MindMapDocument::new(doc.doc().clone(), None, doc.history_limit());
        let (mut view, stamps) = Self::project(&mut doc);
        stamps.discard(&mut view.layers);
        view
    }

    /// [`Self::sync`] without a renderer: what the pass wrote, or `None`
    /// when it found nothing to do — no layer, no overlay, no chrome.
    #[cfg(test)]
    pub(crate) fn sync_for_test(&mut self, doc: &mut MindMapDocument) -> Option<SyncReport> {
        self.faces_added_for_test(doc, &[])
    }

    /// [`Self::faces_added`] without a renderer, as [`Self::sync_for_test`]
    /// is [`Self::sync`].
    #[cfg(test)]
    pub(crate) fn faces_added_for_test(
        &mut self,
        doc: &mut MindMapDocument,
        families: &[String],
    ) -> Option<SyncReport> {
        let pass = self.advance(doc, families);
        // The chrome half, as `sync` writes it, under one journal.
        let changes = pass.as_ref().and_then(|pass| pass.changes.as_ref());
        let chrome: Vec<NodeId> = match self.chrome_pass(doc, changes) {
            Ok(Some(journal)) => {
                let touched = journal.touched().map(|(id, _)| id).collect();
                journal.discard(&mut self.chrome);
                touched
            }
            Ok(None) => Vec::new(),
            Err(error) => panic!("a chrome mutator was refused: {error}"),
        };
        let Some(pass) = pass else {
            return (!chrome.is_empty()).then(|| SyncReport {
                projected: 0,
                rewritten: Vec::new(),
                stamped: Vec::new(),
                presented: Vec::new(),
                chrome,
            });
        };
        let projected = pass.changes.as_ref().map_or(0, |changes| {
            LayerRenderer::journals_of(changes)
                .map(|(_, j)| j.touched().count())
                .sum()
        });
        let rewritten = pass
            .changes
            .iter()
            .flat_map(LayerRenderer::journals_of)
            .flat_map(|(layer, journal)| journal.touched().map(move |(id, _)| (layer, id)))
            .collect();
        let stamped = pass
            .stamps
            .iter()
            .flat_map(OverlayJournals::journals)
            .flat_map(|(layer, journal)| journal.touched().map(move |(id, _)| (layer, id)))
            .collect();
        let mut presented: Vec<(baumhard::projection::Layer, NodeId)> = pass
            .presented
            .journals()
            .flat_map(|(layer, journal)| journal.touched().map(move |(id, _)| (layer, id)))
            .collect();
        presented.sort_unstable_by_key(|&(layer, glyph)| (layer.index(), glyph));
        presented.dedup();
        self.finish(pass);
        Some(SyncReport {
            projected,
            rewritten,
            stamped,
            presented,
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
            reason = "its one user, the rubber band, is native; the handles, on both targets, write in the sync pass"
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

    /// The glyph-layer presentations' stage, for the tests that count
    /// its work.
    #[cfg(test)]
    pub(crate) fn stage(&self) -> &present::Stage {
        &self.stage
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
    /// What the glyph-layer presentations put back, then what they
    /// wrote.
    presented: present::StageJournals,
}

/// What [`MapView::sync_for_test`] saw a pass write.
#[cfg(test)]
pub(crate) struct SyncReport {
    /// How many element touches the projector's journals named.
    pub(crate) projected: usize,
    /// The glyph elements the projector's journals named, by layer.
    pub(crate) rewritten: Vec<(baumhard::projection::Layer, NodeId)>,
    /// The glyphs the overlay stamped or cleared, by layer.
    pub(crate) stamped: Vec<(baumhard::projection::Layer, NodeId)>,
    /// The runs the glyph-layer presentations wrote — put back, or
    /// played on — by layer, each once, in layer then id order.
    pub(crate) presented: Vec<(baumhard::projection::Layer, NodeId)>,
    /// The chrome elements the pass's chrome journal touched: the
    /// handles', the caret's and the section frames'.
    pub(crate) chrome: Vec<NodeId>,
}

/// The selection tint's targets with a contextual color picker on
/// `picked`, picking `axis`: a picked element keeps its tint only where
/// the axis does not color it, so the staged color shows (#248's ruling on
/// the picker's preview). A node picked for its text keeps the tint on its
/// frame's rings; picked for its frame or its rings, on its sections; a
/// section picked for its text, and a path picked for its one color, stand
/// down whole. Every other target is kept as it is. O(the targets and the
/// picked nodes' children).
fn stand_down(doc: &Document, targets: Vec<NodeId>, axis: ColorAxis, picked: &[NodeId]) -> Vec<NodeId> {
    use crate::application::fields::style::frame_of;
    use crate::application::view::inspect::children_of_kind;
    use baumhard::document::element::DocKind;
    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        if !picked.contains(&target) {
            out.push(target);
            continue;
        }
        match (
            crate::application::document::selection::kind_of(doc, target),
            axis,
        ) {
            (Some(DocKind::Node), ColorAxis::Text) => {
                if let Some(frame) = frame_of(doc, target) {
                    out.extend(children_of_kind(doc, frame, DocKind::Ring));
                }
            }
            (Some(DocKind::Node), ColorAxis::Border | ColorAxis::Ring) => {
                out.extend(children_of_kind(doc, target, DocKind::Section));
            }
            (Some(DocKind::Section), ColorAxis::Text)
            | (Some(DocKind::Connector | DocKind::Link), ColorAxis::Text | ColorAxis::Border) => {}
            _ => out.push(target),
        }
    }
    out
}

/// An empty chrome layer: a `Void` root, drawn after the five glyph
/// layers.
fn chrome_layer() -> GfxTree {
    let mut tree = GfxTree::new_non_indexed();
    tree.layer = baumhard::projection::Layer::ALL.len();
    tree
}
