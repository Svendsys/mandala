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

use crate::application::app::dispatch::{self, DispatchOutcome};
use crate::application::app::touch_gesture::Phase;
use std::sync::atomic::{AtomicBool, Ordering};
use web_time::Instant;
use winit::event::Touch;

/// One-shot warn-log latch: fires the first time a recognised
/// touch gesture maps to an Action whose body is `NativeOnly`,
/// so a mobile-browser user who taps-and-holds (default-bound to
/// `EnterResizeMode`) sees evidence in the dev console rather
/// than wondering why their gesture is dead. Static + `swap` is
/// the same shape `event_mouse_click::handle_right_button` uses
/// for the equivalent right-button warning.
static WARNED_NATIVE_ONLY: AtomicBool = AtomicBool::new(false);

impl super::WasmApp {
    /// Handle one `WindowEvent::Touch`. Returns true when the
    /// runtime should request a redraw (always true for
    /// Started/Moved so future cursor-following overlays update;
    /// true for Ended only when a gesture was dispatched).
    pub(super) fn handle_touch_event(&mut self, touch: Touch) -> bool {
        let phase = dispatch::touch_phase(touch.phase);
        let pos = (touch.location.x, touch.location.y);
        let now = Instant::now();
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut())
        else {
            return false;
        };
        // Phase translation, recognizer ingest + tick, and the
        // gesture-to-Action lookup are the shared body native runs
        // too. What is left below is the browser's own dispatch:
        // `dispatch_compatible` plus the `NativeOnly` warn-log.
        if let Some(d) = dispatch::drive_touch_event(
            &mut input.touch_recognizer,
            &self.keybinds,
            phase,
            touch.id,
            pos,
            now,
        ) {
            // Move the cursor to the gesture's reported pos so the
            // dispatched Action sees the right cursor.
            input.cursor_pos = d.cursor_pos;
            let name = d.gesture_name;
            if let Some(a) = d.action {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let outcome = dispatch::action_core::dispatch_compatible(&a, &mut core, None);
                // Whole-PR review BLK-1: when the bound Action is
                // `NativeOnly` (e.g. `EnterResizeMode`,
                // `FastResizeStart` — both default-bound to touch
                // gestures by `keybinds/config.rs:297-298`),
                // `dispatch_compatible` returns `Unhandled` and
                // there's no graceful fallback on WASM. A user who
                // long-presses on mobile gets *literally nothing* —
                // no log, no chrome, no model change. Warn-log once
                // per session so the failure is at least observable
                // in the dev console; the underlying fix (lifting
                // those Actions to `Compatible` + porting the
                // DragState plumbing) is tracked in the plan's
                // open follow-ups.
                if matches!(outcome, DispatchOutcome::Unhandled)
                    && !WARNED_NATIVE_ONLY.swap(true, Ordering::Relaxed)
                {
                    log::warn!(
                        "touch gesture '{}' dispatched a NativeOnly action ({:?}) — \
                         no-op on WASM until DragState / modal-stealer plumbing \
                         lands cross-platform (SECTIONS_BORDERS_RESIZE_PLAN.md \
                         Open follow-ups). Rebind {} to a Compatible action \
                         (e.g. ZoomIn / SelectAll / a custom macro) to opt out.",
                        name, a, name
                    );
                }
                return true;
            }
        }
        matches!(phase, Phase::Started | Phase::Moved)
    }
}
