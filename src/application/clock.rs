// SPDX-License-Identifier: MPL-2.0

//! The app clock: the one time axis everything the app *observes*
//! reads — animations, the drag pause on them, double-click and
//! long-press timing, an `OnClick` trigger's start, a recording's
//! frame times — and the virtual mode that makes that axis advance
//! only when it is told to (`format/ipc.md` §`clock`;
//! `work_plans/LLM_IPC.md` §D9).
//!
//! **Two clocks, and which one a reader wants.** [`wall_ms`] is the
//! process's monotonic wall clock. [`AppClock::now_ms`] is the app
//! clock: the wall clock in real mode, a stored reading in virtual
//! mode. Whatever *measures cost* — the FPS overlay, the freeze
//! watchdog, the drag throttle's budget, an encode's timing, a
//! deadline that guards a client's liveness — reads the wall clock (or
//! a `web_time::Instant`, which is the same clock), because a stepped
//! or frozen clock would make a stall invisible. Whatever *behaves*
//! reads the app clock, because a behavior that read the wall clock
//! would leak scheduler timing into a run that is meant to be a pure
//! function of its script. Each reader's disposition is listed in
//! §D9.
//!
//! **One continuous axis.** Switching modes never moves the reading:
//! virtual time starts where real time stood (rounded up to a whole
//! millisecond, so integer steps keep every reading integral), and
//! real time resumes from where virtual time was left, running at the
//! wall clock's rate from there. That is why engaging or leaving
//! virtual mode rebases nothing — an animation's `start_ms`, the last
//! click's time, a touch's start, a recording's start all stay valid
//! as they are, because they are readings on the one axis. Rebasing
//! the in-flight animations alone (the shift primitive the drag pause
//! uses) would have left every other stored reading on a different
//! axis from the animations'.
//!
//! **Cross-platform.** The type and its modes compile for both
//! targets and the browser holds one, in real mode. What advances
//! virtual time is the heartbeat, and the heartbeat is native today
//! (`CLAUDE.md` "Dual-target status", "Per-frame animation drain"),
//! so `clock.step` is native for the reason the drain is.

use std::ops::RangeInclusive;
use std::sync::OnceLock;
use std::time::Duration;

// `web_time` maps to `performance.now()` on wasm32, where
// `std::time::Instant::now()` traps ("time not implemented on this
// platform"); on native it is `std::time::Instant`.
use web_time::Instant;

/// Virtual milliseconds per heartbeat: the cadence `clock.step` runs
/// the frame at, ≈60 fps, the rate the app targets.
pub(crate) const HEARTBEAT_MS: u64 = 16;

/// The most heartbeats one `clock.step` may run, by either spelling.
/// A responsiveness bound, not the watchdog's safety: every heartbeat
/// ticks the watchdog.
pub(crate) const MAX_STEP_FRAMES: u64 = 100_000;

/// The `ms` a step may ask for: as many as [`MAX_STEP_FRAMES`]
/// heartbeats cover, so the two spellings bound the same work.
pub(crate) const STEP_MS_RANGE: RangeInclusive<u64> = 1..=MAX_STEP_FRAMES * HEARTBEAT_MS;

/// The `frames` a step may ask for.
pub(crate) const STEP_FRAMES_RANGE: RangeInclusive<u64> = 1..=MAX_STEP_FRAMES;

/// The instant the wall clock counts from: the first read of either
/// clock.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// The process's monotonic wall clock, in milliseconds since it was
/// first read. For what measures cost; what behaves reads
/// [`AppClock::now_ms`] (see the module doc).
pub(crate) fn wall_ms() -> f64 {
    epoch().elapsed().as_secs_f64() * 1000.0
}

/// Which way the app clock runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClockMode {
    /// At the wall clock's rate.
    #[default]
    Real,
    /// Only when stepped.
    Virtual,
}

impl ClockMode {
    /// Every mode, in the order the wire lists them.
    pub(crate) const ALL: [ClockMode; 2] = [ClockMode::Real, ClockMode::Virtual];

    /// The wire's word for the mode.
    pub(crate) const fn word(self) -> &'static str {
        match self {
            ClockMode::Real => "real",
            ClockMode::Virtual => "virtual",
        }
    }

    /// The mode `word` names, case-exact as the wire spells it.
    pub(crate) fn parse(word: &str) -> Option<ClockMode> {
        ClockMode::ALL.into_iter().find(|mode| mode.word() == word)
    }
}

/// The app clock (see the module doc). One per app shell; native's
/// lives on `InitState`, the browser's on `WasmInputState`.
#[derive(Debug, Clone, Default)]
pub(crate) struct AppClock {
    /// What real mode adds to the wall clock: zero until virtual time
    /// is first left, then whatever keeps the axis continuous.
    offset_ms: f64,
    /// The reading, while virtual.
    virtual_ms: Option<f64>,
    /// A step is running. A step asked for from inside one (a `clock
    /// step` console line typed by a sample the step itself landed) is
    /// refused rather than nested.
    stepping: bool,
}

