// SPDX-License-Identifier: MPL-2.0

//! Mutation-frequency throttle.
//!
//! Responsiveness is never traded for visual fidelity. Input is
//! accumulated every tick; the throttle gates only how often the
//! mutation-and-rebuild work runs. A moving average over
//! [`WINDOW_SIZE`] drained frames raises the drain divisor `n`
//! toward [`MAX_N`] when work overruns the budget and decays it
//! back when it drops, with a 30% hysteresis band to prevent
//! oscillation. Self-tuning — the only knob is the budget at
//! construction.

use std::collections::VecDeque;
use std::time::Duration;

/// Size of the moving-average window. Eight frames is small enough to
/// react to a sustained stress within ~130ms on a 60 Hz display, and
/// large enough to absorb single-frame noise without oscillating.
pub const WINDOW_SIZE: usize = 8;

/// Maximum drain divisor. At 60 fps, `N = 8` means the dragged node
/// updates every ~133ms — laggy but still tracking. Past this, visual
/// tracking becomes so stale that capping is the kinder behaviour; the
/// remaining budget shortfall has to be addressed by the companion
/// techniques (culling, incremental rebuild, shape-once reuse).
pub const MAX_N: u32 = 8;

/// A conservative default refresh budget: 16.67ms (60 Hz) minus ~2.7ms
/// of safety margin for GPU present and other per-frame overhead. The
/// correct value depends on the actual monitor refresh rate; runtime
/// detection of that is still an open question.
pub const DEFAULT_BUDGET: Duration = Duration::from_micros(14_000);

/// Per-frame throttle that degrades mutation frequency under load.
/// Call [`reset`] when the drag ends so the next drag starts at `n = 1`.
#[derive(Debug)]
pub struct MutationFrequencyThrottle {
    budget: Duration,
    window: VecDeque<Duration>,
    n: u32,
    frames_since_drain: u32,
}

impl MutationFrequencyThrottle {
    /// Construct with the given per-frame work budget. See [`DEFAULT_BUDGET`].
    pub fn new(budget: Duration) -> Self {
        MutationFrequencyThrottle {
            budget,
            window: VecDeque::with_capacity(WINDOW_SIZE),
            n: 1,
            frames_since_drain: 0,
        }
    }

    /// Construct with the default budget. Convenience wrapper for call
    /// sites that don't care to pass one.
    pub fn with_default_budget() -> Self {
        Self::new(DEFAULT_BUDGET)
    }

    /// Returns `true` if the caller should perform its heavy work this
    /// frame, or `false` if the frame should be skipped.
    ///
    /// Increments the internal skip counter. When the counter reaches
    /// the current drain divisor `n`, returns `true` and resets the
    /// counter to zero. Otherwise returns `false` — the caller must
    /// preserve its accumulated state so the next successful drain can
    /// fold in what this frame skipped.
    pub fn should_drain(&mut self) -> bool {
        self.frames_since_drain += 1;
        if self.frames_since_drain >= self.n {
            self.frames_since_drain = 0;
            true
        } else {
            false
        }
    }

    /// Feed a measured work duration back into the tracker. Updates the
    /// moving average and adjusts `n` to hold the invariant:
    ///
    /// - If the moving average exceeds `budget`, `n` increments toward
    ///   [`MAX_N`] (throttle engages more aggressively).
    /// - If the moving average drops below 70% of `budget`, `n` decays
    ///   toward `1` (throttle relaxes). The 30% gap is hysteresis —
    ///   without it, a frame sitting exactly at budget would oscillate.
    pub fn record_work_duration(&mut self, duration: Duration) {
        if self.window.len() >= WINDOW_SIZE {
            self.window.pop_front();
        }
        self.window.push_back(duration);

        let avg = self.moving_average();
        if avg > self.budget {
            if self.n < MAX_N {
                self.n += 1;
            }
        } else if avg < self.budget.mul_f32(0.7) && self.n > 1 {
            self.n -= 1;
        }
    }

    /// Clear the moving-average window, reset `n` to `1`, reset the
    /// skip counter. Call this when the drag ends so a fresh drag
    /// doesn't inherit lingering throttle state from the previous one.
    pub fn reset(&mut self) {
        self.window.clear();
        self.n = 1;
        self.frames_since_drain = 0;
    }

    /// Current drain divisor (1 = every frame; higher under load).
    pub fn current_n(&self) -> u32 {
        self.n
    }

    /// Current moving average, or [`Duration::ZERO`] when nothing recorded.
    pub fn moving_average(&self) -> Duration {
        if self.window.is_empty() {
            return Duration::ZERO;
        }
        let sum: Duration = self.window.iter().sum();
        sum / self.window.len() as u32
    }

    #[cfg(test)]
    pub fn budget(&self) -> Duration {
        self.budget
    }
}

impl Default for MutationFrequencyThrottle {
    fn default() -> Self {
        Self::with_default_budget()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn starts_at_n_equals_one() {
        let t = MutationFrequencyThrottle::new(ms(14));
        assert_eq!(t.current_n(), 1);
    }

    #[test]
    fn healthy_load_drains_every_frame() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        // 5ms is comfortably under budget.
        for _ in 0..20 {
            assert!(t.should_drain());
            t.record_work_duration(ms(5));
        }
        assert_eq!(t.current_n(), 1);
    }

