// SPDX-License-Identifier: MPL-2.0

//! A `Repeat` ring's work budget (#310, `flow::place::ring`): what it
//! takes past its budget, what it holds back for phase 2, how a ring its
//! budget or its cap stopped is reported, and the margin its budget
//! leaves ordinary rings. Every test here sets a test seam or reads a
//! test counter, so none has a `do_*` twin.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
use crate::flow::place::ring::{phase_two_steps, ring_overshoot, RingLoss};
use crate::flow::place::{place_on_closed_capped, Trace, LAST_RING, RING_CROSSED, RING_LATE};
use crate::flow::tests::halves_tests::with_seam;
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, RingLayout, MAX_PLACED_GLYPHS};

/// The size the grid's rings are laid at.
const POINTS: f32 = 16.0;

/// What the grid's rings are of: a text, which phase 2 lays again in
/// reading order, and an ornament — a filler's dot — which it lays as
/// walked, each glyph set by its ink.
const CONTENTS: [(&str, &str); 2] = [("letters", "MANDALA "), ("ornament", "· ")]; // a dot and a space

/// The budgets each ring of the grid is laid with, in glyph steps: from
/// spent before its first glyph to most of its default, so that each of
/// a ring's walks — the first, its seam's measures and solve, the lay of
/// it and phase 2's solves — finds it spent somewhere in the grid.
const BUDGETS: [usize; 14] = [
    1, 500, 1_000, 2_000, 4_000, 8_000, 16_000, 24_000, 32_000, 48_000, 64_000, 128_000, 256_000, 350_000,
];

/// The cap every ring of the grid is laid with (ruling B17.2a), past the
/// glyphs of every one of its outlines but those 10²² across, which hold
/// the engine's cap's of any copy: their rings, at their own budget, run
/// to the cap and stop, short — at the engine's cap, 65,536 glyphs, some
/// 25 s each in a debug build, a value the cap's arithmetic proves
/// (ruling B17.2c), not one the grid needs laid.
const GRID_CAP: usize = 2_048;

/// The caps each ring of the grid is laid with besides, at its own
/// budget (ruling B17.2a): below a copy, a few copies, and most of the
/// glyphs of the outlines at the origin, so that the cap stops a ring's
/// first walk, and its cut of a last copy, somewhere in the grid.
const CAPS: [usize; 4] = [1, 3, 64, 1_024];

/// The budgets the grid's long copy is laid with (ruling B17.1b): every
/// fifth glyph step about where its cut of a copy longer than its outline
/// runs out of budget, between 880 and 900 glyph steps, its every probe
/// found spent before one fit below.
const FINE: std::ops::RangeInclusive<usize> = 700..=1_100;

/// The budgets review 5800041728 found the long bidi copy's cut kept a
/// part copy out of reading order at, between [`FINE`]'s fifths.
const REVIEW_BUDGETS: [usize; 2] = [861, 867];

