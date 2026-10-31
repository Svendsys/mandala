// SPDX-License-Identifier: MPL-2.0

//! Per-field style resolution, recording every input it reads.
//!
//! # The chain
//!
//! Every style field is an `Option`, and `None` means inherit. A
//! resolved value is the first `Some` along a chain of tiers, field by
//! field — never struct by struct, so an element that sets one field
//! still inherits every other (design §5.4):
//!
//! 1. **The element's own field.**
//! 2. **The owner chain.** For text, a span's container — its section,
//!    ring, cap, label, path or portal end — then, for a ring, its frame,
//!    and for a cap, its path's size and color; then the owning node's
//!    `style.text`. A path's and a portal end's
//!    text fields are their `size` and `color`; a ring's fit falls back
//!    to its frame's.
//! 3. **The palette tier.** The owning node's binding — the nearest node
//!    at or above it with `palette: Some(p)` — colors whatever is still
//!    unset, from group `depth(node) − depth(binding)` clamped to the
//!    last group: a node's fill from the group's `fill`, frame and ring
//!    text from `frame`, section text from `text`, and the node's first
//!    section (lowest channel) from `title` — or from `text` when the
//!    group sets no `title` (ruling R5 on #235). Connectors, links, caps,
//!    labels and portal ends are not palette-colored.
//! 4. **`Defaults`**, the map's element: the struct matching each tier
//!    of the owner chain, in the same order — `Defaults.span` for a
//!    span, then the container's (`section`, `ring` then `frame`,
//!    `label`, `connector`, `link` or `portal_end`; a cap reads its
//!    path's), then `node.text`.
//! 5. **[`BUILT_IN`](crate::document::defaults::BUILT_IN)**, keyed by
//!    where the text is drawn — a label by its host.
//!
//! **Size and color stay in their flow** (amendment 3 and ruling R4 on
//! #235). For every flow but a section, `size` and `color` come only
//! from that flow's own chain: the span, its container (a ring's frame
//! and a cap's path included), the palette tier where it maps one (ring and frame text),
//! the flow's own `Defaults` struct and `BUILT_IN`. They never read the
//! owning node's `style.text`, `Defaults.span` or `Defaults.node.text`:
//! a connector, a portal marker, a label or a frame lives at a scale and
//! in a color of its own, and today none of them takes either from the
//! node's text. Inside a frame the span's own size is not read either,
//! so a ring's size is always its one
//! [`Rel`](crate::document::values::FontSize::Rel) size; the projection
//! turns it into points from the node's drawn size. The face, weight
//! and slant keep the whole chain, so a map-wide font reaches every
//! flow.
//!
//! A color tier holding a
//! [`ColorRef`](crate::document::values::ColorRef) resolves it on the
//! spot: an `Rgba` is final, a `Var` reads the theme's active variant,
//! and a `Palette(field)` reads the owning node's binding. A `Var` the
//! theme does not have, or a `Palette` with no binding, leaves the field
//! unset and the chain goes on.
//!
//! # Recording reads
//!
//! Every resolver takes a
//! [`ReadRecorder`](crate::document::resolve::ReadRecorder) and reports
//! to it every input it **looks at**, named as narrowly as the read
//! itself: an element's style fields
//! ([`Input::Style`](crate::document::resolve::Input::Style)), its
//! placement fields (`Placement`), a node's
//! `palette` field on the binding walk (`PaletteBinding`), the structure
//! of a children row (`Row`), the theme, a palette, the defaults, and the
//! depth of a node whose palette group it computed. Tiers that turned
//! out to say nothing are reported too. What it does not look at, it
//! does not report: a span whose own color is an `Rgba` reports its own
//! style and nothing else for its color, and nothing a resolver reads is
//! a position or a size, so moving or resizing a node dirties no
//! resolution, and no text resolution reads a placement, so dragging a
//! label or a portal marker dirties only its placement. V1-B7 builds its dependency index from these records
//! (design §12.3) and maps a journal entry onto them with
//! [`inputs_changed`](crate::document::deps::inputs_changed); a read
//! that goes unreported is a projection that goes stale.
//!
//! Resolvers work from the tree, not the `Document`, so a mutation's
//! inherited-state seed can call them with the tree it is applying to.
//! They cache nothing: caching is the projection's.
//!
//! # One-off questions
//!
//! Each resolver answers for one element, and the palette tier finds
//! its binding by climbing from the owning node to the nearest node
//! with a palette — O(distance to that binding), the whole depth of the
//! node when no node above it binds one. That is the cost of one
//! question, not of a walk: a pass that resolves every node of a map
//! carries the binding down the walk instead of asking per node, or it
//! is quadratic in depth (design §20 rule 11; ruling R16 on #282 notes
//! this for V1-B7 and V1-C2).

