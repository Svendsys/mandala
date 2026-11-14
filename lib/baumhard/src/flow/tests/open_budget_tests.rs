// SPDX-License-Identifier: MPL-2.0

//! The work budget of a placement along an open path (#313): every fit
//! mode along a connector, and a connector's whole-glyph pattern
//! (`place_pitched`), budgeted as a `Repeat` ring is (#310) — what each
//! takes past its budget, how one its budget or its cap stopped is
//! reported, and the margin its budget leaves ordinary connectors. Every
//! test here sets a test seam or reads a test counter, so none has a
//! `do_*` twin.

use glam::Vec2;

use crate::document::values::FitMode;
use crate::flow::place::halves::{table_steps, BUDGET_OVERRIDE, PAST_BUDGET};
use crate::flow::place::pitch::{place_pitched_capped, LAST_PITCHED};
use crate::flow::place::ring::{phase_two_steps, ring_overshoot, RingLoss, RING_STEPS};
use crate::flow::place::{place_on_open_capped, Trace, LAST_OPEN};
use crate::flow::tests::halves_tests::with_seam;
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{Curve, PathEnd, PlacedRun, Rect, MAX_PLACED_GLYPHS};

/// The size every placement here is laid at.
const POINTS: f32 = 16.0;

/// The budgets each placement of the grid is laid with, in glyph steps:
/// from spent before its first cluster to past what most of them take,
/// so that each of a placement's walks — its first, a spread's solve, a
/// fit's measures and rounds, a cut, phase 2's lay — finds it spent
/// somewhere in the grid.
const BUDGETS: [usize; 9] = [1, 300, 1_000, 2_000, 4_000, 8_000, 16_000, 64_000, 200_000];

/// The cap every placement of the grid is laid with, past the glyphs of
/// every one of its paths but the one 10²² long, which holds the
/// engine's cap of any copy: laid at its own budget, it runs to the cap
/// and stops, short — at the engine's cap, 65,536 glyphs, a value the
/// cap's arithmetic proves (ruling B17.2c), not one the grid needs laid.
const GRID_CAP: usize = 1_024;

/// The caps each placement of the grid is laid with besides, at its own
/// budget: below a cluster, a few, and most of the glyphs of the paths at
/// the origin, so that the cap stops a placement's walk, and its cut,
/// somewhere in the grid.
const CAPS: [usize; 3] = [1, 5, 120];

/// What the grid's placements are of: a text, which phase 2 lays again
/// in reading order, and an ornament, which it lays as walked, each glyph
/// set by its ink — a copy for `Repeat`, and thirty of them, longer than
/// the paths at the origin hold at its size, for every other mode.
const CONTENTS: [(&str, &str); 2] = [("letters", "MANDALA "), ("ornament", "· ")];

/// The grid's fit modes along an open path: every one (#313).
fn fits() -> [(&'static str, FitMode); 5] {
    [
        ("Repeat", FitMode::Repeat),
        ("Once", FitMode::Once { filler: None }),
        (
            "Once with a filler",
            FitMode::Once {
                filler: Some("·".to_string()),
            },
        ),
        ("Spread", FitMode::Spread),
        ("Fit", FitMode::Fit { min: 0.5, max: 2.0 }),
    ]
}

/// The grid's paths: a line and a cubic 2,000 long at the origin, the
/// same 10⁷ from it — where an `f32` sets a point to the nearest unit —
/// and a line 10²² long, where the floats of a point some 10²² from the
/// origin set an ornament's glyphs apart by bisection that barely moves
/// them (the cost shape of seed 2771's ring, review 5761697746).
fn paths() -> Vec<(String, bool, Curve)> {
    let line = |at: Vec2, to: Vec2| Curve::Line(at, at + to);
    let cubic = |at: Vec2| {
        Curve::Cubic(
            at,
            at + Vec2::new(700.0, -600.0),
            at + Vec2::new(1_300.0, 600.0),
            at + Vec2::new(2_000.0, 0.0),
        )
    };
    vec![
        (
            "a 2,000 line at the origin".to_string(),
            false,
            line(Vec2::ZERO, Vec2::new(1_600.0, 1_200.0)),
        ),
        (
            "a 2,000 cubic at the origin".to_string(),
            false,
            cubic(Vec2::ZERO),
        ),
        (
            "a 2,000 line at 10⁷".to_string(),
            false,
            line(Vec2::splat(1e7), Vec2::new(1_600.0, -1_200.0)),
        ),
        ("a 2,000 cubic at 10⁷".to_string(), false, cubic(Vec2::splat(1e7))),
        (
            "a cubic 10²² across".to_string(),
            true,
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(3.5e21, -3e21),
                Vec2::new(6.5e21, 3e21),
                Vec2::new(1e22, 0.0),
            ),
        ),
        (
            "a line 10²² long".to_string(),
            true,
            line(Vec2::ZERO, Vec2::new(1e22, 3e21)),
        ),
    ]
}