/// The grid's outlines: an ellipse, a sharp rectangle and a rounded one,
/// each 2,000 across at the origin, 2,000 across 10⁷ from it — where an
/// `f32` sets a point to the nearest unit — and 10²² across, where the
/// floats of a point some 10²² from the origin set an ornament's glyphs
/// apart by bisection that barely moves them (the ring of seed 2771).
fn outlines() -> Vec<(String, bool, Outline)> {
    let shapes = [
        ("ellipse", Shape::Ellipse),
        (
            "sharp rectangle",
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        (
            "rounded rectangle",
            Shape::Rectangle {
                radius: Percent::new(30),
            },
        ),
    ];
    let places = [
        ("2,000 at the origin", Vec2::ZERO, 2_000.0, false),
        ("2,000 at 10⁷", Vec2::splat(1e7), 2_000.0, false),
        ("10²² across", Vec2::ZERO, 1e22, true),
    ];
    let mut out = Vec::new();
    for (shape_name, shape) in shapes {
        for (place_name, at, size, far) in places {
            out.push((
                format!("{shape_name} {place_name}"),
                far,
                outline(Rect::new(at, Vec2::splat(size)), shape),
            ));
        }
    }
    out
}

/// One ring of the grid as laid.
struct Laid {
    what: String,
    /// The glyph steps it took past its budget ([`PAST_BUDGET`]), all it
    /// took, and its budget.
    past: usize,
    spent: usize,
    budget: usize,
    /// Its glyphs a copy, what bounds a cluster's, and the glyphs its
    /// budget was scaled by.
    copy: usize,
    glyphs: usize,
    /// How far past what its budget left after its hold for phase 2 it
    /// was when a walk first found it spent (`RING_CROSSED`).
    crossed: Option<usize>,
    /// The probes and rounds of phase 2 it began with its budget spent
    /// (`RING_LATE`).
    late: usize,
    /// What was held back for phase 2 when it began, whether the ring had
    /// closed then (`RingWork::held_again`), and whether its copy has a
    /// letter in it, which phase 2 lays in reading order.
    held: (usize, bool),
    letters: bool,
    loss: Option<RingLoss>,
    placed: PlacedRun,
}

/// `text` laid as a `Repeat` ring round `shape`, `outward` or not, at
/// most `cap` glyphs, with `budget` in place of its own when given.
fn ring(text: &str, shape: &Outline, outward: bool, budget: Option<usize>, cap: usize) -> Laid {
    let lay = || {
        with_fonts(|fs| {
            let flow = style(POINTS);
            let run = real_run(text, &flow, fs);
            let mut trace = Trace::default();
            PAST_BUDGET.with(|past| past.set(0));
            let (_, placed) = place_on_closed_capped(
                &run,
                shape,
                StartPoint {
                    edge: Edge::Top,
                    at: 0.0,
                },
                Direction::Clockwise,
                &FitMode::Repeat,
                &RingLayout::whole(),
                outward,
                &flow,
                fs,
                cap,
                Some(&mut trace),
            );
            Laid {
                what: String::new(),
                past: PAST_BUDGET.with(std::cell::Cell::get),
                spent: trace.spent,
                budget: trace.budget,
                copy: run.glyphs.len(),
                glyphs: trace.glyphs,
                crossed: RING_CROSSED.with(std::cell::Cell::get),
                late: RING_LATE.with(std::cell::Cell::get),
                held: LAST_RING
                    .with(std::cell::Cell::get)
                    .expect("a ring round a closed outline is recorded")
                    .held,
                letters: run.glyphs.iter().any(|glyph| glyph.letter),
                loss: trace.ring,
                placed,
            }
        })
    };
    match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    }
}

/// One case of the grid: a ring laid at each of its budgets and caps,
/// the last at its own, whether its outline is one of those 10²²
/// across, and its text.
struct Case {
    far: bool,
    text: String,
    rings: Vec<Laid>,
}

/// Every ring of the grid: each of [`CONTENTS`] round each of
/// [`outlines`], outward and inward, at [`GRID_CAP`] and each of
/// [`BUDGETS`], at its own budget and each of [`CAPS`], and last at its
/// own — and the long copies ([`long_copies`]). Laid once, by whichever test
/// asks first.
fn grid() -> &'static [Case] {
    static GRID: std::sync::OnceLock<Vec<Case>> = std::sync::OnceLock::new();
    GRID.get_or_init(lay_grid)
}

