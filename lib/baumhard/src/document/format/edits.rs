// SPDX-License-Identifier: MPL-2.0

//! An edit as a file writes it (`format/edits.md`): the definition, its
//! mutations, its advanced mutator and its filter, each spelled with the
//! format's own value types (`document::format::spelling`).
//!
//! **Names, never indices** (amendment 3 on #250). A theme variable is
//! `"var(accent)"`, a palette a name, a font a family name, a verb an id:
//! an App- or User-tier edit is read before any map is open, so it
//! cannot hold a map's index, and an index would name another entry the
//! moment a name was added. `document::edit` turns the names
//! into the open map's indices when the edit runs.
//!
//! **Strict.** Every object denies unknown keys and every enum is
//! externally tagged in `snake_case`, the gate V1-B2 holds the format to
//! (`document::tests::format_strict_tests`). The authored vocabulary is
//! the document's ([`DocMutation`](crate::document::mutation::ops::DocMutation))
//! for every element an edit can reach below a node; the map's own rows
//! (map, theme, palettes, defaults) are out of reach, and a link's kind
//! changes only with the structure an authored edit cannot hold.

use crate::document::element::DocKind;
use crate::document::format::dto::{AlignDto, AnchorDto, DirectionDto, LinkKindDto, LinkTargetDto, ZoomDto};
use crate::document::format::spelling::{
    ColorDto, EdgeDto, FitDto, LineSpanDto, ShapeDto, SidesDto, StartDto, WeightDto,
};
use crate::document::values::FontSize;
use crate::engine::predicate::Comparator;
use serde::{Deserialize, Serialize};

/// One edit definition (design §10.1), as a file writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditDto {
    /// The id keybinds, the console and verbs name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// What it does, for `edit help`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Which nodes, relative to each anchor node.
    pub reach: ReachDto,
    /// Which elements within those nodes.
    pub target: TargetDto,
    /// Which of those elements it keeps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterDto>,
    /// The mutations applied once to each element; one of three bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutations: Option<Vec<MutationDto>>,
    /// The advanced body: a mutator anchored at each element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutator: Option<MutatorDto>,
    /// A handler's name: a layout the mutator language cannot spell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// How it animates into place; instant when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animate: Option<AnimateDto>,
}

/// A file of edits outside a map: the bundled `assets/edits.json` and the
/// user's `edits.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditsFileDto {
    /// The edits, in the file's order.
    pub edits: Vec<EditDto>,
}

/// `reach`: which nodes, relative to each anchor node.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReachDto {
    /// `"self"`.
    #[serde(rename = "self")]
    SelfNode,
    /// `"children"`.
    Children,
    /// `"descendants"`.
    Descendants,
    /// `"subtree"`.
    Subtree,
    /// `"parent"`.
    Parent,
    /// `"siblings"`.
    Siblings,
}

/// `target`: which elements within the reached nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetDto {
    /// `"node"`.
    Node,
    /// `"sections"`.
    Sections,
    /// `"spans"`: every span of the nodes' sections.
    Spans,
    /// `"frame"`.
    Frame,
    /// `"rings"`: every ring of the nodes' frames.
    Rings,
    /// `"connector"`.
    Connector,
    /// `"links"`: the nodes' outgoing links.
    Links,
    /// `"labels"`: the labels of the nodes' connectors and links.
    Labels,
}

/// `animate`: `{ "ms": 200, "delay_ms": 0, "easing": "ease_out", "snap": "end" }`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnimateDto {
    /// How long the change takes.
    pub ms: u32,
    /// How long it holds at its start first.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u32,
    /// How progress follows time; linear when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<EasingDto>,
    /// When the fields that do not blend switch; `end` when absent
    /// (design §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<SnapDto>,
}

/// Whether a delay is the default, none.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// `easing`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EasingDto {
    /// `"linear"`.
    Linear,
    /// `"ease_in"`.
    EaseIn,
    /// `"ease_out"`.
    EaseOut,
    /// `"ease_in_out"`.
    EaseInOut,
}

