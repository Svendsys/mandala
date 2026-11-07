// SPDX-License-Identifier: MPL-2.0

//! Projection state (design §12.2): what the projector keeps per
//! projected document element, so an update recomputes only what
//! changed and compares against what it wrote, never against a layer.

use std::sync::Arc;

use glam::Vec2;

use crate::arena::NodeId;
use crate::document::resolve::{CarriedBinding, ResolvedText};
use crate::document::values::{Align, Shape, ZoomWindow};
use crate::flow::{Curve, Outline, PlacedRun, Rect, ShapedBox, ShapedRun};
use crate::gfx_structs::run::GlyphRunData;
use crate::projection::layers::Layer;

/// What the projector keeps for a visible node: the facts it carries
/// down to its subtree, and where it is drawn.
#[derive(Clone, Debug)]
pub(crate) struct NodeState {
    /// The node it hangs from, `None` for a root node.
    pub(crate) parent: Option<NodeId>,
    /// How many nodes are above it: the order phase A carries in.
    pub(crate) depth: usize,
    /// Whether its children are folded away, as last projected.
    pub(crate) folded: bool,
    /// Its group in each layer, by [`Layer::index`]; `None` until the
    /// update that made it visible writes its subtree.
    pub(crate) groups: [Option<NodeId>; 5],
    /// The units it owns: its sections, its frame's rings, its connector
    /// and its links.
    pub(crate) units: Vec<NodeId>,
    /// The palette binding carried to it.
    pub(crate) binding: Option<CarriedBinding>,
    /// Its own opacity times every ancestor's.
    pub(crate) opacity: f32,
    /// The zoom window its sections and rings render in.
    pub(crate) zoom: Option<ZoomWindow>,
    /// Its own fields its carry read and no input kind names — its own
    /// opacity and its zoom window — as last carried: a touch of the node
    /// carries it again only when they changed, so a drag carries
    /// nothing. `None` until it is first carried.
    pub(crate) carried_from: Option<(f32, Option<ZoomWindow>)>,
    /// Its drawn rectangle.
    pub(crate) rect: Rect,
    /// Its resolved shape.
    pub(crate) shape: Shape,
    /// The outline its rings run around.
    pub(crate) outline: Outline,
}

impl NodeState {
    /// A node not yet carried or measured, below `parent` at `depth`.
    pub(crate) fn new(parent: Option<NodeId>, depth: usize) -> Self {
        let rect = Rect::new(Vec2::ZERO, Vec2::ZERO);
        let shape = crate::document::defaults::BUILT_IN.node.shape;
        NodeState {
            parent,
            depth,
            folded: false,
            groups: [None; 5],
            units: Vec::new(),
            binding: None,
            opacity: 1.0,
            zoom: None,
            carried_from: None,
            rect,
            shape,
            outline: crate::flow::outline(rect, shape),
        }
    }
}

/// Who a glyph element of a layer belongs to: what the reverse map holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// A node's group.
    Group(NodeId),
    /// Slot `slot` of the unit `unit`.
    Run {
        /// The unit.
        unit: NodeId,
        /// Its slot.
        slot: usize,
    },
}

/// One glyph element a unit may draw.
#[derive(Clone, Debug)]
pub(crate) struct RunSlot {
    /// The layer it is drawn in.
    pub(crate) layer: Layer,
    /// The document element it draws: the section, ring, path, cap,
    /// label or portal end. Set when the unit resolves.
    pub(crate) element: Option<NodeId>,
    /// Its spans, in the order shaping numbered them: the span a glyph's
    /// span index names. Empty when it draws a `content` text.
    pub(crate) spans: Vec<NodeId>,
    /// Its element in the layer, once written.
    pub(crate) glyph: Option<NodeId>,
    /// What was last written there: what the layer holds, since the
    /// projector writes a run only through its slot.
    pub(crate) written: Option<GlyphRunData>,
    /// What the update wants there: `None` for no element.
    pub(crate) want: Option<GlyphRunData>,
}

impl RunSlot {
    /// An empty slot in `layer`.
    pub(crate) fn new(layer: Layer) -> Self {
        RunSlot {
            layer,
            element: None,
            spans: Vec::new(),
            glyph: None,
            written: None,
            want: None,
        }
    }
}

/// What a flow was shaped from: its spans' texts and resolved styles,
/// every size in points. Two equal keys shape the same run, so a key
/// that did not change reuses the shaping (#241 §3).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunKey {
    /// The spans, in flow order.
    pub(crate) spans: Vec<(String, ResolvedText)>,
}

/// What a box flow was shaped from: its spans, its minimum size and its
/// alignment — the box's width aligns its lines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BoxKey {
    /// The spans, in flow order.
    pub(crate) spans: Vec<(String, ResolvedText)>,
    /// The minimum size asked of the box.
    pub(crate) min: Option<Vec2>,
    /// The lines' alignment.
    pub(crate) align: Align,
}