use crate::document::defaults::{BuiltInPath, BUILT_IN};
use crate::document::element::{
    DefaultsData, DocElement, DocKind, FlowStyle, PaletteGroup, TextStyle, ThemeData,
};
use crate::document::ids::{defaults, palette, theme};
use crate::document::values::{
    ColorRef, Direction, FitMode, FontSize, PaletteField, PaletteId, PlainColor, Rgba, Shape, Sides, Span,
    StartPoint, VarId, Weight,
};
use crate::engine::tree::Tree;
use crate::font::fonts::AppFont;
use indextree::NodeId;

/// An input a resolution read. Each names as little as the read
/// needed, so a change dirties only the resolutions that read what
/// changed; [`inputs_changed`](crate::document::deps::inputs_changed)
/// is the other half of the contract, mapping a change of an element
/// onto these.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// An element's style fields: what it draws with — a text style, a
    /// node's shape, fill and fill fade, a path's flow style, a frame's
    /// visibility and fit, a ring's fit, a portal end's marker color,
    /// size and content. Never a position, a placement, an ident or a
    /// flag.
    Style(NodeId),
    /// An element's placement fields: where it sits, not how it looks —
    /// a label's `t` and `offset`, a portal end's `border_t` and
    /// `offset`, a frame's `start`, `direction`, `gap` and `inset`. A
    /// label or portal-marker drag changes only these, so it dirties no
    /// text resolution.
    Placement(NodeId),
    /// A node's `palette` field, read on the walk up to the nearest
    /// binding — one per node the walk looks at.
    PaletteBinding(NodeId),
    /// The structure of an element's children row: which section is a
    /// node's first, and whether a path or portal end has spans.
    Row(NodeId),
    /// The theme: its active variant and that variant's values.
    Theme,
    /// A palette's groups.
    Palette(PaletteId),
    /// The map's `Defaults` element.
    Defaults,
    /// A node's depth: the nodes above it.
    Depth(NodeId),
}

/// Where a resolver reports what it read.
pub trait ReadRecorder {
    /// `input` was read. Called once per input per tier that looked at
    /// it; the same input may be reported more than once.
    fn read(&mut self, input: Input);
}

/// Records nothing: for a resolution whose inputs nobody tracks.
impl ReadRecorder for () {
    fn read(&mut self, _: Input) {}
}

/// Records every read, in order. One push per read.
impl ReadRecorder for Vec<Input> {
    fn read(&mut self, input: Input) {
        self.push(input);
    }
}

/// Resolved text: what a span draws with.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedText {
    /// The face.
    pub font: AppFont,
    /// The size. Still [`Rel`](FontSize::Rel) inside a frame; the
    /// projection converts it with the node's drawn size.
    pub size: FontSize,
    /// The color.
    pub color: Rgba,
    /// The weight.
    pub weight: Weight,
    /// Whether it is italic.
    pub italic: bool,
}

/// A resolved node.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedNode {
    /// The outline.
    pub shape: Shape,
    /// The fill.
    pub fill: Rgba,
    /// The width of the fill's soft edge.
    pub fill_fade: f32,
}

/// A resolved frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFrame {
    /// Whether it is drawn.
    pub visible: bool,
    /// Where its text starts.
    pub start: StartPoint,
    /// Which way its text runs.
    pub direction: Direction,
    /// Its rings' fit, unless a ring overrides it.
    pub fit: FitMode,
    /// The space between rings.
    pub gap: f32,
    /// The first ring's distance from the outline.
    pub inset: f32,
}

/// A resolved ring.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRing {
    /// What its text does when text and outline differ in length.
    pub fit: FitMode,
    /// How much of its half each of its two lines covers.
    pub span: Span,
    /// The ornament in the breaks between its two lines.
    pub sides: Sides,
    /// The space added between its copies when it repeats.
    pub spacing: f32,
}

/// What a path or a portal end draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Its span children.
    Spans,
    /// This text: it has no span children.
    Text(String),
}

/// A resolved connector or line link. Its text is
/// [`resolve_text`] of the path.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPath {
    /// What its text does when text and path differ in length.
    pub fit: FitMode,
    /// The extra space between glyphs.
    pub spacing: f32,
    /// What it draws.
    pub content: Content,
}

/// A resolved label. Its text is [`resolve_text`] of the label.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedLabel {
    /// The position along the path.
    pub t: f32,
    /// The distance from the path.
    pub offset: f32,
}

/// Where a portal end sits around its node's outline.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BorderPoint {
    /// Facing the other end, placed by the projection.
    Auto,
    /// At this position, `0.0..=1.0` around the outline.
    At(f32),
}

/// A resolved portal end. Its marker text is [`resolve_text`] of the
/// end.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPortalEnd {
    /// Where it sits around the outline.
    pub border_t: BorderPoint,
    /// Its distance from the outline.
    pub offset: f32,
    /// What its marker draws.
    pub content: Content,
}

/// The element at `id`, if it is in the tree. O(1).
fn element(tree: &Tree<DocElement>, id: NodeId) -> Option<&DocElement> {
    tree.get(id).map(|node| node.get())
}

