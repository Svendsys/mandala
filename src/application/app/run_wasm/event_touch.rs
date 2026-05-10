// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::Touch` arm for WASM. Mirrors the native shape
//! at `run_native.rs::dispatch_touch_event` — feed the
//! [`crate::application::app::touch_gesture::TouchGestureRecognizer`]
//! state machine, dispatch any recognised gesture through the
//! `MouseGesture` keybind table.
//!
//! Touch parity ships in `SECTIONS_BORDERS_RESIZE_PLAN.md` Batch
//! 7. Pre-Batch-7 the WASM event-loop catch-all dropped touch
//! events silently, leaving mobile-browser users with no input
//! path at all (the canvas only saw mouse events synthesised by
//! the browser, which never fire for touch-and-hold or
//! multi-finger gestures).

#![cfg(target_arch = "wasm32")]

use crate::application::app::dispatch;
use crate::application::app::touch_gesture::{Phase, RecognizedGesture};
use web_time::Instant;
use winit::event::{Touch, TouchPhase};

impl super::WasmApp {
    /// Handle one `WindowEvent::Touch`. Returns true when the
    /// runtime should request a redraw (always true for
    /// Started/Moved so future cursor-following overlays update;
    /// true for Ended only when a gesture was dispatched).
    pub(super) fn handle_touch_event(&mut self, touch: Touch) -> bool {
        let phase = match touch.phase {
            TouchPhase::Started => Phase::Started,
            TouchPhase::Moved => Phase::Moved,
            TouchPhase::Ended | TouchPhase::Cancelled => Phase::Ended,
        };
        let pos = (touch.location.x, touch.location.y);
        let now = Instant::now();
        // Recogniser ingest + tick happens under the input
        // borrow; dispatch happens after the borrow drops so
        // `self.dispatch_action` can re-borrow `self.input`.
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut())
        else {
            return false;
        };
        let from_ingest = input.touch_recognizer.ingest(phase, touch.id, pos, now);
        let from_tick = input.touch_recognizer.tick(now);
        let recognised: Option<RecognizedGesture> = from_ingest.or(from_tick);
        if let Some(g) = recognised {
            // Move the cursor to the gesture's reported pos so the
            // dispatched Action sees the right cursor. Mirrors
            // `dispatch_touch_event` on native.
            input.cursor_pos = g.pos();
            let name = g.mouse_gesture().key_name();
            let action = self.keybinds.action_for_gesture(name, false, false, false);
            if let Some(a) = action {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let _ = dispatch::action_core::dispatch_compatible(&a, &mut core);
                return true;
            }
        }
        matches!(phase, Phase::Started | Phase::Moved)
    }
}
