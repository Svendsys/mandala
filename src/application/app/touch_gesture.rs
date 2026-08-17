// SPDX-License-Identifier: MPL-2.0

//! `TouchGestureRecognizer` — touch input parity for mouse
//! gestures (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.6 / Batch 7).
//!
//! Today's input pipeline is mouse-first: native and WASM both
//! consume `WindowEvent::MouseInput` / `CursorMoved` / `MouseWheel`
//! and route them through the dispatch funnel via the
//! [`crate::application::keybinds::MouseGesture`] table. Touch
//! events on a phone or tablet arrive via `WindowEvent::Touch`
//! (winit) and were dropped silently — the existing comment in
//! `run_wasm/mod.rs:454-461` flagged the gap.
//!
//! This module implements the recogniser the plan calls for: a
//! pure state machine fed raw `(phase, finger_id, position, now)`
//! tuples from the runtime that emits a typed
//! [`RecognizedGesture`] when one of the supported touch
//! gestures fires. The runtime translates each emission into
//! the same `MouseGesture::*` dispatch the mouse path uses, so
//! the keybind table doesn't grow a parallel touch surface —
//! a `LongPress` and a synthetic right-click run the same
//! `Action`.
//!
//! ## State machine
//!
//! ```text
//!                 finger_started(id_a, pos_a)
//!     Idle ─────────────────────────────────► OneFinger { id_a, started_at, started_pos }
//!       ▲                                                │ │
//!       │ finger_ended (no recognition)                  │ │ finger_started(id_b, pos_b)
//!       │ ─────────────────────────────────              │ │ (with id_a still active)
//!       │                                                │ ▼
//!       │ tick after long_press_ms                       │ TwoFingers { ... }
//!       │ with no movement                               │   │
//!       │ ── emit LongPress ─►                           │   │ finger_moved on either
//!       │                                                │   │ ── emit TwoFingerDrag ─►
//!       │ finger_ended on either of the two              │   │
//!       │ ◄──────────────────────────────────────────────┘   │
//!       │                                                    │
//!       └────────────────────────────────────────────────────┘
//! ```
//!
//! `Idle ↔ OneFinger ↔ TwoFingers` with two emit points: the
//! long-press timer (fires once per OneFinger episode at
//! `started_at + LONG_PRESS_MS` if no movement past
//! `POINTER_DRAG_THRESHOLD_PX`); the two-finger-drag movement
//! check (fires every frame while in TwoFingers and the centroid
//! has moved past `POINTER_DRAG_THRESHOLD_PX` since the last
//! emission).
//!
//! ## Why a typed-emission API rather than synthetic mouse events
//!
//! The plan's §6.6 sketch reads "emits `MouseGesture::*` synthetic
//! events into the existing mouse-input pipeline". A literal
//! reading would have the recogniser construct
//! `(ElementState, MouseButton)` tuples and call
//! `event_mouse_click::handle_mouse_input` directly. The downside:
//! the dispatch funnel sees `MouseButton::Left`, not
//! `MouseGesture::LongPress`, so a long-press would dispatch the
//! `LeftClick` binding — wrong.
//!
//! Instead the recogniser emits a [`RecognizedGesture`] with the
//! resolved [`crate::application::keybinds::MouseGesture`]
//! variant + cursor pos. The runtime then runs the same lookup
//! path the click handlers do — `key_name() →
//! action_for_gesture` → `dispatch_action`. The recognition is
//! the only new step; the dispatch chain is unchanged.

use super::POINTER_DRAG_THRESHOLD_SQ_PX;
use crate::application::keybinds::MouseGesture;
use std::time::Duration;
use web_time::Instant;

/// Long-press fires after this much time with no significant
/// movement. 350ms is the convention from iOS' UILongPressGesture
/// recogniser default and from Android's `ViewConfiguration`.
/// Shorter than this and accidental holds during scrolling fire;
/// longer than this and the gesture feels sluggish.
pub const LONG_PRESS_MS: u64 = 350;