/// The parent of `id`. O(1).
fn parent(tree: &Tree<DocElement>, id: NodeId) -> Option<NodeId> {
    tree.get(id)?.parent()
}

/// One resolution's view of the inputs every tier shares — the theme,
/// the defaults, the owning node's palette binding — each looked up the
/// first time a tier needs it, reported then, and kept for the rest of
/// the resolution (and no longer: nothing outlives the call).
struct Inputs<'t, R: ReadRecorder> {
    tree: &'t Tree<DocElement>,
    rec: &'t mut R,
    owning_node: Option<NodeId>,
    theme: Option<Option<&'t ThemeData>>,
    defaults: Option<Option<&'t DefaultsData>>,
    binding: Option<Option<&'t PaletteGroup>>,
}

impl<'t, R: ReadRecorder> Inputs<'t, R> {
    fn new(tree: &'t Tree<DocElement>, rec: &'t mut R, owning_node: Option<NodeId>) -> Self {
        Inputs {
            tree,
            rec,
            owning_node,
            theme: None,
            defaults: None,
            binding: None,
        }
    }

    /// Report that the style fields of the element `id` were read, and
    /// return it.
    fn style(&mut self, id: NodeId) -> Option<&'t DocElement> {
        self.rec.read(Input::Style(id));
        element(self.tree, id)
    }

    /// Report that the placement fields of the element `id` were read,
    /// and return it.
    fn placement(&mut self, id: NodeId) -> Option<&'t DocElement> {
        self.rec.read(Input::Placement(id));
        element(self.tree, id)
    }

    /// Report that the children row of `id` was read.
    fn row(&mut self, id: NodeId) {
        self.rec.read(Input::Row(id));
    }

    /// The theme, reported on first use. O(1).
    fn theme(&mut self) -> Option<&'t ThemeData> {
        let tree = self.tree;
        *self.theme.get_or_insert_with(|| {
            self.rec.read(Input::Theme);
            theme(tree).map(|(_, theme)| theme)
        })
    }

    /// The defaults, reported on first use. O(palettes).
    fn defaults(&mut self) -> Option<&'t DefaultsData> {
        let tree = self.tree;
        *self.defaults.get_or_insert_with(|| {
            self.rec.read(Input::Defaults);
            defaults(tree).map(|(_, defaults)| defaults)
        })
    }

    /// The palette group of the owning node, worked out on first use:
    /// the nearest node at or above it with a palette, and the group
    /// `depth(node) − depth(binding)` of that palette, clamped to the
    /// last. Reports the owning node's depth, the `palette` field of
    /// every node it looks at on the way up, and, when it finds a
    /// binding, the binding's depth and its palette. O(distance to the
    /// binding): the climb stops at the nearest node with a palette, or
    /// runs to the top of the nodes when none has one. A one-off per
    /// resolution (see the module docs).
    fn binding(&mut self) -> Option<&'t PaletteGroup> {
        if let Some(found) = self.binding {
            return found;
        }
        let found = self.find_binding();
        self.binding = Some(found);
        found
    }

    fn find_binding(&mut self) -> Option<&'t PaletteGroup> {
        let tree = self.tree;
        let start = self.owning_node?;
        self.rec.read(Input::Depth(start));
        let mut steps = 0usize;
        let mut at = start;
        loop {
            self.rec.read(Input::PaletteBinding(at));
            let DocElement::Node(node) = element(tree, at)? else {
                return None;
            };
            if let Some(id) = node.palette {
                self.rec.read(Input::Depth(at));
                self.rec.read(Input::Palette(id));
                let groups = &palette(tree, id)?.groups;
                return groups.get(steps.min(groups.len().checked_sub(1)?));
            }
            at = parent(tree, at)?;
            if element(tree, at)?.kind() != DocKind::Node {
                return None;
            }
            steps += 1;
        }
    }

    /// A theme variable's value in the active variant, or `None` when
    /// the theme does not have it.
    fn var(&mut self, var: VarId) -> Option<Rgba> {
        let theme = self.theme()?;
        let variant = theme.variants.get(usize::from(theme.active))?;
        variant.values.get(usize::from(var.0)).copied()
    }

    /// A palette color, final.
    fn plain(&mut self, color: PlainColor) -> Option<Rgba> {
        match color {
            PlainColor::Rgba(rgba) => Some(rgba),
            PlainColor::Var(var) => self.var(var),
        }
    }

    /// A stored color, resolved; `None` lets the chain go on.
    fn color(&mut self, color: ColorRef) -> Option<Rgba> {
        match color {
            ColorRef::Rgba(rgba) => Some(rgba),
            ColorRef::Var(var) => self.var(var),
            ColorRef::Palette(field) => {
                let group = self.binding()?;
                self.plain(palette_color(group, field))
            }
        }
    }

    /// The palette tier for one field: the binding's group's `field`.
    fn palette_tier(&mut self, field: PaletteField) -> Option<Rgba> {
        let group = self.binding()?;
        self.plain(palette_color(group, field))
    }
}

