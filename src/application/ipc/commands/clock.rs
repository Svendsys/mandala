// SPDX-License-Identifier: MPL-2.0

//! The `clock` family: determinism (`format/ipc.md` §`clock`).
//!
//! **A thin surface over two engines.** The clock itself — its two
//! modes and the one continuous axis they share — is
//! `crate::application::clock`, and what a step runs is the heartbeat
//! (`app::heartbeat`), the same frame the event loop runs. The `clock`
//! console verb reaches both through the same doors, so the two
//! surfaces cannot drift apart. These handlers check the request's
//! shape, open the door, and word what came back.
//!
//! **`clock.wait` answers later.** It is the protocol's deferred reply:
//! the handler records the wait in the controller's session and the
//! transport sends the reply when the event loop finds it due
//! (`ipc::wait`). Everything else here replies at once.

use std::time::Instant;

use serde_json::{json, Value};

use crate::application::clock::{AppClock, ClockMode, ClockRefusal, StepRequest};
use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::wait::{PendingWait, Until, DEFAULT_TIMEOUT_MS, TIMEOUT_RANGE};

const MODE: ResultSpec = ResultSpec::new(
    "mode",
    ParamType::String,
    "\"real\" (the wall clock's rate) or \"virtual\" (advances only by clock.step).",
);
const NOW_MS: ResultSpec = ResultSpec::new(
    "now_ms",
    ParamType::Number,
    "The app clock, in milliseconds: the time every animation, click and recording reads.",
);

pub(super) const FAMILY: Family = Family {
    name: "clock",
    commands: &[
        CommandSpec {
            name: "clock.status",
            summary: "Read the app clock: its mode and its reading.",
            params: &[],
            result: &[MODE, NOW_MS],
            run: status,
        },
        CommandSpec {
            name: "clock.set_mode",
            summary: "Run the app clock in real time or in virtual time, which advances only by clock.step. The \
                      reading never jumps.",
            params: &[ParamSpec::new(
                "mode",
                ParamType::String,
                true,
                "\"real\" or \"virtual\". Refused while a recording runs.",
            )],
            result: &[MODE, NOW_MS],
            run: set_mode,
        },
        CommandSpec {
            name: "clock.step",
            summary: "Advance the virtual clock and run frames until caught up.",
            params: &[
                ParamSpec::new(
                    "ms",
                    ParamType::Integer,
                    false,
                    "Virtual milliseconds to advance, 1..=1600000. Exactly one of ms / frames.",
                ),
                ParamSpec::new(
                    "frames",
                    ParamType::Integer,
                    false,
                    "Exact heartbeat count to run, 16 virtual ms apart, 1..=100000. Exactly one of ms / frames.",
                ),
            ],
            result: &[
                ResultSpec::new("now_ms", ParamType::Number, "Virtual clock after the step."),
                ResultSpec::new(
                    "frames_run",
                    ParamType::Integer,
                    "Heartbeats executed: frames, or ceil(ms / 16).",
                ),
            ],
            run: step,
        },
        CommandSpec {
            name: "clock.wait",
            summary: "Reply once the app is settled, or its animations complete; other commands keep running \
                      meanwhile.",
            params: &[
                ParamSpec::new(
                    "until",
                    ParamType::String,
                    true,
                    "\"settled\" (the rendered scene is the final consequence of every command sent so far) or \
                     \"animations_complete\".",
                ),
                ParamSpec::new(
                    "timeout_ms",
                    ParamType::Integer,
                    false,
                    "Wall-clock milliseconds before the reply is `timeout`, 1..=60000 (default 10000), under \
                     either clock mode.",
                ),
            ],
            result: &[ResultSpec::new(
                "elapsed_ms",
                ParamType::Integer,
                "Wall-clock milliseconds the wait took.",
            )],
            run: wait,
        },
    ],
};

/// A clock's mode and reading, as `clock.status` and `clock.set_mode`
/// report them.
fn status_json(clock: &AppClock) -> Value {
    json!({ "mode": clock.mode().word(), "now_ms": clock.now_ms() })
}

