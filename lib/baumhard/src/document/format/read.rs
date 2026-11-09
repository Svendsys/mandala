// SPDX-License-Identifier: MPL-2.0

//! The file's spellings of single values, read from text a person typed
//! — a console line, a keybind's argument — rather than from a map:
//! `format::color` and `format::font` beside them in [`super`], and here
//! the start points, edges, directions, alignments, shapes, weights,
//! line spans, sides and fit modes.
//!
//! Each reads the very spelling type the loader reads
//! (`format::spelling`, `format::dto`) and holds the value to the same
//! bound, so a typed value is accepted exactly when a file's is: a name
//! the file gains is a name the console takes, with no second table to
//! keep in step. Text that is JSON is read as JSON (`700`, `0.5`,
//! `false`, `{"edge": "left", "at": 0.25}`); any other text is read as
//! the JSON string it would be (`top-left`, `bold`, `whole`, `✦`).
//!
//! Each is `None` for anything the file refuses. Cost: the text's
//! length.

use serde::Deserialize;

use crate::document::bounds;
use crate::document::format::dto::{AlignDto, DirectionDto};
use crate::document::format::spelling::{
    EdgeDto, FitDto, LineSpanDto, ShapeDto, SidesDto, StartDto, WeightDto,
};
use crate::document::ids::Document;
use crate::document::values::{
    Align, Direction, Edge, FitMode, LineSpan, Percent, Shape, Sides, StartPoint, Weight,
};

/// `text` as a value of the file's spelling type `T`: the text as JSON
/// when it is JSON (`700`, `false`, an object), else the bare text as a
/// JSON string (`bold`, `top-left`, `✦`) — one rule, so `false` is the
/// value and never the word.
fn read<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    use serde::de::{value::Error, IntoDeserializer};
    if serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok() {
        return serde_json::from_str(text).ok();
    }
    let as_string: serde::de::value::StrDeserializer<'_, Error> = text.into_deserializer();
    T::deserialize(as_string).ok()
}

impl From<EdgeDto> for Edge {
    fn from(edge: EdgeDto) -> Edge {
        match edge {
            EdgeDto::Top => Edge::Top,
            EdgeDto::Right => Edge::Right,
            EdgeDto::Bottom => Edge::Bottom,
            EdgeDto::Left => Edge::Left,
        }
    }
}

impl From<DirectionDto> for Direction {
    fn from(direction: DirectionDto) -> Direction {
        match direction {
            DirectionDto::Clockwise => Direction::Clockwise,
            DirectionDto::Counterclockwise => Direction::CounterClockwise,
        }
    }
}

impl From<AlignDto> for Align {
    fn from(align: AlignDto) -> Align {
        match align {
            AlignDto::Left => Align::Left,
            AlignDto::Center => Align::Center,
            AlignDto::Right => Align::Right,
        }
    }
}

/// A side of the outline: `top`, `right`, `bottom` or `left`.
pub fn edge(text: &str) -> Option<Edge> {
    read::<EdgeDto>(text).map(Edge::from)
}

/// Where a frame's text starts: one of the eight names, or
/// `{"edge": …, "at": …}` with `at` in `0..=1`.
pub fn start(text: &str) -> Option<StartPoint> {
    let StartDto(point) = read(text)?;
    let at = bounds::check_unit_interval(point.at).ok()?;
    Some(StartPoint {
        edge: point.edge.into(),
        at,
    })
}

/// Which way a frame's text runs: `clockwise` or `counterclockwise`.
pub fn direction(text: &str) -> Option<Direction> {
    read::<DirectionDto>(text).map(Direction::from)
}

/// A section's alignment: `left`, `center` or `right`.
pub fn align(text: &str) -> Option<Align> {
    read::<AlignDto>(text).map(Align::from)
}

/// A node's outline: `ellipse`, or `{"rectangle": {"radius": …}}` with
/// the radius in `0..=100`.
pub fn shape(text: &str) -> Option<Shape> {
    Some(match read::<ShapeDto>(text)? {
        ShapeDto::Ellipse => Shape::Ellipse,
        ShapeDto::Rectangle(rectangle) => {
            let radius = u8::try_from(rectangle.radius).ok()?;
            Shape::Rectangle {
                radius: Percent::new(bounds::check_percent(radius).ok()?),
            }
        }
    })
}

/// A weight: `normal`, `bold`, or a number from 100 to 900 in steps of
/// 100.
pub fn weight(text: &str) -> Option<Weight> {
    Weight::new(read::<WeightDto>(text)?.0).ok()
}

/// How much of its half each line of a ring covers: `natural`, `whole`,
/// or a number in `0..=1`.
pub fn line_span(text: &str) -> Option<LineSpan> {
    match read::<LineSpanDto>(text)? {
        LineSpanDto::Natural => Some(LineSpan::NATURAL),
        LineSpanDto::Whole => Some(LineSpan::WHOLE),
        LineSpanDto::Fraction(value) => LineSpan::new(bounds::check_unit_interval(value).ok()?),
    }
}

/// A fit mode: `repeat`, `spread`, `once`, `fit`,
/// `{"once": {"filler": …}}` or `{"fit": {"min": …, "max": …}}`, with the
/// loader's bounds on the filler and the scale range.
pub fn fit(text: &str) -> Option<FitMode> {
    use crate::document::defaults::{FIT_MAX, FIT_MIN};
    Some(match read::<FitDto>(text)? {
        FitDto::Repeat => FitMode::Repeat,
        FitDto::Spread => FitMode::Spread,
        FitDto::Once(None) => FitMode::Once { filler: None },
        FitDto::Once(Some(once)) => FitMode::Once {
            filler: Some(bounds::check_ornament(&once.filler).ok()?.to_owned()),
        },
        FitDto::Fit(None) => FitMode::Fit {
            min: FIT_MIN,
            max: FIT_MAX,
        },
        FitDto::Fit(Some(range)) => {
            let (min, max) = bounds::check_fit(range.min, range.max).ok()?;
            FitMode::Fit { min, max }
        }
    })
}

/// The ornament in a frame ring's breaks: `false` for none, its text
/// (`✦`), or `{"content": …, "color": …}` with a color as `doc` reads one.
/// `true` is refused, as the file refuses it, and so is empty content.
pub fn sides(doc: &Document, text: &str) -> Option<Sides> {
    Some(match read::<SidesDto>(text)? {
        SidesDto::Off => Sides::Off,
        SidesDto::Text(content) => Sides::On {
            content: bounds::check_ornament(&content).ok()?.to_owned(),
            color: None,
        },
        SidesDto::Object(object) => Sides::On {
            content: bounds::check_ornament(&object.content).ok()?.to_owned(),
            color: match object.color {
                Some(color) => Some(super::color_of(doc, color)?),
                None => None,
            },
        },
    })
}
