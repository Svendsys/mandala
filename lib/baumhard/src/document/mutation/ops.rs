// SPDX-License-Identifier: MPL-2.0

//! [`DocMutation`](crate::document::mutation::ops::DocMutation): every
//! change a document element can undergo, one payload per kind of
//! element, and how each one applies.
//!
//! **Each op writes exactly the field it names.** There is no setter
//! beside these ops and no op that writes a value the document derives
//! (a fitted size, a depth, a resolved color). Literal values are
//! checked before anything applies
//! ([`validate`](crate::document::mutation::validate::validate)); the
//! relative ops — `Nudge`, `GrowSize`, `ScaleSize`, `MoveControlPoint`
//! — compute their result from the element and clamp it with
//! [`document::bounds`](crate::document::bounds), so applying never
//! produces a value out of bounds.
//!
//! **A payload reaches only its own kind.** The walker routes by kind
//! and the validator refuses a payload whose variant does not match its
//! mutator's key, so a mismatch inside
//! [`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to) is an
//! engine defect: it changes nothing, logs an error and trips a
//! `debug_assert!`.
//!
//! **An index the reached element does not have** — a palette group, a
//! theme variant or variable, a control point — leaves the element
//! unchanged and logs a `debug!` (design §2 rule 8, ruling R10 on
//! #237): the validator checks an index against the document, but not
//! against the one element a mutator reaches, which only the walk
//! knows.

use crate::document::bounds::{clamp_canvas_coord, clamp_font_size_pt, clamp_rel_size};
use crate::document::element::{DefaultsData, DocElement, DocKind, PaletteGroup, TextStyle};
use crate::document::mutation::inherited::DocInherited;
use crate::document::values::{
    Align, Anchor, ColorRef, Direction, FitMode, FontSize, LinkKind, LinkTarget, PaletteId, Rgba, Shape,
    StartPoint, VarId, VerbId, Weight, ZoomWindow,
};
use crate::engine::mutation::Mutation;
use crate::engine::tree::MutatorTree;
use crate::font::fonts::AppFont;
use crate::util::grapheme_chad::{delete_grapheme_range, insert_str_at_grapheme};
use glam::Vec2;
use log::{debug, error};

/// A mutator tree over the document: what an edit, a presentation in
/// document terms, and every composite builder produce.
pub type DocMutatorTree = MutatorTree<DocElement, DocMutation>;

/// A change to one document element. Each variant names the kinds it
/// applies to ([`DocMutation::applies_to`]); its op names the field.
#[derive(Clone, Debug, PartialEq)]
pub enum DocMutation {
    /// The [`Map`](DocKind::Map).
    Map(MapOp),
    /// The [`Theme`](DocKind::Theme).
    Theme(ThemeOp),
    /// A [`Palette`](DocKind::Palette).
    Palette(PaletteOp),
    /// The [`Defaults`](DocKind::Defaults).
    Defaults(DefaultsOp),
    /// A [`Node`](DocKind::Node).
    Node(NodeOp),
    /// A [`Section`](DocKind::Section).
    Section(SectionOp),
    /// A [`Span`](DocKind::Span).
    Span(SpanOp),
    /// A [`Frame`](DocKind::Frame).
    Frame(FrameOp),
    /// A [`Ring`](DocKind::Ring).
    Ring(RingOp),
    /// A path: a [`Connector`](DocKind::Connector) or a
    /// [`Link`](DocKind::Link).
    Path(PathOp),
    /// A [`Link`](DocKind::Link)'s own fields.
    Link(LinkOp),
    /// A cap's text style: a [`CapStart`](DocKind::CapStart) or a
    /// [`CapEnd`](DocKind::CapEnd).
    Cap(TextOp),
    /// A [`Label`](DocKind::Label).
    Label(LabelOp),
    /// A [`PortalEnd`](DocKind::PortalEnd).
    PortalEnd(PortalEndOp),
}

