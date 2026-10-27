// SPDX-License-Identifier: MPL-2.0

//! The document's elements: one
//! [`DocKind`](crate::document::element::DocKind) per kind of thing a
//! map is made of, one data struct per kind, and
//! [`DocElement`](crate::document::element::DocElement), the enum that
//! carries one of them and is what a `Tree<DocElement>` holds.
//!
//! **Structure is nesting** (D3). A node's sections, frame, connector,
//! links and child nodes are its children in the tree; there is no
//! parent pointer, no id that encodes a path, and no list of edges.
//! Which kinds may sit under which is
//! [`validate_shape`](crate::document::shape::validate_shape)'s table.
//!
//! **Style is per field, and absent means inherit** (design §5.4).
//! Every style field is an `Option`; `None` is resolved through the
//! owner chain, the palette, the map's `Defaults` and
//! [`BUILT_IN`](crate::document::defaults::BUILT_IN) by
//! [the resolvers](crate::document::resolve). Editing one field never copies
//! a default into an element.
//!
//! **Text lives in spans** (D2), never in a node, so a node stays small
//! enough that cloning it — which a journal does once per touched
//! element — is cheap.

use crate::document::values::{
    Align, Anchor, ColorRef, Direction, FitMode, FontSize, LinkKind, LinkTarget, PaletteId, PlainColor, Rgba,
    Shape, StartPoint, VerbId, Weight, ZoomWindow,
};
use crate::engine::element::TreeElement;
use crate::font::fonts::AppFont;
use crate::util::grapheme_chad::split_off_graphemes;
use glam::Vec2;
use indextree::NodeId;
use serde::{Deserialize, Serialize};

/// What kind of element a [`DocElement`] is: the first half of its
/// routing key.
///
/// **The declaration order is the sibling order.** A parent keeps its
/// children sorted by key, kind first (`engine::key`), so the derived
/// `Ord` decides how a parent's children of different kinds are laid
/// out, and the order below is chosen for that: under a node, its
/// sections come first, then its frame, connector, links and child
/// nodes — the order a reader meets them in — and under a path, its
/// spans come before its caps and label. A node is last so that the
/// nodes of a subtree are the tail of every node row, after everything
/// the node itself draws. Changing the order reorders every row of
/// every document.
///
/// `Default` is [`Map`](Self::Map), the root's kind: the kind of
/// `MutatorTree::new`'s `Void` root, which applies only at an element
/// of its own kind (ruling R1 on #232), so a mutator tree applied from
/// the document's root starts at the map.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// The root, exactly one per document.
    #[default]
    Map,
    /// The theme: variables and their values per variant.
    Theme,
    /// A named list of color groups, one per depth.
    Palette,
    /// The map's per-field defaults.
    Defaults,
    /// A box of text inside a node.
    Section,
    /// A run of text with one style.
    Span,
    /// A node's outline text.
    Frame,
    /// One ring of a frame: one line of text around the outline.
    Ring,
    /// The path from a node to its parent.
    Connector,
    /// The glyphs at a path's start.
    CapStart,
    /// The glyphs at a path's end.
    CapEnd,
    /// Text placed along a path, or beside a portal end.
    Label,
    /// A path or a portal from a node to another node.
    Link,
    /// One end of a portal link.
    PortalEnd,
    /// A node.
    Node,
}

impl DocKind {
    /// Every kind, in declaration (and sibling) order.
    pub const ALL: [DocKind; 15] = [
        DocKind::Map,
        DocKind::Theme,
        DocKind::Palette,
        DocKind::Defaults,
        DocKind::Section,
        DocKind::Span,
        DocKind::Frame,
        DocKind::Ring,
        DocKind::Connector,
        DocKind::CapStart,
        DocKind::CapEnd,
        DocKind::Label,
        DocKind::Link,
        DocKind::PortalEnd,
        DocKind::Node,
    ];
}

