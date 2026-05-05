// SPDX-License-Identifier: MPL-2.0

//! Cursor-move dispatch. Owns drag-state transitions (Pending →
//! Panning / SelectingRect / Throttled(...) — where the throttled
//! variants are MovingNode, MovingSection, SectionResize,
//! NodeResize, EdgeHandle, PortalLabel, EdgeLabel), Reparent /
//! Connect hover highlights, and the button-cursor swap for
//! trigger-bearing nodes.

#![cfg(not(target_arch = "wasm32"))]

use winit::dpi::PhysicalPosition;
use winit::window::{CursorIcon, Window};

use super::click::rebuild_all_with_mode;
use super::color_picker_flow::handle_color_picker_mouse_move;
use super::input_context::InputHandlerContext;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use super::throttled_interaction::{
    EdgeHandleInteraction, EdgeLabelInteraction, MovingNodeInteraction, MovingSectionInteraction,
    NodeResizeInteraction, PortalLabelInteraction, SectionResizeInteraction, ThrottledDrag,
};
use super::{AppMode, DragState};
use crate::application::common::RenderDecree;
use crate::application::document::{apply_tree_highlights, hit_test, SelectionState};

pub(super) fn handle_cursor_moved(
    position: PhysicalPosition<f64>,
    window: &Window,
    ctx: &mut InputHandlerContext<'_>,
) {
    let prev_pos = *ctx.cursor_pos;
    *ctx.cursor_pos = (position.x, position.y);
    let cursor_pos_val = *ctx.cursor_pos;

    // Glyph-wheel color picker hover preview. Routes
    // mouse-over to the picker hit-test, updates the
    // current HSV in place, and lives-previews the
    // change on the affected edge/portal.
    //
    // Guard on `DragState::None`: if a canvas-side
    // drag is already in flight, do not route the
    // move to the picker.
    if ctx.color_picker_state.is_open() && matches!(*ctx.drag_state, DragState::None) {
        let consumed = if let Some(doc) = ctx.document.as_mut() {
            handle_color_picker_mouse_move(cursor_pos_val, ctx.color_picker_state, doc, ctx.picker_hover)
        } else {
            true
        };
        if consumed {
            return;
        }
    }

    // Reparent or Connect mode: hit-test under cursor to update the hover
    // target highlight. Skip the regular drag-state handling.
    if matches!(*ctx.app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
        let new_hover = ctx.mindmap_tree.as_mut().and_then(|tree| {
            let canvas_pos = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            hit_test(canvas_pos, tree)
        });
        if new_hover != *ctx.hovered_node {
            *ctx.hovered_node = new_hover;
            if let Some(doc) = ctx.document.as_ref() {
                rebuild_all_with_mode(
                    doc,
                    ctx.app_mode,
                    ctx.hovered_node.as_deref(),
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
        }
        return;
    }

    // Hand cursor over button-like nodes (nodes with any
    // trigger bindings). Only recomputed when idle — during
    // a drag the cursor should stay as-is.
    if matches!(*ctx.drag_state, DragState::None) {
        let over_button = match (ctx.document.as_ref(), ctx.mindmap_tree.as_mut()) {
            (Some(doc), Some(tree)) => {
                let canvas_pos = ctx
                    .renderer
                    .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                hit_test(canvas_pos, tree)
                    .and_then(|id| doc.mindmap.nodes.get(&id))
                    .map(|n| !n.trigger_bindings.is_empty())
                    .unwrap_or(false)
            }
            _ => false,
        };
        if over_button != *ctx.cursor_is_hand {
            window.set_cursor(if over_button {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
            *ctx.cursor_is_hand = over_button;
        }
    }

    match ctx.drag_state {
        DragState::Panning => {
            let dx = cursor_pos_val.0 - prev_pos.0;
            let dy = cursor_pos_val.1 - prev_pos.1;
            ctx.renderer
                .process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
        }
        DragState::Throttled(ThrottledDrag::MovingNode(i)) => {
            // Per-frame mutation + rebuild happens in AboutToWait
            // behind `ThrottledInteraction::drive`'s adaptive gate.
            let delta = canvas_delta(ctx.renderer, prev_pos, cursor_pos_val);
            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::MovingSection(i)) => {
            let delta = canvas_delta(ctx.renderer, prev_pos, cursor_pos_val);
            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::EdgeHandle(i)) => {
            let delta = canvas_delta(ctx.renderer, prev_pos, cursor_pos_val);
            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::SectionResize(i)) => {
            let delta = canvas_delta(ctx.renderer, prev_pos, cursor_pos_val);
            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::NodeResize(i)) => {
            let delta = canvas_delta(ctx.renderer, prev_pos, cursor_pos_val);
            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::EdgeLabel(i)) => {
            // Overwrite discipline: store the latest cursor —
            // `EdgeLabelInteraction::drain` projects it onto the
            // edge path at consume time, so intermediate cursors
            // carry no information the projection needs.
            let cursor_canvas = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            i.pending_cursor = Some(cursor_canvas);
        }
        DragState::Throttled(ThrottledDrag::PortalLabel(i)) => {
            // Overwrite discipline, same as `EdgeLabel` —
            // `PortalLabelInteraction::drain` snaps to the node
            // border at consume time.
            let cursor_canvas = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            i.pending_cursor = Some(cursor_canvas);
        }
        DragState::Pending {
            start_pos,
            hit_node,
            hit_section_idx,
            hit_edge_handle,
            hit_portal_label,
            hit_edge_label,
            hit_section_resize_handle,
            hit_node_resize_handle,
        } => {
            let dist_x = cursor_pos_val.0 - start_pos.0;
            let dist_y = cursor_pos_val.1 - start_pos.1;
            if dist_x * dist_x + dist_y * dist_y > 25.0 {
                // Past threshold — promote `Pending` to the
                // appropriate drag variant. At most one of
                // `hit_edge_label` / `hit_portal_label` is set
                // at press time (see `event_mouse_click.rs`'s
                // click-hit chain), so the ordering here only
                // resolves the `hit_edge_handle`-vs-`hit_node`
                // overlap — a handle sits above its edge's
                // nodes, and a handle-grab drag should always
                // beat the node behind it. Consumption order:
                //   edge-label → portal-label → edge-handle →
                //   node (move) → shift-rect-select → pan.
                // Portal-text is intentionally missing: dragging
                // a portal's text sub-part isn't a supported
                // gesture — the icon carries the drag.
                // Specific-gesture arms (edge-label / portal-label /
                // edge-handle / node-resize-handle / section-resize-
                // handle) each abort with `return` on validation miss
                // rather than fall through to less-specific arms.
                // The user pressed a handle; if the handle's target
                // is gone (deleted mid-press, mutated through the
                // console, etc.), aborting the promotion is the
                // honest UX — falling through to MovingNode would
                // silently pivot the gesture from resize to move.
                if let Some(edge_key) = hit_edge_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        if let Some(original) =
                            doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned()
                        {
                            // Capture `prev` BEFORE the assignment —
                            // post-write capture would always read
                            // the new EdgeLabel selection back, so
                            // `rebuild_after_selection_change` would
                            // see prev == new and pick scene-only
                            // even from a `Single(node)` start. The
                            // node's tree-text highlight would then
                            // stay painted cyan through the drag.
                            let prev = doc.selection.clone();
                            doc.selection = SelectionState::EdgeLabel(
                                crate::application::document::EdgeLabelSel::new(edge_ref.clone()),
                            );
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::Throttled(ThrottledDrag::EdgeLabel(
                                EdgeLabelInteraction::new(edge_ref, original),
                            ));
                            // `rebuild_after_selection_change` picks
                            // `rebuild_scene_only` when both the
                            // previous and new selections are edge-
                            // adjacent (no node-tree highlight to
                            // shift). When the user was on a node
                            // before and drag-starts an edge-label
                            // in the same gesture, falls back to a
                            // full rebuild to clear the old node
                            // highlight from the tree's text buffer.
                            rebuild_after_selection_change(
                                &prev,
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                            return;
                        }
                    }
                    // EdgeLabel hit consumed, validation missed
                    // (edge deleted mid-press / `as_mut` failed) —
                    // abort rather than fall through to MovingNode.
                    return;
                }
                if let Some((edge_key, endpoint)) = hit_portal_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        let original = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned();
                        if let Some(original) = original {
                            doc.selection =
                                SelectionState::PortalLabel(crate::application::document::PortalLabelSel {
                                    edge_key,
                                    endpoint_node_id: endpoint.clone(),
                                });
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::Throttled(ThrottledDrag::PortalLabel(
                                PortalLabelInteraction::new(edge_ref, endpoint, original),
                            ));
                            rebuild_all(
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                            return;
                        }
                    }
                    // PortalLabel hit consumed, validation missed
                    // — abort rather than fall through.
                    return;
                }
                if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                    // Grab the pre-edit snapshot + start
                    // position so the drain loop can do
                    // absolute-positioning math.
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(original) =
                            doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned()
                        {
                            let canvas_pos = ctx
                                .renderer
                                .screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                            let start_handle_pos = doc
                                .hit_test_edge_handle(canvas_pos, &edge_ref, f32::INFINITY)
                                .map(|(_, p)| p)
                                .unwrap_or(canvas_pos);
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::Throttled(ThrottledDrag::EdgeHandle(
                                EdgeHandleInteraction::new(edge_ref, handle_kind, original, start_handle_pos),
                            ));
                            return;
                        }
                    }
                    // EdgeHandle hit consumed, validation missed
                    // — abort rather than fall through.
                    return;
                }
                if let Some((node_id, side)) = hit_node_resize_handle.take() {
                    // Snapshot the node's pre-drag (position, size)
                    // so the drain math + release commit derive
                    // from a stable base. Skip if the node vanished
                    // between press and threshold (deletion mid-
                    // press) or had its size go non-finite /
                    // non-positive (selection-gating means we
                    // shouldn't see this; defensive against
                    // model mutations through the console mid-
                    // press).
                    if let Some(doc) = ctx.document.as_ref() {
                        if let Some(node) = doc.mindmap.nodes.get(&node_id) {
                            if node.size.width.is_finite()
                                && node.size.height.is_finite()
                                && node.size.width > 0.0
                                && node.size.height > 0.0
                            {
                                let start_position = node.position;
                                let start_size = node.size;
                                ctx.scene_cache.clear();
                                *ctx.drag_state = DragState::Throttled(ThrottledDrag::NodeResize(
                                    NodeResizeInteraction::new(
                                        node_id,
                                        side,
                                        start_position,
                                        start_size,
                                    ),
                                ));
                                return;
                            }
                        }
                    }
                    // NodeResize handle consumed, validation missed
                    // (node deleted / non-finite size) — abort
                    // rather than fall through to MovingNode on
                    // the same `hit_node`.
                    return;
                }
                if let Some((node_id, section_idx, side)) = hit_section_resize_handle.take() {
                    // Snapshot the section's pre-drag offset/size
                    // so the drain math + release commit derive
                    // from a stable base. Skip the gesture if the
                    // section vanished between press and threshold
                    // (deletion mid-drag) or its size went `None`
                    // (selection-gating means we shouldn't see this
                    // in practice, but the per-frame dispatch
                    // shouldn't crash on a model the user mutated
                    // through the console mid-press).
                    if let Some(doc) = ctx.document.as_ref() {
                        if let Some(node) = doc.mindmap.nodes.get(&node_id) {
                            if let Some(section) = node.sections.get(section_idx) {
                                if let Some(start_size) = section.size {
                                    let start_offset = section.offset;
                                    ctx.scene_cache.clear();
                                    *ctx.drag_state = DragState::Throttled(ThrottledDrag::SectionResize(
                                        SectionResizeInteraction::new(
                                            node_id,
                                            section_idx,
                                            side,
                                            start_offset,
                                            start_size,
                                        ),
                                    ));
                                    return;
                                }
                            }
                        }
                    }
                    // SectionResize handle consumed, validation
                    // missed — abort rather than fall through to
                    // MovingNode/MovingSection on the same press.
                    return;
                }
                if let Some(node_id) = hit_node.take() {
                    // Capture the pre-demote owning-node set
                    // before any selection mutation below
                    // narrows it. Used by the shift+drag harvest
                    // — without this snapshot, the section /
                    // multi-section demote that runs on whole-
                    // node drag promotion would shrink the set
                    // to one before the harvest reads it,
                    // silently dropping every other selected
                    // node from the drag scope.
                    let pre_demote_ids: Vec<String> = ctx
                        .document
                        .as_ref()
                        .map(|d| d.selection.dedup_owning_node_ids())
                        .unwrap_or_default();

                    // Multi-section + non-shift hits drag only the
                    // pressed section's offset; everything else
                    // (single-section, shift-multi-select) falls
                    // through to whole-node drag, mirroring
                    // `hit_test_target`'s single-section fold.
                    if let Some((section_idx, ox, oy)) = resolve_section_drag_target(
                        ctx.document.as_ref(),
                        &node_id,
                        *hit_section_idx,
                        ctx.modifiers.shift_key(),
                    ) {
                        if let Some(doc) = ctx.document.as_mut() {
                            if let Some(new_sel) = selection_after_section_drag_press(
                                &doc.selection,
                                &node_id,
                                section_idx,
                            ) {
                                doc.selection = new_sel;
                                rebuild_selection_highlight(doc, ctx.mindmap_tree, ctx.renderer);
                            }
                        }
                        ctx.scene_cache.clear();
                        *ctx.drag_state = DragState::Throttled(ThrottledDrag::MovingSection(
                            MovingSectionInteraction::new(node_id, section_idx, (ox, oy)),
                        ));
                        return;
                    }
                    // Whole-node drag fall-through: ensure the
                    // dragged node is selected as a *node*, not a
                    // section sub-selection. A Section selection
                    // on the dragged node satisfies `is_selected`
                    // but leaves the picker hint and per-section
                    // verbs reading "Section[K]" while the user
                    // bodily moves the parent — incoherent. Demote
                    // a same-node Section selection to Single
                    // here so mid-drag UX matches the gesture
                    // (release rebuild lands the same coherent
                    // shape).
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(new_sel) =
                            selection_after_node_drag_press(&doc.selection, &node_id)
                        {
                            doc.selection = new_sel;
                            rebuild_selection_highlight(doc, ctx.mindmap_tree, ctx.renderer);
                        }
                    }
                    // Shift+drag: move all selected nodes together.
                    // Read from the pre-demote snapshot — the
                    // demote above may have just narrowed the
                    // doc selection to `Single(node_id)`, which
                    // would silently drop every other node from
                    // a `Multi` / `MultiSection` set out of the
                    // drag scope.
                    let node_ids = if ctx.modifiers.shift_key() {
                        let mut ids = pre_demote_ids;
                        if !ids.contains(&node_id) {
                            ids.push(node_id);
                        }
                        ids
                    } else {
                        vec![node_id]
                    };
                    // Start each drag with a clean scene cache so
                    // the keyed-edge rebuild picks up the moving
                    // node's edges from scratch.
                    ctx.scene_cache.clear();
                    *ctx.drag_state = DragState::Throttled(ThrottledDrag::MovingNode(
                        MovingNodeInteraction::new(node_ids, ctx.modifiers.alt_key()),
                    ));
                } else if ctx.modifiers.shift_key() {
                    // Shift+drag on empty space: rubber-band selection
                    let start_canvas = ctx
                        .renderer
                        .screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                    let current_canvas = ctx
                        .renderer
                        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                    *ctx.drag_state = DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    };
                } else {
                    // LeftDrag-on-empty pan. Honour the user's
                    // PanCanvas binding: if they unbound LeftDrag from
                    // PanCanvas (or rebound it elsewhere), the pan
                    // doesn't fire. Default `KeybindConfig::default()`
                    // ships with `pan_canvas: ["LeftDrag", "MiddleClick"]`
                    // so out-of-the-box behaviour is unchanged.
                    // `action_for_gesture` falls back to unmodified
                    // when no exact-modifier binding exists, so
                    // Ctrl+LeftDrag-on-empty pans like a bare
                    // LeftDrag-on-empty did pre-branch. Only
                    // `PanCanvas` is dispatched via this shortcut;
                    // future Actions bound to `LeftDrag` won't fire
                    // here without explicit handling.
                    let leftdrag_pans = ctx.keybinds.action_for_gesture(
                        crate::application::keybinds::MouseGesture::LeftDrag.key_name(),
                        ctx.modifiers.control_key(),
                        ctx.modifiers.shift_key(),
                        ctx.modifiers.alt_key(),
                    ) == Some(crate::application::keybinds::Action::PanCanvas);
                    if leftdrag_pans {
                        *ctx.drag_state = DragState::Panning;
                        let dx = cursor_pos_val.0 - prev_pos.0;
                        let dy = cursor_pos_val.1 - prev_pos.1;
                        ctx.renderer
                            .process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                    }
                }
            }
        }
        DragState::SelectingRect { current_canvas, .. } => {
            *current_canvas = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
        }
        DragState::None => {}
    }
}