/// A change to a [`TextStyle`]: the text fields of a node, section,
/// span, frame, ring, cap or label, each reached through its owner's
/// payload (`Node(Text(..))`, `Cap(..)` and so on).
#[derive(Clone, Debug, PartialEq)]
pub enum TextOp {
    /// Set the face; `None` inherits.
    SetFont(Option<AppFont>),
    /// Set the size; `None` inherits. A ring's and a frame's size is
    /// [`Rel`](FontSize::Rel), everything else's [`Pt`](FontSize::Pt)
    /// (ruling R10 on #237).
    SetSize(Option<FontSize>),
    /// Add to the size the element displays — its own, or the one it
    /// inherits when it sets none ([`DocInherited::text_size_for`]) —
    /// and store the result, clamped, as its own: in
    /// [`Rel`](FontSize::Rel) inside a frame, in [`Pt`](FontSize::Pt)
    /// elsewhere.
    ///
    /// **The delta is in the unit stored at the element's place:**
    /// points outside a frame, and inside one a fraction of the node's
    /// shorter side. `GrowSize(2.0)` grows a section by 2 pt, but takes a
    /// ring to the whole side (`Rel(1.0)`, the clamp); a ring grows by a
    /// delta like `0.01`. `ScaleSize` has no unit and reads the same
    /// everywhere.
    GrowSize(f32),
    /// Multiply the size the element displays, with `GrowSize`'s rules.
    /// The factor is a plain ratio, so it means the same in and out of
    /// a frame.
    ScaleSize(f32),
    /// Set the color; `None` inherits.
    SetColor(Option<ColorRef>),
    /// Set the weight; `None` inherits.
    SetWeight(Option<Weight>),
    /// Set the slant; `None` inherits.
    SetItalic(Option<bool>),
}

/// A change to a node.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeOp {
    /// Set the top-left corner.
    SetPosition(Vec2),
    /// Move the top-left corner by a delta; the result is clamped to
    /// the canvas bound.
    Nudge(Vec2),
    /// Set the size the user set; `None` fits the content.
    SetMinSize(Option<Vec2>),
    /// Set the outline; `None` inherits.
    SetShape(Option<Shape>),
    /// Set the fill; `None` inherits.
    SetFill(Option<ColorRef>),
    /// Set the fill's soft edge; `None` inherits.
    SetFillFade(Option<f32>),
    /// Bind a palette, or unbind.
    SetPalette(Option<PaletteId>),
    /// Set the zoom range it renders in.
    SetZoom(Option<ZoomWindow>),
    /// Fold or unfold its descendants.
    SetFolded(bool),
    /// Flip `folded`.
    ToggleFolded,
    /// Hide or show the node and its subtree.
    SetHidden(bool),
    /// Set the opacity that multiplies its subtree's alpha.
    SetOpacity(f32),
    /// Set the verb a click runs.
    SetClick(Option<VerbId>),
    /// Set the verb hovering runs.
    SetHover(Option<VerbId>),
    /// Set its channel among its sibling nodes.
    SetChannel(usize),
    /// Change its text style, which its sections inherit.
    Text(TextOp),
}

/// A change to a section.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionOp {
    /// Set its top-left corner, relative to its node's.
    SetOffset(Vec2),
    /// Move its offset by a delta; the result is clamped to the canvas
    /// bound.
    Nudge(Vec2),
    /// Set the size the user set; `None` fits the content.
    SetMinSize(Option<Vec2>),
    /// Set its alignment; `None` inherits.
    SetAlign(Option<Align>),
    /// Set its channel among the node's sections.
    SetChannel(usize),
    /// Change its text style, which its spans inherit.
    Text(TextOp),
}

/// A change to a span.
#[derive(Clone, Debug, PartialEq)]
pub enum SpanOp {
    /// Replace its text.
    SetText(String),
    /// Insert `text` before grapheme cluster `at`; an `at` past the end
    /// appends.
    InsertText {
        /// A grapheme-cluster index, clamped to the span's length.
        at: usize,
        /// The text to insert.
        text: String,
    },
    /// Delete the grapheme clusters `start..end`, clamped to the span's
    /// length.
    DeleteText {
        /// The first cluster deleted.
        start: usize,
        /// The cluster after the last one deleted.
        end: usize,
    },
    /// Set where it leads when clicked.
    SetLink(Option<LinkTarget>),
    /// Change its style.
    Text(TextOp),
}