/// `snap`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SnapDto {
    /// `"start"`: they switch as the change begins.
    Start,
    /// `"end"`: they switch as it ends.
    End,
}

/// A predicate as a file writes it: `{ "fields": [[{"font_size": {"pt": 12}}, {"GreaterThan": false}]] }`.
/// The generic predicate's shape (V1-A1), with the file's values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilterDto {
    /// The field tests, the first one an element answers deciding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<(FieldDto, ComparatorDto)>,
    /// Whether it matches every element.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub always_match: bool,
}

/// A predicate field (`format/mutations.md`, "The document's kinds and
/// predicate fields"), with the file's values: names where the in-memory
/// field holds an index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FieldDto {
    /// The element's kind.
    Kind(KindDto),
    /// The map's background; never a palette color.
    Background(ColorDto<'static>),
    /// The element's channel.
    Channel(u64),
    /// A node's id.
    Ident(u32),
    /// A node's horizontal position.
    PositionX(f32),
    /// A node's vertical position.
    PositionY(f32),
    /// A node's or section's minimum width.
    MinWidth(f32),
    /// A node's or section's minimum height.
    MinHeight(f32),
    /// A node's shape.
    Shape(ShapeDto),
    /// A node's palette, by name.
    Palette(String),
    /// Whether a node is folded.
    Folded(bool),
    /// Whether a node is hidden.
    Hidden(bool),
    /// A node's opacity.
    Opacity(f32),
    /// The verb a node's click runs, by id.
    OnClick(String),
    /// The verb hovering a node runs, by id.
    OnHover(String),
    /// The lowest zoom an element renders at.
    ZoomMin(f32),
    /// The highest zoom an element renders at.
    ZoomMax(f32),
    /// A section's horizontal offset.
    OffsetX(f32),
    /// A section's vertical offset.
    OffsetY(f32),
    /// A section's alignment.
    Align(AlignDto),
    /// A text style's face, by family name.
    Font(String),
    /// A text size, with its unit: `{"pt": 12}` or `{"rel": 0.1}`.
    FontSize(FontSizeDto),
    /// A text color.
    Color(ColorDto<'static>),
    /// A text weight.
    Weight(WeightDto),
    /// Whether a text style is italic.
    Italic(bool),
    /// Whether a frame, connector or link is drawn.
    Visible(bool),
    /// The side a frame's text starts on.
    StartEdge(EdgeDto),
    /// How far along that side.
    StartAt(f32),
    /// Which way a frame's text runs.
    Direction(DirectionDto),
    /// A fit mode.
    Fit(FitDto<'static>),
    /// A frame's ring gap.
    Gap(f32),
    /// A frame's inset.
    Inset(f32),
    /// A path's or ring's spacing.
    Spacing(f32),
    /// Where a path attaches at its start.
    AnchorFrom(AnchorDto),
    /// Where a path attaches at its end.
    AnchorTo(AnchorDto),
    /// Whether a link is a path or a portal.
    LinkKind(LinkKindDto),
    /// A label's position along its path.
    LabelT(f32),
    /// A label's or portal end's offset.
    Offset(f32),
    /// A portal end's position around its node.
    BorderT(f32),
}

/// The advanced body: the V1-A1 mutator AST, as an edit can author it.
/// It has no runtime holes — a template, a runtime count or a runtime
/// mutation needs a build context an edit does not have — so `single`
/// and `repeat`, which exist only to carry them, are not spelled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MutatorDto {
    /// Routing only: descend where a child's key matches.
    Void {
        /// The kind half of the key.
        kind: KindDto,
        /// The channel half of the key.
        channel: u64,
        /// What applies below.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<MutatorDto>,
    },
    /// Mutations applied to the element the key reaches.
    Macro {
        /// The kind half of the key.
        kind: KindDto,
        /// The channel half of the key.
        channel: u64,
        /// The mutations, in order.
        mutations: Vec<MutationDto>,
        /// What applies below.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<MutatorDto>,
    },
    /// A walk the instruction drives.
    Instruction {
        /// The kind half of the key.
        kind: KindDto,
        /// The channel half of the key.
        channel: u64,
        /// What drives the walk.
        instruction: InstructionDto,
        /// What applies at each element the walk visits.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<MutatorDto>,
    },
}

/// What drives an instruction's walk. A document walk takes neither a
/// spatial descent (a document element is not spatial) nor a positional
/// pairing of children (the validator refuses it), so neither is
/// spelled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstructionDto {
    /// Repeat down the tree for as long as the predicate holds.
    RepeatWhile(FilterDto),
    /// Repeat down the tree everywhere.
    RepeatWhileAlwaysTrue,
}

