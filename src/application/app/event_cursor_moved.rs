// SPDX-License-Identifier: MPL-2.0

//! Cursor-move dispatch. Owns drag-state transitions (pending →
//! Panning / MovingNode / SelectingRect / DraggingEdgeHandle /
//! DraggingPortalLabel), Reparent/Connect hover highlights, and
//! the button-cursor swap for trigger-bearing nodes.

#![cfg(not(target_arch = "wasm32"))]

use super::input_context::InputHandlerContext;
use super::throttled_interaction::{
    EdgeHandleInteraction, EdgeLabelInteraction, MovingNodeInteraction,
    PortalLabelInteraction, ThrottledDrag,
};
use super::*;
use winit::dpi::PhysicalPosition;
use winit::window::Window;

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
            let canvas_pos =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
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
                let canvas_pos =
                    ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
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
            ctx.renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
        }
        DragState::Throttled(ThrottledDrag::MovingNode(i)) => {
            // Convert screen delta to canvas delta and accumulate.
            // Actual mutation + rebuild happens in AboutToWait
            // behind the shared `ThrottledInteraction::drive` gate.
            let old_canvas = ctx.renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
            let new_canvas =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let delta = new_canvas - old_canvas;

            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::EdgeHandle(i)) => {
            // Same accumulation pattern as `MovingNode` — actual
            // edge mutation + buffer rebuild happens in
            // `AboutToWait` behind the adaptive throttle.
            let old_canvas = ctx.renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
            let new_canvas =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let delta = new_canvas - old_canvas;

            i.total_delta += delta;
            i.pending_delta += delta;
        }
        DragState::Throttled(ThrottledDrag::EdgeLabel(i)) => {
            // Overwrite discipline: store the latest cursor —
            // `EdgeLabelInteraction::drain` projects it onto the
            // edge path at consume time, so intermediate cursors
            // carry no information the projection needs.
            let cursor_canvas =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            i.pending_cursor = Some(cursor_canvas);
        }
        DragState::Throttled(ThrottledDrag::PortalLabel(i)) => {
            // Overwrite discipline, same as `EdgeLabel` —
            // `PortalLabelInteraction::drain` snaps to the node
            // border at consume time.
            let cursor_canvas =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            i.pending_cursor = Some(cursor_canvas);
        }
        DragState::Pending {
            start_pos,
            hit_node,
            hit_edge_handle,
            hit_portal_label,
            hit_edge_label,
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
                if let Some(edge_key) = hit_edge_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        if let Some(original) = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned()
                        {
                            doc.selection = SelectionState::EdgeLabel(
                                crate::application::document::EdgeLabelSel::new(
                                    edge_ref.clone(),
                                ),
                            );
                            let prev = doc.selection.clone();
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
                }
                if let Some((edge_key, endpoint)) = hit_portal_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        let original = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned();
                        if let Some(original) = original {
                            doc.selection = SelectionState::PortalLabel(
                                crate::application::document::PortalLabelSel {
                                    edge_key,
                                    endpoint_node_id: endpoint.clone(),
                                },
                            );
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::Throttled(ThrottledDrag::PortalLabel(
                                PortalLabelInteraction::new(edge_ref, endpoint, original),
                            ));
                            rebuild_all(doc, ctx.mindmap_tree, ctx.app_scene, ctx.renderer, ctx.scene_cache);
                            return;
                        }
                    }
                }
                if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                    // Grab the pre-edit snapshot + start
                    // position so the drain loop can do
                    // absolute-positioning math.
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(original) = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned()
                        {
                            let canvas_pos =
                                ctx.renderer.screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                            let start_handle_pos = doc
                                .hit_test_edge_handle(canvas_pos, &edge_ref, f32::INFINITY)
                                .map(|(_, p)| p)
                                .unwrap_or(canvas_pos);
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::Throttled(ThrottledDrag::EdgeHandle(
                                EdgeHandleInteraction::new(
                                    edge_ref,
                                    handle_kind,
                                    original,
                                    start_handle_pos,
                                ),
                            ));
                            return;
                        }
                    }
                }
                if let Some(node_id) = hit_node.take() {
                    // Ensure the dragged node is selected
                    if let Some(doc) = ctx.document.as_mut() {
                        if !doc.selection.is_selected(&node_id) {
                            doc.selection = SelectionState::Single(node_id.clone());
                            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                                let mut new_tree = doc.build_tree();
                                apply_tree_highlights(
                                    &mut new_tree,
                                    doc.selection
                                        .selected_ids()
                                        .into_iter()
                                        .map(|id| (id, HIGHLIGHT_COLOR)),
                                );
                                ctx.renderer.rebuild_buffers_from_tree(&new_tree.tree);
                                *tree = new_tree;
                            }
                        }
                    }
                    // Shift+drag: move all selected nodes together
                    let node_ids = if ctx.modifiers.shift_key() {
                        if let Some(doc) = ctx.document.as_ref() {
                            let mut ids: Vec<String> = doc
                                .selection
                                .selected_ids()
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                            if !ids.contains(&node_id) {
                                ids.push(node_id);
                            }
                            ids
                        } else {
                            vec![node_id]
                        }
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
                    let start_canvas =
                        ctx.renderer.screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                    let current_canvas = ctx.renderer
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
                    let leftdrag_pans = ctx.keybinds
                        .action_for_gesture(
                            crate::application::keybinds::gesture_key_name(
                                crate::application::keybinds::MouseGesture::LeftDrag,
                            ),
                            ctx.modifiers.control_key(),
                            ctx.modifiers.shift_key(),
                            ctx.modifiers.alt_key(),
                        )
                        == Some(crate::application::keybinds::Action::PanCanvas);
                    if leftdrag_pans {
                        *ctx.drag_state = DragState::Panning;
                        let dx = cursor_pos_val.0 - prev_pos.0;
                        let dy = cursor_pos_val.1 - prev_pos.1;
                        ctx.renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                    }
                }
            }
        }
        DragState::SelectingRect {
            current_canvas, ..
        } => {
            *current_canvas =
                ctx.renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
        }
        DragState::None => {}
    }
}