/// The color `field` names in `group`; a `title` the group does not set
/// is its `text`, within the same tier (ruling R5 on #235). O(1).
fn palette_color(group: &PaletteGroup, field: PaletteField) -> PlainColor {
    match field {
        PaletteField::Fill => group.fill,
        PaletteField::Frame => group.frame,
        PaletteField::Text => group.text,
        PaletteField::Title => group.title.unwrap_or(group.text),
    }
}

/// Where text is drawn, which decides its owner chain, its palette
/// field and its `Defaults` and `BUILT_IN` structs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Flow {
    /// In a section.
    Section,
    /// In a ring.
    Ring,
    /// A frame's own text defaults.
    Frame,
    /// In a cap of a connector (`Connector`) or a link (`Link`).
    Cap(DocKind),
    /// In a label on a connector, a line link or a portal end — its
    /// host's kind.
    Label(DocKind),
    /// Along a connector (`Connector`) or a link (`Link`).
    Path(DocKind),
    /// In a portal marker.
    PortalEnd,
    /// A node's own text defaults: what its sections inherit.
    Node,
    /// Anywhere else — the map, theme, palettes or defaults, which hold
    /// no text: only `Defaults` and `BUILT_IN`'s section text apply.
    Other,
}

impl Flow {
    /// The flow an element is the container of. A cap's and a label's
    /// flow depends on the kind of their parent, a structural fact a
    /// field change cannot alter.
    fn of(tree: &Tree<DocElement>, container: &DocElement, container_id: NodeId) -> Flow {
        let host = || {
            parent(tree, container_id)
                .and_then(|p| element(tree, p))
                .map_or(DocKind::Connector, DocElement::kind)
        };
        match container.kind() {
            DocKind::Section => Flow::Section,
            DocKind::Ring => Flow::Ring,
            DocKind::Frame => Flow::Frame,
            DocKind::CapStart | DocKind::CapEnd => Flow::Cap(match host() {
                DocKind::Link => DocKind::Link,
                _ => DocKind::Connector,
            }),
            DocKind::Label => Flow::Label(match host() {
                DocKind::Link => DocKind::Link,
                DocKind::PortalEnd => DocKind::PortalEnd,
                _ => DocKind::Connector,
            }),
            DocKind::Connector => Flow::Path(DocKind::Connector),
            DocKind::Link => Flow::Path(DocKind::Link),
            DocKind::PortalEnd => Flow::PortalEnd,
            DocKind::Node => Flow::Node,
            _ => Flow::Other,
        }
    }

    /// Whether the size comes from the ring chain alone, the span's own
    /// size included.
    fn in_frame(self) -> bool {
        matches!(self, Flow::Ring | Flow::Frame)
    }

    /// Whether size and color stay in the flow's own chain (R4): every
    /// flow but a section's — and a node's own, which is what its
    /// sections inherit.
    fn keeps_size_and_color(self) -> bool {
        !matches!(self, Flow::Section | Flow::Node | Flow::Other)
    }

    /// The text `BUILT_IN` gives this flow.
    fn built_in(self) -> ResolvedText {
        match self {
            Flow::Section | Flow::Node | Flow::Other => BUILT_IN.section,
            Flow::Ring | Flow::Frame => BUILT_IN.ring.text,
            Flow::Cap(DocKind::Link) | Flow::Path(DocKind::Link) => BUILT_IN.link.text,
            Flow::Cap(_) | Flow::Path(_) => BUILT_IN.connector.text,
            Flow::Label(DocKind::Link) => BUILT_IN.label.on_link,
            Flow::Label(DocKind::PortalEnd) => BUILT_IN.label.on_portal_end,
            Flow::Label(_) => BUILT_IN.label.on_connector,
            Flow::PortalEnd => BUILT_IN.portal_end.text,
        }
    }
}

/// Which text fields a tier may give.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Fields {
    /// The size.
    size: bool,
    /// The color.
    color: bool,
    /// The face, weight and slant.
    rest: bool,
}

impl Fields {
    /// Every field.
    const ALL: Fields = Fields {
        size: true,
        color: true,
        rest: true,
    };
    /// The face, weight and slant: what a tier outside a flow's own
    /// chain may still give it.
    const REST: Fields = Fields {
        size: false,
        color: false,
        rest: true,
    };
    /// Every field but the size.
    const NOT_SIZE: Fields = Fields {
        size: false,
        color: true,
        rest: true,
    };
}

/// The text fields resolved so far.
#[derive(Default)]
struct Partial {
    font: Option<AppFont>,
    size: Option<FontSize>,
    color: Option<Rgba>,
    weight: Option<Weight>,
    italic: Option<bool>,
}

impl Partial {
    /// Whether a tier that may give `fields` still has anything to give.
    fn wants(&self, fields: Fields) -> bool {
        (fields.size && self.size.is_none())
            || (fields.color && self.color.is_none())
            || (fields.rest && (self.font.is_none() || self.weight.is_none() || self.italic.is_none()))
    }

