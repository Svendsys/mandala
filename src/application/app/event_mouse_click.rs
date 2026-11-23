// SPDX-License-Identifier: MPL-2.0

//! Mouse-input dispatch. Left / middle / right, pressed / released,
//! routed through the click's selection, the double-click, the start and
//! end of a drag, and the console's steal.
//!
//! **What a click hits is the projector's answer** (`compute_click_hit`
//! through `MapView::hit`), and what it selects is `click_selection`'s:
//! both targets' click and tap paths run the same two.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::{ElementState, MouseButton};

use super::color_picker_flow::{end_color_picker_gesture, handle_color_picker_click, PickerClick};
use super::console_input::save_console_history;
use super::dispatch::cross_dispatch::click_selection;
use super::dispatch::families::edits::Projected;
use super::dispatch::families::text;
use super::dispatch::families::verbs::Host;
use super::input_context::InputHandlerContext;
use super::{
    compute_click_hit, is_double_click, press_hit, ClickHit, DragState, HeldInput, InteractionMode, LastClick,
};
use crate::application::console::ConsoleState;
use crate::application::keybinds::MouseGesture;

/// What a middle-button event does to [`DragState`], with the
/// bound Action and the renderer factored out.
///
/// Pure so both halves of the middle button are pinnable at
/// `DragState` level — the dispatcher below takes an
/// [`InputHandlerContext`], i.e. a live wgpu device, which only the
/// suite's GPU-backed tier stands up (TEST_CONVENTIONS §T8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MiddleButtonRoute {
    /// The drag state is idle — run whatever the user bound to
    /// `MouseGesture::MiddleClick`.
    Dispatch,
    /// End the pan the middle press armed: the state goes to `None`.
    Clear,
    /// Another gesture owns the drag state. Leave it exactly as it
    /// is, and do nothing else.
    Keep,
}

/// Route one middle-button event against the drag state it finds.
///
/// A press dispatches only from an idle state: a press that clobbers a
/// gesture already in flight destroys it. A release ends exactly the
/// pan the middle button holds (`DragState::is_pan_held_by`, #153): a
/// pan held by the left button, by a key, or toggled on from a macro is
/// another gesture's, and `Keep` leaves it. `None` clears trivially — a
/// middle press bound to a non-drag Action leaves the state alone and
/// its release has nothing to end.
fn route_middle_button(state: ElementState, drag_state: &DragState) -> MiddleButtonRoute {
    match state {
        ElementState::Pressed => match drag_state {
            DragState::None => MiddleButtonRoute::Dispatch,
            _ => MiddleButtonRoute::Keep,
        },
        ElementState::Released => match drag_state {
            DragState::None => MiddleButtonRoute::Clear,
            pan if pan.is_pan_held_by(&HeldInput::Button(MouseButton::Middle)) => MiddleButtonRoute::Clear,
            _ => MiddleButtonRoute::Keep,
        },
    }
}

/// `WindowEvent::MouseInput`, after the cursor has been placed.
pub(super) fn handle_mouse_input(
    state: ElementState,
    button: MouseButton,
    ctx: &mut InputHandlerContext<'_>,
) {
    if ctx.console_state.is_open() && state == ElementState::Pressed {
        save_console_history(ctx.console_history);
        *ctx.console_state = ConsoleState::Closed;
        ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
        return;
    }
    // The color picker takes the left and right buttons first: a press
    // on the wheel picks a cell, starts the wheel's move (left) or resize
    // (right), or names commit or cancel, which the funnel runs; a
    // release ends the wheel's gesture. The standalone palette lets a
    // press outside it reach the canvas, so the map can be selected and
    // edited under it; the contextual picker takes every press, and one
    // outside it cancels. As on `main`.
    if ctx.color_picker_state.is_open() && matches!(button, MouseButton::Left | MouseButton::Right) {
        let consumed = if state == ElementState::Pressed {
            match handle_color_picker_click(*ctx.cursor_pos, button, ctx.color_picker_state) {
                PickerClick::Consumed => true,
                PickerClick::FallThrough => false,
                PickerClick::Dispatch(action) => {
                    let _ = super::dispatch::dispatch_action(action, ctx, None);
                    true
                }
            }
        } else {
            end_color_picker_gesture(ctx.color_picker_state)
        };
        // The wheel the press changed is drawn by the next frame
        // (`color_picker_flow::session::drain`).
        if consumed {
            return;
        }
    }
    match button {
        MouseButton::Middle => {
            match route_middle_button(state, ctx.drag_state) {
                MiddleButtonRoute::Dispatch => {
                    if let Some(a) = gesture_action(MouseGesture::MiddleClick, ctx) {
                        let _ = super::dispatch::dispatch_held_action(
                            a,
                            ctx,
                            None,
                            HeldInput::Button(MouseButton::Middle),
                        );
                    }
                }
                MiddleButtonRoute::Clear => *ctx.drag_state = DragState::None,
                MiddleButtonRoute::Keep => {
                    log::debug!("middle-button {state:?} ignored (another gesture owns the drag state); state stays put");
                }
            }
        }
        // In reparent or connect mode the left click picks the target: the
        // press is swallowed, and the release dispatches the confirmation
        // with the node under the cursor.
        MouseButton::Left if !ctx.interaction_mode.sources().is_empty() => {
            if state == ElementState::Released {
                pick_target(ctx);
            }
        }
        MouseButton::Left => match state {
            ElementState::Pressed => handle_left_press(ctx),
            ElementState::Released => handle_left_release(ctx),
        },
        MouseButton::Right => handle_right_button(state, ctx),
        _ => {}
    }
}