/// One placement of the grid as laid.
struct Laid {
    what: String,
    /// The glyph steps it took past its budget ([`PAST_BUDGET`]), all it
    /// took, and its budget.
    past: usize,
    spent: usize,
    budget: usize,
    /// Its text's glyphs; what bounds the one cluster that may find its
    /// budget spent, its widest cluster's glyphs; and the samples its
    /// path's table was built with.
    text: usize,
    copy: usize,
    tables: usize,
    /// How far past what its budget left after its hold for phase 2 it
    /// was when a walk first found it spent, and the probes of phase 2's
    /// solves it began with its budget spent (`LAST_OPEN`).
    crossed: Option<usize>,
    late: usize,
    /// What was held back for phase 2 when it began, and whether its run
    /// has a letter in it, which phase 2 lays in reading order.
    held: usize,
    letters: bool,
    /// What phase 2 took, from when it began to the end of the placement
    /// (`LastRing::again`): what the hold is for.
    again: usize,
    loss: Option<RingLoss>,
    placed: PlacedRun,
}

/// `text` laid along `curve` in `fit`, at most `cap` glyphs, with
/// `budget` in place of its own when given.
fn open(text: &str, curve: &Curve, fit: &FitMode, budget: Option<usize>, cap: usize) -> Laid {
    let lay = || {
        with_fonts(|fs| {
            let flow = style(POINTS);
            let run = real_run(text, &flow, fs);
            let mut trace = Trace::default();
            PAST_BUDGET.with(|past| past.set(0));
            let (_, placed) = place_on_open_capped(&run, curve, fit, 0.0, &flow, fs, cap, Some(&mut trace));
            let (last, crossed, late) = LAST_OPEN
                .with(std::cell::Cell::get)
                .expect("a placement along an open path is recorded");
            Laid {
                what: String::new(),
                past: PAST_BUDGET.with(std::cell::Cell::get),
                spent: trace.spent,
                budget: trace.budget,
                text: run.glyphs.len(),
                copy: last.copy,
                tables: trace.tables,
                crossed,
                late,
                held: last.held.0,
                letters: run.glyphs.iter().any(|glyph| glyph.letter),
                again: last.again,
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

/// One case of the grid: a placement laid at each of its budgets and
/// caps, the last at its own, and whether its path is the one 10²² long.
struct Case {
    far: bool,
    laid: Vec<Laid>,
}

/// Every placement of the grid: each of [`CONTENTS`] along each of
/// [`paths`] in each of [`fits`], at [`GRID_CAP`] and each of
/// [`BUDGETS`], at its own budget and each of [`CAPS`], and last at its
/// own. Laid once, by whichever test asks first.
fn grid() -> &'static [Case] {
    static GRID: std::sync::OnceLock<Vec<Case>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let mut cases = Vec::new();
        for (content, copy) in CONTENTS {
            for (fit_name, fit) in fits() {
                // A `Repeat` lays its copy again and again; every other
                // mode lays its text once, longer than the paths at the
                // origin hold.
                let text = if matches!(fit, FitMode::Repeat) {
                    copy.to_string()
                } else {
                    copy.repeat(30)
                };
                for (path_name, far, curve) in paths() {
                    let what = format!("{content} {fit_name} along {path_name}");
                    let named = |mut laid: Laid, how: String| {
                        laid.what = format!("{what}, {how}");
                        laid
                    };
                    let mut laid: Vec<Laid> = BUDGETS
                        .iter()
                        .map(|&b| {
                            named(
                                open(&text, &curve, &fit, Some(b), GRID_CAP),
                                format!("budget {b}"),
                            )
                        })
                        .collect();
                    laid.extend(
                        CAPS.iter()
                            .map(|&cap| named(open(&text, &curve, &fit, None, cap), format!("cap {cap}"))),
                    );
                    laid.push(named(
                        open(&text, &curve, &fit, None, GRID_CAP),
                        "its own budget".to_string(),
                    ));
                    cases.push(Case { far, laid });
                }
            }
        }
        // A long text's `Fit` (#313): its measures of its text as read walk
        // it as phase 2 would, reading no budget as they go, so each is read
        // against the budget before it runs — at every budget about where
        // they run out.
        let (_, _, line) = &paths()[0];
        let fit = FitMode::Fit { min: 0.5, max: 2.0 };
        let text = CONTENTS[0].1.repeat(LONG_FIT_COPIES);
        let mut laid: Vec<Laid> = LONG_FIT_BUDGETS
            .step_by(250)
            .map(|b| {
                let mut laid = open(&text, line, &fit, Some(b), GRID_CAP);
                laid.what = format!("a long text's Fit along the 2,000 line at the origin, budget {b}");
                laid
            })
            .collect();
        let mut own = open(&text, line, &fit, None, GRID_CAP);
        own.what = "a long text's Fit along the 2,000 line at the origin, its own budget".to_string();
        laid.push(own);
        cases.push(Case { far: false, laid });
        cases
    })
}

/// How many copies of the grid's letters the long text's `Fit` lays: 400,
/// 3,200 glyphs, whose measure of its text as read, laid unread against
/// its budget, takes the placement 609 glyph steps past what its budget
/// leaves (at a budget of 1,500), past the setup and the one cluster it
/// may take past it.
const LONG_FIT_COPIES: usize = 400;

/// The budgets the long text's `Fit` is laid with, every 250th: about
/// where its measures run out.
const LONG_FIT_BUDGETS: std::ops::RangeInclusive<usize> = 500..=4_000;

/// #313: a placement along an open path reads its budget before every
/// cluster its walks lay or measure, before every probe of its spacing
/// solves and every pass of its text as read, and holds back what phase 2
/// lays whatever is spent, as a `Repeat` ring does (#310) — so past its
/// budget it takes no more than its setup and the one cluster that found
/// it spent (`ring_overshoot`), whatever its content, its fit mode, its
/// path, how far that lies from the origin, its cap and where in its work
/// the budget runs out. Its walks stop within its setup and a cluster of
/// what the budget leaves after the hold (crossed); phase 2 begins no
/// probe of its solves once the budget is spent (late); and a placement
/// that reports nothing is laid as at its own budget and the grid's cap,
/// which reports nothing either: never cut short silently, by its budget
/// or its cap.
///
/// Preconditions: a quarter of the placements or more have a walk stopped
/// by their budget, and at their own budget the paths 10²² across run to
/// the grid's cap and no other placement is stopped.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_placement_past_its_budget_takes_its_overshoot_at_most() {
    let cases = grid();
    let laid: Vec<&Laid> = cases.iter().flat_map(|case| &case.laid).collect();
    let stopped = laid.iter().filter(|l| l.crossed.is_some()).count();
    assert!(
        stopped * 4 >= laid.len(),
        "precondition: a quarter of the placements or more have a walk stopped by their budget ({stopped} of {})",
        laid.len()
    );
    let mut broken = Vec::new();
    for case in cases {
        let own = case
            .laid
            .last()
            .expect("every case lays its placement at its own budget");
        assert!(
            case.far || own.loss.is_none(),
            "precondition: {} is laid whole at its own budget ({:?})",
            own.what,
            own.loss
        );
        for laid in &case.laid {
            let allowed = ring_overshoot(laid.copy, laid.tables);
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
                    "{}: phase 2 began {} probes with its budget spent (late)",
                    laid.what, laid.late
                ));
            }
            let owed = phase_two_steps(laid.placed.glyphs.len(), laid.letters);
            if laid.again > laid.held {
                broken.push(format!(
                    "{}: phase 2 took {} glyph steps, past the {} held back for it (phase 2)",
                    laid.what, laid.again, laid.held
                ));
            }
            if laid.held < owed {
                broken.push(format!(
                    "{}: it held back {} glyph steps for phase 2, short of its work on its glyphs, {owed} (held)",
                    laid.what, laid.held
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
        "{} of {} placements past the bound:\n{}",
        broken.len(),
        laid.len(),
        broken.join("\n")
    );
}

/// #313: a placement along an open path whose budget stops its work says
/// how it was left (`Trace::ring`), each way somewhere in the grid: short,
/// a walk stopped before its text or its copies were laid; uneven, a
/// solve of its spacing or of a `Fit`'s scale stopped — and, at budgets
/// that afford it, not at all. A `Fit` of an ornament, whose measure at a
/// scale walks it as laid, is left uneven where that walk finds its
/// budget spent (review 5902086644), as a text's is where its measure as
/// read cannot be afforded.
///
/// Controls: with each of the two left unreported, red (that one); with
/// an ornament's measure stopped by its budget left unreported, red on
/// the ornament's `Fit`.
#[test]
fn test_flow_open_budget_a_spent_placement_says_how_it_was_left() {
    let laid: Vec<&Laid> = grid().iter().flat_map(|case| &case.laid).collect();
    for loss in [None, Some(RingLoss::Short), Some(RingLoss::Uneven)] {
        assert!(
            laid.iter().any(|l| l.loss == loss),
            "no placement of the grid's {} is left {loss:?}",
            laid.len()
        );
    }
    let ornament_fits: Vec<&&Laid> = laid
        .iter()
        .filter(|l| l.what.starts_with("ornament Fit along"))
        .collect();
    assert!(
        ornament_fits.iter().any(|l| l.loss == Some(RingLoss::Uneven)),
        "no ornament's Fit of the grid's {} is left uneven",
        ornament_fits.len()
    );
}

/// #313 (review 5902086644): a `Once` whose filler's cluster is wider than
/// every cluster of its text — a letter under fifteen marks, sixteen
/// glyphs, after two letters — is held to the overshoot of a cluster of
/// the filler's (`RingWork::widen`), the one cluster that may find its
/// budget spent being one of its copies: the placement records the
/// filler's cluster as its copy, and at every budget from its setup's to
/// its own it takes past its budget, and stops past what its budget
/// leaves (crossed), no more than that overshoot.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_fillers_wide_cluster_is_its_overshoots_cluster() {
    const MARKS: &str = "\u{0300}\u{0301}\u{0302}\u{0303}\u{0304}\u{0306}\u{0307}\u{0308}\u{030A}\u{030B}\u{030C}\u{0323}\u{0324}\u{0325}\u{0330}";
    let filler = format!("o{MARKS}");
    let wide = with_fonts(|fs| real_run(&filler, &style(POINTS), fs).glyphs.len());
    assert!(wide >= 8, "precondition: the filler's cluster is {wide} glyphs");
    let fit = FitMode::Once {
        filler: Some(filler.clone()),
    };
    let (_, _, line) = &paths()[0];
    let own = open("ab", line, &fit, None, GRID_CAP);
    let setup = open("ab", line, &fit, Some(1), GRID_CAP);
    assert!(
        own.placed.glyphs.len() > 2 * wide && own.spent > setup.spent,
        "precondition: its copies are laid ({} glyphs, {} steps)",
        own.placed.glyphs.len(),
        own.spent
    );
    let mut crossed = 0;
    for step in 0..=60 {
        let budget = setup.spent + (own.spent - setup.spent) * step / 60;
        let laid = open("ab", line, &fit, Some(budget), GRID_CAP);
        // One that laid a copy of its filler took the filler's cluster as
        // its copy before it laid any; one stopped before it came to them
        // laid none.
        let copies = laid.placed.glyphs.len() > 2;
        assert!(
            !copies || laid.copy == wide,
            "budget {budget}: its copy is the filler's cluster, {wide} glyphs, not {}",
            laid.copy
        );
        let allowed = ring_overshoot(laid.copy, laid.tables);
        assert!(
            laid.past <= allowed && laid.crossed.is_none_or(|c| c <= allowed),
            "budget {budget}: {} past its budget, crossed {:?}, against {allowed}",
            laid.past,
            laid.crossed
        );
        crossed += usize::from(laid.crossed.is_some() && copies);
    }
    assert!(
        crossed >= 10,
        "precondition: the budgets stop its walks among its copies ({crossed} of 61)"
    );
}

/// #313, ruling B18.1 §8: a `Fit`'s hold never shrinks — what its report
/// of a scale its budget stopped rests on (`place::fit_to`) — and a debug
/// build says so the moment it would: a `Fit`'s work its walk would note,
/// as a `Repeat`'s does, as stopping near its path's end (`ends_by`), so
/// that its run on fell, panics at its next read of its budget. A `Fit`'s
/// work never comes there: every placement of the grid in `Fit` reads its
/// budget unasserted.
///
/// Control: `fit_to` noting its walk's end as a `Repeat` does, red on
/// every budgeted `Fit` of the grid (its hold shrinks).
#[test]
fn test_flow_open_budget_a_fits_hold_never_shrinks() {
    use crate::flow::place::ring::RingWork;
    use crate::flow::ArcPath;
    let path = ArcPath::open(&Curve::Line(Vec2::ZERO, Vec2::new(2_000.0, 0.0)), 1.0);
    let fits = |end: bool| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let work = RingWork::new(&path, 1, true, 20.0, 100, GRID_CAP);
            work.fitting();
            let _ = work.affords(&path);
            if end {
                work.ends_by(1_900.0);
            }
            work.affords(&path)
        }))
    };
    assert!(fits(false).is_ok(), "a Fit's work reads its budget");
    if cfg!(debug_assertions) {
        assert!(
            fits(true).is_err(),
            "a Fit's hold that shrank is asserted, not passed over"
        );
    }
}

