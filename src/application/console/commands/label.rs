// SPDX-License-Identifier: MPL-2.0

//! `label text="hi"` / `label clear` / `label position=middle` —
//! edge label operations.
//!
//! `text=` routes through the `HasLabel` trait (edge-only today).
//! `position=` is edge-specific and therefore handled outside the
//! trait layer — if the selection isn't an edge the pair reports
//! not-applicable. `clear` is the positional form of `text=<empty>`.
//! `edit` is a positional verb that hands off to the inline label
//! editor modal.
//!
//! **Rebuilt on the one writer by V1-C6 (#247).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::path_selected;
use crate::application::console::spec::{
    bare_words, free, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleEffects, ExecResult};

pub const POSITIONS: &[&str] = &["start", "middle", "end"];
const POSITION_WORDS: &[Word] = &bare_words::<3>(POSITIONS);

const KEYS: &[Key] = &[
    Key::new("text", "label text (quote multi-word values)", free("text")),
    Key::new(
        "position",
        "anchor the label at the start, middle or end of the edge",
        Vocabulary::Words(POSITION_WORDS),
    ),
    Key::new(
        "position_t",
        "fractional position along the edge, 0.0 … 1.0",
        free("f32"),
    ),
    Key::new(
        "perpendicular",
        "offset away from the edge, in canvas units",
        free("f32"),
    ),
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("edit", "editor", "open the inline single-line editor"),
    Subverb::bare("clear", "reset", "drop the label text"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "label",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["text", "position", "position_t", "perpendicular"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Edit, clear, reposition, or offset the selected edge's label",
    applicable: path_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "offset", "drag"],
    execute: execute_label,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_label(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("label", super::FIELD_PATHS_OWNER)
}