/// Compute the canvas-space delta between two screen positions.
/// Used by every accumulating drag arm; the camera transform
/// (zoom + pan) lives in the renderer, so a screen → canvas
/// conversion at both ends is the only honest way to derive a
/// delta that survives an interleaved camera pan.
fn canvas_delta(
    renderer: &crate::application::renderer::Renderer,
    prev: (f64, f64),
    curr: (f64, f64),
) -> glam::Vec2 {
    let prev_canvas = renderer.screen_to_canvas(prev.0 as f32, prev.1 as f32);
    let curr_canvas = renderer.screen_to_canvas(curr.0 as f32, curr.1 as f32);
    curr_canvas - prev_canvas
}

/// Decide whether a press on `node_id` with `hit_section_idx` and
/// the given shift modifier should promote to section drag rather
/// than the default whole-node drag. Returns `Some((idx, ox, oy))`
/// when the section's offset can be captured for the drag
/// `start_offset`; `None` when the press should fall through to
/// the existing whole-node path.
///
/// The multi-section + non-shift gate mirrors `hit_test_target`'s
/// single-section fold: single-section nodes never produce a
/// `Section` hit; shift is reserved for multi-node selection so
/// shift+drag-on-section deliberately falls through to whole-node.
pub(super) fn resolve_section_drag_target(
    doc: Option<&crate::application::document::MindMapDocument>,
    node_id: &str,
    hit_section_idx: Option<usize>,
    shift: bool,
) -> Option<(usize, f64, f64)> {
    if shift {
        return None;
    }
    let idx = hit_section_idx?;
    let node = doc?.mindmap.nodes.get(node_id)?;
    if node.sections.len() <= 1 {
        return None;
    }
    node.sections.get(idx).map(|s| (idx, s.offset.x, s.offset.y))
}

