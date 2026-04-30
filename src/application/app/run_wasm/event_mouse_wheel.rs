// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::MouseWheel` arm. Converts the scroll delta into a
//! zoom factor, clears any pending click (zoom invalidates the
//! click-down screen position), routes the zoom through the
//! renderer's `CameraZoom` decree, then issues a scene-only
//! rebuild — zoom touches connection sample spacing and the
//! viewport cull rect but not the node text tree.

#![cfg(target_arch = "wasm32")]

use winit::event::MouseScrollDelta;

use super::PendingClick;
use crate::application::app::scene_rebuild::rebuild_scene_only;
use crate::application::common::RenderDecree;

impl super::WasmApp {
    pub(super) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let scroll_y = match delta {
            MouseScrollDelta::LineDelta(_, y) => y as f64,
            MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
        };
        let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        if let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) {
            // A zoom mid-click invalidates the pending selection:
            // the canvas coord the user pressed over has shifted
            // to a new screen position, so committing the pending
            // click on the eventual mouse-up would select whatever
            // now sits under the release cursor — not what the
            // user pressed on. Clear it so release falls through
            // to empty-click handling.
            input.pending_click = PendingClick::None;
            renderer.process_decree(RenderDecree::CameraZoom {
                screen_x: input.cursor_pos.0 as f32,
                screen_y: input.cursor_pos.1 as f32,
                factor: factor as f32,
            });
            // Zoom touches scene geometry (connection glyph
            // sample spacing, viewport cull rect) but not the
            // node text tree — scene-only rebuild is enough.
            rebuild_scene_only(
                &input.document,
                &mut input.app_scene,
                renderer,
                &mut input.scene_cache,
            );
        }
    }
}
