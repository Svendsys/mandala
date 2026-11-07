// SPDX-License-Identifier: MPL-2.0

//! `zoom` — refused until V1-C5 (#246) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::{Bare, Form, Grammar, Key, Subverb, Vocabulary, Word};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

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

/// Refuses, naming the issue that rebuilds this verb.
fn execute_zoom(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("zoom", super::FIELD_NODES_OWNER)
}
