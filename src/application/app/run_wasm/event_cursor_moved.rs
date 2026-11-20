// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::CursorMoved` arm. Stores the latest cursor
//! position into `WasmInputState::cursor_pos` so the keyboard,
//! mouse-click, and wheel arms can read screen coordinates
//! without re-querying winit, and moves the hover a node's `on.hover`
//! verb is held for (`families::verbs`), as native's cursor-move
//! handler does; and the `WindowEvent::CursorLeft` arm, which releases
//! that hover as native's does. WASM has no drag state today (full drag
//! machine deferred to a later parity session).

#![cfg(target_arch = "wasm32")]

use crate::application::app::dispatch::families::edits::Projected;
use crate::application::app::dispatch::families::verbs;
use crate::application::platform::window::PhysicalPosition;

impl super::Borrowed<'_> {
    /// Returns whether to request a redraw. WASM doesn't yet have
    /// full drag/picker state (see `WasmInputState` field comments),
    /// so the CursorMoved redraw conditions native checks for
    /// collapse to "yes" for any in-progress click: conservative, a
    /// redraw on every move that arrives while a click is pending —
    /// and for a hover that changed, whose verb may have started or
    /// stopped what is drawn. A move while a press is down leaves the
    /// hover where it was, as native's does during a gesture.
    pub(super) fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> bool {
        self.input.cursor_pos = (position.x, position.y);
        if self.input.pending_click.is_some() {
            return true;
        }
        let now_ms = self.input.clock.now_ms();
        let canvas_pos = self
            .renderer
            .screen_to_canvas(position.x as f32, position.y as f32);
        let input = &mut *self.input;
        let Some(view) = input.view.as_mut() else {
            return false;
        };
        let hit = view.hit(canvas_pos);
        let hovered = verbs::hovered(input.document.doc(), hit.as_ref());
        // A hand over what a click triggers something on, the default
        // elsewhere; set only when it changes.
        let points = verbs::points_at(input.document.doc(), hit.as_ref());
        if points != input.cursor_is_hand {
            input.cursor_is_hand = points;
            self.renderer.set_cursor(if points {
                winit::window::CursorIcon::Pointer
            } else {
                winit::window::CursorIcon::Default
            });
        }
        verbs::Host {
            doc: &mut input.document,
            drawn: &mut Projected {
                view,
                renderer: &mut *self.renderer,
            },
            edits: &input.edits,
        }
        .hover(hovered, now_ms)
    }

    /// `WindowEvent::CursorLeft`: the pointer left the canvas, and with
    /// it the node it hovered, so the hover is released (ruling D3.2
    /// item 4), unless a click is pending, as a move then leaves it.
    /// Returns whether to request a redraw: whether the hover changed.
    pub(super) fn on_cursor_left(&mut self) -> bool {
        if self.input.pending_click.is_some() {
            return false;
        }
        let now_ms = self.input.clock.now_ms();
        let input = &mut *self.input;
        let Some(view) = input.view.as_mut() else {
            return false;
        };
        verbs::Host {
            doc: &mut input.document,
            drawn: &mut Projected {
                view,
                renderer: &mut *self.renderer,
            },
            edits: &input.edits,
        }
        .hover(None, now_ms)
    }
}
