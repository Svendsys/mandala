// SPDX-License-Identifier: MPL-2.0

//! An authored edit made concrete against one map: names to indices,
//! the file's spellings to the document's values (amendment 3 on #250).
//!
//! The same functions run at load, against no map, where every
//! map-specific name answers — so a load refuses what no map could
//! accept (a face this build lacks, a radius past 100, an inverted fit)
//! — and when the edit runs, against the open [`Document`], which
//! answers only the names it defines. Bounds the writer checks again before it
//! applies anything (`mutation::validate`) are not checked twice here:
//! only what building the value itself needs.

use crate::document::bounds::{self, BoundError};
use crate::document::defaults::{FIT_MAX, FIT_MIN};
use crate::document::edit::EditError;
use crate::document::element::{DocElement, DocKind};
use crate::document::format::dto::{AnchorDto, LinkKindDto, LinkTargetDto, ZoomDto};
use crate::document::format::edits::{
    FieldDto, FilterDto, FrameOpDto, InstructionDto, LabelOpDto, LayoutOpDto, MutationDto, MutatorDto,
    NodeOpDto, PathOpDto, PortalEndOpDto, RingOpDto, SectionOpDto, SpanOpDto, TextOpDto,
};
use crate::document::format::spelling::{
    ColorDto, FitDto, LineSpanDto, ShapeDto, SidesDto, StartDto, WeightDto,
};
use crate::document::ids::Document;
use crate::document::inspect::DocField;
use crate::document::mutation::ops::{
    DocMutation, FrameOp, LabelOp, LayoutOp, NodeOp, PathOp, PortalEndOp, RingOp, SectionOp, SpanOp, TextOp,
};
use crate::document::place::{nameable_font, Unit};
use crate::document::values::{
    Anchor, ColorRef, FitMode, FontSize, LineSpan, LinkKind, LinkTarget, PaletteId, Percent, PlainColor,
    Rgba, Shape, Sides, StartPoint, VarId, VerbId, Weight, ZoomWindow,
};
use crate::engine::builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};
use crate::engine::predicate::Predicate;
use crate::font::fonts::{app_font_by_family, AppFont};
use glam::Vec2;

/// What a name names, for a refusal.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum NameKind {
    /// A theme variable.
    Variable,
    /// A palette.
    Palette,
    /// A verb.
    Verb,
    /// A font family.
    Font,
}

/// Why an authored value could not be made concrete.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fault {
    /// A name nothing defines.
    Unknown {
        /// The field.
        field: &'static str,
        /// What it names.
        kind: NameKind,
        /// The name.
        name: String,
    },
    /// A value its field does not take.
    Bound {
        /// The field.
        field: &'static str,
        /// The bound it broke.
        error: BoundError,
    },
    /// A value with no meaning in its field.
    Invalid {
        /// The field.
        field: &'static str,
        /// Why.
        message: &'static str,
    },
}

impl Fault {
    /// The refusal of a run of the edit `edit`.
    pub(crate) fn into_error(self, edit: &str) -> EditError {
        let edit = edit.to_string();
        match self {
            Fault::Unknown { field, kind, name } => EditError::UnknownName {
                edit,
                field,
                name: format!(
                    "{} `{name}`",
                    match kind {
                        NameKind::Variable => "theme variable",
                        NameKind::Palette => "palette",
                        NameKind::Verb => "verb",
                        NameKind::Font => "font family",
                    }
                ),
            },
            Fault::Bound { field, error } => EditError::Value {
                edit,
                field,
                message: error.to_string(),
            },
            Fault::Invalid { field, message } => EditError::Value {
                edit,
                field,
                message: message.to_string(),
            },
        }
    }
}

type Resolved<T> = Result<T, Fault>;

/// What a name is resolved against: the open map, or, at load, no map
/// — where every map-specific name answers, so only what no map could
/// accept is refused.
type Names<'d> = Option<&'d Document>;

/// A theme variable by name: its position among the theme's variables.
fn var_of(names: Names<'_>, name: &str) -> Option<VarId> {
    let Some(doc) = names else {
        return Some(VarId(0));
    };
    let position = doc.theme().variables.iter().position(|known| known == name)?;
    u16::try_from(position).ok().map(VarId)
}