/// A mutation as an edit writes it: the payload's element, then its op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MutationDto {
    /// A node.
    Node(NodeOpDto),
    /// A section.
    Section(SectionOpDto),
    /// A span.
    Span(SpanOpDto),
    /// A frame.
    Frame(FrameOpDto),
    /// A ring.
    Ring(RingOpDto),
    /// A connector or a link: its path.
    Path(PathOpDto),
    /// A cap's text style.
    Cap(TextOpDto),
    /// A label.
    Label(LabelOpDto),
    /// A portal end.
    PortalEnd(PortalEndOpDto),
}

/// A text style op. A size is a number in the unit the element's place
/// stores (`format/schema.md`): points, or a fraction of the node inside
/// a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TextOpDto {
    /// Set the face by family name; `null` inherits.
    SetFont(Option<String>),
    /// Set the size; `null` inherits.
    SetSize(Option<f32>),
    /// Add to the size displayed.
    GrowSize(f32),
    /// Multiply the size displayed.
    ScaleSize(f32),
    /// Set the color; `null` inherits.
    SetColor(Option<ColorDto<'static>>),
    /// Set the weight; `null` inherits.
    SetWeight(Option<WeightDto>),
    /// Set the slant; `null` inherits.
    SetItalic(Option<bool>),
}

/// A node op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NodeOpDto {
    /// Set the top-left corner, `[x, y]`.
    SetPosition([f32; 2]),
    /// Move the top-left corner by `[dx, dy]`.
    Nudge([f32; 2]),
    /// Set the size the user set; `null` fits the content.
    SetMinSize(Option<[f32; 2]>),
    /// Set the outline; `null` inherits.
    SetShape(Option<ShapeDto>),
    /// Bind a palette by name, or unbind with `null`.
    SetPalette(Option<String>),
    /// Set the zoom range.
    SetZoom(Option<ZoomDto>),
    /// Fold or unfold.
    SetFolded(bool),
    /// Flip folded.
    ToggleFolded,
    /// Hide or show.
    SetHidden(bool),
    /// Set the opacity.
    SetOpacity(f32),
    /// Set the verb a click runs, by id.
    SetClick(Option<String>),
    /// Set the verb hovering runs, by id.
    SetHover(Option<String>),
    /// Set the channel.
    SetChannel(u64),
    /// Change the text style.
    Text(TextOpDto),
}

/// A section op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SectionOpDto {
    /// Set the offset from the node, `[x, y]`.
    SetOffset([f32; 2]),
    /// Move the offset by `[dx, dy]`.
    Nudge([f32; 2]),
    /// Set the size the user set; `null` fits the content.
    SetMinSize(Option<[f32; 2]>),
    /// Set the alignment; `null` inherits.
    SetAlign(Option<AlignDto>),
    /// Set the channel.
    SetChannel(u64),
    /// Change the text style.
    Text(TextOpDto),
}

/// A span op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SpanOpDto {
    /// Replace the text.
    SetText(String),
    /// Insert text before a grapheme cluster.
    InsertText {
        /// The cluster index.
        at: usize,
        /// The text.
        text: String,
    },
    /// Delete grapheme clusters `start..end`.
    DeleteText {
        /// The first cluster deleted.
        start: usize,
        /// The cluster after the last.
        end: usize,
    },
    /// Set where it leads; `null` for nowhere.
    SetLink(Option<LinkTargetDto<'static>>),
    /// Change the text style.
    Text(TextOpDto),
}

