// SPDX-License-Identifier: MPL-2.0

//! Projection: the document into glyph layers, **forward only, with work
//! proportional to what changed** (design §12, V1-B7 #241).
//!
//! The [`Projector`](crate::projection::Projector) turns a
//! [`Document`](crate::document::ids::Document) into five [`GlyphLayers`](crate::projection::GlyphLayers)
//! — sections, frames, connections, labels and portals; there is no fill
//! layer (D12, ruling B7.1) — each mirroring the visible nodes as nested
//! `Void` groups that hold the nodes' `GlyphRun`s. It projects everything
//! once at load ([`Projector::project_full`](crate::projection::Projector::project_full)), then consumes the
//! document's change feed ([`Projector::update`](crate::projection::Projector::update)) and re-projects exactly
//! what the changes touched and whatever **read** something they touched,
//! updating the layers in place through journaled glyph mutations.
//!
//! # What is projected, and what is kept
//!
//! A **unit** is a document element that draws glyph elements of its
//! own: a section (one box run), a frame's ring (one path run), a
//! connector or a line link (its body and caps in the connections layer,
//! its label in the labels layer), a portal link (each end's marker and
//! label in the portals layer). A node draws through its units and holds
//! the facts they read: its palette binding, carried down the walk from
//! its parent's (ruling R16 on #282), its effective opacity, its zoom
//! window, its drawn rectangle and outline. Every resolved value, shaped
//! run and placement is kept as projection state; a flow is shaped again
//! only when the text and styles it was shaped from — its shape key —
//! changed, and a placement equal to the last one reuses its `Arc`s, so
//! an unchanged run is written nothing.
//!
//! # Captured dependencies
//!
//! Every read of another element's state goes through a recorder: the
//! resolvers report what they read ([`crate::document::resolve`]), and
//! the projector reports what it read of its own derived state —
//! another node's geometry, visibility or opacity, an inner ring's line
//! height ([`ProjectionInput`](crate::projection::ProjectionInput)). Those records are the
//! dependency index; nothing is listed by hand. A record says which
//! stage of which unit read the input
//! ([`Stage`](crate::projection::Stage)), so a change that touches only
//! geometry — a drag — reaches only placement: nothing is resolved or
//! shaped again.
//!
//! # An update
//!
//! 1. **Structure.** Nodes that left, or stopped being visible
//!    (`hidden`, a `folded` ancestor), lose their groups in every layer;
//!    nodes that became visible are projected, subtree and all; a node
//!    whose parent changed has its groups moved. Units are created and
//!    dropped as their elements come and go.
//! 2. **Seeds.** The record's input kinds and rearranged rows (ruling
//!    B7.2) mark their readers; each touched element's own projection is
//!    marked.
//! 3. **Phase A:** nodes carry their inherited facts down, top-down;
//!    sections resolve and shape; nodes measure. **Phase B:** sections
//!    and rings place. **Phase C:** connectors and links resolve, and
//!    each connector and line link finds where it leaves its two nodes —
//!    every one of them before any is laid, since the paths that leave
//!    one side of a node are fanned together (#296); then connectors,
//!    links, labels and portal ends are laid. A piece of work runs at
//!    most once per phase.
//! 4. **Emit.** Every run whose new data differs from what the projector
//!    last wrote — its own state, never the live element, which overlays
//!    may have written (#241 amendment 2) — is set, inserted, removed or
//!    moved, journaled per layer: the [`LayerChanges`](crate::projection::LayerChanges).
//!
//! The camera is not an input: panning and zooming never call the
//! projector, and zoom windows are evaluated by the renderer (§12.4). No
//! string is parsed and no layer is diffed or hashed whole.
//!
//! `format/projection.md` spells where each element is drawn.

/// The geometry rules: drawn rectangles, anchors, curves, labels, portal
/// ends.
pub(crate) mod geometry;
/// The node bodies' hit index and the hit test.
mod hit;
/// The dependency index and what it is keyed by.
mod input;
/// The five layers, and one update's journals of them.
mod layers;
/// The stages of a node's and a unit's projection.
mod stages;
/// What the projector keeps per projected element.
mod state;
/// The structural step: what is visible, and what is a unit.
mod structure;
/// Writing the layers.
mod write;

/// [`layers_equivalent`](equivalent::layers_equivalent): the oracle's
/// comparison of two sets of layers.
pub mod equivalent;

