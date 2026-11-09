// SPDX-License-Identifier: MPL-2.0

//! The spellings a console line and a keybind's argument give a field's
//! value, read once into the document's typed values (design §5.3) as
//! the line is run. Where the file has a spelling the console takes the
//! file's ([`color`], [`font`]); where the file's is a JSON object, a
//! short form stands for it (`fit=once:·`, `start=top:0.25`). `reset`
//! is the console's word for "set nothing here": the field inherits, as
//! a key a file leaves out does.

use baumhard::document::format;
use baumhard::document::ids::Document;
use baumhard::document::values::{
    Align, ColorRef, Direction, Edge, FitMode, LineSpan, Shape, Sides, StartPoint, Weight,
};
use baumhard::font::fonts::AppFont;

/// The word that clears a field so it inherits.
pub(crate) const RESET: &str = "reset";

/// `text` as a value of `what`, or `None` for [`RESET`]: what `read`
/// gives, with the message naming the field.
pub(crate) fn or_reset<T>(
    text: &str,
    read: impl FnOnce(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    if text.eq_ignore_ascii_case(RESET) {
        Ok(None)
    } else {
        read(text).map(Some)
    }
}

/// A color as the file spells one — `#rrggbb`, `#rrggbbaa`, `var(name)`
/// naming one of `doc`'s theme variables, `palette(frame|text|title)` —
/// or [`RESET`].
pub(crate) fn color(doc: &Document, text: &str) -> Result<Option<ColorRef>, String> {
    or_reset(text, |text| {
        format::color(doc, text).ok_or_else(|| {
            format!(
                "'{text}' is not a color: `#rrggbb`, `#rrggbbaa`, `var(name)` of the theme, \
                 `palette(frame|text|title)` or `{RESET}`"
            )
        })
    })
}

/// A font family a map may name, as the loader reads one, or [`RESET`].
pub(crate) fn font(text: &str) -> Result<Option<AppFont>, String> {
    or_reset(text, |text| {
        format::font(text).ok_or_else(|| format!("'{text}' is not a font a map can name"))
    })
}

/// A finite number.
pub(crate) fn number(text: &str) -> Result<f32, String> {
    text.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("'{text}' is not a number"))
}

/// A whole number.
pub(crate) fn index(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|_| format!("'{text}' is not a whole number"))
}

/// `true` or `false`.
pub(crate) fn boolean(text: &str) -> Result<bool, String> {
    match text.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("'{text}' is not `true` or `false`")),
    }
}

/// A weight: `normal`, `bold`, or `100`…`900` in steps of 100.
pub(crate) fn weight(text: &str) -> Result<Weight, String> {
    let bad = || format!("'{text}' is not a weight: `normal`, `bold` or 100…900 in steps of 100");
    match text.to_ascii_lowercase().as_str() {
        "normal" => Ok(Weight::NORMAL),
        "bold" => Ok(Weight::BOLD),
        other => Weight::new(other.parse().map_err(|_| bad())?).map_err(|_| bad()),
    }
}

/// A fit mode: `once`, `once:<filler>`, `repeat`, `spread`, `fit` (the
/// built-in scale range) or `fit:<min>,<max>`.
pub(crate) fn fit(text: &str) -> Result<FitMode, String> {
    let bad = || {
        format!("'{text}' is not a fit mode: `once[:<filler>]`, `repeat`, `spread` or `fit[:<min>,<max>]`")
    };
    let (head, rest) = match text.split_once(':') {
        Some((head, rest)) => (head, Some(rest)),
        None => (text, None),
    };
    match (head.to_ascii_lowercase().as_str(), rest) {
        ("once", None) => Ok(FitMode::Once { filler: None }),
        ("once", Some(filler)) if !filler.is_empty() => Ok(FitMode::Once {
            filler: Some(filler.to_string()),
        }),
        ("repeat", None) => Ok(FitMode::Repeat),
        ("spread", None) => Ok(FitMode::Spread),
        ("fit", None) => Ok(FitMode::Fit {
            min: baumhard::document::defaults::FIT_MIN,
            max: baumhard::document::defaults::FIT_MAX,
        }),
        ("fit", Some(range)) => {
            let (min, max) = range.split_once(',').ok_or_else(bad)?;
            Ok(FitMode::Fit {
                min: number(min.trim()).map_err(|_| bad())?,
                max: number(max.trim()).map_err(|_| bad())?,
            })
        }
        _ => Err(bad()),
    }
}

