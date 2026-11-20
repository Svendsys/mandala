// SPDX-License-Identifier: MPL-2.0

//! Cursor-move dispatch. Owns the drag-state transitions (a pending
//! press crossing the threshold becomes a pan, a rubber band, or
//! whatever the `LeftDrag` / `RightDrag` gesture is bound to), the pan's
//! camera steps, the rubber band's corner, the hand cursor over what a
//! click triggers something on, and the hover a node's `on.hover` verb
//! is held for (`families::verbs`).
//!
//! The drags that edit the map — moving and resizing nodes and
//! sections, the connector handles, the labels — return as transactions
//! with V1-C7 (#248).

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::window::{CursorIcon, PhysicalPosition};

use super::dispatch::families::edits::Projected;
use super::dispatch::families::structure::valid_reparent_target;
use super::dispatch::families::verbs;
use super::input_context::InputHandlerContext;
use super::shell::Shell;
use super::{ClickHit, DragState, HeldInput, InteractionMode};
use crate::application::common::RenderDecree;
use crate::application::keybinds::MouseGesture;
use crate::application::platform::input::MouseButton;

/// `WindowEvent::CursorMoved`: place the cursor, update its icon and the
/// hover, and advance the drag state. Returns whether the hover changed:
/// a verb it invoked or released may have started or stopped what is
/// drawn.
pub(super) fn handle_cursor_moved(
    position: PhysicalPosition<f64>,
    shell: &Shell,
    ctx: &mut InputHandlerContext<'_>,
) -> bool {
    let prev_pos = *ctx.cursor_pos;
    *ctx.cursor_pos = (position.x, position.y);
    *ctx.pointer_inside = true;
    let cursor = *ctx.cursor_pos;

    // Idle: a hand over what a click triggers something on, the default
    // elsewhere, and the hover follows the pointer. During a gesture the
    // cursor stays as the gesture began, and so does the hover.
    // `set_cursor` goes through `cursor_icon_last` because winit dedupes
    // it on macOS / X11 but not on Windows or Wayland.
    let mut hover_changed = false;
    if matches!(*ctx.drag_state, DragState::None) {
        let (points, changed) = follow_pointer(ctx, cursor);
        hover_changed = changed;
        *ctx.cursor_is_hand = points;
        let icon = if points {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        };
        if icon != *ctx.cursor_icon_last {
            shell.set_cursor(icon);
            *ctx.cursor_icon_last = icon;
        }
        follow_target(ctx, cursor);
    }

    advance_drag_state(ctx, prev_pos, cursor);
    hover_changed
}

/// `WindowEvent::CursorLeft`: the pointer left the window, and with it
/// the node it hovered, so the hover is released (#252 §3, ruling D3.2
/// item 4) — a node at the canvas's edge is left this way as often as by
/// a move. During a gesture the hover stays, as a move during one leaves
/// it, and is released when the gesture ends with the pointer still out
/// ([`release_hover_if_outside`]; ruling D3.3 item 4). Returns whether
/// the hover changed.
pub(super) fn handle_cursor_left(ctx: &mut InputHandlerContext<'_>) -> bool {
    *ctx.pointer_inside = false;
    release_hover_if_outside(ctx)
}

/// Release the hover when the pointer is outside the window and no
/// gesture runs: what a leave does at once, and what the end of a
/// gesture the pointer left during does — a release, a key's release, a
/// lost focus — since no move and maybe no second leave comes after
/// (ruling D3.3 item 4 on #252). Returns whether the hover changed.
pub(super) fn release_hover_if_outside(ctx: &mut InputHandlerContext<'_>) -> bool {
    if *ctx.pointer_inside || !matches!(*ctx.drag_state, DragState::None) {
        return false;
    }
    let now_ms = ctx.clock.now_ms();
    let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) else {
        return false;
    };
    verbs::Host {
        doc,
        drawn: &mut Projected {
            view,
            renderer: &mut *ctx.renderer,
        },
        edits: ctx.edits,
    }
    .hover(None, now_ms)
}

/// In reparent or connect mode, tint the node under the cursor when the
/// mode could land on it — for a reparent, not a source nor one below a
/// source. The next frame's sync stamps it (`MapView::set_hover_target`).
fn follow_target(ctx: &mut InputHandlerContext<'_>, cursor: (f64, f64)) {
    let sources = ctx.interaction_mode.sources();
    let (Some(doc), Some(view)) = (ctx.document.as_ref(), ctx.view.as_mut()) else {
        return;
    };
    let target = if sources.is_empty() {
        None
    } else {
        let canvas_pos = ctx.renderer.screen_to_canvas(cursor.0 as f32, cursor.1 as f32);
        let reparenting = matches!(ctx.interaction_mode, InteractionMode::Reparent { .. });
        view.node_at(canvas_pos)
            .filter(|&node| !reparenting || valid_reparent_target(doc.doc(), sources, node))
    };
    view.set_hover_target(target);
}