impl AppClock {
    /// The app clock's reading, in milliseconds.
    pub(crate) fn now_ms(&self) -> f64 {
        self.reading_at(wall_ms())
    }

    /// [`Self::now_ms`] as the whole milliseconds animation and
    /// trigger stamps are kept in. Exact under virtual time, whose
    /// readings are integral (see [`Self::set_mode`]).
    pub(crate) fn now_whole_ms(&self) -> u64 {
        self.now_ms() as u64
    }

    /// The reading as an [`Instant`], for a reader typed in instants
    /// (the touch recognizer). It is the wall clock's instant in real
    /// mode until virtual time is first used, and the same axis as
    /// [`Self::now_ms`] always, so durations measured between two of
    /// these are app-clock durations.
    pub(crate) fn instant(&self) -> Instant {
        epoch() + Duration::from_secs_f64(self.now_ms().max(0.0) / 1000.0)
    }

    pub(crate) fn mode(&self) -> ClockMode {
        if self.virtual_ms.is_some() {
            ClockMode::Virtual
        } else {
            ClockMode::Real
        }
    }

    pub(crate) fn is_virtual(&self) -> bool {
        self.virtual_ms.is_some()
    }

    /// Run the clock in `mode` from now on, without moving its reading
    /// (see the module doc). Entering virtual time rounds the reading
    /// up to the next whole millisecond, so a step of whole
    /// milliseconds lands on whole milliseconds and the `u64` stamps
    /// animations keep lose nothing. Returns whether the mode changed.
    pub(crate) fn set_mode(&mut self, mode: ClockMode) -> bool {
        self.set_mode_at(mode, wall_ms())
    }

    /// Move a virtual clock forward to `to_ms`. Never backwards: a
    /// target behind the reading leaves it where it is, so the axis
    /// stays monotonic whatever the caller computes. A no-op in real
    /// mode, where the wall clock is what moves it.
    pub(crate) fn advance_to(&mut self, to_ms: f64) {
        if let Some(reading) = self.virtual_ms.as_mut() {
            if to_ms > *reading {
                *reading = to_ms;
            }
        }
    }

    /// Mark a step as running. `false` when one already is, and the
    /// caller must not run another inside it.
    pub(crate) fn begin_step(&mut self) -> bool {
        !std::mem::replace(&mut self.stepping, true)
    }

    /// The step [`Self::begin_step`] began is over.
    pub(crate) fn end_step(&mut self) {
        self.stepping = false;
    }

    /// The reading when the wall clock reads `wall`.
    fn reading_at(&self, wall: f64) -> f64 {
        self.virtual_ms.unwrap_or(wall + self.offset_ms)
    }

    /// [`Self::set_mode`] with the wall clock reading `wall`.
    fn set_mode_at(&mut self, mode: ClockMode, wall: f64) -> bool {
        match (mode, self.virtual_ms) {
            (ClockMode::Virtual, None) => {
                self.virtual_ms = Some(self.reading_at(wall).ceil());
                true
            }
            (ClockMode::Real, Some(reading)) => {
                self.offset_ms = reading - wall;
                self.virtual_ms = None;
                true
            }
            _ => false,
        }
    }
}

/// Why a clock surface refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClockRefusal {
    /// A step needs virtual time, and the clock is real.
    Real,
    /// A step was asked for from inside a running one (a `clock step`
    /// console line typed by an injected key mid-step).
    Stepping,
    /// The mode cannot change while a recording runs: its pacing, and
    /// so what its frame times mean, is fixed for its whole length.
    Recording,
}

impl ClockRefusal {
    /// What went wrong and what to do instead, for every surface: each
    /// adds only its own spelling of the remedy's command.
    pub(crate) fn message(self) -> &'static str {
        match self {
            ClockRefusal::Real => {
                "the clock is real, and only a virtual clock is stepped; switch it first \
                 (clock.set_mode virtual, or `clock virtual` in the console)"
            }
            ClockRefusal::Stepping => {
                "a step is already running, and this one was asked for from inside it; \
                 steps do not nest"
            }
            ClockRefusal::Recording => {
                "a recording is running, and its pacing is fixed for its whole length; stop it \
                 first (capture.record_stop, or `record stop` in the console)"
            }
        }
    }
}

/// What a step did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stepped {
    /// The app clock after the step.
    pub(crate) now_ms: f64,
    /// Cadence heartbeats run: [`StepRequest::frames`]. A recording's
    /// boundary ticks are the recorder's to count, in its frames.
    pub(crate) frames_run: u64,
}