    /// Take every unset field among `fields` that `style` sets.
    fn take_text<R: ReadRecorder>(&mut self, style: &TextStyle, fields: Fields, inputs: &mut Inputs<'_, R>) {
        if fields.rest {
            self.font = self.font.or(style.font);
            self.weight = self.weight.or(style.weight);
            self.italic = self.italic.or(style.italic);
        }
        if fields.size {
            self.size = self.size.or(style.size);
        }
        if fields.color && self.color.is_none() {
            self.color = style.color.and_then(|c| inputs.color(c));
        }
    }

    /// Take the size and color a path or portal end sets; its size is
    /// in points.
    fn take_flow<R: ReadRecorder>(
        &mut self,
        size: Option<f32>,
        color: Option<ColorRef>,
        inputs: &mut Inputs<'_, R>,
    ) {
        self.size = self.size.or(size.map(FontSize::Pt));
        if self.color.is_none() {
            self.color = color.and_then(|c| inputs.color(c));
        }
    }

    /// Fill what is still unset from `built_in`.
    fn finish(self, built_in: ResolvedText) -> ResolvedText {
        ResolvedText {
            font: self.font.unwrap_or(built_in.font),
            size: self.size.unwrap_or(built_in.size),
            color: self.color.unwrap_or(built_in.color),
            weight: self.weight.unwrap_or(built_in.weight),
            italic: self.italic.unwrap_or(built_in.italic),
        }
    }
}

/// The text an element draws with: `owner` is a span, or an element
/// that holds spans — a section, ring, frame, cap, label, connector,
/// link or portal end — whose text a span with no style of its own
/// would draw with (a path's and a portal end's `content` draws with
/// it). For a node it is what the node's sections inherit from it.
///
/// See the module docs for the chain. An `owner` not in the tree
/// resolves to `BUILT_IN`'s section text and reads nothing.
///
/// Cost: the owner chain, at most four elements, plus — only when the
/// palette tier is reached — the climb to the nearest binding, O(its
/// distance; the depth when there is none). A one-off question: see the
/// module docs. No allocation.
pub fn resolve_text(tree: &Tree<DocElement>, owner: NodeId, rec: &mut impl ReadRecorder) -> ResolvedText {
    let Some(owner_element) = element(tree, owner) else {
        return BUILT_IN.section;
    };
    let (span, container) = if owner_element.kind() == DocKind::Span {
        (Some(owner), parent(tree, owner))
    } else {
        (None, Some(owner))
    };
    let container_element = container.and_then(|c| element(tree, c));
    let flow = match (container, container_element) {
        (Some(id), Some(e)) => Flow::of(tree, e, id),
        _ => Flow::Other,
    };
    // What the span itself may give: inside a frame, not its size.
    let own = if flow.in_frame() {
        Fields::NOT_SIZE
    } else {
        Fields::ALL
    };
    // What tiers outside the flow's own chain may give it.
    let outside = if flow.keeps_size_and_color() {
        Fields::REST
    } else {
        Fields::ALL
    };
    let owning = container.and_then(|c| crate::document::ids::owning_node(tree, c));
    let mut inputs = Inputs::new(tree, rec, owning);
    let mut got = Partial::default();

    // 1. The span's own style.
    if let Some(span) = span {
        if let Some(style) = inputs.style(span).and_then(DocElement::text_style) {
            got.take_text(style, own, &mut inputs);
        }
    }

    // 2. The container, then a ring's frame.
    let flow_fields = Fields {
        size: true,
        color: true,
        rest: false,
    };
    if let (Some(id), Some(container_element)) = (container, container_element) {
        match container_element {
            DocElement::Connector(_) | DocElement::Link(_) => {
                if got.wants(flow_fields) {
                    if let Some(path) = inputs.style(id).and_then(DocElement::as_path) {
                        got.take_flow(path.flow.size, path.flow.color, &mut inputs);
                    }
                }
            }
            DocElement::PortalEnd(_) => {
                if got.wants(flow_fields) {
                    if let Some(DocElement::PortalEnd(end)) = inputs.style(id) {
                        got.take_flow(end.style.size, end.style.color, &mut inputs);
                    }
                }
            }
            DocElement::Node(_) => {}
            _ => {
                if got.wants(Fields::ALL) {
                    if let Some(style) = inputs.style(id).and_then(DocElement::text_style) {
                        got.take_text(style, Fields::ALL, &mut inputs);
                    }
                }
                if flow == Flow::Ring && got.wants(Fields::ALL) {
                    if let Some(frame) = parent(tree, id) {
                        if let Some(style) = inputs.style(frame).and_then(DocElement::text_style) {
                            got.take_text(style, Fields::ALL, &mut inputs);
                        }
                    }
                }
                // A cap belongs to its path, as a ring to its frame: the
                // path's own size and color come next.
                if matches!(flow, Flow::Cap(_)) && got.wants(flow_fields) {
                    if let Some(path) = parent(tree, id) {
                        if let Some(path) = inputs.style(path).and_then(DocElement::as_path) {
                            got.take_flow(path.flow.size, path.flow.color, &mut inputs);
                        }
                    }
                }
            }
        }
    }

    // 2b. The owning node: outside every flow's own chain but a
    // section's.
    if let Some(node) = owning {
        if got.wants(outside) {
            if let Some(style) = inputs.style(node).and_then(DocElement::text_style) {
                got.take_text(style, outside, &mut inputs);
            }
        }
    }

    // 3. The palette tier, for the color.
    if got.color.is_none() {
        let field = match flow {
            Flow::Section => Some(match (owning, container) {
                (Some(node), Some(section)) => {
                    inputs.row(node);
                    if node.children(tree.arena()).next() == Some(section) {
                        PaletteField::Title
                    } else {
                        PaletteField::Text
                    }
                }
                _ => PaletteField::Text,
            }),
            Flow::Node => Some(PaletteField::Text),
            Flow::Ring | Flow::Frame => Some(PaletteField::Frame),
            _ => None,
        };
        if let Some(field) = field {
            got.color = inputs.palette_tier(field);
        }
    }

    // 4. Defaults: the structs matching the owner chain, in its order.
    if got.wants(Fields::ALL) {
        if let Some(defaults) = inputs.defaults() {
            if span.is_some() {
                let span_fields = if flow.keeps_size_and_color() {
                    Fields::REST
                } else {
                    own
                };
                got.take_text(&defaults.span, span_fields, &mut inputs);
            }
            match flow {
                Flow::Section => got.take_text(&defaults.section, Fields::ALL, &mut inputs),
                Flow::Ring => {
                    got.take_text(&defaults.ring.text, Fields::ALL, &mut inputs);
                    got.take_text(&defaults.frame.text, Fields::ALL, &mut inputs);
                }
                Flow::Frame => got.take_text(&defaults.frame.text, Fields::ALL, &mut inputs),
                Flow::Label(_) => got.take_text(&defaults.label.text, Fields::ALL, &mut inputs),
                Flow::Path(kind) | Flow::Cap(kind) => {
                    let flow_style = path_defaults(defaults, kind);
                    got.take_flow(flow_style.size, flow_style.color, &mut inputs);
                }
                Flow::PortalEnd => {
                    got.take_flow(defaults.portal_end.size, defaults.portal_end.color, &mut inputs)
                }
                Flow::Node | Flow::Other => {}
            }
            got.take_text(&defaults.node.text, outside, &mut inputs);
        }
    }

    // 5. BUILT_IN.
    got.finish(flow.built_in())
}