/// The Action `gesture` is bound to under the held modifiers, through the
/// modifier-falling-back gesture lookup.
fn gesture_action(
    gesture: MouseGesture,
    ctx: &InputHandlerContext<'_>,
) -> Option<crate::application::keybinds::Action> {
    ctx.keybinds.action_for_gesture(
        gesture.key_name(),
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    )
}

/// A click in reparent or connect mode: `ReparentToTarget` or
/// `ConnectToTarget` with the integer id of the node under the cursor,
/// `None` over empty canvas. The click pairs with no later one.
fn pick_target(ctx: &mut InputHandlerContext<'_>) {
    let canvas_pos = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    let target = match (ctx.document.as_ref(), ctx.view.as_ref()) {
        (Some(doc), Some(view)) => view
            .node_at(canvas_pos)
            .and_then(|node| crate::application::view::inspect::ident_of(doc.doc(), node))
            .map(|ident| ident.0),
        _ => None,
    };
    let action = match ctx.interaction_mode {
        InteractionMode::Reparent { .. } => crate::application::keybinds::Action::ReparentToTarget(target),
        _ => crate::application::keybinds::Action::ConnectToTarget(target),
    };
    let _ = super::dispatch::dispatch_action(action, ctx, None);
    *ctx.last_click = None;
}

/// What the cursor is over, in canvas space, and what a click there hits.
fn hit_under_cursor(ctx: &mut InputHandlerContext<'_>) -> (glam::Vec2, ClickHit) {
    let canvas_pos = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    let hit = compute_click_hit(ctx.document.as_ref(), ctx.view.as_mut(), canvas_pos);
    (canvas_pos, hit)
}

/// A left press: either the second press of a double-click, which
/// dispatches the `DoubleClick` binding with what both presses hit, or
/// the start of a click or a drag (`DragState::Pending`).
fn handle_left_press(ctx: &mut InputHandlerContext<'_>) {
    if ctx.drag_state.would_abandon_gesture() {
        log::debug!("left-button press ignored (a pending gesture owns the pointer); state stays put");
        return;
    }
    let screen_pos = *ctx.cursor_pos;
    let canvas_pos = ctx
        .renderer
        .screen_to_canvas(screen_pos.0 as f32, screen_pos.1 as f32);
    let (landed, hit) = press_hit(ctx.document.as_ref(), ctx.view.as_mut(), canvas_pos);
    let grab = match (ctx.document.as_ref(), ctx.view.as_ref()) {
        (Some(doc), Some(view)) => super::live::grab::press_grab(
            doc.doc(),
            view,
            landed.as_ref(),
            canvas_pos,
            super::handle_tolerance(ctx.renderer),
            ctx.interaction_mode.node_edit(),
        ),
        _ => None,
    };
    // A press on a handle takes no part in a double-click (#218): it
    // neither completes one nor starts one, so a second press on a
    // handle grabs it again rather than open an editor under it.
    if grab.is_some_and(super::live::grab::Grab::is_handle) {
        *ctx.last_click = None;
        *ctx.drag_state = DragState::Pending {
            start_pos: screen_pos,
            hit,
            landed,
            grab,
        };
        return;
    }
    let now = ctx.clock.now_ms();
    // A double-click on what an open editor edits does not open it again:
    // the editor is already there, and its transaction would refuse a
    // second (as `main` guarded the re-open).
    let editing_here = text::editing_under(ctx, landed.as_ref());
    let doubled = !editing_here
        && ctx
            .last_click
            .as_ref()
            .is_some_and(|prev| is_double_click(prev, now, screen_pos, &hit));
    if doubled {
        *ctx.last_click = None;
        *ctx.drag_state = DragState::None;
        if let Some(a) = gesture_action(MouseGesture::DoubleClick, ctx) {
            let dispatch_hit = super::dispatch::DispatchHit {
                click_hit: hit,
                element: landed.as_ref().map(|landed| landed.element),
                canvas_pos,
                held_by: Some(MouseButton::Left),
            };
            let _ = super::dispatch::dispatch_action(a, ctx, Some(&dispatch_hit));
        }
        return;
    }
    *ctx.last_click = Some(LastClick {
        time: now,
        screen_pos,
        hit,
    });
    *ctx.drag_state = DragState::Pending {
        start_pos: screen_pos,
        hit,
        landed,
        grab,
    };
}

