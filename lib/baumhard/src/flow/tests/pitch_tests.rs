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
fn test_flow_pitched_keeps_one_ink_gap_in_every_direction() {
    do_flow_pitched_keeps_one_ink_gap_in_every_direction();
}

/// The distance between two boxes where they are apart, `0` where they
/// meet.
fn box_apart(a: &Rect, b: &Rect) -> f32 {
    (b.origin - a.max())
        .max(a.origin - b.max())
        .max(Vec2::ZERO)
        .length()
}

/// A pattern keeps one ink gap whichever way the path runs (rule 5, as
/// ruled on #326): a dash, its ink 4 wide and 1 tall with a natural gap
/// of 2, is laid 103 units across, down and on the diagonal, every two
/// neighbors' boxes the same distance apart. So its centers step 4 + 2
/// across and 1 + 2 down: across 17 dashes, `17·4 + 18·g = 103` at
/// `g = 1.944`, nearer 2 than 16 at 2.294; down 34, `34·1 + 35·g = 103`
/// at `g = 1.971`, nearer than 33 at 2.059.
pub fn do_flow_pitched_keeps_one_ink_gap_in_every_direction() {
    let mut run = dot();
    run.glyphs[0].ink = Some(Ink {
        min: Vec2::new(1.0, -3.0),
        max: Vec2::new(5.0, -2.0),
    });
    let laid = |to: Vec2| {
        let path = Curve::Line(Vec2::ZERO, to);
        let boxes = inks(&run, &place_pitched(&run, &path, 0.0, [NO_END; 2]));
        let gaps: Vec<f32> = boxes.windows(2).map(|p| box_apart(&p[0], &p[1])).collect();
        (boxes, gaps)
    };
    for (to, count, gap) in [
        (Vec2::new(103.0, 0.0), 17, 2.0 - 1.0 / 18.0),
        (Vec2::new(0.0, 103.0), 34, 2.0 - 1.0 / 35.0),
        (Vec2::new(103.0, 103.0) / 2.0_f32.sqrt(), 0, 0.0),
    ] {
        let (boxes, gaps) = laid(to);
        if count > 0 {
            assert_eq!(boxes.len(), count, "the nearer whole count toward {to}");
            close(gaps[0], gap, 1e-3, &format!("the gap toward {to}"));
        }
        for g in &gaps {
            close(*g, gaps[0], 1e-3, &format!("every ink gap toward {to}"));
        }
        assert!(
            (gaps[0] - 2.0).abs() < 0.2,
            "the gap toward {to} is the dash's own 2, and its share of the slack: {}",
            gaps[0]
        );
    }
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

/// On a path whose floats resolve the pattern's gaps, a round of the slack
/// that the lay does not answer does not stop the solve (#333): a curve
/// steps each ink's place by the float steps of the canvas, so a move of
/// the slack too small to cross one leaves the lay where it was, and a
/// later move, landing across one, does move it. Along a cubic arch some
/// 1 128 across and 314 high, the solve for 210 inks meets such a round
/// and goes on past it, to a slack at which the last ink lies within the
/// solve's tolerance, 1e-3, of where its end gap puts it. (The arch was
/// 1 120.6 across and 313.77 high, 208 inks, until its points were found
/// from its start in `f64` and rounded once (#313, ruling B18.1 §6),
/// which moved where the canvas's float steps fall: that arch's solve
/// meets no such round now.) A searched constant, not a derived one: the
/// arches 1 120.6 + 0.37·i across and 313.77 + 0.113·j high, i and j
/// below 40 and 100, were laid and this one taken of the 382 whose solve
/// met such a round. Nothing holds it there but the arithmetic as it
/// stands, so a later change of where the floats fall fails this test at
/// its precondition, that the solve met a round the lay did not answer,
/// not at its answer; search again then. Not every arch's solve that
/// meets such a round reaches its tolerance: a few of a grid of them fall
/// short of it by little more than it, which #358 takes up.
///
/// Control: the stop for a round the lay does not answer taken on every
/// path, not only where the path's float steps are wider than the
/// pattern's gaps, red: the solve stops at that round, its last ink
/// 1.8e-3 short ("210 inks laid at a slack of -0.010735823, the last
/// -0.0018310547 short of its end gap").
///
/// Not a `do_*` body: it reads what the solve reached through a
/// test-only record (`pitch::SOLVED`), which a bench build does not
/// compile.
#[test]
fn test_flow_pitched_solves_past_a_round_the_lay_does_not_answer() {
    use crate::flow::place::pitch::SOLVED;
    // The arch: how far across, and how high its control points stand.
    // Exact: the round the lay does not answer falls where the canvas's
    // float steps fall.
    let (across, high) = (1128.37_f32, 313.994_f32);
    let run = dot();
    let path = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(0.0, high),
        Vec2::new(across, high),
        Vec2::new(across, 0.0),
    );
    // A Bézier is no longer than its control polygon.
    let longest = across + 2.0 * high;
    assert!(
        2.0 * longest * f32::EPSILON < GAP,
        "precondition: the path's float steps, {} at the most, are narrower than the pattern's gap {GAP}",
        longest * f32::EPSILON
    );
    SOLVED.with(|solved| solved.set(None));
    let placed = place_pitched(&run, &path, 0.0, [NO_END; 2]);
    let solved = SOLVED.with(std::cell::Cell::get).expect("the pattern is laid");
    assert_eq!(
        placed.glyphs.len(),
        solved.count,
        "precondition: one glyph an ink"
    );
    assert!(
        solved.unanswered > 0,
        "precondition: the solve met a round the lay did not answer ({solved:?})"
    );
    assert!(
        solved.missed.abs() <= 1e-3,
        "{} inks laid at a slack of {}, the last {} short of its end gap",
        solved.count,
        solved.slack,
        solved.missed
    );
}

