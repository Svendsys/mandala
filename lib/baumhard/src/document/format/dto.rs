// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` shape as Rust types: one struct per JSON object,
//! its fields in the order the schema (`format/schema.md`) lists its
//! keys, which is the order the saver writes them.
//!
//! **Every struct is `#[serde(deny_unknown_fields)]`**, and the serde
//! type graph in `util` walks this module's types to hold the format to
//! it (an unknown key is a load error, never preserved or skipped).
//! Nothing here uses `flatten` or `untagged`, which would hide a key
//! from that attribute; so the five text-style keys (`font`, `size`,
//! `color`, `weight`, `italic`) are written out on every struct that
//! takes them, rather than shared.
//!
//! **Every optional key is an `Option` and absent means "not set"**
//! (inherit); `null` is not a way to write it
//! ([`present`]). The keys
//! that have a default instead of an inherited value (`channel`,
//! `folded`, `hidden`, `opacity`, a section's `offset`) are omitted by
//! the saver when they equal it.
//!
//! **A DTO holds names, not indices**: a theme variable, a palette, a
//! font and a verb are the strings the file spells; the loader resolves
//! them (design §7.2) and the saver spells them again. The DTO names
//! carry a `Dto` suffix because the serde type graph indexes types by
//! bare name, and the document's own types, the model's and the
//! primitives' already hold the plain ones.
//!
//! These are plain data. Nothing here checks a bound or a reference:
//! that is [`load`](crate::document::format::load)'s, which can name
//! the path of what it refuses.

use crate::document::format::rows::Rows;
use crate::document::format::spelling::{
    present, ColorDto, FitDto, HexDto, Named, ShapeDto, StartDto, WeightDto,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Whether a channel is the default, `0`.
pub(crate) fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Whether a flag is the default, `false`.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

/// Whether an opacity is the default, `1.0`.
pub(crate) fn is_opaque(value: &f32) -> bool {
    *value == 1.0
}

/// The default opacity, for `#[serde(default = ...)]`.
pub(crate) fn opaque() -> f32 {
    1.0
}

/// Whether an offset is the default, `[0, 0]`.
pub(crate) fn is_origin(value: &[f32; 2]) -> bool {
    *value == [0.0, 0.0]
}

/// An alignment: `"left"`, `"center"` or `"right"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AlignDto {
    /// `"left"`.
    Left,
    /// `"center"`.
    Center,
    /// `"right"`.
    Right,
}

spelled!(
    AlignDto,
    "an alignment: `left`, `center` or `right`",
    [
        ("left", AlignDto::Left),
        ("center", AlignDto::Center),
        ("right", AlignDto::Right),
    ]
);

/// Where a path attaches to a node: `"auto"`, `"top"`, `"right"`,
/// `"bottom"` or `"left"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnchorDto {
    /// `"auto"`.
    Auto,
    /// `"top"`.
    Top,
    /// `"right"`.
    Right,
    /// `"bottom"`.
    Bottom,
    /// `"left"`.
    Left,
}

spelled!(
    AnchorDto,
    "an anchor: `auto`, `top`, `right`, `bottom` or `left`",
    [
        ("auto", AnchorDto::Auto),
        ("top", AnchorDto::Top),
        ("right", AnchorDto::Right),
        ("bottom", AnchorDto::Bottom),
        ("left", AnchorDto::Left),
    ]
);

/// Which way a frame's text runs: `"clockwise"` or `"counterclockwise"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum DirectionDto {
    /// `"clockwise"`.
    Clockwise,
    /// `"counterclockwise"`.
    Counterclockwise,
}

spelled!(
    DirectionDto,
    "a direction: `clockwise` or `counterclockwise`",
    [
        ("clockwise", DirectionDto::Clockwise),
        ("counterclockwise", DirectionDto::Counterclockwise),
    ]
);

/// What a link draws: `"line"` or `"portal"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkKindDto {
    /// `"line"`.
    Line,
    /// `"portal"`.
    Portal,
}

