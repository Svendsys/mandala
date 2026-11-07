// SPDX-License-Identifier: MPL-2.0

//! `color` — refused until V1-C5 (#246) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

use super::range_kv::TARGET_KEYS;
use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::{
    bare_words, free_words, Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleEffects, ExecResult};

pub const VALUE_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];
const PRESET_WORDS: &[Word] = &bare_words::<4>(VALUE_PRESETS);

/// A color value: any `#hex` or `var(--name)`, plus the four preset
/// names the theme resolves.
const COLOR_VOCAB: Vocabulary = free_words("#hex|var(--name)", PRESET_WORDS);

const ON_OFF: &[Word] = &[
    Word::new("on", "open the picker as a persistent standalone palette"),
    Word::new("off", "close any open picker"),
];

const KEYS: &[Key] = &[
    Key::new("bg", "fill / background color", COLOR_VOCAB),
    Key::new("text", "text / label color", COLOR_VOCAB),
    Key::new("border", "frame / line color", COLOR_VOCAB),
];

/// The three axes are subverbs as well as keys: `color bg` opens the
/// glyph wheel on that axis, `color bg=#fff` writes it directly. The
/// two vocabularies are separate namespaces, so one word can be in
/// both — and the popup at the first slot offers each shape once.
const SUBVERBS: &[Subverb] = &[
    Subverb::bare("bg", "picker", "pick a fill color on the glyph wheel").gated(),
    Subverb::bare("text", "picker", "pick a text color on the glyph wheel").gated(),
    Subverb::bare("border", "picker", "pick a frame color on the glyph wheel").gated(),
    Subverb::bare("pick", "picker", "pick a color for the current selection").gated(),
    Subverb::bare("picker", "picker", "open or close the standalone palette")
        .taking(&[Form::slots(&[Slot::req(Vocabulary::Words(ON_OFF))])])
        .gated(),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "color",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS, TARGET_KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["bg", "text", "border", "section", "range"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set bg/text/border color, or pick via the glyph wheel",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["wheel"],
    execute: execute_color,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_color(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("color", super::FIELD_NODES_OWNER)
}
