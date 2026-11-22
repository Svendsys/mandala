// SPDX-License-Identifier: MPL-2.0

//! `path` — the selected connectors and line links, on the one writer
//! (V1-C6, #247; design §5.6, §6.1, D8, D13, D34). One line is one
//! request, so one undo entry, however many paths are selected.
//!
//! - `path content=… fit=… spacing=… size=… color=… anchors=…
//!   visible=…` sets those fields. Content is text (D8): `content="·"`
//!   dots the line and `content="━"` makes it heavy (D13); `content=none`
//!   inherits. `anchors=<from>,<to>` names where it attaches at each end;
//! - `path straight` makes each one straight segment, dropping its
//!   control points; `path curve` makes it the default curve again;
//! - `path reset` gives back every field it inherits, the default curve
//!   and `auto` anchors, and keeps its spans, caps, label, visibility and
//!   zoom;
//! - `path cap start=… end=…` sets a cap's text, one span, or removes it
//!   with `none`.
//!
//! A portal link draws no path, so it is passed over
//! (`crate::application::fields::path`). Values are read in the
//! console's spellings (`crate::application::fields::values`); `reset`
//! inherits.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{
    free, free_words, usage, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::path::{self, Geometry, PathFields};
use crate::application::fields::values;

const RESET: &[Word] = &[Word::new("reset", "inherit")];
const NONE: &[Word] = &[Word::new("none", "inherit the content")];
const NO_CAP: &[Word] = &[Word::new("none", "remove the cap")];
const BOOLS: &[Word] = &[Word::bare("true"), Word::bare("false")];

/// The fit modes a path takes, as the file names them.
pub(super) const FITS: &[Word] = &[
    Word::new("once", "the text once; `once:<filler>` fills the rest"),
    Word::new("repeat", "the text repeated along the path"),
    Word::new("spread", "the text once, spread to the ends"),
    Word::new("fit", "the text once, scaled to fit; `fit:<min>,<max>`"),
    Word::new("reset", "inherit"),
];

const KEYS: &[Key] = &[
    Key::new(
        "content",
        "the text it draws when it has no spans",
        free_words("text", NONE),
    ),
    Key::new(
        "fit",
        "what its text does along it",
        free_words("mode", FITS),
    ),
    Key::new(
        "spacing",
        "the extra space between its glyphs",
        free_words("px", RESET),
    ),
    Key::new("size", "its text size, in points", free_words("pt", RESET)),
    Key::new("color", "its text color", super::color::COLOR_VOCAB),
    Key::new(
        "anchors",
        "where it attaches at its start and end, <from>,<to>: each auto, top, right, bottom or left",
        free("from,to"),
    ),
    Key::new("visible", "whether it is drawn", Vocabulary::Words(BOOLS)),
    Key::new(
        "start",
        "the cap at its start: its text, one span",
        free_words("text", NO_CAP),
    ),
    Key::new(
        "end",
        "the cap at its end: its text, one span",
        free_words("text", NO_CAP),
    ),
];

/// The keys the bare form reads, in the order help lists them.
const FIELD_KEYS: &[&str] = &[
    "content", "fit", "spacing", "size", "color", "anchors", "visible",
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("straight", "geometry", "one straight segment, its control points dropped"),
    Subverb::bare("curve", "geometry", "the default curve, its control points cleared"),
    Subverb::bare("reset", "geometry", "every field inherited, the default curve, auto anchors"),
    Subverb::bare("cap", "caps", "set or remove the caps at its ends")
        .taking(&[Form::opt(&["start", "end"])]),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "path",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("fields", &[Form::opt(FIELD_KEYS)])),
};

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Straight,
    Curve,
    Reset,
    Cap,
}

const ROUTES: &Routes<Route> = &[
    ("straight", Route::Straight),
    ("curve", Route::Curve),
    ("reset", Route::Reset),
    ("cap", Route::Cap),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "path",
    aliases: &[],
    summary: "Set the selected connectors' and line links' content, fit, spacing, size, color, anchors and caps; straighten, curve or reset them",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["connector", "edge", "line", "body", "glyph", "dotted", "dashed", "anchor", "arrow"],
    execute: execute_path,
};

fn execute_path(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let subverb = match descent.stop {
        Stop::Matched(subverb) => subverb,
        _ if pairs.is_empty() => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            let steps = fields(eff, &pairs).and_then(|fields| path::set(eff.document(), &fields));
            return super::edit(eff, steps, "path");
        }
    };
    let doc = eff.document();
    let (steps, label) = match route::lookup(ROUTES, subverb) {
        Some(Route::Straight) => (path::geometry(doc, Geometry::Straight), "path straight"),
        Some(Route::Curve) => (path::geometry(doc, Geometry::Curve), "path curve"),
        Some(Route::Reset) => (path::geometry(doc, Geometry::Reset), "path reset"),
        Some(Route::Cap) => {
            let cap = |key| kvs::value(&pairs, key).map(values::content);
            let (start, end) = (cap("start"), cap("end"));
            (
                path::caps(
                    doc,
                    start.as_ref().map(Option::as_deref),
                    end.as_ref().map(Option::as_deref),
                ),
                "path cap",
            )
        }
        None => return ExecResult::err(route::miss(descent.level, subverb)),
    };
    super::edit(eff, steps, label)
}

/// The fields a `path` line's keys set, each from its last occurrence
/// (`kvs::value`).
fn fields(eff: &ConsoleEffects, pairs: &[Pair]) -> Result<PathFields, String> {
    let read = |key: &str| kvs::value(pairs, key);
    let number = |value| values::or_reset(value, values::number);
    Ok(PathFields {
        content: read("content").map(values::content),
        fit: read("fit").map(|v| values::or_reset(v, values::fit)).transpose()?,
        spacing: read("spacing").map(number).transpose()?,
        size: read("size").map(number).transpose()?,
        color: read("color")
            .map(|v| values::color(eff.document().doc(), v))
            .transpose()?,
        anchors: read("anchors").map(values::anchors).transpose()?,
        visible: read("visible").map(values::boolean).transpose()?,
    })
}