/// Three of [`dot`] in a row: three inks, each its own gap of
/// [`GAP`] from the next round the pattern.
fn dots() -> ShapedRun {
    let mut run = synthetic(&[ADVANCE; 3], 12.0);
    for glyph in &mut run.glyphs {
        glyph.letter = false;
        glyph.ink = Some(Ink {
            min: Vec2::new(2.0, -3.0),
            max: Vec2::new(2.0 + INK, -3.0 + INK),
        });
    }
    run
}

/// How far apart two points are, in `f64`: a length past 1.8 × 10¹⁹ has
/// no `f32` square.
fn far_apart(a: Vec2, b: Vec2) -> f64 {
    (f64::from(b.x) - f64::from(a.x)).hypot(f64::from(b.y) - f64::from(a.y))
}

#[test]
fn test_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly() {
    do_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly();
}

/// A gap past the root of the largest `f32` is laid as evenly as any
/// other (the case CI's fuzz run on #360 comes down to, beside
/// `test_flow_structured_every_face_finding_a_directional_ring_whose_gap_is_past_the_root_of_the_largest_f32`).
/// Along a slant — where two inks are apart across both axes, and their
/// gap is the distance between their corners — 3.12 × 10²⁰ long, a
/// pattern spaced 3 × 10¹⁹ fits ten gaps at 3 × 10²⁰ and eleven at
/// 3.3 × 10²⁰; ten needs the smaller change, so nine inks are laid, the
/// path's tenths apart and a tenth from each end. The inks' own 2 and
/// their natural gap of 4 are below the path's float steps there.
///
/// The gap's square was taken in `f32`, where it is infinite, and the lay
/// then set every ink at one place: at 59826cfd it lays its cap, 65,536
/// glyphs, all at the one point 3 × 10¹⁹ along the path, every one
/// finite, and says it was cut short.
///
/// Control: with the square taken in `f32` alone, red ("9 inks along the
/// slant", 65,536 laid).
pub fn do_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly() {
    let run = dot();
    let (length, spacing) = (3.12e20_f32, 3e19_f32);
    let to = Vec2::new(0.6, 0.8) * length;
    assert!(
        ((GAP + spacing) * (GAP + spacing)).is_infinite() && (0.1 * length * (0.1 * length)).is_infinite(),
        "precondition: neither the pattern's gap nor a tenth of the path has an `f32` square"
    );
    let placed = place_pitched(&run, &Curve::Line(Vec2::ZERO, to), spacing, [NO_END; 2]);
    let centers: Vec<Vec2> = inks(&run, &placed).iter().map(Rect::center).collect();
    assert_eq!(centers.len(), 9, "9 inks along the slant");
    assert!(!placed.truncated, "the path holds its nine whole");
    let tenth = f64::from(length) / 10.0;
    let stops: Vec<Vec2> = std::iter::once(Vec2::ZERO)
        .chain(centers.iter().copied())
        .chain(std::iter::once(to))
        .collect();
    for (k, pair) in stops.windows(2).enumerate() {
        let apart = far_apart(pair[0], pair[1]);
        assert!(
            (apart - tenth).abs() <= 1e-5 * tenth,
            "gap {k} of the ten, from the path's start to its end, is a tenth of it: {apart} against {tenth}"
        );
    }
}