/// Text style: what a span, or anything that holds spans, may say
/// about its text. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextStyle {
    /// The face.
    pub font: Option<AppFont>,
    /// The size. `Rel` only inside a frame.
    pub size: Option<FontSize>,
    /// The color.
    pub color: Option<ColorRef>,
    /// The weight.
    pub weight: Option<Weight>,
    /// Whether the text is italic.
    pub italic: Option<bool>,
}

/// The style of a path flow — a connector or a line link — and its
/// default content. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowStyle {
    /// What the flow does when its text and its path differ in length.
    pub fit: Option<FitMode>,
    /// Extra space between glyphs, in canvas units.
    pub spacing: Option<f32>,
    /// The text size in points.
    pub size: Option<f32>,
    /// The text color.
    pub color: Option<ColorRef>,
    /// The text drawn along the path when it has no span children; a
    /// path with spans ignores it.
    pub content: Option<String>,
}

/// The map's own data: its name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapData {
    /// The map's name.
    pub name: String,
}

/// The theme: named variables, and one value per variable in each
/// variant (D21). The live values are the active variant's; they are
/// never copied anywhere else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeData {
    /// The variable names. A [`VarId`](crate::document::values::VarId)
    /// indexes this list.
    pub variables: Vec<String>,
    /// The variants.
    pub variants: Vec<ThemeVariant>,
    /// The index of the active variant in [`variants`](Self::variants).
    pub active: u16,
}

/// One variant of a theme.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeVariant {
    /// The variant's name.
    pub name: String,
    /// One value per theme variable, in the same order:
    /// `values.len() == variables.len()`.
    pub values: Vec<Rgba>,
}

/// A palette: color groups by depth below the node bound to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteData {
    /// The palette's name.
    pub name: String,
    /// The groups: `groups[0]` colors the bound node, `groups[1]` its
    /// children, and so on; depths past the end take the last group.
    pub groups: Vec<PaletteGroup>,
}

/// The four colors of one palette depth.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PaletteGroup {
    /// A node's fill.
    pub fill: PlainColor,
    /// A node's frame text.
    pub frame: PlainColor,
    /// Section text.
    pub text: PlainColor,
    /// The text of a node's first section, when it differs from the
    /// other sections': `None` gives the first section `text`, in the
    /// same tier (ruling R5 on #235).
    pub title: Option<PlainColor>,
}

/// The map's defaults: one struct per kind with style, consulted after
/// the element and its owners and before
/// [`BUILT_IN`](crate::document::defaults::BUILT_IN).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefaultsData {
    /// Node defaults.
    pub node: NodeStyle,
    /// Section text defaults.
    pub section: TextStyle,
    /// Frame defaults.
    pub frame: FrameStyle,
    /// Ring defaults.
    pub ring: RingStyle,
    /// Connector defaults.
    pub connector: FlowStyle,
    /// Line-link defaults.
    pub link: FlowStyle,
    /// Label defaults.
    pub label: LabelStyle,
    /// Portal-end defaults.
    pub portal_end: PortalEndStyle,
    /// Span defaults.
    pub span: TextStyle,
}

/// A node's style. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeStyle {
    /// The outline its fill and frame share.
    pub shape: Option<Shape>,
    /// The background fill.
    pub fill: Option<ColorRef>,
    /// The width of the fill's soft edge, in canvas units (D12).
    pub fill_fade: Option<f32>,
    /// Text defaults for everything the node holds.
    pub text: TextStyle,
}

/// What a node does when it is clicked or hovered: a verb each.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Triggers {
    /// The verb a click runs.
    pub click: Option<VerbId>,
    /// The verb hovering runs.
    pub hover: Option<VerbId>,
}

