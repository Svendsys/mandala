// SPDX-License-Identifier: MPL-2.0

//! `spacing` — refused until V1-C6 (#247) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::{bare_words, free_words, Bare, Form, Grammar, Key, Word};
use crate::application::console::{ConsoleEffects, ExecResult};

pub const VALUE_PRESETS: &[&str] = &["tight", "normal", "wide"];
const PRESET_WORDS: &[Word] = &bare_words::<3>(VALUE_PRESETS);

const KEYS: &[Key] = &[Key::new(
    "value",
    "glyph spacing: a preset or a float",
    free_words("float", PRESET_WORDS),
)];

pub static GRAMMAR: Grammar = Grammar {
    label: "spacing",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::keys(&["value"], &[])])),
};

pub const COMMAND: Command = Command {
    name: "spacing",
    aliases: &[],
    summary: "Set the glyph spacing of the selected edge",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "tight", "wide"],
    execute: execute_spacing,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_spacing(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("spacing", super::FIELD_PATHS_OWNER)
}