/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
/// `#[allow(clippy::unwrap_used)]` because this is a tests tree (§T2.2).
#[allow(clippy::unwrap_used)]
pub mod tests;

pub use hit::Hit;
pub use input::{Dependent, ProjectionInput, Stage};
pub use layers::{GlyphLayers, Layer, LayerChanges, CHILD_GROUP_CHANNEL, RUN_CHANNEL};

use std::sync::Arc;

use glam::Vec2;
use rustc_hash::FxHashMap;

use crate::arena::NodeId;
use crate::document::ids::Document;
use crate::document::writer::feed::ChangeRecord;
use crate::flow::{Curve, Rect, ShapedBox};
use crate::font::fonts::FontSystemLease;
use crate::projection::hit::BodyIndex;
use crate::projection::input::DepIndex;
use crate::projection::layers::LayerBuilders;
use crate::projection::stages::PhaseWork;
use crate::projection::state::{NodeState, Owner, ProjectedUnit, UnitBody};

/// The projector: the projection state of one document and its glyph
/// layers. See the [module docs](crate::projection).
#[derive(Default)]
pub struct Projector {
    /// Every visible node's state.
    pub(crate) nodes: FxHashMap<NodeId, NodeState>,
    /// Every unit of a visible node.
    pub(crate) units: FxHashMap<NodeId, ProjectedUnit>,
    /// The reverse map: each glyph element of each layer, and whose it is.
    pub(crate) owners: FxHashMap<(Layer, NodeId), Owner>,
    /// The dependency index.
    pub(crate) deps: DepIndex,
    /// The visible nodes' bodies, for the hit test.
    pub(crate) bodies: BodyIndex,
    /// Each label a unit projects, and that unit: what
    /// [`Projector::label_pos`] reads.
    pub(crate) label_hosts: FxHashMap<NodeId, NodeId>,
    /// Each node, and the drawn connectors and line links with an end at
    /// it (#296): the paths whose ends one side of it fans.
    pub(crate) junctions: FxHashMap<NodeId, Vec<NodeId>>,
    /// Every piece of work the last call ran, in order: what the tests of
    /// the dependency index read. Test builds only: the product keeps no
    /// log of its work.
    #[cfg(test)]
    pub(crate) ran: Vec<Dependent>,
    /// Each ring whose shaping key the last call built: a ring shaped
    /// again with its key unchanged builds none. Test builds only.
    #[cfg(test)]
    pub(crate) key_builds: Vec<NodeId>,
}

impl Projector {
    /// Project all of `doc` into `layers`, which are replaced by fresh
    /// ones: what loading does, and what the oracle compares updates
    /// against. Nothing else projects everything.
    ///
    /// Cost: one walk of the visible document, every flow shaped and
    /// placed once; the layers are written in pre-order, each element
    /// appended where it goes. The font lock is taken through `fonts`
    /// when the first flow is shaped.
    pub fn project_full(doc: &Document, layers: &mut GlyphLayers, fonts: &mut FontSystemLease) -> Projector {
        *layers = GlyphLayers::new();
        let mut projector = Projector::default();
        let mut work = PhaseWork::default();
        projector.project_roots(doc.tree(), &mut work);
        projector.run_phases(doc, &mut work, fonts);
        projector.write_full(doc.tree(), layers);
        #[cfg(test)]
        {
            projector.ran = work.ran;
        }
        projector
    }

    /// Bring the projection up to date with `doc` after `changes`, the
    /// records the document's feed gave since the last call
    /// ([`Document::take_changes`]), and return what changed in each
    /// layer. See the [module docs](crate::projection) for the steps.
    ///
    /// `fonts` takes the font lock only when something must be shaped or
    /// placed again: an update that only re-places box flows, or changes
    /// nothing drawn, never takes it.
    ///
    /// Cost: proportional to the changes and to what read what they
    /// touched — never a walk of the document or of a layer.
    pub fn update(
        &mut self,
        doc: &Document,
        changes: &[ChangeRecord],
        layers: &mut GlyphLayers,
        fonts: &mut FontSystemLease,
    ) -> LayerChanges {
        let mut work = PhaseWork::default();
        let mut out = LayerBuilders::new();
        let tree = doc.tree();
        #[cfg(test)]
        self.key_builds.clear();
        let touched = structure::TouchedElements::of(changes);
        self.restructure(tree, &touched, &mut work, layers, &mut out);
        self.seed(tree, changes, &touched, &mut work);
        self.run_phases(doc, &mut work, fonts);
        self.write_update(tree, &mut work, layers, &mut out);
        #[cfg(test)]
        {
            self.ran = work.ran;
        }
        out.close(layers)
    }