/// What the pitched grid's placements are of: ornaments a connector lays
/// as whole glyphs, a dot, and a bar and a double bar, whose inks differ
/// in shape.
const PATTERNS: [(&str, &str); 2] = [("a dot", "·"), ("a bar and a double bar", "-=")];

/// The budgets each placement of the pitched grid is laid with: from
/// spent before its first ink to past what most of them take.
const PITCHED_BUDGETS: [usize; 8] = [1, 300, 1_000, 3_000, 10_000, 30_000, 100_000, 300_000];

/// One pitched placement of the grid as laid.
struct Pitched {
    what: String,
    /// The glyph steps it took past its budget ([`PAST_BUDGET`]), all it
    /// took, and its budget (`LAST_PITCHED`).
    past: usize,
    spent: usize,
    budget: usize,
    /// The samples its path's table was built with, counted in its setup.
    tables: usize,
    loss: Option<RingLoss>,
    placed: PlacedRun,
}

/// `boxes` glyph-sized boxes in a row across a path's end at `at`, as a
/// frame's inks lie round a node: what an end of the pitched grid's path
/// keeps its ink a gap from.
fn end_boxes(at: Vec2, boxes: usize) -> Vec<Rect> {
    (0..boxes)
        .map(|k| {
            Rect::new(
                at + Vec2::new(-4.0, (k as f32 - boxes as f32 / 2.0) * 12.0),
                Vec2::new(8.0, 10.0),
            )
        })
        .collect()
}

