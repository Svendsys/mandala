// SPDX-License-Identifier: MPL-2.0

//! Whole glyphs between two ends (#296, rule 5): the count is whole and
//! the nearer of the two that fit, every gap between two inks the same,
//! the ends' too, one pitch whichever way the path runs, and each end's
//! gap measured to the boxes beside it. The pattern is synthetic — one
//! glyph, advance 6, its ink a 2 × 2 square 2 in from its pen — so its
//! own gap is `6 − 4 + 2 = 4` and every expected position is a closed
//! form of those numbers.

use glam::Vec2;

use crate::flow::shape::Ink;
use crate::flow::tests::support::{close, synthetic};
use crate::flow::{place_pitched, Curve, PathEnd, PlacedRun, Rect, ShapedRun};

/// The fixture's advance, ink square and the gap they leave.
const ADVANCE: f32 = 6.0;
const INK: f32 = 2.0;
const GAP: f32 = ADVANCE - INK;

/// A one-glyph ornament: advance 6, ink the 2 × 2 square 2 in from its
/// pen, 3 above its baseline.
fn dot() -> ShapedRun {
    let mut run = synthetic(&[ADVANCE], 12.0);
    let glyph = &mut run.glyphs[0];
    glyph.letter = false;
    glyph.ink = Some(Ink {
        min: Vec2::new(2.0, -3.0),
        max: Vec2::new(2.0 + INK, -3.0 + INK),
    });
    run
}

/// The canvas box of each placed glyph's ink, by the run's metrics: the
/// pen sits the run's baseline below the glyph's box.
fn inks(run: &ShapedRun, placed: &PlacedRun) -> Vec<Rect> {
    placed
        .glyphs
        .iter()
        .map(|g| {
            let ink = run.glyphs[g.glyph as usize]
                .ink
                .expect("the fixture's glyph is inked");
            let pen = g.pos + Vec2::new(0.0, run.baseline());
            Rect::new(pen + ink.min, ink.max - ink.min)
        })
        .collect()
}

/// The gap between two boxes on the axis they are apart on.
fn apart(a: &Rect, b: &Rect) -> f32 {
    let d = (b.origin - a.max()).max(a.origin - b.max());
    d.max_element()
}

const NO_END: PathEnd<'static> = PathEnd {
    inks: &[],
    back: 0.0,
    hold: 0.0,
};

#[test]
fn test_flow_pitched_holds_the_nearer_whole_count_evenly() {
    do_flow_pitched_holds_the_nearer_whole_count_evenly();
}

/// Along 103 units 16 inks fit at their own gap, taking 100 with both end
/// gaps; 17 take 106. 17 needs the smaller change, −3 over 18 gaps against
/// +3 over 17, so 17 are laid, every gap `4 − 1/6`, the two at the ends
/// from the path's ends.
pub fn do_flow_pitched_holds_the_nearer_whole_count_evenly() {
    let run = dot();
    let placed = place_pitched(
        &run,
        &Curve::Line(Vec2::ZERO, Vec2::new(103.0, 0.0)),
        0.0,
        [NO_END; 2],
    );
    let boxes = inks(&run, &placed);
    assert_eq!(
        boxes.len(),
        17,
        "the nearer whole count: 17 (−3 over 18 gaps), not 16 (+3 over 17)"
    );
    let gap = GAP - 3.0 / 18.0;
    close(
        boxes[0].origin.x,
        gap,
        1e-3,
        "the start's gap, from the path's start",
    );
    close(
        103.0 - boxes[16].max().x,
        gap,
        1e-3,
        "the end's gap, to the path's end",
    );
    for pair in boxes.windows(2) {
        close(apart(&pair[0], &pair[1]), gap, 1e-3, "every gap between two inks");
    }
    for b in &boxes {
        close(b.center().y, 0.0, 1e-3, "every ink centered on the path");
    }
}

#[test]
fn test_flow_pitched_keeps_one_pitch_in_every_direction() {
    do_flow_pitched_keeps_one_pitch_in_every_direction();
}

/// A pattern keeps one pitch whichever way the path runs (rule 5): a
/// dash, its ink 4 wide and 1 tall, is laid across and down 103 units as
/// the same 17 inks, their centers the same distance apart — not stacked
/// by its height down the path, as a box step would set them (§6.2), at
/// 1 + 2 a step where across it takes 4 + 2.
pub fn do_flow_pitched_keeps_one_pitch_in_every_direction() {
    let mut run = dot();
    run.glyphs[0].ink = Some(Ink {
        min: Vec2::new(1.0, -3.0),
        max: Vec2::new(5.0, -2.0),
    });
    let pitch = |to: Vec2| {
        let placed = place_pitched(&run, &Curve::Line(Vec2::ZERO, to), 0.0, [NO_END; 2]);
        let boxes = inks(&run, &placed);
        let steps: Vec<f32> = boxes
            .windows(2)
            .map(|p| p[0].center().distance(p[1].center()))
            .collect();
        (boxes.len(), steps.iter().sum::<f32>() / steps.len() as f32)
    };
    let across = pitch(Vec2::new(103.0, 0.0));
    let down = pitch(Vec2::new(0.0, 103.0));
    assert_eq!(
        across.0, 17,
        "across, 17 dashes: 16 take 98 at their own gap of 2, 17 take 104"
    );
    assert_eq!(down.0, across.0, "as many dashes down as across");
    close(
        down.1,
        across.1,
        1e-3,
        "the pitch down the path, against across it",
    );
    close(
        across.1,
        6.0 - 1.0 / 18.0,
        1e-3,
        "the pitch across: a dash and a gap, less a share of the 1 too many",
    );
}

