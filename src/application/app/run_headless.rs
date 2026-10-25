// SPDX-License-Identifier: MPL-2.0

//! The headless driver for [`super::Application::run`] (`--headless`):
//! no event loop, no display, no window, no surface — the app runs on a
//! pump of its own, driven by IPC commands and nothing else.
//!
//! **The same loop body, scheduled by hand.** Each pass of the pump is
//! one iteration of the body winit's `about_to_wait` runs
//! ([`pump::iterate`]), so what a frame does and in what order is not
//! this driver's to decide. What is its own is what winit does around
//! that body:
//!
//! - **Frames.** A frame owed — the first one, unasked, as a new
//!   window's, and then each ask no render has drawn, as the redraw
//!   ledger keeps them (`RedrawLedger::frame_owed`) — is drawn here
//!   before the pump next waits, where winit would deliver the
//!   `RedrawRequested`, then followed by another iteration, as winit
//!   runs `about_to_wait` after every iteration that delivered an
//!   event. A headless frame is bookkeeping, not pixels
//!   (`renderer::Sink::Headless`): a capture draws what it shows.
//!   Frames are paced to one per [`HEARTBEAT_MS`] of wall time, the
//!   refresh a window's present blocks on: continuous work (a real-time
//!   animation, a paced drag) advances at a display's rate instead of
//!   spinning a core.
//! - **Waiting.** The pump parks its thread — until the iteration's
//!   deadline, or indefinitely when it has none — and the IPC threads
//!   wake it with `Thread::unpark`, the role winit's `EventLoopProxy`
//!   plays under a window. An unpark that lands while the pump is busy
//!   is kept for its next park, so a request is never slept through.
//! - **Input.** There is none but the IPC `input` family, which enters
//!   below winit already: the same `on_*` entry points a window's
//!   events reach.
//! - **Time.** The clock starts virtual (`run_native_init::build`):
//!   with no one watching, time moves only when `clock.step` moves it.
//!
//! **What ends it.** `Action::Quit` — sent over IPC as `act.action
//! Quit` — which is the graceful exit a window's close button is, through
//! the same `pump::shut_down`; or a one-shot launch's capture
//! (`--screenshot`), with the code it reports. A signal is an unclean
//! exit here as under a window: the socket is left to self-heal.
//!
//! Native-only (`CLAUDE.md` "Dual-target status", "Headless").

#![cfg(not(target_arch = "wasm32"))]

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use super::freeze_watchdog::FreezeWatchdog;
use super::pump::{self, Next, Running};
use super::run_native_init;
use super::shell::Shell;
use super::Options;
use crate::application::clock::HEARTBEAT_MS;
use crate::application::ipc::IpcServer;

/// The wall-clock interval between two headless frames: one heartbeat,
/// the cadence a 60 Hz display paces a window's frames at.
const FRAME_INTERVAL: Duration = Duration::from_millis(HEARTBEAT_MS);

/// Run the app with no window until a one-shot capture ends it, and
/// return the code it ended with.
///
/// The watchdog and the IPC listener start first, as they do under a
/// window, so a bad socket path fails the launch before the renderer
/// bootstrap; requests that arrive while the app is still being built
/// wait in the queue for the first iteration.
pub(super) fn run(options: Options) -> ExitCode {
    let watchdog = FreezeWatchdog::spawn();
    let main = thread::current();
    let mut ipc = options.ipc_socket.as_deref().map(|path| {
        let main = main.clone();
        IpcServer::start(path, move || main.unpark())
            .expect("ipc: the `--ipc` listener could not start (format/ipc.md \"Socket, flag, lifecycle\")")
    });
    let mut init = run_native_init::build(&options, Shell::Headless, watchdog.clone());
    watchdog.unparked();
    // What the loop still owes, taken when it fires; `options.one_shot`
    // stays the launch's request, which `build` read above.
    let mut one_shot = options.one_shot.clone();
    let mut pacer = Pacer::default();
    let code = loop {
        watchdog.unparked();
        let next = pump::iterate(&mut Running {
            init: &mut init,
            ipc: &mut ipc,
            one_shot: &mut one_shot,
        });
        if let Next::Exit(code) = next {
            break code;
        }
        if init.redraws.frame_owed() {
            pacer.pace(Instant::now());
            init.draw_frame();
            pump::evaluate_wait(&mut ipc, &init);
            continue;
        }
        watchdog.parked();
        match next {
            Next::Now | Next::Exit(_) => {}
            Next::Until(deadline) => {
                if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                    thread::park_timeout(wait);
                }
            }
            Next::Idle => thread::park(),
        }
    };
    pump::shut_down(&mut ipc, Some(&mut init));
    code
}

/// Holds headless frames to one per [`FRAME_INTERVAL`].
#[derive(Debug, Default)]
struct Pacer {
    /// When the last frame was let through.
    last: Option<Instant>,
}

impl Pacer {
    /// How long a frame asked for at `now` waits before it is drawn: the
    /// rest of the interval since the last frame, or nothing once the
    /// interval has passed. Records `now + wait` as the new last frame.
    fn next_wait(&mut self, now: Instant) -> Duration {
        let wait = self.last.map_or(Duration::ZERO, |last| {
            (last + FRAME_INTERVAL).saturating_duration_since(now)
        });
        self.last = Some(now + wait);
        wait
    }

    /// Wait out [`Self::next_wait`]. A sleep rather than a park: the
    /// interval is a display's refresh, which an IPC wake does not
    /// shorten under a window either, and it is far inside the
    /// watchdog's threshold.
    fn pace(&mut self, now: Instant) {
        let wait = self.next_wait(now);
        if !wait.is_zero() {
            thread::sleep(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first frame is drawn at once; one asked for inside the
    /// interval waits out its rest; one asked for after it does not wait
    /// at all — and the interval runs from when a frame was let through,
    /// not from when it was asked for, so a steady stream is one per
    /// interval.
    #[test]
    fn test_headless_frames_are_paced_to_one_per_heartbeat() {
        let t0 = Instant::now();
        let mut pacer = Pacer::default();
        assert_eq!(pacer.next_wait(t0), Duration::ZERO, "the first frame");
        assert_eq!(
            pacer.next_wait(t0 + Duration::from_millis(4)),
            Duration::from_millis(12)
        );
        // Let through at 16 ms; the next is due at 32 ms.
        assert_eq!(
            pacer.next_wait(t0 + Duration::from_millis(20)),
            Duration::from_millis(12)
        );
        assert_eq!(
            pacer.next_wait(t0 + Duration::from_millis(100)),
            Duration::ZERO,
            "long after"
        );
        assert_eq!(FRAME_INTERVAL, Duration::from_millis(16));
    }
}