/// `pattern` laid as whole glyphs along `curve`, its ends beside `boxes`
/// boxes each, at most `cap` glyphs, with `budget` in place of its own
/// when given.
fn pitched(pattern: &str, curve: &Curve, boxes: usize, budget: Option<usize>, cap: usize) -> Pitched {
    let (first, last) = (
        end_boxes(curve.point(0.0), boxes),
        end_boxes(curve.point(1.0), boxes),
    );
    let lay = || {
        with_fonts(|fs| {
            let run = real_run(pattern, &style(POINTS), fs);
            let ends = [&first, &last].map(|inks| PathEnd {
                inks,
                back: 0.0,
                hold: 0.0,
            });
            PAST_BUDGET.with(|past| past.set(0));
            LAST_PITCHED.with(|last| last.set(None));
            let mut trace = Trace::default();
            let placed = place_pitched_capped(
                &run,
                std::slice::from_ref(curve),
                0.0,
                ends,
                cap,
                Some(&mut trace),
            );
            // A path whose length is not a positive number lays nothing,
            // and is not budgeted.
            let (spent, budget, loss) = LAST_PITCHED.with(std::cell::Cell::get).unwrap_or_else(|| {
                assert!(placed.glyphs.is_empty(), "a placement not budgeted lays nothing");
                (0, 0, None)
            });
            assert!(
                trace.spent == spent && trace.budget == budget && trace.ring == loss,
                "the trace holds what the placement recorded"
            );
            Pitched {
                what: String::new(),
                past: PAST_BUDGET.with(std::cell::Cell::get),
                spent,
                budget,
                tables: trace.tables,
                loss,
                placed,
            }
        })
    };
    match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    }
}