/// What the recogniser's `ingest` / `tick` returns when a
/// gesture is identified. Cursor pos comes back in whatever space
/// it went in — physical pixels, the same space
/// `WindowEvent::CursorMoved` reports and the same space
/// `ctx.cursor_pos` holds — so the runtime can move
/// `ctx.cursor_pos` to this point before dispatching the bound
/// Action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecognizedGesture {
    /// One finger held in place for [`LONG_PRESS_MS`]ms with
    /// movement under
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX) px.
    /// The runtime
    /// dispatches the binding for [`MouseGesture::LongPress`]
    /// at `pos`.
    LongPress { pos: (f64, f64) },
    /// Two fingers down with the centroid travelling past
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX)
    /// since the last emission. Emitted
    /// repeatedly while the user is moving the two fingers
    /// (one emission per "drag step"). The runtime updates
    /// `ctx.cursor_pos` to the centroid before dispatching the
    /// binding for [`MouseGesture::TwoFingerDrag`].
    TwoFingerDrag { pos: (f64, f64) },
}

impl RecognizedGesture {
    /// The [`MouseGesture`] variant whose binding the runtime
    /// should dispatch. Lets the runtime call
    /// `keybinds.action_for_gesture(g.mouse_gesture().key_name(),
    /// ...)` without re-pattern-matching.
    pub fn mouse_gesture(self) -> MouseGesture {
        match self {
            RecognizedGesture::LongPress { .. } => MouseGesture::LongPress,
            RecognizedGesture::TwoFingerDrag { .. } => MouseGesture::TwoFingerDrag,
        }
    }

    /// The cursor position the runtime should move
    /// `ctx.cursor_pos` to before dispatching. For `LongPress`
    /// this is the finger's resting position; for
    /// `TwoFingerDrag` this is the centroid of the two fingers.
    pub fn pos(self) -> (f64, f64) {
        match self {
            RecognizedGesture::LongPress { pos } => pos,
            RecognizedGesture::TwoFingerDrag { pos } => pos,
        }
    }
}

/// What the runtime feeds the recogniser. winit's
/// `event::TouchPhase` doesn't impl `Hash`/`Copy` reliably across
/// versions, and we don't want the recogniser to depend on a
/// specific winit version, so the runtime translates phases into
/// this stable enum at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Finger just landed. Equivalent to `TouchPhase::Started`.
    Started,
    /// Finger is moving. Equivalent to `TouchPhase::Moved`.
    Moved,
    /// Finger lifted. Equivalent to `TouchPhase::Ended` or
    /// `TouchPhase::Cancelled` (both clear the slot from the
    /// recogniser's perspective; the difference is invisible to
    /// the gesture-recognition logic).
    Ended,
}

/// One tracked finger. Held by both `OneFinger` and `TwoFingers`
/// state branches.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FingerTrack {
    id: u64,
    started_at: Instant,
    started_pos: (f64, f64),
    current_pos: (f64, f64),
    /// Set true on the first `Moved` event whose distance from
    /// `started_pos` exceeds
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX).
    /// Cancels the
    /// long-press timer; doesn't otherwise affect TwoFingers
    /// behaviour. Sticky — once true stays true for the
    /// finger's lifetime.
    has_moved: bool,
}

impl FingerTrack {
    fn new(id: u64, pos: (f64, f64), now: Instant) -> Self {
        Self {
            id,
            started_at: now,
            started_pos: pos,
            current_pos: pos,
            has_moved: false,
        }
    }

    /// `move_threshold_sq` is the *squared* travel budget — the
    /// caller's, not the module constant's, so the recognizer's
    /// test constructor reaches this latch as well as the centroid
    /// check. It used to read the constant directly, which left
    /// `TouchGestureRecognizer::with_thresholds` half-honored:
    /// a test could loosen the two-finger step and not the
    /// long-press cancel.
    fn update_pos(&mut self, pos: (f64, f64), move_threshold_sq: f64) {
        self.current_pos = pos;
        let (dx, dy) = (pos.0 - self.started_pos.0, pos.1 - self.started_pos.1);
        if dx * dx + dy * dy > move_threshold_sq {
            self.has_moved = true;
        }
    }
}

