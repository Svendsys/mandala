// SPDX-License-Identifier: MPL-2.0

//! Style-affecting apply_* helpers — border / color / font /
//! spacing edits applied to the current selection. Each delegates
//! to a `console::commands::*::apply_*_to_selection` mutation
//! core, then routes through the shared `apply_with_rebuild`
//! envelope so a `true` from the core triggers a geometry-change
//! scene rebuild.

use super::{apply_with_rebuild, RebuildContext};

/// Set a named border field (e.g. `top` / `bottom` / `corner`)
/// to a parsed `value` across the current selection. Border-glyph
/// change → full rebuild via `apply_with_rebuild`.
pub(in crate::application::app) fn apply_set_border_field(
    field: &str,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::border::apply_border_field_to_selection(
            doc, field, value,
        )
    });
}

/// Set a single colour axis (`bg` / `frame` / `text` / `title`
/// — see [`crate::application::keybinds::ColorAxis`]) to a
/// hex / palette / `var(--name)` value across the current
/// selection. Visual change → full rebuild.
pub(in crate::application::app) fn apply_set_color_axis(
    axis: crate::application::keybinds::ColorAxis,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::color::apply_color_axis_to_selection(
            doc,
            axis.into(),
            value,
        )
    });
}

/// Pin the font family on every node / edge / portal in the
/// current selection. The `family` is validated against the
/// loaded-families table by the mutation core; an unknown name
/// surfaces as a `false` from the helper and the rebuild is
/// skipped. Geometry change (per-glyph advance shifts) → full
/// rebuild.
pub(in crate::application::app) fn apply_set_font_family(
    family: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_family_to_selection(doc, family)
    });
}

/// Set one font-size kv (`size` / `min` / `max` — see
/// [`crate::application::keybinds::FontSlot`]) across the
/// current selection. `pt` is already-parsed (the dispatcher's
/// caller is responsible for parsing the user-facing `String`
/// payload — invalid floats emit a warn-log and skip the
/// helper call entirely). Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_font_kv(
    slot: crate::application::keybinds::FontSlot,
    pt: f32,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_kv_to_selection(
            doc,
            slot.into(),
            pt,
        )
    });
}

/// Set per-glyph connection-path spacing (the gap between sample
/// glyphs along an edge body) across the current selection from
/// the unparsed `input` string. Mutation core handles parsing +
/// the same is_positive_finite gating every spacing-accepting
/// verb uses. Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_spacing(
    input: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::spacing::apply_spacing_to_selection(doc, input)
    });
}