/// A node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeData {
    /// The node's external identity: unique in the document, never
    /// reused while it is open ([`crate::document::ids`]).
    pub ident: crate::document::ids::NodeIdent,
    /// The node's channel among its sibling nodes.
    pub channel: usize,
    /// The top-left corner, in canvas units.
    pub position: Vec2,
    /// The size the user set, a minimum: the node grows to fit its
    /// content (D5).
    pub min_size: Option<Vec2>,
    /// The node's style.
    pub style: NodeStyle,
    /// The palette that colors this node and its descendants by depth.
    pub palette: Option<PaletteId>,
    /// The zoom range the node renders in.
    pub zoom: Option<ZoomWindow>,
    /// Whether its descendants are collapsed.
    pub folded: bool,
    /// Whether the node, its subtree, its connector and the links
    /// touching the subtree are left out.
    pub hidden: bool,
    /// Multiplies the alpha of everything the node and its subtree
    /// draw, `0.0..=1.0`.
    pub opacity: f32,
    /// The verbs its click and hover run.
    pub on: Triggers,
}

/// A section: a box of spans inside its node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionData {
    /// The section's channel among the node's sections.
    pub channel: usize,
    /// The top-left corner, relative to the node's.
    pub offset: Vec2,
    /// The size the user set, a minimum (D5).
    pub min_size: Option<Vec2>,
    /// How the lines align in the box.
    pub align: Option<Align>,
    /// Text defaults for its spans.
    pub text: TextStyle,
}

/// A span: text with one style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanData {
    /// The text.
    pub text: String,
    /// Its style.
    pub style: TextStyle,
    /// Where it leads when clicked.
    pub link: Option<LinkTarget>,
}

/// A frame's style. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameStyle {
    /// Whether the frame is drawn.
    pub visible: Option<bool>,
    /// Where on the outline its text starts.
    pub start: Option<StartPoint>,
    /// Which way its text runs.
    pub direction: Option<Direction>,
    /// What its rings do when their text and the outline differ in
    /// length.
    pub fit: Option<FitMode>,
    /// The space between rings, in canvas units.
    pub gap: Option<f32>,
    /// The distance of the first ring from the outline, in canvas units.
    pub inset: Option<f32>,
    /// Text defaults for its rings.
    pub text: TextStyle,
}

/// A frame: the node's outline text, in rings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameData {
    /// Its style.
    pub style: FrameStyle,
}

/// A ring's style. Every field inherits when `None`. The ring's size is
/// its `text.size`, always [`Rel`](crate::document::values::FontSize::Rel).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RingStyle {
    /// Overrides the frame's fit mode for this ring.
    pub fit: Option<FitMode>,
    /// Text defaults for its spans.
    pub text: TextStyle,
}

/// A ring: one line of text around the outline, innermost first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RingData {
    /// The ring's channel among the frame's rings.
    pub channel: usize,
    /// Its style.
    pub style: RingStyle,
}

/// What a connector and a link share: the path between two nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct PathData {
    /// Its style and default content.
    pub flow: FlowStyle,
    /// Where it attaches at its start and at its end.
    pub anchors: (Anchor, Anchor),
    /// Bézier control points, offsets in canvas units: 0, 1 or 2
    /// ([`MAX_CONTROL_POINTS`](crate::document::bounds::MAX_CONTROL_POINTS)).
    pub control_points: Vec<Vec2>,
    /// Whether it is drawn.
    pub visible: bool,
    /// The zoom range it renders in.
    pub zoom: Option<ZoomWindow>,
}

/// The line from a node to its parent. A connector is always a line
/// (D22); a portal between parent and child is a hidden connector plus
/// a portal link.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectorData {
    /// The path.
    pub path: PathData,
}

/// A path or a portal from its node to another node.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkData {
    /// The node it leads to, by arena id (D3).
    pub to: NodeId,
    /// Whether it draws a path or two portal ends.
    pub kind: LinkKind,
    /// The path of a line link. A portal link draws no path, but keeps
    /// the fields within a session so that a link can switch kind
    /// without losing them. **They are never saved for a portal link**:
    /// the file's path keys belong to line links, so a portal link
    /// reloads with the default path. That is the one thing a save and a
    /// load do not carry over; everything else is the same document.
    pub path: PathData,
}

/// A path's start or end cap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapData {
    /// Text defaults for its spans.
    pub text: TextStyle,
}