#[test]
fn test_flow_pitched_lays_gaps_that_sum_past_the_largest_f32_by_their_mean() {
    do_flow_pitched_lays_gaps_that_sum_past_the_largest_f32_by_their_mean();
}

/// A pattern whose gaps sum past the largest `f32` is laid by their mean
/// all the same (found beside CI's fuzz finding on #360). Three inks
/// spaced 1.2 × 10³⁸ along 3 × 10³⁸: two gaps and an ink take 2.4 × 10³⁸
/// and three gaps and two inks 3.6 × 10³⁸; three need the smaller change,
/// so two inks are laid, at the path's thirds. And the pattern's own gap,
/// which the paths leaving a node's side are fanned by (`pattern_gap`),
/// is that mean.
///
/// The mean was the gaps' `f32` sum over their count, and the sum of
/// three such gaps is infinite: an end gap of infinity fits no ink, and
/// nothing was laid.
///
/// Controls: with the mean taken from the `f32` sum alone, red ("2 inks
/// at the path's thirds", none laid); with `pattern_gap` alone taking
/// it so, red ("the pattern's own gap", infinite).
pub fn do_flow_pitched_lays_gaps_that_sum_past_the_largest_f32_by_their_mean() {
    use crate::flow::pattern_gap;
    let run = dots();
    let (length, spacing) = (3e38_f32, 1.2e38_f32);
    let gap = GAP + spacing;
    assert!(
        gap.is_finite() && (3.0 * gap).is_infinite() && 2.0 * gap < length,
        "precondition: each gap is finite, the three sum past the largest `f32`, and the path holds two"
    );
    let path = Curve::Line(Vec2::ZERO, Vec2::new(length, 0.0));
    let placed = place_pitched(&run, &path, spacing, [NO_END; 2]);
    let centers: Vec<Vec2> = inks(&run, &placed).iter().map(Rect::center).collect();
    assert_eq!(centers.len(), 2, "2 inks at the path's thirds");
    for (k, center) in centers.iter().enumerate() {
        let third = length / 3.0 * (k + 1) as f32;
        assert!(
            (center.x - third).abs() <= 1e-5 * length && center.y.abs() <= 12.0,
            "ink {k} sits at a third of the path, on it: {center} against {third}"
        );
    }
    assert_eq!(
        pattern_gap(&run, spacing),
        Some((gap, INK)),
        "the pattern's own gap is the mean of its three, and its size its ink's"
    );
}

/// The gap between two boxes, in `f64`: their corners' distance where
/// they are apart across both axes, the wider axis's where across one,
/// below 0 where they overlap. A gap past 1.8 × 10¹⁹ has no `f32` square.
fn far_gap(a: &Rect, b: &Rect) -> f64 {
    let apart = (b.origin - a.max()).max(a.origin - b.max()).as_dvec2();
    if apart.x > 0.0 && apart.y > 0.0 {
        apart.length()
    } else {
        apart.max_element()
    }
}