/// Internal state. Tested through the public `ingest` / `tick`
/// API rather than directly — the variants are
/// implementation-detail.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    /// Single finger tracked; long-press candidate.
    /// `long_press_emitted` is the latch that prevents a single
    /// hold from firing repeated `LongPress` emissions on
    /// subsequent `tick` calls.
    OneFinger {
        track: FingerTrack,
        long_press_emitted: bool,
    },
    /// Two fingers tracked; two-finger-drag candidate. Emits one
    /// `TwoFingerDrag` per "drag step" (centroid moves more than
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX)
    /// from `last_emit_centroid`). The
    /// initial centroid at second-finger-down is recorded as
    /// `last_emit_centroid` so the first emission requires
    /// actual movement (not just second-finger landing).
    TwoFingers {
        primary: FingerTrack,
        secondary: FingerTrack,
        last_emit_centroid: (f64, f64),
    },
}

/// Touch gesture recogniser. One per app instance. Fed by the
/// runtime's `WindowEvent::Touch` handler; consulted at frame
/// boundaries via [`Self::tick`] for time-based gestures
/// (long-press) that don't fire on the touch event itself.
#[derive(Debug, Clone)]
pub struct TouchGestureRecognizer {
    state: State,
    /// Long-press threshold, configurable for tests. Default
    /// [`LONG_PRESS_MS`].
    long_press: Duration,
    /// Squared movement threshold for both long-press cancellation
    /// and two-finger-drag emission. Configurable for tests.
    /// Squared so neither emit point pays a `sqrt` per motion
    /// event, matching the mouse arms in `event_cursor_moved.rs`.
    move_threshold_sq: f64,
}

