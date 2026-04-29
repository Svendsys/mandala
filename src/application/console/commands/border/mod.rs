// SPDX-License-Identifier: MPL-2.0

//! `border` — configure a node's glyph border.
//!
//! Selection-aware (per `font` / `color`): operates on the current
//! [`SelectionState::Single`] / [`SelectionState::Multi`].
//! Edge-adjacent selections surface a "not applicable to <kind>"
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
mod show;

#[cfg(test)]
mod tests;

pub use complete::complete_border;
pub use execute::execute_border;
pub(crate) use execute::apply_border_field_to_selection;

/// kv keys recognised on the kv-form path.
pub const KEYS: &[&str] = &[
    "preset", "font", "size", "color", "palette", "field", "padding",
    "top", "bottom", "left", "right", "tl", "tr", "bl", "br",
];

/// Positional verbs surfaced as token-0 completions alongside kv
/// keys.
pub const VERBS: &[&str] = &["on", "off", "show", "reset"];

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
    usage:
        "border on|off|show|reset | border [preset=…] [font=…] [size=…] [color=…] \
         [palette=…] [field=…] [padding=…] [top=…] [bottom=…] [left=…] [right=…] \
         [tl=…] [tr=…] [bl=…] [br=…]",
    tags: &[
        "border", "frame", "glyph", "preset", "corner", "side", "pattern",
        "palette", "padding", "rounded", "heavy", "double", "light", "custom",
    ],
    applicable: always,
    complete: complete_border,
    execute: execute_border,
};