/// A shaped path flow and the key it was shaped from.
#[derive(Clone, Debug)]
pub(crate) struct RunFlow {
    /// Shared with the projector's shaping cache (`projection::shapes`).
    pub(crate) key: Arc<RunKey>,
    pub(crate) shaped: Arc<ShapedRun>,
}

/// A shaped box flow and the key it was shaped from.
#[derive(Clone, Debug)]
pub(crate) struct BoxFlow {
    /// Shared with the projector's shaping cache (`projection::shapes`).
    pub(crate) key: Arc<BoxKey>,
    pub(crate) shaped: Arc<ShapedBox>,
}

/// A path flow as placed: the run it was given, the run placement
/// returned for it, and the placement — each kept, so an unchanged
/// placement reuses its `Arc`s and writes nothing.
#[derive(Clone, Debug)]
pub(crate) struct Placed {
    /// The run placement was given.
    pub(crate) input: Arc<ShapedRun>,
    /// The run the placement refers to: `input`, or the run placement
    /// returned (a `Fit` rescale, a filler or sides appended).
    pub(crate) run: Arc<ShapedRun>,
    /// Where each glyph sits.
    pub(crate) placed: Arc<PlacedRun>,
}

/// A label: its position along its host and its box.
#[derive(Clone, Debug, Default)]
pub(crate) struct LabelState {
    /// The label element.
    pub(crate) id: Option<NodeId>,
    /// Its resolved position along the path.
    pub(crate) t: f32,
    /// Its resolved offset from the path.
    pub(crate) offset: f32,
    /// Its shaping.
    pub(crate) flow: Option<BoxFlow>,
    /// Where its box centers, once placed.
    pub(crate) center: Option<Vec2>,
}

/// One flow of a path: a body, a cap or a portal marker.
#[derive(Clone, Debug, Default)]
pub(crate) struct PathFlow {
    /// The element it draws.
    pub(crate) id: Option<NodeId>,
    /// Its shaping.
    pub(crate) flow: Option<RunFlow>,
    /// Its placement.
    pub(crate) placed: Option<Placed>,
}

/// What a section keeps.
#[derive(Clone, Debug, Default)]
pub(crate) struct SectionState {
    /// Its shaping.
    pub(crate) flow: Option<BoxFlow>,
    /// Its box within its node: its offset and its shaped size.
    pub(crate) local: Option<Rect>,
}

/// What a ring keeps.
#[derive(Clone, Debug, Default)]
pub(crate) struct RingState {
    /// The frame it belongs to.
    pub(crate) frame: Option<NodeId>,
    /// Whether its frame is drawn.
    pub(crate) visible: bool,
    /// Its frame's and its own resolved layout and fit, as last placed.
    pub(crate) resolved: Option<RingResolved>,
    /// Its spans' texts and styles, sizes as resolved (relative).
    pub(crate) spans: Vec<(String, ResolvedText)>,
    /// Its own text style, size as resolved: what a filler and sides are
    /// shaped in.
    pub(crate) text: Option<ResolvedText>,
    /// The flow.
    pub(crate) flow: PathFlow,
    /// The sum of the line heights it and the rings inside it were placed
    /// at, and how many rings that is: what the ring just outside it
    /// clears (`flow::ring_offset` over this prefix). Each ring reads only
    /// the one just inside it, so a frame's rings place in O(rings).
    pub(crate) extent: Option<(f32, u32)>,
}

/// A ring's resolution, besides its spans.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RingResolved {
    /// The frame's fields.
    pub(crate) frame: crate::document::resolve::ResolvedFrame,
    /// The ring's fit, span, sides and spacing.
    pub(crate) ring: crate::document::resolve::ResolvedRing,
}

/// What a connector or a line link keeps.
#[derive(Clone, Debug, Default)]
pub(crate) struct LineState {
    /// The node it starts at and the node it ends at.
    pub(crate) ends: Option<(NodeId, NodeId)>,
    /// Whether it is drawn: its own `visible` and both ends visible.
    pub(crate) drawn: bool,
    /// Its fit, spacing and text style.
    pub(crate) resolved: Option<(crate::document::resolve::ResolvedPath, ResolvedText)>,
    /// Its body, its start cap and its end cap.
    pub(crate) flows: [PathFlow; 3],
    /// Its label.
    pub(crate) label: LabelState,
    /// Its curve, once placed.
    pub(crate) curve: Option<Curve>,
}

/// One end of a portal link.
#[derive(Clone, Debug, Default)]
pub(crate) struct EndState {
    /// Its marker.
    pub(crate) marker: PathFlow,
    /// Its label.
    pub(crate) label: LabelState,
    /// Where its marker centers, once placed.
    pub(crate) center: Option<Vec2>,
}

