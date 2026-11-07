// SPDX-License-Identifier: MPL-2.0

//! `font size=14 [min=8] [max=128]` — atomic font + clamp setter
//! dispatched against the current selection.
//!
//! Parses all three optional kvs up front, then applies them in a
//! single atomic document call so that order-sensitive cases like
//! `font size=14 max=10` land as `size=10, max=10` (min/max write
//! first, then size clamps against the new bounds) instead of the
//! wrong `size=14, max=10`.
//!
//! Routing against the active selection (kv form):
//! - `Node`: `size` sets the node font size; `min`/`max` are
//!   NotApplicable (nodes have no screen-space clamps).
//! - `Edge`: writes `glyph_connection.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}`.
//! - `EdgeLabel`: writes `label_config.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}` so the label can be sized independently of
//!   the edge body.
//! - `PortalLabel`: writes the owning edge's `glyph_connection` —
//!   the icon inherits edge-body clamps and splitting icon clamps
//!   off from the edge is outside the user-level spec.
//! - `PortalText`: writes `PortalEndpointState.{text_font_size_pt,
//!   text_min_font_size_pt, text_max_font_size_pt}` — sibling of
//!   `EdgeLabel` for portal-mode edges.
//!
//! Two positional subverbs sit alongside the kv form:
//! - `font set <family>` — pin the font family on the current
//!   selection via the `AcceptsFontFamily` trait
//!   (`AcceptsFontFamily::set_font_family` on `TargetView`). Each
//!   selection variant decides which channel the family lands on
//!   (nodes → every `TextRun.font`, edges + portal icons →
//!   `glyph_connection.font`); edge labels and portal text return
//!   `NotApplicable` and surface a clear "not applicable to
//!   `<kind>`" message.
//! - `font list` — emit one scrollback line per loaded family,
//!   each rendered in its own face. Sorted alphabetically.
//!
//! **Rebuilt on the one writer by V1-C5 (#246).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

use super::range_kv::TARGET_KEYS;
use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::{
    Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

/// Point sizes the popup suggests. Any positive float is legal, so
/// these are *suggestions* rather than a vocabulary — hence
/// [`Vocabulary::Rows`], whose usage line prints `<pt>` and whose
/// popup prints the list.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

fn size_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    prefix_filter(SIZE_PRESETS, partial)
}

const SIZE_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "pt",
    rows: size_rows,
    sentinels: &[],
};

fn family_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    font_family_completions(partial)
}

const FAMILY_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "family",
    rows: family_rows,
    sentinels: &[],
};

const KEYS: &[Key] = &[
    Key::new("size", "target on-screen size in points", SIZE_VOCAB),
    Key::new("min", "lower screen-space clamp in points", SIZE_VOCAB),
    Key::new("max", "upper screen-space clamp in points", SIZE_VOCAB),
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("set", "family", "pin the font family on the current selection")
        .taking(&[Form::slots(&[Slot::req(FAMILY_VOCAB)]).reading(&["section", "range"])]),
    Subverb::bare(
        "list",
        "family",
        "list every loaded font, each rendered in its face",
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "font",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS, TARGET_KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::keys(&["size"], &["min", "max", "section", "range"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set font family / size / clamps on the selection, or list fonts",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["family", "clamp", "pt", "smaller", "larger"],
    execute: execute_font,
};

/// Build font-family completions: one entry per loaded family
/// whose name starts with `partial` (case-insensitive). Each entry
/// carries `font_family = Some(<name>)` so the popup row shapes the
/// candidate label in that very face. Streams from
/// [`baumhard::font::fonts::loaded_families_iter`] so the
/// keystroke-hot path doesn't allocate a fresh `Vec<String>` per
/// call.
///
/// Family names that contain whitespace are returned with the
/// inserted `text` wrapped in double quotes — the tokenizer would
/// otherwise split `Norse Bold` into two tokens, and the user would
/// hit "not a loaded font" on the first chunk. Quoting at the
/// completion source means tab-accept always produces a parseable
/// command. The `display` row stays unquoted so the popup reads
/// naturally.
///
/// `partial` arrives already-unquoted: the tokenizer
/// (`parser::tokenize`) drops the leading `"` on an unterminated
/// quoted token, so a user mid-typing `font set "Nor` lands here
/// with `partial = "Nor"`, not `"\"Nor"`. No leading-quote
/// stripping is needed.
///
/// This is the console's *only* font-family vocabulary. A second copy
/// once omitted the quoting above, so tab-accepting any of the
/// whitespace-bearing families produced a line the verb then refused
/// with "'DejaVu' is not a loaded font". One body, so two surfaces
/// cannot disagree about it again.
fn font_family_completions(partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families_iter()
        .filter(|f| f.to_ascii_lowercase().starts_with(&partial_lc))
        .map(|family| {
            let needs_quoting = family.chars().any(char::is_whitespace);
            let text = if needs_quoting {
                format!("\"{}\"", family)
            } else {
                family.to_string()
            };
            Completion {
                text,
                display: family.to_string(),
                hint: None,
                font_family: Some(family.to_string()),
            }
        })
        .collect()
}

/// Refuses, naming the issue that rebuilds this verb.
fn execute_font(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("font", super::FIELD_NODES_OWNER)
}