/// How far one `clock.step` goes: its two spellings, which are two
/// ways of naming the same work (`format/ipc.md` §`clock.step`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepRequest {
    /// Advance exactly this many virtual milliseconds.
    Ms(u64),
    /// Run exactly this many heartbeats, [`HEARTBEAT_MS`] apart.
    Frames(u64),
}

impl StepRequest {
    /// The virtual milliseconds the step advances.
    pub(crate) fn duration_ms(self) -> u64 {
        match self {
            StepRequest::Ms(ms) => ms,
            StepRequest::Frames(frames) => frames * HEARTBEAT_MS,
        }
    }

    /// The heartbeats the step runs: one per [`HEARTBEAT_MS`], the
    /// last one landing on the step's end, so `ceil(ms / 16)` for the
    /// `ms` spelling.
    pub(crate) fn frames(self) -> u64 {
        match self {
            StepRequest::Ms(ms) => ms.div_ceil(HEARTBEAT_MS),
            StepRequest::Frames(frames) => frames,
        }
    }

    /// The step that `ms` or `frames` (exactly one of them) names, or
    /// why there is none: the parameter to blame and what is wrong
    /// with it. The one check behind the IPC command and the console
    /// verb, so the bounds cannot drift apart.
    pub(crate) fn of(ms: Option<u64>, frames: Option<u64>) -> Result<StepRequest, (&'static str, String)> {
        let within = |name: &'static str, value: u64, range: RangeInclusive<u64>| {
            let (low, high) = (*range.start(), *range.end());
            if range.contains(&value) {
                Ok(value)
            } else if value > high {
                Err((
                    name,
                    format!(
                        "`{name}` must be in {low}..={high}, got {value}: step in chunks of at most {high}"
                    ),
                ))
            } else {
                Err((name, format!("`{name}` must be in {low}..={high}, got {value}")))
            }
        };
        match (ms, frames) {
            (Some(ms), None) => within("ms", ms, STEP_MS_RANGE).map(StepRequest::Ms),
            (None, Some(frames)) => within("frames", frames, STEP_FRAMES_RANGE).map(StepRequest::Frames),
            (Some(_), Some(_)) => Err((
                "frames",
                "`ms` and `frames` are two spellings of one step: give one of them, not both".into(),
            )),
            (None, None) => Err((
                "ms",
                "a step needs one of `ms` (virtual milliseconds) or `frames` (heartbeats)".into(),
            )),
        }
    }
}

