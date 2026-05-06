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
        crate::application::console::commands::border::apply_border_field_to_selection(doc, field, value)
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
        crate::application::console::commands::color::apply_color_axis_to_selection(doc, axis.into(), value)
    });
}

/// Pin the font family on every node / edge / portal in the
/// current selection. The `family` is validated against the
/// loaded-families table by the mutation core; an unknown name
/// surfaces as a `false` from the helper and the rebuild is
/// skipped. Geometry change (per-glyph advance shifts) → full
/// rebuild.
pub(in crate::application::app) fn apply_set_font_family(family: &str, rc: &mut RebuildContext<'_>) {
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
        crate::application::console::commands::font::apply_font_kv_to_selection(doc, slot.into(), pt)
    });
}

/// Set per-glyph connection-path spacing (the gap between sample
/// glyphs along an edge body) across the current selection from
/// the unparsed `input` string. Mutation core handles parsing +
/// the same is_positive_finite gating every spacing-accepting
/// verb uses. Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_spacing(input: &str, rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::spacing::apply_spacing_to_selection(doc, input)
    });
}

/// Resolve the (node_id, section_idx) the section-targeted
/// Action variants apply to. Section / SectionRange use their
/// inner SectionSel; MultiSection collapses to the first.
/// `Single` is rejected — section verbs require an explicit
/// section selection (matches the console verb path's contract;
/// a node may have its title at section 0 and body at section 1
/// and a silent "default to 0" would nudge the wrong section).
fn target_section(
    sel: &crate::application::document::SelectionState,
) -> Option<(String, usize)> {
    if let Some(s) = sel.selected_section() {
        return Some((s.node_id.clone(), s.section_idx));
    }
    if let Some(first) = sel.selected_sections().first() {
        return Some((first.node_id.clone(), first.section_idx));
    }
    None
}

/// Nudge the selected section by `(dx, dy)` canvas units.
/// AABB rejection on overflow surfaces as a `log::warn!` and
/// no-op (the model setter returns `Err`). Mirror of
/// `section move <dx> <dy>` for the keybind / macro path.
pub(in crate::application::app) fn apply_set_section_offset_delta(
    dx: f64,
    dy: f64,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionOffsetDelta: no section selected");
        return;
    };
    let (cx, cy) = match rc
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .and_then(|n| n.sections.get(idx))
        .map(|s| (s.offset.x, s.offset.y))
    {
        Some(p) => p,
        None => {
            log::warn!("SetSectionOffsetDelta: section[{}] not found on node '{}'", idx, node_id);
            return;
        }
    };
    apply_with_rebuild(rc, |doc| {
        match doc.set_section_offset(&node_id, idx, cx + dx, cy + dy) {
            Ok(changed) => changed,
            Err(msg) => {
                log::warn!("SetSectionOffsetDelta: {}", msg);
                false
            }
        }
    });
}

/// Pin the selected section's size to `(w, h)` (or fill-parent
/// when `size = None`). Mirror of `section resize <w> <h>` /
/// `section resize none` for the keybind / macro path.
pub(in crate::application::app) fn apply_set_section_size(
    size: Option<baumhard::mindmap::model::Size>,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionSize: no section selected");
        return;
    };
    let exists = rc
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.get(idx).is_some())
        .unwrap_or(false);
    if !exists {
        log::warn!("SetSectionSize: section[{}] not found on node '{}'", idx, node_id);
        return;
    }
    apply_with_rebuild(rc, |doc| {
        match doc.set_section_size(&node_id, idx, size) {
            Ok(changed) => changed,
            Err(msg) => {
                log::warn!("SetSectionSize: {}", msg);
                false
            }
        }
    });
}
