// SPDX-License-Identifier: MPL-2.0

//! A projection of a document kept up to date as the app's view keeps
//! its own, with no renderer: what the field families' tests read the
//! projector's answers and each update's `LayerChanges` from.

use baumhard::arena::NodeId;
use baumhard::gfx_structs::run::RunShape;
use baumhard::projection::{GlyphLayers, Layer, Projector};

use crate::application::document::MindMapDocument;

/// A projection of a document, brought up to it by [`Projected::sync`].
pub(super) struct Projected {
    pub(super) projector: Projector,
    pub(super) layers: GlyphLayers,
}

/// What one update wrote: for each glyph element its journals touched,
/// in the order they touched them, its layer, the document element it is
/// drawn for, and the text of its run when it is a path run.
pub(super) type Written = Vec<(Layer, NodeId, Option<String>)>;

impl Projected {
    /// A full projection of `doc`, its pending changes taken, as a load
    /// leaves the view.
    pub(super) fn new(doc: &mut MindMapDocument) -> Self {
        baumhard::font::fonts::init();
        let mut layers = GlyphLayers::new();
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("field family tests");
        let projector = Projector::project_full(doc.doc(), &mut layers, &mut fonts);
        doc.take_changes();
        Projected { projector, layers }
    }

    /// Bring the projection up to `doc`, returning the document elements
    /// whose glyphs the update wrote, in any layer, each once, sorted.
    pub(super) fn sync(&mut self, doc: &mut MindMapDocument) -> Vec<NodeId> {
        let mut touched: Vec<NodeId> = self.update(doc).into_iter().map(|(_, element, _)| element).collect();
        touched.sort();
        touched.dedup();
        touched
    }

    /// Bring the projection up to `doc` by one update, returning what it
    /// wrote ([`Written`]), read from the update's own `LayerChanges`.
    pub(super) fn update(&mut self, doc: &mut MindMapDocument) -> Written {
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("field family tests");
        let records = doc.take_changes();
        let changes = self
            .projector
            .update(doc.doc(), &records, &mut self.layers, &mut fonts);
        let mut written = Vec::new();
        for layer in Layer::ALL {
            let tree = self.layers.tree(layer);
            for (glyph, _) in changes.journal(layer).touched() {
                let Some(element) = self.projector.element_of(layer, glyph) else {
                    continue;
                };
                let text = tree
                    .arena()
                    .get(glyph)
                    .and_then(|node| node.get().glyph_run())
                    .and_then(|run| match run.shaped() {
                        RunShape::Path { run, .. } => {
                            Some(run.buffer.lines.iter().map(|line| line.text()).collect::<String>())
                        }
                        RunShape::Box(_) => None,
                    });
                written.push((layer, element, text));
            }
        }
        changes.discard(&mut self.layers);
        written
    }
}