/// A palette by name.
fn palette_of(names: Names<'_>, name: &str) -> Option<PaletteId> {
    let Some(doc) = names else {
        return Some(PaletteId(0));
    };
    doc.palettes()
        .find(|(_, palette)| palette.name == name)
        .map(|(id, _)| id)
}

/// A verb by id: its position among the map's verbs.
fn verb_of(names: Names<'_>, id: &str) -> Option<VerbId> {
    let Some(doc) = names else {
        return Some(VerbId(0));
    };
    let position = doc.definitions().verbs.iter().position(|verb| verb.id == id)?;
    u16::try_from(position).ok().map(VerbId)
}

/// `dto` against `names`.
pub(crate) fn mutation(dto: &MutationDto, names: Names<'_>) -> Resolved<DocMutation> {
    Ok(match dto {
        MutationDto::Node(op) => DocMutation::Node(node_op(op, names)?),
        MutationDto::Section(op) => DocMutation::Section(section_op(op, names)?),
        MutationDto::Span(op) => DocMutation::Span(span_op(op, names)?),
        MutationDto::Frame(op) => DocMutation::Frame(frame_op(op, names)?),
        MutationDto::Ring(op) => DocMutation::Ring(ring_op(op, names)?),
        MutationDto::Path(op) => DocMutation::Path(path_op(op, names)?),
        MutationDto::Cap(op) => DocMutation::Cap(text_op(op, DocKind::CapStart, names)?),
        MutationDto::Label(op) => DocMutation::Label(match op {
            LabelOpDto::SetT(t) => LabelOp::SetT(*t),
            LabelOpDto::SetOffset(offset) => LabelOp::SetOffset(*offset),
            LabelOpDto::Text(op) => LabelOp::Text(text_op(op, DocKind::Label, names)?),
        }),
        MutationDto::PortalEnd(op) => DocMutation::PortalEnd(match op {
            PortalEndOpDto::SetBorderT(t) => PortalEndOp::SetBorderT(*t),
            PortalEndOpDto::SetOffset(offset) => PortalEndOp::SetOffset(*offset),
            PortalEndOpDto::SetColor(color) => PortalEndOp::SetColor(opt(color, |c| self::color(c, names))?),
            PortalEndOpDto::SetSize(size) => PortalEndOp::SetSize(*size),
            PortalEndOpDto::SetContent(content) => PortalEndOp::SetContent(content.clone()),
        }),
    })
}

/// `dto` as a mutator AST, every payload and predicate resolved.
/// Recursive in the definition's depth, which a load holds to
/// [`bounds::MAX_DEFINITION_DEPTH`].
pub(crate) fn mutator(dto: &MutatorDto, names: Names<'_>) -> Resolved<MutatorNode<DocElement, DocMutation>> {
    let children = |list: &[MutatorDto]| {
        list.iter()
            .map(|child| mutator(child, names))
            .collect::<Resolved<Vec<_>>>()
    };
    Ok(match dto {
        MutatorDto::Void {
            kind,
            channel,
            children: list,
        } => MutatorNode::Void {
            kind: (*kind).into(),
            channel: self::channel(*channel)?,
            children: children(list)?,
        },
        MutatorDto::Macro {
            kind,
            channel,
            mutations,
            children: list,
        } => MutatorNode::Macro {
            kind: (*kind).into(),
            channel: self::channel(*channel)?,
            mutations: MutationListSrc::Literal(
                mutations
                    .iter()
                    .map(|m| mutation(m, names))
                    .collect::<Resolved<Vec<_>>>()?,
            ),
            children: children(list)?,
        },
        MutatorDto::Instruction {
            kind,
            channel,
            instruction,
            children: list,
        } => MutatorNode::Instruction {
            kind: (*kind).into(),
            channel: self::channel(*channel)?,
            instruction: match instruction {
                InstructionDto::RepeatWhile(filter) => {
                    InstructionSpec::RepeatWhile(predicate(filter, names)?)
                }
                InstructionDto::RepeatWhileAlwaysTrue => InstructionSpec::RepeatWhileAlwaysTrue,
            },
            mutation: MutationSrc::None,
            children: children(list)?,
        },
    })
}

/// `dto` as a predicate over document elements.
pub(crate) fn predicate(dto: &FilterDto, names: Names<'_>) -> Resolved<Predicate<DocElement>> {
    let mut fields = Vec::with_capacity(dto.fields.len());
    for (field, comparator) in &dto.fields {
        fields.push((self::field(field, names)?, (*comparator).into()));
    }
    Ok(Predicate {
        fields,
        always_match: dto.always_match,
    })
}

