// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::CursorMoved` arm. Stores the latest cursor
//! position into `WasmInputState::cursor_pos` so the keyboard,
//! mouse-click, and wheel arms can read screen coordinates
//! without re-querying winit. WASM has no hover / drag state
//! today (full drag machine deferred to a later parity session),
//! so the body is just a position write.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::window::PhysicalPosition;

impl super::WasmApp {
    pub(super) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(input) = self.input.borrow_mut().as_mut() {
            input.cursor_pos = (position.x, position.y);
        }
    }
}