/// Every placement of the pitched grid: each of [`PATTERNS`] along each of
/// [`paths`], its ends beside no boxes and beside two hundred, at
/// [`GRID_CAP`] and each of [`PITCHED_BUDGETS`], at its own budget and
/// each of [`CAPS`], and last at its own: each case's placements, the last
/// at its own budget, and whether its path is one far out.
fn pitched_grid() -> &'static [(bool, Vec<Pitched>)] {
    static GRID: std::sync::OnceLock<Vec<(bool, Vec<Pitched>)>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let mut cases = Vec::new();
        for (name, pattern) in PATTERNS {
            for (path_name, far, curve) in paths() {
                for boxes in [0, 200] {
                    let what = format!("{name} along {path_name}, beside {boxes} boxes at each end");
                    let named = |mut laid: Pitched, how: String| {
                        laid.what = format!("{what}, {how}");
                        laid
                    };
                    let mut laid: Vec<Pitched> = PITCHED_BUDGETS
                        .iter()
                        .map(|&b| {
                            named(
                                pitched(pattern, &curve, boxes, Some(b), GRID_CAP),
                                format!("budget {b}"),
                            )
                        })
                        .collect();
                    laid.extend(
                        CAPS.iter().map(|&cap| {
                            named(pitched(pattern, &curve, boxes, None, cap), format!("cap {cap}"))
                        }),
                    );
                    laid.push(named(
                        pitched(pattern, &curve, boxes, None, GRID_CAP),
                        "its own budget".to_string(),
                    ));
                    cases.push((far, laid));
                }
            }
        }
        cases
    })
}