/// A left release: a press that never became a drag is a click — one on
/// a handle clicks nothing, and in the resize mode any other leaves it;
/// a rubber band ends by selecting what it covers; a drag the left button
/// holds commits (`live::Live::release`); a pan the left button holds
/// ends.
fn handle_left_release(ctx: &mut InputHandlerContext<'_>) {
    match std::mem::replace(ctx.drag_state, DragState::None) {
        DragState::Pending { grab: Some(grab), .. } if grab.is_handle() => {}
        DragState::Pending { hit, landed, .. } => {
            // An open editor keeps a release on what it edits, and commits
            // on one anywhere else, before the click goes on, as on `main`.
            if text::release_keeps_editing(ctx, landed.as_ref()) {
                return;
            }
            if matches!(*ctx.interaction_mode, InteractionMode::Resize { .. }) {
                *ctx.interaction_mode = InteractionMode::Default;
            }
            // In NodeEdit a click inside the node picks there; one outside
            // leaves the mode and goes on to select what it landed on.
            if let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) {
                if text::node_edit_click(doc, ctx.interaction_mode, landed.as_ref()) {
                    view.sync(doc, ctx.renderer);
                    return;
                }
            }
            handle_click(hit, landed, ctx.modifiers.shift_key(), ctx)
        }
        DragState::SelectingRect {
            start_canvas,
            current_canvas,
        } => super::drain_frame::end_rect_select_gesture(ctx, start_canvas, current_canvas),
        DragState::Dragging(live) if live.held_by == MouseButton::Left => release_drag(*live, ctx),
        other => {
            *ctx.drag_state = other;
            ctx.drag_state
                .end_pan_held_by(&HeldInput::Button(MouseButton::Left));
        }
    }
}

/// The release of the button that holds `live`: it commits, as one
/// history entry, and the view draws the committed map.
pub(super) fn release_drag(live: super::live::Live, ctx: &mut InputHandlerContext<'_>) {
    let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) else {
        return;
    };
    live.release(doc, view.projector());
    view.sync(doc, ctx.renderer);
}

/// A click on what its press `landed` on — `hit`, what the selection
/// takes of it, both from the press's one hit test (`press_hit`) —
/// Shift held or not, first runs what it triggers: a linked span's verb
/// or address, or the clicked node's `on.click` verb (`families::verbs`).
/// Then it updates the selection (`click_selection`), as a click always
/// does, and syncs the view when either changed anything.
pub(super) fn handle_click(
    hit: ClickHit,
    landed: Option<baumhard::projection::Hit>,
    shift: bool,
    ctx: &mut InputHandlerContext<'_>,
) {
    let now_ms = ctx.clock.now_ms();
    let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) else {
        return;
    };
    let triggered = Host {
        doc: &mut *doc,
        drawn: &mut Projected {
            view: &mut *view,
            renderer: &mut *ctx.renderer,
        },
        edits: ctx.edits,
    }
    .click(landed.as_ref(), now_ms);
    if click_selection(&mut doc.selection, hit, shift) || triggered {
        view.sync(doc, ctx.renderer);
    }
}