/// A frame op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrameOpDto {
    /// Show or hide; `null` inherits.
    SetVisible(Option<bool>),
    /// Set where the text starts; `null` inherits.
    SetStart(Option<StartDto>),
    /// Set which way it runs; `null` inherits.
    SetDirection(Option<DirectionDto>),
    /// Set the rings' fit; `null` inherits.
    SetFit(Option<FitDto<'static>>),
    /// Set the ring gap; `null` inherits.
    SetGap(Option<f32>),
    /// Set the inset; `null` inherits.
    SetInset(Option<f32>),
    /// Change how its rings are laid.
    Layout(LayoutOpDto),
    /// Change the text style.
    Text(TextOpDto),
}

/// A ring layout op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    clippy::enum_variant_names,
    reason = "the ops are named as the document's `LayoutOp` names them, and as the file spells them: `set_span`, …"
)]
pub(crate) enum LayoutOpDto {
    /// Set the line span; `null` inherits.
    SetSpan(Option<LineSpanDto>),
    /// Set the sides ornament; `null` inherits.
    SetSides(Option<SidesDto<'static>>),
    /// Set the repeat spacing; `null` inherits.
    SetSpacing(Option<f32>),
}

/// A ring op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RingOpDto {
    /// Override the fit; `null` inherits.
    SetFit(Option<FitDto<'static>>),
    /// Override the layout.
    Layout(LayoutOpDto),
    /// Set the channel.
    SetChannel(u64),
    /// Change the text style.
    Text(TextOpDto),
}

/// A path op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PathOpDto {
    /// Set the fit; `null` inherits.
    SetFit(Option<FitDto<'static>>),
    /// Set the spacing; `null` inherits.
    SetSpacing(Option<f32>),
    /// Set the text size in points; `null` inherits.
    SetSize(Option<f32>),
    /// Set the text color; `null` inherits.
    SetColor(Option<ColorDto<'static>>),
    /// Set the content; `null` inherits.
    SetContent(Option<String>),
    /// Set both anchors, `[start, end]`.
    SetAnchors(AnchorDto, AnchorDto),
    /// Replace the control points.
    SetControlPoints(Vec<[f32; 2]>),
    /// Move one control point.
    MoveControlPoint {
        /// Which point.
        index: usize,
        /// Where to.
        to: [f32; 2],
    },
    /// Show or hide.
    SetVisible(bool),
    /// Set the zoom range.
    SetZoom(Option<ZoomDto>),
}

/// A label op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LabelOpDto {
    /// Set the position along the path; `null` inherits.
    SetT(Option<f32>),
    /// Set the distance from the path; `null` inherits.
    SetOffset(Option<f32>),
    /// Change the text style.
    Text(TextOpDto),
}

/// A portal end op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PortalEndOpDto {
    /// Set the position around the outline; `null` inherits.
    SetBorderT(Option<f32>),
    /// Set the distance from the outline; `null` inherits.
    SetOffset(Option<f32>),
    /// Set the marker color; `null` inherits.
    SetColor(Option<ColorDto<'static>>),
    /// Set the marker size in points; `null` inherits.
    SetSize(Option<f32>),
    /// Set the marker; `null` inherits.
    SetContent(Option<String>),
}

/// A comparator: the generic predicate's own spelling (V1-A1),
/// `{"Equals": false}` and so on, the flag negating.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ComparatorDto {
    /// `==`, or `!=` negated.
    Equals(bool),
    /// The field is set, or unset negated.
    Exists(bool),
    /// `>`, or `<=` negated.
    GreaterThan(bool),
    /// `<`, or `>=` negated.
    LessThan(bool),
}

impl From<ComparatorDto> for Comparator {
    fn from(dto: ComparatorDto) -> Self {
        match dto {
            ComparatorDto::Equals(negated) => Comparator::Equals(negated),
            ComparatorDto::Exists(negated) => Comparator::Exists(negated),
            ComparatorDto::GreaterThan(negated) => Comparator::GreaterThan(negated),
            ComparatorDto::LessThan(negated) => Comparator::LessThan(negated),
        }
    }
}