/// #313: a connector's whole-glyph pattern (`place_pitched`) reads its
/// budget before it lays anything, before each ink it counts and before
/// each round of its slack, each time holding back what the lays that may
/// follow before it reads it again take — its path's reads, the gaps it
/// measures to the boxes at its ends and between its inks, and the glyphs
/// it places — so it takes no more than its budget, but for its setup and
/// its path's table, counted before it first reads it, whatever its
/// pattern, its path, how far that lies from the origin, the boxes at its
/// ends, its cap and where in its work the budget runs out. One that reports
/// nothing is laid as at its own budget and the grid's cap, which reports
/// nothing either; one its budget or its cap stopped is short.
///
/// Preconditions: a quarter of the placements or more are stopped by
/// their budget or their cap, and every one along a path at the origin is
/// laid whole at its own budget.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_pitched_pattern_takes_its_budget_at_most() {
    let cases = pitched_grid();
    let laid: Vec<&Pitched> = cases.iter().flat_map(|(_, case)| case).collect();
    let short = laid.iter().filter(|l| l.loss == Some(RingLoss::Short)).count();
    assert!(
        short * 4 >= laid.len(),
        "precondition: a quarter of the placements or more are stopped by their budget or their cap ({short} of {})",
        laid.len()
    );
    let mut broken = Vec::new();
    for (_, case) in cases {
        let own = case
            .last()
            .expect("every case lays its placement at its own budget");
        assert!(
            !own.what.contains("at the origin") || (own.loss.is_none() && !own.placed.glyphs.is_empty()),
            "precondition: {} is laid whole at its own budget ({:?}, {} glyphs)",
            own.what,
            own.loss,
            own.placed.glyphs.len()
        );
        for laid in case {
            let setup = RING_STEPS + table_steps(laid.tables);
            if laid.past > setup || laid.spent > laid.budget.max(setup) {
                broken.push(format!(
                    "{}: {} past its budget of {}, {} in all, against its setup's {setup}",
                    laid.what, laid.past, laid.budget, laid.spent
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
        "{} of {} pitched placements past their budget:\n{}",
        broken.len(),
        laid.len(),
        broken.join("\n")
    );
}

/// #313 (review 5902086644): a pitched placement counts the gaps between
/// boxes it measures — to the boxes at its ends, and between its inks —
/// [`BOXES_A_STEP`](crate::flow::place::halves) to a glyph step, exactly:
/// its counted steps are its gaps over that, rounded up; and beside two
/// hundred boxes at each end its gaps are many.
///
/// Control: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_pitched_pattern_counts_the_gaps_it_measures() {
    use crate::flow::place::halves::BOXES_A_STEP;
    use crate::flow::place::pitch::LAST_PITCHED_GAPS;
    let (_, _, line) = &paths()[0];
    for boxes in [0, 200] {
        LAST_PITCHED_GAPS.with(|last| last.set((0, 0)));
        let _ = pitched(PATTERNS[1].1, line, boxes, None, GRID_CAP);
        let (gaps, steps) = LAST_PITCHED_GAPS.with(std::cell::Cell::get);
        assert!(
            gaps >= 2 * boxes,
            "precondition: beside {boxes} boxes at each end it measures {gaps} gaps"
        );
        assert_eq!(
            steps,
            gaps.div_ceil(BOXES_A_STEP),
            "beside {boxes} boxes: {gaps} gaps counted {steps} glyph steps"
        );
    }
}

/// #313, ruling B18.1 §4: a pitched placement of wide clusters — a dot
/// under 240 marks, 241 glyphs an ink — holds back, before each ink it
/// counts and each round of its slack, the glyphs of the inks it would
/// then hold, which are counted once it is laid: no placement takes more
/// after its last read of its budget than that read held back for
/// (`PITCH_EDGE`), at any budget from its setup's to its own, by
/// construction — the four lays it holds back for besides cover some
/// thirty glyphs an ink at their most reads and gaps, and no more.
///
/// Control: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_pitched_pattern_of_wide_clusters_holds_its_glyphs() {
    use crate::flow::place::pitch::PITCH_EDGE;
    const MARKS: &str = "\u{0300}\u{0301}\u{0302}\u{0303}\u{0304}\u{0306}\u{0307}\u{0308}\u{030A}\u{030B}\u{030C}\u{0323}\u{0324}\u{0325}\u{0330}";
    let pattern = format!("·{}", MARKS.repeat(16));
    let (_, _, line) = &paths()[0];
    let own = pitched(&pattern, line, 0, None, MAX_PLACED_GLYPHS);
    let setup = pitched(&pattern, line, 0, Some(1), MAX_PLACED_GLYPHS);
    assert!(
        own.placed.glyphs.len() > 4 * 200 && own.spent > setup.spent,
        "precondition: it lays many inks ({} glyphs, {} steps)",
        own.placed.glyphs.len(),
        own.spent
    );
    PITCH_EDGE.with(|edge| edge.set(None));
    let mut short = 0;
    for step in 0..=100 {
        let budget = setup.spent + (own.spent - setup.spent) * step / 100;
        let laid = pitched(&pattern, line, 0, Some(budget), MAX_PLACED_GLYPHS);
        short += usize::from(laid.loss == Some(RingLoss::Short));
    }
    let edge = PITCH_EDGE
        .with(std::cell::Cell::get)
        .expect("precondition: it reads its budget");
    assert!(
        edge <= 0,
        "a pitched placement took {edge} glyph steps past what its last read of its budget held back for"
    );
    assert!(short >= 10, "precondition: the budgets stop it ({short} of 101)");
}

/// #313, as ruling B17.1a has it for a ring's copies: a `Once` with a
/// filler is budgeted by the copies of its filler its path holds past its
/// text, not by its text alone — along the cubic 10²² across, which holds
/// the grid's cap of copies, its budget is past its text's.
///
/// Control: see the pull request of #313.
#[test]
fn test_flow_open_budget_a_once_with_a_filler_is_budgeted_by_its_copies() {
    use crate::flow::place::halves::frame_budget;
    let fillers: Vec<&Laid> = grid()
        .iter()
        .filter_map(|case| case.laid.last())
        .filter(|own| own.what.contains("Once with a filler along a cubic 10²² across"))
        .collect();
    assert!(
        fillers.len() == 2 && fillers.iter().all(|own| own.placed.glyphs.len() > own.text),
        "precondition: the grid lays both its fillers along the far cubic past their text"
    );
    for own in fillers {
        assert!(
            own.budget > frame_budget(own.text),
            "{}: budgeted {} glyph steps, its text's {} alone, though it placed {} glyphs of a text of {}",
            own.what,
            own.budget,
            frame_budget(own.text),
            own.placed.glyphs.len(),
            own.text
        );
    }
}

/// The connectors of the margin sweep: a line and a cubic at the origin,
/// from 1,000 long to 200,000, as a map's connectors and line links run.
fn sweep_paths() -> Vec<(String, Curve)> {
    let mut paths = Vec::new();
    for length in [1_000.0_f32, 10_000.0, 50_000.0, 200_000.0] {
        paths.push((
            format!("a {length} line"),
            Curve::Line(Vec2::ZERO, Vec2::new(0.8 * length, 0.6 * length)),
        ));
        paths.push((
            format!("a {length} cubic"),
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(0.35 * length, -0.3 * length),
                Vec2::new(0.65 * length, 0.3 * length),
                Vec2::new(length, 0.0),
            ),
        ));
    }
    paths
}