/// `clock.status`. Before the window exists the app clock is the
/// wall clock, real and unshifted — what a new clock reads — so the
/// answer needs no app.
fn status(call: &mut Call<'_>) -> Reply {
    let result = match call.observe() {
        Some(app) => status_json(app.clock()),
        None => status_json(&AppClock::default()),
    };
    call.ok(result)
}

/// The mode words, for a refusal that lists them.
fn mode_words() -> String {
    ClockMode::ALL
        .iter()
        .map(|mode| format!("\"{}\"", mode.word()))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// The error reply for a clock surface's `refusal`.
fn refuse(call: &Call<'_>, refusal: ClockRefusal) -> Reply {
    match refusal {
        // Neither is a parameter's fault: the state of the app is, and
        // `cmd` names the command that met it, as a recorder state
        // refusal does.
        ClockRefusal::Real => call.invalid_params("cmd", refusal.message()),
        ClockRefusal::Recording => call.invalid_params("mode", refusal.message()),
        ClockRefusal::Stepping => call.error(code::BUSY, refusal.message(), None),
    }
}

/// `clock.set_mode`.
fn set_mode(call: &mut Call<'_>) -> Reply {
    let word = call.str_param("mode").unwrap_or_default();
    let Some(mode) = ClockMode::parse(word) else {
        return call.invalid_params("mode", format!("`mode` must be {}, got \"{word}\"", mode_words()));
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.set_clock_mode(mode) {
        Ok(_) => {
            let result = status_json(app.clock());
            call.ok(result)
        }
        Err(refusal) => refuse(call, refusal),
    }
}

/// An integer parameter as a count: `Some(None)` when absent, `None`
/// for a negative one (which no count is).
fn count(call: &Call<'_>, name: &str) -> Option<Option<u64>> {
    match call.param(name) {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
    }
}

/// `clock.step`.
fn step(call: &mut Call<'_>) -> Reply {
    let (Some(ms), Some(frames)) = (count(call, "ms"), count(call, "frames")) else {
        let param = if count(call, "ms").is_none() {
            "ms"
        } else {
            "frames"
        };
        return call.invalid_params(param, format!("`{param}` must be a positive integer"));
    };
    let request = match StepRequest::of(ms, frames) {
        Ok(request) => request,
        Err((param, message)) => return call.invalid_params(param, message),
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.step_clock(request) {
        Ok(stepped) => call.ok(json!({ "now_ms": stepped.now_ms, "frames_run": stepped.frames_run })),
        Err(refusal) => refuse(call, refusal),
    }
}

/// `clock.wait`: the shape is checked here, and the wait itself joins
/// the session to be answered later (see the module doc).
fn wait(call: &mut Call<'_>) -> Reply {
    let word = call.str_param("until").unwrap_or_default();
    let Some(until) = Until::parse(word) else {
        let words: Vec<String> = Until::ALL.iter().map(|u| format!("\"{}\"", u.word())).collect();
        return call.invalid_params(
            "until",
            format!("`until` must be {}, got \"{word}\"", words.join(" or ")),
        );
    };
    let timeout_ms = match count(call, "timeout_ms") {
        Some(None) => DEFAULT_TIMEOUT_MS,
        Some(Some(ms)) if TIMEOUT_RANGE.contains(&ms) => ms,
        _ => {
            return call.invalid_params(
                "timeout_ms",
                format!(
                    "`timeout_ms` must be in {}..={}; a longer wait is several waits",
                    TIMEOUT_RANGE.start(),
                    TIMEOUT_RANGE.end()
                ),
            )
        }
    };
    if call.observe().is_none() {
        return call.no_document();
    }
    if call.session().wait().is_some() {
        return call.error(
            code::BUSY,
            "a clock.wait is already pending on this connection; waits are exclusive — send this \
             one after its reply arrives",
            None,
        );
    }
    let wait = PendingWait::new(call.id().clone(), until, timeout_ms, Instant::now());
    call.defer(wait)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::capture::plan::CaptureRequest;
    use crate::application::capture::record::{RecordDestination, Started, Summary};
    use crate::application::capture::{CaptureError, Destination};
    use crate::application::clock::Stepped;
    use crate::application::console::LineOutcome;
    use crate::application::document::MindMapDocument;
    use crate::application::ipc::envelope::{decode_line, Decoded, Frame, OUTBOUND_FRAME_CAP};
    use crate::application::ipc::events::{Observed, Session};
    use crate::application::ipc::injected::{Overloaded, Pacing, Sample};
    use crate::application::ipc::registry::execute;
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::space::Point;
    use crate::application::ipc::target::CommandTarget;
    use crate::application::ipc::{Injected, SceneView, Screenshot, Settling, ShellState};
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;

    /// An app with a real clock and a scripted answer for a step or a
    /// switch, recording what it was asked.
    struct ClockApp {
        clock: AppClock,
        doc: MindMapDocument,
        camera: baumhard::gfx_structs::camera::Camera2D,
        mode: crate::application::app::InteractionMode,
        recording: bool,
        stepped: Vec<StepRequest>,
        changed: bool,
    }

    impl ClockApp {
        fn new() -> Self {
            ClockApp {
                clock: AppClock::default(),
                doc: crate::application::document::tests_common::load_test_doc(),
                camera: baumhard::gfx_structs::camera::Camera2D::new(800, 600),
                mode: crate::application::app::InteractionMode::Default,
                recording: false,
                stepped: Vec::new(),
                changed: false,
            }
        }
    }

    impl CommandTarget for ClockApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> crate::application::ipc::ActionOutcome {
            panic!("a clock command must not dispatch an Action");
        }
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            panic!("a clock command must not run a console line");
        }
        fn has_macro(&self, _: &str) -> bool {
            panic!("a clock command must not look up a macro");
        }
        fn run_macro(&mut self, _: &str, _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a clock command must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep], _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a clock command must not run macro steps");
        }
        fn observed(&self) -> Option<Observed<'_>> {
            Some(Observed {
                document: Some(&self.doc),
                camera: &self.camera,
                mode: &self.mode,
            })
        }
        fn changed_view(&self) -> bool {
            self.changed
        }
        fn document(&self) -> Option<&MindMapDocument> {
            Some(&self.doc)
        }
        fn animation_pause(&self) -> Option<u64> {
            None
        }
        fn shell_state(&self, _: ShellState) -> Value {
            panic!("a clock command must not read the shell state");
        }
        fn scene(&self) -> Option<SceneView<'_>> {
            panic!("a clock command must not read the scene");
        }
        fn hit_test(&mut self, _: Point) -> Option<Value> {
            panic!("a clock command must not hit-test");
        }
        fn surface_point(&self, _: Point) -> (f64, f64) {
            panic!("a clock command must not place a pointer");
        }
        fn inject(&mut self, _: Vec<Sample>, _: Pacing) -> Result<Injected, Overloaded> {
            panic!("a clock command must not inject input");
        }
        fn injected_pending(&self) -> usize {
            panic!("a clock command must not read the injected-input queue");
        }
        fn capture(
            &mut self,
            _: &CaptureRequest,
            _: Destination,
            _: bool,
        ) -> Option<Result<Screenshot, CaptureError>> {
            panic!("a clock command must not take a screenshot");
        }
        fn record_start(
            &mut self,
            _: &CaptureRequest,
            _: u32,
            _: u32,
            _: RecordDestination,
        ) -> Option<Result<Started, CaptureError>> {
            panic!("a clock command must not start a recording");
        }
        fn record_stop(&mut self) -> Result<Summary, CaptureError> {
            panic!("a clock command must not stop a recording");
        }
        fn clock(&self) -> &AppClock {
            &self.clock
        }
        fn set_clock_mode(&mut self, mode: ClockMode) -> Result<bool, ClockRefusal> {
            if self.recording && self.clock.mode() != mode {
                return Err(ClockRefusal::Recording);
            }
            Ok(self.clock.set_mode(mode))
        }
        fn step_clock(&mut self, request: StepRequest) -> Result<Stepped, ClockRefusal> {
            if !self.clock.is_virtual() {
                return Err(ClockRefusal::Real);
            }
            self.stepped.push(request);
            let to = self.clock.now_ms() + request.duration_ms() as f64;
            self.clock.advance_to(to);
            self.changed = true;
            Ok(Stepped {
                now_ms: self.clock.now_ms(),
                frames_run: request.frames(),
            })
        }
    }

    /// Run `frame` in `session` and return the wire reply, or `None`
    /// for a reply held back.
    fn run_held(frame: &str, app: &mut ClockApp, session: &mut Session) -> Option<Value> {
        let Decoded::Request(request) = decode_line(frame.as_bytes(), 0, 1) else {
            panic!("{frame} must decode to a request");
        };
        let executed = execute(&request, Some(app), session);
        executed.reply.map(|reply| {
            let bytes = Frame::Reply(reply).encode(OUTBOUND_FRAME_CAP).expect("encodes");
            serde_json::from_slice(&bytes).expect("JSON")
        })
    }

    #[test]
    fn test_status_before_the_window_is_the_real_wall_clock() {
        let before = crate::application::clock::wall_ms();
        let (reply, repaint) = run_detached(r#"{"id":1,"cmd":"clock.status"}"#);
        assert!(!repaint);
        assert_eq!(reply["result"]["mode"], json!("real"));
        let now = reply["result"]["now_ms"].as_f64().expect("now_ms");
        assert!(now >= before, "{now} < {before}");
    }

    /// The round trip an agent makes: virtual, step, read, real — the
    /// reading never jumps, and each step lands exactly where it said.
    #[test]
    fn test_set_mode_and_step_drive_one_continuous_clock() {
        let mut app = ClockApp::new();
        let (reply, repaint) = run_attached(r#"{"id":1,"cmd":"clock.set_mode","mode":"virtual"}"#, &mut app);
        assert!(!repaint, "a switch changes no pixels");
        assert_eq!(reply["result"]["mode"], json!("virtual"));
        let engaged = reply["result"]["now_ms"].as_f64().expect("now_ms");
        assert_eq!(engaged.fract(), 0.0, "virtual time starts on a whole millisecond");

        let (reply, repaint) = run_attached(r#"{"id":2,"cmd":"clock.step","ms":500}"#, &mut app);
        assert!(repaint, "a step owes a frame");
        assert_eq!(
            reply["result"],
            json!({ "now_ms": engaged + 500.0, "frames_run": 32 })
        );
        let (reply, _) = run_attached(r#"{"id":3,"cmd":"clock.step","frames":3}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({ "now_ms": engaged + 548.0, "frames_run": 3 })
        );
        assert_eq!(app.stepped, [StepRequest::Ms(500), StepRequest::Frames(3)]);

        let (reply, _) = run_attached(r#"{"id":4,"cmd":"clock.status"}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({ "mode": "virtual", "now_ms": engaged + 548.0 })
        );
        let (reply, _) = run_attached(r#"{"id":5,"cmd":"clock.set_mode","mode":"real"}"#, &mut app);
        assert_eq!(reply["result"]["mode"], json!("real"));
        assert!(reply["result"]["now_ms"].as_f64().expect("now_ms") >= engaged + 548.0);
    }

    #[test]
    fn test_a_step_on_a_real_clock_or_a_switch_mid_recording_is_refused() {
        let mut app = ClockApp::new();
        let (reply, repaint) = run_attached(r#"{"id":1,"cmd":"clock.step","ms":16}"#, &mut app);
        assert!(!repaint);
        assert_eq!(reply["error"]["code"], json!("invalid_params"));
        assert_eq!(reply["error"]["data"]["param"], json!("cmd"));
        assert!(app.stepped.is_empty());

        app.recording = true;
        let (reply, _) = run_attached(r#"{"id":2,"cmd":"clock.set_mode","mode":"virtual"}"#, &mut app);
        assert_eq!(reply["error"]["data"]["param"], json!("mode"));
        assert_eq!(app.clock.mode(), ClockMode::Real);
        let (reply, _) = run_attached(r#"{"id":3,"cmd":"clock.set_mode","mode":"real"}"#, &mut app);
        assert_eq!(
            reply["result"]["mode"],
            json!("real"),
            "naming the mode it is in is no switch"
        );
    }

    /// Every malformed step is `invalid_params` naming its parameter,
    /// and steps nothing.
    #[test]
    fn test_a_malformed_step_names_its_parameter_and_steps_nothing() {
        let mut app = ClockApp::new();
        app.clock.set_mode(ClockMode::Virtual);
        for (frame, param) in [
            (r#"{"id":1,"cmd":"clock.step"}"#, "ms"),
            (r#"{"id":1,"cmd":"clock.step","ms":16,"frames":1}"#, "frames"),
            (r#"{"id":1,"cmd":"clock.step","ms":0}"#, "ms"),
            (r#"{"id":1,"cmd":"clock.step","ms":-16}"#, "ms"),
            (r#"{"id":1,"cmd":"clock.step","ms":1600001}"#, "ms"),
            (r#"{"id":1,"cmd":"clock.step","frames":100001}"#, "frames"),
            (r#"{"id":1,"cmd":"clock.step","frames":-1}"#, "frames"),
            (r#"{"id":1,"cmd":"clock.step","ms":1.5}"#, "ms"),
            (r#"{"id":1,"cmd":"clock.set_mode","mode":"wall"}"#, "mode"),
            (r#"{"id":1,"cmd":"clock.set_mode","mode":"Virtual"}"#, "mode"),
        ] {
            let (reply, repaint) = run_attached(frame, &mut app);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}");
            assert!(!repaint, "{frame}");
        }
        assert!(app.stepped.is_empty());
    }

    #[test]
    fn test_the_clock_commands_that_need_the_app_answer_no_document_before_it() {
        for frame in [
            r#"{"id":1,"cmd":"clock.set_mode","mode":"virtual"}"#,
            r#"{"id":1,"cmd":"clock.step","ms":16}"#,
            r#"{"id":1,"cmd":"clock.wait","until":"settled"}"#,
        ] {
            let (reply, _) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
        }
    }

    /// A wait is held back, exclusive per controller, and answered by
    /// the session when its condition holds — and a disconnect drops
    /// it.
    #[test]
    fn test_a_wait_is_held_back_exclusive_and_resolved_by_the_session() {
        let mut app = ClockApp::new();
        let mut session = Session::default();
        assert_eq!(
            run_held(
                r#"{"id":"w1","cmd":"clock.wait","until":"settled"}"#,
                &mut app,
                &mut session
            ),
            None,
            "the reply is held back"
        );
        let busy = run_held(
            r#"{"id":"w2","cmd":"clock.wait","until":"animations_complete"}"#,
            &mut app,
            &mut session,
        )
        .expect("a second wait answers at once");
        assert_eq!(busy["id"], json!("w2"));
        assert_eq!(busy["error"]["code"], json!("busy"));

        let quiet = Settling {
            animations_active: false,
            continuing: false,
            redraw_owed: false,
            virtual_clock: false,
        };
        let owed = Settling {
            redraw_owed: true,
            ..quiet
        };
        assert!(
            session.resolve_wait(Instant::now(), owed).is_none(),
            "still owed a frame"
        );
        let reply = session.resolve_wait(Instant::now(), quiet).expect("settled");
        let bytes = Frame::Reply(reply).encode(OUTBOUND_FRAME_CAP).expect("encodes");
        let reply: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(reply["id"], json!("w1"));
        assert!(reply["result"]["elapsed_ms"].is_u64());
        assert!(session.wait().is_none(), "a resolved wait is over");

        assert_eq!(
            run_held(
                r#"{"id":"w3","cmd":"clock.wait","until":"settled"}"#,
                &mut app,
                &mut session
            ),
            None
        );
        session.reset_controller();
        assert!(session.wait().is_none(), "a disconnect cancels the wait");
    }

    #[test]
    fn test_a_malformed_wait_names_its_parameter() {
        let mut app = ClockApp::new();
        let mut session = Session::default();
        for (frame, param) in [
            (r#"{"id":1,"cmd":"clock.wait"}"#, "until"),
            (r#"{"id":1,"cmd":"clock.wait","until":"idle"}"#, "until"),
            (
                r#"{"id":1,"cmd":"clock.wait","until":"settled","timeout_ms":0}"#,
                "timeout_ms",
            ),
            (
                r#"{"id":1,"cmd":"clock.wait","until":"settled","timeout_ms":60001}"#,
                "timeout_ms",
            ),
            (
                r#"{"id":1,"cmd":"clock.wait","until":"settled","timeout_ms":-5}"#,
                "timeout_ms",
            ),
        ] {
            let reply = run_held(frame, &mut app, &mut session).expect("answered at once");
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}");
        }
        assert!(session.wait().is_none(), "a refused wait is not held");
    }
}
