// SPDX-License-Identifier: MPL-2.0

//! `border` — configure a node's glyph border.
//!
//! Selection-aware (per `font` / `color`): operates on the current
//! [`crate::application::document::SelectionState::Single`] /
//! [`crate::application::document::SelectionState::Multi`].
//! Edge-adjacent selections surface a "not applicable to `<kind>`"
//! message — borders are node-only.
//!
//! ## Verbs
//!
//! - `border on` / `border off` — flip `style.show_frame`.
//! - `border show` — multi-line readout of the resolved config.
//! - `border reset` — drop the per-node override.
//! - kv form: `preset=`, `font=`, `size=`, `color=`, `palette=`,
//!   `field=`, `padding=`, `top=`, `bottom=`, `left=`, `right=`,
//!   `tl=`, `tr=`, `bl=`, `br=`. Multiple kvs compose in a single
//!   atomic edit, so `border on preset=heavy size=12 palette=coral`
//!   is one call.
//!
//! See `format/border-patterns.md` for the side-pattern grammar.

use baumhard::mindmap::border::BORDER_PRESETS;

use super::Command;
use crate::application::console::predicates::always;

mod complete;
mod execute;
mod preview;
mod show;

#[cfg(test)]
mod tests;

pub use complete::complete_border;
pub(crate) use complete::{kv_value_completions, preview_subverb_completions};
pub(crate) use execute::apply_border_field_to_selection;
pub use execute::execute_border;
// Re-exported for the `section frame preview …` and
// `canvas border preview …` / `canvas section-frame [focused]
// preview …` verbs. Each verb's `preview` arm wraps
// `dispatch_border_preview` with a target-resolver closure;
// commit / cancel terminator paths route through that helper
// too. The other three preview symbols
// (`cancel_border_preview_verb`, `commit_border_preview_verb`,
// `stage_kv_for_preview`) are private to `border::preview` —
// no downstream consumer reaches in.
pub(crate) use preview::dispatch_border_preview;
// Re-exports consumed by sibling verbs that share the kv vocabulary
// (currently `section frame …` and `canvas …`). All are
// `pub(crate)` on the underlying definitions; the duplication these
// re-exports replaced (three copies each of `kv_hint`,
// `edits_has_glyph_field`, `custom_preset_hint`) violated
// `CODE_CONVENTIONS.md` §5 ("avoid duplicating logic").
pub(crate) use execute::{custom_preset_hint, edits_has_glyph_field, kv_hint, nodes_in_selection, stage_kv};

/// kv keys recognised on the kv-form path.
pub const KEYS: &[&str] = &[
    "preset", "font", "size", "color", "palette", "field", "padding", "top", "bottom", "left", "right", "tl",
    "tr", "bl", "br",
];

/// Positional verbs surfaced as token-0 completions alongside kv
/// keys.
pub const VERBS: &[&str] = &["on", "off", "show", "reset", "preview"];

/// Subverbs surfaced under `border preview` — the
/// commit/cancel terminator pair plus the kv keys (handled
/// through completion's `KvKey` arm). `preview <kv>=…` and
/// `preview commit` / `preview cancel` are siblings.
pub const PREVIEW_SUBVERBS: &[&str] = &["commit", "cancel"];

/// Border preset names — surfaced in completion.
pub const PRESETS: &[&str] = BORDER_PRESETS;

/// Palette field names — surfaced in completion. Mirrors
/// `PaletteField::ALL` but kept here as a `&'static [&'static str]`
/// for `prefix_filter` ergonomics.
pub const FIELDS: &[&str] = &["frame", "background", "text", "title"];

/// Common color preset names mirrored from the `color` command so
/// users can type `border color=accent` and have it resolve the
/// same way.
pub const COLOR_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "border",
    aliases: &[],
    summary: "Configure the node border (preset, font, color, custom glyphs, palette)",
    usage: "border on|off|show|reset | border [preset=…] [font=…] [size=…] [color=…] \
         [palette=…] [field=…] [padding=…] [top=…] [bottom=…] [left=…] [right=…] \
         [tl=…] [tr=…] [bl=…] [br=…] | border preview <kv>=… | border preview commit|cancel",
    tags: &[
        "border", "frame", "glyph", "preset", "corner", "side", "pattern", "palette", "padding", "rounded",
        "heavy", "double", "light", "custom",
    ],
    applicable: always,
    complete: complete_border,
    execute: execute_border,
};
