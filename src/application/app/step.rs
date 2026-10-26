// SPDX-License-Identifier: MPL-2.0

//! A clock step's schedule: the heartbeats it runs the frame at, and
//! the trait a target's frame implements to be run there
//! (`format/ipc.md` §`clock`; `work_plans/LLM_IPC.md` §D9).
//!
//! **What a step runs.** A heartbeat every
//! [`HEARTBEAT_MS`](crate::application::clock::HEARTBEAT_MS) virtual
//! milliseconds, the last on the step's end
//! ([`heartbeat_times`]),
//! each the whole frame. Between two of them, a running recording's
//! frame boundary gets a tick of its own ([`Beat::Boundary`]): the
//! clock stops on the boundary exactly, the frame drains and the
//! recorder takes its frame, so a stepped recording's frames are its
//! `1000/fps` instants, each rounded up to a whole millisecond, and
//! nothing else. No injected sample lands on a boundary tick, so
//! recording a gesture never changes when the gesture's samples land.
//!
//! Cross-platform by construction: the schedule reads a clock and a
//! recorder's next due instant, both plain data. Native's
//! `heartbeat` implements [`Steps`] over its handler context — the
//! whole frame, the recorder, the freeze watchdog — and the browser's
//! `run_wasm::step` over its own, which has no recorder and so never
//! sees a [`Beat::Boundary`]. One schedule, so a stepped run is the
//! same function of its command sequence on both targets.

use crate::application::clock::{heartbeat_times, AppClock, StepRequest};

/// Which frame is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Beat {
    /// One iteration of the event loop. In real time, the whole frame;
    /// under a virtual clock time has not moved, so the input-driven
    /// half alone.
    Loop,
    /// A step's heartbeat, on the
    /// [`HEARTBEAT_MS`](crate::application::clock::HEARTBEAT_MS)
    /// cadence: the whole frame.
    Cadence,
    /// A running recording's frame boundary, between two cadence
    /// heartbeats of a step: the whole frame but the injected sample.
    Boundary,
}

impl Beat {
    /// Whether this frame runs the time-driven half.
    pub(super) fn moves_time(self, clock: &AppClock) -> bool {
        match self {
            Beat::Loop => !clock.is_virtual(),
            Beat::Cadence | Beat::Boundary => true,
        }
    }

    /// Whether this frame lands one injected-input sample.
    pub(super) fn lands_injected(self, clock: &AppClock) -> bool {
        match self {
            Beat::Loop => !clock.is_virtual(),
            Beat::Cadence => true,
            Beat::Boundary => false,
        }
    }
}

/// What a step drives: a clock it stops at each instant the step
/// names, a recording whose boundaries it stops at too, and the frame
/// it runs there. The handler context is the app's; the tests drive a
/// real recording on a real clock through the same [`run_step`], so the
/// schedule they pin is the one the app runs.
pub(super) trait Steps {
    /// The clock's reading.
    fn now_ms(&self) -> f64;
    /// When the running recording's next frame is due, on the same
    /// clock; `None` with nothing recording or the recording full.
    fn next_frame_due(&self) -> Option<f64>;
    /// Stop the clock at `at_ms` and run the frame `beat` names there.
    fn beat(&mut self, beat: Beat, at_ms: f64);
}

/// One step's loop (see the module doc): a [`Beat::Cadence`] heartbeat
/// at each of [`heartbeat_times`], and before each, a
/// [`Beat::Boundary`] tick at every frame boundary of a running
/// recording that falls strictly between it and the last instant run —
/// one on a heartbeat is the heartbeat's to take. Returns the cadence
/// heartbeats run.
pub(super) fn run_step(app: &mut impl Steps, request: StepRequest) -> u64 {
    let start = app.now_ms();
    let mut frames_run = 0;
    for at in heartbeat_times(start, request) {
        // Ends: each tick stops the clock on `due`, so a boundary the
        // recorder did not move past fails `due > now` next time round.
        while let Some(due) = app
            .next_frame_due()
            .filter(|due| *due > app.now_ms() && *due < at)
        {
            app.beat(Beat::Boundary, due);
        }
        app.beat(Beat::Cadence, at);
        frames_run += 1;
    }
    frames_run
}
