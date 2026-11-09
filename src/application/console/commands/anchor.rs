// SPDX-License-Identifier: MPL-2.0

//! `anchor` — refused until V1-C6 (#247) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::{bare_words, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};

pub const SIDES: &[&str] = &["auto", "top", "right", "bottom", "left"];
const SIDE_WORDS: &[Word] = &bare_words::<5>(SIDES);

const KEYS: &[Key] = &[
    Key::new(
        "from",
        "which side of the source node the edge leaves",
        Vocabulary::Words(SIDE_WORDS),
    ),
    Key::new(
        "to",
        "which side of the target node the edge arrives at",
        Vocabulary::Words(SIDE_WORDS),
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "anchor",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::opt(&["from", "to"])])),
};

pub const COMMAND: Command = Command {
    name: "anchor",
    aliases: &[],
    summary: "Set the from/to anchor side of the selected edge",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "side"],
    execute: execute_anchor,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_anchor(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("anchor", super::FIELD_PATHS_OWNER)
}
