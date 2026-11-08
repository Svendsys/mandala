// SPDX-License-Identifier: MPL-2.0

//! `node` — refused until V1-C5 (#246) rebuilds it on the one writer.
//!
//! The verb's grammar parses, and help and tab completion read it;
//! running it changes nothing and answers with that issue named
//! ([`super::rebuilt_by`]). What the verb will do is its owner's to
//! specify.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::{free, Form, Grammar, Slot, Subverb};
use crate::application::console::{ConsoleEffects, ExecResult};

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("resize", "geometry", "pin the node's width and height")
        .taking(&[Form::slots(&[Slot::req(free("w")), Slot::req(free("h"))])]),
    Subverb::bare("fit", "geometry", "shrink the node to its content"),
    Subverb::bare("edit", "editor", "enter node-edit mode on the selection"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "node",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "node",
    aliases: &[],
    summary: "Resize the selected node, fit it to its content, or enter node-edit mode",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["size", "shrink", "content", "node-edit"],
    execute: execute_node,
};

/// Refuses, naming the issue that rebuilds this verb.
fn execute_node(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    super::rebuilt_by("node", super::FIELD_NODES_OWNER)
}