/// Decide what the selection should become when a section drag
/// promotes from `Pending` to `Throttled(MovingSection)`. Returns
/// `Some(new_sel)` when the selection needs to be rewritten;
/// `None` when the press lands exactly on the already-selected
/// `Section(node_id, section_idx)` and no rewrite is needed.
///
/// **Demote discipline.** The gesture is "move this section",
/// so the selection narrows to a single `Section`. A pre-existing
/// `MultiSection` set IS demoted — mirroring the whole-node arm
/// (`selection_after_node_drag_press`) which collapses the same
/// multi-section state to `Single(node_id)`. Without the demote
/// the picker hint + per-section verbs would read out a
/// multi-section state mid-drag while the user bodily moves a
/// single section.
pub(super) fn selection_after_section_drag_press(
    prev: &SelectionState,
    node_id: &str,
    section_idx: usize,
) -> Option<SelectionState> {
    let target = crate::application::document::SectionSel {
        node_id: node_id.to_string(),
        section_idx,
    };
    if matches!(prev, SelectionState::Section(s) if s == &target) {
        return None;
    }
    Some(SelectionState::Section(target))
}

/// Decide what the selection should become when a whole-node
/// drag promotes from `Pending` to `Throttled(MovingNode)`.
/// Returns `Some(new_sel)` when the selection needs rewriting;
/// `None` to keep the existing selection (the dragged node is
/// already part of a `Multi(ids)` / `Single(node_id)` set).
///
/// `Section` / `MultiSection` selections that touch the dragged
/// node demote to `Single(node_id)` — the gesture is "move this
/// node", not "operate on these sections".
pub(super) fn selection_after_node_drag_press(
    prev: &SelectionState,
    node_id: &str,
) -> Option<SelectionState> {
    let needs_demote = match prev {
        SelectionState::Section(s) => s.node_id == node_id,
        SelectionState::MultiSection(secs) => secs.iter().any(|s| s.node_id == node_id),
        _ => false,
    };
    if needs_demote || !prev.is_selected(node_id) {
        Some(SelectionState::Single(node_id.to_string()))
    } else {
        None
    }
}