/// One predicate field.
fn field(dto: &FieldDto, names: Names<'_>) -> Resolved<DocField> {
    use FieldDto as F;
    Ok(match dto {
        F::Kind(kind) => DocField::Kind((*kind).into()),
        F::Background(color) => DocField::Background(plain_color("background", color, names)?),
        F::Channel(value) => DocField::Channel(channel(*value)?),
        F::Ident(value) => DocField::Ident(*value),
        F::PositionX(value) => DocField::PositionX(*value),
        F::PositionY(value) => DocField::PositionY(*value),
        F::MinWidth(value) => DocField::MinWidth(*value),
        F::MinHeight(value) => DocField::MinHeight(*value),
        F::Shape(shape) => DocField::Shape(self::shape(*shape)?),
        F::Palette(name) => DocField::Palette(palette("palette", name, names)?.0),
        F::Folded(value) => DocField::Folded(*value),
        F::Hidden(value) => DocField::Hidden(*value),
        F::Opacity(value) => DocField::Opacity(*value),
        F::OnClick(id) => DocField::OnClick(verb("on_click", id, names)?.0),
        F::OnHover(id) => DocField::OnHover(verb("on_hover", id, names)?.0),
        F::ZoomMin(value) => DocField::ZoomMin(*value),
        F::ZoomMax(value) => DocField::ZoomMax(*value),
        F::OffsetX(value) => DocField::OffsetX(*value),
        F::OffsetY(value) => DocField::OffsetY(*value),
        F::Align(align) => DocField::Align((*align).into()),
        F::Font(family) => DocField::Font(font(family)?),
        F::FontSize(size) => DocField::FontSize((*size).into()),
        F::Color(color) => DocField::Color(self::color(color, names)?),
        F::Weight(weight) => DocField::Weight(self::weight(*weight)?.get()),
        F::Italic(value) => DocField::Italic(*value),
        F::Visible(value) => DocField::Visible(*value),
        F::StartEdge(edge) => DocField::StartEdge((*edge).into()),
        F::StartAt(value) => DocField::StartAt(*value),
        F::Direction(direction) => DocField::Direction((*direction).into()),
        F::Fit(fit) => DocField::Fit(self::fit(fit)?),
        F::Gap(value) => DocField::Gap(*value),
        F::Inset(value) => DocField::Inset(*value),
        F::Spacing(value) => DocField::Spacing(*value),
        F::AnchorFrom(anchor) => DocField::AnchorFrom((*anchor).into()),
        F::AnchorTo(anchor) => DocField::AnchorTo((*anchor).into()),
        F::LinkKind(kind) => DocField::LinkKind((*kind).into()),
        F::LabelT(value) => DocField::LabelT(*value),
        F::Offset(value) => DocField::Offset(*value),
        F::BorderT(value) => DocField::BorderT(*value),
    })
}

/// A text style op on an element of `kind`, whose size is in the unit
/// its tier stores.
fn text_op(dto: &TextOpDto, kind: DocKind, names: Names<'_>) -> Resolved<TextOp> {
    Ok(match dto {
        TextOpDto::SetFont(family) => TextOp::SetFont(opt(family, |family| font(family))?),
        TextOpDto::SetSize(size) => TextOp::SetSize(size.map(|size| match Unit::of_tier(kind) {
            Unit::Relative => FontSize::Rel(size),
            _ => FontSize::Pt(size),
        })),
        TextOpDto::GrowSize(delta) => TextOp::GrowSize(*delta),
        TextOpDto::ScaleSize(factor) => TextOp::ScaleSize(*factor),
        TextOpDto::SetColor(color) => TextOp::SetColor(opt(color, |c| self::color(c, names))?),
        TextOpDto::SetWeight(value) => TextOp::SetWeight(opt(value, |w| weight(*w))?),
        TextOpDto::SetItalic(value) => TextOp::SetItalic(*value),
    })
}

