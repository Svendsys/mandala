// SPDX-License-Identifier: MPL-2.0

//! One iteration of the native main loop — the body both drivers run:
//! winit's `about_to_wait` under a window (`app::run_native`), and the
//! headless pump with none (`app::run_headless`).
//!
//! **One body, two schedulers.** What an iteration *does* is the
//! same whichever driver runs it, and lives here once ([`iterate`]):
//! the frame up to the recorder (`app::heartbeat`), the IPC drain
//! between the two, the redraw the iteration owes, the FPS overlay's
//! idle flip, the clock waits, the map the next client is told about,
//! and whether the loop ends — a dispatched `Action::Quit`, a one-shot
//! launch's capture. What it returns is *when the next one should run*
//! ([`Next`]), as a value: winit turns it into a `ControlFlow`, the
//! headless pump into a park. So the drivers differ only in how they
//! wait and in where a frame comes from — winit delivers
//! `RedrawRequested`, the headless pump draws the frame the redraw
//! ledger says is owed — and the order of what runs is not theirs to
//! change. That order is pinned by the tests below against a recording
//! fake ([`Iteration`] is the seam), so it is held without a GPU
//! (TEST_CONVENTIONS §T8).
//!
//! **Shut down once, too.** [`shut_down`] is what a graceful exit does
//! in either driver: the client hears `shutdown`, a recording in
//! progress is finished, and the artifacts dir goes. A loop that ends
//! with the one-shot's capture still owed — a quit, a closed window —
//! ends with `1` ([`ended_early`]).
//!
//! Native-only, with both drivers (`CLAUDE.md` "Dual-target status").

#![cfg(not(target_arch = "wasm32"))]

use std::process::ExitCode;
use std::time::Instant;

use crate::application::ipc::{self, DrainOutcome, IpcServer};

use super::capture_flow;
use super::heartbeat::Beat;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::OneShot;

/// When the next iteration should run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Next {
    /// At once: continuous work is in flight, or the IPC queue was not
    /// drained. (winit: `ControlFlow::Poll`, which a window's present
    /// paces; the headless pump paces its frames itself.)
    Now,
    /// At this instant, or at the next event before it: the FPS
    /// overlay's idle grace, a recording's next frame, a `clock.wait`'s
    /// deadline. (`ControlFlow::WaitUntil`.)
    Until(Instant),
    /// At the next event. (`ControlFlow::Wait`.)
    Idle,
    /// Never: the one-shot launch has its capture, and the process ends
    /// with this code.
    Exit(ExitCode),
}

/// The app as one iteration drives it: each method is one step of
/// [`iterate`], in the order it runs them. Implemented by [`Running`]
/// over the real app, and in the tests by a fake that records the
/// order.
pub(super) trait Iteration {
    /// Whether the loop must keep iterating with no further input
    /// (`heartbeat::needs_continuation`).
    fn needs_continuation(&self) -> bool;
    /// The frame up to the recorder: one injected sample when it lands
    /// one, then the drains (`Beat::Loop`).
    fn drain_frame(&mut self);
    /// Execute the queued IPC requests.
    fn drain_ipc(&mut self) -> DrainOutcome;
    /// The recording's frame, if one is due.
    fn record_tick(&mut self);
    /// The loop is about to sleep with a recording running: spend the
    /// sleep encoding.
    fn record_idle(&mut self);
    /// Ask for a frame.
    fn request_redraw(&mut self);
    /// The FPS overlay's active-to-idle flip, given whether work
    /// continues: flips it (and asks for the frame that shows it) once
    /// the grace has passed, and returns the instant the grace ends
    /// while it has not.
    fn fps_idle(&mut self, continuing: bool) -> Option<Instant>;
    /// The earliest instant something other than the FPS overlay needs
    /// the loop awake. An active recording is a timer source — an idle
    /// canvas would otherwise park with no heartbeat to take its next
    /// frame (`format/ipc.md` §`capture.record_start`) — and so is a
    /// pending `clock.wait`'s deadline, which is wall-clock under either
    /// clock mode: it guards the client's liveness, and a parked loop
    /// must still answer it.
    fn deadline(&self) -> Option<Instant>;
    /// Evaluate the pending `clock.wait` against the app as it stands.
    fn evaluate_wait(&mut self);
    /// Tell the IPC server which map is open, for the next `hello`.
    fn publish_map(&mut self);
    /// Whether the loop ends here, and with what code: when
    /// `Action::Quit` was dispatched, or when the one-shot launch has its
    /// capture (or has failed it). `None` while neither.
    fn exit(&mut self) -> Option<ExitCode>;
}

