// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::CursorMoved` arm. Stores the latest cursor
//! position into `WasmInputState::cursor_pos` so the keyboard,
//! mouse-click, and wheel arms can read screen coordinates
//! without re-querying winit. WASM has no hover / drag state
//! today (full drag machine deferred to a later parity session),
//! so the body is just a position write.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::window::PhysicalPosition;

use super::PendingClick;

impl super::Borrowed<'_> {
    /// Returns whether to request a redraw. WASM doesn't yet have
    /// full drag/picker state (see `WasmInputState` field comments),
    /// so the CursorMoved redraw conditions native checks for
    /// collapse to "yes" for any in-progress click: conservative, a
    /// redraw on every move that arrives while a click is pending.
    pub(super) fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> bool {
        self.input.cursor_pos = (position.x, position.y);
        !matches!(self.input.pending_click, PendingClick::None)
    }
}
