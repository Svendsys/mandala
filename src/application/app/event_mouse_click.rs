// SPDX-License-Identifier: MPL-2.0

//! Mouse-input dispatch. Left/middle/right + Pressed/Released routed
//! through selection, double-click, drag start/end, and the
//! console / color-picker steals.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;
use winit::event::{ElementState, MouseButton};

use super::click::handle_click;
use super::color_picker_flow::{end_color_picker_gesture, handle_color_picker_click};
use super::console_input::save_console_history;
use super::edge_drag::apply_edge_handle_drag;
use super::input_context::InputHandlerContext;
use super::portal_label_drag::apply_portal_label_drag;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all, rebuild_scene_only};
use super::throttled_interaction::ThrottledDrag;
use super::{is_double_click, now_ms, AppMode, DragState, LastClick, EDGE_HANDLE_HIT_TOLERANCE_PX};
use crate::application::console::ConsoleState;
use crate::application::document::{apply_drag_delta, rect_select, SelectionState, UndoAction};
use crate::application::keybinds::Action;

/// Dispatch a `WindowEvent::MouseInput`. Persistent state arrives
/// via [`InputHandlerContext`].
pub(super) fn handle_mouse_input(
    state: ElementState,
    button: MouseButton,
    ctx: &mut InputHandlerContext<'_>,
) {
    let cursor_pos_val = *ctx.cursor_pos;
    // The console swallows mouse clicks as a close
    // gesture. Clicking anywhere while open dismisses
    // the console without running a command, mirroring
    // Escape.
    if ctx.console_state.is_open() && state == ElementState::Pressed {
        save_console_history(ctx.console_history);
        *ctx.console_state = ConsoleState::Closed;
        ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
        return;
    }

    // Glyph-wheel color picker click handling. The
    // picker captures both left- and right-mouse
    // buttons:
    // - LMB on a `DragAnchor` → wheel-move gesture;
    //   on any other hit → preview / commit / chip
    //   focus.
    // - RMB on a `DragAnchor` → wheel-resize
    //   gesture (drag away to grow, toward to shrink).
    //   RMB elsewhere is currently a no-op — only
    //   the empty backdrop region acts as the resize
    //   handle, mirroring the LMB-move convention.
    // Release of either button ends any active
    // gesture. In **Standalone** (persistent
    // palette) mode, clicks outside the picker
    // backdrop fall through to normal dispatch —
    // otherwise the user couldn't select anything
    // else while the palette was open. In
    // **Contextual** mode the picker captures
    // everything; outside-click cancels.
    if ctx.color_picker_state.is_open() && matches!(button, MouseButton::Left | MouseButton::Right) {
        let consumed = if state == ElementState::Pressed {
            if let Some(doc) = ctx.document.as_mut() {
                handle_color_picker_click(
                    cursor_pos_val,
                    button,
                    ctx.color_picker_state,
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                    ctx.picker_hover,
                )
            } else {
                true
            }
        } else {
            // Release — end any active wheel gesture.
            // If no gesture was active (e.g.
            // Standalone + outside-press fell
            // through), this is a no-op and the
            // release should also fall through.
            end_color_picker_gesture(ctx.color_picker_state)
        };
        if consumed {
            return;
        }
    }
    match button {
        MouseButton::Middle => {
            if state == ElementState::Pressed {
                // Middle-click press: lookup what's bound to MiddleClick
                // (default `PanCanvas`). The dispatch arm sets
                // `DragState::Panning`. Release unconditionally resets
                // drag state below — mirrors today's behaviour where
                // any drag's release goes to None regardless of which
                // gesture started it.
                let name = crate::application::keybinds::MouseGesture::MiddleClick.key_name();
                // Modifier-fallback: Ctrl+MiddleClick matches the bare
                // MiddleClick binding when no exact-modifier match
                // exists. Preserves pre-branch modifier-agnostic
                // behaviour for mouse gestures.
                let action = ctx.keybinds.action_for_gesture(
                    name,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
                if let Some(a) = action {
                    let _ = super::dispatch::dispatch_action(a, ctx, None);
                }
            } else {
                *ctx.drag_state = DragState::None;
            }
        }
        MouseButton::Left => {
            // In reparent or connect mode, left-click (release) is consumed as
            // a "choose target" gesture and never transitions to Pending/drag.
            // Hit-test inline so the dispatch arm receives a resolved target id;
            // the arms read the source(s) from `ctx.app_mode` directly.
            if matches!(ctx.app_mode, AppMode::Reparent { .. }) {
                if state == ElementState::Released {
                    let target: Option<String> = ctx.mindmap_tree.as_mut().and_then(|tree| {
                        let canvas_pos = ctx
                            .renderer
                            .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                        crate::application::document::hit_test(canvas_pos, tree)
                    });
                    let _ = super::dispatch::dispatch_action(Action::ReparentToTarget(target), ctx, None);
                    // Mode-exit via target click — clear any stale
                    // click so the first post-mode click can't be
                    // paired into a double-click. Stays here per the
                    // §3 carve-out: pre-funnel state-machine
                    // bookkeeping, not user-named effect.
                    *ctx.last_click = None;
                }
                // Pressed: swallow — do not transition drag state
            } else if matches!(ctx.app_mode, AppMode::Connect { .. }) {
                if state == ElementState::Released {
                    let target: Option<String> = ctx.mindmap_tree.as_mut().and_then(|tree| {
                        let canvas_pos = ctx
                            .renderer
                            .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                        crate::application::document::hit_test(canvas_pos, tree)
                    });
                    // `target = None` (empty-canvas) and
                    // `target = Some(id)` both flow through the
                    // funnel; the arm body owns the mode-exit
                    // rebuild on either branch. Symmetric with
                    // `Action::ReparentToTarget` (also takes
                    // `Option<String>`).
                    let _ = super::dispatch::dispatch_action(Action::ConnectToTarget(target), ctx, None);
                    // Mode-exit via target click — clear any stale
                    // click so the first post-mode click can't be
                    // paired into a double-click. Stays here per
                    // the §3 carve-out: pre-funnel state-machine
                    // bookkeeping, not user-named effect.
                    *ctx.last_click = None;
                }
                // Pressed: swallow
            } else if state == ElementState::Pressed {
                // Hit test to determine if clicking on a node
                let canvas_pos = ctx
                    .renderer
                    .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);

                // Double-click detection. If this press within the
                // double-click window matches the previous one (same
                // hit target, within time + distance), dispatch:
                //  - Double-click on a node → open the text editor.
                //  - Double-click on a portal marker → pan the camera
                //    to the OTHER endpoint of the portal-mode edge.
                //  - Double-click on empty space (and no edge
                //    selected) → create a new orphan and edit it.
                //
                // Guard: if the editor is already open on the same
                // hit target, DO NOT re-open it — that would
                // silently discard the in-progress buffer. Let the
                // press fall through; the corresponding release
                // will be swallowed as click-inside.
                let now = now_ms();
                let parts = super::compute_click_hit(canvas_pos, ctx.mindmap_tree.as_mut(), ctx.renderer);
                let super::ClickHitParts {
                    click_hit,
                    hit_node,
                    hit_section_idx,
                    portal_text_hit,
                    portal_icon_hit,
                    edge_label_hit,
                } = parts;
                // Suppress the double-click → open-editor gesture when
                // an editor is already open on the click's target. The
                // three editor states are mutually exclusive by
                // construction (the event-keyboard dispatch steals on
                // whichever is open first), so one match suffices.
                // Without this guard for the label / portal-text
                // editors, a double-click while editing would call
                // `open_label_edit` / `open_portal_text_edit` a second
                // time, which re-seeds the buffer from the committed
                // model value and silently destroys the in-progress
                // edit.
                let already_editing_same_target = {
                    let node_match = ctx
                        .text_edit_state
                        .node_id()
                        .map(|id| hit_node.as_deref() == Some(id))
                        .unwrap_or(false);
                    let edge_label_match = ctx
                        .label_edit_state
                        .edited_edge_ref()
                        .zip(edge_label_hit.as_ref())
                        .map(|(er, hit)| {
                            hit.from_id == er.from_id.as_str()
                                && hit.to_id == er.to_id.as_str()
                                && hit.edge_type == er.edge_type.as_str()
                        })
                        .unwrap_or(false);
                    let portal_text_match = ctx
                        .portal_text_edit_state
                        .edited_endpoint()
                        .zip(portal_text_hit.as_ref())
                        .map(|((er, ep), (hit_key, hit_ep))| {
                            hit_key.from_id == er.from_id.as_str()
                                && hit_key.to_id == er.to_id.as_str()
                                && hit_key.edge_type == er.edge_type.as_str()
                                && hit_ep.as_str() == ep
                        })
                        .unwrap_or(false);
                    node_match || edge_label_match || portal_text_match
                };
                let is_dblclick = !already_editing_same_target
                    && ctx
                        .last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, cursor_pos_val, &click_hit))
                        .unwrap_or(false);
                if is_dblclick {
                    *ctx.last_click = None;
                    // Look up which Action (if any) the user has bound
                    // to `DoubleClick`. Default is `DoubleClickActivate`
                    // which routes by `ClickHit`; `Empty` only fires
                    // `CreateOrphanNodeAndEdit` when the user has
                    // explicitly bound that Action somewhere
                    // (off-by-default per user request).
                    let dblclick_name = crate::application::keybinds::MouseGesture::DoubleClick.key_name();
                    // Modifier-fallback so Shift+DoubleClick still
                    // activates the bare DoubleClick binding when no
                    // explicit Shift+DoubleClick binding exists.
                    let action = ctx.keybinds.action_for_gesture(
                        dblclick_name,
                        ctx.modifiers.control_key(),
                        ctx.modifiers.shift_key(),
                        ctx.modifiers.alt_key(),
                    );
                    if let Some(a) = action {
                        let dispatch_hit = super::dispatch::DispatchHit {
                            click_hit: click_hit.clone(),
                            canvas_pos,
                        };
                        let _ = super::dispatch::dispatch_action(a, ctx, Some(&dispatch_hit));
                        return;
                    }
                    // No Action bound to DoubleClick: silently no-op.
                    // (The double-click consumed `ctx.last_click`; we don't
                    // fall through to the single-click selection path.)
                    return;
                }
                *ctx.last_click = Some(LastClick {
                    time: now,
                    screen_pos: cursor_pos_val,
                    hit: click_hit,
                });

                // If an edge is currently selected, check
                // whether the cursor is over one of its
                // grab-handles. This check has precedence
                // over the node hit at threshold-cross
                // time — see the `Pending` → drag
                // transition below. Returns `None` if no
                // edge is selected, nothing is in range,
                // or the hit test infrastructure isn't
                // ready yet.
                let hit_edge_handle = match ctx.document.as_ref() {
                    Some(doc) => match &doc.selection {
                        SelectionState::Edge(er) => {
                            let tol = EDGE_HANDLE_HIT_TOLERANCE_PX * ctx.renderer.canvas_per_pixel();
                            doc.hit_test_edge_handle(canvas_pos, er, tol)
                                .map(|(kind, _pos)| (er.clone(), kind))
                        }
                        _ => None,
                    },
                    None => None,
                };
                // Portal-label drag capture. Takes precedence
                // over `hit_node` at threshold-cross time so
                // pressing a marker and dragging slides the label
                // along its owning node's border rather than
                // moving the node itself. Captured regardless of
                // current selection — grabbing a marker is a
                // valid first action, not just a follow-up to a
                // prior click.
                // Portal **icon** drag captures the `border_t`
                // slide gesture — dragging the text sub-part
                // isn't a supported interaction. Only populate
                // this when the icon-side hit was present.
                let hit_portal_label = match &portal_icon_hit {
                    Some((key, endpoint)) if hit_node.is_none() => Some((key.clone(), endpoint.clone())),
                    _ => None,
                };
                // Reuse the press-time edge-label hit captured
                // earlier so the threshold-cross transition can
                // promote to `DraggingEdgeLabel`. Priority
                // ordering in `event_cursor_moved.rs` still
                // gives portal-label / edge-handle drag higher
                // precedence when multiple hits overlap.
                *ctx.drag_state = DragState::Pending {
                    start_pos: cursor_pos_val,
                    hit_node,
                    hit_section_idx,
                    hit_edge_handle,
                    hit_portal_label,
                    hit_edge_label: edge_label_hit,
                };
            } else {
                // Released
                match std::mem::replace(ctx.drag_state, DragState::None) {
                    DragState::Pending {
                        hit_node,
                        hit_section_idx,
                        hit_edge_label,
                        ..
                    } => {
                        // If the node text editor is open, the
                        // release decides whether to commit or
                        // swallow. If the release lands inside the
                        // edited node's AABB, keep editing (no
                        // commit, no selection change). Otherwise
                        // commit and fall through.
                        if ctx.text_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            // Refresh the subtree-AABB cache before the
                            // overflow-aware containment check —
                            // `point_in_node_aabb` reads
                            // `subtree_aabb()` which returns `None`
                            // when the cache is dirty (post-mutation
                            // / post-tree-rebuild). A `None` falls
                            // back to the container-only path,
                            // regressing the multi-section overflow
                            // gesture this branch was added to fix.
                            // `ensure_subtree_aabbs` is O(1) on a
                            // clean cache and O(arena) on the first
                            // call after a mutation; either way it's
                            // cheap relative to the click handler.
                            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                                tree.tree.ensure_subtree_aabbs();
                            }
                            let inside = ctx
                                .text_edit_state
                                .node_id()
                                .zip(ctx.mindmap_tree.as_ref())
                                .map(|(id, tree)| {
                                    crate::application::document::point_in_node_aabb(release_canvas, id, tree)
                                })
                                .unwrap_or(false);
                            if inside {
                                // Click-inside: keep
                                // editing. Do NOT fall
                                // through to handle_click
                                // (that would change the
                                // selection). Also do
                                // not transition drag
                                // state — the release
                                // is fully consumed.
                                return;
                            }
                            // Click-outside: commit the edit through
                            // the funnel (`Action::TextEditCommit`),
                            // then fall through to the regular click
                            // path so the new selection lands.
                            let _ = super::dispatch::dispatch_action(Action::TextEditCommit, ctx, None);
                        }
                        // Same shape for the inline edge-label
                        // editor: a release that doesn't hit the
                        // edge currently being edited commits the
                        // buffer; a release that lands back on
                        // the same edge label keeps the editor
                        // open. Without this branch, the only way
                        // to close the editor was Esc / Enter,
                        // and clicking elsewhere felt unresponsive.
                        // Mirrors the node text editor's behaviour
                        // so the same muscle memory transfers.
                        if ctx.label_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            let edited = ctx.label_edit_state.edited_edge_ref().cloned();
                            let stays_on_edited_label = edited
                                .as_ref()
                                .and_then(|er| {
                                    ctx.renderer.hit_test_any_edge_label(release_canvas).map(|hit| {
                                        hit.from_id == er.from_id.as_str()
                                            && hit.to_id == er.to_id.as_str()
                                            && hit.edge_type == er.edge_type.as_str()
                                    })
                                })
                                .unwrap_or(false);
                            if stays_on_edited_label {
                                return;
                            }
                            // Click-outside the edited label: commit
                            // through the funnel (`LabelEditCommit`).
                            let _ = super::dispatch::dispatch_action(Action::LabelEditCommit, ctx, None);
                        }
                        // Portal-text editor uses the portal-text
                        // hitbox instead of the edge-label hitbox,
                        // and matches `(edge_key, endpoint)` rather
                        // than just the edge key — clicking the
                        // *other* endpoint of the same portal edge
                        // commits this side and then routes the
                        // click as a fresh selection on the new
                        // endpoint.
                        if ctx.portal_text_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            let edited = ctx
                                .portal_text_edit_state
                                .edited_endpoint()
                                .map(|(er, ep)| (er.clone(), ep.to_string()));
                            let stays_on_edited_text = edited
                                .as_ref()
                                .and_then(|(er, ep)| {
                                    ctx.renderer.hit_test_portal_text(release_canvas).map(
                                        |(hit_key, hit_ep)| {
                                            hit_key.from_id == er.from_id.as_str()
                                                && hit_key.to_id == er.to_id.as_str()
                                                && hit_key.edge_type == er.edge_type.as_str()
                                                && hit_ep == *ep
                                        },
                                    )
                                })
                                .unwrap_or(false);
                            if stays_on_edited_text {
                                return;
                            }
                            // Click-outside the edited portal-text:
                            // commit through the funnel. The dispatch
                            // arm picks `portal_text_edit_state` over
                            // `label_edit_state` since portal is
                            // checked first.
                            let _ = super::dispatch::dispatch_action(Action::LabelEditCommit, ctx, None);
                        }
                        // Edge-label single click: route to the
                        // `EdgeLabel` selection rather than opening
                        // the editor. Matches the "click to select,
                        // dbl-click to edit" idiom the node /
                        // portal-label variants already follow —
                        // the dbl-click branch above handles the
                        // editor-open case.
                        //
                        // Consume the `hit_edge_label` captured at
                        // press time (with its full priority chain:
                        // node > portal_text > portal_icon >
                        // edge_label > edge_body). Re-hit-testing
                        // at release would ignore that chain — a
                        // press that landed on a portal icon but
                        // drifted a few pixels onto an overlapping
                        // edge label before release would mis-
                        // route to `EdgeLabel` instead of the
                        // portal's sub-threshold single-click.
                        let edge_label_target: Option<crate::application::document::EdgeRef> = hit_edge_label
                            .map(|k| {
                                crate::application::document::EdgeRef::new(
                                    k.from_id.as_str(),
                                    k.to_id.as_str(),
                                    k.edge_type.as_str(),
                                )
                            });
                        let entered_label_select = if let Some(er) = edge_label_target {
                            if let Some(doc) = ctx.document.as_mut() {
                                let prev = doc.selection.clone();
                                doc.selection = SelectionState::EdgeLabel(
                                    crate::application::document::EdgeLabelSel::new(er),
                                );
                                rebuild_after_selection_change(
                                    &prev,
                                    doc,
                                    ctx.mindmap_tree,
                                    ctx.app_scene,
                                    ctx.renderer,
                                    ctx.scene_cache,
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !entered_label_select {
                            handle_click(
                                hit_node,
                                hit_section_idx,
                                cursor_pos_val,
                                ctx.modifiers.shift_key(),
                                ctx.document,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::MovingNode(i)) => {
                        // Flush any remaining pending delta to the tree before drop.
                        // This always runs regardless of the throttle — on release
                        // we want the final position committed in full, even if
                        // the throttle was mid-stretch skipping intermediate drains.
                        let had_pending = i.pending_delta != Vec2::ZERO;
                        if had_pending {
                            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                                for nid in &i.node_ids {
                                    apply_drag_delta(
                                        tree,
                                        nid,
                                        i.pending_delta.x,
                                        i.pending_delta.y,
                                        !i.individual,
                                    );
                                }
                            }
                        }
                        // Drop: sync to model, full rebuild, push undo
                        if let Some(doc) = ctx.document.as_mut() {
                            let dx = i.total_delta.x as f64;
                            let dy = i.total_delta.y as f64;
                            let undo_data = doc.apply_move_multiple(&i.node_ids, dx, dy, i.individual);
                            doc.undo_stack.push(UndoAction::MoveNodes {
                                original_positions: undo_data,
                            });
                            doc.dirty = true;

                            // Under rapid drag the throttle can skip the last
                            // drain or two, leaving `pending_delta` stranded
                            // in the accumulator. The flush above syncs the
                            // tree and `apply_move_multiple` above syncs the
                            // model, but the `SceneConnectionCache`'s
                            // `pre_clip_positions` still reflect the
                            // second-to-last drain's `offsets` (short of the
                            // committed position by `pending_delta`). The
                            // subsequent `rebuild_all` → `rebuild_scene_only`
                            // runs with empty offsets, so the cache's fast
                            // path returns those stale samples and the edges
                            // appear glued to the node's pre-flush position
                            // until the next cache-invalidating event
                            // (mutation or zoom). Clearing here forces a
                            // resample from the now-authoritative model.
                            if had_pending {
                                ctx.scene_cache.clear();
                            }

                            // Full rebuild from model
                            rebuild_all(
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::EdgeHandle(i)) => {
                        // The drain loop has been writing
                        // each new edge state directly
                        // into the model. Before release,
                        // flush one last write using the
                        // full `total_delta` (independent
                        // of any throttled pending drain)
                        // so the final committed state
                        // matches the cursor position
                        // exactly. Reaching this branch
                        // means the drag threshold was
                        // crossed, so push an EditEdge
                        // undo with the pre-drag snapshot
                        // unconditionally.
                        let super::throttled_interaction::EdgeHandleInteraction {
                            edge_ref,
                            handle,
                            original,
                            start_handle_pos,
                            total_delta,
                            ..
                        } = i;
                        if let Some(doc) = ctx.document.as_mut() {
                            apply_edge_handle_drag(doc, &edge_ref, handle, start_handle_pos, total_delta);
                            // Crossing the drag threshold guarantees a
                            // state change, so commit unconditionally.
                            doc.commit_throttled_edge_drag(&edge_ref, original, |_, _| true);
                            rebuild_all(
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::PortalLabel(i)) => {
                        // Flush the final cursor if one is buffered.
                        // When `pending_cursor` is `None` the last
                        // drain already consumed it and no flush is
                        // needed; when `Some`, the throttle skipped
                        // that cursor and release must commit the
                        // user's actual drop position rather than
                        // wherever the prior drain happened to land.
                        // Bypasses the throttle — there is no "next
                        // frame" after release.
                        let super::throttled_interaction::PortalLabelInteraction {
                            edge_ref,
                            endpoint_node_id,
                            original,
                            pending_cursor,
                            ..
                        } = i;
                        if let (Some(doc), Some(cursor)) = (ctx.document.as_mut(), pending_cursor) {
                            apply_portal_label_drag(doc, &edge_ref, &endpoint_node_id, cursor);
                        }
                        // Commit with a single EditEdge undo
                        // carrying the pre-drag snapshot, matching
                        // the EdgeHandle release path. The no-op
                        // check compares only the two fields this
                        // drag touches (`portal_from` /
                        // `portal_to`) — whole-edge `PartialEq`
                        // would fold in float-fragile
                        // `control_points`.
                        if let Some(doc) = ctx.document.as_mut() {
                            doc.commit_throttled_edge_drag(&edge_ref, original, |c, o| {
                                c.portal_from != o.portal_from || c.portal_to != o.portal_to
                            });
                            rebuild_all(
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::EdgeLabel(i)) => {
                        // Flush the final cursor if one is buffered.
                        // See the portal release arm above for the
                        // rationale — `None` means the last drain
                        // already caught it, `Some` means the
                        // throttle skipped the final CursorMoved.
                        let super::throttled_interaction::EdgeLabelInteraction {
                            edge_ref,
                            original,
                            pending_cursor,
                            ..
                        } = i;
                        if let (Some(doc), Some(cursor)) = (ctx.document.as_mut(), pending_cursor) {
                            super::edge_label_drag::apply_edge_label_drag(doc, &edge_ref, cursor);
                        }
                        // Commit with a single `EditEdge` carrying
                        // the pre-drag snapshot, skipping the undo
                        // entry if nothing actually moved.
                        if let Some(doc) = ctx.document.as_mut() {
                            doc.commit_throttled_edge_drag(&edge_ref, original, |c, o| {
                                c.label_config != o.label_config
                            });
                            // Scene-only rebuild: every per-frame
                            // drain already used `rebuild_scene_only`
                            // because node trees are untouched by a
                            // label move; the release commit is
                            // the same story.
                            rebuild_scene_only(doc, ctx.app_scene, ctx.renderer, ctx.scene_cache);
                        }
                    }
                    DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    } => {
                        // Finalize: select all nodes in the rectangle
                        ctx.renderer.clear_overlay_buffers();
                        if let (Some(doc), Some(tree)) = (ctx.document.as_mut(), ctx.mindmap_tree.as_ref()) {
                            let hits = rect_select(start_canvas, current_canvas, tree);
                            doc.selection = SelectionState::from_ids(hits);
                            rebuild_all(
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Panning | DragState::None => {}
                }
            }
        }
        _ => {}
    }
}