/// Run one iteration of the main loop over `app`, and say when the next
/// one should run.
///
/// The order, and why each step stands where it does:
///
/// 1. The continuation predicate, **before** anything runs: if the
///    loop entered this iteration with continuous work in flight, the
///    drains do work that must land on screen.
/// 2. The frame up to the recorder ([`Iteration::drain_frame`]): in
///    real time one injected-input sample ahead of the input drain —
///    where a winit event lands — then the drains and the animation
///    tick; under a virtual clock the input-driven drains alone.
/// 3. The IPC drain, after the input drain, so a command observes every
///    input sent before it and its effects are ordered after them.
/// 4. The recorder, after the IPC drain, so a frame shows what this
///    iteration's commands did.
/// 5. The continuation predicate again, which the frame's
///    camera-geometry rebuild can have changed.
/// 6. Encoding a recording's backlog — only when nothing else wants
///    the iteration: no frame owed, no IPC left queued.
/// 7. The redraw: if work happened (continuing on entry), is pending
///    (continuing now — the OR catches an animation's last frame), or a
///    command changed pixels.
/// 8. The FPS overlay's idle flip, deferred by its grace so a drag's
///    between-drain gaps do not flicker it.
/// 9. The clock wait, after the redraw this iteration owes has been
///    asked for, so a pixels-affecting command of the iteration counts
///    as a frame still owed.
/// 10. The map the next client's `hello` names, after every handler of
///     the iteration has run.
/// 11. The exit check, last — a dispatched `Action::Quit`, or the
///     one-shot's capture — so it sees the iteration's whole effect,
///     at the same point the clock wait is evaluated.
pub(super) fn iterate(app: &mut impl Iteration) -> Next {
    let was_continuing = app.needs_continuation();
    app.drain_frame();
    let ipc = app.drain_ipc();
    app.record_tick();
    let still_continuing = app.needs_continuation();
    if !still_continuing && !was_continuing && !ipc.repaint && !ipc.backlog {
        app.record_idle();
    }
    if was_continuing || still_continuing || ipc.repaint {
        app.request_redraw();
    }
    let fps_deadline = app.fps_idle(still_continuing);
    let wake_by = [fps_deadline, app.deadline()].into_iter().flatten().min();
    app.evaluate_wait();
    app.publish_map();
    if let Some(code) = app.exit() {
        return Next::Exit(code);
    }
    if still_continuing || ipc.backlog {
        Next::Now
    } else if let Some(at) = wake_by {
        Next::Until(at)
    } else {
        Next::Idle
    }
}

/// The real app, as [`iterate`] drives it: the state, the IPC server
/// when `--ipc` was passed, and the one-shot capture still owed when
/// `--screenshot` was.
pub(super) struct Running<'a> {
    pub(super) init: &'a mut InitState,
    pub(super) ipc: &'a mut Option<IpcServer>,
    /// Taken when it fires, so it fires once.
    pub(super) one_shot: &'a mut Option<OneShot>,
}

