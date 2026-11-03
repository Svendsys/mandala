// SPDX-License-Identifier: MPL-2.0

//! A `Repeat` ring's work budget (#310, `flow::place::ring`): what it
//! takes past its budget, what it holds back for phase 2, and how a ring
//! its budget stopped is reported. Every test here sets a test seam or
//! reads a test counter, so none has a `do_*` twin.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
use crate::flow::place::ring::{ring_overshoot, RingLoss};
use crate::flow::place::{place_on_closed_traced, Trace, RING_CROSSED};
use crate::flow::tests::halves_tests::with_seam;
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, RingLayout};

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

/// The grid's outlines: an ellipse, a sharp rectangle and a rounded one,
/// each 2,000 across at the origin, 2,000 across 10⁷ from it — where an
/// `f32` sets a point to the nearest unit — and 10²² across, where the
/// floats of a point some 10²² from the origin set an ornament's glyphs
/// apart by bisection that barely moves them (the ring of seed 2771).
fn outlines() -> Vec<(String, Outline)> {
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
        ("2,000 at the origin", Vec2::ZERO, 2_000.0),
        ("2,000 at 10⁷", Vec2::splat(1e7), 2_000.0),
        ("10²² across", Vec2::ZERO, 1e22),
    ];
    let mut out = Vec::new();
    for (shape_name, shape) in shapes {
        for (place_name, at, size) in places {
            out.push((
                format!("{shape_name} {place_name}"),
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
    /// Its glyphs a copy, what bounds a cluster's.
    glyphs: usize,
    /// How far past what its budget left after its hold for phase 2 it
    /// was when a walk first found it spent (`RING_CROSSED`).
    crossed: Option<usize>,
    loss: Option<RingLoss>,
    placed: PlacedRun,
}

/// `text` laid as a `Repeat` ring round `shape`, `outward` or not, with
/// `budget` in place of its own when given.
fn ring(
    text: &str,
    shape: &Outline,
    outward: bool,
    budget: Option<usize>,
) -> (usize, Option<usize>, Trace, PlacedRun) {
    let lay = || {
        with_fonts(|fs| {
            let flow = style(POINTS);
            let run = real_run(text, &flow, fs);
            let mut trace = Trace::default();
            PAST_BUDGET.with(|past| past.set(0));
            let (_, placed) = place_on_closed_traced(
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
                Some(&mut trace),
            );
            (
                PAST_BUDGET.with(std::cell::Cell::get),
                RING_CROSSED.with(std::cell::Cell::get),
                trace,
                placed,
            )
        })
    };
    match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    }
}

/// Every ring of the grid: each of [`CONTENTS`] round each of
/// [`outlines`], outward and inward, at each of [`BUDGETS`] and at its
/// own; the last of each case at its own. Laid once, by whichever test
/// asks first.
fn grid() -> &'static [Vec<Laid>] {
    static GRID: std::sync::OnceLock<Vec<Vec<Laid>>> = std::sync::OnceLock::new();
    GRID.get_or_init(lay_grid)
}

/// [`grid`], laid.
fn lay_grid() -> Vec<Vec<Laid>> {
    let mut cases = Vec::new();
    for (content, text) in CONTENTS {
        for (name, shape) in outlines() {
            for outward in [true, false] {
                let what = format!(
                    "{content} round the {name}, {}",
                    if outward { "outward" } else { "inward" }
                );
                let budgets = BUDGETS.iter().map(|&b| Some(b)).chain(std::iter::once(None));
                let case = budgets
                    .map(|budget| {
                        let (past, crossed, trace, placed) = ring(text, &shape, outward, budget);
                        Laid {
                            what: format!(
                                "{what}, budget {}",
                                budget.map_or("its own".to_string(), |b| b.to_string())
                            ),
                            past,
                            spent: trace.spent,
                            budget: trace.budget,
                            glyphs: trace.glyphs,
                            crossed,
                            loss: trace.ring,
                            placed,
                        }
                    })
                    .collect();
                cases.push(case);
            }
        }
    }
    cases
}

/// #310: a ring reads its budget before every cluster its walks lay or
/// measure and before every probe of phase 2's solves, and holds back
/// what phase 2 lays whatever is spent, so past its budget it takes no
/// more than its setup and the one cluster that found it spent
/// (`ring_overshoot`) — `PAST_BUDGET`, a take that crosses the budget
/// counted for the part past it (ruling R50.4) — whatever its content,
/// its outline, how far it lies from the origin, its direction and where
/// in its work the budget runs out. Its walks stop within its setup and
/// a cluster of what the budget leaves after the hold (crossed), and
/// a ring that reports nothing is laid as at its own budget: never
/// capped silently.
///
/// Controls: with the budget read only between a seam's measures, not
/// before each body they walk, red (crossed); with nothing held back for
/// phase 2, red (`PAST_BUDGET`); with the first walk's stop not reported,
/// red (silent).
#[test]
fn test_flow_ring_budget_a_ring_past_its_budget_takes_its_overshoot_at_most() {
    let cases = grid();
    let rings: Vec<&Laid> = cases.iter().flatten().collect();
    let stopped = rings.iter().filter(|r| r.crossed.is_some()).count();
    assert!(
        stopped * 4 >= rings.len(),
        "precondition: a quarter of the rings or more have a walk stopped by their budget ({stopped} of {})",
        rings.len()
    );
    let mut broken = Vec::new();
    for case in cases {
        let own = case.last().expect("every case lays its ring at its own budget");
        for laid in case {
            let allowed = ring_overshoot(laid.glyphs);
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
            if laid.loss.is_none() && own.loss.is_none() && laid.placed != own.placed {
                broken.push(format!(
                    "{}: reported nothing, and laid otherwise than at its own budget (silent)",
                    laid.what
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

/// #310: a ring whose budget stops its work says how it was left
/// (`Trace::ring`), each way somewhere in the grid: short, its first walk
/// stopped before its copies filled the outline; unclosed, its seam's
/// measures or its lay stopped; uneven, phase 2's solves stopped — and,
/// at budgets that afford it, not at all.
///
/// Controls: with each of the three left unreported, red (that one).
#[test]
fn test_flow_ring_budget_a_spent_ring_says_how_it_was_left() {
    let rings: Vec<&Laid> = grid().iter().flatten().collect();
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