/// Rebuild the tree's selection highlight from the current
/// `doc.selection`. Shared by the section-drag and whole-node
/// drag promotion arms — both set selection then need the tree
/// + renderer buffers refreshed to reflect the new highlight.
fn rebuild_selection_highlight(
    doc: &mut crate::application::document::MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut crate::application::renderer::Renderer,
) {
    if let Some(tree) = mindmap_tree.as_mut() {
        let mut new_tree = doc.build_tree();
        // Routes through the canonical
        // `selection_highlight_entries` helper — Section /
        // MultiSection narrow the highlight to the selected
        // sections, whole-node selections paint every section.
        let highlights = super::scene_rebuild::selection_highlight_entries(&doc.selection);
        apply_tree_highlights(&mut new_tree, highlights);
        renderer.rebuild_buffers_from_tree(&new_tree.tree);
        *tree = new_tree;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_section_drag_target, selection_after_node_drag_press,
        selection_after_section_drag_press,
    };
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
    use crate::application::document::{SectionSel, SelectionState};

    /// Multi-section node + non-shift + valid section_idx → drag
    /// the section. Pins the threshold-cross promotion gate.
    #[test]
    fn resolve_section_drag_target_multi_section_non_shift_returns_some() {
        let (doc, id) = pinned_two_section_node();
        let result = resolve_section_drag_target(Some(&doc), &id, Some(1), false);
        assert!(result.is_some(), "multi-section + non-shift must promote");
        let (idx, _, _) = result.unwrap();
        assert_eq!(idx, 1);
    }

    /// `section_idx=0` on a multi-section node also drags — the
    /// gate is `sections.len() > 1`, not `idx > 0`. Closes the
    /// review's test gap.
    #[test]
    fn resolve_section_drag_target_section_zero_on_multi_section_returns_some() {
        let (doc, id) = pinned_two_section_node();
        let result = resolve_section_drag_target(Some(&doc), &id, Some(0), false);
        assert!(result.is_some(), "section_idx=0 on multi-section must promote");
    }

    /// Single-section node falls to whole-node drag — mirrors
    /// `hit_test_target`'s single-section fold to `NodeContainer`.
    #[test]
    fn resolve_section_drag_target_single_section_returns_none() {
        let mut doc = load_test_doc();
        // `first_testament_node_id` returns a node that may have
        // multiple sections under some test orderings; force
        // single-section by clearing extras.
        let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
        if let Some(n) = doc.mindmap.nodes.get_mut(&nid) {
            n.sections.truncate(1);
        }
        let result = resolve_section_drag_target(Some(&doc), &nid, Some(0), false);
        assert!(result.is_none(), "single-section node must NOT promote");
    }

    /// Shift+drag-on-section falls to whole-node drag (multi-
    /// select discipline). Pins the second half of the gate.
    #[test]
    fn resolve_section_drag_target_shift_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let result = resolve_section_drag_target(Some(&doc), &id, Some(1), true);
        assert!(result.is_none(), "shift+drag must fall to whole-node");
    }

    /// Out-of-range section index → fall-through (no panic, no
    /// mis-promotion).
    #[test]
    fn resolve_section_drag_target_out_of_range_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let result = resolve_section_drag_target(Some(&doc), &id, Some(99), false);
        assert!(result.is_none());
    }

    /// `None` document or `None` hit_section_idx → fall-through.
    #[test]
    fn resolve_section_drag_target_no_doc_or_idx_returns_none() {
        assert!(resolve_section_drag_target(None, "0", Some(0), false).is_none());
        let (doc, id) = pinned_two_section_node();
        assert!(resolve_section_drag_target(Some(&doc), &id, None, false).is_none());
    }

    // ── Selection-after-press helpers ────────────────────────────

    /// Pressing on the already-selected `Section(node, idx)` does
    /// not rewrite the selection — pins the no-op early-out so a
    /// section drag started on its own selection doesn't trigger
    /// a redundant tree highlight rebuild.
    #[test]
    fn section_drag_press_on_already_selected_section_returns_none() {
        let prev = SelectionState::Section(SectionSel::new("a", 1));
        assert!(selection_after_section_drag_press(&prev, "a", 1).is_none());
    }

    /// **Demote-on-press for MultiSection.** Pressing on a section
    /// that lives inside a `MultiSection` set demotes the entire
    /// set down to a single `Section`. Pins the parallel of the
    /// whole-node-arm demote (a multi-section selection on the
    /// dragged node demotes to `Single(node_id)`).
    #[test]
    fn section_drag_press_demotes_multisection_to_section() {
        let prev = SelectionState::MultiSection(vec![
            SectionSel::new("a", 0),
            SectionSel::new("a", 1),
            SectionSel::new("b", 0),
        ]);
        let new = selection_after_section_drag_press(&prev, "a", 1).expect("rewrite");
        match new {
            SelectionState::Section(s) => assert_eq!(s, SectionSel::new("a", 1)),
            other => panic!("expected Section, got {:?}", other),
        }
    }

    /// `Section(node, k)` press on a different `(node, j)` pair
    /// rewrites the selection to the new pair — narrows from one
    /// section to another when the user clicks a sibling section.
    #[test]
    fn section_drag_press_rewrites_when_different_section() {
        let prev = SelectionState::Section(SectionSel::new("a", 0));
        let new = selection_after_section_drag_press(&prev, "a", 1).expect("rewrite");
        assert!(matches!(
            new,
            SelectionState::Section(s) if s == SectionSel::new("a", 1)
        ));
    }

    /// Whole-node press on a `Single(node)` matching the dragged
    /// id is a no-op — the dragged node is already the selected
    /// node, no rewrite + no highlight churn needed.
    #[test]
    fn node_drag_press_on_single_same_node_returns_none() {
        let prev = SelectionState::Single("a".into());
        assert!(selection_after_node_drag_press(&prev, "a").is_none());
    }

    /// Whole-node press on a `Multi(ids)` containing the dragged
    /// node is a no-op — the multi-set is preserved so the
    /// shift+drag harvest below sees every selected node.
    #[test]
    fn node_drag_press_on_multi_containing_node_returns_none() {
        let prev = SelectionState::Multi(vec!["a".into(), "b".into()]);
        assert!(selection_after_node_drag_press(&prev, "a").is_none());
    }

    /// Whole-node press on a `Section(node)` for the same node
    /// demotes to `Single(node)` — the gesture is to move the
    /// parent node, not operate on the section.
    #[test]
    fn node_drag_press_demotes_same_node_section() {
        let prev = SelectionState::Section(SectionSel::new("a", 1));
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// Whole-node press on a `MultiSection` containing the dragged
    /// node demotes to `Single(node)`. Pins the parallel of the
    /// section-drag arm's demote.
    #[test]
    fn node_drag_press_demotes_multisection_to_single() {
        let prev = SelectionState::MultiSection(vec![
            SectionSel::new("a", 0),
            SectionSel::new("b", 0),
        ]);
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// Whole-node press on a selection that doesn't include the
    /// dragged node rewrites to `Single(node)` — the user clicked
    /// to grab a fresh node, the prior selection is reset.
    #[test]
    fn node_drag_press_replaces_when_node_not_selected() {
        let prev = SelectionState::Single("b".into());
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// **Pre-demote snapshot for shift+drag harvest.** A
    /// `MultiSection` set's `dedup_owning_node_ids()` is what the
    /// shift+drag harvest reads — pin that this snapshot
    /// preserves every owning node when a single section's drag
    /// would otherwise demote the set down to one. Without this
    /// pre-demote capture the demote runs *before* the harvest
    /// and the drag scope shrinks to one node.
    #[test]
    fn multisection_pre_demote_snapshot_preserves_all_nodes() {
        let prev = SelectionState::MultiSection(vec![
            SectionSel::new("a", 0),
            SectionSel::new("b", 1),
            SectionSel::new("c", 0),
        ]);
        // What the call site captures BEFORE the demote runs.
        let captured = prev.dedup_owning_node_ids();
        // Now simulate the demote (whole-node press path).
        let after_demote = selection_after_node_drag_press(&prev, "a")
            .map(|s| s.dedup_owning_node_ids())
            .unwrap_or_else(|| prev.dedup_owning_node_ids());
        // The captured snapshot has every node; the post-demote
        // selection has only the dragged node. The shift+drag
        // arm reads the captured snapshot, not the post-demote.
        assert_eq!(captured, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(after_demote, vec!["a".to_string()]);
    }
}