/// A change to a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameOp {
    /// Show or hide it; `None` inherits.
    SetVisible(Option<bool>),
    /// Set where its text starts; `None` inherits.
    SetStart(Option<StartPoint>),
    /// Set which way its text runs; `None` inherits.
    SetDirection(Option<Direction>),
    /// Set its rings' fit mode; `None` inherits.
    SetFit(Option<FitMode>),
    /// Set the space between rings; `None` inherits.
    SetGap(Option<f32>),
    /// Set the first ring's distance from the outline; `None` inherits.
    SetInset(Option<f32>),
    /// Change its text style, which its rings inherit.
    Text(TextOp),
}

/// A change to a ring. Its size is its text style's `size`, always
/// [`Rel`](FontSize::Rel), written through `Text` (ruling R10 on #237).
#[derive(Clone, Debug, PartialEq)]
pub enum RingOp {
    /// Override the frame's fit mode; `None` inherits.
    SetFit(Option<FitMode>),
    /// Set its channel among the frame's rings.
    SetChannel(usize),
    /// Change its text style, which its spans inherit.
    Text(TextOp),
}

/// A change to a path: a connector or a link.
#[derive(Clone, Debug, PartialEq)]
pub enum PathOp {
    /// Set its fit mode; `None` inherits.
    SetFit(Option<FitMode>),
    /// Set the extra space between glyphs; `None` inherits.
    SetSpacing(Option<f32>),
    /// Set its text size in points; `None` inherits.
    SetSize(Option<f32>),
    /// Set its text color; `None` inherits.
    SetColor(Option<ColorRef>),
    /// Set the text drawn when it has no spans; `None` inherits.
    SetContent(Option<String>),
    /// Set where it attaches at its start and at its end.
    SetAnchors(Anchor, Anchor),
    /// Replace its control points: 0, 1 or 2.
    SetControlPoints(Vec<Vec2>),
    /// Move one control point; one the path does not have is left
    /// alone.
    MoveControlPoint {
        /// Which point.
        index: usize,
        /// Where to, clamped to the canvas bound.
        to: Vec2,
    },
    /// Show or hide it.
    SetVisible(bool),
    /// Set the zoom range it renders in.
    SetZoom(Option<ZoomWindow>),
}

/// A change to a link's own fields.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkOp {
    /// Switch between a path and a portal. Only together with the
    /// structural commands that give the link the children its new kind
    /// needs ([`set_link_kind`](crate::document::mutation::compose::set_link_kind));
    /// a bare switch is refused by the validator.
    SetKind(LinkKind),
}

/// A change to a label.
#[derive(Clone, Debug, PartialEq)]
pub enum LabelOp {
    /// Set its position along the path; `None` inherits.
    SetT(Option<f32>),
    /// Set its distance from the path; `None` inherits.
    SetOffset(Option<f32>),
    /// Change its text style, which its spans inherit.
    Text(TextOp),
}

/// A change to a portal end.
#[derive(Clone, Debug, PartialEq)]
pub enum PortalEndOp {
    /// Set its position around the outline; `None` inherits.
    SetBorderT(Option<f32>),
    /// Set its distance from the outline; `None` inherits.
    SetOffset(Option<f32>),
    /// Set its marker color; `None` inherits.
    SetColor(Option<ColorRef>),
    /// Set its marker size in points; `None` inherits.
    SetSize(Option<f32>),
    /// Set the marker drawn when it has no spans; `None` inherits.
    SetContent(Option<String>),
}

/// A change to the theme.
#[derive(Clone, Debug, PartialEq)]
pub enum ThemeOp {
    /// Make the variant at this index the active one.
    SetActive(u16),
    /// Set one variable's value in one variant.
    SetValue {
        /// The variant's index.
        variant: u16,
        /// The variable.
        var: VarId,
        /// Its new value.
        value: Rgba,
    },
}