/// A label's style. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelStyle {
    /// The position along the path, `0.0..=1.0`.
    pub t: Option<f32>,
    /// The distance from the path along its normal, in canvas units.
    pub offset: Option<f32>,
    /// Text defaults for its spans.
    pub text: TextStyle,
}

/// Text along a path, or beside a portal end.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelData {
    /// Its style.
    pub style: LabelStyle,
}

/// A portal end's style. Every field inherits when `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortalEndStyle {
    /// The position around the node's outline, `0.0..=1.0`; `None` all
    /// the way down faces the other end.
    pub border_t: Option<f32>,
    /// The distance from the outline, in canvas units.
    pub offset: Option<f32>,
    /// The marker color.
    pub color: Option<ColorRef>,
    /// The marker size in points.
    pub size: Option<f32>,
    /// The marker drawn when the end has no span children; an end with
    /// spans ignores it.
    pub content: Option<String>,
}

/// One end of a portal link.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortalEndData {
    /// Which end: 0 is the source end, beside the link's node; 1 is the
    /// target end, beside the node it leads to. It is the end's channel.
    pub end: u8,
    /// Its style.
    pub style: PortalEndStyle,
}

/// One element of a document tree: a [`DocKind`] and its data.
///
/// `Defaults` is boxed: its data holds a style struct for every kind,
/// and inline it would make every element of every document as large
/// as itself.
#[derive(Clone, Debug, PartialEq)]
pub enum DocElement {
    /// The root.
    Map(MapData),
    /// The theme.
    Theme(ThemeData),
    /// A palette.
    Palette(PaletteData),
    /// The map's defaults.
    Defaults(Box<DefaultsData>),
    /// A section.
    Section(SectionData),
    /// A span.
    Span(SpanData),
    /// A frame.
    Frame(FrameData),
    /// A ring.
    Ring(RingData),
    /// A connector.
    Connector(ConnectorData),
    /// A start cap.
    CapStart(CapData),
    /// An end cap.
    CapEnd(CapData),
    /// A label.
    Label(LabelData),
    /// A link.
    Link(LinkData),
    /// A portal end.
    PortalEnd(PortalEndData),
    /// A node.
    Node(NodeData),
}

impl DocElement {
    /// The element's kind. O(1).
    pub fn kind(&self) -> DocKind {
        match self {
            DocElement::Map(_) => DocKind::Map,
            DocElement::Theme(_) => DocKind::Theme,
            DocElement::Palette(_) => DocKind::Palette,
            DocElement::Defaults(_) => DocKind::Defaults,
            DocElement::Section(_) => DocKind::Section,
            DocElement::Span(_) => DocKind::Span,
            DocElement::Frame(_) => DocKind::Frame,
            DocElement::Ring(_) => DocKind::Ring,
            DocElement::Connector(_) => DocKind::Connector,
            DocElement::CapStart(_) => DocKind::CapStart,
            DocElement::CapEnd(_) => DocKind::CapEnd,
            DocElement::Label(_) => DocKind::Label,
            DocElement::Link(_) => DocKind::Link,
            DocElement::PortalEnd(_) => DocKind::PortalEnd,
            DocElement::Node(_) => DocKind::Node,
        }
    }

    /// The element's channel within its kind: stored for a node, a
    /// section and a ring; a portal end's [`end`](PortalEndData::end);
    /// `0` for every other kind. O(1).
    pub fn channel(&self) -> usize {
        match self {
            DocElement::Node(node) => node.channel,
            DocElement::Section(section) => section.channel,
            DocElement::Ring(ring) => ring.channel,
            DocElement::PortalEnd(end) => usize::from(end.end),
            _ => 0,
        }
    }

    /// The node's data, if this is a node. O(1).
    pub fn as_node(&self) -> Option<&NodeData> {
        match self {
            DocElement::Node(node) => Some(node),
            _ => None,
        }
    }

    /// The path, if this is a connector or a link. O(1).
    pub fn as_path(&self) -> Option<&PathData> {
        match self {
            DocElement::Connector(connector) => Some(&connector.path),
            DocElement::Link(link) => Some(&link.path),
            _ => None,
        }
    }

