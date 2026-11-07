// SPDX-License-Identifier: MPL-2.0

//! `zoom min=<pt|unset> max=<pt|unset>` / `zoom clear` — set or
//! clear the zoom-visibility window on the current selection.
//!
//! Orthogonal to `font min=/max=` (which writes screen-space font
//! clamps): this command writes `min_zoom_to_render` /
//! `max_zoom_to_render` — the presence gate controlling whether
//! an element is rendered at all at the current camera zoom.
//!
//! Routing against the active selection:
//! - `Node`: writes `MindNode.{min,max}_zoom_to_render`.
//! - `Edge`: writes `MindEdge.{min,max}_zoom_to_render`.
//! - `EdgeLabel`: writes `label_config.{min,max}_zoom_to_render`
//!   (replace-not-intersect cascade vs. edge).
//! - `PortalLabel`: writes the owning edge's top-level pair —
//!   the icon inherits edge bounds, same posture as `font`.
//! - `PortalText`: writes `PortalEndpointState.{min,max}_zoom_to_render`
//!   (replace-not-intersect cascade vs. edge).
//! - `Multi`: fans out over each node id.
//!
//! **Rebuilt on the one writer by V1-C5 (#246).** Until then the verb's
//! grammar, help and completion stand, and running it refuses with
//! that issue named ([`super::rebuilt_by`]).

use baumhard::util::geometry::is_positive_finite;

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{kvs, usage, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word};
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