#[test]
fn test_flow_pitched_measures_each_end_to_the_boxes_beside_it() {
    do_flow_pitched_measures_each_end_to_the_boxes_beside_it();
}

/// The boxes at each end lie off the path's ends: at the start 10 before
/// it, at the end 5 past it. The end glyphs follow them, their inks back
/// past the start and on past the end as far as `back` allows, so each
/// end's gap to its box is the gap between the inks. Held to `back: 0`,
/// the start glyph's ink comes back to the path's start and no further:
/// 10 from its box, more than the gap it would keep.
pub fn do_flow_pitched_measures_each_end_to_the_boxes_beside_it() {
    let run = dot();
    let path = Curve::Line(Vec2::ZERO, Vec2::new(103.0, 0.0));
    let start = [Rect::new(Vec2::new(-30.0, -10.0), Vec2::new(20.0, 20.0))];
    let end = [Rect::new(Vec2::new(108.0, -10.0), Vec2::new(20.0, 20.0))];
    let laid = |back: f32| {
        let ends = [
            PathEnd {
                inks: &start,
                back,
                hold: 0.0,
            },
            PathEnd {
                inks: &end,
                back: 5.0,
                hold: 0.0,
            },
        ];
        inks(&run, &place_pitched(&run, &path, 0.0, ends))
    };
    let boxes = laid(10.0);
    let inner: Vec<f32> = boxes.windows(2).map(|p| apart(&p[0], &p[1])).collect();
    let mean = inner.iter().sum::<f32>() / inner.len() as f32;
    for g in &inner {
        close(*g, mean, 1e-2, "every gap between two inks");
    }
    close(
        apart(&start[0], &boxes[0]),
        mean,
        0.02 * mean,
        "the start's gap, to the box behind the path's start",
    );
    close(
        apart(&boxes[boxes.len() - 1], &end[0]),
        mean,
        0.02 * mean,
        "the end's gap, to the box past the path's end",
    );
    let held = laid(0.0);
    assert!(
        mean < 9.0,
        "precondition: the gap {mean} is less than the 10 the box lies back, so holding shows"
    );
    close(
        apart(&start[0], &held[0]),
        10.0,
        0.02,
        "held to the path's start, the start glyph's ink reaches the start and no further: the box's 10 back",
    );
}

#[test]
fn test_flow_pitched_measures_the_far_end_of_a_long_path() {
    do_flow_pitched_measures_the_far_end_of_a_long_path();
}

/// A path some 26 000 long holds thousands of inks, and its far end's gap
/// is measured to the box past it as a short path's is: within 2% of the
/// gap the inks are laid at, at every one of twenty lengths. Laid by
/// thousands of `f32` steps, the far end drifted by whole points, and a
/// hair's move of the end moved its glyph by one, so the measure never
/// settled (#296: `stress_long_edges`' long links).
pub fn do_flow_pitched_measures_the_far_end_of_a_long_path() {
    let run = dot();
    for k in 0..20 {
        let length = 26_000.0 + 37.3 * k as f32;
        let path = Curve::Line(Vec2::ZERO, Vec2::new(length, 0.0));
        let end = [Rect::new(Vec2::new(length + 1.0, -10.0), Vec2::new(20.0, 20.0))];
        let ends = [
            NO_END,
            PathEnd {
                inks: &end,
                back: 5.0,
                hold: 0.0,
            },
        ];
        let boxes = inks(&run, &place_pitched(&run, &path, 0.0, ends));
        assert!(
            boxes.len() > 4000,
            "precondition: thousands of inks ({})",
            boxes.len()
        );
        let near: Vec<f32> = boxes[boxes.len() - 20..]
            .windows(2)
            .map(|p| apart(&p[0], &p[1]))
            .collect();
        let mean = near.iter().sum::<f32>() / near.len() as f32;
        close(
            apart(&boxes[boxes.len() - 1], &end[0]),
            mean,
            0.02 * mean,
            &format!("the far end's gap on a path {length} long"),
        );
    }
}

#[test]
fn test_flow_pitched_lays_no_text_and_no_empty_path() {
    do_flow_pitched_lays_no_text_and_no_empty_path();
}

/// Text has no ink to space by, and a path of no length no room: neither
/// is laid. The same path laid with the inked fixture holds glyphs, so
/// each emptiness is the case's own.
pub fn do_flow_pitched_lays_no_text_and_no_empty_path() {
    let line = Curve::Line(Vec2::ZERO, Vec2::new(103.0, 0.0));
    assert!(
        !place_pitched(&dot(), &line, 0.0, [NO_END; 2]).glyphs.is_empty(),
        "precondition: the dot is laid"
    );
    let text = synthetic(&[ADVANCE, ADVANCE], 12.0);
    assert!(
        place_pitched(&text, &line, 0.0, [NO_END; 2]).glyphs.is_empty(),
        "text is not laid"
    );
    let point = Curve::Line(Vec2::ZERO, Vec2::ZERO);
    assert!(
        place_pitched(&dot(), &point, 0.0, [NO_END; 2]).glyphs.is_empty(),
        "a path of no length holds nothing"
    );
}
