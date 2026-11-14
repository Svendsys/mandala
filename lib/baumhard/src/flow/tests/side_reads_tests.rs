// SPDX-License-Identifier: MPL-2.0

//! A frame's side ornament counts its lays' path reads (#313): far from
//! the origin, where a copy's cluster reads its path many times where it
//! reads it once at the origin, what it takes past its budget, how its
//! counted steps stand to its reads, and the most a cluster reads, what
//! its lays are held back for. Every test here sets a test seam or reads
//! a test counter, so none has a `do_*` twin.

use glam::Vec2;

use crate::document::values::{FitMode, LineSpan, Percent, Rgba, Shape};
use crate::flow::place::halves::{
    overshoot_steps, BUDGET_OVERRIDE, MOST_SIDE_CLUSTER_READS, PAST_BUDGET, SIDE_CLUSTER_READS, SIDE_EDGE,
    SIDE_READS, SIDE_READS_A_STEP, SIDE_STEPS, SIDE_STEP_LOOKUPS,
};
use crate::flow::place::{SideLoss, Trace};
use crate::flow::tests::flow_fuzz::ornament_run;
use crate::flow::tests::halves_tests::{frame_with, with_seam};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, RingLayout};

/// The text every frame here is laid with, and its size.
const TEXT: &str = "To see a World in a Grain of Sand";
const POINTS: f32 = 22.0;

/// The side ornaments of the grid: a box-drawing pair and a dot, which
/// review 5761697746 found read their path 32 to 53 times a counted step
/// far out.
const SIDES: [&str; 2] = ["─═", "· "];

/// The budgets each frame of the grid is laid with besides its own: each
/// spends it partway through its side ornament's lays, far out.
const BUDGETS: [usize; 3] = [20_000, 60_000, 150_000];