/// The `Defaults` struct of a path of `kind`.
fn path_defaults(defaults: &DefaultsData, kind: DocKind) -> &FlowStyle {
    if kind == DocKind::Link {
        &defaults.link
    } else {
        &defaults.connector
    }
}

/// The `BUILT_IN` struct of a path of `kind`.
fn path_built_in(kind: DocKind) -> BuiltInPath {
    if kind == DocKind::Link {
        BUILT_IN.link
    } else {
        BUILT_IN.connector
    }
}

/// A node's shape, fill and fill fade. The chain: the node's style, the
/// palette tier (the fill, from the group's `fill`), `Defaults.node`,
/// `BUILT_IN`. A `node` that is not a node has no own tier.
///
/// Cost: when the palette tier is reached, the climb to the nearest
/// binding, O(its distance; the depth when there is none); else O(1).
/// A one-off question: see the module docs. No allocation.
pub fn resolve_node(tree: &Tree<DocElement>, node: NodeId, rec: &mut impl ReadRecorder) -> ResolvedNode {
    let owning = element(tree, node)
        .filter(|e| e.kind() == DocKind::Node)
        .map(|_| node);
    let mut inputs = Inputs::new(tree, rec, owning);
    let own = owning
        .and_then(|id| inputs.style(id))
        .and_then(DocElement::as_node);
    let mut shape = own.and_then(|n| n.style.shape);
    let mut fill = own.and_then(|n| n.style.fill).and_then(|c| inputs.color(c));
    let mut fill_fade = own.and_then(|n| n.style.fill_fade);
    if fill.is_none() {
        fill = inputs.palette_tier(PaletteField::Fill);
    }
    if shape.is_none() || fill.is_none() || fill_fade.is_none() {
        if let Some(defaults) = inputs.defaults() {
            shape = shape.or(defaults.node.shape);
            if fill.is_none() {
                fill = defaults.node.fill.and_then(|c| inputs.color(c));
            }
            fill_fade = fill_fade.or(defaults.node.fill_fade);
        }
    }
    ResolvedNode {
        shape: shape.unwrap_or(BUILT_IN.node.shape),
        fill: fill.unwrap_or(BUILT_IN.node.fill),
        fill_fade: fill_fade.unwrap_or(BUILT_IN.node.fill_fade),
    }
}

