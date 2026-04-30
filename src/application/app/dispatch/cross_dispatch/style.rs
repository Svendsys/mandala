// SPDX-License-Identifier: MPL-2.0

//! Style-affecting apply_* helpers — border / color / font /
//! spacing edits applied to the current selection. Each delegates
//! to a `console::commands::*::apply_*_to_selection` mutation
//! core, then routes through the shared `apply_with_rebuild`
//! envelope so a `true` from the core triggers a geometry-change
//! scene rebuild.

use super::{apply_with_rebuild, RebuildContext};

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

pub(in crate::application::app) fn apply_set_font_family(
    family: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_family_to_selection(doc, family)
    });
}

/// `pt` is already-parsed (the dispatcher's caller is responsible
/// for parsing the user-facing `String` payload — invalid floats
/// emit a warn-log and skip the helper call entirely).
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

pub(in crate::application::app) fn apply_set_spacing(
    input: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::spacing::apply_spacing_to_selection(doc, input)
    });
}
