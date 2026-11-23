// SPDX-License-Identifier: MPL-2.0

//! Cross-platform action dispatcher.
//!
//! Handles every Action whose body needs only state both targets have:
//! the history (`Undo`, `Redo`), the camera, the selection, the FPS
//! readout and `ExitMode`. Before any of it, the Action is offered to
//! the structure family (`super::families::structure`: create, delete,
//! reparent, connect, the clipboard) and the field family
//! (`super::families::field_nodes`: node, section and frame fields, and
//! style across the selection), the path family
//! (`super::families::field_paths`) and the text family
//! (`super::families::text`: the section editor and node editing).
//! Returns `Handled` when a body ran;
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
use super::the_map::OnTheMap;

/// Cross-platform action dispatcher. Returns `Handled` when the arm body
/// ran; `Unhandled` for variants this dispatcher doesn't own (NativeOnly
/// Actions, and the halves of the families that are native's).
///
/// `hit` carries the pointer-event-only payload (what the press landed
/// on, where it landed in canvas space): `CreateOrphanNode` places its
/// node there, and `DoubleClickActivate` edits what it landed on.
pub(in crate::application::app) fn dispatch_compatible(
    action: &Action,
    core: &mut InputContextCore<'_>,
    hit: Option<&cross_dispatch::DispatchHit>,
) -> DispatchOutcome {
    let reads_the_map = !leaves_the_map_unread(action);
    // Settle an animated edit first, so whatever this Action reads to
    // build its change — a copy, a field's value, a structure — reads
    // the map the edit leaves (D28); the writer would fast-forward it
    // only at `apply` or `begin`, after the read.
    if reads_the_map {
        if let Some(doc) = core.document.as_deref_mut() {
            doc.finish_animation();
        }
    }
    // And read the map itself, not what a presentation shows on it
    // (`the_map`): the layer is aside until this returns.
    let mut core = OnTheMap::when(core, reads_the_map);
    dispatch_arms(action, &mut core, hit)
}

/// The arms of [`dispatch_compatible`]: the owner families first, then
/// the cross-platform ones.
fn dispatch_arms(
    action: &Action,
    core: &mut InputContextCore<'_>,
    hit: Option<&cross_dispatch::DispatchHit>,
) -> DispatchOutcome {
    if let Some(outcome) = families::structure::dispatch(action, core, hit) {
        return outcome;
    }
    if let Some(outcome) = families::field_nodes::dispatch(action, core) {
        return outcome;
    }
    if let Some(outcome) = families::field_paths::dispatch(action, core) {
        return outcome;
    }
    if let Some(outcome) = families::text::dispatch(action, core, hit) {
        return outcome;
    }
    match action {
        Action::ExitMode => {
            // An open frame preview is reverted first, and that is the
            // whole of the key's work, as on `main`: the next press
            // leaves the mode.
            if let Some(doc) = core.document.as_deref_mut() {
                if doc.cancel(crate::application::document::Interaction::FramePreview) {
                    if let Some(mut vc) = view_ctx!(core, doc) {
                        vc.sync();
                    }
                    return DispatchOutcome::Handled;
                }
            }
            // Leaving NodeEdit lifts a selection inside the node back to
            // the node, as on `main`.
            if let Some(doc) = core.document.as_deref_mut() {
                families::text::leave_node_edit(doc, core.interaction_mode);
            }
            *core.interaction_mode = InteractionMode::Default;
            // A post-Esc click is not paired with a pre-Esc one.
            *core.last_click = None;
            // A refusal's status line goes at the next frame's sync; a
            // mode's tints and line follow the mode there.
            if let Some(doc) = core.document.as_deref_mut() {
                doc.dismiss_status();
            }
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

/// Whether `action` reads nothing of the map to change it: it moves the
/// camera, changes the selection, or toggles a readout — what it reads
/// is what is drawn. So it leaves an animated edit running, and a
/// presentation applied. Every other Action settles the edit and sets
/// the presentation layer aside first ([`dispatch_compatible`]).
fn leaves_the_map_unread(action: &Action) -> bool {
    matches!(
        action,
        Action::PanCanvas
            | Action::ZoomIn
            | Action::ZoomOut
            | Action::ZoomReset
            | Action::ZoomFit
            | Action::PanCameraNorth
            | Action::PanCameraSouth
            | Action::PanCameraEast
            | Action::PanCameraWest
            | Action::CenterOnSelection
            | Action::JumpToRoot
            | Action::SelectAll
            | Action::DeselectAll
            | Action::InvertSelection
            | Action::SelectParent
            | Action::SelectChild
            | Action::SelectNextSibling
            | Action::SelectPrevSibling
            | Action::ToggleFps
            | Action::ToggleFpsDebug
    )
}
