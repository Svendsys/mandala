// SPDX-License-Identifier: MPL-2.0

//! The glyph layers the projector writes, and what one update changed in
//! them (design §12.1, ruling B7.1 on #241).

use rustc_hash::FxHashMap;

use crate::arena::NodeId;
use crate::engine::journal::{Journal, JournalBuilder};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::tree::GfxTree;

/// The channel a node's group puts its child nodes' groups on. A group's
/// own glyph elements are on [`RUN_CHANNEL`], so they sort before the
/// child groups in every row.
pub const CHILD_GROUP_CHANNEL: usize = 1;

/// The channel a node's own glyph elements sit on under its group.
pub const RUN_CHANNEL: usize = 0;

/// One of the five glyph layers. There is no fill layer: the map's one
/// background is the color every frame is cleared to, and nothing else
/// is filled (D12, ruling B7.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// One box run per section.
    Sections,
    /// One path run per frame ring.
    Frames,
    /// Each connector's and line link's body and caps.
    Connections,
    /// Each connector's and line link's label.
    Labels,
    /// Each portal link's two markers and their labels.
    Portals,
}

impl Layer {
    /// Every layer, in draw order: the first is drawn first, under the
    /// others. [`LayerChanges::per_layer`] is indexed in this order.
    pub const ALL: [Layer; 5] = [
        Layer::Sections,
        Layer::Frames,
        Layer::Connections,
        Layer::Labels,
        Layer::Portals,
    ];

    /// The order a hit test asks the layers in: top-down, portals first
    /// (design §12.1, ruling B7.1). A point no layer's glyph holds is then
    /// tested against the nodes' bodies.
    pub const HIT_ORDER: [Layer; 5] = [
        Layer::Portals,
        Layer::Labels,
        Layer::Connections,
        Layer::Frames,
        Layer::Sections,
    ];

    /// The layer's index in [`Layer::ALL`]. O(1).
    pub const fn index(self) -> usize {
        match self {
            Layer::Sections => 0,
            Layer::Frames => 1,
            Layer::Connections => 2,
            Layer::Labels => 3,
            Layer::Portals => 4,
        }
    }
}

/// The five glyph layers. Each mirrors the **visible** document nodes:
/// every visible node has one `Void` group in every layer, groups nest as
/// the nodes do (child groups on [`CHILD_GROUP_CHANNEL`]), and a group's
/// own `GlyphRun`s are its children on [`RUN_CHANNEL`], in document
/// order. The layer roots are `Void`s and stand for the map.
pub struct GlyphLayers {
    /// One box run per section.
    pub sections: GfxTree,
    /// One path run per ring of a visible frame.
    pub frames: GfxTree,
    /// Each connector's body then its caps, then each line link's.
    pub connections: GfxTree,
    /// Each connector's label, then each line link's.
    pub labels: GfxTree,
    /// For each portal link: end 0's marker and label, end 1's marker and
    /// label.
    pub portals: GfxTree,
}

impl Default for GlyphLayers {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphLayers {
    /// Five empty layers: a `Void` root each, its draw order its index in
    /// [`Layer::ALL`]. O(1); one arena allocation per layer.
    pub fn new() -> Self {
        let layer = |l: Layer| {
            let mut tree = GfxTree::new_non_indexed();
            tree.layer = l.index();
            tree
        };
        GlyphLayers {
            sections: layer(Layer::Sections),
            frames: layer(Layer::Frames),
            connections: layer(Layer::Connections),
            labels: layer(Layer::Labels),
            portals: layer(Layer::Portals),
        }
    }

    /// The tree of `layer`. O(1).
    pub fn tree(&self, layer: Layer) -> &GfxTree {
        match layer {
            Layer::Sections => &self.sections,
            Layer::Frames => &self.frames,
            Layer::Connections => &self.connections,
            Layer::Labels => &self.labels,
            Layer::Portals => &self.portals,
        }
    }