/// What the pointer at `cursor` (surface pixels) is over, by one hit
/// test (the projector's, `Projector::hit`; no glyph layer is read for
/// the answer): whether a click there triggers something — a node's
/// `on.click`, a linked span — which the hand cursor shows, and the hover,
/// moved onto the node there whose `on.hover` names a verb, or off any
/// (`families::verbs`). Returns the first, and whether the hover changed.
fn follow_pointer(ctx: &mut InputHandlerContext<'_>, cursor: (f64, f64)) -> (bool, bool) {
    let now_ms = ctx.clock.now_ms();
    let canvas_pos = ctx.renderer.screen_to_canvas(cursor.0 as f32, cursor.1 as f32);
    let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) else {
        return (false, false);
    };
    let hit = view.hit(canvas_pos);
    let points = verbs::points_at(doc.doc(), hit.as_ref());
    let hovered = verbs::hovered(doc.doc(), hit.as_ref());
    let changed = verbs::Host {
        doc,
        drawn: &mut Projected {
            view,
            renderer: &mut *ctx.renderer,
        },
        edits: ctx.edits,
    }
    .hover(hovered, now_ms);
    (points, changed)
}

/// Whether the cursor has moved past the drag threshold from `start`.
fn past_threshold(start: (f64, f64), cursor: (f64, f64)) -> bool {
    let (dx, dy) = (cursor.0 - start.0, cursor.1 - start.1);
    dx * dx + dy * dy > super::POINTER_DRAG_THRESHOLD_SQ_PX
}

/// Advance the drag state by one cursor move from `prev` to `cursor`.
fn advance_drag_state(ctx: &mut InputHandlerContext<'_>, prev: (f64, f64), cursor: (f64, f64)) {
    match ctx.drag_state {
        DragState::Panning { .. } => {
            ctx.renderer.process_decree(RenderDecree::CameraPan(
                (cursor.0 - prev.0) as f32,
                (cursor.1 - prev.1) as f32,
            ));
        }
        DragState::SelectingRect { current_canvas, .. } => {
            *current_canvas = ctx.renderer.screen_to_canvas(cursor.0 as f32, cursor.1 as f32);
        }
        DragState::Pending { start_pos, hit, .. } => {
            let (start_pos, hit) = (*start_pos, *hit);
            if past_threshold(start_pos, cursor) {
                promote_left_drag(ctx, start_pos, hit, cursor);
            }
        }
        DragState::PendingRight { start_pos, .. } => {
            if !past_threshold(*start_pos, cursor) {
                return;
            }
            let Some(dispatch_hit) = ctx.drag_state.right_drag_hit() else {
                return;
            };
            *ctx.drag_state = DragState::None;
            let action = ctx.keybinds.action_for_gesture(
                MouseGesture::RightDrag.key_name(),
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            );
            if let Some(a) = action {
                super::dispatch::dispatch_held_action(
                    a,
                    ctx,
                    Some(&dispatch_hit),
                    HeldInput::Button(MouseButton::Right),
                );
            }
        }
        DragState::None => {}
    }
}

/// A left press crossed the threshold: Shift on empty canvas starts a
/// rubber band; anything else dispatches the `LeftDrag` binding (by
/// default `Action::PanCanvas`) with the press's hit, and a pan it arms
/// is held by the left button and moves the camera from the press
/// already, so the first frame of the drag is not lost.
fn promote_left_drag(
    ctx: &mut InputHandlerContext<'_>,
    start_pos: (f64, f64),
    hit: ClickHit,
    cursor: (f64, f64),
) {
    let start_canvas = ctx
        .renderer
        .screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
    if ctx.modifiers.shift_key() && hit == ClickHit::Empty {
        *ctx.drag_state = DragState::SelectingRect {
            start_canvas,
            current_canvas: ctx.renderer.screen_to_canvas(cursor.0 as f32, cursor.1 as f32),
        };
        return;
    }
    // The press is a drag now, not a click: nothing pairs it with the
    // next press as a double-click, and its release clicks nothing.
    *ctx.last_click = None;
    *ctx.drag_state = DragState::None;
    let action = ctx.keybinds.action_for_gesture(
        MouseGesture::LeftDrag.key_name(),
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );
    let Some(a) = action else {
        return;
    };
    let dispatch_hit = super::dispatch::DispatchHit {
        click_hit: hit,
        canvas_pos: start_canvas,
        held_by: Some(MouseButton::Left),
    };
    super::dispatch::dispatch_held_action(a, ctx, Some(&dispatch_hit), HeldInput::Button(MouseButton::Left));
    if ctx.drag_state.is_panning() {
        ctx.renderer.process_decree(RenderDecree::CameraPan(
            (cursor.0 - start_pos.0) as f32,
            (cursor.1 - start_pos.1) as f32,
        ));
    }
}
