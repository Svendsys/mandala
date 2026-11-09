// SPDX-License-Identifier: MPL-2.0

//! `zoom min=<n> max=<n>` and `zoom clear` — the camera zooms the
//! selected nodes, connectors and links draw at, on the one writer
//! (V1-C5, #246): one line is one request, so one undo entry. A bound
//! set keeps the other bound each element has; `unset` opens one;
//! `clear` opens both (`crate::application::fields::style::zoom`).

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{usage, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::fields::{style, values};

/// Zoom levels the popup suggests. The camera clamps to
/// `[0.05, 5.0]`, so values outside that range are accepted but
/// will never match a real camera zoom.
///
/// These are *suggestions*, not a vocabulary: any positive float is
/// legal, which is why the usage line prints `<zoom|unset>` rather
/// than this list. [`Vocabulary::Rows`] is the shape that says so.
pub const VALUE_PRESETS: &[&str] = &["0.25", "0.5", "1.0", "1.5", "2.0", "3.0", "5.0"];

const UNSET: &[Word] = &[Word::new("unset", "remove the bound")];

fn zoom_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    prefix_filter(VALUE_PRESETS, partial)
}

const ZOOM_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "zoom",
    rows: zoom_rows,
    sentinels: UNSET,
};

const KEYS: &[Key] = &[
    Key::new("min", "lower inclusive zoom bound (or `unset`)", ZOOM_VOCAB),
    Key::new("max", "upper inclusive zoom bound (or `unset`)", ZOOM_VOCAB),
];

const SUBVERBS: &[Subverb] = &[Subverb::bare(
    "clear",
    "reset",
    "drop both bounds so the selection always renders",
)];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Clear,
}

const ROUTES: &Routes<Route> = &[("clear", Route::Clear)];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "zoom",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("bounds", &[Form::opt(&["min", "max"])])),
};

pub const COMMAND: Command = Command {
    name: "zoom",
    aliases: &["visibility"],
    summary: "Gate the selection's rendering on camera zoom level",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &[
        "visibility",
        "presence",
        "render",
        "clamp",
        "unset",
        "layer",
        "lod",
    ],
    execute: execute_zoom,
};

fn execute_zoom(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if let Stop::Matched(subverb) = descent.stop {
        return match route::lookup(ROUTES, subverb) {
            Some(Route::Clear) => {
                let steps = style::zoom(eff.document(), style::Zoom::CLEAR);
                super::edit(eff, steps, "zoom clear")
            }
            None => ExecResult::err(route::miss(descent.level, subverb)),
        };
    }
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    // Both bounds are one window per element, in one request.
    let mut change = style::Zoom::default();
    for pair in &pairs {
        let value = match values::zoom_bound(pair.value) {
            Ok(value) => Some(value),
            Err(msg) => return ExecResult::err(msg),
        };
        if pair.key.name == "min" {
            change.min = value;
        } else {
            change.max = value;
        }
    }
    let steps = style::zoom(eff.document(), change);
    super::edit(eff, steps, "zoom")
}
