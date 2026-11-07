// SPDX-License-Identifier: MPL-2.0

//! `cap from=arrow to=none` — set the start/end cap glyph on the
//! selected edge. Edge-specific.
//!
//! **Rebuilt on the one writer by V1-C6 (#247).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{bare_words, kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};

pub const NAMES: &[&str] = &["arrow", "circle", "diamond", "none"];
const CAP_WORDS: &[Word] = &bare_words::<4>(NAMES);

const KEYS: &[Key] = &[
    Key::new(
        "from",
        "cap glyph at the edge's start",
        Vocabulary::Words(CAP_WORDS),
    ),
    Key::new("to", "cap glyph at the edge's end", Vocabulary::Words(CAP_WORDS)),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "cap",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::opt(&["from", "to"])])),
};

pub const COMMAND: Command = Command {
    name: "cap",
    aliases: &[],
    summary: "Set the start/end cap glyph of the selected edge",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "end", "start"],
    execute: execute_cap,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_cap(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("cap", super::FIELD_PATHS_OWNER)
}
