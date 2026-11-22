// SPDX-License-Identifier: MPL-2.0

//! The spellings a console line and a keybind's argument give a field's
//! value, read once into the document's typed values (design §5.3) as
//! the line is run. Where the file has a spelling the console takes the
//! file's, read by the format's own readers (`format::color`,
//! `format::font`, `format::start`, `format::fit`, …), so a name the
//! file gains is a name the console takes. Where the file's spelling is
//! a JSON object, a short form stands for it as well (`fit=once:·`,
//! `start=top:0.25`, `shape=rectangle:25`), turned into that object and
//! read by the same reader; and `direction` takes `cw` and `ccw`. `reset`
//! is the console's word for "set nothing here": the field inherits, as
//! a key a file leaves out does.

use baumhard::document::format;
use baumhard::document::ids::Document;
use baumhard::document::values::{
    Align, Anchor, ColorRef, Direction, FitMode, FontRef, LineSpan, LinkKind, Shape, Sides, StartPoint,
    Weight,
};

/// The word that clears a field so it inherits.
pub(crate) const RESET: &str = "reset";

/// The word that sets no text where a path, a cap or a portal end takes
/// one: a path or an end then inherits its content, and a cap is
/// removed.
pub(crate) const NONE: &str = "none";

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

/// A font family a map may name, as the loader reads one — a compiled
/// face, or an installed family, whether or not this machine has it
/// (#329) — or [`RESET`].
pub(crate) fn font(text: &str) -> Result<Option<FontRef>, String> {
    or_reset(text, |text| {
        format::font(text).map_err(|fault| format!("'{text}': {fault}"))
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

/// A weight as the file spells one: `normal`, `bold`, or `100`…`900` in
/// steps of 100 (`format::weight`).
pub(crate) fn weight(text: &str) -> Result<Weight, String> {
    format::weight(text)
        .ok_or_else(|| format!("'{text}' is not a weight: `normal`, `bold` or 100…900 in steps of 100"))
}

/// A fit mode: the file's spellings (`format::fit`: `once`, `repeat`,
/// `spread`, `fit`, and its objects as JSON), or the console's short
/// forms `once:<filler>` and `fit:<min>,<max>`, held to the same bounds.
pub(crate) fn fit(text: &str) -> Result<FitMode, String> {
    let bad = || {
        format!("'{text}' is not a fit mode: `once[:<filler>]`, `repeat`, `spread` or `fit[:<min>,<max>]`")
    };
    if let Some(fit) = format::fit(text) {
        return Ok(fit);
    }
    let (head, rest) = text.split_once(':').ok_or_else(bad)?;
    let object = match head {
        "once" => serde_json::json!({"once": {"filler": rest}}),
        "fit" => {
            let (min, max) = rest.split_once(',').ok_or_else(bad)?;
            serde_json::json!({"fit": {"min": number(min.trim())?, "max": number(max.trim())?}})
        }
        _ => return Err(bad()),
    };
    format::fit(&object.to_string()).ok_or_else(bad)
}

/// A start point: the file's spellings (`format::start`: the eight
/// names, or its object as JSON), or the console's short form
/// `<edge>:<at>` — a side and a fraction along it, clockwise.
pub(crate) fn start(text: &str) -> Result<StartPoint, String> {
    let bad = || {
        format!(
            "'{text}' is not a start point: `top-left`, `top`, … `left`, or `<top|right|bottom|left>:<at>`"
        )
    };
    if let Some(start) = format::start(text) {
        return Ok(start);
    }
    let (edge, at) = text.split_once(':').ok_or_else(bad)?;
    format::edge(edge).ok_or_else(bad)?;
    let object = serde_json::json!({"edge": edge, "at": number(at).map_err(|_| bad())?});
    format::start(&object.to_string()).ok_or_else(bad)
}

/// A direction: the console's short forms `cw` and `ccw`, or the file's
/// `clockwise` and `counterclockwise` (`format::direction`).
pub(crate) fn direction(text: &str) -> Result<Direction, String> {
    match text {
        "cw" => Ok(Direction::Clockwise),
        "ccw" => Ok(Direction::CounterClockwise),
        _ => format::direction(text).ok_or_else(|| format!("'{text}' is not a direction: `cw` or `ccw`")),
    }
}

/// An alignment as the file spells one: `left`, `center` or `right`
/// (`format::align`).
pub(crate) fn align(text: &str) -> Result<Align, String> {
    format::align(text).ok_or_else(|| format!("'{text}' is not an alignment: `left`, `center` or `right`"))
}

/// A shape: the file's spellings (`format::shape`: `ellipse`, or its
/// object as JSON), or the console's short forms `rectangle` (sharp
/// corners) and `rectangle:<radius>`, the radius a percentage of half
/// the shorter side.
pub(crate) fn shape(text: &str) -> Result<Shape, String> {
    let bad = || format!("'{text}' is not a shape: `ellipse`, `rectangle` or `rectangle:<0-100>`");
    if let Some(shape) = format::shape(text) {
        return Ok(shape);
    }
    let radius = match text.split_once(':') {
        None if text == "rectangle" => "0",
        Some(("rectangle", radius)) => radius,
        _ => return Err(bad()),
    };
    rectangle(radius).ok_or_else(bad)
}

/// A rectangle with corner radius `radius` percent, as the file reads
/// one; `None` past 100 or not a whole number.
pub(crate) fn rectangle(radius: &str) -> Option<Shape> {
    let radius: u32 = radius.parse().ok()?;
    format::shape(&serde_json::json!({"rectangle": {"radius": radius}}).to_string())
}

/// How much of its half each line of a ring covers, as the file spells
/// it: `natural`, `whole`, or a fraction from 0 to 1
/// (`format::line_span`).
pub(crate) fn line_span(text: &str) -> Result<LineSpan, String> {
    format::line_span(text)
        .ok_or_else(|| format!("'{text}' is not a span: `natural`, `whole` or a number from 0 to 1"))
}

/// The ornament in a ring's breaks as the file spells it
/// (`format::sides`): its text, `false` for none, or
/// `{"content": …, "color": …}` with a color of `doc`'s.
pub(crate) fn sides(doc: &Document, text: &str) -> Result<Sides, String> {
    format::sides(doc, text).ok_or_else(|| {
        format!("'{text}' is not sides: an ornament's text, `false`, or {{\"content\": …, \"color\": …}}")
    })
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

/// Text a path, a cap or a portal end draws, or `None` for [`NONE`],
/// spelled exactly so: any other text — `None` and `NONE` among it — is
/// taken as it is, since content is text (D8). Empty text is the
/// caller's to refuse, in the words of the key it was given to.
pub(crate) fn content(text: &str) -> Option<String> {
    (text != NONE).then(|| text.to_string())
}

/// Where a path attaches, as the file spells it: `auto`, `top`, `right`,
/// `bottom` or `left` (`format::anchor`).
pub(crate) fn anchor(text: &str) -> Result<Anchor, String> {
    format::anchor(text)
        .ok_or_else(|| format!("'{text}' is not an anchor: `auto`, `top`, `right`, `bottom` or `left`"))
}

/// A path's two anchors, its start's and its end's, as `<from>,<to>`.
pub(crate) fn anchors(text: &str) -> Result<(Anchor, Anchor), String> {
    let (from, to) = text.split_once(',').ok_or_else(|| {
        format!("'{text}' is not two anchors: `<from>,<to>`, each `auto`, `top`, `right`, `bottom` or `left`")
    })?;
    Ok((anchor(from.trim())?, anchor(to.trim())?))
}

/// What a link draws, as the file spells it: `line` or `portal`
/// (`format::link_kind`).
pub(crate) fn link_kind(text: &str) -> Result<LinkKind, String> {
    format::link_kind(text).ok_or_else(|| format!("'{text}' is not a link kind: `line` or `portal`"))
}

/// A label's place along its path: a number from 0 (its start) to 1 (its
/// end), or `start`, `middle` and `end` for 0, 0.5 and 1. The bound is
/// the writer's, whose refusal names the field.
pub(crate) fn label_t(text: &str) -> Result<f32, String> {
    match text.to_ascii_lowercase().as_str() {
        "start" => Ok(0.0),
        "middle" => Ok(0.5),
        "end" => Ok(1.0),
        _ => number(text)
            .map_err(|_| format!("'{text}' is not a place along the path: 0-1, `start`, `middle` or `end`")),
    }
}

/// Which end of a portal: `0`, the source end beside the link's node, or
/// `1`, the target end.
pub(crate) fn portal_end(text: &str) -> Result<u8, String> {
    match text {
        "0" => Ok(0),
        "1" => Ok(1),
        _ => Err(format!(
            "'{text}' is not a portal end: `0` (the source) or `1` (the target)"
        )),
    }
}