spelled!(
    LinkKindDto,
    "a link kind: `line` or `portal`",
    [("line", LinkKindDto::Line), ("portal", LinkKindDto::Portal)]
);

/// Where a span leads: `{ "url": "…" }` or `{ "verb": "verb-id" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LinkTargetDto<'a> {
    /// `{ "url": "…" }`.
    Url(Cow<'a, str>),
    /// `{ "verb": "verb-id" }`.
    Verb(Cow<'a, str>),
}

/// The zoom range an element renders in: `{ "min": n, "max": n }`, either bound optional.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZoomDto {
    /// The lowest camera zoom it renders at.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub min: Option<f32>,
    /// The highest camera zoom it renders at.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub max: Option<f32>,
}

/// A node's triggers: `{ "click": "verb-id", "hover": "verb-id" }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OnDto<'a> {
    /// The verb a click runs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub click: Option<Cow<'a, str>>,
    /// The verb hovering runs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub hover: Option<Cow<'a, str>>,
}

/// An edit definition. Only its identity for now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditDto {
    /// The id the file names it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// A presentation definition. Only its identity for now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationDto {
    /// The id the file names it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// A verb definition. Only its identity for now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerbDto {
    /// The id the file names it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// The theme: `{ "active": "dark", "variants": { "dark": { "bg": "#101014" } } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThemeDto {
    /// The name of the active variant.
    pub active: String,
    /// The variants, each a map of variable name to literal color.
    pub variants: Named<Named<HexDto>>,
}

/// The colors of one palette depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaletteGroupDto<'a> {
    /// A node's fill.
    pub fill: ColorDto<'a>,
    /// A node's frame text.
    pub frame: ColorDto<'a>,
    /// Section text.
    pub text: ColorDto<'a>,
    /// The text of a node's first section; the group's `text` when absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub title: Option<ColorDto<'a>>,
}

/// `defaults.node`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeDefaultsDto<'a> {
    /// The outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub shape: Option<ShapeDto>,
    /// The fill.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fill: Option<ColorDto<'a>>,
    /// The width of the fill's soft edge.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fill_fade: Option<f32>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
}

/// `defaults.section` and `defaults.span`: text style only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextDefaultsDto<'a> {
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
}

/// `defaults.frame`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameDefaultsDto<'a> {
    /// Whether the frame is drawn.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub visible: Option<bool>,
    /// Where its text starts.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub start: Option<StartDto>,
    /// Which way its text runs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub direction: Option<DirectionDto>,
    /// What its rings do when text and outline differ in length.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// The space between rings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub gap: Option<f32>,
    /// The first ring's distance from the outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub inset: Option<f32>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
}

/// `defaults.ring`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RingDefaultsDto<'a> {
    /// Overrides the frame's fit mode.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
}

/// `defaults.connector` and `defaults.link`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlowDefaultsDto<'a> {
    /// What the text does when it and the path differ in length.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// Extra space between glyphs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub spacing: Option<f32>,
    /// The text size in points.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The text color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The text drawn when the path has no spans.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub content: Option<Cow<'a, str>>,
}

/// `defaults.label`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelDefaultsDto<'a> {
    /// The position along the path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub t: Option<f32>,
    /// The distance from the path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub offset: Option<f32>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
}

/// `defaults.portal_end`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortalEndDefaultsDto<'a> {
    /// The position around the node's outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub border_t: Option<f32>,
    /// The distance from the outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub offset: Option<f32>,
    /// The marker color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The marker size in points.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The marker drawn when the end has no spans.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub content: Option<Cow<'a, str>>,
}