/// The heartbeats of one step, as app-clock readings: one every
/// [`HEARTBEAT_MS`] from `start_ms`, the last on the step's end
/// exactly — so a step of a length that is not a multiple of the
/// cadence ends with a shorter interval rather than overshooting.
/// Pure, so the cadence is pinned without an app.
pub(crate) fn heartbeat_times(start_ms: f64, request: StepRequest) -> impl Iterator<Item = f64> {
    let end = start_ms + request.duration_ms() as f64;
    (1..=request.frames()).map(move |beat| (start_ms + (beat * HEARTBEAT_MS) as f64).min(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_in(mode: ClockMode, wall: f64) -> AppClock {
        let mut clock = AppClock::default();
        clock.set_mode_at(mode, wall);
        clock
    }

    #[test]
    fn test_a_new_clock_is_real_and_reads_the_wall_clock() {
        let clock = AppClock::default();
        assert_eq!(clock.mode(), ClockMode::Real);
        assert_eq!(clock.reading_at(1234.5), 1234.5);
        let before = wall_ms();
        let now = clock.now_ms();
        assert!(now >= before, "{now} < {before}");
    }

    /// Engaging virtual time stops the clock where it stood, rounded
    /// up so whole-millisecond steps stay whole.
    #[test]
    fn test_virtual_time_starts_at_the_real_reading_rounded_up_and_stands_still() {
        let clock = clock_in(ClockMode::Virtual, 1234.25);
        assert_eq!(clock.mode(), ClockMode::Virtual);
        assert_eq!(clock.reading_at(1234.25), 1235.0);
        assert_eq!(
            clock.reading_at(99_999.0),
            1235.0,
            "the wall clock no longer moves it"
        );
        assert_eq!(clock.now_ms(), 1235.0);
        assert_eq!(clock.now_whole_ms(), 1235);
    }

    /// **The axis is continuous across both switches.** Leaving virtual
    /// time resumes from the virtual reading, at the wall clock's rate,
    /// whether virtual time ran ahead of the wall clock or fell behind
    /// it — so no stored reading (an animation's start, the last click)
    /// is ever on the wrong side of a jump.
    #[test]
    fn test_leaving_virtual_time_resumes_from_the_virtual_reading_at_the_wall_rate() {
        for (stepped_to, wall_at_exit) in [(60_000.0, 2_000.0), (1_100.0, 9_000.0)] {
            let mut clock = clock_in(ClockMode::Virtual, 1_000.0);
            clock.advance_to(stepped_to);
            assert!(clock.set_mode_at(ClockMode::Real, wall_at_exit));
            assert_eq!(clock.reading_at(wall_at_exit), stepped_to);
            assert_eq!(clock.reading_at(wall_at_exit + 250.0), stepped_to + 250.0);
        }
    }

    #[test]
    fn test_setting_the_mode_the_clock_is_in_changes_nothing() {
        let mut clock = clock_in(ClockMode::Virtual, 10.0);
        assert!(!clock.set_mode_at(ClockMode::Virtual, 5_000.0));
        assert_eq!(clock.reading_at(5_000.0), 10.0);
        let mut real = AppClock::default();
        assert!(!real.set_mode_at(ClockMode::Real, 5.0));
        assert_eq!(real.reading_at(5.0), 5.0);
    }

    /// A virtual clock moves only forward, and a real one is moved by
    /// nothing but the wall clock.
    #[test]
    fn test_advancing_never_moves_the_clock_backwards_or_a_real_clock_at_all() {
        let mut clock = clock_in(ClockMode::Virtual, 100.0);
        clock.advance_to(500.0);
        clock.advance_to(200.0);
        assert_eq!(clock.reading_at(0.0), 500.0);
        let mut real = AppClock::default();
        real.advance_to(1e9);
        assert_eq!(real.reading_at(7.0), 7.0);
    }

    /// The instant a reader typed in instants sees is the same axis as
    /// the milliseconds: two virtual readings 400 ms apart are two
    /// instants 400 ms apart, however long the wall clock took.
    #[test]
    fn test_the_instant_is_on_the_app_clocks_axis() {
        let mut clock = AppClock::default();
        clock.set_mode(ClockMode::Virtual);
        let before = clock.instant();
        clock.advance_to(clock.now_ms() + 400.0);
        let after = clock.instant();
        let apart = after.saturating_duration_since(before).as_secs_f64() * 1000.0;
        // Each instant is a float-to-`Duration` conversion, which may
        // round by a nanosecond either way.
        assert!((apart - 400.0).abs() < 1e-3, "{apart} ms apart");
    }

    #[test]
    fn test_mode_words_round_trip_case_exact() {
        for mode in ClockMode::ALL {
            assert_eq!(ClockMode::parse(mode.word()), Some(mode));
        }
        assert_eq!(ClockMode::parse("Virtual"), None);
        assert_eq!(ClockMode::parse("wall"), None);
    }

    /// `ms` and `frames` are two spellings of one budget: `frames =
    /// ceil(ms / 16)`, both bounded at 100 000 heartbeats.
    #[test]
    fn test_both_step_spellings_name_the_same_bounded_work() {
        assert_eq!(StepRequest::Ms(500).frames(), 32);
        assert_eq!(StepRequest::Ms(16).frames(), 1);
        assert_eq!(StepRequest::Ms(1).frames(), 1);
        assert_eq!(StepRequest::Frames(30).duration_ms(), 480);
        assert_eq!(
            StepRequest::of(Some(1_600_000), None),
            Ok(StepRequest::Ms(1_600_000))
        );
        assert_eq!(
            StepRequest::of(None, Some(100_000)),
            Ok(StepRequest::Frames(100_000))
        );
        assert_eq!(
            StepRequest::of(Some(1_600_000), None).map(StepRequest::frames),
            StepRequest::of(None, Some(100_000)).map(StepRequest::frames),
            "the two bounds are the same budget"
        );
        for (ms, frames, blamed) in [
            (Some(1_600_001), None, "ms"),
            (Some(0), None, "ms"),
            (None, Some(100_001), "frames"),
            (None, Some(0), "frames"),
            (Some(16), Some(1), "frames"),
            (None, None, "ms"),
        ] {
            let (param, _) = StepRequest::of(ms, frames).expect_err("refused");
            assert_eq!(param, blamed, "ms={ms:?} frames={frames:?}");
        }
    }

    /// The cadence: a heartbeat every 16 virtual ms, the last on the
    /// step's end exactly, never past it.
    #[test]
    fn test_heartbeats_run_every_sixteen_ms_and_the_last_lands_on_the_end() {
        let times: Vec<f64> = heartbeat_times(1000.0, StepRequest::Ms(40)).collect();
        assert_eq!(times, [1016.0, 1032.0, 1040.0]);
        let times: Vec<f64> = heartbeat_times(0.0, StepRequest::Frames(3)).collect();
        assert_eq!(times, [16.0, 32.0, 48.0]);
        let times: Vec<f64> = heartbeat_times(7.0, StepRequest::Ms(500)).collect();
        assert_eq!(times.len(), 32);
        assert_eq!(times.last(), Some(&507.0));
    }
}
