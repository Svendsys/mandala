// SPDX-License-Identifier: MPL-2.0

//! `clock.wait`: the one deferred reply (`format/ipc.md` §`clock.wait`,
//! `work_plans/LLM_IPC.md` §D7).
//!
//! **A wait is a reply held back, not a loop.** The command records a
//! [`PendingWait`] in the controller's session and answers nothing;
//! the event loop evaluates it at the end of every iteration and after
//! every window event (the frame's render among them), and the
//! transport sends its reply the moment its condition holds or its
//! deadline passes. Nothing blocks: other commands keep executing
//! while it is pending, which is why replies are correlated by `id`
//! and never by order.
//!
//! **One per controller, gone with it.** A second wait while one is
//! pending is refused `busy` — there is no named use for two, and an
//! unbounded set would be a memory and scan cost a client could run
//! up. The wait lives in the controller's session, so a disconnect,
//! which resets the session, cancels it.
//!
//! **The deadline is wall-clock.** It guards the client's liveness,
//! not simulation time: a frozen virtual clock must not disable it.

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::envelope::{code, Reply};

/// How long a wait waits when the client names no `timeout_ms`.
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// The `timeout_ms` a wait accepts.
pub(crate) const TIMEOUT_RANGE: RangeInclusive<u64> = 1..=60_000;

/// What a wait waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Until {
    /// The rendered scene is the final consequence of every command so
    /// far: nothing continuing, nothing owed to the window.
    Settled,
    /// The document has no active animation.
    AnimationsComplete,
}

impl Until {
    /// Every condition, in the order the wire lists them.
    pub(crate) const ALL: [Until; 2] = [Until::Settled, Until::AnimationsComplete];

    pub(crate) const fn word(self) -> &'static str {
        match self {
            Until::Settled => "settled",
            Until::AnimationsComplete => "animations_complete",
        }
    }

    pub(crate) fn parse(word: &str) -> Option<Until> {
        Until::ALL.into_iter().find(|until| until.word() == word)
    }
}

/// What a wait reads off the app at an evaluation point. The app side
/// computes it (`InitState::settling`), because each part is app state
/// the `ipc` modules do not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settling {
    /// The document has an active animation.
    pub(crate) animations_active: bool,
    /// The loop needs another iteration with no further input: a
    /// throttled drag or picker hover pending, camera geometry to
    /// rebuild — and, in real time, an animation running or injected
    /// input queued. Under a virtual clock those last two are owed to
    /// future virtual instants, which only `clock.step` reaches, so
    /// they are nothing the app still owes *now*.
    pub(crate) continuing: bool,
    /// A redraw has been asked for that no render has drawn yet.
    pub(crate) redraw_owed: bool,
    /// The clock is virtual, so time moves only by `clock.step` — what
    /// a timeout says when the condition waits on time.
    pub(crate) virtual_clock: bool,
}

impl Settling {
    /// Whether `until` holds.
    pub(crate) fn holds(self, until: Until) -> bool {
        match until {
            Until::Settled => !self.continuing && !self.redraw_owed,
            Until::AnimationsComplete => !self.animations_active,
        }
    }

    /// What still keeps `until` from holding, for a timeout's message.
    fn owed(self, until: Until) -> &'static str {
        match until {
            // Under a virtual clock an animation is frozen between
            // steps, and no wait outlasts that: the remedy is a step.
            Until::AnimationsComplete if self.virtual_clock => {
                "an animation is still running, and the clock is virtual: it advances only by \
                 clock.step, so step it to the animation's end"
            }
            Until::AnimationsComplete => "an animation is still running",
            // Animations and queued input are owed to future virtual
            // instants, so they never keep a virtual clock unsettled.
            Until::Settled if self.continuing && self.virtual_clock => {
                "the app still had frames to run (a pending drag or picker hover, or a camera \
                 rebuild)"
            }
            Until::Settled if self.continuing => {
                "the app still had frames to run (an animation, a pending drag or picker hover, \
                 queued input or a camera rebuild)"
            }
            Until::Settled => "the renderer had not drawn the latest frame",
        }
    }
}

/// A `clock.wait` whose reply is held back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingWait {
    /// The request's `id`, which its reply carries.
    id: Value,
    until: Until,
    /// When it was received, on the wall clock.
    since: Instant,
    deadline: Instant,
}

impl PendingWait {
    /// A wait for `until` received at `since`, giving up after
    /// `timeout_ms` (already within [`TIMEOUT_RANGE`]).
    pub(crate) fn new(id: Value, until: Until, timeout_ms: u64, since: Instant) -> Self {
        PendingWait {
            id,
            until,
            since,
            deadline: since + Duration::from_millis(timeout_ms),
        }
    }