/// A frame's fields. The chain: the frame's style, `Defaults.frame`,
/// `BUILT_IN`. Reads the frame's style (`visible`, `fit`) and its
/// placement (`start`, `direction`, `gap`, `inset`). Its text is
/// [`resolve_text`] of the frame.
///
/// Cost: O(1). No allocation beyond cloning a `FitMode::Once` filler.
pub fn resolve_frame(tree: &Tree<DocElement>, frame: NodeId, rec: &mut impl ReadRecorder) -> ResolvedFrame {
    let mut inputs = Inputs::new(tree, rec, None);
    inputs.placement(frame);
    let own = match inputs.style(frame) {
        Some(DocElement::Frame(frame)) => Some(&frame.style),
        _ => None,
    };
    let mut visible = own.and_then(|s| s.visible);
    let mut start = own.and_then(|s| s.start);
    let mut direction = own.and_then(|s| s.direction);
    let mut fit = own.and_then(|s| s.fit.as_ref());
    let mut gap = own.and_then(|s| s.gap);
    let mut inset = own.and_then(|s| s.inset);
    let complete = visible.is_some()
        && start.is_some()
        && direction.is_some()
        && fit.is_some()
        && gap.is_some()
        && inset.is_some();
    if !complete {
        if let Some(defaults) = inputs.defaults() {
            let d = &defaults.frame;
            visible = visible.or(d.visible);
            start = start.or(d.start);
            direction = direction.or(d.direction);
            fit = fit.or(d.fit.as_ref());
            gap = gap.or(d.gap);
            inset = inset.or(d.inset);
        }
    }
    let built_in = BUILT_IN.frame;
    ResolvedFrame {
        visible: visible.unwrap_or(built_in.visible),
        start: start.unwrap_or(built_in.start),
        direction: direction.unwrap_or(built_in.direction),
        fit: fit.cloned().unwrap_or(built_in.fit),
        gap: gap.unwrap_or(built_in.gap),
        inset: inset.unwrap_or(built_in.inset),
    }
}

/// A ring's fit, span, sides and spacing, each on its own. The chain:
/// the ring, its frame, `Defaults.ring`, `Defaults.frame`, `BUILT_IN`'s
/// frame. Its text is [`resolve_text`] of the ring.
///
/// Cost: O(1). No allocation beyond cloning a `FitMode::Once` filler and
/// a `Sides` ornament.
pub fn resolve_ring(tree: &Tree<DocElement>, ring: NodeId, rec: &mut impl ReadRecorder) -> ResolvedRing {
    let mut inputs = Inputs::new(tree, rec, None);
    let own = match inputs.style(ring) {
        Some(DocElement::Ring(ring)) => Some(&ring.style),
        _ => None,
    };
    let mut fit = own.and_then(|s| s.fit.as_ref());
    let mut span = own.and_then(|s| s.ring_layout.span);
    let mut sides = own.and_then(|s| s.ring_layout.sides.as_ref());
    let mut spacing = own.and_then(|s| s.ring_layout.spacing);
    let complete =
        |fit: Option<&FitMode>, span: Option<Span>, sides: Option<&Sides>, spacing: Option<f32>| {
            fit.is_some() && span.is_some() && sides.is_some() && spacing.is_some()
        };
    if !complete(fit, span, sides, spacing) {
        if let Some(frame) = parent(tree, ring) {
            if let Some(DocElement::Frame(frame)) = inputs.style(frame) {
                let f = &frame.style;
                fit = fit.or(f.fit.as_ref());
                span = span.or(f.ring_layout.span);
                sides = sides.or(f.ring_layout.sides.as_ref());
                spacing = spacing.or(f.ring_layout.spacing);
            }
        }
    }
    if !complete(fit, span, sides, spacing) {
        if let Some(defaults) = inputs.defaults() {
            let (r, f) = (&defaults.ring, &defaults.frame);
            fit = fit.or(r.fit.as_ref()).or(f.fit.as_ref());
            span = span.or(r.ring_layout.span).or(f.ring_layout.span);
            sides = sides
                .or(r.ring_layout.sides.as_ref())
                .or(f.ring_layout.sides.as_ref());
            spacing = spacing.or(r.ring_layout.spacing).or(f.ring_layout.spacing);
        }
    }
    let built_in = &BUILT_IN.frame;
    ResolvedRing {
        fit: fit.cloned().unwrap_or(built_in.fit.clone()),
        span: span.unwrap_or(built_in.span),
        sides: sides.cloned().unwrap_or(built_in.sides.clone()),
        spacing: spacing.unwrap_or(built_in.spacing),
    }
}

/// Whether `id` has a span child: its row is sorted by kind, and a span
/// sorts before a cap, a label and a portal end, so it is the first
/// child when there is one. The caller reports the row read. O(1).
fn has_spans(tree: &Tree<DocElement>, id: NodeId) -> bool {
    tree.get(id).is_some()
        && id
            .children(tree.arena())
            .next()
            .is_some_and(|first| tree.arena()[first].get().kind() == DocKind::Span)
}

