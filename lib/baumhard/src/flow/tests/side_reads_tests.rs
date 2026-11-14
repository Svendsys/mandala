// SPDX-License-Identifier: MPL-2.0

//! A frame counts its path reads (#313): its side ornament's lays', and
//! its tracking's past one a cluster (ruling B18.1 §2). Far from the
//! origin, where a cluster reads its path many times where it reads it
//! once at the origin: what a frame takes past its budget, how its
//! counted steps stand to its reads, and the most a cluster reads, what
//! its lays are held back for and its overshoot holds for each line's lay.
//! Every test here sets a test seam or reads a test counter, so none has a
//! `do_*` twin.
//!
//! The far ellipses' paths are as long as their outlines: their tables
//! measure from their centers (#356). Until they did, they summed their
//! points' rounding as length, and their frames laid more copies than an
//! outline that long would hold; nothing here rested on that length —
//! each bound is in counted reads and steps, and each band in reads a
//! step or a cluster.

use glam::Vec2;

use crate::document::values::{FitMode, LineSpan, Percent, Rgba, Shape};
use crate::flow::place::halves::{
    overshoot_steps, ARCS, ARC_EDGE, BUDGET_OVERRIDE, MOST_SIDE_CLUSTER_READS, MOST_SIDE_STEP_LOOKUPS,
    MOST_TRACK_CLUSTER_READS, MOST_TRACK_STEP_LOOKUPS, PAST_BUDGET, SIDE_CLUSTER_READS, SIDE_EDGE,
    SIDE_READS, SIDE_STEPS, SIDE_STEP_LOOKUPS, STEP_LOOKUPS, TRACK_CLUSTER_READS, TRACK_EDGE, TRACK_READS,
    TRACK_READ_STEPS,
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
/// spends it partway through its tracking or its side ornament's lays,
/// far out.
const BUDGETS: [usize; 6] = [2_000, 5_000, 10_000, 20_000, 60_000, 150_000];

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
    /// The most path lookups one step of its sides' lays took
    /// (`MOST_SIDE_STEP_LOOKUPS`, counted apart from the reads).
    most_lookups: usize,
    /// Its tracking (#313, ruling B18.1 §2): the path reads past one a
    /// cluster it counted ([`TRACK_READS`]) and the glyph steps they
    /// counted ([`TRACK_READ_STEPS`]); the glyph steps its tracks took and
    /// the path lookups they took (`STEP_LOOKUPS` less
    /// `SIDE_STEP_LOOKUPS`, counted apart from the reads); and the most
    /// reads one cluster of a track took, and the most lookups one step
    /// did ([`MOST_TRACK_CLUSTER_READS`], `MOST_TRACK_STEP_LOOKUPS`).
    track_reads: usize,
    track_read_steps: usize,
    track_steps: usize,
    track_lookups: usize,
    most_track: usize,
    most_track_lookups: usize,
    /// The most one of its tracks counted past what a frame's overshoot
    /// holds for a line's lay ([`TRACK_EDGE`]); `None` where none was
    /// laid.
    track_edge: Option<i64>,
    /// The nearest an arc's anchor read left it to its budget past what
    /// the arc held back for ([`ARC_EDGE`]); `None` where none was taken.
    arc_edge: Option<i64>,
    /// The arcs of its sides it took ([`ARCS`]), each an anchor read.
    arcs: usize,
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
    let counters = [
        &SIDE_READS,
        &SIDE_STEPS,
        &MOST_SIDE_CLUSTER_READS,
        &PAST_BUDGET,
        &MOST_SIDE_STEP_LOOKUPS,
        &TRACK_READS,
        &TRACK_READ_STEPS,
        &MOST_TRACK_CLUSTER_READS,
        &MOST_TRACK_STEP_LOOKUPS,
    ];
    for counter in counters {
        counter.with(|count| count.set(0));
    }
    SIDE_EDGE.with(|edge| edge.set(None));
    TRACK_EDGE.with(|edge| edge.set(None));
    ARC_EDGE.with(|edge| edge.set(None));
    ARCS.with(|count| count.set(0));
    let (side_steps_before, lookups_before) = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
    let (steps_before, all_lookups_before) = STEP_LOOKUPS.with(std::cell::Cell::get);
    let lay = || frame_with(TEXT, POINTS, shape, &fit, &layout, true);
    let (_, placed, trace): (_, PlacedRun, Trace) = match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    };
    let [reads, steps, most, past, most_lookups, track_reads, track_read_steps, most_track, most_track_lookups] =
        counters.map(|counter| counter.with(std::cell::Cell::get));
    let (side_steps, side_lookups) = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
    let (all_steps, all_lookups) = STEP_LOOKUPS.with(std::cell::Cell::get);
    Laid {
        what: String::new(),
        far: false,
        past,
        spent: trace.spent,
        budget: trace.budget,
        allowed,
        reads,
        steps,
        lookups: side_lookups - lookups_before,
        most,
        most_lookups,
        track_reads,
        track_read_steps,
        track_steps: (all_steps - steps_before) - (side_steps - side_steps_before),
        track_lookups: (all_lookups - all_lookups_before) - (side_lookups - lookups_before),
        most_track,
        most_track_lookups,
        track_edge: TRACK_EDGE.with(std::cell::Cell::get),
        arc_edge: ARC_EDGE.with(std::cell::Cell::get),
        arcs: ARCS.with(std::cell::Cell::get),
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
/// back the most a lay may read before it is laid, and its tracking counts
/// its lines' reads past one a cluster, which its overshoot holds for each
/// line's lay (ruling B18.1 §2), so a frame far from the origin, whose
/// clusters read their path 11 to 20 times where at the origin they read
/// it once, takes past its budget no more than `overshoot_steps` — `PAST_BUDGET`, a take that crosses the budget
/// counted for the part past it (ruling R50.4) — and in all no more than
/// its budget and that (ruling R47.2): no lay of its sides leaves it past
/// what its budget holds for their touch check (crossed). And a frame
/// that lays fewer glyphs than at its own budget says which arcs it left
/// empty or kept short.
///
/// Preconditions: a frame far out spends its budget partway through its
/// sides, at some budget of the grid; and at the grid's least budgets,
/// before them, in its tracking.
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
    // hold of the edge: the edge is measured, and the hold is no more than
    // a copy's.
    let nearest = frames.iter().filter_map(|f| f.edge).max();
    assert!(
        nearest.is_some_and(|edge| edge >= -COPY_HOLD),
        "some frame's sides' lays come within a copy's hold, {COPY_HOLD} glyph steps, of what its budget holds for their check ({nearest:?})"
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
            if f.arc_edge.is_some_and(|edge| edge > 0) {
                broken.push(format!(
                    "{}: an arc's read left it {:?} glyph steps past what the arc held back for, a copy and its check (arc)",
                    f.what, f.arc_edge
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
/// their clusters make, counted apart from the reads by the lookups' own
/// counter, is among them — and their counted steps stand to their reads
/// within a pinned band: at most [`MOST_READS_A_STEP`] reads a counted
/// step, every read counted at the weight ruling R47.3's measure gave
/// past the step a cluster, and far out at least [`FAR_READS_A_STEP`],
/// where their clusters read the path many times — where, counted a step
/// a cluster, they read it up to 20 times a step (and 32 to 53, review
/// 5761697746 found, before #309).
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_side_reads_counted_steps_stand_to_their_reads_within_a_band() {
    let frames: Vec<&Laid> = grid().iter().flatten().filter(|f| f.steps > 0).collect();
    for case in grid() {
        let own = case.last().expect("every case lays its frame at its own budget");
        assert!(
            own.steps > 0 && own.reads > 0,
            "precondition: {} lays its sides ({} steps, {} reads)",
            own.what,
            own.steps,
            own.reads
        );
    }
    let mut broken = Vec::new();
    for f in &frames {
        if f.lookups + f.arcs > f.reads {
            broken.push(format!(
                "{}: its sides' clusters looked the path up {} times and its {} arcs read their anchors, past the {} reads they counted",
                f.what, f.lookups, f.arcs, f.reads
            ));
        }
        let per = f.reads as f64 / f.steps as f64;
        if per > MOST_READS_A_STEP || (f.far && per < FAR_READS_A_STEP) {
            broken.push(format!(
                "{}: {} reads over {} counted steps, {per:.2} a step, outside {FAR_READS_A_STEP} to {MOST_READS_A_STEP}{}",
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

/// The most a lay of one copy of the grid's ornaments is held back for
/// (#313, review 5902086644): a step each of its two clusters, and its
/// reads, [`SIDE_CLUSTER_READS`] a cluster at four to a step —
/// `SideOrnament::copy_steps`, worked out here apart from it: 2 + 15 =
/// 17. Some frame's sides' lays, stopped by the hold of their next copy,
/// come within it of what its budget holds for their touch check — 6
/// glyph steps, the nearest the grid's frames come — so a lay held back
/// for more than a copy needs, which would leave arcs empty that a copy
/// fits, turns the test red, as does an edge recorded short of where it
/// is.
const COPY_HOLD: i64 = 2 + (2 * SIDE_CLUSTER_READS as i64 + 3) / 4;

/// The most path reads a counted step of a side ornament's lays takes
/// (#313): the weight of a read, four to a step, that ruling R47.3's
/// measure on the ASan build gave (`SIDE_READS_A_STEP`), held as a number
/// of its own, so that the weight's constant moved — to the ring's
/// sixteen, say — turns the band red rather than moving it.
const MOST_READS_A_STEP: f64 = 4.0;

/// The fewest path reads a counted step of a side ornament's lays takes in
/// a frame far from the origin (#313): the band's floor, below the 2.6 to
/// 3.3 the grid's far frames take (`─═` and `· `), above the 0.2 to 0.4 a
/// frame at the origin takes. A side whose reads went uncounted would take
/// as many reads a step as its clusters take each, 6 to 20 far out, past
/// [`MOST_READS_A_STEP`], and so would one whose reads were counted at
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
    // Held from below by a count apart from the reads: no cluster reads
    // its path fewer times than one of its steps looked it up; and far out,
    // at its own budget, as many times as the grid's far frames read it,
    // 21 or 22.
    for case in grid() {
        let own = case.last().expect("every case lays its frame at its own budget");
        assert!(
            !own.far || own.most >= 20,
            "{}: far out, no cluster of its sides read its path 20 times or more ({})",
            own.what,
            own.most
        );
    }
    for f in &frames {
        assert!(
            f.most >= f.most_lookups,
            "{}: the most reads a cluster of its sides counted, {}, is short of the {} lookups one of their steps took",
            f.what,
            f.most,
            f.most_lookups
        );
    }
    assert!(
        most <= SIDE_CLUSTER_READS,
        "a side's cluster read its path {most} times, past the {SIDE_CLUSTER_READS} its lays are held back for"
    );
}

/// The weight of a tracking read past the first a cluster (#313, ruling
/// B18.1 §2): eight to a step, what ruling R47.3's measure on the ASan
/// build gave (`TRACK_READS_A_STEP`), held as a number of its own so that
/// the weight's constant moved turns the test red rather than moving it.
const TRACK_READS_WEIGHT: usize = 8;

/// #313, ruling B18.1 §2: a frame's tracking counts its path's reads past
/// one a cluster — exactly the lookups its tracks' steps took past one a
/// step, counted apart by the lookups' own counter (`STEP_LOOKUPS` less
/// `SIDE_STEP_LOOKUPS`) — at [`TRACK_READS_WEIGHT`] to a counted step,
/// rounded up over the frame. Far out its lines' clusters read their path
/// 11 to 13 times past the first (review 5902086644), at the origin less
/// than once in four.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_track_reads_a_frame_counts_its_lines_reads_past_one_a_cluster() {
    let frames: Vec<&Laid> = grid().iter().flatten().collect();
    let mut broken = Vec::new();
    for f in &frames {
        assert!(f.track_steps > 0, "precondition: {} tracks its lines", f.what);
        if f.track_reads + f.track_steps != f.track_lookups {
            broken.push(format!(
                "{}: its tracks counted {} reads past one a cluster over {} steps, where they looked the path up {} times",
                f.what, f.track_reads, f.track_steps, f.track_lookups
            ));
        }
        if f.track_read_steps != f.track_reads.div_ceil(TRACK_READS_WEIGHT) {
            broken.push(format!(
                "{}: {} reads counted {} glyph steps, not {} at {TRACK_READS_WEIGHT} to a step",
                f.what,
                f.track_reads,
                f.track_read_steps,
                f.track_reads.div_ceil(TRACK_READS_WEIGHT)
            ));
        }
        let per = f.track_reads as f64 / f.track_steps as f64;
        if (f.far && per < 10.0) || (!f.far && per > 0.5) {
            broken.push(format!(
                "{}: {per:.2} reads past the first a cluster, {}",
                f.what,
                if f.far {
                    "below 10 far out"
                } else {
                    "above 0.5 at the origin"
                }
            ));
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} frames:\n{}",
        broken.len(),
        frames.len(),
        broken.join("\n")
    );
}

/// #313, ruling B18.1 §2: no cluster of a frame's [`track`] reads its path
/// more than [`TRACK_CLUSTER_READS`] times, what a frame's overshoot holds
/// for each line's lay; held from below by the lookups a step of a track
/// took, counted apart, and far out a cluster reads it 21 or 22 times. And
/// no track counts more than that overshoot holds for it
/// ([`TRACK_EDGE`]), while far out one comes within half of it.
///
/// [`track`]: crate::flow::place::halves
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_track_reads_no_line_lay_counts_past_what_its_overshoot_holds() {
    let frames: Vec<&Laid> = grid().iter().flatten().collect();
    for f in &frames {
        assert!(
            f.most_track <= TRACK_CLUSTER_READS,
            "{}: a cluster of a track read its path {} times, past the {TRACK_CLUSTER_READS} its overshoot holds",
            f.what,
            f.most_track
        );
        assert!(
            f.most_track >= f.most_track_lookups,
            "{}: the most reads a cluster of a track counted, {}, is short of the {} lookups one step took",
            f.what,
            f.most_track,
            f.most_track_lookups
        );
        assert!(
            !f.far || f.most_track >= 20,
            "{}: far out, no cluster of a track read its path 20 times or more ({})",
            f.what,
            f.most_track
        );
        let edge = f.track_edge.expect("every frame tracks a line");
        assert!(
            edge <= 0,
            "{}: a track counted {edge} glyph steps past what its overshoot holds",
            f.what
        );
    }
    let far_nearest = frames.iter().filter(|f| f.far).filter_map(|f| f.track_edge).max();
    assert!(
        far_nearest.is_some_and(|edge| edge >= -TRACK_NEAR),
        "far out, some track comes within {TRACK_NEAR} glyph steps of what its overshoot holds ({far_nearest:?})"
    );
}

/// How near what a frame's overshoot holds for a line's lay some far
/// frame's track comes (#313, ruling B18.1 §2): the grid's far frames'
/// nearest come 17 to 25 glyph steps short of it, its frames at the
/// origin 43 or more. A hold its reads do not reach — a weight or a bound
/// of reads a cluster too large — turns the test red, as one too small
/// does past it.
const TRACK_NEAR: i64 = 24;

/// #313, ruling B18.1 §3: a model lay of a side's copies counts its
/// clusters at [`MODEL_CLUSTERS_A_STEP`](crate::flow::place::halves::MODEL_CLUSTERS_A_STEP)
/// — two — to a step, rounded up once over the lay, as it did before its
/// copies were each read against the budget: an ornament of one cluster,
/// `x`, whose copies each rounded up on their own would count a step a
/// cluster, twice its weight. Round an ellipse 2,000 across at the origin,
/// where its lays are modeled.
///
/// Controls: see the pull request of #313.
#[test]
fn test_flow_side_reads_a_model_lay_counts_its_clusters_rounded_once() {
    use crate::flow::place::halves::MODEL_LAYS;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(2_000.0)), Shape::Ellipse);
    MODEL_LAYS.with(|lays| lays.borrow_mut().clear());
    let _ = frame("x", &shape, None);
    let lays = MODEL_LAYS.with(|lays| lays.borrow().clone());
    assert!(
        lays.iter().any(|&(clusters, _)| clusters >= 2),
        "precondition: a model lay lays two copies or more ({} lays)",
        lays.len()
    );
    for (clusters, steps) in lays {
        assert_eq!(
            steps,
            clusters.div_ceil(2),
            "a model lay of {clusters} clusters counted {steps} glyph steps past its reads"
        );
    }
}