    /// The glyph elements `element` produced, each with its layer: a
    /// node's groups, a unit's runs. Empty for an element the projection
    /// has nothing of. O(the unit's slots).
    pub fn glyphs_of(&self, element: NodeId) -> Vec<(Layer, NodeId)> {
        if let Some(node) = self.nodes.get(&element) {
            return Layer::ALL
                .iter()
                .filter_map(|&l| node.groups[l.index()].map(|g| (l, g)))
                .collect();
        }
        self.units.get(&element).map_or_else(Vec::new, |unit| {
            unit.slots
                .iter()
                .filter_map(|slot| slot.glyph.map(|g| (slot.layer, g)))
                .collect()
        })
    }

    /// The document element a glyph element of `layer` was projected
    /// from: a node for a group; for a run, the section, ring, path, cap,
    /// label or portal end it draws. O(1).
    pub fn element_of(&self, layer: Layer, glyph: NodeId) -> Option<NodeId> {
        match *self.owners.get(&(layer, glyph))? {
            Owner::Group(node) => Some(node),
            Owner::Run { unit, slot } => self.units.get(&unit)?.slots.get(slot)?.element,
        }
    }

    /// A visible node's drawn rectangle (D5): served from projection
    /// state, recomputing nothing. O(1).
    pub fn node_rect(&self, node: NodeId) -> Option<Rect> {
        self.nodes.get(&node).map(|n| n.rect)
    }

    /// A section's box: its canvas top-left and its shaping, which its
    /// caret geometry is read from (V1-C8). O(1).
    pub fn section_box(&self, section: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)> {
        let unit = self.units.get(&section)?;
        let UnitBody::Section(state) = &unit.body else {
            return None;
        };
        let origin = self.nodes.get(&unit.node)?.rect.origin + state.local?.origin;
        Some((origin, &state.flow.as_ref()?.shaped))
    }

    /// A drawn connector's or line link's curves, each starting where the
    /// one before it ends: `None` until it is placed. O(1).
    pub fn curve(&self, path: NodeId) -> Option<&[Curve]> {
        match &self.units.get(&path)?.body {
            UnitBody::Line(line) if line.drawn && !line.path.is_empty() => Some(&line.path),
            _ => None,
        }
    }

    /// Where a drawn label's box centers: on its path, or beside its
    /// portal end. O(1).
    pub fn label_pos(&self, label: NodeId) -> Option<Vec2> {
        let host = *self.label_hosts.get(&label)?;
        match &self.units.get(&host)?.body {
            UnitBody::Line(line) if line.drawn && line.label.id == Some(label) => line.label.center,
            UnitBody::Portal(portal) if portal.drawn => portal
                .end
                .iter()
                .find(|end| end.label.id == Some(label))
                .and_then(|end| end.label.center),
            _ => None,
        }
    }

    /// How many (input, work) pairs the dependency index holds: what the
    /// test that holds it linear in a deep chain reads. Test builds only.
    #[cfg(test)]
    pub(crate) fn dependency_count(&self) -> usize {
        self.deps.len()
    }
}

/// Read-only geometry from projection state, for the consumers that act
/// on where things are drawn (V1-C7, V1-C8, V1-D1's edit handlers). Sizes
/// are derived (D5), so they come from here; nothing reads a glyph layer
/// to learn them.
pub trait Geometry {
    /// A visible node's drawn rectangle.
    fn node_rect(&self, node: NodeId) -> Option<Rect>;
    /// A section's canvas top-left and shaping.
    fn section_box(&self, section: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)>;
    /// A drawn connector's or line link's curves.
    fn curve(&self, path: NodeId) -> Option<&[Curve]>;
    /// Where a drawn label centers.
    fn label_pos(&self, label: NodeId) -> Option<Vec2>;
}

impl Geometry for Projector {
    fn node_rect(&self, node: NodeId) -> Option<Rect> {
        Projector::node_rect(self, node)
    }
    fn section_box(&self, section: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)> {
        Projector::section_box(self, section)
    }
    fn curve(&self, path: NodeId) -> Option<&[Curve]> {
        Projector::curve(self, path)
    }
    fn label_pos(&self, label: NodeId) -> Option<Vec2> {
        Projector::label_pos(self, label)
    }
}