/// A change to a palette.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteOp {
    /// Replace the group at `index`.
    SetGroup {
        /// The group's index: its depth below the bound node.
        index: usize,
        /// Its new colors.
        group: PaletteGroup,
    },
}

/// A change to the map's defaults.
#[derive(Clone, Debug, PartialEq)]
pub enum DefaultsOp {
    /// Replace every default at once.
    Replace(Box<DefaultsData>),
}

/// A change to the map's own data.
#[derive(Clone, Debug, PartialEq)]
pub enum MapOp {
    /// Rename the map.
    SetName(String),
}

impl DocMutation {
    /// Whether this payload applies to an element of `kind`: its own
    /// kind, both path kinds for [`Path`](Self::Path), both caps for
    /// [`Cap`](Self::Cap). O(1).
    pub fn applies_to(&self, kind: DocKind) -> bool {
        use DocKind as K;
        matches!(
            (self, kind),
            (DocMutation::Map(_), K::Map)
                | (DocMutation::Theme(_), K::Theme)
                | (DocMutation::Palette(_), K::Palette)
                | (DocMutation::Defaults(_), K::Defaults)
                | (DocMutation::Node(_), K::Node)
                | (DocMutation::Section(_), K::Section)
                | (DocMutation::Span(_), K::Span)
                | (DocMutation::Frame(_), K::Frame)
                | (DocMutation::Ring(_), K::Ring)
                | (DocMutation::Path(_), K::Connector | K::Link)
                | (DocMutation::Link(_), K::Link)
                | (DocMutation::Cap(_), K::CapStart | K::CapEnd)
                | (DocMutation::Label(_), K::Label)
                | (DocMutation::PortalEnd(_), K::PortalEnd)
        )
    }

    /// The payload's name, for messages. O(1).
    pub fn name(&self) -> &'static str {
        match self {
            DocMutation::Map(_) => "Map",
            DocMutation::Theme(_) => "Theme",
            DocMutation::Palette(_) => "Palette",
            DocMutation::Defaults(_) => "Defaults",
            DocMutation::Node(_) => "Node",
            DocMutation::Section(_) => "Section",
            DocMutation::Span(_) => "Span",
            DocMutation::Frame(_) => "Frame",
            DocMutation::Ring(_) => "Ring",
            DocMutation::Path(_) => "Path",
            DocMutation::Link(_) => "Link",
            DocMutation::Cap(_) => "Cap",
            DocMutation::Label(_) => "Label",
            DocMutation::PortalEnd(_) => "PortalEnd",
        }
    }
}

/// Literal mutations only: the document's mutator trees are built by
/// code — the composite builders, handlers, the writer — never from a
/// template, so the builder's runtime look-up has nothing to resolve.
impl Mutation<DocElement> for DocMutation {
    type Template = ();