fn node_op(dto: &NodeOpDto, names: Names<'_>) -> Resolved<NodeOp> {
    Ok(match dto {
        NodeOpDto::SetPosition(at) => NodeOp::SetPosition(Vec2::from(*at)),
        NodeOpDto::Nudge(by) => NodeOp::Nudge(Vec2::from(*by)),
        NodeOpDto::SetMinSize(size) => NodeOp::SetMinSize(size.map(Vec2::from)),
        NodeOpDto::SetShape(value) => NodeOp::SetShape(opt(value, |s| shape(*s))?),
        NodeOpDto::SetPalette(name) => NodeOp::SetPalette(opt(name, |n| palette("set_palette", n, names))?),
        NodeOpDto::SetZoom(value) => NodeOp::SetZoom(opt(value, zoom)?),
        NodeOpDto::SetFolded(value) => NodeOp::SetFolded(*value),
        NodeOpDto::ToggleFolded => NodeOp::ToggleFolded,
        NodeOpDto::SetHidden(value) => NodeOp::SetHidden(*value),
        NodeOpDto::SetOpacity(value) => NodeOp::SetOpacity(*value),
        NodeOpDto::SetClick(id) => NodeOp::SetClick(opt(id, |id| verb("set_click", id, names))?),
        NodeOpDto::SetHover(id) => NodeOp::SetHover(opt(id, |id| verb("set_hover", id, names))?),
        NodeOpDto::SetChannel(value) => NodeOp::SetChannel(channel(*value)?),
        NodeOpDto::Text(op) => NodeOp::Text(text_op(op, DocKind::Node, names)?),
    })
}

fn section_op(dto: &SectionOpDto, names: Names<'_>) -> Resolved<SectionOp> {
    Ok(match dto {
        SectionOpDto::SetOffset(at) => SectionOp::SetOffset(Vec2::from(*at)),
        SectionOpDto::Nudge(by) => SectionOp::Nudge(Vec2::from(*by)),
        SectionOpDto::SetMinSize(size) => SectionOp::SetMinSize(size.map(Vec2::from)),
        SectionOpDto::SetAlign(align) => SectionOp::SetAlign(align.map(Into::into)),
        SectionOpDto::SetChannel(value) => SectionOp::SetChannel(channel(*value)?),
        SectionOpDto::Text(op) => SectionOp::Text(text_op(op, DocKind::Section, names)?),
    })
}

fn span_op(dto: &SpanOpDto, names: Names<'_>) -> Resolved<SpanOp> {
    Ok(match dto {
        SpanOpDto::SetText(text) => SpanOp::SetText(text.clone()),
        SpanOpDto::InsertText { at, text } => SpanOp::InsertText {
            at: *at,
            text: text.clone(),
        },
        SpanOpDto::DeleteText { start, end } => SpanOp::DeleteText {
            start: *start,
            end: *end,
        },
        SpanOpDto::SetLink(target) => SpanOp::SetLink(opt(target, |target| match target {
            LinkTargetDto::Url(url) => Ok(LinkTarget::Url(url.to_string())),
            LinkTargetDto::Verb(id) => verb("set_link", id, names).map(LinkTarget::Verb),
        })?),
        SpanOpDto::Text(op) => SpanOp::Text(text_op(op, DocKind::Span, names)?),
    })
}

fn frame_op(dto: &FrameOpDto, names: Names<'_>) -> Resolved<FrameOp> {
    Ok(match dto {
        FrameOpDto::SetVisible(value) => FrameOp::SetVisible(*value),
        FrameOpDto::SetStart(value) => FrameOp::SetStart(opt(value, start)?),
        FrameOpDto::SetDirection(value) => FrameOp::SetDirection(value.map(Into::into)),
        FrameOpDto::SetFit(value) => FrameOp::SetFit(opt(value, fit)?),
        FrameOpDto::SetGap(value) => FrameOp::SetGap(*value),
        FrameOpDto::SetInset(value) => FrameOp::SetInset(*value),
        FrameOpDto::Layout(op) => FrameOp::Layout(layout_op(op, names)?),
        FrameOpDto::Text(op) => FrameOp::Text(text_op(op, DocKind::Frame, names)?),
    })
}

fn layout_op(dto: &LayoutOpDto, names: Names<'_>) -> Resolved<LayoutOp> {
    Ok(match dto {
        LayoutOpDto::SetSpan(value) => LayoutOp::SetSpan(opt(value, |s| line_span(*s))?),
        LayoutOpDto::SetSides(value) => LayoutOp::SetSides(opt(value, |s| sides(s, names))?),
        LayoutOpDto::SetSpacing(value) => LayoutOp::SetSpacing(*value),
    })
}

