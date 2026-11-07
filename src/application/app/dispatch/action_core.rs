// SPDX-License-Identifier: MPL-2.0

//! Cross-platform action dispatcher.
//!
//! Handles every Action whose body needs only state both targets have:
//! the history (`Undo`, `Redo`), the camera, the selection, the FPS
//! readout and `ExitMode`. Before any of it, the Action is offered to
//! the families an owner issue rebuilds (`super::families`), which
//! answer theirs `Unhandled`. Returns `Handled` when a body ran;
//! `Unhandled` for variants this dispatcher doesn't own — the caller's
//! fall-through (native only) runs the platform-specific arm.
//!
//! Both targets reach the same body:
//! - **Native**: `dispatch::dispatch_action` splits its
//!   `InputHandlerContext` into `(InputContextCore, NativeContextExt)`
//!   via `split_borrow` and calls into here first; on `Unhandled`,
//!   falls through to the native-only arm match.
//! - **WASM**: `run_wasm`'s handlers and its macro target call this
//!   directly with `&mut InputContextCore` built from
//!   `WasmInputState::input_context_core`.

use crate::application::keybinds::Action;

use super::super::input_context_core::InputContextCore;
use super::super::InteractionMode;
use super::cross_dispatch::{self, view_ctx, DispatchOutcome, PanDir, SelectionMove, ZoomDir};
use super::families;

/// Cross-platform action dispatcher. Returns `Handled` when the arm body
/// ran; `Unhandled` for variants this dispatcher doesn't own (NativeOnly
/// Actions, and the families' Actions).
///
/// `hit` carries the pointer-event-only payload (what the press landed
/// on, where it landed in canvas space); no arm here reads it today, and
/// the pointer-driven arms that will (V1-C4's reparent target, V1-C8's
/// double-click) take it from here.
pub(in crate::application::app) fn dispatch_compatible(
    action: &Action,
    core: &mut InputContextCore<'_>,
    _hit: Option<&cross_dispatch::DispatchHit>,
) -> DispatchOutcome {
    if let Some(outcome) = families::dispatch(action) {
        return outcome;
    }
    match action {
        Action::ExitMode => {
            *core.interaction_mode = InteractionMode::Default;
            // A post-Esc click is not paired with a pre-Esc one.
            *core.last_click = None;
            DispatchOutcome::Handled
        }
        Action::Undo | Action::Redo => {
            let Some(doc) = core.document.as_deref_mut() else {
                return DispatchOutcome::Unhandled;
            };
            let Some(mut vc) = view_ctx!(core, doc) else {
                return DispatchOutcome::Unhandled;
            };
            let changed = if *action == Action::Undo {
                cross_dispatch::apply_undo(&mut vc)
            } else {
                cross_dispatch::apply_redo(&mut vc)
            };
            if changed {
                DispatchOutcome::Handled
            } else {
                DispatchOutcome::Unhandled
            }
        }
        Action::ZoomIn => {
            cross_dispatch::apply_zoom_step(ZoomDir::In, *core.cursor_pos, core.renderer);
            DispatchOutcome::Handled
        }
        Action::ZoomOut => {
            cross_dispatch::apply_zoom_step(ZoomDir::Out, *core.cursor_pos, core.renderer);
            DispatchOutcome::Handled
        }
        Action::ZoomReset => {
            cross_dispatch::apply_zoom_reset(core.renderer);
            DispatchOutcome::Handled
        }
        Action::ZoomFit => {
            cross_dispatch::apply_zoom_fit(core.renderer);
            DispatchOutcome::Handled
        }
        Action::PanCameraNorth => pan(core, PanDir::North),
        Action::PanCameraSouth => pan(core, PanDir::South),
        Action::PanCameraEast => pan(core, PanDir::East),
        Action::PanCameraWest => pan(core, PanDir::West),
        Action::CenterOnSelection => match (core.document.as_deref(), core.view.as_ref()) {
            (Some(doc), Some(view)) => {
                cross_dispatch::apply_center_on_selection(doc, view, core.renderer);
                DispatchOutcome::Handled
            }
            _ => DispatchOutcome::Unhandled,
        },
        Action::JumpToRoot => {
            let Some(doc) = core.document.as_deref_mut() else {
                return DispatchOutcome::Unhandled;
            };
            let Some(mut vc) = view_ctx!(core, doc) else {
                return DispatchOutcome::Unhandled;
            };
            cross_dispatch::apply_jump_to_root(&mut vc);
            DispatchOutcome::Handled
        }
        Action::ToggleFps => {
            cross_dispatch::apply_toggle_fps(core.renderer);
            DispatchOutcome::Handled
        }
        Action::ToggleFpsDebug => {
            cross_dispatch::apply_toggle_fps_debug(core.renderer);
            DispatchOutcome::Handled
        }
        Action::SelectAll => select(core, SelectionMove::All),
        Action::DeselectAll => select(core, SelectionMove::None),
        Action::InvertSelection => select(core, SelectionMove::Invert),
        Action::SelectParent => select(core, SelectionMove::Parent),
        Action::SelectChild => select(core, SelectionMove::Child),
        Action::SelectNextSibling => select(core, SelectionMove::NextSibling),
        Action::SelectPrevSibling => select(core, SelectionMove::PrevSibling),
        _ => DispatchOutcome::Unhandled,
    }
}

/// A pan nudge: always handled.
fn pan(core: &mut InputContextCore<'_>, dir: PanDir) -> DispatchOutcome {
    cross_dispatch::apply_pan_camera(dir, core.renderer);
    DispatchOutcome::Handled
}

/// A selection move: handled when there is a map to select in.
fn select(core: &mut InputContextCore<'_>, step: SelectionMove) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Unhandled;
    };
    let Some(mut vc) = view_ctx!(core, doc) else {
        return DispatchOutcome::Unhandled;
    };
    cross_dispatch::apply_selection_move(step, &mut vc);
    DispatchOutcome::Handled
}