/// The copies longer than their outline (ruling B17.1b): "MANDALA "
/// sixty times, 480 glyphs, and a text of Latin and Hebrew words sixty
/// times, 480 glyphs, whose visual order is not its reading order (review
/// 5800041728) — each round [`long_copy_outline`], a ring too short for
/// one copy, whose walk cuts the copy to the longest prefix in reading
/// order that fits, by a binary search of lays that its budget reads
/// before each cluster of.
fn long_copies() -> [(&'static str, String); 2] {
    [
        ("a long copy", "MANDALA ".repeat(60)),
        ("a long bidi copy", "abc אבג ".repeat(60)),
    ]
}

/// The outline the long copies are laid round: an ellipse 300 across at
/// the origin.
fn long_copy_outline() -> Outline {
    outline(Rect::new(Vec2::ZERO, Vec2::splat(300.0)), Shape::Ellipse)
}

/// One case's rings: `text` round `shape`, `outward` or not, at
/// [`GRID_CAP`] and each of `budgets`, then at its own budget and each of
/// [`CAPS`], then at its own.
fn lay_case(what: &str, text: &str, shape: &Outline, outward: bool, budgets: &[usize]) -> Vec<Laid> {
    let named = |mut laid: Laid, how: String| {
        laid.what = format!("{what}, {how}");
        laid
    };
    let mut rings: Vec<Laid> = budgets
        .iter()
        .map(|&b| {
            named(
                ring(text, shape, outward, Some(b), GRID_CAP),
                format!("budget {b}"),
            )
        })
        .collect();
    rings.extend(
        CAPS.iter()
            .map(|&cap| named(ring(text, shape, outward, None, cap), format!("cap {cap}"))),
    );
    rings.push(named(
        ring(text, shape, outward, None, GRID_CAP),
        "its own budget".to_string(),
    ));
    rings
}

/// [`grid`], laid.
fn lay_grid() -> Vec<Case> {
    let mut cases = Vec::new();
    for (content, text) in CONTENTS {
        for (name, far, shape) in outlines() {
            for outward in [true, false] {
                let what = format!(
                    "{content} round the {name}, {}",
                    if outward { "outward" } else { "inward" }
                );
                cases.push(Case {
                    far,
                    text: text.to_string(),
                    rings: lay_case(&what, text, &shape, outward, &BUDGETS),
                });
            }
        }
    }
    for (k, (what, text)) in long_copies().into_iter().enumerate() {
        let shape = long_copy_outline();
        // The bidi copy at the two budgets review 5800041728 found its cut
        // out of reading order at, too.
        let review: &[usize] = if k == 1 { &REVIEW_BUDGETS } else { &[] };
        let fine: Vec<usize> = FINE.step_by(5).chain(review.iter().copied()).collect();
        cases.push(Case {
            far: false,
            rings: lay_case(
                &format!("{what} round a 300 ellipse, outward"),
                &text,
                &shape,
                true,
                &fine,
            ),
            text,
        });
    }
    cases
}

/// The case of the grid laid of `text`.
fn case_of(text: &str) -> &'static Case {
    grid()
        .iter()
        .find(|case| case.text == text)
        .expect("the grid lays every text it names")
}