/// What a portal link keeps.
#[derive(Clone, Debug, Default)]
pub(crate) struct PortalState {
    /// The node it starts at and the node it leads to.
    pub(crate) ends: Option<(NodeId, NodeId)>,
    /// Whether it is drawn: both ends visible.
    pub(crate) drawn: bool,
    /// End 0, beside its own node, and end 1, beside the node it leads
    /// to.
    pub(crate) end: [EndState; 2],
}

/// What a unit is, and what it keeps.
#[derive(Clone, Debug)]
pub(crate) enum UnitBody {
    /// A section: one box run in the sections layer.
    Section(SectionState),
    /// A ring: one path run in the frames layer.
    Ring(RingState),
    /// A connector or a line link: body, start cap, end cap in the
    /// connections layer, its label in the labels layer.
    Line(Box<LineState>),
    /// A portal link: each end's marker and label in the portals layer.
    Portal(Box<PortalState>),
}

/// The layers of a line's slots, in slot order.
pub(crate) const LINE_SLOTS: [Layer; 4] = [
    Layer::Connections,
    Layer::Connections,
    Layer::Connections,
    Layer::Labels,
];

/// The layers of a portal's slots: end 0's marker and label, end 1's.
pub(crate) const PORTAL_SLOTS: [Layer; 4] = [Layer::Portals; 4];

/// A unit: a document element the projector projects into glyph
/// elements of its own — a section, a ring, a connector or a link.
#[derive(Clone, Debug)]
pub(crate) struct ProjectedUnit {
    /// The node whose group holds its glyph elements.
    pub(crate) node: NodeId,
    /// Its glyph elements.
    pub(crate) slots: Vec<RunSlot>,
    /// Slots of the kind it was before — a link switched between line and
    /// portal — whose elements the next write removes.
    pub(crate) retired: Vec<RunSlot>,
    /// What it keeps.
    pub(crate) body: UnitBody,
    /// The fields of its own element its resolution read and no input
    /// kind names, as last resolved: a touch of the element resolves it
    /// again only when they changed, and otherwise only places it.
    pub(crate) own: OwnFields,
}

/// The fields of a unit's own element that its resolution reads and that
/// no [`InputKind`](crate::document::deps::InputKind) names. A touch of
/// the element with these unchanged changed only what placement reads
/// — a section's offset, a path's anchors, control points and zoom
/// window — so it is placed and not resolved (B1's granularity note; the
/// review of #317). A label's and a portal end's own fields are all
/// named by an input kind, so neither has any here.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum OwnFields {
    /// Not resolved yet.
    #[default]
    Unread,
    /// A section's minimum size and alignment: its shaping key's.
    Section {
        /// Its `min_size`.
        min: Option<Vec2>,
        /// Its `align`.
        align: Option<crate::document::values::Align>,
    },
    /// A ring: nothing beyond what its input kinds name.
    Ring,
    /// A path's visibility, and a link's target and kind.
    Path {
        /// Its `visible`.
        visible: bool,
        /// A link's `to` and `kind`; `None` for a connector.
        link: Option<(NodeId, crate::document::values::LinkKind)>,
    },
}

impl OwnFields {
    /// The fields `element` holds now, as a unit of its kind reads them.
    pub(crate) fn of(element: &crate::document::element::DocElement) -> OwnFields {
        use crate::document::element::DocElement as E;
        match element {
            E::Section(s) => OwnFields::Section {
                min: s.min_size,
                align: s.align,
            },
            E::Ring(_) => OwnFields::Ring,
            E::Connector(c) => OwnFields::Path {
                visible: c.path.visible,
                link: None,
            },
            E::Link(l) => OwnFields::Path {
                visible: l.path.visible,
                link: Some((l.to, l.kind)),
            },
            _ => OwnFields::Unread,
        }
    }
}

/// The slots a unit of `body`'s kind has.
fn slots_for(body: &UnitBody) -> Vec<RunSlot> {
    match body {
        UnitBody::Section(_) => vec![RunSlot::new(Layer::Sections)],
        UnitBody::Ring(_) => vec![RunSlot::new(Layer::Frames)],
        UnitBody::Line(_) => LINE_SLOTS.iter().map(|&l| RunSlot::new(l)).collect(),
        UnitBody::Portal(_) => PORTAL_SLOTS.iter().map(|&l| RunSlot::new(l)).collect(),
    }
}

impl ProjectedUnit {
    /// A unit of `body` owned by `node`, with the slots its kind has.
    pub(crate) fn new(node: NodeId, body: UnitBody) -> Self {
        ProjectedUnit {
            node,
            slots: slots_for(&body),
            retired: Vec::new(),
            body,
            own: OwnFields::Unread,
        }
    }

    /// Become `body`, another kind: the old slots that hold an element are
    /// retired for the next write to remove, and the new kind's slots
    /// start empty.
    pub(crate) fn rebody(&mut self, body: UnitBody) {
        let old = std::mem::replace(&mut self.slots, slots_for(&body));
        self.retired
            .extend(old.into_iter().filter(|slot| slot.glyph.is_some()));
        self.body = body;
    }
}