    /// When the loop must wake to time this wait out.
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The reply this wait is owed at `now`, given what the app reads:
    /// `Some` when the condition holds — checked first, so a condition
    /// that holds at the deadline still succeeds — or the deadline has
    /// passed, `None` while it should keep waiting.
    pub(crate) fn resolve(&self, now: Instant, app: Settling) -> Option<Reply> {
        let elapsed_ms = now.saturating_duration_since(self.since).as_millis() as u64;
        if app.holds(self.until) {
            return Some(Reply::ok(self.id.clone(), json!({ "elapsed_ms": elapsed_ms })));
        }
        if now < self.deadline {
            return None;
        }
        Some(Reply::error(
            self.id.clone(),
            code::TIMEOUT,
            format!(
                "`{}` did not hold within {elapsed_ms} ms: {}",
                self.until.word(),
                app.owed(self.until)
            ),
            Some(json!({ "elapsed_ms": elapsed_ms })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::envelope::{Frame, OUTBOUND_FRAME_CAP};

    fn wire(reply: Reply) -> Value {
        let bytes = Frame::Reply(reply).encode(OUTBOUND_FRAME_CAP).expect("encodes");
        serde_json::from_slice(&bytes).expect("JSON")
    }

    const QUIET: Settling = Settling {
        animations_active: false,
        continuing: false,
        redraw_owed: false,
        virtual_clock: false,
    };

    #[test]
    fn test_until_words_round_trip_case_exact() {
        for until in Until::ALL {
            assert_eq!(Until::parse(until.word()), Some(until));
        }
        assert_eq!(Until::parse("Settled"), None);
        assert_eq!(
            Until::parse("idle"),
            None,
            "there is deliberately no idle condition"
        );
    }

    /// `settled` needs both halves: nothing continuing and nothing owed
    /// to the window. `animations_complete` reads the animations alone.
    #[test]
    fn test_settled_needs_nothing_continuing_and_nothing_undrawn() {
        assert!(QUIET.holds(Until::Settled));
        for app in [
            Settling {
                continuing: true,
                ..QUIET
            },
            Settling {
                redraw_owed: true,
                ..QUIET
            },
        ] {
            assert!(!app.holds(Until::Settled), "{app:?}");
            assert!(app.holds(Until::AnimationsComplete), "{app:?}");
        }
        let animating = Settling {
            animations_active: true,
            ..QUIET
        };
        assert!(!animating.holds(Until::AnimationsComplete));
        assert!(
            animating.holds(Until::Settled),
            "under a virtual clock an animation frozen between steps is settled"
        );
    }

    #[test]
    fn test_a_wait_keeps_waiting_until_its_condition_holds() {
        let since = Instant::now();
        let wait = PendingWait::new(json!(9), Until::Settled, 1_000, since);
        let busy = Settling {
            continuing: true,
            ..QUIET
        };
        assert!(wait.resolve(since + Duration::from_millis(999), busy).is_none());
        let reply = wire(
            wait.resolve(since + Duration::from_millis(40), QUIET)
                .expect("resolves"),
        );
        assert_eq!(reply["id"], json!(9));
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["result"], json!({ "elapsed_ms": 40 }));
    }

    /// At the deadline a wait answers `timeout` with `data.elapsed_ms`
    /// and says what was still owed; a condition that holds at the
    /// deadline still succeeds.
    #[test]
    fn test_a_wait_past_its_deadline_times_out_naming_what_was_owed() {
        let since = Instant::now();
        let wait = PendingWait::new(json!("w"), Until::Settled, 250, since);
        let at = since + Duration::from_millis(250);
        let owed = Settling {
            redraw_owed: true,
            ..QUIET
        };
        let reply = wire(wait.resolve(at, owed).expect("times out"));
        assert_eq!(reply["error"]["code"], json!("timeout"));
        assert_eq!(reply["error"]["data"], json!({ "elapsed_ms": 250 }));
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(message.contains("not drawn"), "{message}");
        assert_eq!(wire(wait.resolve(at, QUIET).expect("holds"))["ok"], json!(true));
        assert_eq!(wait.deadline(), at);
    }

    /// Under a virtual clock an animation never finishes on its own, so
    /// a wait for it that runs out names the clock and the remedy, and
    /// a `settled` one does not blame animations or queued input, which
    /// virtual time does not owe.
    #[test]
    fn test_a_timeout_under_a_virtual_clock_says_so() {
        let since = Instant::now();
        let at = since + Duration::from_millis(10);
        let message = |until, app| {
            let reply = wire(
                PendingWait::new(json!(1), until, 10, since)
                    .resolve(at, app)
                    .expect("times out"),
            );
            reply["error"]["message"].as_str().expect("message").to_string()
        };
        let frozen = Settling {
            animations_active: true,
            virtual_clock: true,
            ..QUIET
        };
        let frozen_message = message(Until::AnimationsComplete, frozen);
        assert!(frozen_message.contains("clock is virtual") && frozen_message.contains("clock.step"));
        let real = Settling {
            virtual_clock: false,
            ..frozen
        };
        assert!(!message(Until::AnimationsComplete, real).contains("virtual"));
        let dragging = Settling {
            continuing: true,
            virtual_clock: true,
            ..QUIET
        };
        let dragging_message = message(Until::Settled, dragging);
        assert!(
            !dragging_message.contains("animation") && !dragging_message.contains("queued input"),
            "{dragging_message}"
        );
    }
}
