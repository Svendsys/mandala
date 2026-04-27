// SPDX-License-Identifier: MPL-2.0

//! Shared helpers and the trait-default test macro for the
//! throttled-interaction implementors.
//!
//! Five impls under this module (`MovingNodeInteraction`,
//! `EdgeHandleInteraction`, `EdgeLabelInteraction`,
//! `PortalLabelInteraction`, `ColorPickerHoverInteraction`) all
//! inherit the `should_perform_drain` ordering invariant from
//! [`super::ThrottledInteraction`]'s default method. Pre-macro,
//! every impl carried its own four copies of the same four tests
//! exercising that invariant — the only varying parts being a
//! fixture constructor and a per-impl "set pending" hook. Those
//! four tests now live once, inside
//! [`trait_default_tests_for_throttled_interaction!`], and each
//! impl invokes the macro with its own `build` and `set_pending`
//! closures.
//!
//! Per-impl-specific tests (e.g. `test_handle_variant_round_trips_control_point`
//! on `EdgeHandleInteraction`, `test_canvas_needs_rebuild_*` on
//! `ColorPickerHoverInteraction`) are not in the macro's scope —
//! they stay inline at their respective sites.

use crate::application::frame_throttle::MutationFrequencyThrottle;
use baumhard::mindmap::model::MindEdge;
use std::time::Duration;

/// Push the throttle's average over-budget until `n > 1`. Returns
/// the final drain divisor for assertion plumbing.
pub fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
    for _ in 0..80 {
        if t.should_drain() {
            t.record_work_duration(Duration::from_micros(50_000));
        }
    }
    t.current_n()
}

/// Construct a minimally-valid `MindEdge` for the tests. The drag
/// state only references the snapshot for its pre-drag identity
/// bookkeeping; no field inside it is under test here.
pub fn fixture_edge() -> MindEdge {
    MindEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
        edge_type: "parent_child".to_string(),
        color: "#888888".to_string(),
        width: 4,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Emit the four trait-default `should_perform_drain` tests for a
/// [`super::ThrottledInteraction`] implementor.
///
/// The default method's contract is identical for every implementor:
///
/// - **Idle** → returns `false` without touching the throttle.
/// - **Pending + fresh throttle** → returns `true`.
/// - **Pending + skipping throttle (n > 1)** → returns `false` on the
///   skipped frames in the cadence.
/// - **Idle calls don't advance the throttle's skip counter.**
///
/// A new implementor invokes this macro once inside its
/// `#[cfg(test)] mod tests { ... }` block:
///
/// ```ignore
/// trait_default_tests_for_throttled_interaction! {
///     build = || MyInteraction::new(/* idle inputs */),
///     set_pending = |i: &mut MyInteraction| { i.dirty = true; },
/// }
/// ```
///
/// `build` returns a fresh idle instance (the throttle at `n = 1`,
/// no pending state). `set_pending` is `FnMut(&mut T)` and flips
/// the impl's `has_pending()` to true — the macro calls it
/// repeatedly inside the cadence-skip test, so it must be cheap
/// and idempotent.
///
/// The macro reaches the throttle through the trait's
/// [`ThrottledInteraction::throttle`] accessor — the same seam
/// production drain code uses. A future implementor that satisfies
/// the trait works without macro changes; a renamed `throttle` field
/// shows up as a clean compile error at the trait impl, not at every
/// macro expansion.
macro_rules! trait_default_tests_for_throttled_interaction {
    (
        build = $build:expr,
        set_pending = $set_pending:expr $(,)?
    ) => {
        #[test]
        fn test_should_perform_drain_false_when_idle() {
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            assert!(
                !i.should_perform_drain(),
                "idle interaction must report no drain"
            );
        }

        #[test]
        fn test_should_perform_drain_true_when_pending_and_throttle_fresh() {
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            ($set_pending)(&mut i);
            assert!(
                i.should_perform_drain(),
                "pending interaction with fresh throttle must drain"
            );
        }

        #[test]
        fn test_should_perform_drain_false_when_throttle_skipping() {
            // Throttle cadence under sustained over-budget load: at
            // n > 1, should_perform_drain must return false on the
            // skipped frames even when pending state is set.
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            $crate::application::app::throttled_interaction::test_utils::drive_throttle_over_budget(
                i.throttle(),
            );
            assert!(i.throttle().current_n() > 1);

            let n = i.throttle().current_n() as usize;
            ($set_pending)(&mut i);
            let mut saw_skip = false;
            for _ in 0..(n * 2) {
                if !i.should_perform_drain() {
                    saw_skip = true;
                }
                // Keep n stable while probing cadence.
                i.throttle().record_work_duration(::std::time::Duration::from_micros(50_000));
                ($set_pending)(&mut i);
            }
            assert!(saw_skip, "expected at least one skipped drain at n > 1");
        }

        #[test]
        fn test_idle_should_perform_drain_does_not_advance_throttle() {
            // Invariant — if should_perform_drain consulted
            // should_drain first, this would be off by n: several
            // idle calls would advance `frames_since_drain` and the
            // next pending tick would skip instead of drain.
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            for _ in 0..5 {
                assert!(!i.should_perform_drain());
            }
            ($set_pending)(&mut i);
            assert!(
                i.should_perform_drain(),
                "first pending tick after idles must drain"
            );
        }
    };
}

pub(crate) use trait_default_tests_for_throttled_interaction;