// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::RedrawRequested` arm: the browser's frame.
//!
//! Native's frame has an input-driven half and a time-driven half
//! (`app::heartbeat`). The browser's input-driven half runs at the
//! input: every handler that moves the camera reprojects at the
//! mover, and there is no drag state to drain. What the frame runs
//! is the time-driven half — the animation tick, the same
//! `animation_drain::drain_animation_tick` body native's heartbeat
//! runs — and then the render, the sole entry to the render path on
//! web.
//!
//! The frame asks for the next frame itself while an animation is
//! running. That is this target's spelling of native's
//! `needs_continuation` → `ControlFlow::Poll`, and both read the one
//! predicate, `animation_drain::keeps_ticking`: a `request_redraw`
//! from inside a `RedrawRequested` is a `requestAnimationFrame` for
//! the next frame (winit-web coalesces it with any the input
//! handlers ask for), so a running animation is ticked once per
//! browser frame and the asking stops on the frame it completes,
//! which keeps the idle canvas idle — the event-driven redraw model
//! `run` installs stays as it was for everything but a running
//! animation. Before #144 nothing on this target ticked an
//! animation at all: an animated `CustomMutation` started and never
//! advanced.

#![cfg(target_arch = "wasm32")]

use crate::application::app::animation_drain;

impl super::WasmApp {
    pub(super) fn handle_redraw_requested(&mut self) {
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let Some(renderer) = renderer_borrow.as_mut() else {
            return;
        };
        // Before init completes there is no document to tick; the
        // first frame is drawn all the same.
        let mut animating = false;
        if let Some(input) = input_borrow.as_mut() {
            // The loop's frame moves time only on a real clock, as
            // native's `Beat::Loop` does: under a virtual clock the
            // heartbeats that advance an animation are a step's. The
            // browser's clock is always real today (`WasmInputState::clock`);
            // the guard is what makes the day it is not a driver's
            // work alone.
            if !input.clock.is_virtual() {
                animation_drain::drain_animation_tick(
                    input.clock.now_whole_ms(),
                    Some(&mut input.document),
                    &input.interaction_mode,
                    &mut input.mindmap_tree,
                    &mut input.app_scene,
                    renderer,
                    &mut input.scene_cache,
                );
            }
            animating = animation_drain::keeps_ticking(Some(&input.document), &input.clock);
        }
        // Coalesced by winit-web from any number of `request_redraw`
        // calls in the prior event chain. A transient skip asks
        // again, as native's does: no input may follow to ask for
        // it. A running animation asks for the frame that ticks it
        // next.
        let skipped = renderer.process().is_err_and(|skip| skip.is_transient());
        if skipped || animating {
            renderer.request_redraw();
        }
    }
}