/// What a path or portal end with no spans draws: the first tier's
/// `content` that is set, else `built_in`. Allocates the `String`.
fn content_text(tiers: [Option<&String>; 2], built_in: &str) -> Content {
    let text = tiers
        .into_iter()
        .flatten()
        .next()
        .map_or(built_in, String::as_str);
    Content::Text(text.to_string())
}

/// A connector's or line link's fit, spacing and content. The chain:
/// the path's flow style, `Defaults.connector` or `Defaults.link`,
/// `BUILT_IN`. A path with span children draws them and never reads
/// `content`. Its text is [`resolve_text`] of the path.
///
/// Cost: O(1). Allocates the content `String` when there are no spans.
pub fn resolve_path(tree: &Tree<DocElement>, path: NodeId, rec: &mut impl ReadRecorder) -> ResolvedPath {
    let mut inputs = Inputs::new(tree, rec, None);
    let own_element = inputs.style(path);
    let kind = own_element.map_or(DocKind::Connector, DocElement::kind);
    let own = own_element.and_then(DocElement::as_path).map(|p| &p.flow);
    inputs.row(path);
    let has_spans = has_spans(tree, path);
    let mut fit = own.and_then(|f| f.fit.as_ref());
    let mut spacing = own.and_then(|f| f.spacing);
    let own_content = if has_spans {
        None
    } else {
        own.and_then(|f| f.content.as_ref())
    };
    let mut default_content = None;
    if fit.is_none() || spacing.is_none() || (!has_spans && own_content.is_none()) {
        if let Some(defaults) = inputs.defaults() {
            let d = path_defaults(defaults, kind);
            fit = fit.or(d.fit.as_ref());
            spacing = spacing.or(d.spacing);
            if !has_spans {
                default_content = d.content.as_ref();
            }
        }
    }
    let built_in = path_built_in(kind);
    ResolvedPath {
        fit: fit.cloned().unwrap_or(built_in.fit),
        spacing: spacing.unwrap_or(built_in.spacing),
        content: if has_spans {
            Content::Spans
        } else {
            content_text([own_content, default_content], built_in.content)
        },
    }
}

/// A label's position and offset — its placement. The chain: the label,
/// `Defaults.label`, `BUILT_IN`. Its text is [`resolve_text`] of the
/// label.
///
/// Cost: O(1). No allocation.
pub fn resolve_label(tree: &Tree<DocElement>, label: NodeId, rec: &mut impl ReadRecorder) -> ResolvedLabel {
    let mut inputs = Inputs::new(tree, rec, None);
    let own = match inputs.placement(label) {
        Some(DocElement::Label(label)) => Some(&label.style),
        _ => None,
    };
    let mut t = own.and_then(|s| s.t);
    let mut offset = own.and_then(|s| s.offset);
    if t.is_none() || offset.is_none() {
        if let Some(defaults) = inputs.defaults() {
            t = t.or(defaults.label.t);
            offset = offset.or(defaults.label.offset);
        }
    }
    ResolvedLabel {
        t: t.unwrap_or(BUILT_IN.label.t),
        offset: offset.unwrap_or(BUILT_IN.label.offset),
    }
}

/// A portal end's position, offset and marker content. The chain: the
/// end, `Defaults.portal_end`, `BUILT_IN`. Reads the end's placement
/// (`border_t`, `offset`) and, with no spans, its style (`content`). An end with span children
/// draws them and never reads `content`. Its marker text is
/// [`resolve_text`] of the end.
///
/// Cost: O(1). Allocates the content `String` when there are no spans.
pub fn resolve_portal_end(
    tree: &Tree<DocElement>,
    end: NodeId,
    rec: &mut impl ReadRecorder,
) -> ResolvedPortalEnd {
    let mut inputs = Inputs::new(tree, rec, None);
    let own = match inputs.placement(end) {
        Some(DocElement::PortalEnd(end)) => Some(&end.style),
        _ => None,
    };
    inputs.row(end);
    let has_spans = has_spans(tree, end);
    let mut border_t = own.and_then(|s| s.border_t);
    let mut offset = own.and_then(|s| s.offset);
    let own_content = if has_spans {
        None
    } else {
        // The marker's content is how it looks, not where it sits.
        inputs.style(end);
        own.and_then(|s| s.content.as_ref())
    };
    let mut default_content = None;
    if border_t.is_none() || offset.is_none() || (!has_spans && own_content.is_none()) {
        if let Some(defaults) = inputs.defaults() {
            let d = &defaults.portal_end;
            border_t = border_t.or(d.border_t);
            offset = offset.or(d.offset);
            if !has_spans {
                default_content = d.content.as_ref();
            }
        }
    }
    let built_in = BUILT_IN.portal_end;
    ResolvedPortalEnd {
        border_t: border_t.map_or(built_in.border_t, BorderPoint::At),
        offset: offset.unwrap_or(built_in.offset),
        content: if has_spans {
            Content::Spans
        } else {
            content_text([own_content, default_content], built_in.content)
        },
    }
}