/// #310: a ring reads its budget before every cluster its walks lay or
/// measure and before every probe of phase 2's solves, and holds back
/// what phase 2 lays whatever is spent, so past its budget it takes no
/// more than its setup and the one cluster that found it spent
/// (`ring_overshoot`) — `PAST_BUDGET`, a take that crosses the budget
/// counted for the part past it (ruling R50.4) — whatever its content,
/// its outline, how far it lies from the origin, its direction, its cap
/// and where in its work the budget runs out. Its walks stop within its
/// setup and a cluster of what the budget leaves after the hold
/// (crossed); phase 2 begins no probe of its solves, and no round of its
/// regrouping, once the budget is spent (late, ruling B17.1d); and a ring
/// that reports nothing is laid as at its own budget and the grid's cap —
/// a ring whose own is lossy included (ruling B17.1c): never capped
/// silently, by its budget or its cap (ruling B17.2a).
///
/// Controls: with the budget read only between a seam's measures, not
/// before each body they walk, red (crossed); with nothing held back for
/// phase 2, or its run on alone not held back, red (`PAST_BUDGET`: 90
/// rings, all 10²² across, for the run on); with phase 2's solves, or
/// `even_out`'s rounds, not reading the budget, red (late, 10 rings each)
/// — and with them not reading it and the whole run on held through phase
/// 2 as before, still red (late, 8 rings), where `PAST_BUDGET` was green;
/// with the first walk's stop, or its stop at the cap, not reported, red
/// (silent: 121 rings for the first walk's, 84 of them 10²² across, which
/// a check of only the rings whose own reports nothing does not see).
#[test]
fn test_flow_ring_budget_a_ring_past_its_budget_takes_its_overshoot_at_most() {
    let cases = grid();
    let rings: Vec<&Laid> = cases.iter().flat_map(|case| &case.rings).collect();
    let stopped = rings.iter().filter(|r| r.crossed.is_some()).count();
    assert!(
        stopped * 4 >= rings.len(),
        "precondition: a quarter of the rings or more have a walk stopped by their budget ({stopped} of {})",
        rings.len()
    );
    let closed = rings.iter().filter(|r| r.held.1).count();
    assert!(
        closed * 8 >= rings.len(),
        "precondition: an eighth of the rings or more close ({closed} of {})",
        rings.len()
    );
    let mut broken = Vec::new();
    for case in cases {
        let own = case
            .rings
            .last()
            .expect("every case lays its ring at its own budget");
        assert!(
            case.far || own.glyphs < GRID_CAP,
            "precondition: {} scales its budget by {} glyphs, within the grid's cap",
            own.what,
            own.glyphs
        );
        for laid in &case.rings {
            let allowed = ring_overshoot(laid.copy);
            if laid.past > allowed || laid.spent > laid.budget + allowed {
                broken.push(format!(
                    "{}: {} past its budget of {}, {} in all, against {allowed} allowed past it",
                    laid.what, laid.past, laid.budget, laid.spent
                ));
            }
            if laid.crossed.is_some_and(|crossed| crossed > allowed) {
                broken.push(format!(
                    "{}: a walk stopped {:?} glyph steps past what its budget left, against its setup and a cluster's {allowed} (crossed)",
                    laid.what, laid.crossed
                ));
            }
            if laid.late > 0 {
                broken.push(format!(
                    "{}: phase 2 began {} probes or rounds with its budget spent (late)",
                    laid.what, laid.late
                ));
            }
            let (held, closed) = laid.held;
            let owed = phase_two_steps(laid.placed.glyphs.len(), laid.letters);
            if closed && held != owed {
                broken.push(format!(
                    "{}: closed, it held back {held} glyph steps for phase 2, where its work on its glyphs is {owed} and it runs on not at all (held)",
                    laid.what
                ));
            }
            if laid.loss.is_none() && (own.loss.is_some() || laid.placed != own.placed) {
                broken.push(format!(
                    "{}: reported nothing, and laid otherwise than at its own budget, {:?} (silent)",
                    laid.what, own.loss
                ));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} rings past the bound:\n{}",
        broken.len(),
        rings.len(),
        broken.join("\n")
    );
}

/// Ruling B17.1b: a ring too short for one copy cuts the copy to the
/// longest prefix that fits, and a cut its budget, not its path, stops
/// says so — at every budget about where its search runs out, those
/// whose every probe found it spent before one fit included, which kept
/// no cluster and reported nothing (review 5761697746: 44 of the budgets
/// from 17,147 to 17,190, where the hold kept its whole run on then) —
/// and so does a cut its cap stops.
///
/// Precondition: the row's budgets lie on both sides of where its search
/// runs out — some ring keeps no cluster, some all of its own — and some
/// cap stops the cut. Control: with `lay_or_cut` returning its final
/// lay's stop alone, red (8 rings).
#[test]
fn test_flow_ring_budget_a_copy_longer_than_its_outline_cut_short_says_so() {
    let case = case_of(&long_copies()[0].1);
    let own = case
        .rings
        .last()
        .expect("every case lays its ring at its own budget");
    assert!(
        own.loss.is_none() && own.placed.truncated && !own.placed.glyphs.is_empty() && own.placed.glyphs.len() < own.copy,
        "precondition: at its own budget the long copy is cut to a prefix, and nothing is lost ({}, {} glyphs of {})",
        own.what,
        own.placed.glyphs.len(),
        own.copy
    );
    let fine = || case.rings.iter().filter(|r| r.what.contains(", budget "));
    assert!(
        fine().any(|r| r.placed.glyphs.is_empty()) && fine().any(|r| r.placed == own.placed),
        "precondition: the row's budgets lie on both sides of where the cut search runs out"
    );
    assert!(
        case.rings
            .iter()
            .any(|r| r.what.contains("cap") && r.placed.glyphs.len() < own.placed.glyphs.len()),
        "precondition: a cap stops the cut"
    );
    let silent: Vec<&str> = case
        .rings
        .iter()
        .filter(|r| r.loss.is_none() && r.placed != own.placed)
        .map(|r| r.what.as_str())
        .collect();
    assert!(
        silent.is_empty(),
        "{} rings cut short by their budget or their cap reported nothing:\n{}",
        silent.len(),
        silent.join("\n")
    );
}

/// #310: a ring whose budget stops its work says how it was left
/// (`Trace::ring`), each way somewhere in the grid: short, its first walk
/// stopped before its copies filled the outline; unclosed, its seam's
/// measures or its lay stopped; uneven, phase 2's solves stopped — and,
/// at budgets that afford it, not at all.
///
/// Controls: with each of the three left unreported, red (that one).
#[test]
fn test_flow_ring_budget_a_spent_ring_says_how_it_was_left() {
    let rings: Vec<&Laid> = grid().iter().flat_map(|case| &case.rings).collect();
    for loss in [
        None,
        Some(RingLoss::Short),
        Some(RingLoss::Unclosed),
        Some(RingLoss::Uneven),
    ] {
        assert!(
            rings.iter().any(|r| r.loss == loss),
            "no ring of the grid's {} is left {loss:?}",
            rings.len()
        );
    }
}

/// The rings of the margin sweep (ruling B17.1a): a box-drawing border, a
/// dot and a text, round the outlines review 5761697746 found ordinary
/// rings ran out on and smaller ones, from 1,000 across to 200,000; and
/// two ornaments of sparse ink, a bar alone and a bar, a space and a dot
/// (review 5800041728) — set by their ink, which is narrow beside their
/// advance, so their steps are shorter than the advance their glyphs are
/// estimated by, and each glyph works more. The review measured `✦ ·`,
/// the ornament UNIFIED_ARCHITECTURE §6.2 names; no compiled face held
/// `✦` then, so it was shaped by whatever face a host had (§T14), and the
/// bar is the compiled faces' glyph of the same kind: ink a fifth of its
/// advance. The compiled ornament face holds `✦` since #297; the sweep
/// keeps the bar its numbers were measured on.
const SWEEP: [(&str, &[f32]); 5] = [
    ("─═", &[1_000.0, 5_000.0, 10_000.0, 15_000.0, 20_000.0]),
    ("· ", &[1_000.0, 5_000.0, 15_000.0, 20_000.0]),
    ("MANDALA ", &[1_000.0, 10_000.0, 100_000.0, 200_000.0]),
    ("|", &[5_000.0, 10_000.0, 20_000.0]),
    ("| ·", &[5_000.0, 10_000.0, 20_000.0]),
];

/// Ruling B17.1a: a ring's budget scales with the glyphs it will lay, as
/// a frame's with its text (ruling R44.2), so every ordinary ring of the
/// sweep — letters and ornaments round ellipses and squares at the origin,
/// outward and inward, the engine's cap — closes at its own budget, and
/// `STEPS_PER_GLYPH` is at least five times the most steps a glyph any of
/// them takes: the margin R47.1 pins for frames. A ring's glyphs are the
/// fewer of those it placed and those its budget was scaled by, so that
/// neither an estimate past what it lays nor one short of it hides a
/// ring's cost. 6.57 times, at this head: `| ·` round an ellipse 10,000
/// across, outward, 456.8 steps a glyph (the review's `✦ ·` there, in its
/// host's faces, 5.60).
///
/// Control: with the budget scaled by one copy's glyphs, as before, red:
/// twelve rings of the sweep — `─═` round the ellipses 10,000 to 20,000
/// across and `· ` round three of its outlines unclosed, as review
/// 5761697746 found, `MANDALA ` round the 100,000 square uneven and round
/// the 200,000 outlines short.
#[test]
fn test_flow_ring_budget_ordinary_rings_close_within_a_fifth_of_their_budget_a_glyph() {
    use crate::flow::place::halves::STEPS_PER_GLYPH;
    let shapes = [
        ("ellipse", Shape::Ellipse),
        (
            "square",
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
    ];
    let mut worst = (0.0_f64, String::new());
    let mut open = Vec::new();
    for (text, sizes) in SWEEP {
        for &size in sizes {
            for (shape_name, shape) in &shapes {
                for outward in [true, false] {
                    let outline = outline(Rect::new(Vec2::ZERO, Vec2::splat(size)), *shape);
                    let laid = ring(text, &outline, outward, None, MAX_PLACED_GLYPHS);
                    let what = format!(
                        "{text:?} round a {size} {shape_name}, {}",
                        if outward { "outward" } else { "inward" }
                    );
                    if laid.loss.is_some() || laid.placed.truncated {
                        open.push(format!(
                            "{what}: {:?}, truncated {}",
                            laid.loss, laid.placed.truncated
                        ));
                    }
                    let glyphs = laid.placed.glyphs.len().min(laid.glyphs).max(1);
                    let per = laid.spent as f64 / glyphs as f64;
                    if per > worst.0 {
                        worst = (per, what);
                    }
                }
            }
        }
    }
    assert!(
        open.is_empty(),
        "{} rings not closed at their own budget:\n{}",
        open.len(),
        open.join("\n")
    );
    let ratio = STEPS_PER_GLYPH as f64 / worst.0;
    assert!(
        ratio >= 5.0,
        "STEPS_PER_GLYPH is {ratio:.2} times the sweep's worst, {:.1} steps a glyph ({})",
        worst.0,
        worst.1
    );
}

/// #310: a spacing solve whose doubling reaches a gap as long as the
/// whole path, the walk ending no further on than at the probe before,
/// stops there: the far-out case, where no gap moves a step. A miss that
/// stands still for every gap — a ring whose glyphs no `f32` sets apart
/// — is looked at once for each doubling to the path's length, not for
/// all of the doubling's rounds; a miss that stands still only below
/// the path's floats' grain, and moves past it, is solved as before.
///
/// Controls: without the stop, the still miss looks 49 times; with it
/// taken before the path's length, the grained miss stops at 0.
#[test]
fn test_flow_ring_budget_a_solve_no_gap_moves_stops_at_the_path_s_length() {
    use crate::flow::place::solve_gap_seam;
    let length = 1_000.0;
    let (gap, looks) = solve_gap_seam(|_| -5.0, length, 1.0);
    assert_eq!(
        gap, 512.0,
        "a still miss keeps the largest gap it doubled to short of the path's length"
    );
    assert!(
        looks <= 12,
        "a miss no gap moves is looked at {looks} times: once at no gap and once a doubling to the path's length"
    );
    // A miss whose walk moves only once its gap passes the floats' grain
    // (a hundredth here), then grows ten a unit of gap: solved at ½.
    let grained = |e: f32| if e < 0.01 { -5.0 } else { 10.0 * e - 5.0 };
    let (gap, _) = solve_gap_seam(grained, length, 1e-4);
    assert!(
        (gap - 0.5).abs() < 0.01 && grained(gap) <= 0.0,
        "a miss that moves past the floats' grain is solved: {gap}"
    );
}

/// Review 5800041728: a cut of a copy longer than its outline keeps the
/// longest prefix of the copy in reading order (ruling R9.4), at every
/// budget about where its budget runs out — those whose whole lay of the
/// copy the budget stopped part way included, which kept what it had
/// reached in visual order: for a text of Latin and Hebrew words, not a
/// prefix in reading order (13 and 37 glyphs, at budgets 861 and 867,
/// [`REVIEW_BUDGETS`], and 29 at 865) — and at every cap.
///
/// Precondition: the row's budgets lie on both sides of where the cut
/// runs out, and some cut keeps glyphs of both directions. Control: with
/// `lay_or_cut`'s whole lay keeping what it reached when the budget stops
/// it, red (3 cuts).
#[test]
fn test_flow_ring_budget_a_cut_short_keeps_a_prefix_in_reading_order() {
    let text = &long_copies()[1].1;
    let case = case_of(text);
    let run = with_fonts(|fs| real_run(text, &style(POINTS), fs));
    // Each glyph's place in reading order: its span, and where its
    // cluster starts in the span's text.
    let reading = |index: u32| {
        let glyph = &run.glyphs[index as usize];
        (glyph.span, glyph.bytes.start)
    };
    let mut broken = Vec::new();
    for laid in &case.rings {
        let kept: std::collections::BTreeSet<u32> = laid.placed.glyphs.iter().map(|g| g.glyph).collect();
        let last_kept = kept.iter().map(|&g| reading(g)).max();
        let first_dropped = (0..run.glyphs.len() as u32)
            .filter(|g| !kept.contains(g))
            .map(reading)
            .min();
        if let (Some(last), Some(first)) = (last_kept, first_dropped) {
            if first < last {
                broken.push(format!(
                    "{}: kept {} glyphs, one read after one it dropped (loss {:?})",
                    laid.what,
                    kept.len(),
                    laid.loss
                ));
            }
        }
    }
    let fine = || case.rings.iter().filter(|r| r.what.contains(", budget "));
    let own = case
        .rings
        .last()
        .expect("every case lays its ring at its own budget");
    assert!(
        fine().any(|r| r.placed.glyphs.is_empty()) && fine().any(|r| r.placed == own.placed),
        "precondition: the row's budgets lie on both sides of where the cut runs out"
    );
    let hebrew = |g: &crate::flow::PlacedGlyph| {
        text[run.glyphs[g.glyph as usize].bytes.start..]
            .starts_with(|c: char| ('\u{5D0}'..='\u{5EA}').contains(&c))
    };
    assert!(
        own.placed.glyphs.iter().any(hebrew) && !own.placed.glyphs.iter().all(hebrew),
        "precondition: the cut keeps glyphs of both directions"
    );
    assert!(
        broken.is_empty(),
        "{} cuts are not a prefix in reading order:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// The budget the run on is pinned at (ruling B17.1d): within the
/// [`RUN_ON_WINDOW`] glyph steps between the least budget at which
/// [`RUN_ON_RING`] closes holding back the run on of where its walk would
/// end, 269,149, and the least at which it closes holding back the run on
/// of its whole path, 273,242 — each bisected on #309's base and on every
/// commit of #309 (review 5810838863; #311 measured 269,158 and 273,254 on
/// the code before it).
const RUN_ON_BUDGET: usize = 271_200;

/// How many glyph steps more [`RUN_ON_RING`] needs to close holding back
/// the run on of its whole path than holding back the run on of where its
/// walk would end: 273,242 less 269,149, measured for #309 (review
/// 5810838863; #311's review 5802360938 measured 4,096 on the code before
/// it). A ring that does not close [`RUN_ON_WINDOW`] below
/// [`RUN_ON_BUDGET`] holding back its walk's run on would not close at
/// [`RUN_ON_BUDGET`] holding back its whole path's: so the pin's
/// precondition, set at the window's edge, is what keeps its control red.
const RUN_ON_WINDOW: usize = 4_093;

/// A ring whose path is long beside its line height, so that the run on
/// of the whole of it is thousands of glyph steps: a text round an
/// ellipse 100,000 across, outward.
const RUN_ON_RING: (&str, f32) = ("MANDALA ", 100_000.0);

/// Ruling B17.1d: a ring holds back the run on phase 2 would take of it
/// left unclosed where its walk is, not of its whole path — so a ring
/// whose budget covers its walk, its close and phase 2 with that hold
/// closes, where the hold of its whole path would have stopped it.
/// (Every closed ring of the grid holds back no run on in phase 2: the
/// grid's bound, `held`.)
///
/// Precondition: [`RUN_ON_WINDOW`] less, the ring does not close — at the
/// window's edge, so that a ring the hold of its whole path would close at
/// [`RUN_ON_BUDGET`] fails the precondition instead of leaving the pin
/// green under its control (#311, review 5802360938: at 2 percent less, a
/// ring's work falling by 0.8 to 1.3 percent did so). Control: with the
/// run on held from the path's start whatever the walk has laid
/// (`RingWork::ends_by` doing nothing), red.
#[test]
fn test_flow_ring_budget_a_ring_holds_back_the_run_on_its_walk_leaves() {
    let (text, size) = RUN_ON_RING;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(size)), Shape::Ellipse);
    let closes = |laid: &Laid| !matches!(laid.loss, Some(RingLoss::Short | RingLoss::Unclosed));
    let edge = RUN_ON_BUDGET - RUN_ON_WINDOW;
    let less = ring(text, &shape, true, Some(edge), MAX_PLACED_GLYPHS);
    assert!(
        !closes(&less),
        "precondition: at {edge} glyph steps, the run on window's edge below {RUN_ON_BUDGET}, the ring does not close ({:?}, {} spent): were it to, the hold of its whole path would close it at {RUN_ON_BUDGET} and the pin would hold under its control",
        less.loss,
        less.spent
    );
    let laid = ring(text, &shape, true, Some(RUN_ON_BUDGET), MAX_PLACED_GLYPHS);
    assert!(
        closes(&laid),
        "at {RUN_ON_BUDGET} glyph steps the ring does not close ({:?}, {} spent): its walk held back more run on than its path leaves",
        laid.loss,
        laid.spent
    );
}
