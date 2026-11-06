// SPDX-License-Identifier: MPL-2.0

//! The exact work removals of #309, each held by its own counter against
//! a figure computed apart from the code it counts (review 5810838863):
//! revert a removal and its pin goes red, whatever else lands on top.

use crate::document::values::FitMode;
use crate::flow::place::{frame_halves, level_step_seam, LEVEL_ROUNDS, MIN_TABLE_SPACING};
use crate::flow::tests::halves_tests::{frame_with, sweep_outlines, QUATRAIN};
use crate::flow::tests::work_tests::Kind;
use crate::flow::RingLayout;

/// The Ecclesiastes center frame's text.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

/// R27's measures read a level step's length alone (#309): its Newton
/// rounds' lookups, [`LEVEL_ROUNDS`] `+ 1` at most, not the chord its
/// direction is read from as well — and the rounds stop at their fixed
/// point, a round being a function of the length alone, so a step along a
/// straight side ends in a few. Along both halves of each outline of the
/// aspect sweep, a 6-wide cluster's step every 6 from each half's start to
/// its end: each measure is the step's own length to the bit, takes no
/// more than that many lookups, and along a sharp outline's halves takes
/// [`LEVEL_ROUNDS`] `+ 1` on so few steps that the mean is under 2.5 —
/// 2.00 over their 800 steps, the most any step took 5.
///
/// Controls (run, not committed): `step_len` taking the step, its chord's
/// two lookups with it, red on the most a step took; the fixed point's
/// stop removed, every step taking its rounds, red on the sharp outlines'
/// mean.
#[test]
fn test_flow_r27_measures_a_level_step_by_its_length_alone() {
    let mut faults = Vec::new();
    let (mut sharp_steps, mut sharp_lookups) = (0usize, 0usize);
    let mut most = 0;
    for (name, shape, _, _) in sweep_outlines() {
        let halves = frame_halves(&shape, MIN_TABLE_SPACING);
        for (k, (path, span)) in halves.iter().enumerate() {
            let mut s = span.start;
            while s < span.end {
                let (len, step, lookups) = level_step_seam(path, s, 6.0, 12.0);
                if len.to_bits() != step.to_bits() {
                    faults.push(format!(
                        "{name} half {k} at {s}: measured {len}, the step is {step}"
                    ));
                }
                most = most.max(lookups);
                if Kind::of(&shape) == Kind::Lines {
                    sharp_steps += 1;
                    sharp_lookups += lookups;
                }
                s += 6.0;
            }
        }
    }
    let mean = sharp_lookups as f64 / sharp_steps.max(1) as f64;
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    assert!(
        sharp_steps > 100,
        "precondition: the sharp outlines' halves are walked"
    );
    assert!(
        most <= LEVEL_ROUNDS + 1,
        "a level step's measure took {most} path lookups, past LEVEL_ROUNDS + 1 = {}",
        LEVEL_ROUNDS + 1
    );
    assert!(
        mean < 2.5,
        "along the sharp outlines a level step's measure took {mean:.2} path lookups on the mean"
    );
}

/// R27's fit stops a line once it has passed its span's end (#309): no
/// step is negative, so the clusters after it would only confirm it. Over
/// the aspect sweep's frames laid `Fit`, each laid again with its fit
/// walking every cluster (`NO_FIT_STOP`): every glyph is placed where it
/// was, no frame takes more level steps with the stop than without it,
/// and the sweep takes fewer.
///
/// Control (run, not committed): the stop removed, red on "fewer".
#[test]
fn test_flow_r27s_fit_stops_a_line_past_its_spans_end() {
    use crate::flow::place::{NO_FIT_STOP, STEPS_TAKEN};
    struct On;
    impl Drop for On {
        fn drop(&mut self) {
            NO_FIT_STOP.with(|off| off.set(false));
        }
    }
    let bits = |placed: &crate::flow::PlacedRun| -> Vec<(u32, u32, u32)> {
        placed
            .glyphs
            .iter()
            .map(|g| (g.glyph, g.pos.x.to_bits(), g.pos.y.to_bits()))
            .collect()
    };
    let steps = || STEPS_TAKEN.with(std::cell::Cell::get);
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let (mut with, mut without) = (0, 0);
    for (name, shape, _, points) in sweep_outlines() {
        for text in [ECCLESIASTES, QUATRAIN] {
            for outward in [true, false] {
                let what = format!("{name} {text:.10}… outward {outward}");
                let before = steps();
                let (_, stopped, _) = frame_with(text, points, &shape, &fit, &RingLayout::whole(), outward);
                let on = steps() - before;
                let _reset = On;
                NO_FIT_STOP.with(|off| off.set(true));
                let before = steps();
                let (_, walked, _) = frame_with(text, points, &shape, &fit, &RingLayout::whole(), outward);
                let off = steps() - before;
                NO_FIT_STOP.with(|off| off.set(false));
                assert!(
                    bits(&stopped) == bits(&walked),
                    "{what}: placed otherwise with the stop"
                );
                assert!(on <= off, "{what}: {on} level steps with the stop, {off} without");
                (with, without) = (with + on, without + off);
            }
        }
    }
    assert!(
        with < without,
        "R27's fit stops no line past its span's end: {with} steps with the stop, {without} without"
    );
}
