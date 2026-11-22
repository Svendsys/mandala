// SPDX-License-Identifier: MPL-2.0

//! `link kind=line|portal` — how the selected links are drawn, on the
//! one writer (V1-C6, #247; design §5.3, D22). One line is one request,
//! so one undo entry, however many links are selected.
//!
//! A link made a portal gets its two ends, with nothing set, and loses
//! its spans, caps and label, which an undo gives back; a portal made a
//! line loses its ends. A connector is always a line (D22): it is passed
//! over, and selected alone it is refused. A parent-child relation shown
//! as a portal is a hidden connector (`path visible=false`) plus a
//! portal link to the child.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::link_selected;
use crate::application::console::spec::{kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::path;
use crate::application::fields::values;

const KINDS: &[Word] = &[
    Word::new("line", "a path from node to node"),
    Word::new("portal", "a marker at each end"),
];

const KEYS: &[Key] = &[Key::new(
    "kind",
    "how the link is drawn: line | portal",
    Vocabulary::Words(KINDS),
)];

pub static GRAMMAR: Grammar = Grammar {
    label: "link",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("kind", &[Form::keys(&["kind"], &[])])),
};

pub const COMMAND: Command = Command {
    name: "link",
    aliases: &[],
    summary: "Draw the selected links as lines or as portals",
    applicable: link_selected,
    grammar: &GRAMMAR,
    synonyms: &["cross-link", "display", "connect"],
    execute: execute_link,
};

fn execute_link(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let Some(kind) = kvs::value(&pairs, "kind") else {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    };
    let steps = values::link_kind(kind)
        .map_err(Into::into)
        .and_then(|kind| path::link_kind(eff.document(), kind));
    super::edit(eff, steps, "link kind")
}
