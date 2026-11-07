// SPDX-License-Identifier: MPL-2.0

//! **The chrome builders' hook** (design §12.1; #244's chrome
//! amendment): chrome that follows the document — V1-C7's resize
//! handles, which move "when `LayerChanges` touches a selected
//! element", and V1-C8's caret — registers a [`ChromeBuilder`] on the
//! view ([`super::MapView::load_with_chrome`]) instead of editing the
//! sync pass. Each sync pass that changed the layers hands its
//! [`LayerChanges`] to every builder ([`PassChanges`]) before the pass
//! discards them; what the builders return is applied to the chrome
//! layer under one journal, which the renderer applies in the same pass
//! and the pass then discards, as it does the projector's. A pass that
//! changed no layer — a pan, a zoom, a selection alone — reaches no
//! builder.
//!
//! The rubber band (`view::band`) is chrome too, but it follows a
//! gesture, not the document, so it writes through
//! `MapView::apply_chrome` when the gesture moves.

use baumhard::arena::NodeId;
use baumhard::document::ids::Document;
use baumhard::engine::anchor::Anchors;
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::{Layer, LayerChanges, Projector};

use super::overlay::owner_of;

/// Chrome that follows the document: planted once in the chrome layer,
/// then rewritten only when a sync pass's changes call for it.
pub(crate) trait ChromeBuilder {
    /// Put this builder's elements in `chrome`, under its root: once,
    /// when the view is projected, before the renderer loads it.
    fn plant(&mut self, chrome: &mut GfxTree);

    /// The chrome mutators `pass` calls for, each with the chrome
    /// elements it is anchored at; nothing when the pass touches nothing
    /// this builder draws from. Called once per sync pass that changed
    /// the layers, and never otherwise.
    fn follow(&mut self, pass: &PassChanges<'_>) -> Vec<(GfxMutatorTree, Anchors)>;
}

/// What a sync pass hands its chrome builders: the projector's changes
/// and what to read them against, before the pass discards them.
pub(crate) struct PassChanges<'a> {
    /// What the projector wrote in this pass, per layer.
    pub(crate) changes: &'a LayerChanges,
    /// The document as the pass left it.
    pub(crate) doc: &'a Document,
    /// The projection, to read geometry and owners from.
    pub(crate) projector: &'a Projector,
}

impl PassChanges<'_> {
    /// Whether the pass wrote a run `element` owns — a node's, through
    /// its sections, frames and labels, or a connector's or a link's —
    /// in any layer: the question a handle or a caret asks of its
    /// target. An element the run's owner merely descends from is not
    /// touched by it.
    ///
    /// Cost: O(the runs the pass wrote × the depth to each one's owner,
    /// at most three); no walk of a layer or of the document.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "V1-C7's handles and V1-C8's caret are its first readers (#248, #249)"
        )
    )]
    pub(crate) fn touches(&self, element: NodeId) -> bool {
        Layer::ALL.into_iter().any(|layer| {
            self.changes
                .journal(layer)
                .touched()
                .any(|(glyph, _)| owner_of(self.doc, self.projector, layer, glyph) == Some(element))
        })
    }
}