impl Iteration for Running<'_> {
    fn needs_continuation(&self) -> bool {
        self.init.needs_continuation()
    }

    fn drain_frame(&mut self) {
        self.init.input_context().drain_frame(Beat::Loop);
    }

    fn drain_ipc(&mut self) -> DrainOutcome {
        drain_ipc(self.ipc, Some(self.init))
    }

    fn record_tick(&mut self) {
        self.init.input_context().record_tick(Beat::Loop);
    }

    fn record_idle(&mut self) {
        self.init.record_idle();
    }

    fn request_redraw(&mut self) {
        self.init.request_redraw();
    }

    fn fps_idle(&mut self, continuing: bool) -> Option<Instant> {
        self.init.fps_idle(continuing)
    }

    fn deadline(&self) -> Option<Instant> {
        [
            super::record_flow::deadline(&self.init.recorder, &self.init.clock),
            self.ipc.as_ref().and_then(IpcServer::wait_deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn evaluate_wait(&mut self) {
        evaluate_wait(self.ipc, self.init);
    }

    fn publish_map(&mut self) {
        if let Some(server) = self.ipc.as_mut() {
            server.publish_map(
                self.init
                    .document
                    .as_ref()
                    .and_then(|doc| doc.file_path.as_deref()),
            );
        }
    }

    fn exit(&mut self) -> Option<ExitCode> {
        if self.init.quit_requested {
            return Some(ended_early(self.one_shot, "the app was told to quit").unwrap_or(ExitCode::SUCCESS));
        }
        let shot = self.one_shot.as_ref()?;
        let code = capture_flow::one_shot(self.init, shot)?;
        *self.one_shot = None;
        Some(code)
    }
}

/// The loop is ending, for `why`, before the one-shot capture it owes —
/// a quit, a closed window — so the launch's promise is broken: say so
/// on stderr, and end with `1`. `None` when no capture is owed, which
/// leaves the code to whatever ended the loop.
pub(super) fn ended_early(one_shot: &mut Option<OneShot>, why: &str) -> Option<ExitCode> {
    let shot = one_shot.take()?;
    log::error!(
        "screenshot: {why} before the map was captured; nothing was written to {}",
        shot.path.display()
    );
    Some(ExitCode::FAILURE)
}

/// Execute queued IPC requests on the main thread, with the same access
/// an input handler has: each command gets an [`IpcCommandTarget`]
/// over the same state a winit handler is given. `init` is `None`
/// before winit's window exists, and a command that needs the app then
/// answers `no_document`. A free function over the one field, so a
/// driver can call it while it holds `InitState`.
pub(super) fn drain_ipc(server: &mut Option<IpcServer>, init: Option<&mut InitState>) -> DrainOutcome {
    let Some(server) = server else {
        return DrainOutcome::default();
    };
    match init {
        Some(init) => {
            // Look at what the user just did before any command runs,
            // so a change of theirs is reported as theirs and not
            // folded into the first command's. Only when subscribed:
            // with nothing to report, an input handler's changes cost
            // no observation at all.
            if server.is_subscribed() {
                server.observe(&init.observed());
            }
            server.drain(|request, session| {
                let mut target = IpcCommandTarget::new(init);
                ipc::execute(request, Some(&mut target), session)
            })
        }
        None => server.drain(|request, session| ipc::execute(request, None, session)),
    }
}

/// Evaluate the controller's pending `clock.wait`, if any, against the
/// app as it stands (`format/ipc.md` §`clock.wait`). Called at the end
/// of every iteration, after its redraw has been asked for — so a
/// pixels-affecting command of that iteration counts as a frame still
/// owed — and after every frame drawn, which is when an owed frame
/// stops being owed.
pub(super) fn evaluate_wait(server: &mut Option<IpcServer>, init: &InitState) {
    if let Some(server) = server {
        server.evaluate_wait(init.settling(), &init.observed());
    }
}

/// A graceful exit, in either driver: the client hears `shutdown`, the
/// socket file is removed, a recording in progress is finished — its
/// frames and manifest on disk, or, in the artifacts dir, gone with it
/// — and the artifacts dir is removed with every server-chosen capture
/// in it. `init` is `None` when winit exits before its window existed.
pub(super) fn shut_down(server: &mut Option<IpcServer>, init: Option<&mut InitState>) {
    if let Some(server) = server.take() {
        server.shutdown();
    }
    if let Some(init) = init {
        super::record_flow::shutdown(
            &mut init.recorder,
            &mut init.artifacts,
            &init.watchdog,
            &init.clock,
        );
        init.artifacts.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    /// One step of an iteration, as the fake saw it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Step {
        Continuation,
        DrainFrame,
        DrainIpc,
        RecordTick,
        RecordIdle,
        Redraw,
        FpsIdle { continuing: bool },
        EvaluateWait,
        PublishMap,
        Exit,
    }

    /// An app that records the order [`iterate`] drives it in, and
    /// answers each question with what the test scripted. The log is a
    /// `RefCell` so the continuation reads, which take `&self`, are
    /// recorded where they happen too.
    #[derive(Default)]
    struct Recorded {
        log: RefCell<Vec<Step>>,
        /// The continuation predicate's answers, in order; `false` once
        /// they run out.
        continuing: Vec<bool>,
        ipc: DrainOutcome,
        fps_deadline: Option<Instant>,
        deadline: Option<Instant>,
        exit: Option<ExitCode>,
    }

    impl Recorded {
        fn note(&self, step: Step) {
            self.log.borrow_mut().push(step);
        }

        fn steps(&self) -> Vec<Step> {
            self.log.borrow().clone()
        }

        fn took(&self, step: Step) -> bool {
            self.log.borrow().contains(&step)
        }

        fn position(&self, step: Step) -> Option<usize> {
            self.log.borrow().iter().position(|s| *s == step)
        }
    }

    impl Iteration for Recorded {
        fn needs_continuation(&self) -> bool {
            let asked = self
                .log
                .borrow()
                .iter()
                .filter(|s| **s == Step::Continuation)
                .count();
            self.note(Step::Continuation);
            self.continuing.get(asked).copied().unwrap_or(false)
        }
        fn drain_frame(&mut self) {
            self.note(Step::DrainFrame);
        }
        fn drain_ipc(&mut self) -> DrainOutcome {
            self.note(Step::DrainIpc);
            std::mem::take(&mut self.ipc)
        }
        fn record_tick(&mut self) {
            self.note(Step::RecordTick);
        }
        fn record_idle(&mut self) {
            self.note(Step::RecordIdle);
        }
        fn request_redraw(&mut self) {
            self.note(Step::Redraw);
        }
        fn fps_idle(&mut self, continuing: bool) -> Option<Instant> {
            self.note(Step::FpsIdle { continuing });
            self.fps_deadline
        }
        fn deadline(&self) -> Option<Instant> {
            self.deadline
        }
        fn evaluate_wait(&mut self) {
            self.note(Step::EvaluateWait);
        }
        fn publish_map(&mut self) {
            self.note(Step::PublishMap);
        }
        fn exit(&mut self) -> Option<ExitCode> {
            self.note(Step::Exit);
            self.exit
        }
    }

    /// **The heartbeat order, pinned.** An idle iteration runs the
    /// frame's drains, then the IPC drain, then the recorder, then —
    /// with nothing owed — spends the sleep encoding, flips the FPS
    /// overlay, evaluates the wait, publishes the map and checks for an
    /// exit, in that order, asks for no frame, and parks.
    #[test]
    fn test_an_idle_iteration_runs_the_drains_in_order_and_parks() {
        let mut app = Recorded::default();
        assert_eq!(iterate(&mut app), Next::Idle);
        assert_eq!(
            app.steps(),
            vec![
                Step::Continuation,
                Step::DrainFrame,
                Step::DrainIpc,
                Step::RecordTick,
                Step::Continuation,
                Step::RecordIdle,
                Step::FpsIdle { continuing: false },
                Step::EvaluateWait,
                Step::PublishMap,
                Step::Exit,
            ]
        );
    }

    /// A frame is asked for exactly when work happened, is pending, or a
    /// command changed pixels — and before the wait is evaluated, so the
    /// frame counts as owed — and the recorder's idle encode runs only
    /// when none of that is so.
    #[test]
    fn test_a_frame_is_owed_by_work_done_work_pending_or_a_repaint() {
        let cases = [
            // (continuing on entry, after, repaint, redraw, idle encode)
            (false, false, false, false, true),
            (true, false, false, true, false),
            (false, true, false, true, false),
            (false, false, true, true, false),
            (true, true, true, true, false),
        ];
        for (entry, after, repaint, redraw, idle) in cases {
            let mut app = Recorded {
                continuing: vec![entry, after],
                ipc: DrainOutcome {
                    repaint,
                    backlog: false,
                },
                ..Recorded::default()
            };
            iterate(&mut app);
            let case = format!("entry {entry}, after {after}, repaint {repaint}");
            assert_eq!(app.took(Step::Redraw), redraw, "{case}");
            assert_eq!(app.took(Step::RecordIdle), idle, "{case}");
            if redraw {
                assert!(
                    app.position(Step::Redraw) < app.position(Step::EvaluateWait),
                    "{case}: the frame is owed first"
                );
            }
            assert!(
                app.took(Step::FpsIdle { continuing: after }),
                "{case}: the FPS flip hears whether work continues"
            );
        }
    }

    /// When the next iteration runs: at once while work continues or the
    /// IPC queue is backed up, at the earliest deadline otherwise, else
    /// at the next event — and never, once the loop is to end (a quit, a
    /// one-shot's capture), whatever else is pending.
    #[test]
    fn test_the_next_iteration_is_scheduled_by_what_is_pending() {
        let soon = Instant::now() + Duration::from_millis(5);
        let later = soon + Duration::from_millis(500);

        let mut continuing = Recorded {
            continuing: vec![false, true],
            deadline: Some(soon),
            ..Recorded::default()
        };
        assert_eq!(iterate(&mut continuing), Next::Now);

        let mut backlog = Recorded {
            ipc: DrainOutcome {
                repaint: false,
                backlog: true,
            },
            ..Recorded::default()
        };
        assert_eq!(iterate(&mut backlog), Next::Now);
        assert!(!backlog.took(Step::RecordIdle), "a backlog is not idle");

        for (fps_deadline, deadline) in [
            (Some(later), Some(soon)),
            (Some(soon), Some(later)),
            (None, Some(soon)),
        ] {
            let mut app = Recorded {
                fps_deadline,
                deadline,
                ..Recorded::default()
            };
            assert_eq!(iterate(&mut app), Next::Until(soon), "the earlier of the two");
        }

        let mut done = Recorded {
            continuing: vec![true, true],
            ipc: DrainOutcome {
                repaint: true,
                backlog: true,
            },
            exit: Some(ExitCode::SUCCESS),
            ..Recorded::default()
        };
        assert_eq!(iterate(&mut done), Next::Exit(ExitCode::SUCCESS));
        assert_eq!(
            done.steps().last(),
            Some(&Step::Exit),
            "the exit check is the last word"
        );
    }

    /// A loop that ends with the one-shot still owed breaks the launch's
    /// promise and says so with `1`, once; with nothing owed it leaves
    /// the code to whatever ended the loop.
    #[test]
    fn test_ending_before_the_one_shot_capture_is_a_failure() {
        let mut owed = Some(OneShot {
            path: "/tmp/never-written.png".into(),
            overwrite: false,
        });
        assert_eq!(ended_early(&mut owed, "a test ended it"), Some(ExitCode::FAILURE));
        assert_eq!(owed, None, "taken, so a second end does not report it again");
        assert_eq!(ended_early(&mut owed, "again"), None);
    }
}