    /// Write the field the op names on `target`. `inherited` is the
    /// target's parent's state, read only by `GrowSize` and `ScaleSize`
    /// on an element that sets no size of its own. See the module docs
    /// for a payload of another kind and for an index the element lacks.
    ///
    /// Cost: O(1) per op, except the text edits (one grapheme walk over
    /// the span) and the replacements that clone what they carry.
    fn apply_to(&self, target: &mut DocElement, inherited: &DocInherited) {
        let kind = target.kind();
        match (self, target) {
            (DocMutation::Map(MapOp::SetName(name)), DocElement::Map(map)) => map.name.clone_from(name),
            (DocMutation::Theme(op), DocElement::Theme(theme)) => match op {
                ThemeOp::SetActive(active) => {
                    if usize::from(*active) < theme.variants.len() {
                        theme.active = *active;
                    } else {
                        debug!("document mutation: the theme has no variant {active}");
                    }
                }
                ThemeOp::SetValue { variant, var, value } => {
                    match theme
                        .variants
                        .get_mut(usize::from(*variant))
                        .and_then(|v| v.values.get_mut(usize::from(var.0)))
                    {
                        Some(slot) => *slot = *value,
                        None => debug!(
                            "document mutation: the theme has no value {} in variant {variant}",
                            var.0
                        ),
                    }
                }
            },
            (DocMutation::Palette(PaletteOp::SetGroup { index, group }), DocElement::Palette(palette)) => {
                match palette.groups.get_mut(*index) {
                    Some(slot) => *slot = *group,
                    None => debug!(
                        "document mutation: palette {:?} has no group {index}",
                        palette.name
                    ),
                }
            }
            (DocMutation::Defaults(DefaultsOp::Replace(defaults)), DocElement::Defaults(slot)) => {
                slot.clone_from(defaults)
            }
            (DocMutation::Node(op), DocElement::Node(node)) => match op {
                NodeOp::SetPosition(position) => node.position = *position,
                NodeOp::Nudge(delta) => node.position = nudge(node.position, *delta),
                NodeOp::SetMinSize(size) => node.min_size = *size,
                NodeOp::SetShape(shape) => node.style.shape = *shape,
                NodeOp::SetFill(fill) => node.style.fill = *fill,
                NodeOp::SetFillFade(fade) => node.style.fill_fade = *fade,
                NodeOp::SetPalette(palette) => node.palette = *palette,
                NodeOp::SetZoom(zoom) => node.zoom = *zoom,
                NodeOp::SetFolded(folded) => node.folded = *folded,
                NodeOp::ToggleFolded => node.folded = !node.folded,
                NodeOp::SetHidden(hidden) => node.hidden = *hidden,
                NodeOp::SetOpacity(opacity) => node.opacity = *opacity,
                NodeOp::SetClick(verb) => node.on.click = *verb,
                NodeOp::SetHover(verb) => node.on.hover = *verb,
                NodeOp::SetChannel(channel) => node.channel = *channel,
                NodeOp::Text(op) => apply_text(op, &mut node.style.text, kind, inherited),
            },
            (DocMutation::Section(op), DocElement::Section(section)) => match op {
                SectionOp::SetOffset(offset) => section.offset = *offset,
                SectionOp::Nudge(delta) => section.offset = nudge(section.offset, *delta),
                SectionOp::SetMinSize(size) => section.min_size = *size,
                SectionOp::SetAlign(align) => section.align = *align,
                SectionOp::SetChannel(channel) => section.channel = *channel,
                SectionOp::Text(op) => apply_text(op, &mut section.text, kind, inherited),
            },
            (DocMutation::Span(op), DocElement::Span(span)) => match op {
                SpanOp::SetText(text) => span.text.clone_from(text),
                SpanOp::InsertText { at, text } => insert_str_at_grapheme(&mut span.text, *at, text),
                SpanOp::DeleteText { start, end } => delete_grapheme_range(&mut span.text, *start, *end),
                SpanOp::SetLink(link) => span.link.clone_from(link),
                SpanOp::Text(op) => apply_text(op, &mut span.style, kind, inherited),
            },
            (DocMutation::Frame(op), DocElement::Frame(frame)) => match op {
                FrameOp::SetVisible(visible) => frame.style.visible = *visible,
                FrameOp::SetStart(start) => frame.style.start = *start,
                FrameOp::SetDirection(direction) => frame.style.direction = *direction,
                FrameOp::SetFit(fit) => frame.style.fit.clone_from(fit),
                FrameOp::SetGap(gap) => frame.style.gap = *gap,
                FrameOp::SetInset(inset) => frame.style.inset = *inset,
                FrameOp::Text(op) => apply_text(op, &mut frame.style.text, kind, inherited),
            },
            (DocMutation::Ring(op), DocElement::Ring(ring)) => match op {
                RingOp::SetFit(fit) => ring.style.fit.clone_from(fit),
                RingOp::SetChannel(channel) => ring.channel = *channel,
                RingOp::Text(op) => apply_text(op, &mut ring.style.text, kind, inherited),
            },
            (DocMutation::Path(op), DocElement::Connector(connector)) => apply_path(op, &mut connector.path),
            (DocMutation::Path(op), DocElement::Link(link)) => apply_path(op, &mut link.path),
            (DocMutation::Link(LinkOp::SetKind(link_kind)), DocElement::Link(link)) => link.kind = *link_kind,
            (DocMutation::Cap(op), DocElement::CapStart(cap) | DocElement::CapEnd(cap)) => {
                apply_text(op, &mut cap.text, kind, inherited)
            }
            (DocMutation::Label(op), DocElement::Label(label)) => match op {
                LabelOp::SetT(t) => label.style.t = *t,
                LabelOp::SetOffset(offset) => label.style.offset = *offset,
                LabelOp::Text(op) => apply_text(op, &mut label.style.text, kind, inherited),
            },
            (DocMutation::PortalEnd(op), DocElement::PortalEnd(end)) => match op {
                PortalEndOp::SetBorderT(t) => end.style.border_t = *t,
                PortalEndOp::SetOffset(offset) => end.style.offset = *offset,
                PortalEndOp::SetColor(color) => end.style.color = *color,
                PortalEndOp::SetSize(size) => end.style.size = *size,
                PortalEndOp::SetContent(content) => end.style.content.clone_from(content),
            },
            (mutation, _) => {
                error!(
                    "document mutation: a {} payload reached a {kind:?}, which the validator refuses",
                    mutation.name()
                );
                debug_assert!(false, "a {} payload reached a {kind:?}", mutation.name());
            }
        }
    }
}