    /// The element's own text style, for the kinds that hold one: a
    /// node's `style.text`, a section's, a span's, a frame's, a ring's,
    /// a cap's and a label's. `None` for every other kind — a path and a
    /// portal end carry their text fields in their own style
    /// ([`FlowStyle`], [`PortalEndStyle`]). O(1).
    pub fn text_style(&self) -> Option<&TextStyle> {
        match self {
            DocElement::Node(node) => Some(&node.style.text),
            DocElement::Section(section) => Some(&section.text),
            DocElement::Span(span) => Some(&span.style),
            DocElement::Frame(frame) => Some(&frame.style.text),
            DocElement::Ring(ring) => Some(&ring.style.text),
            DocElement::CapStart(cap) | DocElement::CapEnd(cap) => Some(&cap.text),
            DocElement::Label(label) => Some(&label.style.text),
            _ => None,
        }
    }
}

/// The document instantiation of the engine.
///
/// - `Kind` is [`DocKind`], so the walker routes on kind and channel and
///   a mutator for one kind never lands on another (§4.1).
/// - `Cache` is `()`: a document derives nothing per tree. Resolved
///   values are projection state (V1-B7), never cached here.
/// - `Inherited` is `()` for now. What a document walk carries down —
///   the resolved text size, and whether the walk is inside a frame —
///   belongs with the mutations that read it, V1-B3.
/// - `SPATIAL` is `false`: a document element has no box to hit-test;
///   hit-testing happens on the glyph layers.
/// - Only a span splits and merges: `SplitPoint` is a grapheme-cluster
///   index into its text (baumhard §B3), and two spans merge when their
///   styles and links are equal.
impl TreeElement for DocElement {
    type Kind = DocKind;
    type Cache = ();
    type Inherited = ();
    /// A grapheme-cluster index into a span's text.
    type SplitPoint = usize;
    const SPATIAL: bool = false;

    fn kind(&self) -> DocKind {
        DocElement::kind(self)
    }

    fn channel(&self) -> usize {
        DocElement::channel(self)
    }

    /// An unnamed map, the root of an empty tree.
    fn void() -> Self {
        DocElement::Map(MapData::default())
    }

    /// `==`: every field of every kind is state — a document element
    /// holds no callback, cache or other bookkeeping — and the derived
    /// `Clone` copies all of it. (A `NaN` compares unequal to itself,
    /// which costs a redundant journal entry, never a lost one.)
    fn same_state(&self, other: &Self) -> bool {
        self == other
    }

    fn inherit(&self, _parent: &()) {}

    /// A span splits at a grapheme-cluster index into its first `at`
    /// clusters and the rest, both with its style and link; it splits
    /// only where both parts hold text, so `at` is between 1 and its
    /// cluster count minus one. No other kind splits. Cost: one
    /// grapheme walk to `at`, the copy of the rest and a clone of the
    /// style and link.
    fn split(&self, at: &usize) -> Option<(Self, Self)> {
        let DocElement::Span(span) = self else {
            return None;
        };
        if *at == 0 {
            return None;
        }
        let mut first = span.clone();
        let rest = split_off_graphemes(&mut first.text, *at);
        if rest.is_empty() {
            return None;
        }
        let second = SpanData {
            text: rest,
            style: span.style.clone(),
            link: span.link.clone(),
        };
        Some((DocElement::Span(first), DocElement::Span(second)))
    }

    /// Two spans with equal styles and equal links merge into one
    /// holding the first's text followed by the second's. Nothing else
    /// merges. Cost: the two style and link compares and the joined
    /// text.
    fn merge(&self, next: &Self) -> Option<Self> {
        let (DocElement::Span(span), DocElement::Span(next)) = (self, next) else {
            return None;
        };
        if span.style != next.style || span.link != next.link {
            return None;
        }
        let mut merged = span.clone();
        merged.text.push_str(&next.text);
        Some(DocElement::Span(merged))
    }
}