fn ring_op(dto: &RingOpDto, names: Names<'_>) -> Resolved<RingOp> {
    Ok(match dto {
        RingOpDto::SetFit(value) => RingOp::SetFit(opt(value, fit)?),
        RingOpDto::Layout(op) => RingOp::Layout(layout_op(op, names)?),
        RingOpDto::SetChannel(value) => RingOp::SetChannel(channel(*value)?),
        RingOpDto::Text(op) => RingOp::Text(text_op(op, DocKind::Ring, names)?),
    })
}

fn path_op(dto: &PathOpDto, names: Names<'_>) -> Resolved<PathOp> {
    Ok(match dto {
        PathOpDto::SetFit(value) => PathOp::SetFit(opt(value, fit)?),
        PathOpDto::SetSpacing(value) => PathOp::SetSpacing(*value),
        PathOpDto::SetSize(value) => PathOp::SetSize(*value),
        PathOpDto::SetColor(color) => PathOp::SetColor(opt(color, |c| self::color(c, names))?),
        PathOpDto::SetContent(content) => PathOp::SetContent(content.clone()),
        PathOpDto::SetAnchors(from, to) => PathOp::SetAnchors((*from).into(), (*to).into()),
        PathOpDto::SetControlPoints(points) => {
            PathOp::SetControlPoints(points.iter().copied().map(Vec2::from).collect())
        }
        PathOpDto::MoveControlPoint { index, to } => PathOp::MoveControlPoint {
            index: *index,
            to: Vec2::from(*to),
        },
        PathOpDto::SetVisible(value) => PathOp::SetVisible(*value),
        PathOpDto::SetZoom(value) => PathOp::SetZoom(opt(value, zoom)?),
    })
}

/// `f` over an optional value.
fn opt<T, U>(value: &Option<T>, f: impl FnOnce(&T) -> Resolved<U>) -> Resolved<Option<U>> {
    value.as_ref().map(f).transpose()
}

/// A color, its variable by name.
fn color(dto: &ColorDto, names: Names<'_>) -> Resolved<ColorRef> {
    Ok(match dto {
        ColorDto::Hex(hex) => ColorRef::Rgba(Rgba(hex.0)),
        ColorDto::Var(name) => ColorRef::Var(var_of(names, name).ok_or_else(|| Fault::Unknown {
            field: "color",
            kind: NameKind::Variable,
            name: name.to_string(),
        })?),
        ColorDto::Palette(field) => ColorRef::Palette((*field).into()),
    })
}

/// A color with no palette field: the map's background.
fn plain_color(field: &'static str, dto: &ColorDto, names: Names<'_>) -> Resolved<PlainColor> {
    match color(dto, names)? {
        ColorRef::Rgba(rgba) => Ok(PlainColor::Rgba(rgba)),
        ColorRef::Var(var) => Ok(PlainColor::Var(var)),
        ColorRef::Palette(_) => Err(Fault::Invalid {
            field,
            message: "a background is never a palette color",
        }),
    }
}

/// A palette by name.
fn palette(field: &'static str, name: &str, names: Names<'_>) -> Resolved<PaletteId> {
    palette_of(names, name).ok_or_else(|| Fault::Unknown {
        field,
        kind: NameKind::Palette,
        name: name.to_string(),
    })
}

/// A verb by id.
fn verb(field: &'static str, id: &str, names: Names<'_>) -> Resolved<VerbId> {
    verb_of(names, id).ok_or_else(|| Fault::Unknown {
        field,
        kind: NameKind::Verb,
        name: id.to_string(),
    })
}

/// A compiled-in face by family name, as the loader reads one: a name
/// this build has no nameable face for is refused, on every map.
fn font(family: &str) -> Resolved<AppFont> {
    app_font_by_family(family)
        .filter(|font| nameable_font(*font))
        .ok_or_else(|| Fault::Unknown {
            field: "font",
            kind: NameKind::Font,
            name: family.to_string(),
        })
}

/// A channel, held to [`bounds::MAX_CHANNEL`] as the loader holds one.
fn channel(value: u64) -> Resolved<usize> {
    usize::try_from(value)
        .map_err(|_| BoundError::NotAChannel(value))
        .and_then(|channel| bounds::check_channel(channel).map(|()| channel))
        .map_err(|error| Fault::Bound {
            field: "channel",
            error,
        })
}