/// The texts of the margin sweep: a connector's label, a word and a
/// sentence, and the ornaments a connector repeats.
const SWEEP_TEXTS: [&str; 5] = ["MANDALA", "To see a World in a Grain of Sand", "· ", "─═", "| ·"];

/// #313, as ruling B17.1a has it for rings: a placement along an open
/// path is budgeted by the glyphs it will lay, as a frame is by its text
/// (ruling R44.2), so every ordinary connector of the sweep — a label, a
/// sentence and ornaments along lines and cubics from 1,000 long to
/// 200,000, in every fit mode, and each ornament laid as whole glyphs
/// (`place_pitched`), at the engine's cap — is laid at its own budget
/// with nothing lost, and its budget is at least five times what it
/// takes: the margin ruling B17.1a pinned for rings, five times the most
/// steps a glyph any of them takes within `STEPS_PER_GLYPH`, read here
/// against the whole budget, its floor included. A text laid once along
/// a path far longer than it pays for the path past it — phase 2's run on
/// past where its walk ends, a step a line height of path — whatever its
/// glyphs, so its margin is read against its whole budget: `· ` once
/// along the 200,000 cubic, two glyphs, takes 3,820 steps, 1,910 a glyph.
/// The worst, the sentence once along that cubic, takes 3,841 of its
/// 400,000, a 104th.
///
/// Control: see the pull request of #313.
#[test]
fn test_flow_open_budget_ordinary_connectors_are_laid_within_a_fifth_of_their_budget() {
    use crate::flow::MAX_PLACED_GLYPHS;
    let mut worst = (0.0_f64, String::new());
    let mut lost = Vec::new();
    let mut laid_count = 0;
    let mut note = |what: String, spent: usize, budget: usize, loss: Option<RingLoss>| {
        laid_count += 1;
        if loss.is_some() {
            lost.push(format!("{what}: {loss:?}"));
        }
        let share = spent as f64 / budget as f64;
        if share > worst.0 {
            worst = (share, format!("{what}: {spent} of {budget}"));
        }
    };
    for (path_name, curve) in sweep_paths() {
        for text in SWEEP_TEXTS {
            for (fit_name, fit) in fits() {
                let laid = open(text, &curve, &fit, None, MAX_PLACED_GLYPHS);
                note(
                    format!("{text:?} {fit_name} along {path_name}"),
                    laid.spent,
                    laid.budget,
                    laid.loss,
                );
            }
            let laid = pitched(text, &curve, 0, None, MAX_PLACED_GLYPHS);
            if laid.budget > 0 {
                note(
                    format!("{text:?} as whole glyphs along {path_name}"),
                    laid.spent,
                    laid.budget,
                    laid.loss,
                );
            }
        }
    }
    assert!(
        laid_count > 100,
        "precondition: the sweep lays its connectors ({laid_count})"
    );
    assert!(
        lost.is_empty(),
        "{} connectors not laid whole at their own budget:\n{}",
        lost.len(),
        lost.join("\n")
    );
    let ratio = 1.0 / worst.0;
    assert!(
        ratio >= 5.0,
        "the sweep's worst connector takes a {ratio:.2}th of its budget, at most a fifth ({})",
        worst.1
    );
}