#[test]
fn test_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly_where_the_path_bends() {
    do_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly_where_the_path_bends();
}

/// Where the path bends, an ink is set from the one before it by the gap
/// between their boxes as measured on the path (`signed_gap`), and that
/// gap, across both axes, is their corners' distance: the same square as
/// the slant's above, one function up (review 5911092160). A pattern
/// spaced 3 × 10¹⁹ along a cubic 10²⁵ across, and along two lines that
/// meet at a right angle, half that each, is held to its cap of 154 and
/// says so; and it is laid evenly: no two inks at one point, and every
/// gap between two neighbors' boxes within a thousandth of their mean.
///
/// Measured in `f32` alone the gap is infinite wherever two inks are
/// apart across both axes, and every ink was set where the last was: at
/// fe7061c9 all 154 sit at one point along the cubic.
///
/// Control: with `signed_gap` measuring in `f32` alone, red on the cubic
/// ("154 inks, each at a point of its own", 1).
pub fn do_flow_pitched_lays_a_gap_past_the_root_of_the_largest_f32_evenly_where_the_path_bends() {
    use crate::flow::place::pitch::place_pitched_capped;
    let run = dot();
    let (across, spacing, cap) = (1e25_f32, 3e19_f32, 154);
    assert!(
        ((GAP + spacing) * (GAP + spacing)).is_infinite(),
        "precondition: the pattern's gap has no `f32` square"
    );
    let cubic = [Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(0.0, 0.6 * across),
        Vec2::new(across, -0.3 * across),
        Vec2::new(across, 0.3 * across),
    )];
    let turn = Vec2::new(0.5 * across, 0.0);
    let corner = [
        Curve::Line(Vec2::ZERO, turn),
        Curve::Line(turn, Vec2::new(0.5 * across, 0.5 * across)),
    ];
    for (what, path) in [("the cubic", &cubic[..]), ("the two lines", &corner[..])] {
        let placed = place_pitched_capped(&run, path, spacing, [NO_END; 2], cap, None);
        let boxes = inks(&run, &placed);
        let mut places: Vec<(u32, u32)> = boxes
            .iter()
            .map(|b| (b.center().x.to_bits(), b.center().y.to_bits()))
            .collect();
        places.sort_unstable();
        places.dedup();
        assert_eq!(
            (boxes.len(), places.len()),
            (cap, cap),
            "along {what}: {cap} inks, each at a point of its own"
        );
        assert!(placed.truncated, "along {what}: a lay held to its cap says so");
        let gaps: Vec<f64> = boxes.windows(2).map(|pair| far_gap(&pair[0], &pair[1])).collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        assert!(
            mean > f64::from(spacing),
            "precondition: along {what} the gaps, {mean} on average, are the pattern's and its slack"
        );
        for (k, gap) in gaps.iter().enumerate() {
            assert!(
                (gap - mean).abs() <= 1e-3 * mean,
                "along {what}: gap {k} is {gap}, the mean {mean}"
            );
        }
    }
}

#[test]
fn test_flow_pitched_gaps_an_ink_wider_than_the_root_of_the_largest_f32() {
    do_flow_pitched_gaps_an_ink_wider_than_the_root_of_the_largest_f32();
}