fn weight(dto: WeightDto) -> Resolved<Weight> {
    Weight::new(dto.0).map_err(|error| Fault::Bound {
        field: "weight",
        error,
    })
}

fn shape(dto: ShapeDto) -> Resolved<Shape> {
    Ok(match dto {
        ShapeDto::Ellipse => Shape::Ellipse,
        ShapeDto::Rectangle(rectangle) => {
            let radius = u8::try_from(rectangle.radius)
                .map_err(|_| BoundError::Above {
                    value: rectangle.radius as f32,
                    max: f32::from(bounds::MAX_PERCENT),
                })
                .and_then(bounds::check_percent)
                .map_err(|error| Fault::Bound {
                    field: "radius",
                    error,
                })?;
            Shape::Rectangle {
                radius: Percent::new(radius),
            }
        }
    })
}

fn fit(dto: &FitDto) -> Resolved<FitMode> {
    Ok(match dto {
        FitDto::Repeat => FitMode::Repeat,
        FitDto::Spread => FitMode::Spread,
        FitDto::Once(None) => FitMode::Once { filler: None },
        FitDto::Once(Some(once)) => FitMode::Once {
            filler: Some(
                bounds::check_ornament(&once.filler)
                    .map_err(|error| Fault::Bound {
                        field: "filler",
                        error,
                    })?
                    .to_owned(),
            ),
        },
        FitDto::Fit(None) => FitMode::Fit {
            min: FIT_MIN,
            max: FIT_MAX,
        },
        FitDto::Fit(Some(range)) => {
            let (min, max) = bounds::check_fit(range.min, range.max)
                .map_err(|error| Fault::Bound { field: "fit", error })?;
            FitMode::Fit { min, max }
        }
    })
}

fn start(dto: &StartDto) -> Resolved<StartPoint> {
    let at = bounds::check_unit_interval(dto.0.at).map_err(|error| Fault::Bound { field: "at", error })?;
    Ok(StartPoint {
        edge: dto.0.edge.into(),
        at,
    })
}

fn line_span(dto: LineSpanDto) -> Resolved<LineSpan> {
    match dto {
        LineSpanDto::Natural => Ok(LineSpan::NATURAL),
        LineSpanDto::Whole => Ok(LineSpan::WHOLE),
        LineSpanDto::Fraction(value) => bounds::check_unit_interval(value)
            .ok()
            .and_then(LineSpan::new)
            .ok_or(Fault::Invalid {
                field: "span",
                message: "a span is `natural`, `whole` or a number from 0 to 1",
            }),
    }
}

fn sides(dto: &SidesDto, names: Names<'_>) -> Resolved<Sides> {
    let content = |content: &str| {
        bounds::check_ornament(content)
            .map(str::to_owned)
            .map_err(|error| Fault::Bound {
                field: "sides",
                error,
            })
    };
    Ok(match dto {
        SidesDto::Off => Sides::Off,
        SidesDto::Text(text) => Sides::On {
            content: content(text)?,
            color: None,
        },
        SidesDto::Object(object) => Sides::On {
            content: content(&object.content)?,
            color: opt(&object.color, |c| color(c, names))?,
        },
    })
}

fn zoom(dto: &ZoomDto) -> Resolved<ZoomWindow> {
    bounds::check_zoom_window(dto.min, dto.max).map_err(|error| Fault::Bound { field: "zoom", error })?;
    Ok(ZoomWindow {
        min: dto.min,
        max: dto.max,
    })
}

impl From<AnchorDto> for Anchor {
    fn from(dto: AnchorDto) -> Self {
        match dto {
            AnchorDto::Auto => Anchor::Auto,
            AnchorDto::Top => Anchor::Top,
            AnchorDto::Right => Anchor::Right,
            AnchorDto::Bottom => Anchor::Bottom,
            AnchorDto::Left => Anchor::Left,
        }
    }
}

impl From<LinkKindDto> for LinkKind {
    fn from(dto: LinkKindDto) -> Self {
        match dto {
            LinkKindDto::Line => LinkKind::Line,
            LinkKindDto::Portal => LinkKind::Portal,
        }
    }
}
