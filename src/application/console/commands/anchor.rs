// SPDX-License-Identifier: MPL-2.0

//! `anchor from=top to=auto` — edge anchor side setter.
//!
//! Component-specific (edge only); the anchor concept doesn't
//! generalize to nodes or portals, so this bypasses the trait layer
//! and calls `set_edge_anchor` directly.
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
