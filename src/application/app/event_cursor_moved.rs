// SPDX-License-Identifier: MPL-2.0

//! Cursor-move dispatch. Owns the drag-state transitions (a pending
//! press crossing the threshold becomes a pan, a rubber band, or
//! whatever the `LeftDrag` / `RightDrag` gesture is bound to), the pan's
//! camera steps, the rubber band's corner, and the hand cursor over
//! nodes whose click runs a verb.
//!
//! The drags that edit the map — moving and resizing nodes and
//! sections, the connector handles, the labels — return as transactions
//! with V1-C7 (#248).

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::window::{CursorIcon, PhysicalPosition};

use super::dispatch::families::structure::valid_reparent_target;
use super::input_context::InputHandlerContext;
use super::shell::Shell;
use super::{ClickHit, DragState, HeldInput, InteractionMode};
use crate::application::common::RenderDecree;
use crate::application::keybinds::MouseGesture;
use crate::application::platform::input::MouseButton;

/// `WindowEvent::CursorMoved`: place the cursor, update its icon, and
/// advance the drag state.
pub(super) fn handle_cursor_moved(
    position: PhysicalPosition<f64>,
    shell: &Shell,
    ctx: &mut InputHandlerContext<'_>,
) {
    let prev_pos = *ctx.cursor_pos;
    *ctx.cursor_pos = (position.x, position.y);
    let cursor = *ctx.cursor_pos;

    // Idle: a hand over a node whose click runs a verb, the default
    // elsewhere. During a gesture the cursor stays as the gesture began.
    // `set_cursor` goes through `cursor_icon_last` because winit dedupes
    // it on macOS / X11 but not on Windows or Wayland.
    if matches!(*ctx.drag_state, DragState::None) {
        let over_verb = node_with_click_verb_under(ctx, cursor);
        *ctx.cursor_is_hand = over_verb;
        let icon = if over_verb {
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

/// Whether the cursor at `cursor` (surface pixels) is over a node whose
/// `on.click` names a verb: the projector's hit test, then the node's
/// triggers. No glyph layer is read for the answer.
fn node_with_click_verb_under(ctx: &mut InputHandlerContext<'_>, cursor: (f64, f64)) -> bool {
    let (Some(doc), Some(view)) = (ctx.document.as_ref(), ctx.view.as_mut()) else {
        return false;
    };
    let canvas_pos = ctx.renderer.screen_to_canvas(cursor.0 as f32, cursor.1 as f32);
    view.hit(canvas_pos)
        .and_then(|hit| doc.doc().get(hit.node))
        .and_then(|element| element.as_node())
        .is_some_and(|node| node.on.click.is_some())
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
        DragState::Pending { start_pos, hit } => {
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
