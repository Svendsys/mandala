// SPDX-License-Identifier: MPL-2.0

//! The style keys of a file become the `Option` style structs of the
//! document: every key checked against its bound, every name resolved,
//! every absent key left `None` to inherit.
//!
//! One function per group of keys, over a view of the fields
//! ([`TextIn`] and its siblings, built by the macros in `macros.rs`),
//! so an element and the defaults for it share the conversion.

use crate::document::bounds;
use crate::document::element::DocKind;
use crate::document::element::{
    FlowStyle, FrameStyle, LabelStyle, PortalEndStyle, RingLayoutStyle, TextStyle,
};
use crate::document::format::dto::DirectionDto;
use crate::document::format::from_dto::{Builder, LoadResult};
use crate::document::format::spelling::{ColorDto, FitDto, LineSpanDto, SidesDto, StartDto, WeightDto};
use crate::document::place::Unit;
use crate::document::values::{Direction, Sides, Span};

/// A view of the five text-style keys.
pub(crate) struct TextIn<'a> {
    /// `font`.
    pub font: Option<&'a str>,
    /// `size`.
    pub size: Option<f32>,
    /// `color`.
    pub color: Option<&'a ColorDto<'a>>,
    /// `weight`.
    pub weight: Option<WeightDto>,
    /// `italic`.
    pub italic: Option<bool>,
}

/// A view of a frame's keys.
pub(crate) struct FrameIn<'a> {
    /// `visible`.
    pub visible: Option<bool>,
    /// `start`.
    pub start: Option<&'a StartDto>,
    /// `direction`.
    pub direction: Option<DirectionDto>,
    /// `fit`.
    pub fit: Option<&'a FitDto<'a>>,
    /// `gap`.
    pub gap: Option<f32>,
    /// `inset`.
    pub inset: Option<f32>,
    /// The layout keys.
    pub layout: LayoutIn<'a>,
    /// The text-style keys.
    pub text: TextIn<'a>,
}

/// A view of the layout keys a frame, a ring and their defaults share.
pub(crate) struct LayoutIn<'a> {
    /// `span`.
    pub span: Option<LineSpanDto>,
    /// `sides`.
    pub sides: Option<&'a SidesDto<'a>>,
    /// `spacing`.
    pub spacing: Option<f32>,
}

/// A view of a path's style keys.
pub(crate) struct FlowIn<'a> {
    /// `fit`.
    pub fit: Option<&'a FitDto<'a>>,
    /// `spacing`.
    pub spacing: Option<f32>,
    /// `size`.
    pub size: Option<f32>,
    /// `color`.
    pub color: Option<&'a ColorDto<'a>>,
    /// `content`.
    pub content: Option<&'a str>,
}

/// A view of a portal end's style keys.
pub(crate) struct PortalIn<'a> {
    /// `border_t`.
    pub border_t: Option<f32>,
    /// `offset`.
    pub offset: Option<f32>,
    /// `color`.
    pub color: Option<&'a ColorDto<'a>>,
    /// `size`.
    pub size: Option<f32>,
    /// `content`.
    pub content: Option<&'a str>,
}

impl Builder {
    /// A text style whose `size` is in `unit`. The current path is the
    /// object that holds the keys.
    pub(super) fn text_style(&mut self, text: TextIn<'_>, unit: Unit) -> LoadResult<TextStyle> {
        Ok(TextStyle {
            font: match text.font {
                Some(family) => Some(self.font(family)?),
                None => None,
            },
            size: text.size.map(|size| self.size(size, unit)).transpose()?,
            color: match text.color {
                Some(color) => Some(self.color("color", color)?),
                None => None,
            },
            weight: match text.weight {
                Some(weight) => Some(self.weight(weight.0)?),
                None => None,
            },
            italic: text.italic,
        })
    }