/// The corners' distance takes the inks' own size squared as well as the
/// gap's, and an ink past 1.3 × 10¹⁹ across has no `f32` square either
/// (review 5911092160). No document makes such an ink — a glyph's is a
/// few times its font size — so this holds the arithmetic on a fixture:
/// one ink 4 × 10¹⁹ wide and 2 tall, spaced 6 × 10¹⁹, along a slant
/// 6 × 10²⁰ long. Its inks are apart across both axes, and every gap
/// between two neighbors' boxes is one gap, to a thousandth.
///
/// With the inks' size squared in `f32`, infinite, the square root was of
/// nothing and each ink was set its own reach along the slant from the
/// last, 2.4 × 10¹⁹ on: overlapping it.
///
/// Control: with the inks' size squared in `f32`, red ("two neighbors
/// apart across both axes").
pub fn do_flow_pitched_gaps_an_ink_wider_than_the_root_of_the_largest_f32() {
    let (wide, spacing, length) = (4e19_f32, 6e19_f32, 6e20_f32);
    let mut run = synthetic(&[wide], 12.0);
    let glyph = &mut run.glyphs[0];
    glyph.letter = false;
    glyph.ink = Some(Ink {
        min: Vec2::new(0.0, -3.0),
        max: Vec2::new(wide, -3.0 + INK),
    });
    assert!(
        (wide * wide).is_infinite() && (spacing * spacing).is_infinite(),
        "precondition: neither the ink's width nor its gap has an `f32` square"
    );
    let dir = Vec2::new(0.6, 0.8);
    let to = dir * length;
    let placed = place_pitched(&run, &Curve::Line(Vec2::ZERO, to), spacing, [NO_END; 2]);
    let boxes = inks(&run, &placed);
    assert!(
        boxes.len() >= 3,
        "precondition: the slant holds three inks or more ({})",
        boxes.len()
    );
    let gaps: Vec<f64> = boxes.windows(2).map(|pair| far_gap(&pair[0], &pair[1])).collect();
    for pair in boxes.windows(2) {
        let apart = pair[1].origin - pair[0].max();
        assert!(
            apart.x > 0.0 && apart.y > 0.0,
            "two neighbors apart across both axes: {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }
    let gap = gaps[0];
    assert!(
        gap >= f64::from(spacing) * 0.5,
        "precondition: the gap between two inks, {gap}, is the pattern's and its slack"
    );
    for other in &gaps {
        assert!(
            (other - gap).abs() <= 1e-3 * gap,
            "a gap between two inks is {other}, the first {gap}"
        );
    }
}

#[test]
fn test_flow_pitched_leaves_no_end_gap_below_none() {
    do_flow_pitched_leaves_no_end_gap_below_none();
}

/// The least slack a lay is solved to leaves no gap below 0, and an end's
/// gap is one of them: the mean of the pattern's gaps, which as `f32`
/// reckons it — their sum over their count — can read a float under the
/// least of them, where they are equal (review 5911092160 took the mean
/// for dead there; in exact arithmetic it would be). Three inks whose
/// gaps are each 748.39197: their sum rounds down, and the mean reads
/// 748.3919, a float under. Along a path a 32,768th shorter than one ink,
/// with ends an ink may reach back past, none is laid: a path too short
/// for an ink holds none at no gap at all.
///
/// With the least slack set by the least gap between two inks alone, each
/// end's gap may be a float under 0, a 16,384th here, and the ink is laid
/// over both of the path's ends.
///
/// Control: with the least slack the least of the gaps between inks, the
/// mean left out, red (an ink laid).
pub fn do_flow_pitched_leaves_no_end_gap_below_none() {
    use crate::flow::pattern_gap;
    let run = dots();
    let spacing = 744.391_97_f32;
    let gap = GAP + spacing;
    let (mean, _) = pattern_gap(&run, spacing).expect("the pattern has ink");
    assert!(
        mean.to_bits() + 1 == gap.to_bits(),
        "precondition: the mean of three gaps of {gap} reads {mean}, a float under them"
    );
    let short = INK - 2f32.powi(-15);
    assert!(
        short < INK && INK - short < gap - mean,
        "precondition: the path is shorter than an ink, by less than the mean reads under the gap"
    );
    let back = PathEnd {
        inks: &[],
        back: f32::INFINITY,
        hold: 0.0,
    };
    let placed = place_pitched(
        &run,
        &Curve::Line(Vec2::ZERO, Vec2::new(short, 0.0)),
        spacing,
        [back; 2],
    );
    assert!(
        placed.glyphs.is_empty(),
        "an ink laid along a path shorter than it: {:?}",
        placed.glyphs.iter().map(|g| g.pos).collect::<Vec<_>>()
    );
}