/// `position + delta`, each coordinate clamped to the canvas bound.
/// O(1).
fn nudge(position: Vec2, delta: Vec2) -> Vec2 {
    Vec2::new(
        clamp_canvas_coord(position.x + delta.x),
        clamp_canvas_coord(position.y + delta.y),
    )
}

/// Apply `op` to `style`, the text style of an element of `kind` whose
/// parent handed down `inherited`. O(1).
fn apply_text(op: &TextOp, style: &mut TextStyle, kind: DocKind, inherited: &DocInherited) {
    match op {
        TextOp::SetFont(font) => style.font = *font,
        TextOp::SetSize(size) => style.size = *size,
        TextOp::GrowSize(delta) => style.size = Some(relative(style, kind, inherited, |v| v + delta)),
        TextOp::ScaleSize(factor) => style.size = Some(relative(style, kind, inherited, |v| v * factor)),
        TextOp::SetColor(color) => style.color = *color,
        TextOp::SetWeight(weight) => style.weight = *weight,
        TextOp::SetItalic(italic) => style.italic = *italic,
    }
}

/// The result of a relative size op: `f` of the size the element
/// displays — its own, or the inherited one when it sets none — stored
/// in the unit its place takes and clamped to that unit's bound. O(1).
fn relative(style: &TextStyle, kind: DocKind, inherited: &DocInherited, f: impl Fn(f32) -> f32) -> FontSize {
    let shown = style.size.unwrap_or_else(|| inherited.text_size_for(kind));
    let value = match shown {
        FontSize::Pt(v) | FontSize::Rel(v) => f(v),
    };
    if inherited.places_in_frame(kind) {
        FontSize::Rel(clamp_rel_size(value))
    } else {
        FontSize::Pt(clamp_font_size_pt(value))
    }
}

/// Apply `op` to a connector's or a link's path. O(1), or the length of
/// what a replacement carries.
fn apply_path(op: &PathOp, path: &mut crate::document::element::PathData) {
    match op {
        PathOp::SetFit(fit) => path.flow.fit.clone_from(fit),
        PathOp::SetSpacing(spacing) => path.flow.spacing = *spacing,
        PathOp::SetSize(size) => path.flow.size = *size,
        PathOp::SetColor(color) => path.flow.color = *color,
        PathOp::SetContent(content) => path.flow.content.clone_from(content),
        PathOp::SetAnchors(start, end) => path.anchors = (*start, *end),
        PathOp::SetControlPoints(points) => path.control_points.clone_from(points),
        PathOp::MoveControlPoint { index, to } => match path.control_points.get_mut(*index) {
            Some(point) => *point = Vec2::new(clamp_canvas_coord(to.x), clamp_canvas_coord(to.y)),
            None => debug!("document mutation: the path has no control point {index}"),
        },
        PathOp::SetVisible(visible) => path.visible = *visible,
        PathOp::SetZoom(zoom) => path.zoom = *zoom,
    }
}
