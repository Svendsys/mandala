// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::MouseWheel` arm. Resolves the scroll direction to a
//! `MouseGesture`, looks up whatever the user has bound to it, and
//! runs that Action through the funnel — the same three steps native
//! takes. `WheelUp` / `WheelDown` are default-bound to `ZoomIn` /
//! `ZoomOut`; rebinding or unbinding them takes effect here.
//!
//! Pre-unification this arm hardcoded a 1.1× zoom factor, emitted the
//! `CameraZoom` decree itself, and rebuilt every canvas role
//! unconditionally — so a browser user's `keybinds.json` had no say
//! in what the wheel did.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::MouseScrollDelta;

use super::PendingClick;
use crate::application::app::scene_rebuild::rebuild_camera_geometry;
use crate::application::app::{dispatch, wheel_gesture, wheel_lines};

impl super::WasmApp {
    pub(super) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let lines = wheel_lines(delta);
        let gesture_name = wheel_gesture(lines).key_name();
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return;
        };
        // A scroll mid-click invalidates the pending selection: the
        // canvas coord the user pressed over has shifted to a new
        // screen position, so committing the pending click on the
        // eventual mouse-up would select whatever now sits under the
        // release cursor — not what the user pressed on. Clear it so
        // release falls through to empty-click handling. Runs
        // regardless of what (if anything) the wheel is bound to: the
        // press-time coordinate is stale either way.
        input.pending_click = PendingClick::None;

        // `action_for_gesture` falls back to the unmodified binding
        // when no exact-modifier match exists, so Ctrl+Wheel keeps
        // zooming even though only bare `WheelUp` / `WheelDown` ship
        // bound. An explicitly-cleared binding means the wheel is
        // silently ignored — same as native.
        let action = self.keybinds.action_for_gesture(
            gesture_name,
            input.modifiers.control_key(),
            input.modifiers.shift_key(),
            input.modifiers.alt_key(),
        );
        let Some(a) = action else {
            return;
        };
        {
            let mut core = input.input_context_core(renderer, &self.keybinds);
            let _ = dispatch::action_core::dispatch_compatible(&a, &mut core, None);
        }
        // Native picks the post-camera-change reprojection up in its
        // per-frame drain (`drain_camera_geometry_rebuild`); the
        // browser has no drain loop, so the same body runs here under
        // the same renderer-side dirty flag. Gating on the flag —
        // rather than rebuilding unconditionally — is what keeps a
        // non-camera Action bound to the wheel from paying for a
        // scene reprojection it did not cause.
        if renderer.take_connection_geometry_dirty() {
            rebuild_camera_geometry(
                &input.document,
                &input.interaction_mode,
                &mut input.app_scene,
                renderer,
                &mut input.scene_cache,
            );
        }
    }
}
