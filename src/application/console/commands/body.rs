// SPDX-License-Identifier: MPL-2.0

//! `body glyph=dash` — set the body glyph of the selected edge.
//! Edge-specific; the concept doesn't generalize beyond edges.
//!
//! **Rebuilt on the one writer by V1-C6 (#247).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::{Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};

/// Body-glyph presets. Kept as `(name, glyph)` pairs so the command
/// table stays one source of truth for both completion and exec.
pub const PRESETS: &[(&str, &str)] = &[
    ("dot", "\u{00B7}"),    // ·
    ("dash", "\u{2500}"),   // ─
    ("double", "\u{2550}"), // ═
    ("wave", "\u{223C}"),   // ∼
    ("chain", "\u{22EF}"),  // ⋯
];

/// The preset names, lifted out of [`PRESETS`] at const-fn time so
/// the popup's vocabulary and the glyph table cannot come apart.
const GLYPH_WORDS: &[Word] = &{
    const N: usize = PRESETS.len();
    let mut out = [Word::bare(""); N];
    let mut i = 0;
    while i < N {
        out[i] = Word::bare(PRESETS[i].0);
        i += 1;
    }
    out
};

const KEYS: &[Key] = &[Key::new(
    "glyph",
    "the repeated glyph the edge body is drawn from",
    Vocabulary::Words(GLYPH_WORDS),
)];

pub static GRAMMAR: Grammar = Grammar {
    label: "body",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::keys(&["glyph"], &[])])),
};

pub const COMMAND: Command = Command {
    name: "body",
    aliases: &[],
    summary: "Set the body glyph of the selected edge",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "style"],
    execute: execute_body,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_body(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("body", super::FIELD_PATHS_OWNER)
}
