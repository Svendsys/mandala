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

/// Stage a single-kv border preview against the live selection.
/// `target_kind` discriminates between
/// `node` / `section` / `canvas-border` / `canvas-sf` /
/// `canvas-sf-focused`. Always returns `true` for the rebuild
/// envelope (the scene needs to re-emit with the previewed
/// edits visible). Unknown `target_kind` is a no-op + warn.
pub(in crate::application::app) fn apply_set_border_preview(
    target_kind: crate::application::keybinds::BorderPreviewTargetKind,
    field: &str,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        use crate::application::console::commands::border::stage_kv;
        use crate::application::document::{BorderConfigEdits, BorderPreviewTarget};
        use crate::application::keybinds::BorderPreviewTargetKind;

        let mut edits = BorderConfigEdits::default();
        if let Err(msg) = stage_kv(&mut edits, field, value) {
            log::warn!("apply_set_border_preview: stage_kv error: {msg}");
            return false;
        }

        // Resolve `target_kind` → `BorderPreviewTarget`. The
        // selection-bound variants (`Node`, `Section`) walk the
        // live selection via the same resolvers the verb path
        // uses; canvas variants are constants. Typed-enum
        // dispatch — pre-fix this was a stringly-typed match
        // with `unknown` arms warning at runtime.
        let target = match target_kind {
            BorderPreviewTargetKind::Node => {
                let ids = match crate::application::console::commands::border::nodes_in_selection(
                    &doc.selection,
                    "border preview",
                ) {
                    Ok(ids) => ids,
                    Err(_) => return false,
                };
                BorderPreviewTarget::Nodes(ids)
            }
            BorderPreviewTargetKind::Section => {
                let pairs: Vec<(String, usize)> = match &doc.selection {
                    crate::application::document::SelectionState::Section(s) => {
                        vec![(s.node_id.clone(), s.section_idx)]
                    }
                    crate::application::document::SelectionState::SectionRange { sel, range } => {
                        let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
                        (lo..=hi).map(|i| (sel.node_id.clone(), i)).collect()
                    }
                    crate::application::document::SelectionState::MultiSection(sels) => sels
                        .iter()
                        .map(|s| (s.node_id.clone(), s.section_idx))
                        .collect(),
                    _ => return false,
                };
                BorderPreviewTarget::Sections(pairs)
            }
            BorderPreviewTargetKind::CanvasBorder => BorderPreviewTarget::CanvasDefault,
            BorderPreviewTargetKind::CanvasSf => BorderPreviewTarget::CanvasSectionFrame,
            BorderPreviewTargetKind::CanvasSfFocused => BorderPreviewTarget::CanvasSectionFrameFocused,
        };
        let _ = doc.set_border_preview(target, edits);
        true
    });
}

/// Commit the active border preview through the matching
/// committing setter and clear the slot. No-op + no rebuild
/// when no preview is active.
pub(in crate::application::app) fn apply_commit_border_preview(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| doc.commit_border_preview().is_some());
}

/// Cancel the active border preview without writing the model.
/// Returns `true` (rebuild) when a preview was actually cleared
/// — the scene needs to re-emit without the staged edits.
pub(in crate::application::app) fn apply_cancel_border_preview(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| doc.cancel_border_preview());
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
/// `section move dx=<dx> dy=<dy>` for the keybind / macro
/// path.
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
/// when `size = None`). Mirror of `section resize w=<w> h=<h>`
/// / `section resize fill` for the keybind / macro path.
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