/// The grid's outlines: an ellipse and a sharp square — whose corners
/// the band curve sweeps round, a read each (`ArcPath::offset_in`) — 2,000
/// across at the origin, 2,000 across 3·10⁷ from it, and 200,000 across
/// 10¹⁰ from it (review 5761697746), and whether each is far out.
fn outlines() -> Vec<(String, bool, Outline)> {
    let shapes = [
        ("ellipse", Shape::Ellipse),
        (
            "sharp square",
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
    ];
    let places = [
        ("2,000 at the origin", Vec2::ZERO, 2_000.0, false),
        ("2,000 at 3·10⁷", Vec2::splat(3e7), 2_000.0, true),
        ("200,000 at 10¹⁰", Vec2::splat(1e10), 200_000.0, true),
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

/// One frame of the grid as laid.
struct Laid {
    what: String,
    far: bool,
    /// The glyph steps it took past its budget ([`PAST_BUDGET`]), all it
    /// took, its budget, and what it may take past it
    /// (`overshoot_steps`).
    past: usize,
    spent: usize,
    budget: usize,
    allowed: usize,
    /// Its side ornament's lays: the path reads they counted
    /// ([`SIDE_READS`]), the glyph steps they counted ([`SIDE_STEPS`]),
    /// the path lookups their clusters took (`SIDE_STEP_LOOKUPS`, counted
    /// apart from the reads), and the most reads one cluster took
    /// ([`MOST_SIDE_CLUSTER_READS`]).
    reads: usize,
    steps: usize,
    lookups: usize,
    most: usize,
    /// The nearest a lay of its sides left it to what its budget holds for
    /// their touch check ([`SIDE_EDGE`]): how far past, below none where
    /// short of it; `None` where none was laid.
    edge: Option<i64>,
    sides: Vec<(usize, SideLoss)>,
    placed: PlacedRun,
}

/// The frame of [`TEXT`] round `shape`, `Once` at span natural with
/// `sides`, outward, with `budget` in place of its own when given.
fn frame(sides: &str, shape: &Outline, budget: Option<usize>) -> Laid {
    let layout = RingLayout {
        span: LineSpan::NATURAL,
        sides: Some((sides.to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let fit = FitMode::Once { filler: None };
    let allowed = with_fonts(|fs| {
        let flow = style(POINTS);
        let run = real_run(TEXT, &flow, fs);
        let ornament = ornament_run(&layout, &flow, 1.0, fs);
        let tables = crate::flow::place::setup_table_samples(&run, shape, &fit, &flow);
        overshoot_steps(&run, ornament.as_ref(), false, tables)
    });
    let counters = [&SIDE_READS, &SIDE_STEPS, &MOST_SIDE_CLUSTER_READS, &PAST_BUDGET];
    for counter in counters {
        counter.with(|count| count.set(0));
    }
    SIDE_EDGE.with(|edge| edge.set(None));
    let (_, lookups_before) = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
    let lay = || frame_with(TEXT, POINTS, shape, &fit, &layout, true);
    let (_, placed, trace): (_, PlacedRun, Trace) = match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    };
    let [reads, steps, most, past] = counters.map(|counter| counter.with(std::cell::Cell::get));
    Laid {
        what: String::new(),
        far: false,
        past,
        spent: trace.spent,
        budget: trace.budget,
        allowed,
        reads,
        steps,
        lookups: SIDE_STEP_LOOKUPS.with(std::cell::Cell::get).1 - lookups_before,
        most,
        edge: SIDE_EDGE.with(std::cell::Cell::get),
        sides: trace.unheld.sides,
        placed,
    }
}

/// Every frame of the grid: each of [`SIDES`] round each of
/// [`outlines`], at each of [`BUDGETS`] and last at its own — each case's
/// frames, the last at its own budget. Laid once, by whichever test asks
/// first.
fn grid() -> &'static [Vec<Laid>] {
    static GRID: std::sync::OnceLock<Vec<Vec<Laid>>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let mut cases = Vec::new();
        for sides in SIDES {
            for (name, far, shape) in outlines() {
                let budgets = BUDGETS.iter().map(|&b| Some(b)).chain(std::iter::once(None));
                let laid = budgets
                    .map(|budget| {
                        let mut laid = frame(sides, &shape, budget);
                        laid.what = format!(
                            "{sides:?} round the {name}, {}",
                            budget.map_or_else(|| "its own budget".to_string(), |b| format!("budget {b}"))
                        );
                        laid.far = far;
                        laid
                    })
                    .collect();
                cases.push(laid);
            }
        }
        cases
    })
}

/// #313: a frame's side ornament counts its lays' path reads, and holds
/// back the most a lay may read before it is laid, so a frame far from the
/// origin, whose copies read their path 15 to 20 times a cluster where at
/// the origin they read it once, takes past its budget no more than
/// `overshoot_steps` — `PAST_BUDGET`, a take that crosses the budget
/// counted for the part past it (ruling R50.4) — and in all no more than
/// its budget and that (ruling R47.2): no lay of its sides leaves it past
/// what its budget holds for their touch check (crossed). And a frame
/// that lays fewer glyphs than at its own budget says which arcs it left
/// empty or kept short.
///
/// Preconditions: a frame far out spends its budget partway through its
/// sides, at some budget of the grid.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_side_reads_a_far_out_frame_takes_its_overshoot_at_most() {
    let cases = grid();
    let frames: Vec<&Laid> = cases.iter().flatten().collect();
    assert!(
        frames
            .iter()
            .any(|f| f.far && !f.sides.is_empty() && f.spent * 2 >= f.budget),
        "precondition: a frame far out spends its budget partway through its sides"
    );
    // A lay that the next one's hold stopped left the frame within that
    // hold of the edge: the edge is measured, not left at its start.
    let nearest = frames.iter().filter_map(|f| f.edge).max();
    assert!(
        nearest.is_some_and(|edge| edge >= -EDGE_NEAR),
        "precondition: some frame's sides' lays come within {EDGE_NEAR} glyph steps of what its budget holds for their check ({nearest:?})"
    );
    let mut broken = Vec::new();
    for case in cases {
        let own = case.last().expect("every case lays its frame at its own budget");
        for f in case {
            if f.past > f.allowed || f.spent > f.budget + f.allowed {
                broken.push(format!(
                    "{}: {} past its budget of {}, {} in all, against {} allowed past it",
                    f.what, f.past, f.budget, f.spent, f.allowed
                ));
            }
            if f.edge.is_some_and(|edge| edge > 0) {
                broken.push(format!(
                    "{}: a lay of its sides left it {:?} glyph steps past what its budget holds for their check (crossed)",
                    f.what, f.edge
                ));
            }
            if f.placed.glyphs.len() < own.placed.glyphs.len() && f.sides.is_empty() {
                broken.push(format!(
                    "{}: laid {} glyphs, fewer than its own budget's {}, and reported no side (silent)",
                    f.what,
                    f.placed.glyphs.len(),
                    own.placed.glyphs.len()
                ));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} frames past the bound:\n{}",
        broken.len(),
        frames.len(),
        broken.join("\n")
    );
}

/// #313 (ruling R46.1: one counted unit costs about the same everywhere):
/// a side ornament's lays count every path read they take — each lookup
/// their clusters make, counted apart from the reads by the lookups'
/// own counter, is among them — and their counted steps stand to their
/// reads within a pinned band: at most [`SIDE_READS_A_STEP`] reads a
/// counted step, every read counted at that weight past the step a
/// cluster, and
/// far out at least [`FAR_READS_A_STEP`], where their clusters read the
/// path many times — where, counted a step a cluster, they read it up to
/// 20 times a step (and 32 to 53, review 5761697746 found, before #309).
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_side_reads_counted_steps_stand_to_their_reads_within_a_band() {
    let frames: Vec<&Laid> = grid().iter().flatten().collect();
    let mut broken = Vec::new();
    for f in &frames {
        assert!(
            f.steps > 0 && f.reads > 0,
            "precondition: {} lays its sides ({} steps, {} reads)",
            f.what,
            f.steps,
            f.reads
        );
        if f.lookups > f.reads {
            broken.push(format!(
                "{}: its sides' clusters looked the path up {} times, past the {} reads they counted",
                f.what, f.lookups, f.reads
            ));
        }
        let per = f.reads as f64 / f.steps as f64;
        if per > SIDE_READS_A_STEP as f64 || (f.far && per < FAR_READS_A_STEP) {
            broken.push(format!(
                "{}: {} reads over {} counted steps, {per:.2} a step, outside {FAR_READS_A_STEP} to {SIDE_READS_A_STEP}{}",
                f.what,
                f.reads,
                f.steps,
                if f.far { "" } else { " (at the origin, no floor)" }
            ));
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} frames out of the band:\n{}",
        broken.len(),
        frames.len(),
        broken.join("\n")
    );
}

/// How near what its budget holds for their touch check some frame's
/// sides' lays come in the grid (#313): within a copy's hold of it, where
/// the hold of the next copy of a floor's row stopped it — 2 glyph steps,
/// the nearest the grid's frames come.
const EDGE_NEAR: i64 = 1_000;

/// The fewest path reads a counted step of a side ornament's lays takes in
/// a frame far from the origin (#313): the band's floor, below the 2.6 to
/// 3.3 the grid's far frames take (`─═` and `· `), above the 0.2 to 0.4 a
/// frame at the origin takes. A side whose reads went uncounted would take
/// as many reads a step as its clusters take each, 6 to 20 far out, past
/// [`SIDE_READS_A_STEP`], and so would one whose reads were counted at
/// the ring's weight, sixteen to a step; one that counted a read a step,
/// or none, would read a step as [`FAR_READS_A_STEP`] or less.
const FAR_READS_A_STEP: f64 = 2.0;

/// #313: no cluster of a side ornament's lay reads its path more than
/// [`SIDE_CLUSTER_READS`] times, what a lay is held back for before it is
/// laid; and far out a cluster does read it many times — the bound is the
/// one the far frames come near.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_side_reads_no_cluster_reads_past_what_its_lay_holds_back() {
    let frames: Vec<&Laid> = grid().iter().flatten().collect();
    let most = frames.iter().map(|f| f.most).max().unwrap_or(0);
    assert!(
        frames.iter().filter(|f| f.far).all(|f| f.most >= 8),
        "precondition: every far frame has a cluster that reads its path 8 times or more"
    );
    assert!(
        most <= SIDE_CLUSTER_READS,
        "a side's cluster read its path {most} times, past the {SIDE_CLUSTER_READS} its lays are held back for"
    );
}
