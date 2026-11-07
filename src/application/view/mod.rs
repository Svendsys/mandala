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
//! **One font lease per pass** (#241, B7's note): a load and a sync each
//! hold one `FontSystemLease`, taken at the first font read and released
//! when the pass ends, never across frames.
//!
//! **What is read here, and what is not.** Selection and hit testing ask
//! the projector — [`MapView::hit`], [`MapView::nodes_in`] — which keeps
//! the projection map; nothing reads a glyph layer to learn what the
//! document is.

pub(crate) mod inspect;
pub(crate) mod overlay;
#[cfg(test)]
mod tests;

use baumhard::arena::NodeId;
use baumhard::document::resolve::resolve_background;
use baumhard::document::writer::feed::ChangeRecord;
use baumhard::flow::Rect;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::{GlyphLayers, Hit, LayerChanges, Projector};
use glam::Vec2;

use crate::application::document::MindMapDocument;
use crate::application::renderer::{LayerRenderer, Renderer};

use self::overlay::{Overlay, OverlayJournals, SELECTION_HIGHLIGHT};

/// The view of one open map. See the [module docs](self).
pub(crate) struct MapView {
    /// The projection state: what every glyph element was projected from.
    projector: Projector,
    /// The five glyph layers the projector writes.
    layers: GlyphLayers,
    /// The chrome layer (design §12.1): view-only glyphs built from app
    /// state rather than the document — V1-C7's handles, V1-C8's section
    /// frames. Drawn above the five layers; nothing in V1-C3 puts a run
    /// in it.
    chrome: GfxTree,
    /// The selection tint, stamped on the selection — or, while a rubber
    /// band is out, on what it covers (`preview`).
    highlight: Overlay,
    /// What a rubber band covers while one is out: the highlight follows
    /// it instead of the selection, whose highlight stands down until the
    /// band's release replaces the selection.
    preview: Option<Vec<NodeId>>,
    /// The change records drained since the event loop last cleared
    /// them ([`Self::clear_feed`]): the second consumer of the one drain
    /// a sync makes (#244 amendment 3; V1-E2 builds events from them).
    /// Cleared every iteration, so it holds at most an iteration's
    /// changes.
    feed: Vec<ChangeRecord>,
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
        let (mut view, stamps) = Self::project(doc);
        renderer.load_view(&view.layers, &view.chrome);
        renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
        stamps.discard(&mut view.layers);
        view
    }

    /// The projection half of [`Self::load`]: the view of `doc`, its
    /// highlight stamped, and the stamps' journals still to discard.
    fn project(doc: &mut MindMapDocument) -> (MapView, OverlayJournals) {
        doc.take_changes();
        let mut layers = GlyphLayers::new();
        let projector = {
            let mut fonts = FontSystemLease::new("view::load");
            Projector::project_full(doc.doc(), &mut layers, &mut fonts)
        };
        let mut view = MapView {
            projector,
            layers,
            chrome: chrome_layer(),
            highlight: Overlay::tint(SELECTION_HIGHLIGHT),
            preview: None,
            feed: Vec::new(),
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
    /// the document did not change and the highlight's targets did not
    /// either: what a frame with nothing to do, a pan and a zoom cost.
    /// Returns whether it did anything — whether the frame owes a redraw.
    ///
    /// Cost: the projector's update (proportional to the changes and their
    /// readers), the overlay's re-stamp (the runs named), the renderer's
    /// apply (the elements the journals name).
    pub(crate) fn sync(&mut self, doc: &mut MindMapDocument, renderer: &mut Renderer) -> bool {
        let Some(pass) = self.advance(doc) else {
            return false;
        };
        let projected = pass.changes.as_ref().into_iter().flat_map(LayerRenderer::journals_of);
        renderer.apply_layer_journals(&self.layers, projected.chain(pass.stamps.journals()));
        if pass.changes.is_some() {
            renderer.set_background(resolve_background(doc.doc().tree(), &mut ()).0);
        }
        self.finish(pass);
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
        let targets: Vec<NodeId> = match &self.preview {
            Some(covered) => covered.clone(),
            None => doc.selection.iter().collect(),
        };
        if changes.is_none() && targets == self.highlight.targets() {
            return None;
        }
        let stamps =
            self.highlight
                .update(&targets, changes.as_ref(), doc.doc(), &self.projector, &mut self.layers);
        self.feed.extend(records);
        Some(Pass { changes, stamps })
    }

    /// Discard a pass's journals once the renderer has applied them.
    fn finish(&mut self, pass: Pass) {
        if let Some(changes) = pass.changes {
            changes.discard(&mut self.layers);
        }
        pass.stamps.discard(&mut self.layers);
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
    /// when it found nothing to do.
    #[cfg(test)]
    pub(crate) fn sync_for_test(&mut self, doc: &mut MindMapDocument) -> Option<SyncReport> {
        let pass = self.advance(doc)?;
        let projected = pass
            .changes
            .as_ref()
            .map_or(0, |changes| LayerRenderer::journals_of(changes).map(|(_, j)| j.touched().count()).sum());
        let stamped = pass
            .stamps
            .journals()
            .flat_map(|(layer, journal)| journal.touched().map(move |(id, _)| (layer, id)))
            .collect();
        self.finish(pass);
        Some(SyncReport { projected, stamped })
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
    /// over it selects (`Projector::nodes_in`).
    pub(crate) fn nodes_in(&self, rect: Rect) -> Vec<NodeId> {
        self.projector.nodes_in(rect)
    }

    /// Point the highlight at `covered` while a rubber band is out, or
    /// back at the selection with `None`. Takes effect at the next
    /// [`Self::sync`]. Returns whether it changed.
    pub(crate) fn set_preview(&mut self, covered: Option<Vec<NodeId>>) -> bool {
        if self.preview == covered {
            return false;
        }
        self.preview = covered;
        true
    }

    /// The rubber band's covered set, while one is out.
    pub(crate) fn preview(&self) -> Option<&[NodeId]> {
        self.preview.as_deref()
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
    /// What the overlay re-stamped.
    stamps: OverlayJournals,
}

/// What [`MapView::sync_for_test`] saw a pass write.
#[cfg(test)]
pub(crate) struct SyncReport {
    /// How many element touches the projector's journals named.
    pub(crate) projected: usize,
    /// The glyphs the overlay stamped or cleared, by layer.
    pub(crate) stamped: Vec<(baumhard::projection::Layer, NodeId)>,
}

/// An empty chrome layer: a `Void` root, drawn after the five glyph
/// layers.
fn chrome_layer() -> GfxTree {
    let mut tree = GfxTree::new_non_indexed();
    tree.layer = baumhard::projection::Layer::ALL.len();
    tree
}