    /// A frame's style: its text style is in fractions of the node.
    pub(super) fn frame_style(&mut self, frame: FrameIn<'_>) -> LoadResult<FrameStyle> {
        Ok(FrameStyle {
            visible: frame.visible,
            start: match frame.start {
                Some(start) => Some(self.start("start", start)?),
                None => None,
            },
            direction: frame.direction.map(|direction| match direction {
                DirectionDto::Clockwise => Direction::Clockwise,
                DirectionDto::Counterclockwise => Direction::CounterClockwise,
            }),
            fit: match frame.fit {
                Some(fit) => Some(self.fit("fit", fit)?),
                None => None,
            },
            gap: self.opt_number("gap", frame.gap, bounds::check_non_negative)?,
            inset: self.opt_number("inset", frame.inset, bounds::check_non_negative)?,
            ring_layout: self.ring_layout(frame.layout)?,
            text: self.text_style(frame.text, Unit::of_tier(DocKind::Frame))?,
        })
    }

    /// A frame's or a ring's layout keys: a numeric `span` in `0..=1`, a
    /// `sides` ornament that is not empty, a `spacing` `>= 0`. The
    /// current path is the object that holds the keys.
    pub(super) fn ring_layout(&mut self, layout: LayoutIn<'_>) -> LoadResult<RingLayoutStyle> {
        let span = match layout.span {
            None => None,
            Some(LineSpanDto::Natural) => Some(Span::NATURAL),
            Some(LineSpanDto::Whole) => Some(Span::WHOLE),
            Some(LineSpanDto::Fraction(value)) => {
                let value = self.number("span", value, bounds::check_unit_interval)?;
                // In range: the check above holds it to `0..=1`.
                Some(Span::new(value).unwrap_or(Span::WHOLE))
            }
        };
        let sides = match layout.sides {
            None => None,
            Some(SidesDto::Off) => Some(Sides::Off),
            Some(SidesDto::Text(content)) => Some(Sides::On {
                content: self.bounded("sides", bounds::check_ornament(content))?.to_owned(),
                color: None,
            }),
            Some(SidesDto::Object(object)) => Some(self.key("sides", |b| {
                let content = b.bounded("content", bounds::check_ornament(&object.content))?;
                Ok(Sides::On {
                    content: content.to_owned(),
                    color: match &object.color {
                        Some(color) => Some(b.color("color", color)?),
                        None => None,
                    },
                })
            })?),
        };
        Ok(RingLayoutStyle {
            span,
            sides,
            spacing: self.opt_number("spacing", layout.spacing, bounds::check_non_negative)?,
        })
    }

    /// A path's style and default content. Its `size` is in points.
    pub(super) fn flow_style(&mut self, flow: FlowIn<'_>) -> LoadResult<FlowStyle> {
        Ok(FlowStyle {
            fit: match flow.fit {
                Some(fit) => Some(self.fit("fit", fit)?),
                None => None,
            },
            spacing: self.opt_number("spacing", flow.spacing, bounds::check_non_negative)?,
            size: self.opt_number("size", flow.size, bounds::check_font_size_pt)?,
            color: match flow.color {
                Some(color) => Some(self.color("color", color)?),
                None => None,
            },
            content: flow.content.map(str::to_string),
        })
    }

    /// A label's style: its position along the path, its offset from
    /// it, and its text.
    pub(super) fn label_style(
        &mut self,
        t: Option<f32>,
        offset: Option<f32>,
        text: TextIn<'_>,
    ) -> LoadResult<LabelStyle> {
        Ok(LabelStyle {
            t: self.opt_number("t", t, bounds::check_unit_interval)?,
            offset: self.opt_number("offset", offset, bounds::check_canvas_coord)?,
            text: self.text_style(text, Unit::of_tier(DocKind::Label))?,
        })
    }

    /// A portal end's style. Its `size` is in points.
    pub(super) fn portal_end_style(&mut self, end: PortalIn<'_>) -> LoadResult<PortalEndStyle> {
        Ok(PortalEndStyle {
            border_t: self.opt_number("border_t", end.border_t, bounds::check_unit_interval)?,
            offset: self.opt_number("offset", end.offset, bounds::check_canvas_coord)?,
            color: match end.color {
                Some(color) => Some(self.color("color", color)?),
                None => None,
            },
            size: self.opt_number("size", end.size, bounds::check_font_size_pt)?,
            content: end.content.map(str::to_string),
        })
    }
}