    #[test]
    fn sustained_over_budget_raises_n() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        // 30ms is twice the budget; N should start climbing.
        for _ in 0..20 {
            if t.should_drain() {
                t.record_work_duration(ms(30));
            }
        }
        assert!(t.current_n() > 1, "expected n > 1, got {}", t.current_n());
    }

    #[test]
    fn very_heavy_load_caps_at_max_n() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        for _ in 0..200 {
            if t.should_drain() {
                t.record_work_duration(ms(200));
            }
        }
        assert_eq!(t.current_n(), MAX_N);
    }

    #[test]
    fn load_drop_decays_n_toward_one() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        // Push it up.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(ms(50));
            }
        }
        assert!(t.current_n() > 1);
        let peak = t.current_n();
        // Then drop load well under budget and run long enough to decay.
        for _ in 0..400 {
            if t.should_drain() {
                t.record_work_duration(ms(2));
            }
        }
        assert!(
            t.current_n() < peak,
            "expected n to decay from {} but got {}",
            peak,
            t.current_n()
        );
        assert_eq!(t.current_n(), 1, "expected full decay to 1");
    }

    #[test]
    fn decay_has_hysteresis_around_budget() {
        let mut t = MutationFrequencyThrottle::new(ms(10));
        // Push to n > 1.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(ms(20));
            }
        }
        let raised = t.current_n();
        assert!(raised > 1);
        // Feed durations right at 90% of budget — under budget but inside
        // the hysteresis band, so n must NOT decay.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(ms(9));
            }
        }
        assert_eq!(t.current_n(), raised, "hysteresis should prevent decay");
    }

    #[test]
    fn throttled_frames_skip_work() {
        let mut t = MutationFrequencyThrottle::new(ms(10));
        // Drive N up.
        for _ in 0..50 {
            if t.should_drain() {
                t.record_work_duration(ms(50));
            }
        }
        assert!(t.current_n() > 1);
        // Count how many of the next 32 frames actually drain.
        let mut drained = 0;
        for _ in 0..32 {
            if t.should_drain() {
                drained += 1;
                // Keep N stable: feed the same heavy duration.
                t.record_work_duration(ms(50));
            }
        }
        // At N > 1, we should drain fewer than all 32.
        assert!(drained < 32, "expected throttling to skip frames");
        // And at least once per N frames.
        assert!(drained >= 32 / MAX_N as usize);
    }

    #[test]
    fn moving_average_is_arithmetic_mean_of_window() {
        let mut t = MutationFrequencyThrottle::new(ms(100));
        t.record_work_duration(ms(10));
        t.record_work_duration(ms(20));
        t.record_work_duration(ms(30));
        assert_eq!(t.moving_average(), ms(20));
    }

    #[test]
    fn window_evicts_oldest_beyond_size() {
        let mut t = MutationFrequencyThrottle::new(ms(100));
        // Fill with 10ms frames.
        for _ in 0..WINDOW_SIZE {
            t.record_work_duration(ms(10));
        }
        assert_eq!(t.moving_average(), ms(10));
        // Push a single 100ms frame; the oldest 10ms evicts, new window
        // sum is 7*10 + 100 = 170ms spread across 8 slots = 21.25ms.
        // `Duration / u32` keeps sub-millisecond precision, so compute
        // the exact nanosecond expectation rather than truncating.
        t.record_work_duration(ms(100));
        let expected_nanos =
            (10 * (WINDOW_SIZE as u64 - 1) + 100) * 1_000_000 / WINDOW_SIZE as u64;
        assert_eq!(t.moving_average(), Duration::from_nanos(expected_nanos));
    }

    #[test]
    fn reset_returns_to_fresh_state() {
        let mut t = MutationFrequencyThrottle::new(ms(10));
        for _ in 0..50 {
            if t.should_drain() {
                t.record_work_duration(ms(50));
            }
        }
        assert!(t.current_n() > 1);
        t.reset();
        assert_eq!(t.current_n(), 1);
        assert_eq!(t.moving_average(), Duration::ZERO);
        // First post-reset call should drain immediately.
        assert!(t.should_drain());
    }

    #[test]
    fn drain_cadence_matches_n() {
        // Force n = 4 by hand, then confirm cadence.
        let mut t = MutationFrequencyThrottle::new(ms(10));
        // Reach over-budget average then manually inspect.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(ms(100));
            }
        }
        // Whatever n landed at, cadence should follow it exactly on a
        // stable window.
        let n = t.current_n();
        assert!(n >= 2);
        // Track next drain positions assuming stable n (we feed the same
        // duration so n won't move).
        let mut drain_indices = Vec::new();
        for i in 0..(n * 4) {
            if t.should_drain() {
                drain_indices.push(i);
                t.record_work_duration(ms(100));
            }
        }
        // Drain indices should be evenly spaced by `n`.
        for w in drain_indices.windows(2) {
            assert_eq!(w[1] - w[0], n, "drains not spaced by n = {}", n);
        }
    }

    #[test]
    fn default_budget_is_sub_frame_time() {
        // Sanity: default budget should be less than a 60 Hz frame.
        assert!(DEFAULT_BUDGET < Duration::from_micros(16_667));
    }

    #[test]
    fn zero_frames_recorded_reports_zero_average() {
        let t = MutationFrequencyThrottle::new(ms(14));
        assert_eq!(t.moving_average(), Duration::ZERO);
    }

    // ── §T1 comprehensive coverage ─────────────────────────────────

    #[test]
    fn test_fresh_throttle_always_drains() {
        // A new throttle with n=1 should return should_drain() == true
        // on every call, since it starts with frames_since_drain = 0
        // and n = 1 means drain every frame.
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));
        for _ in 0..50 {
            assert!(t.should_drain());
        }
        assert_eq!(t.current_n(), 1);
    }

    #[test]
    fn test_under_budget_keeps_n_at_one() {
        // Feed durations well under budget — n must stay 1 throughout.
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));
        for _ in 0..40 {
            assert!(t.should_drain());
            // 3ms is ~21% of budget — comfortably under
            t.record_work_duration(Duration::from_micros(3_000));
            assert_eq!(t.current_n(), 1);
        }
    }

    #[test]
    fn test_over_budget_raises_n() {
        // Feed durations exceeding budget (20ms when budget is 14ms)
        // repeatedly until n > 1, then verify should_drain() returns
        // false for some frames.
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));
        for _ in 0..20 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(20_000));
            }
        }
        assert!(t.current_n() > 1, "expected n > 1 after sustained over-budget, got {}", t.current_n());

        // With n > 1, some should_drain() calls must return false.
        let mut saw_false = false;
        for _ in 0..20 {
            if !t.should_drain() {
                saw_false = true;
                break;
            }
            t.record_work_duration(Duration::from_micros(20_000));
        }
        assert!(saw_false, "with n > 1, should_drain() must sometimes return false");
    }

    #[test]
    fn test_n_clamped_at_max_n() {
        // Feed extreme durations — n must never exceed MAX_N.
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));
        for _ in 0..500 {
            if t.should_drain() {
                // 500ms — absurdly over budget
                t.record_work_duration(Duration::from_micros(500_000));
            }
            assert!(t.current_n() <= MAX_N, "n exceeded MAX_N: {}", t.current_n());
        }
        assert_eq!(t.current_n(), MAX_N);
    }

    #[test]
    fn test_recovery_lowers_n() {
        // After n rises, feed under-budget durations; verify n
        // eventually returns to 1.
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));

        // Drive n up with heavy load.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(50_000));
            }
        }
        let peak = t.current_n();
        assert!(peak > 1, "n should have risen above 1");

        // Now feed very light durations — well below 70% hysteresis.
        // 1ms = 1000us, budget is 14000us, 70% is 9800us. 1ms < 9800us.
        for _ in 0..1000 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(1_000));
            }
        }
        assert_eq!(t.current_n(), 1, "n should recover to 1 after sustained under-budget load");
    }

    #[test]
    fn test_reset_returns_to_fresh_state() {
        let mut t = MutationFrequencyThrottle::new(Duration::from_micros(14_000));

        // Drive into stressed state.
        for _ in 0..100 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(50_000));
            }
        }
        assert!(t.current_n() > 1);
        assert!(t.moving_average() > Duration::ZERO);

        // Reset and verify everything looks fresh.
        t.reset();
        assert_eq!(t.current_n(), 1);
        assert_eq!(t.moving_average(), Duration::ZERO);
        // First call after reset should drain.
        assert!(t.should_drain());
        // And it should keep draining every frame (n=1, no history).
        assert!(t.should_drain());
        assert!(t.should_drain());
    }

    #[test]
    fn test_hysteresis_prevents_oscillation() {
        // Feed durations right at the boundary (~70% of budget) — n
        // should stay stable rather than flipping between 1 and 2.
        //
        // Budget = 10_000us. Hysteresis threshold = 70% = 7_000us.
        // A duration of 8_000us is under budget (no raise) but above
        // the 70% lower-threshold (no decay). Once n > 1, it should
        // stay there.
        let budget = Duration::from_micros(10_000);
        let mut t = MutationFrequencyThrottle::new(budget);

        // First, push n above 1 with clearly over-budget durations.
        for _ in 0..50 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(20_000));
            }
        }
        let raised_n = t.current_n();
        assert!(raised_n > 1, "n should be raised above 1");

        // Now feed durations inside the hysteresis band: 8ms is 80% of
        // budget, which is > 70% (lower threshold) and < 100% (upper
        // threshold). n must not change.
        for _ in 0..200 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(8_000));
            }
        }
        assert_eq!(
            t.current_n(),
            raised_n,
            "n should stay stable in hysteresis band, expected {} got {}",
            raised_n,
            t.current_n()
        );
    }
}
