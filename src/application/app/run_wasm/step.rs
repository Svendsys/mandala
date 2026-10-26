// SPDX-License-Identifier: MPL-2.0

//! The browser's clock step driver: `clock.step` run over this
//! target's frame (`format/ipc.md` §`clock`; `CLAUDE.md` "Dual-target
//! status", "Virtual-clock stepping").
//!
//! The schedule is `app::step::run_step`, the one native's heartbeat
//! runs; what this module supplies is the frame it stops the clock to
//! run at each heartbeat — the time-driven half the browser's loop
//! frame runs in real time (`event_redraw_requested`): one injected
//! sample, then the animation tick. There is no recorder on this
//! target, so no step here ever sees a [`Beat::Boundary`]; the arm is
//! written for it anyway, so the day a recorder lands the schedule
//! needs nothing.
//!
//! Synchronous, like native's: when [`step`] returns, the app is what
//! it is at the step's end, and the caller owes one redraw to show it.

#![cfg(target_arch = "wasm32")]

use crate::application::app::step::{run_step, Beat, Steps};
use crate::application::clock::{ClockRefusal, StepRequest, Stepped};

use super::Borrowed;

impl Steps for Borrowed<'_> {
    fn now_ms(&self) -> f64 {
        self.input.clock.now_ms()
    }

    fn next_frame_due(&self) -> Option<f64> {
        None
    }

    fn beat(&mut self, beat: Beat, at_ms: f64) {
        self.input.clock.advance_to(at_ms);
        if beat.lands_injected(&self.input.clock) {
            self.land_injected();
        }
        if beat.moves_time(&self.input.clock) {
            self.tick_animations();
        }
    }
}

/// Advance a virtual clock by `request`, running the frame at every
/// heartbeat along the way. Refused on a real clock and mid-step, as
/// native's is.
pub(super) fn step(app: &mut Borrowed<'_>, request: StepRequest) -> Result<Stepped, ClockRefusal> {
    if !app.input.clock.is_virtual() {
        return Err(ClockRefusal::Real);
    }
    if !app.input.clock.begin_step() {
        return Err(ClockRefusal::Stepping);
    }
    let frames_run = run_step(app, request);
    app.input.clock.end_step();
    Ok(Stepped {
        now_ms: app.input.clock.now_ms(),
        frames_run,
    })
}
