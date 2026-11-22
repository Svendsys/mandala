// SPDX-License-Identifier: MPL-2.0

//! `label` — the labels of the selected connectors and line links, on
//! the one writer (V1-C6, #247; design §5.6, §6.1). One line is one
//! request, so one undo entry, however many paths are selected.
//!
//! - `label text=… t=… offset=…` sets each label's text, one span —
//!   giving a path with no label one — its place along the path (`t`,
//!   0 to 1, or `start`, `middle`, `end`) and its distance from it;
//!   without `text=` only the paths that have a label are reached;
//! - `label delete` removes them.
//!
//! A portal link's labels are its ends': editing them belongs with the
//! span editors (V1-C8, #249), so a portal link alone is refused, and
//! one beside a path is passed over.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{free, free_words, usage, Bare, Form, Grammar, Key, Subverb, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::path::{self, LabelFields};
use crate::application::fields::values;

const RESET: &[Word] = &[Word::new("reset", "inherit")];
const PLACES: &[Word] = &[
    Word::new("start", "at the path's start, 0"),
    Word::new("middle", "half way along, 0.5"),
    Word::new("end", "at the path's end, 1"),
    Word::new("reset", "inherit"),
];

const KEYS: &[Key] = &[
    Key::new(
        "text",
        "the label's text, one span (quote multi-word values)",
        free("text"),
    ),
    Key::new(
        "t",
        "its place along the path: 0-1, start, middle or end",
        free_words("0-1", PLACES),
    ),
    Key::new(
        "offset",
        "its distance from the path, in canvas units",
        free_words("px", RESET),
    ),
];

const SUBVERBS: &[Subverb] = &[Subverb::bare("delete", "remove", "remove the label")];

pub static GRAMMAR: Grammar = Grammar {
    label: "label",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("fields", &[Form::opt(&["text", "t", "offset"])])),
};

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Delete,
}

const ROUTES: &Routes<Route> = &[("delete", Route::Delete)];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Set the text, place and offset of the selected paths' labels, or delete them",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["connector", "edge", "caption", "position"],
    execute: execute_label,
};

fn execute_label(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    match descent.stop {
        Stop::Matched(subverb) => match route::lookup(ROUTES, subverb) {
            Some(Route::Delete) => {
                let steps = path::delete_label(eff.document());
                super::edit(eff, steps, "label delete")
            }
            None => ExecResult::err(route::miss(descent.level, subverb)),
        },
        _ if pairs.is_empty() => ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            let steps = fields(&pairs)
                .map_err(Into::into)
                .and_then(|fields| path::label(eff.document(), &fields));
            super::edit(eff, steps, "label")
        }
    }
}

/// The fields a `label` line's keys set, each from its last occurrence
/// (`kvs::value`).
fn fields(pairs: &[Pair]) -> Result<LabelFields, String> {
    let read = |key: &str| kvs::value(pairs, key);
    Ok(LabelFields {
        text: read("text").map(str::to_string),
        t: read("t")
            .map(|v| values::or_reset(v, values::label_t))
            .transpose()?,
        offset: read("offset")
            .map(|v| values::or_reset(v, values::number))
            .transpose()?,
    })
}