impl Default for TouchGestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchGestureRecognizer {
    /// New recogniser at production thresholds.
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            long_press: Duration::from_millis(LONG_PRESS_MS),
            move_threshold_sq: POINTER_DRAG_THRESHOLD_SQ_PX,
        }
    }

    /// Test-only constructor. Lets the state-machine tests pin
    /// timing without sleeping for 350ms per case.
    /// `move_threshold_px` is linear (a test says "four pixels",
    /// not "sixteen"); it is squared here, once, at construction.
    #[cfg(test)]
    pub(crate) fn with_thresholds(long_press: Duration, move_threshold_px: f64) -> Self {
        Self {
            state: State::Idle,
            long_press,
            move_threshold_sq: move_threshold_px * move_threshold_px,
        }
    }

    /// Drive a touch event through the state machine.
    /// `id` is the finger id (winit's `Touch.id`); `pos` is in
    /// **physical** pixels — `dispatch_touch_event` passes
    /// `touch.location`, a `PhysicalPosition<f64>`, straight
    /// through. (This used to claim logical, "the runtime converts
    /// via the scale factor before calling here"; no such
    /// conversion exists, on either target. See
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX)
    /// for what that costs and what closing it would take.)
    /// Returns `Some(gesture)` when the event triggered a
    /// recognition (e.g. a `Moved` on the second finger that
    /// crosses the centroid-movement threshold). The runtime
    /// is responsible for dispatching that gesture; tests
    /// frequently discard the return when they're staging the
    /// state machine for a later assertion (no `#[must_use]`
    /// for that reason — the caller layer is the right place
    /// for the lint, and the runtime is a single 5-line call
    /// site that can't accidentally drop the result).
    pub fn ingest(
        &mut self,
        phase: Phase,
        id: u64,
        pos: (f64, f64),
        now: Instant,
    ) -> Option<RecognizedGesture> {
        match phase {
            Phase::Started => self.on_started(id, pos, now),
            Phase::Moved => self.on_moved(id, pos),
            Phase::Ended => {
                self.on_ended(id);
                None
            }
        }
    }

    /// Frame-boundary tick. The runtime calls this once per frame
    /// (cheap when the state is `Idle` — one branch). Long-press
    /// fires here, not on the `Moved` / `Started` events, because
    /// "the user has been holding for 350ms" is a wall-clock
    /// transition, not a touch-event-driven one. Symmetric with
    /// [`Self::ingest`] — no `#[must_use]` for the same
    /// reasoning.
    pub fn tick(&mut self, now: Instant) -> Option<RecognizedGesture> {
        if let State::OneFinger {
            track,
            long_press_emitted,
        } = &mut self.state
        {
            if !*long_press_emitted
                && !track.has_moved
                && now.duration_since(track.started_at) >= self.long_press
            {
                *long_press_emitted = true;
                return Some(RecognizedGesture::LongPress {
                    pos: track.current_pos,
                });
            }
        }
        None
    }

    /// Reset to `Idle` — cancels any in-flight gesture without
    /// emitting.
    ///
    /// Test-gated. The doc used to say "called on context loss /
    /// window minimize"; neither run loop has ever called it, on
    /// either target. Its contract stays pinned by the recognizer
    /// tests so the day a suspend / visibility-change handler lands,
    /// the behavior it needs is already specified.
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    fn on_started(&mut self, id: u64, pos: (f64, f64), now: Instant) -> Option<RecognizedGesture> {
        match self.state {
            State::Idle => {
                self.state = State::OneFinger {
                    track: FingerTrack::new(id, pos, now),
                    long_press_emitted: false,
                };
                None
            }
            State::OneFinger { track, .. } => {
                let secondary = FingerTrack::new(id, pos, now);
                let centroid = midpoint(track.current_pos, secondary.current_pos);
                self.state = State::TwoFingers {
                    primary: track,
                    secondary,
                    last_emit_centroid: centroid,
                };
                None
            }
            State::TwoFingers { .. } => {
                // Third finger landed — outside the supported
                // gesture vocabulary. Stay in TwoFingers; ignore
                // the third finger's events. (winit will route
                // its `Moved`/`Ended` to us; we'll filter by id.)
                None
            }
        }
    }

    fn on_moved(&mut self, id: u64, pos: (f64, f64)) -> Option<RecognizedGesture> {
        let move_threshold_sq = self.move_threshold_sq;
        match &mut self.state {
            State::Idle => None,
            State::OneFinger { track, .. } => {
                if track.id == id {
                    track.update_pos(pos, move_threshold_sq);
                }
                None
            }
            State::TwoFingers {
                primary,
                secondary,
                last_emit_centroid,
            } => {
                if primary.id == id {
                    primary.update_pos(pos, move_threshold_sq);
                } else if secondary.id == id {
                    secondary.update_pos(pos, move_threshold_sq);
                } else {
                    return None;
                }
                let centroid = midpoint(primary.current_pos, secondary.current_pos);
                let (dx, dy) = (
                    centroid.0 - last_emit_centroid.0,
                    centroid.1 - last_emit_centroid.1,
                );
                if dx * dx + dy * dy > move_threshold_sq {
                    *last_emit_centroid = centroid;
                    return Some(RecognizedGesture::TwoFingerDrag { pos: centroid });
                }
                None
            }
        }
    }

    fn on_ended(&mut self, id: u64) {
        match self.state {
            State::Idle => {}
            State::OneFinger { track, .. } => {
                if track.id == id {
                    self.state = State::Idle;
                }
            }
            State::TwoFingers {
                primary, secondary, ..
            } => {
                if primary.id == id {
                    // Demote to OneFinger tracking the secondary.
                    // No long-press recognition here — the user
                    // has been actively two-finger-dragging, so
                    // resetting the long-press timer for the
                    // remaining finger is the wrong UX.
                    self.state = State::OneFinger {
                        track: FingerTrack {
                            // Reset started_at to "long ago" so
                            // long-press can never fire from a
                            // demoted finger.
                            started_at: primary.started_at,
                            ..secondary
                        },
                        long_press_emitted: true,
                    };
                } else if secondary.id == id {
                    self.state = State::OneFinger {
                        track: FingerTrack {
                            started_at: primary.started_at,
                            ..primary
                        },
                        long_press_emitted: true,
                    };
                }
            }
        }
    }
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::POINTER_DRAG_THRESHOLD_PX;

    /// Test thresholds. Tight long-press (10ms) so tests don't
    /// sleep; same move threshold as production so distance math
    /// matches the real recogniser.
    const TEST_LONG_PRESS: Duration = Duration::from_millis(10);

    fn r() -> TouchGestureRecognizer {
        TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, POINTER_DRAG_THRESHOLD_PX)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// `LongPress` fires when one finger is held in place past
    /// the threshold with no movement. The recogniser latches
    /// the emission so subsequent ticks while the same finger
    /// is held don't re-fire.
    #[test]
    fn long_press_fires_after_threshold_with_no_movement() {
        let mut rec = r();
        let t = t0();
        assert!(rec.ingest(Phase::Started, 1, (100.0, 200.0), t).is_none());
        // Tick before threshold — no fire.
        assert!(rec.tick(t + Duration::from_millis(5)).is_none());
        // Tick after threshold — fire.
        let g = rec.tick(t + Duration::from_millis(15)).expect("LongPress");
        assert_eq!(g, RecognizedGesture::LongPress { pos: (100.0, 200.0) });
        // Latch: same state, additional tick should not re-fire.
        assert!(rec.tick(t + Duration::from_millis(20)).is_none());
    }

    /// Movement past the threshold cancels the long-press.
    #[test]
    fn long_press_cancelled_by_movement_past_threshold() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        // Move 10px (well past the 5px threshold).
        rec.ingest(Phase::Moved, 1, (110.0, 200.0), t + Duration::from_millis(2));
        // Tick past threshold — must not fire.
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    /// Movement under the threshold doesn't cancel — the user's
    /// finger jitter shouldn't kill the long-press.
    #[test]
    fn long_press_survives_sub_threshold_jitter() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        // 2px move — under the 5px threshold.
        rec.ingest(Phase::Moved, 1, (102.0, 200.0), t + Duration::from_millis(2));
        let g = rec
            .tick(t + Duration::from_millis(15))
            .expect("LongPress despite jitter");
        // Long-press emits the *current* position, not the
        // started position — surfaces the jittered location.
        assert_eq!(g, RecognizedGesture::LongPress { pos: (102.0, 200.0) });
    }

    /// Lifting the finger before the threshold cancels.
    #[test]
    fn long_press_cancelled_by_finger_lift_before_threshold() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        rec.ingest(Phase::Ended, 1, (100.0, 200.0), t + Duration::from_millis(5));
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    /// A second finger landing transitions OneFinger →
    /// TwoFingers; long-press never fires from that point.
    #[test]
    fn second_finger_cancels_long_press_path() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 200.0), t + Duration::from_millis(2));
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    /// `TwoFingerDrag` fires when the centroid moves past the
    /// threshold after the second finger lands. The first
    /// emission requires actual centroid movement, not just the
    /// second finger landing.
    #[test]
    fn two_finger_drag_fires_on_centroid_movement() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        // Centroid at (150, 100) after second finger lands.
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        // Both fingers move 10px right — centroid shifts 10px.
        rec.ingest(Phase::Moved, 1, (110.0, 100.0), t);
        let g = rec
            .ingest(Phase::Moved, 2, (210.0, 100.0), t)
            .expect("TwoFingerDrag fires");
        // New centroid: (160, 100) — moved 10px from (150, 100).
        assert_eq!(g, RecognizedGesture::TwoFingerDrag { pos: (160.0, 100.0) });
    }

    /// Sub-threshold centroid movement doesn't fire — the user
    /// is just stabilising their grip.
    #[test]
    fn two_finger_drag_does_not_fire_on_sub_threshold_movement() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        // 2px finger jitter → ~1px centroid shift.
        assert!(rec.ingest(Phase::Moved, 1, (102.0, 100.0), t).is_none());
    }

    /// Each "drag step" past the threshold fires once.
    /// Continuous dragging should produce multiple emissions
    /// — the dispatch chain treats each as a discrete
    /// fast-resize-start (matching `RightDrag` semantics).
    #[test]
    fn two_finger_drag_emits_per_step() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        // First step: centroid (150, 100) → (160, 100). Fire.
        rec.ingest(Phase::Moved, 1, (110.0, 100.0), t);
        assert!(rec.ingest(Phase::Moved, 2, (210.0, 100.0), t).is_some());
        // Second step: centroid (160, 100) → (170, 100). Fire.
        rec.ingest(Phase::Moved, 1, (120.0, 100.0), t);
        assert!(rec.ingest(Phase::Moved, 2, (220.0, 100.0), t).is_some());
    }

    /// Lifting one finger from TwoFingers demotes back to
    /// OneFinger but with `long_press_emitted = true` so the
    /// remaining finger can never trigger a stale long-press.
    #[test]
    fn lifting_one_of_two_fingers_does_not_trigger_long_press() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        rec.ingest(Phase::Ended, 1, (100.0, 100.0), t + Duration::from_millis(5));
        // Even after long-press timeout the remaining finger
        // shouldn't fire LongPress — the user was clearly
        // mid-two-finger-drag, not mid-long-press.
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
    }

    /// Lifting both fingers returns the recogniser to Idle.
    /// A subsequent fresh start produces a normal long-press.
    #[test]
    fn back_to_idle_after_both_fingers_lift() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        rec.ingest(Phase::Ended, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Ended, 2, (200.0, 100.0), t);
        // Fresh long-press episode.
        let t1 = t + Duration::from_millis(100);
        rec.ingest(Phase::Started, 3, (50.0, 50.0), t1);
        let g = rec
            .tick(t1 + Duration::from_millis(15))
            .expect("fresh LongPress after Idle");
        assert_eq!(g, RecognizedGesture::LongPress { pos: (50.0, 50.0) });
    }

    /// A third finger landing on TwoFingers is ignored — its
    /// `Moved` events also don't drive the centroid.
    #[test]
    fn third_finger_does_not_disrupt_two_finger_drag() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        // Third finger ignored.
        rec.ingest(Phase::Started, 3, (1000.0, 1000.0), t);
        // Move third finger far away — centroid math should be
        // unaffected (only 1 + 2 contribute).
        let g_third = rec.ingest(Phase::Moved, 3, (2000.0, 2000.0), t);
        assert!(g_third.is_none(), "third-finger move must not emit");
        // Move primary; centroid moves enough to fire.
        let g_primary = rec.ingest(Phase::Moved, 1, (120.0, 100.0), t);
        // (120 + 200) / 2 = 160; was 150 → 10px shift > 5px.
        assert_eq!(
            g_primary,
            Some(RecognizedGesture::TwoFingerDrag { pos: (160.0, 100.0) })
        );
    }

    /// `reset()` clears state regardless of variant. Covers
    /// the runtime path that responds to context loss / window
    /// minimisation by aborting the in-flight gesture.
    #[test]
    fn reset_clears_state_from_any_variant() {
        let mut rec = r();
        let t = t0();
        // From OneFinger:
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.reset();
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
        // From TwoFingers:
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        rec.reset();
        // After reset, a fresh first-finger gesture works.
        let t1 = t + Duration::from_millis(100);
        rec.ingest(Phase::Started, 3, (50.0, 50.0), t1);
        assert!(rec.tick(t1 + Duration::from_millis(15)).is_some());
    }

    /// `RecognizedGesture::mouse_gesture` round-trip — the
    /// recogniser's emit form maps to the [`MouseGesture`]
    /// variants the keybind table indexes.
    #[test]
    fn recognized_gesture_maps_to_mouse_gesture_variant() {
        assert_eq!(
            RecognizedGesture::LongPress { pos: (0.0, 0.0) }.mouse_gesture(),
            MouseGesture::LongPress
        );
        assert_eq!(
            RecognizedGesture::TwoFingerDrag { pos: (0.0, 0.0) }.mouse_gesture(),
            MouseGesture::TwoFingerDrag
        );
    }

    /// `Moved` events whose finger id doesn't match the tracked
    /// finger are ignored. Guards against a bogus winit event
    /// stream where a never-Started id arrives Moved.
    #[test]
    fn moved_for_untracked_finger_id_is_ignored() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Moved, 999, (500.0, 500.0), t);
        // Long-press should still fire from the tracked finger.
        let g = rec
            .tick(t + Duration::from_millis(15))
            .expect("LongPress despite untracked Moved");
        assert_eq!(g, RecognizedGesture::LongPress { pos: (100.0, 100.0) });
    }
    /// The **touch** side of the one-threshold-for-every-pointer
    /// claim: a finger promotes to a drag after exactly the travel
    /// [`POINTER_DRAG_THRESHOLD_SQ_PX`] allows. The reference distance
    /// is read back out of that constant and driven through a
    /// *production* [`TouchGestureRecognizer`], so the assertion spans
    /// the recognizer rather than restating the number.
    ///
    /// The input that makes it fail is the shape this replaced: a
    /// touch-local `MOVE_THRESHOLD_PX = 4.0`. A 4.99px centroid
    /// step is inside the mouse's budget and outside that one, so
    /// the first assertion fires.
    ///
    /// **What it does not catch**, stated because the claim it used to
    /// carry was false: re-inlining `25.0` in `event_cursor_moved.rs`
    /// leaves this test green. Its "mouse side" is the same constant
    /// the recognizer reads, so the two ends of the comparison are two
    /// derivations from one source — planting that literal leaves the
    /// whole `mandala` suite green but for the one test written for it,
    /// `event_cursor_moved::tests::test_the_mouse_drag_arms_name_the_shared_pointer_threshold`,
    /// which reads the mouse arms out of the source. That pin and this
    /// test are the parity claim together.
    #[test]
    fn test_pointer_drag_threshold_is_shared_by_touch_and_mouse() {
        let budget_px = POINTER_DRAG_THRESHOLD_SQ_PX.sqrt();
        // Centroid of two fingers moves half as far as one finger,
        // so a `2 * d` finger step is a `d` centroid step.
        let under = 2.0 * (budget_px - 0.01);
        let over = 2.0 * (budget_px + 0.01);
        let t = t0();

        let mut rec = TouchGestureRecognizer::new();
        rec.ingest(Phase::Started, 1, (0.0, 0.0), t);
        rec.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert!(
            rec.ingest(Phase::Moved, 1, (under, 0.0), t).is_none(),
            "a centroid step of {} px is inside the {budget_px}px pointer budget \
             and must not emit a drag",
            budget_px - 0.01
        );

        let mut rec = TouchGestureRecognizer::new();
        rec.ingest(Phase::Started, 1, (0.0, 0.0), t);
        rec.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert_eq!(
            rec.ingest(Phase::Moved, 2, (100.0 + over, 0.0), t),
            Some(RecognizedGesture::TwoFingerDrag {
                pos: (50.0 + budget_px + 0.01, 0.0)
            }),
            "a centroid step of {} px is outside the {budget_px}px pointer budget \
             and must emit a drag",
            budget_px + 0.01
        );
    }

    /// The long-press cancel latch reads the recognizer's own
    /// threshold, not the module constant. Both directions are
    /// exercised on the one path: a loosened budget must let a
    /// 10px drift keep the candidate alive, and a tightened one
    /// must kill it at 2px.
    ///
    /// The input that makes it fail is the shape this replaced,
    /// where `FingerTrack::update_pos` compared against the
    /// module constant directly: the loose case then cancels at
    /// 10px and the first assertion fires, leaving
    /// `TouchGestureRecognizer::with_thresholds` half-honored —
    /// it configured the two-finger step and not this one.
    #[test]
    fn test_long_press_cancel_honors_the_configured_move_threshold() {
        let t = t0();

        let mut loose = TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, 40.0);
        loose.ingest(Phase::Started, 1, (100.0, 200.0), t);
        loose.ingest(Phase::Moved, 1, (110.0, 200.0), t + Duration::from_millis(2));
        assert_eq!(
            loose.tick(t + Duration::from_millis(15)),
            Some(RecognizedGesture::LongPress { pos: (110.0, 200.0) }),
            "a 10px drift is inside a 40px move budget; the candidate must survive it"
        );

        let mut tight = TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, 1.0);
        tight.ingest(Phase::Started, 1, (100.0, 200.0), t);
        tight.ingest(Phase::Moved, 1, (102.0, 200.0), t + Duration::from_millis(2));
        assert!(
            tight.tick(t + Duration::from_millis(15)).is_none(),
            "a 2px drift is outside a 1px move budget; the candidate must be cancelled"
        );
    }
}