/// The right button: a press arms `PendingRight` with what it hit; a
/// release with no drag in between fires the `RightClick` binding, and a
/// release ends a pan the right button holds.
fn handle_right_button(state: ElementState, ctx: &mut InputHandlerContext<'_>) {
    if state == ElementState::Pressed {
        if !matches!(*ctx.drag_state, DragState::None) {
            log::debug!("right-button press ignored (drag already in flight); state stays put");
            return;
        }
        // The resize mode's handles are its resize; a fast resize from
        // the body would compete with them, as on `main`.
        if matches!(*ctx.interaction_mode, InteractionMode::Resize { .. }) {
            log::debug!("right-button press ignored (Resize mode active; use the visible handles)");
            return;
        }
        let start_pos = *ctx.cursor_pos;
        let (start_canvas, hit) = hit_under_cursor(ctx);
        *ctx.drag_state = DragState::PendingRight {
            start_pos,
            start_canvas,
            hit,
        };
        return;
    }
    match std::mem::replace(ctx.drag_state, DragState::None) {
        DragState::PendingRight { .. } => {
            if let Some(a) = gesture_action(MouseGesture::RightClick, ctx) {
                let _ = super::dispatch::dispatch_action(a, ctx, None);
            }
        }
        // A fast resize the right button holds (`FastResizeStart`).
        DragState::Dragging(live) if live.held_by == MouseButton::Right => release_drag(*live, ctx),
        other => {
            *ctx.drag_state = other;
            ctx.drag_state
                .end_pan_held_by(&HeldInput::Button(MouseButton::Right));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::PanOwner;
    use super::*;

    /// A pending right press — the one state a press must not overwrite —
    /// is kept by both halves of the middle button.
    fn pending_right() -> DragState {
        DragState::PendingRight {
            start_pos: (0.0, 0.0),
            start_canvas: glam::Vec2::ZERO,
            hit: ClickHit::Empty,
        }
    }

    /// A middle press with a gesture in flight must not reach the
    /// `MiddleClick` binding, and its release must not end that gesture.
    #[test]
    fn test_middle_click_mid_gesture_leaves_the_gesture_intact() {
        let drag = pending_right();
        assert_eq!(
            route_middle_button(ElementState::Pressed, &drag),
            MiddleButtonRoute::Keep
        );
        assert_eq!(
            route_middle_button(ElementState::Released, &drag),
            MiddleButtonRoute::Keep
        );
    }

    /// The other half of the same guard — without these rows the fix
    /// could be "answer `Keep` always", which would leave the middle
    /// button unable to pan at all.
    #[test]
    fn test_middle_click_from_idle_still_arms_and_ends_a_pan() {
        assert_eq!(
            route_middle_button(ElementState::Pressed, &DragState::None),
            MiddleButtonRoute::Dispatch,
            "an idle drag state is what a middle press is for"
        );
        assert_eq!(
            route_middle_button(
                ElementState::Released,
                &held_pan(HeldInput::Button(MouseButton::Middle))
            ),
            MiddleButtonRoute::Clear,
            "the release must end the pan its own press armed"
        );
        assert_eq!(
            route_middle_button(ElementState::Released, &DragState::None),
            MiddleButtonRoute::Clear,
            "a middle press bound to a non-drag Action leaves None behind; its release has nothing to end"
        );
    }

    /// **A middle release ends only the pan the middle button holds**
    /// (#153). The control is the `Clear` row above: the same variant,
    /// held by the middle button, *is* cleared.
    #[test]
    fn test_middle_release_leaves_a_pan_another_input_holds() {
        for owner in [
            PanOwner::Held(HeldInput::Button(MouseButton::Left)),
            PanOwner::Held(HeldInput::Key("space".into())),
            PanOwner::Toggled,
        ] {
            let pan = DragState::Panning { owner: owner.clone() };
            assert_eq!(
                route_middle_button(ElementState::Released, &pan),
                MiddleButtonRoute::Keep,
                "a middle release must not end a pan owned by {owner:?}"
            );
        }
    }

    /// A pan held by `input`.
    fn held_pan(input: HeldInput) -> DragState {
        DragState::Panning {
            owner: PanOwner::Held(input),
        }
    }

    /// A middle press must not cancel the two unthrottled gestures
    /// either: both are live gestures the user is performing.
    #[test]
    fn test_middle_click_does_not_end_a_rect_select_or_a_pending_press() {
        for drag in [
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending {
                start_pos: (0.0, 0.0),
                hit: ClickHit::Empty,
                landed: None,
                grab: None,
            },
        ] {
            assert_eq!(
                route_middle_button(ElementState::Pressed, &drag),
                MiddleButtonRoute::Keep
            );
            assert_eq!(
                route_middle_button(ElementState::Released, &drag),
                MiddleButtonRoute::Keep
            );
        }
    }
}
