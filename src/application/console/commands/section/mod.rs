// SPDX-License-Identifier: MPL-2.0

//! `section …` — kv-form per-section verbs targeting either the
//! selection's section (when the selection is
//! `SelectionState::Section` / `SectionRange`) or an explicit
//! `section=K` kv (when the selection is a single node). Subverbs
//! (per `work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md` §4.5):
//!
//! - `section show [section=<idx>]` — multi-line resolved-property
//!   readout (text preview / runs / offset / size / channel /
//!   bindings / frame override).
//! - `section move dx=<f64> dy=<f64>` (delta) or
//!   `section move x=<f64> y=<f64>` (absolute) — delta and
//!   absolute forms are mutually exclusive; mixing rejects.
//! - `section resize w=<f64> h=<f64>` or `section resize fill` —
//!   the `fill` literal renames the prior `none` (which read as
//!   "remove the section"); `fill` clears `size = None` so the
//!   tree builder fills the parent's AABB.
//! - `section text "<text>" [runs=preserve|clear]` — replace
//!   text with optional run handling.
//! - `section add [at=<idx>] [text="<text>"]` — insert.
//! - `section delete [section=<idx>]` — remove.
//! - `section split [section=<idx>] at=<grapheme>` — split in
//!   two at a grapheme boundary.
//!
//! The `section frame …` level is gone with the old per-section
//! border: a frame is the node's, and V1-C5 (#246) gives it its own
//! verb, `frame`.
//!
//! **Rebuilt on the one writer by V1-C5 (#246).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

pub(crate) mod grammar;

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::node_or_section_selected_single_node;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::kvs::Pair;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{kvs, usage, Descent};
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
