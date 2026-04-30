// SPDX-License-Identifier: MPL-2.0

//! Edge-mutating apply_* helpers — anchor / body-glyph / cap /
//! type / display-mode / reset edits and edge-label text /
//! position edits, all applied across the current selection.
//! Each delegates to a `console::commands::*::apply_*_to_selection`
//! mutation core through the shared `apply_with_rebuild` envelope
//! so a `true` from the core triggers a geometry-change rebuild.

use super::{apply_with_rebuild, RebuildContext};

/// Set both endpoints' anchor side (`top` / `bottom` / `left` /
/// `right` / `center`) on every edge in the current selection.
/// Geometry change → full rebuild via `apply_with_rebuild`.
pub(in crate::application::app) fn apply_set_edge_anchor(
    from: &str,
    to: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::anchor::apply_anchor_to_selection(
            doc,
            Some(from),
            Some(to),
        )
    });
}

/// Stamp the named body-glyph preset (`solid` / `dashed` / etc.)
/// on every edge in the current selection. Geometry change →
/// full rebuild.
pub(in crate::application::app) fn apply_set_edge_body_glyph(
    preset: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::body::apply_body_glyph_to_selection(doc, preset)
    });
}

/// Set both endpoints' arrow-cap glyph (`arrow` / `none` /
/// preset name) on every edge in the current selection. Geometry
/// change → full rebuild.
pub(in crate::application::app) fn apply_set_edge_cap(
    from: &str,
    to: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::cap::apply_cap_to_selection(
            doc,
            Some(from),
            Some(to),
        )
    });
}

/// Switch every edge in the current selection to the named
/// `edge_type` (`parent_child` / `cross_link`). Topology
/// change → full rebuild.
pub(in crate::application::app) fn apply_set_edge_type(
    edge_type: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_type_to_selection(doc, edge_type)
    });
}

/// Switch every edge in the current selection to the named
/// `display_mode` (`line` / `portal`). Visual change → full
/// rebuild.
pub(in crate::application::app) fn apply_set_edge_display_mode(
    mode: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_display_mode_to_selection(
            doc, mode,
        )
    });
}

/// Reset the named edge attribute kind (`anchor` / `cap` /
/// `body` / `color` / `font` / `label`) to its default value
/// across the current selection. Visual change → full rebuild.
pub(in crate::application::app) fn apply_reset_edge(
    kind: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_reset_to_selection(doc, kind)
    });
}

/// Set the label text on every edge in the current selection.
/// Empty `text` clears the label. Visual change → full rebuild.
pub(in crate::application::app) fn apply_set_edge_label_text(
    text: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::label::apply_label_text_to_selection(doc, text)
    });
}

/// Set the label position along each edge in the current
/// selection (`start` / `middle` / `end` / numeric `t`).
/// Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_edge_label_position(
    position: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::label::apply_label_position_to_selection(
            doc, position,
        )
    });
}
