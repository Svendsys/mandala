// SPDX-License-Identifier: MPL-2.0

//! What the projection's tests share: a document kept projected through
//! its change feed, and the oracle's check of it against a full
//! projection.

use glam::Vec2;

use crate::document::ids::Document;
use crate::document::writer::error::ApplyError;
use crate::document::writer::{ChangeSource, EditRequest};
use crate::font::fonts::FontSystemLease;
use crate::projection::equivalent::layers_equivalent;
use crate::projection::{GlyphLayers, Layer, LayerChanges, Projector};

/// How many nodes the oracle hits at: every node of a map up to this
/// many, an even spread of a larger one's.
const HIT_NODES: usize = 64;

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
        drop(fonts);
        let fonts = FontSystemLease::new("projection tests");
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
        let mut projected = Projected {
            doc,
            projector,
            layers,
            fonts,
        };
        projected.release();
        projected
    }

    /// Feed the document's change feed to the projector, and return what
    /// changed in each layer, its journals not yet discarded.
    pub fn sync(&mut self) -> LayerChanges {
        let changes = self.doc.take_changes();
        let out = self
            .projector
            .update(&self.doc, &changes, &mut self.layers, &mut self.fonts);
        self.release();
        out
    }

    /// Give back the font lock the lease took, as a caller does between
    /// frames: a sweep holding it from one call to the next would keep
    /// every other test thread waiting on it.
    pub fn release(&mut self) {
        self.fonts = FontSystemLease::new("projection tests");
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
    /// projector says of each node's drawn rectangle — and what its
    /// consumers read of it ([`Self::consumers_agree`]). `Err` names the
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
        self.release();
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
        self.consumers_agree(&full, &mut fresh)
    }

    /// What the projection's consumers read of the kept projector agrees
    /// with what `full`, projected into `fresh`, says (the review of #317):
    /// the reverse map at every pre-order position of every layer — the
    /// layers are already equal, so the positions pair up — the label
    /// index, the read API on every unit and label, and the hit test at
    /// three points of each of [`HIT_NODES`] nodes spread over the map:
    /// its center, just inside its corner and just past its far one.
    /// `Err` names the first difference. O(the layers, and a hit per
    /// point): the nodes are a sample because a hit scans the bodies of
    /// one cell, which a map of stacked nodes fills with all of them.
    fn consumers_agree(&mut self, full: &Projector, fresh: &mut GlyphLayers) -> Result<(), String> {
        for layer in Layer::ALL {
            let (kept, whole) = (self.layers.tree(layer), fresh.tree(layer));
            let pairs = kept
                .root()
                .descendants(kept.arena())
                .zip(whole.root().descendants(whole.arena()));
            for (i, (a, b)) in pairs.enumerate().skip(1) {
                let (x, y) = (self.projector.element_of(layer, a), full.element_of(layer, b));
                if x != y {
                    return Err(format!(
                        "{layer:?} glyph element {i} is kept as {x:?}'s, in full {y:?}'s"
                    ));
                }
            }
        }
        if self.projector.label_hosts != full.label_hosts {
            return Err(format!(
                "the label index is kept as {:?}, in full {:?}",
                self.projector.label_hosts, full.label_hosts
            ));
        }
        for &id in full.units.keys() {
            let (a, b) = (self.projector.curve(id), full.curve(id));
            if a != b {
                return Err(format!("the curve of {id:?} is kept as {a:?}, in full {b:?}"));
            }
            let a = self.projector.section_box(id).map(|(o, b)| (o, b.size));
            let b = full.section_box(id).map(|(o, b)| (o, b.size));
            if a != b {
                return Err(format!("the box of {id:?} is kept as {a:?}, in full {b:?}"));
            }
        }
        for &label in full.label_hosts.keys() {
            let (a, b) = (self.projector.label_pos(label), full.label_pos(label));
            if a != b {
                return Err(format!("{label:?} is kept at {a:?}, in full at {b:?}"));
            }
        }
        let mut ids: Vec<_> = full.nodes.keys().copied().collect();
        ids.sort_unstable();
        let every = ids.len().div_ceil(HIT_NODES).max(1);
        let points: Vec<Vec2> = ids
            .iter()
            .step_by(every)
            .map(|id| &full.nodes[id])
            .flat_map(|n| {
                [
                    n.rect.center(),
                    n.rect.origin + Vec2::splat(1.0),
                    n.rect.max() + Vec2::splat(6.0),
                ]
            })
            .collect();
        for point in points {
            let (a, b) = (
                self.projector.hit(&mut self.layers, point),
                full.hit(fresh, point),
            );
            if a != b {
                return Err(format!("{point} hits {a:?} kept, {b:?} in full"));
            }
        }
        Ok(())
    }
}