/// The map's defaults: one optional object per kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultsDto<'a> {
    /// Node defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub node: Option<NodeDefaultsDto<'a>>,
    /// Section text defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub section: Option<TextDefaultsDto<'a>>,
    /// Frame defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub frame: Option<FrameDefaultsDto<'a>>,
    /// Ring defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub ring: Option<RingDefaultsDto<'a>>,
    /// Connector defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub connector: Option<FlowDefaultsDto<'a>>,
    /// Line-link defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub link: Option<FlowDefaultsDto<'a>>,
    /// Label defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub label: Option<LabelDefaultsDto<'a>>,
    /// Portal-end defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub portal_end: Option<PortalEndDefaultsDto<'a>>,
    /// Span defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub span: Option<TextDefaultsDto<'a>>,
}

/// A span: text with one style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanDto<'a> {
    /// The text.
    pub text: Cow<'a, str>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Where it leads when clicked.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub link: Option<LinkTargetDto<'a>>,
}

/// A section: a box of spans inside a node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SectionDto<'a> {
    /// The channel among siblings of its kind.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channel: u64,
    /// The top-left corner, relative to the node's.
    #[serde(default, skip_serializing_if = "is_origin")]
    pub offset: [f32; 2],
    /// The size the user set, a minimum.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub min_size: Option<[f32; 2]>,
    /// How the lines align in the box.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub align: Option<AlignDto>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
}

/// One ring of a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RingDto<'a> {
    /// The channel among siblings of its kind.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channel: u64,
    /// Overrides the frame's fit mode.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
}

/// A node's frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameDto<'a> {
    /// Whether the frame is drawn.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub visible: Option<bool>,
    /// Where its text starts.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub start: Option<StartDto>,
    /// Which way its text runs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub direction: Option<DirectionDto>,
    /// What its rings do when text and outline differ in length.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// The space between rings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub gap: Option<f32>,
    /// The first ring's distance from the outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub inset: Option<f32>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its rings, innermost first.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub rings: Rows<'a, RingDto<'a>>,
}

/// A path's start or end cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapDto<'a> {
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
}

/// A label on a path or beside a portal end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelDto<'a> {
    /// The position along the path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub t: Option<f32>,
    /// The distance from the path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub offset: Option<f32>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
}

/// One end of a portal link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortalEndDto<'a> {
    /// The position around the node's outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub border_t: Option<f32>,
    /// The distance from the outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub offset: Option<f32>,
    /// The marker color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The marker size in points.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The marker drawn when the end has no spans.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub content: Option<Cow<'a, str>>,
    /// Its spans, the marker.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
    /// Its label.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub label: Option<LabelDto<'a>>,
}

/// A node's connector: the line to its parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathDto<'a> {
    /// What the text does when it and the path differ in length.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// Extra space between glyphs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub spacing: Option<f32>,
    /// The text size in points.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The text color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The text drawn when the path has no spans.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub content: Option<Cow<'a, str>>,
    /// Where it attaches at its start and at its end.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub anchors: Option<[AnchorDto; 2]>,
    /// Bézier control points: 0, 1 or 2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_points: Vec<[f32; 2]>,
    /// Whether it is drawn.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub visible: Option<bool>,
    /// The zoom range it renders in.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub zoom: Option<ZoomDto>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
    /// The glyphs at its start.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub cap_start: Option<CapDto<'a>>,
    /// The glyphs at its end.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub cap_end: Option<CapDto<'a>>,
    /// Its label.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub label: Option<LabelDto<'a>>,
}

