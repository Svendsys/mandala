// SPDX-License-Identifier: MPL-2.0

//! A sided frame lays its sides far from the origin wherever the
//! coordinates can hold its glyphs apart (#362, ruling B18.1 §9), as it
//! lays them at the origin.
//!
//! Round an ellipse 2,000 across a frame's side arcs hold some two hundred
//! copies of their ornament. Each arc searched for its count from a guess
//! the straight path gives, some thirty counts off there, and moved a count
//! at a time, each count a solve of its gap; far out each solve lays the
//! copies some ten times, and each copy's letters read their path up to
//! twenty times, so the search spent the frame's budget before it ended
//! and left both arcs empty, at 10⁶ where an `f32` is 0.0625 apart. The
//! search now moves to the count the gaps solved point to, and the
//! pairwise rule ends where its bracket's centers set an ink on
//! neighboring `f32`s. Further out, from 10⁷ where an `f32` is 1 apart,
//! three more things left arcs empty that the origin fills, each a length
//! held finer than the coordinates resolve: the copies are now held apart
//! from the text on the boxes the lay sets them at, as the text's are;
//! their least gap is no less than holds the first apart from the text ink
//! it starts past; and the frame's band curve is held to the offset its
//! inks want no finer than the spacing of the `f32`s they are set at
//! (`flow::place::halves`).
//!
//! Every test here lays frames far out, which no benchmark measures, or
//! reads the count search through its test seam, so none has a `do_*`
//! twin.

use glam::Vec2;

use crate::document::values::{FitMode, LineSpan, Rgba, Shape};
use crate::flow::place::Trace;
use crate::flow::tests::flow_fuzz::{coordinates_hold_apart, ornament_run, outline_reach};
use crate::flow::tests::halves_tests::frame_with;
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, RingLayout};

/// The size every frame here is laid at, in points.
const POINTS: f32 = 16.0;

/// The side ornament every frame here is laid with.
const SIDES: &str = "─═";

/// The issue's two texts.
const SAND: &str = "To see a World in a Grain of Sand";
const MANDALA: &str = "MANDALA";