/// The eight named start points and the side and fraction each means,
/// as the file names them.
const NAMED_STARTS: [(&str, Edge, f32); 8] = [
    ("top-left", Edge::Top, 0.0),
    ("top", Edge::Top, 0.5),
    ("top-right", Edge::Right, 0.0),
    ("right", Edge::Right, 0.5),
    ("bottom-right", Edge::Bottom, 0.0),
    ("bottom", Edge::Bottom, 0.5),
    ("bottom-left", Edge::Left, 0.0),
    ("left", Edge::Left, 0.5),
];

/// A start point: one of the eight names, or `<edge>:<at>` — a side
/// (`top|right|bottom|left`) and a fraction along it, clockwise.
pub(crate) fn start(text: &str) -> Result<StartPoint, String> {
    let lower = text.to_ascii_lowercase();
    if let Some(&(_, edge, at)) = NAMED_STARTS.iter().find(|(name, _, _)| *name == lower) {
        return Ok(StartPoint { edge, at });
    }
    let bad = || {
        format!(
            "'{text}' is not a start point: `top-left`, `top`, … `left`, or `<top|right|bottom|left>:<at>`"
        )
    };
    let (edge, at) = lower.split_once(':').ok_or_else(bad)?;
    let edge = match edge {
        "top" => Edge::Top,
        "right" => Edge::Right,
        "bottom" => Edge::Bottom,
        "left" => Edge::Left,
        _ => return Err(bad()),
    };
    Ok(StartPoint {
        edge,
        at: number(at).map_err(|_| bad())?,
    })
}

/// `cw` or `ccw`.
pub(crate) fn direction(text: &str) -> Result<Direction, String> {
    match text.to_ascii_lowercase().as_str() {
        "cw" => Ok(Direction::Clockwise),
        "ccw" => Ok(Direction::CounterClockwise),
        _ => Err(format!("'{text}' is not a direction: `cw` or `ccw`")),
    }
}

/// `left`, `center` or `right`.
pub(crate) fn align(text: &str) -> Result<Align, String> {
    match text.to_ascii_lowercase().as_str() {
        "left" => Ok(Align::Left),
        "center" => Ok(Align::Center),
        "right" => Ok(Align::Right),
        _ => Err(format!(
            "'{text}' is not an alignment: `left`, `center` or `right`"
        )),
    }
}

/// A shape: `ellipse`, `rectangle` (sharp corners) or
/// `rectangle:<radius>`, the radius a percentage of half the shorter
/// side.
pub(crate) fn shape(text: &str) -> Result<Shape, String> {
    let bad = || format!("'{text}' is not a shape: `ellipse`, `rectangle` or `rectangle:<0-100>`");
    let lower = text.to_ascii_lowercase();
    match lower.split_once(':') {
        None if lower == "ellipse" => Ok(Shape::Ellipse),
        None if lower == "rectangle" => Ok(rectangle(0)),
        Some(("rectangle", radius)) => Ok(rectangle(percent(radius).map_err(|_| bad())?)),
        _ => Err(bad()),
    }
}

/// A rectangle with corner radius `radius` percent.
pub(crate) fn rectangle(radius: u8) -> Shape {
    Shape::Rectangle {
        radius: baumhard::document::values::Percent::new(radius),
    }
}

/// A percentage, `0`…`100`. Refused above 100 rather than clamped, as a
/// file's is.
pub(crate) fn percent(text: &str) -> Result<u8, String> {
    text.parse::<u8>()
        .ok()
        .filter(|&v| v <= 100)
        .ok_or_else(|| format!("'{text}' is not a percentage from 0 to 100"))
}

/// How much of its half each line of a ring covers: `natural`, `whole`,
/// or a fraction from 0 to 1.
pub(crate) fn line_span(text: &str) -> Result<LineSpan, String> {
    let bad = || format!("'{text}' is not a span: `natural`, `whole` or a number from 0 to 1");
    match text.to_ascii_lowercase().as_str() {
        "natural" => Ok(LineSpan::NATURAL),
        "whole" => Ok(LineSpan::WHOLE),
        other => LineSpan::new(number(other).map_err(|_| bad())?).ok_or_else(bad),
    }
}

/// The ornament in a ring's breaks: its text, or `false` for none.
pub(crate) fn sides(text: &str) -> Result<Sides, String> {
    match text {
        "false" => Ok(Sides::Off),
        "" => Err("an ornament is not empty: give its text, or `false`".to_string()),
        content => Ok(Sides::On {
            content: content.to_string(),
            color: None,
        }),
    }
}

/// A zoom bound: a positive number, or `unset`.
pub(crate) fn zoom_bound(text: &str) -> Result<Option<f32>, String> {
    if text.eq_ignore_ascii_case("unset") || text.is_empty() {
        return Ok(None);
    }
    number(text)
        .ok()
        .filter(|v| *v > 0.0)
        .map(Some)
        .ok_or_else(|| format!("'{text}' is not a zoom: a positive number or `unset`"))
}