/// A text size with its unit: `{"pt": 12}` or `{"rel": 0.1}`.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FontSizeDto {
    /// In points.
    Pt(f32),
    /// A fraction of the node's shorter side.
    Rel(f32),
}

impl From<FontSizeDto> for FontSize {
    fn from(dto: FontSizeDto) -> Self {
        match dto {
            FontSizeDto::Pt(size) => FontSize::Pt(size),
            FontSizeDto::Rel(size) => FontSize::Rel(size),
        }
    }
}

/// An element's kind, by its name (`format/mutations.md`, "Kinds").
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum KindDto {
    /// `"map"`.
    Map,
    /// `"theme"`.
    Theme,
    /// `"palette"`.
    Palette,
    /// `"defaults"`.
    Defaults,
    /// `"section"`.
    Section,
    /// `"span"`.
    Span,
    /// `"frame"`.
    Frame,
    /// `"ring"`.
    Ring,
    /// `"connector"`.
    Connector,
    /// `"cap_start"`.
    CapStart,
    /// `"cap_end"`.
    CapEnd,
    /// `"label"`.
    Label,
    /// `"link"`.
    Link,
    /// `"portal_end"`.
    PortalEnd,
    /// `"node"`.
    Node,
}

spelled!(
    KindDto,
    "an element kind: `node`, `section`, `span`, `frame`, `ring`, `connector`, `link`, `label`, …",
    [
        ("map", KindDto::Map),
        ("theme", KindDto::Theme),
        ("palette", KindDto::Palette),
        ("defaults", KindDto::Defaults),
        ("section", KindDto::Section),
        ("span", KindDto::Span),
        ("frame", KindDto::Frame),
        ("ring", KindDto::Ring),
        ("connector", KindDto::Connector),
        ("cap_start", KindDto::CapStart),
        ("cap_end", KindDto::CapEnd),
        ("label", KindDto::Label),
        ("link", KindDto::Link),
        ("portal_end", KindDto::PortalEnd),
        ("node", KindDto::Node),
    ]
);

impl From<KindDto> for DocKind {
    fn from(dto: KindDto) -> Self {
        match dto {
            KindDto::Map => DocKind::Map,
            KindDto::Theme => DocKind::Theme,
            KindDto::Palette => DocKind::Palette,
            KindDto::Defaults => DocKind::Defaults,
            KindDto::Section => DocKind::Section,
            KindDto::Span => DocKind::Span,
            KindDto::Frame => DocKind::Frame,
            KindDto::Ring => DocKind::Ring,
            KindDto::Connector => DocKind::Connector,
            KindDto::CapStart => DocKind::CapStart,
            KindDto::CapEnd => DocKind::CapEnd,
            KindDto::Label => DocKind::Label,
            KindDto::Link => DocKind::Link,
            KindDto::PortalEnd => DocKind::PortalEnd,
            KindDto::Node => DocKind::Node,
        }
    }
}

impl MutationDto {
    /// The kinds this payload applies to, as
    /// [`DocMutation::applies_to`](crate::document::mutation::ops::DocMutation::applies_to)
    /// has them. O(1).
    pub(crate) fn applies_to(&self, kind: DocKind) -> bool {
        use DocKind as K;
        matches!(
            (self, kind),
            (MutationDto::Node(_), K::Node)
                | (MutationDto::Section(_), K::Section)
                | (MutationDto::Span(_), K::Span)
                | (MutationDto::Frame(_), K::Frame)
                | (MutationDto::Ring(_), K::Ring)
                | (MutationDto::Path(_), K::Connector | K::Link)
                | (MutationDto::Cap(_), K::CapStart | K::CapEnd)
                | (MutationDto::Label(_), K::Label)
                | (MutationDto::PortalEnd(_), K::PortalEnd)
        )
    }
}
