// SPDX-License-Identifier: MPL-2.0

//! The exact work removals of #309, each held by its own counter against
//! a figure computed apart from the code it counts (review 5810838863):
//! revert a removal and its pin goes red, whatever else lands on top.

use std::collections::BTreeSet;

use crate::document::values::{FitMode, LineSpan, Rgba};
use crate::flow::place::{frame_halves, level_step_seam, LEVEL_ROUNDS, MIN_TABLE_SPACING};
use crate::flow::tests::halves_tests::{frame_with, sweep_outlines, QUATRAIN};
use crate::flow::tests::work_tests::{Counters, Kind};
use crate::flow::{Outline, RingLayout};

/// The Ecclesiastes center frame's text.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

/// The frames of the aspect sweep the census lays
/// ([`census`](crate::flow::tests::work_tests::census)): each outline,
/// both texts, outward and inward, `Fit` and `Once`, at three spans, and
/// outward with sides too.
fn sweep_frames() -> Vec<(String, Outline, &'static str, f32, FitMode, RingLayout, bool)> {
    let mut frames = Vec::new();
    for (name, shape, _, points) in sweep_outlines() {
        for text in [ECCLESIASTES, QUATRAIN] {
            for outward in [true, false] {
                for fit in [
                    FitMode::Fit { min: 0.5, max: 2.0 },
                    FitMode::Once { filler: None },
                ] {
                    let sides_tried = if outward { 2 } else { 1 };
                    for sides in [None, Some(("·".to_owned(), Rgba::opaque(9, 9, 9)))]
                        .into_iter()
                        .take(sides_tried)
                    {
                        for span in [
                            LineSpan::WHOLE,
                            LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
                            LineSpan::NATURAL,
                        ] {
                            let layout = RingLayout {
                                span,
                                sides: sides.clone(),
                                ..RingLayout::whole()
                            };
                            frames.push((name.clone(), shape, text, points, fit.clone(), layout, outward));
                        }
                    }
                }
            }
        }
    }
    frames
}

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

/// A frame reads each distinct glyph's outline once an inking (#309):
/// a glyph's ink is a function of its face, id, size and the rest of
/// what `inked` keys it by, so a letter the text repeats reads the ink its
/// first read did. Over the census' frames without sides, laid `Once`: the
/// outlines read are the inkings times the run's distinct glyphs that are
/// not blank, by face, id and size, counted from the shaped run itself.
///
/// Control (run, not committed): `inked`'s reads not kept, red ("37
/// outlines read over 1 inkings of 16 distinct glyphs", round the circle).
#[test]
fn test_flow_a_frame_reads_each_distinct_glyphs_outline_once_an_inking() {
    let mut frames = 0;
    for (name, shape, text, points, fit, layout, outward) in sweep_frames() {
        if layout.sides.is_some() || !matches!(fit, FitMode::Once { .. }) {
            continue;
        }
        let before = Counters::now();
        let (run, _, _) = frame_with(text, points, &shape, &fit, &layout, outward);
        let work = Counters::now().since(before);
        let distinct: BTreeSet<_> = run
            .layout_glyphs()
            .zip(&run.glyphs)
            .filter(|(_, g)| !g.blank)
            .map(|(laid, _)| (laid.font_id, laid.glyph_id, laid.font_size.to_bits()))
            .collect();
        assert!(work.inkings > 0, "precondition, {name}: the frame inks its text");
        assert_eq!(
            work.outline_reads,
            work.inkings * distinct.len(),
            "{name} {text:.10}…: {} outlines read over {} inkings of {} distinct glyphs",
            work.outline_reads,
            work.inkings,
            distinct.len()
        );
        frames += 1;
    }
    assert!(frames > 50, "precondition: {frames} frames laid");
}

/// A frame's [`TrackMemo`](crate::flow::place::halves) answers a line it
/// laid as that line, to the bit (#309): over the aspect sweep's frames,
/// some lines are read from it, and every frame that reads one, laid again
/// with the memo answering none (`NO_TRACK_MEMO`), places every glyph
/// where it did, and lays a track for each line it had read. A line read
/// counts the steps of the track it stands in for, as `track` counts them
/// (`TRACKED_STEPS`), so the frame's work reads as it did without the memo
/// (counting a read as what it is is ruling B16.1c's).
///
/// Over the sweep, 841 lines read in 138 frames.
///
/// Controls (run, not committed): the memo answering none, red on the
/// precondition; its key without the gap, red on "placed otherwise"; a
/// line read charged nothing, red on its work.
#[test]
fn test_flow_a_frames_remembered_lines_are_the_lines_and_cost_what_they_are() {
    use crate::flow::place::halves::{NO_TRACK_MEMO, TRACKED_STEPS};
    struct On;
    impl Drop for On {
        fn drop(&mut self) {
            NO_TRACK_MEMO.with(|off| off.set(false));
        }
    }
    let bits = |placed: &crate::flow::PlacedRun| -> Vec<(u32, u32, u32)> {
        placed
            .glyphs
            .iter()
            .map(|g| (g.glyph, g.pos.x.to_bits(), g.pos.y.to_bits()))
            .collect()
    };
    let tracked = || TRACKED_STEPS.with(std::cell::Cell::get);
    let (mut frames, mut hits) = (0, 0);
    for (name, shape, text, points, fit, layout, outward) in sweep_frames() {
        let what = format!("{name} {text:.10}… {fit:?} outward {outward}");
        let (before, steps) = (Counters::now(), tracked());
        let (_, with, t_on) = frame_with(text, points, &shape, &fit, &layout, outward);
        let (on, steps_on) = (Counters::now().since(before), tracked() - steps);
        if on.track_hits == 0 {
            continue;
        }
        let _reset = On;
        NO_TRACK_MEMO.with(|off| off.set(true));
        let (before, steps) = (Counters::now(), tracked());
        let (_, without, t_off) = frame_with(text, points, &shape, &fit, &layout, outward);
        let (off, steps_off) = (Counters::now().since(before), tracked() - steps);
        NO_TRACK_MEMO.with(|off| off.set(false));
        assert!(
            bits(&with) == bits(&without),
            "{what}: placed otherwise with the memo"
        );
        assert_eq!(
            off.tracks,
            on.tracks + on.track_hits,
            "{what}: {} tracks without the memo, {} with it and {} lines read",
            off.tracks,
            on.tracks,
            on.track_hits
        );
        assert_eq!(off.track_hits, 0, "{what}: the memo answered while switched off");
        let read = steps_off - steps_on;
        assert_eq!(
            t_on.spent, t_off.spent,
            "{what}: {} glyph steps with the memo, {} without it, its {} lines' tracks {read}",
            t_on.spent, t_off.spent, on.track_hits
        );
        frames += 1;
        hits += on.track_hits;
    }
    assert!(
        frames > 0 && hits > 0,
        "precondition: the sweep's frames read lines from their memo ({hits} in {frames} frames)"
    );
}
