// SPDX-License-Identifier: MPL-2.0

//! Edge-mutating apply_* helpers — anchor / body-glyph / cap /
//! type / display-mode / reset edits and edge-label text /
//! position edits, all applied across the current selection.
//! Each delegates to a `console::commands::*::apply_*_to_selection`
//! mutation core through the shared `apply_with_rebuild` envelope
//! so a `true` from the core triggers a geometry-change rebuild.

use super::{apply_with_rebuild, RebuildContext};

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

pub(in crate::application::app) fn apply_set_edge_body_glyph(
    preset: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::body::apply_body_glyph_to_selection(doc, preset)
    });
}

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

pub(in crate::application::app) fn apply_set_edge_type(
    edge_type: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_type_to_selection(doc, edge_type)
    });
}

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

pub(in crate::application::app) fn apply_reset_edge(
    kind: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_reset_to_selection(doc, kind)
    });
}

pub(in crate::application::app) fn apply_set_edge_label_text(
    text: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::label::apply_label_text_to_selection(doc, text)
    });
}

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