    /// The tree of `layer`, to change or to hit-test (whose memo it
    /// fills). O(1).
    pub fn tree_mut(&mut self, layer: Layer) -> &mut GfxTree {
        match layer {
            Layer::Sections => &mut self.sections,
            Layer::Frames => &mut self.frames,
            Layer::Connections => &mut self.connections,
            Layer::Labels => &mut self.labels,
            Layer::Portals => &mut self.portals,
        }
    }
}

/// What one [`Projector::update`](crate::projection::Projector::update)
/// changed in each layer: the journal of every glyph mutation it applied
/// there — the runs it set, the groups and runs it inserted, removed and
/// moved — which names exactly what the renderer must refresh (design
/// §13.1).
///
/// **The journals must be discarded once consumed** (#241 amendment 4):
/// a journal owns the subtrees its removals detached, and only
/// [`Journal::discard`] frees them. The renderer's pipeline (V1-C3)
/// calls [`LayerChanges::discard`] after `LayerRenderer::apply` has read
/// them. A journal kept instead can revert or replay its layer, as any
/// journal can, as long as the layer has not changed since.
pub struct LayerChanges {
    /// One journal per layer, indexed by [`Layer::index`].
    pub per_layer: [Journal<GfxElement>; 5],
    /// The former owner of each glyph element the update removed, by
    /// layer and glyph: captured when the removal was written, since by
    /// the time a reader sees the update the projector has forgotten the
    /// glyph ([`Self::removed_owner`]).
    removed: FxHashMap<(Layer, NodeId), NodeId>,
}

impl LayerChanges {
    /// The journal of `layer`. O(1).
    pub fn journal(&self, layer: Layer) -> &Journal<GfxElement> {
        &self.per_layer[layer.index()]
    }

    /// The document element a glyph this update removed from `layer`
    /// was drawn for, as the projector knew it when it wrote the
    /// removal: a group's node; a section's or a ring's node; a
    /// connector's or a link's own element for its runs. `None` for a
    /// glyph the update did not remove. What
    /// [`Projector::element_of`](crate::projection::Projector::element_of)
    /// can no longer answer for a removed glyph — a deleted, hidden or
    /// folded-away node's included. O(1).
    pub fn removed_owner(&self, layer: Layer, glyph: NodeId) -> Option<NodeId> {
        self.removed.get(&(layer, glyph)).copied()
    }

    /// Whether the update changed nothing in any layer. O(1).
    pub fn is_empty(&self) -> bool {
        self.per_layer.iter().all(Journal::is_empty)
    }

    /// Free what the journals hold detached — the group subtrees and runs
    /// the update removed — once nothing will revert them. Cost: O(the
    /// elements they hold).
    pub fn discard(self, layers: &mut GlyphLayers) {
        let LayerChanges {
            per_layer,
            removed: _,
        } = self;
        let [sections, frames, connections, labels, portals] = per_layer;
        sections.discard(&mut layers.sections);
        frames.discard(&mut layers.frames);
        connections.discard(&mut layers.connections);
        labels.discard(&mut layers.labels);
        portals.discard(&mut layers.portals);
    }
}

/// One journal builder per layer, for one update, and the former owner
/// of each glyph it removed ([`LayerChanges::removed_owner`]).
pub(crate) struct LayerBuilders {
    journals: [JournalBuilder<GfxElement>; 5],
    removed: FxHashMap<(Layer, NodeId), NodeId>,
}

impl LayerBuilders {
    /// Five empty builders. O(1).
    pub(crate) fn new() -> Self {
        LayerBuilders {
            journals: std::array::from_fn(|_| JournalBuilder::new()),
            removed: FxHashMap::default(),
        }
    }

    /// The builder of `layer`. O(1).
    pub(crate) fn of(&mut self, layer: Layer) -> &mut JournalBuilder<GfxElement> {
        &mut self.journals[layer.index()]
    }

    /// Record that `glyph` of `layer`, which this update removes, was
    /// drawn for `owner`. O(1).
    pub(crate) fn removed(&mut self, layer: Layer, glyph: NodeId, owner: NodeId) {
        self.removed.insert((layer, glyph), owner);
    }

    /// Close every builder against its layer. O(what each recorded).
    pub(crate) fn close(self, layers: &GlyphLayers) -> LayerChanges {
        let LayerBuilders { journals, removed } = self;
        let [sections, frames, connections, labels, portals] = journals;
        LayerChanges {
            removed,
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
