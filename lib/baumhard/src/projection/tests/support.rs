// SPDX-License-Identifier: MPL-2.0

//! What the projection's tests share: a document kept projected through
//! its change feed, and the oracle's check of it against a full
//! projection.

use crate::document::ids::Document;
use crate::document::writer::error::ApplyError;
use crate::document::writer::{ChangeSource, EditRequest};
use crate::font::fonts::FontSystemLease;
use crate::projection::equivalent::layers_equivalent;
use crate::projection::{GlyphLayers, LayerChanges, Projector};

/// A lease on the font system, with the fonts initialized first (the
/// precondition of every shaping call).
pub fn fonts() -> FontSystemLease {
    crate::font::fonts::init();
    FontSystemLease::new("projection tests")
}

/// A document, its projector and its layers, kept in step by feeding
/// every change the document records to [`Projector::update`].
pub struct Projected {
    /// The document.
    pub doc: Document,
    /// Its projector.
    pub projector: Projector,
    /// Its layers.
    pub layers: GlyphLayers,
    /// The font lease every call shapes under.
    pub fonts: FontSystemLease,
}

impl Projected {
    /// Project `doc` in full. Anything already in its feed is drained
    /// first: the full projection is of the document as it stands.
    pub fn new(mut doc: Document) -> Self {
        doc.take_changes();
        let mut fonts = fonts();
        let mut layers = GlyphLayers::new();
        let projector = Projector::project_full(&doc, &mut layers, &mut fonts);
        Projected {
            doc,
            projector,
            layers,
            fonts,
        }
    }

    /// [`Self::new`], the full projection sharing shapings of equal keys
    /// with every full projection the thread's oracle checks make
    /// (`stages::memo`): for a sweep, whose comparisons are by value and
    /// which never asks whether two runs share their shaping.
    #[cfg(test)]
    pub fn new_shared(mut doc: Document) -> Self {
        doc.take_changes();
        let mut fonts = fonts();
        let mut layers = GlyphLayers::new();
        let projector = crate::projection::stages::memo::sharing(|| {
            Projector::project_full(&doc, &mut layers, &mut fonts)
        });
        Projected {
            doc,
            projector,
            layers,
            fonts,
        }
    }

    /// Feed the document's change feed to the projector, and return what
    /// changed in each layer, its journals not yet discarded.
    pub fn sync(&mut self) -> LayerChanges {
        let changes = self.doc.take_changes();
        self.projector
            .update(&self.doc, &changes, &mut self.layers, &mut self.fonts)
    }

    /// [`Self::sync`], the journals discarded as the renderer would once
    /// it had read them.
    pub fn sync_and_discard(&mut self) {
        let changes = self.sync();
        changes.discard(&mut self.layers);
    }

    /// Apply `req` as an action, then sync and discard.
    pub fn apply(&mut self, req: EditRequest) -> Result<(), ApplyError> {
        self.doc.apply(req, ChangeSource::Action)?;
        self.sync_and_discard();
        Ok(())
    }

    /// The oracle (#241): the layers as kept up to date equal a full
    /// projection of the document as it now stands, and so does what the
    /// projector says of each node's drawn rectangle. `Err` names the
    /// first difference.
    pub fn check(&mut self) -> Result<(), String> {
        let mut fresh = GlyphLayers::new();
        // The full projection may share the shapings of equal keys
        // (`stages::memo`): it is compared by value.
        #[cfg(test)]
        let full = crate::projection::stages::memo::sharing(|| {
            Projector::project_full(&self.doc, &mut fresh, &mut self.fonts)
        });
        #[cfg(not(test))]
        let full = Projector::project_full(&self.doc, &mut fresh, &mut self.fonts);
        layers_equivalent(&self.layers, &fresh).map_err(|d| d.to_string())?;
        if full.nodes.len() != self.projector.nodes.len() {
            return Err(format!(
                "{} nodes projected in full, {} kept",
                full.nodes.len(),
                self.projector.nodes.len()
            ));
        }
        for (id, state) in &full.nodes {
            let Some(kept) = self.projector.nodes.get(id) else {
                return Err(format!("{id:?} projected in full, not kept"));
            };
            if kept.rect != state.rect || kept.binding != state.binding || kept.opacity != state.opacity {
                return Err(format!("{id:?}: kept {kept:?}, in full {state:?}"));
            }
        }
        if full.units.len() != self.projector.units.len() {
            return Err(format!(
                "{} units projected in full, {} kept",
                full.units.len(),
                self.projector.units.len()
            ));
        }
        Ok(())
    }
}
