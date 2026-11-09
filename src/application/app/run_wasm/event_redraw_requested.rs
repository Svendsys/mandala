// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::RedrawRequested` arm: the browser's frame.
//!
//! Native's frame has an input-driven half and a time-driven half
//! (`app::heartbeat`). The browser has no drag state to drain, so what
//! its frame runs is one sample of the IPC injected-input queue, where
//! a winit event lands, then the view's sync — the `MapView::sync`
//! native's `heartbeat::drain_inputs` runs, which hands whatever the
//! document did since the last frame to the projector and the layer
//! renderer — and then the render, the sole entry to the render path on
//! web.
//!
//! The frame asks for the next frame itself while injected input is
//! queued. That is this target's spelling of native's
//! `needs_continuation` → `ControlFlow::Poll`: a `request_redraw` from
//! inside a `RedrawRequested` is a `requestAnimationFrame` for the next
//! frame (winit-web coalesces it with any the input handlers ask for),
//! so a paced drag lands one sample per browser frame, and the asking
//! stops on the frame that lands the last of them, which keeps the idle
//! canvas idle — the event-driven redraw model `run` installs.
//!
//! Under a virtual clock the frame lands no sample, as native's loop
//! frame does not: the heartbeats that land one are a step's
//! (`run_wasm::step`).

#![cfg(target_arch = "wasm32")]

use crate::application::app::ipc_scene::surface_of;
use crate::application::app::step::Beat;

impl super::WasmApp {
    /// The browser's frame: land a sample, sync the view, render, and
    /// ask for the next frame while one is owed.
    pub(super) fn handle_redraw_requested(&mut self) {
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let Some(renderer) = renderer_borrow.as_mut() else {
            return;
        };
        // Before init completes there is no document to sync; the
        // first frame is drawn all the same.
        let mut continuing = false;
        if let Some(input) = input_borrow.as_mut() {
            let mut borrowed = super::Borrowed {
                input,
                renderer,
                keybinds: &self.keybinds,
            };
            // The loop's frame lands a sample only on a real clock —
            // `Beat::Loop`, the predicate native's loop frame reads.
            if Beat::Loop.lands_injected(&borrowed.input.clock) {
                borrowed.land_injected();
            }
            borrowed.sync_view();
            continuing = borrowed.needs_continuation();
        }
        // Coalesced by winit-web from any number of `request_redraw`
        // calls in the prior event chain. A transient skip asks
        // again, as native's does: no input may follow to ask for
        // it. A queued sample asks for the frame that lands it next.
        let drew = renderer.process();
        let skipped = drew.is_err_and(|skip| skip.is_transient());
        let ask_again = skipped || continuing;
        if let Some(input) = input_borrow.as_mut() {
            // Drawn means rendered, as native's ledger counts it; a
            // frame asked for again from here is owed again.
            input.redraw_owed = drew.is_err() || ask_again;
        }
        if ask_again {
            renderer.request_redraw();
        }
        drop(renderer_borrow);
        drop(input_borrow);
        // A frame drawn is when an owed frame stops being owed, and
        // when a sample landed: evaluate the pending `clock.wait`, if
        // one is held, against the app as it stands now.
        self.ipc_after_frame();
    }
}

impl super::Borrowed<'_> {
    /// Land the injected-input queue's next sample, if one is queued,
    /// through the entry point its winit event reaches.
    pub(super) fn land_injected(&mut self) {
        let renderer = &*self.renderer;
        if let Some(sample) = self.input.injected.pop(|point| surface_of(renderer, point)) {
            // The redraw this sample owes is the frame's own: the
            // frame asks again while samples are queued, and a step's
            // caller repaints once at its end.
            let _ = self.apply_injected(sample);
        }
    }
}
