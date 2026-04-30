// SPDX-License-Identifier: MPL-2.0

//! Free-fn body helpers reachable from closures passed into
//! [`super::super::MindMapDocument::mutate_edge`] without having
//! to capture `Self`. The corresponding methods on
//! [`super::super::MindMapDocument`] (`ensure_glyph_connection`,
//! `ensure_label_config`) delegate here.

use baumhard::mindmap::model::{
    Canvas, EdgeLabelConfig, GlyphConnectionConfig, MindEdge, PortalEndpointState,
};

/// Free-fn body of [`MindMapDocument::ensure_glyph_connection`].
/// Reachable from closures passed into
/// [`MindMapDocument::mutate_edge`] without having to capture
/// `Self`. Forks the canvas-default connection style onto the
/// edge on first edit; subsequent edits reuse the per-edge copy.
/// Must be called AFTER the caller has snapshotted the edge into
/// an `UndoAction::EditEdge { before, .. }` so the undo entry
/// still carries the pre-fork `None`.
pub(super) fn ensure_glyph_connection_inline<'a>(
    edge: &'a mut MindEdge,
    canvas: &Canvas,
) -> &'a mut GlyphConnectionConfig {
    if edge.glyph_connection.is_none() {
        let seed = canvas
            .default_connection
            .clone()
            .unwrap_or_default();
        edge.glyph_connection = Some(seed);
    }
    edge.glyph_connection.as_mut().expect("just installed")
}

/// Free-fn body of [`MindMapDocument::ensure_label_config`].
/// Reachable from closures passed into
/// [`MindMapDocument::mutate_edge`]. Forks a default
/// [`EdgeLabelConfig`] onto the edge on first label edit;
/// subsequent edits reuse it. Mirrors
/// [`ensure_glyph_connection_inline`] for the label channel.
pub(super) fn ensure_label_config_inline(edge: &mut MindEdge) -> &mut EdgeLabelConfig {
    edge.label_config.get_or_insert_with(EdgeLabelConfig::default)
}

/// Write `value` to the `Option<T>` field on the
/// `PortalEndpointState` slot, lazily forking a default state on
/// `Some` and scrubbing the slot back to `None` on `Some(field) ->
/// None` clears that leave the state entirely default.
///
/// `setter` is given `(&mut state, value)` and writes the field
/// directly (`s.color = v`, `s.text = v`, etc.). The "scrub when
/// default" rollback ensures unchanged selections leave no undo
/// droppings — the rule the pre-helper portal setters each
/// hand-rolled with the same `if existing == &PortalEndpointState::
/// default() { *slot = None; }` block.
///
/// Caller is responsible for the equality / no-op short-circuit
/// before invoking — this helper unconditionally writes. Pairs
/// with [`MindMapDocument::mutate_edge`]: the closure does
/// `current == new_val? false : { write_endpoint_field(...); true }`.
pub(super) fn write_endpoint_field<T, S>(
    slot: &mut Option<PortalEndpointState>,
    value: Option<T>,
    setter: S,
) where
    S: FnOnce(&mut PortalEndpointState, Option<T>),
{
    match value {
        Some(v) => {
            let s = slot.get_or_insert_with(PortalEndpointState::default);
            setter(s, Some(v));
        }
        None => {
            if let Some(existing) = slot.as_mut() {
                setter(existing, None);
                if existing == &PortalEndpointState::default() {
                    *slot = None;
                }
            }
        }
    }
}