/// A frame's layout: its lines at their natural width, [`SIDES`] round
/// its junctions.
fn layout() -> RingLayout {
    RingLayout {
        span: LineSpan::NATURAL,
        sides: Some((SIDES.to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    }
}

/// An ellipse `size` across, centered at `(at, at)`.
fn ellipse(size: Vec2, at: f32) -> Outline {
    outline(Rect::new(Vec2::splat(at) - size * 0.5, size), Shape::Ellipse)
}

/// The fit mode whose least scale the coordinates of a frame laid in `fit`
/// are held to (ruling B18.1 §9): its own, but for `Once`, which owes no
/// text whole and so has none of its own in the harness's condition, and
/// lays its text at scale 1, a `Spread`'s least.
fn held_as(fit: &FitMode) -> FitMode {
    match fit {
        FitMode::Once { .. } => FitMode::Spread,
        other => other.clone(),
    }
}

/// Whether the coordinates `outline` is drawn in can hold apart the glyphs
/// of a frame of `text` laid round it in `fit` with [`layout`]: its text's
/// and its ornament's (`coordinates_hold_apart`, ruling B18.1 §9).
fn coordinates_hold(text: &str, outline: &Outline, fit: &FitMode) -> bool {
    let fit = held_as(fit);
    with_fonts(|fs| {
        let flow = style(POINTS);
        let run = real_run(text, &flow, fs);
        let ornament = ornament_run(&layout(), &flow, 1.0, fs).expect("the layout has sides");
        let reach = outline_reach(outline);
        coordinates_hold_apart(reach, &run, &fit) && coordinates_hold_apart(reach, &ornament, &fit)
    })
}

/// A frame as laid: its glyphs, its sides' among them, and its trace.
struct Laid {
    placed: PlacedRun,
    sides: usize,
    trace: Trace,
}

/// `text` laid round `outline` in `fit` with [`layout`], outward.
fn laid(text: &str, outline: &Outline, fit: &FitMode) -> Laid {
    let (_, placed, trace) = frame_with(text, POINTS, outline, fit, &layout(), true);
    let lines: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
    Laid {
        sides: placed.glyphs.len() - lines,
        placed,
        trace,
    }
}

/// Whether `laid` lays its sides as owed: some copies, and no junction
/// left empty or kept short.
fn lays_its_sides(laid: &Laid) -> bool {
    laid.sides > 0 && laid.trace.unheld.sides.is_empty()
}

/// The issue's frames (#362): each with its fit mode and the ellipse it is
/// laid round, 2,000 across. "MANDALA" in `Fit` round the ellipse 2,000 ×
/// 500 lays its sides at 10⁶ and 10⁷ on the epic before #362 (255 glyphs at
/// the origin, 253 at 10⁶, 243 at 10⁷), so the issue's "either" aspect is
/// the 0.6 one alone here.
fn issue_frames() -> Vec<(&'static str, FitMode, Vec2)> {
    let wide = Vec2::new(2_000.0, 1_200.0);
    let thin = Vec2::new(2_000.0, 500.0);
    vec![
        (SAND, FitMode::Spread, wide),
        (SAND, FitMode::Once { filler: None }, wide),
        (MANDALA, FitMode::Fit { min: 0.5, max: 2.0 }, wide),
        (MANDALA, FitMode::Spread, thin),
        (MANDALA, FitMode::Once { filler: None }, thin),
    ]
}

/// #362: the issue's frames lay their sides at the origin, at 10⁶ and at
/// 10⁷, where an `f32` is 0.0625 and 1 apart and the coordinates hold
/// their glyphs apart (precondition, ruling B18.1 §9): some copies, and
/// no junction left empty or kept short. Before #362 every one of them
/// left both junctions empty at both (32, 7 and 7 glyphs).
///
/// Control: with the count search moving a count at a time, as before
/// #362, red at 10⁶: "To see…" in `Spread` keeps an arc short, 396,660 of
/// its 400,000 glyph steps spent.
#[test]
fn test_far_sides_the_issues_frames_lay_their_sides_at_1e6_and_1e7() {
    for (text, fit, size) in issue_frames() {
        let origin = laid(text, &ellipse(size, 0.0), &fit);
        assert!(
            lays_its_sides(&origin),
            "precondition: {text:?} {fit:?} round the ellipse {size} lays its sides at the origin ({} glyphs, {} of its sides, {:?})",
            origin.placed.glyphs.len(),
            origin.sides,
            origin.trace.unheld.sides
        );
        for at in [1e6, 1e7] {
            let shape = ellipse(size, at);
            assert!(
                coordinates_hold(text, &shape, &fit),
                "precondition: the coordinates at {at:e} hold the glyphs of {text:?} {fit:?} apart"
            );
            let far = laid(text, &shape, &fit);
            assert!(
                lays_its_sides(&far),
                "{text:?} {fit:?} round the ellipse {size} at {at:e} does not lay its sides as at the origin: {} glyphs, {} of its sides ({} at the origin), {:?}, its budget {} of {} spent",
                far.placed.glyphs.len(),
                far.sides,
                origin.sides,
                far.trace.unheld.sides,
                far.trace.spent,
                far.trace.budget
            );
        }
    }
}

/// The spread's sizes: ellipses about 2,000 across.
const SIZES: [f32; 3] = [1_800.0, 2_000.0, 2_200.0];

/// The spread's aspects, height over width: the issue's two.
const ASPECTS: [f32; 2] = [0.6, 0.25];

/// The centers each frame of the spread is laid at, past the origin, for
/// `text` in `fit`: to the last power of two of `f32` spacing at which its
/// coordinates hold its glyphs apart — its text's and its ornament's at
/// the least scale `fit` lays them at — where one power of two further,
/// twice as far, they do not ([`beyond`]). At 16 pt, [`SAND`] in `Spread`
/// to `f32`s 1 apart, from 2²³ to 2²⁴, and in `Fit {0.5, 2}`, whose least
/// is half that size, to 0.5 apart, from 2²² to 2²³; [`MANDALA`] a power
/// of two further in each.
fn centers(text: &str, fit: &FitMode) -> &'static [f32] {
    match (text == SAND, matches!(fit, FitMode::Fit { .. })) {
        (true, true) => &[1e5, 1e6, 4e6, 8e6],
        (true, false) | (false, true) => &[1e5, 1e6, 4e6, 1e7, 1.6e7],
        (false, false) => &[1e5, 1e6, 4e6, 1e7, 1.6e7, 3e7],
    }
}

/// Twice the last of the [`centers`] of `text` in `fit`: one power of two
/// of `f32` spacing further, where the coordinates no longer hold its
/// glyphs apart, so the spread runs to the last center at which they do.
fn beyond(text: &str, fit: &FitMode) -> f32 {
    2.0 * centers(text, fit).last().copied().unwrap_or(0.0)
}

/// The spread's frames for `text` in `fit`: an ellipse of each size of
/// [`SIZES`] across, each aspect of [`ASPECTS`], wide and tall.
fn spread(text: &'static str, fit: &FitMode) -> Vec<(&'static str, FitMode, Vec2)> {
    let mut frames = Vec::new();
    for size in SIZES {
        for aspect in ASPECTS {
            for wide in [true, false] {
                let across = if wide {
                    Vec2::new(size, size * aspect)
                } else {
                    Vec2::new(size * aspect, size)
                };
                frames.push((text, fit.clone(), across));
            }
        }
    }
    frames
}

/// #362: every frame of the spread for `text` in `fit` lays its sides at
/// the origin (precondition), and at each of its [`centers`], where its
/// coordinates hold its glyphs apart (precondition) — some copies, and no
/// junction left empty or kept short — and they do not one power of two
/// further (precondition: the spread runs to the last center at which they
/// do). Returns how many far frames it held, which its callers hold to the
/// spread's whole count.
fn holds_the_spread(text: &'static str, fit: &FitMode) -> usize {
    let mut held = 0;
    let mut broken = Vec::new();
    for (text, fit, size) in spread(text, fit) {
        let origin = laid(text, &ellipse(size, 0.0), &fit);
        assert!(
            lays_its_sides(&origin),
            "precondition: {text:?} {fit:?} round the ellipse {size} lays its sides at the origin ({:?})",
            origin.trace.unheld.sides
        );
        assert!(
            !coordinates_hold(text, &ellipse(size, beyond(text, &fit)), &fit),
            "precondition: the coordinates at {:e} do not hold the glyphs of {text:?} {fit:?} apart",
            beyond(text, &fit)
        );
        for &at in centers(text, &fit) {
            let shape = ellipse(size, at);
            assert!(
                coordinates_hold(text, &shape, &fit),
                "precondition: the coordinates at {at:e} hold the glyphs of {text:?} {fit:?} round the ellipse {size} apart"
            );
            let far = laid(text, &shape, &fit);
            held += 1;
            if !lays_its_sides(&far) {
                broken.push(format!(
                    "{text:?} {fit:?} round the ellipse {size} at {at:e}: {} glyphs, {} of its sides ({} at the origin), {:?}, its budget {} of {} spent",
                    far.placed.glyphs.len(),
                    far.sides,
                    origin.sides,
                    far.trace.unheld.sides,
                    far.trace.spent,
                    far.trace.budget
                ));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {held} far frames do not lay their sides as at the origin:\n{}",
        broken.len(),
        broken.join("\n")
    );
    held
}

/// #362: [`holds_the_spread`] for [`SAND`] in `Spread`: 12 frames, each
/// at 5 centers.
///
/// Controls: with the pairwise rule closing its bracket to a float of the
/// center, the spread of [`SAND`] in `Spread` red at 4·10⁶, two frames
/// with an arc left empty and their budgets spent; with no floor holding
/// the first copy apart from the text (`halves::held_apart`), red at 10⁷ and
/// 1.6·10⁷, three frames round the tall ellipses 450 × 1,800 and 500 ×
/// 2,000 with an arc left empty, their copies touching the text. With one
/// center too many, red on its precondition; with the spread's `beyond` its
/// last center, red on that precondition.
#[test]
fn test_far_sides_sand_in_spread_lays_its_sides_wherever_its_coordinates_hold_them() {
    assert_eq!(holds_the_spread(SAND, &FitMode::Spread), 12 * 5);
}

/// #362: [`holds_the_spread`] for [`SAND`] in `Fit {0.5, 2}`: 12 frames,
/// each at 4 centers.
///
/// Controls: see [`test_far_sides_sand_in_spread_lays_its_sides_wherever_its_coordinates_hold_them`].
#[test]
fn test_far_sides_sand_in_fit_lays_its_sides_wherever_its_coordinates_hold_them() {
    assert_eq!(
        holds_the_spread(SAND, &FitMode::Fit { min: 0.5, max: 2.0 }),
        12 * 4
    );
}

/// #362: [`holds_the_spread`] for [`MANDALA`] in `Spread`: 12 frames,
/// each at 6 centers.
///
/// Controls: see [`test_far_sides_sand_in_spread_lays_its_sides_wherever_its_coordinates_hold_them`].
#[test]
fn test_far_sides_mandala_in_spread_lays_its_sides_wherever_its_coordinates_hold_them() {
    assert_eq!(holds_the_spread(MANDALA, &FitMode::Spread), 12 * 6);
}

/// #362: [`holds_the_spread`] for [`MANDALA`] in `Fit {0.5, 2}`: 12
/// frames, each at 5 centers.
///
/// Controls: see [`test_far_sides_sand_in_spread_lays_its_sides_wherever_its_coordinates_hold_them`].
#[test]
fn test_far_sides_mandala_in_fit_lays_its_sides_wherever_its_coordinates_hold_them() {
    assert_eq!(
        holds_the_spread(MANDALA, &FitMode::Fit { min: 0.5, max: 2.0 }),
        12 * 5
    );
}

/// The gap at which `k` copies, each `copy` long along a straight path
/// and of `inked` inked clusters, fill `length` of it: the room their
/// lengths leave, shared among their gaps — one before each ink and the
/// join to the end.
fn straight_gap(length: f64, copy: f64, inked: f64, k: usize) -> f64 {
    (length - k as f64 * copy) / (k as f64 * inked + 1.0)
}

/// The most copies, of the `fit` that fit, whose gap along that straight
/// path reaches `target`: found by trying every count.
fn most_reaching(length: f64, copy: f64, inked: f64, target: f64, fit: usize) -> usize {
    (1..=fit)
        .filter(|&k| straight_gap(length, copy, inked, k) >= target)
        .max()
        .unwrap_or(0)
}

/// The paths the count search's tests read: each length, copy length and
/// target of a side arc, a count of copies near 200 reaching the target.
const PATHS: [(f64, f64, f64); 4] = [
    (1_500.0, 6.2, 0.643_5),
    (1_497.0, 6.19, 0.643_5),
    (2_400.0, 9.7, 1.31),
    (980.0, 4.4, 0.26),
];

/// #362: past two counts solved, the count search reads where the target
/// lies from the arc's length shared among copies of one length and their
/// gaps (`next_count`): on a straight path, which that holds exactly, the
/// count it tries next is the most whose gap reaches the target — found
/// here by trying every count — from any two counts either side of it or
/// both on one side, inside the bracket the counts solved close.
///
/// Control: with the copies' length read from one count's gaps alone
/// (`gaps(n)` for both), red: along 1,500, counts 227 and 217 solved, the
/// search tries 201 where 200 copies reach the target.
#[test]
fn test_far_sides_the_count_search_reads_the_count_from_two_solved() {
    use crate::flow::place::halves::next_count_seam;
    let inked = 2.0;
    let fit = 256;
    let mut held = 0;
    for (length, copy, target) in PATHS {
        let gap = |k: usize| straight_gap(length, copy, inked, k) as f32;
        let want = most_reaching(length, copy, inked, target, fit);
        assert!(
            (150..=230).contains(&want),
            "precondition: about 200 copies reach the target along {length} ({want})"
        );
        for (m, n) in [
            (227, 217),
            (120, 180),
            (240, 160),
            (want - 10, want + 10),
            (want + 3, want + 1),
        ] {
            let (reached, fell) = match (gap(m) >= target as f32, gap(n) >= target as f32) {
                (true, true) => (m.max(n), fit + 1),
                (false, false) => (0, m.min(n)),
                (true, false) => (m, n),
                (false, true) => (n, m),
            };
            if fell == reached + 1 {
                continue;
            }
            let (next, _) = next_count_seam(
                (n, gap(n)),
                Some((m, gap(m))),
                target as f32,
                (reached, fell, fit),
                (usize::MAX, 0),
                inked as f32,
                |_| f32::NAN,
            );
            let within = if gap(n) < target as f32 {
                (reached + 1)..=(n - 1)
            } else {
                (n + 1)..=(fell - 1)
            };
            assert_eq!(
                next,
                want.clamp(*within.start(), *within.end()),
                "along {length}, counts {m} and {n} solved, the bracket {reached} to {fell}: the search tries {next}, where {want} copies reach the target"
            );
            held += 1;
        }
    }
    assert_eq!(
        held, 20,
        "precondition: the search is read at five brackets along each path"
    );
}

/// #362: from one count solved, the count search reads where the target
/// lies from the straight path's gaps less the amount the solved gap falls
/// short of its own: where the bend shifts every count's gap alike, the
/// most whose gap reaches the target, inside the bracket.
///
/// Control: with the straight path's gaps read as they are, red: along
/// 1,500, count 256 solved, the search tries 231 where 200 copies reach the
/// target.
#[test]
fn test_far_sides_the_count_search_reads_the_count_from_one_solved() {
    use crate::flow::place::halves::next_count_seam;
    let inked = 2.0;
    let fit = 256;
    for (length, copy, target) in PATHS {
        let gap = |k: usize| straight_gap(length, copy, inked, k) as f32;
        let want = most_reaching(length, copy, inked, target, fit);
        // The straight path's gaps, the bend's half a point off each.
        let straight = |k: usize| gap(k) + 0.5;
        for n in [fit, want + 25, want + 1] {
            let (next, _) = next_count_seam(
                (n, gap(n)),
                None,
                target as f32,
                (0, n, fit),
                (usize::MAX, 0),
                inked as f32,
                straight,
            );
            assert_eq!(
                next,
                want.min(n - 1),
                "along {length}, count {n} solved: the search tries {next}, where {want} copies reach the target"
            );
        }
    }
}

/// #362: a bracket closed at both ends that has not halved over two counts
/// tried is bisected, whatever the gaps point to: the count search tries
/// at most about three counts a halving, however the gaps mislead it. A
/// bracket that has halved, or one count tried since it did, is read from
/// the gaps; and a bracket bisected counts its halving afresh.
///
/// Control: with the bisection removed, red, the count tried 199, where
/// the gaps point, not 150.
#[test]
fn test_far_sides_the_count_search_bisects_a_bracket_that_does_not_halve() {
    use crate::flow::place::halves::next_count_seam;
    let fit = 256;
    // Gaps that point at the bracket's far end each time: 199 copies reach
    // the target along the straight path read, wherever the search stands.
    let straight = |k: usize| if k <= 199 { 1.0 } else { 0.0 };
    let solve = |halved: (usize, usize)| {
        next_count_seam((200, 0.5), None, 0.75, (100, 200, fit), halved, 2.0, straight)
    };
    assert_eq!(
        solve((100, 1)),
        (150, (100, 0)),
        "the bracket 100 to 200, as wide as when it last halved and one count tried since, is bisected"
    );
    assert_eq!(
        solve((100, 0)),
        (199, (100, 1)),
        "the bracket 100 to 200, as wide as when it last halved and no count tried since, is read from the gaps"
    );
    assert_eq!(
        solve((200, 1)),
        (199, (100, 0)),
        "the bracket 100 to 200, half as wide as when it last halved, is read from the gaps and its halving counted afresh"
    );
}

/// The tight spread's sizes: ellipses small and large, whose junctions
/// hold a copy or a few at a span that leaves the lines little slack.
const TIGHT_SIZES: [f32; 5] = [150.0, 300.0, 600.0, 1_200.0, 2_400.0];

/// The tight spread's aspects, height over width.
const TIGHT_ASPECTS: [f32; 3] = [1.0, 0.6, 0.3];

/// The tight spread's centers: from `10⁶` to `3.3·10⁷`, each side of a
/// power of two of `f32` spacing; each frame held at those where its
/// coordinates hold its glyphs apart.
const TIGHT_CENTERS: [f32; 8] = [1.0e6, 4.1e6, 8.3e6, 1.0e7, 1.2e7, 1.6e7, 3.0e7, 3.3e7];

/// #362: every frame of the tight spread at `span` — each text in `Spread`
/// and `Fit {0.5, 2}`, round an ellipse of each of [`TIGHT_SIZES`] and
/// [`TIGHT_ASPECTS`] — that lays its sides at the origin lays them at each
/// of [`TIGHT_CENTERS`] where its coordinates hold its glyphs apart: some
/// copies, and no junction left empty or kept short. Returns how many far
/// frames it held.
fn holds_the_tight_spread(span: LineSpan) -> usize {
    let layout = RingLayout { span, ..layout() };
    let lay = |text: &str, shape: &Outline, fit: &FitMode| {
        let (_, placed, trace) = frame_with(text, POINTS, shape, fit, &layout, true);
        let lines: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
        Laid {
            sides: placed.glyphs.len() - lines,
            placed,
            trace,
        }
    };
    let mut held = 0;
    let mut broken = Vec::new();
    for text in [SAND, MANDALA] {
        for fit in [FitMode::Spread, FitMode::Fit { min: 0.5, max: 2.0 }] {
            for size in TIGHT_SIZES {
                for aspect in TIGHT_ASPECTS {
                    let across = Vec2::new(size, size * aspect);
                    let origin = lay(text, &ellipse(across, 0.0), &fit);
                    if !lays_its_sides(&origin) {
                        continue;
                    }
                    for at in TIGHT_CENTERS {
                        let shape = ellipse(across, at);
                        if !coordinates_hold(text, &shape, &fit) {
                            continue;
                        }
                        let far = lay(text, &shape, &fit);
                        held += 1;
                        if !lays_its_sides(&far) {
                            broken.push(format!(
                                "{text:?} {fit:?} round the ellipse {across} at {at:e}: {} glyphs, {} of its sides ({} at the origin), {:?}, its budget {} of {} spent",
                                far.placed.glyphs.len(),
                                far.sides,
                                origin.sides,
                                far.trace.unheld.sides,
                                far.trace.spent,
                                far.trace.budget
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {held} far frames at span {span:?} do not lay their sides as at the origin:\n{}",
        broken.len(),
        broken.join("\n")
    );
    held
}

/// #362: [`holds_the_tight_spread`] with the lines over their whole
/// halves, where the sides fill the junctions' breaks alone: 340 far
/// frames or more of the 480 it may lay, 345 as it stands.
///
/// Control: with the band curve held to a hundredth of a point wherever it
/// lies (`Halves::offset_tolerance`), red: four frames of "MANDALA" round
/// ellipses 150 to 2,400 across, at 10⁷ to 3.3·10⁷, both arcs emptied as
/// wanting more of the curve than it was last laid at, little of their
/// budgets spent.
#[test]
fn test_far_sides_a_frame_at_whole_lays_its_sides_wherever_its_coordinates_hold_them() {
    let held = holds_the_tight_spread(LineSpan::WHOLE);
    assert!(
        held >= 340,
        "precondition: the spread holds 340 far frames or more ({held})"
    );
}

/// #362: [`holds_the_tight_spread`] with the lines over half their halves'
/// slack: 340 far frames or more of the 480 it may lay, 345 as it stands.
///
/// Control: as for
/// [`test_far_sides_a_frame_at_whole_lays_its_sides_wherever_its_coordinates_hold_them`],
/// red: two frames of "MANDALA" in `Fit` at 8.3·10⁶.
#[test]
fn test_far_sides_a_frame_at_half_span_lays_its_sides_wherever_its_coordinates_hold_them() {
    let held = holds_the_tight_spread(LineSpan::new(0.5).expect("a half is a span"));
    assert!(
        held >= 340,
        "precondition: the spread holds 340 far frames or more ({held})"
    );
}

/// #362: a larger budget leaves no arc empty that a smaller one keeps
/// copies on: "To see a World in a Grain of Sand" in `Spread` round the
/// ellipse 2,000 × 1,200 at 10⁶, at budgets from 40,000 to 70,000 in steps
/// of 1,000. A count search the budget stops keeps, of the counts it solved
/// whole, the one whose gap is nearest the target, where it kept the count
/// it stood at: with more budget the search reaches further before the
/// stop, onto a count whose solve the stop cut short, and kept nothing.
/// Precondition: some budget keeps an arc short, and so stops its search.
///
/// Control: with the search keeping the count it stands at, red at 53,000
/// to 55,000, both junctions left empty, where 46,000 to 52,000 keep the
/// first short.
#[test]
fn test_far_sides_a_larger_budget_keeps_the_copies_a_smaller_one_does() {
    use crate::flow::place::halves::BUDGET_OVERRIDE;
    use crate::flow::place::SideLoss;
    use crate::flow::tests::halves_tests::with_seam;
    let shape = ellipse(Vec2::new(2_000.0, 1_200.0), 1e6);
    let fit = FitMode::Spread;
    let mut held: [Option<usize>; 2] = [None; 2];
    let mut short = 0;
    let mut broken = Vec::new();
    for budget in (40_000..=70_000).step_by(1_000) {
        let frame = with_seam(&BUDGET_OVERRIDE, budget, || laid(SAND, &shape, &fit));
        let losses = &frame.trace.unheld.sides;
        short += losses.iter().filter(|loss| loss.1 == SideLoss::Short).count();
        for (j, kept) in held.iter_mut().enumerate() {
            let empty = losses.contains(&(j, SideLoss::Empty));
            match (*kept, empty) {
                (Some(at), true) => broken.push(format!(
                    "at a budget of {budget} junction {j} is left empty ({losses:?}), where {at} kept copies there"
                )),
                (None, false) if frame.sides > 0 => *kept = Some(budget),
                _ => {}
            }
        }
    }
    assert!(
        short > 0,
        "precondition: some budget from 40,000 to 70,000 keeps an arc short"
    );
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}