/// A link to another node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkDto<'a> {
    /// The `id` of the node it leads to, anywhere in the list. Read
    /// wider than an id may be, as a node's `id` is.
    pub to: u64,
    /// Whether it draws a path or two portal ends.
    pub kind: LinkKindDto,
    /// What the text does when it and the path differ in length.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fit: Option<FitDto<'a>>,
    /// Extra space between glyphs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub spacing: Option<f32>,
    /// The text size in points.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The text color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The text drawn when the path has no spans.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub content: Option<Cow<'a, str>>,
    /// Where it attaches at its start and at its end.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub anchors: Option<[AnchorDto; 2]>,
    /// Bézier control points: 0, 1 or 2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_points: Vec<[f32; 2]>,
    /// Whether it is drawn.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub visible: Option<bool>,
    /// The zoom range it renders in.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub zoom: Option<ZoomDto>,
    /// Its spans.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub spans: Rows<'a, SpanDto<'a>>,
    /// The glyphs at its start.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub cap_start: Option<CapDto<'a>>,
    /// The glyphs at its end.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub cap_end: Option<CapDto<'a>>,
    /// Its label.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub label: Option<LabelDto<'a>>,
    /// The two ends of a portal link: the source end, then the target end.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub ends: Option<[PortalEndDto<'a>; 2]>,
}

/// A node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeDto<'a> {
    /// The id links and verbs name it by, unique in the map: an integer
    /// in `1..=4294967294`. Read wider than it may be, so that a value
    /// out of range is a bound error naming the bound rather than the
    /// reader's message about a `u32`.
    pub id: u64,
    /// The `id` of the node it hangs from, which comes before it in the
    /// list and is on the chain of the node listed just before it
    /// (design B.4). Absent on a top-level node.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub parent: Option<u64>,
    /// The channel among its sibling nodes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channel: u64,
    /// The top-left corner.
    pub position: [f32; 2],
    /// The size the user set, a minimum.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub min_size: Option<[f32; 2]>,
    /// The outline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub shape: Option<ShapeDto>,
    /// The background fill.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fill: Option<ColorDto<'a>>,
    /// The width of the fill's soft edge.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub fill_fade: Option<f32>,
    /// The name of the palette that colors it and its descendants.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub palette: Option<Cow<'a, str>>,
    /// The zoom range it renders in.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub zoom: Option<ZoomDto>,
    /// Whether its descendants are collapsed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub folded: bool,
    /// Whether it, its subtree and the links touching them are left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Multiplies the alpha of everything it and its subtree draw.
    #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
    pub opacity: f32,
    /// The verbs its click and hover run.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub on: Option<OnDto<'a>>,
    /// The font family name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub font: Option<Cow<'a, str>>,
    /// The size: points, or a fraction of the node's shorter side inside a frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub size: Option<f32>,
    /// The color.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub color: Option<ColorDto<'a>>,
    /// The weight.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub weight: Option<WeightDto>,
    /// Whether the text is italic.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub italic: Option<bool>,
    /// Its sections.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub sections: Rows<'a, SectionDto<'a>>,
    /// Its frame.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub frame: Option<FrameDto<'a>>,
    /// The line to its parent; absent means a connector with every field inherited.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub connector: Option<PathDto<'a>>,
    /// Its links.
    #[serde(borrow, default, skip_serializing_if = "Rows::is_empty")]
    pub links: Rows<'a, LinkDto<'a>>,
}

/// Every top-level key of a file but `nodes`: the header, which the
/// loader reads before any node and the saver writes before the node
/// list. The nodes are never held as a list of DTOs: the loader parses
/// and builds one at a time from its extent, and the saver writes one
/// at a time as its walk reaches it (design B.14, B.15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapHeaderDto<'a> {
    /// The format version: `1`.
    pub format: u64,
    /// The map's name.
    pub name: String,
    /// The theme. Absent is the empty theme (no variables, no variants),
    /// which has no other spelling; a theme that is present has at least
    /// one variant.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub theme: Option<ThemeDto>,
    /// The palettes, by name.
    #[serde(default, skip_serializing_if = "Named::is_empty")]
    pub palettes: Named<Vec<PaletteGroupDto<'a>>>,
    /// The map's defaults.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub defaults: Option<DefaultsDto<'a>>,
    /// The edits the map defines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<EditDto>,
    /// The presentations the map defines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presentations: Vec<PresentationDto>,
    /// The verbs the map defines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbs: Vec<VerbDto>,
}
