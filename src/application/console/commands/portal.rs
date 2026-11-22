// SPDX-License-Identifier: MPL-2.0

//! `portal end=<0|1> …` — one end of each selected portal link, on the
//! one writer (V1-C6, #247; design §5.6, §6.1). One line is one request,
//! so one undo entry, however many portals are selected.
//!
//! End `0` is the source end, beside the link's node; end `1` the target
//! end, beside the node it leads to. `border_t` places it around its
//! node's outline (0 to 1), `offset` sets its distance from it, and
//! `content`, `size` and `color` its marker: the text it draws when it
//! has no spans, or `none` to inherit. `link kind=portal` makes a link a
//! portal.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::link_selected;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::{free_words, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::path::{self, PortalFields};
use crate::application::fields::values;

const RESET: &[Word] = &[Word::new("reset", "inherit")];
const NONE: &[Word] = &[Word::new("none", "inherit the marker")];
const ENDS: &[Word] = &[
    Word::new("0", "the source end, beside the link's node"),
    Word::new("1", "the target end, beside the node it leads to"),
];

const KEYS: &[Key] = &[
    Key::new(
        "end",
        "which end: 0 the source, 1 the target",
        Vocabulary::Words(ENDS),
    ),
    Key::new(
        "border_t",
        "its place around its node's outline, 0-1",
        free_words("0-1", RESET),
    ),
    Key::new("offset", "its distance from the outline", free_words("px", RESET)),
    Key::new(
        "content",
        "the marker it draws when it has no spans",
        free_words("text", NONE),
    ),
    Key::new("size", "its marker's size, in points", free_words("pt", RESET)),
    Key::new("color", "its marker's color", super::color::COLOR_VOCAB),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "portal",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "ends",
        &[Form::keys(
            &["end"],
            &["border_t", "offset", "content", "size", "color"],
        )],
    )),
};

pub const COMMAND: Command = Command {
    name: "portal",
    aliases: &[],
    summary: "Place one end of the selected portal links, and set its marker",
    applicable: link_selected,
    grammar: &GRAMMAR,
    synonyms: &["marker", "portal_end"],
    execute: execute_portal,
};

fn execute_portal(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    let Some(end) = kvs::value(&pairs, "end") else {
        return ExecResult::err("portal: give end=0 (the source) or end=1 (the target)");
    };
    let steps = values::portal_end(end).and_then(|end| {
        let fields = fields(eff, &pairs)?;
        path::portal(eff.document(), end, &fields)
    });
    super::edit(eff, steps, "portal")
}

/// The fields a `portal` line's keys set, `end` aside, each from its
/// last occurrence (`kvs::value`).
fn fields(eff: &ConsoleEffects, pairs: &[Pair]) -> Result<PortalFields, String> {
    let read = |key: &str| kvs::value(pairs, key);
    let number = |value| values::or_reset(value, values::number);
    Ok(PortalFields {
        border_t: read("border_t").map(number).transpose()?,
        offset: read("offset").map(number).transpose()?,
        content: read("content").map(values::content),
        size: read("size").map(number).transpose()?,
        color: read("color")
            .map(|v| values::color(eff.document().doc(), v))
            .transpose()?,
    })
}
