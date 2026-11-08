// SPDX-License-Identifier: MPL-2.0

//! `section` — refused until V1-C5 (#246) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

pub(crate) mod grammar;

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::node_or_section_selected_single_node;
use crate::application::console::{ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "section",
    aliases: &[],
    summary: "Inspect, move, resize, edit text, or structurally modify a section (add / delete / split)",
    // Section subverbs need a single node target, so a multiple
    // selection is excluded.
    applicable: node_or_section_selected_single_node,
    grammar: &grammar::SECTION,
    // Every subverb and key is derived. `info`, `offset`, `size`,
    // `border` and `preset` are the words a user greps for that the
    // grammar does not contain — the last two reach the `frame`
    // subject's vocabulary, which lives one level down under
    // `border`'s own declaration.
    synonyms: &["info", "offset", "size", "border", "preset", "glyph"],
    execute: execute_section,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_section(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("section", super::FIELD_NODES_OWNER)
}
