// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::RedrawRequested` arm: the browser's frame.
//!
//! Native's frame has an input-driven half and a time-driven half
//! (`app::heartbeat`). The browser's input-driven half runs at the
//! input: every handler that moves the camera reprojects at the
//! mover, and there is no drag state to drain. What the frame runs
//! is the time-driven half — one sample of the IPC injected-input
//! queue, ahead of the tick, where a winit event lands, then the
//! animation tick, the same `animation_drain::drain_animation_tick`
//! body native's heartbeat runs — and then the render, the sole entry
//! to the render path on web.
//!
//! The frame asks for the next frame itself while an animation is
//! running or injected input is queued. That is this target's
//! spelling of native's `needs_continuation` → `ControlFlow::Poll`,
//! and both read the one predicate, `animation_drain::keeps_ticking`,
//! plus the queue: a `request_redraw` from inside a `RedrawRequested`
//! is a `requestAnimationFrame` for the next frame (winit-web
//! coalesces it with any the input handlers ask for), so a running
//! animation is ticked once per browser frame, a paced drag lands one
//! sample per browser frame, and the asking stops on the frame that
//! completes the last of them, which keeps the idle canvas idle — the
//! event-driven redraw model `run` installs stays as it was for
//! everything but a running animation or a queued sample. Before #144
//! nothing on this target ticked an animation at all: an animated
//! `CustomMutation` started and never advanced.
//!
//! Under a virtual clock the frame moves no time and lands no sample,
//! as native's loop frame does not: the heartbeats that advance an
//! animation or land a sample are a step's (`run_wasm::step`).

#![cfg(target_arch = "wasm32")]

use crate::application::app::animation_drain;
use crate::application::app::ipc_scene::surface_of;
use crate::application::app::step::Beat;

impl super::WasmApp {
    pub(super) fn handle_redraw_requested(&mut self) {
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let Some(renderer) = renderer_borrow.as_mut() else {
            return;
        };
        // Before init completes there is no document to tick; the
        // first frame is drawn all the same.
        let mut continuing = false;
        if let Some(input) = input_borrow.as_mut() {
            // The loop's frame lands a sample and moves time only on a
            // real clock — `Beat::Loop`, the predicates native's loop
            // frame reads: under a virtual clock the heartbeats that
            // advance an animation or land a sample are a step's.
            let mut borrowed = super::Borrowed {
                input,
                renderer,
                keybinds: &self.keybinds,
                suppress_keys: &self.suppress_keys,
            };
            if Beat::Loop.lands_injected(&borrowed.input.clock) {
                borrowed.land_injected();
            }
            if Beat::Loop.moves_time(&borrowed.input.clock) {
                borrowed.tick_animations();
            }
            continuing = borrowed.needs_continuation();
        }
        // Coalesced by winit-web from any number of `request_redraw`
        // calls in the prior event chain. A transient skip asks
        // again, as native's does: no input may follow to ask for
        // it. A running animation, or a queued sample, asks for the
        // frame that advances it next.
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
    /// Land one injected-input sample, if one is queued, at the entry
    /// point its winit event reaches. With [`Self::tick_animations`],
    /// the time-driven half of a frame: what the loop's frame runs on
    /// a real clock, and what a step's heartbeat runs at each virtual
    /// instant (`run_wasm::step`), so a stepped frame and a real-time
    /// frame are one body.
    pub(super) fn land_injected(&mut self) {
        let renderer = &*self.renderer;
        if let Some(sample) = self.input.injected.pop(|point| surface_of(renderer, point)) {
            // The redraw this sample owes is the frame's own: the
            // frame asks again while samples are queued, and a step's
            // caller repaints once at its end.
            let _ = self.apply_injected(sample);
        }
    }

    /// The animation tick at the clock's instant: the body native's
    /// heartbeat runs.
    pub(super) fn tick_animations(&mut self) {
        animation_drain::drain_animation_tick(
            self.input.clock.now_whole_ms(),
            Some(&mut self.input.document),
            &self.input.interaction_mode,
            &mut self.input.mindmap_tree,
            &mut self.input.app_scene,
            self.renderer,
            &mut self.input.scene_cache,
        );
    }
}
