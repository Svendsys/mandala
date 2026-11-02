// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33, R33.1, R34, R35 and R38).
//!
//! The owner, on the frame text: "It should follow the path of its
//! halve, this should be adjustable in the solution. But the point here
//! was to demo the text actually taking the shape of our nodes."
//!
//! **The halves** (ruling R33). The outline is split on its diagonal, at
//! its support points in the directions `(−1, −1)` and `(1, 1)`
//! ([`Outline::support`]): the point where the tangent is at 45° on an
//! ellipse, the corner arc's midpoint on a rounded rectangle — the
//! reading of R33's "in its own aspect" that keeps every horizontal run
//! reading rightward, where the bounding box's diagonal would start the
//! second half leftward on a wide ellipse. The first line takes the
//! upper-right half — from the top-left junction clockwise over the top
//! and down the right side to the bottom-right junction — and the second
//! the lower-left half — from the top-left junction down the left side
//! and along the bottom to the bottom-right one. So every vertical run
//! reads down and every horizontal one reads right, glyphs upright. Each
//! line carries its half with it: a text that opens with a line break
//! has one line, on the second half.
//!
//! **The lines** (rulings R27, R34). R27 parts the text and sizes a
//! `Fit` frame ([`place_lines`]); a `Once` or `Spread` frame's lines are
//! parted as R27 parts them but not cut to its level span
//! ([`line_partition`]).
//!
//! **The span** (ruling R32, [`LineSpan`]). At [`LineSpan::NATURAL`]
//! each line lies at its natural width: tracked at its font's own gap,
//! centered on its half's apex (where R27 centers it), and
//! slid along its half where that would reach into a junction's
//! clearance. At [`LineSpan::WHOLE`] each end is at its junction less
//! the clearance; in between, each end moves by arc length that part of
//! the way from natural to whole, so the ends move only outward.
//!
//! **Even tracking.** Over the part it covers, a line is laid with one
//! ink gap `g` between every two neighboring letters, its letters' side
//! bearings left out, and a space's advance plus `g` more between words:
//! a word gap is `2g` plus a space. Each letter after the first is set
//! by the pairwise rule (ruling R31), as far from the last letter's ink
//! as owed, a zero gap included, so the line's length grows with `g`,
//! nearly as a line does, and `g` is solved by regula falsi from the
//! straight path's answer — or from the line's last lay in the same
//! clearance search — to the precision its letters are set to: the
//! line's first and last inks centered on its part's ends ([`track`],
//! [`solve_tracking`]). The rule is solved for each letter from where
//! the two inks would be owed apart were the path straight on, the last
//! letter's share of that its warm start, within a bracket of centers
//! too near and far enough ([`pair`], ruling R43): a lookup or a few a
//! letter, 64 at most. Round a sharp corner, where no center sets a
//! letter exactly `g` from the one before, it is set at the nearest
//! center tried past that is at least `g` away, and clear by `g` of the
//! few letters before its neighbor ([`EARLIER`]). A side ornament's
//! copies are set by the same rule ([`InkLay`]). A glyph with no
//! outline steps as its box; a line of one letter sits centered on its
//! part.
//!
//! **Each junction is a visible break** (rulings R33.1, R35.1). At every
//! span the two lines' end inks at a junction are at least two word
//! gaps apart, box to box, of the mean of the lines that have one (a
//! line of one word has none: the span stretches its letter gaps
//! alone), and with `sides` one whole copy of the ornament fits between
//! them; two end inks whose outward normals turn more than 135° apart
//! are lines stacked across a thin outline, and need only not touch.
//! Each line keeps its own clearance from each junction, grown round by
//! round among the lines whose drawing back parts the ends there, at
//! whole and then on at the span laid ([`Layout::lay_clear`]): each
//! round's step damped by how much drawing back eases the break wanted,
//! and no line drawn back past its slack while the other line has room
//! to give (rulings R38.1, R38.3).
//!
//! **No line is laid tighter than its font, and no two inks touch**
//! (rulings R34, R35.2). Where the halves do not hold the frame so — a
//! line its half less the clearance cannot hold at its font's gap, a
//! junction left short of a break, two inks touching — the last resorts
//! follow: a `Fit` frame is shrunk, within its bounds; a line held even
//! so by nothing is cut at its end, the placement reporting it; two
//! inks that still touch — an outline too thin to hold the two lines
//! apart — have the later line cut, or left out, the trace saying so
//! ([`Trace::cut_apart`], ruling R38.1: never
//! overlap wins over never cut); and what is left unheld even so is
//! reported in the trace ([`Unheld`]), never silent.
//! Every last resort is
//! bounded in work (rulings R35.3, R44.2, R46): a shrink's probe looks
//! first whether its lines fit at all, and a frame's budget,
//! [`STEPS_PER_GLYPH`] glyph steps a glyph of its text and at least
//! [`TRACK_BUDGET`] ([`frame_budget`]), counts all of its work — its
//! tracking and side lays, its shaping and inking, its touch sweeps and
//! R27's walks, each weighted to about one tracking step's time — and is
//! read before every lay, build and reshape — in each tracking solve,
//! slide, shrink, cut and backtrack, and R27's fit — so a frame
//! overshoots it by at most one build's shaping and inking and a lay of
//! each line ([`overshoot_steps`], ruling R47.2). A frame whose budget
//! runs out while two of its inks still touch keeps the longest run of
//! its inks that touch nothing ([`untouched`], within a share of the
//! budget held back for it), and says it was cut ([`Trace::cut_apart`]). A frame's text is cut to
//! [`MAX_PLACED_GLYPHS`] at a cluster before its lines are prepared
//! (ruling R44.1), its sides placing no more than the cap leaves, so the
//! budget is at most `STEPS_PER_GLYPH × MAX_PLACED_GLYPHS`.
//!
//! **Sides.** With `sides`, an ornament closes what the lines leave
//! round each junction: whole copies, at least one and at most
//! [`MAX_SIDE_COPIES`], every gap and both joins to the text equal
//! (ruling R31's lay, seeded by the text's end inks), on the text's own
//! center line, the ornament's ink no taller than the text's x-height.
//! The ornament is shaped into the run as a line of its own, the way a
//! `Once` filler is.

use std::cell::Cell;
use std::cmp::Ordering;
use std::ops::Range;

use glam::Vec2;

use super::lines::{frame_lines, line_partition, place_lines_in};
use super::*;
use crate::document::values::{Edge, FontSize};
use crate::flow::shape::shape_run;
use crate::flow::span::FlowSpan;
use crate::font::fonts::glyph_outline_ink;
use crate::font::SwashCache;

/// The most probes a line's tracking bracket is closed in ([`falsi`]):
/// as many as a bisection of a few points to some millionths of one
/// takes, though a line's length, near linear in its gap, closes in a
/// few.
const TRACK_ROUNDS: usize = 20;

/// The most times a line's tracking bracket widens looking for a gap
/// too wide to fit ([`solve_tracking`]): past about `2^24` points nothing
/// is laid.
const TRACK_DOUBLINGS: usize = 24;

/// The most rounds a natural line is slid back from its bottom-right
/// junction in: each a secant probe of the start whose line ends at the
/// clearance, its length changing only with the path's bend
/// ([`Layout::slid`]).
const SLIDE_ROUNDS: usize = 16;

/// How near the clearance a slid line's end must come, in points, where
/// the path's floats resolve that ([`resolved`]).
const SLIDE_TOLERANCE: f32 = 1e-4;

/// How many float steps of the positions compared a tolerance is held
/// to at the least ([`resolved`]).
const RESOLUTION_STEPS: f32 = 2.0;

/// `tolerance`, or [`RESOLUTION_STEPS`] float steps of a position `at`
/// along a path where those are wider: a line's end and its part's end
/// some 2,400 points round an outline are a float step — 2.4e-4 points —
/// apart at the nearest, so a tolerance of 1e-4 there can never be met,
/// and a solve that waits for it runs out its rounds and keeps a worse
/// answer than the one it had (a slide left a line a float step too
/// long for its part, read as pressed; ruling R43).
fn resolved(tolerance: f32, at: f32) -> f32 {
    let step = at.abs() * f32::EPSILON;
    if step.is_finite() {
        tolerance.max(RESOLUTION_STEPS * step)
    } else {
        tolerance
    }
}

/// The rounds a junction's clearance is grown in.
const CLEAR_ROUNDS: usize = 16;

/// The most copies of a side ornament one junction holds.
const MAX_SIDE_COPIES: usize = 256;

/// The most probes a side arc's gap is solved in past its bracket
/// ([`falsi`], ruling R44.3).
const SIDE_ROUNDS: usize = 30;

/// How narrow a side arc's gap is solved, in points, where the path's
/// floats resolve that ([`resolved`]): a thousandth of a point, the
/// pairwise rule's own tolerance (ruling R43: thirty rounds closed a
/// bracket of some points to some millionths of one).
const SIDE_PRECISION: f32 = INK_TOLERANCE;

/// How many word gaps apart two lines' end inks stay at a junction
/// (ruling R33.1).
pub(crate) const JUNCTION_WORD_GAPS: f32 = 2.0;

/// How far over the clearance the rounds aim, so a measure of it never
/// rounds below.
const CLEAR_MARGIN: f32 = 1.01;

/// The least a junction's clearance grows by in a round, in points.
const MIN_GROWTH: f32 = 0.25;

/// How many times a round of the clearance that would press a line is
/// halved before it is taken anyway.
const BACKTRACKS: usize = 6;

/// How many times a shrink's probe halves such a round (ruling R35.3):
/// a probe asks only whether its size can be held, and a round that
/// presses a line after one halving says, near enough, that it cannot.
const PROBE_BACKTRACKS: usize = 1;

/// The most probes a natural line is centered on its apex in
/// ([`falsi`]), and how near equal its two arcs either side must come,
/// in points.
const CENTER_ROUNDS: usize = 40;
const CENTER_TOLERANCE: f32 = 1e-3;

/// How far below its font's own gap a line's solved gap may fall and
/// still have it: the bisection's precision, in points.
const GAP_TOLERANCE: f32 = 1e-4;

/// The least gap a side ornament's copy is laid at, in points: past
/// [`INK_TOLERANCE`], so the pairwise rule always sets a copy clear of
/// the ink before it — an ornament with no side bearings (`─`, `█`)
/// included — rather than where its box would fall.
const JOIN_FLOOR: f32 = 4.0 * INK_TOLERANCE;

/// The rounds of bisection a `Fit` frame is shrunk in, when its halves
/// cannot hold its lines at the size R27 gave it.
const SHRINK_ROUNDS: usize = 12;

/// The shrink stops once its bracket on the scale is within this share
/// of the scale held (ruling R42.2).
const SHRINK_PRECISION: f32 = 0.01;

/// The shrink stops once its bracket spans less than this size step, in
/// points, at the frame's largest glyph (ruling R42.2).
const SHRINK_LEAST_STEP_PT: f32 = 0.25;

/// How much of the clearance of the largest size a shrink has found
/// held a probe at a larger size starts from (ruling R35.3): a clearance
/// grows with the word gap, so with the size — or stays, where a wide
/// space of no size of its own sets it — and the probe's rounds grow it
/// the rest of the way, from below.
const SEED_SHARE: f32 = 0.9;

/// The least glyph steps a frame's work may take in all, each glyph a
/// [`track`] or a side's lay sets counting one (rulings R35.3, R44.2):
/// the floor of its budget ([`frame_budget`]), which scales with its text
/// past it. A frame that reaches its budget keeps the best it has laid —
/// the largest size its shrink found clean, the cuts made so far, its
/// inks cut to those that touch nothing — and its trace reports what is
/// left unheld.
///
/// Ten times the most any real frame took while a step counted tracking
/// alone (rulings R42.3, R43.3): the aspect sweep's worst, 36,748 glyph
/// steps — the quatrain outward round the 600 × 12 rectangle, `Once` at
/// span ½, its sides' lays counted — rounded up to 40,000; the demo
/// maps' frames took under 2,400. At 30411a6e it was 2^21, and the
/// worst real frame took 306,098. Since a step counts all of a frame's
/// work (ruling R46), the sweep's worst, the quatrain inward round the
/// 50:1 ellipse, `Fit`, takes 59,468: the budget is unchanged, six times
/// that (ruling R47.1), so the fuzzers' per-input bound holds (R45).
pub(crate) const TRACK_BUDGET: usize = 400_000;

/// The glyph steps a frame's work may take a glyph of its text, past
/// [`TRACK_BUDGET`] (ruling R44.2): ten times the most any frame of the
/// aspect sweep took a glyph, rounded up, while a step counted tracking
/// alone; five times its 597 since a step counts all of a frame's work
/// (rulings R46, R47.1). A frame's work grows with its
/// text, so a fixed count would cut real text past some length — a
/// 2,000-word frame took more than 400,000 — where this bounds the work
/// a glyph, and the glyph cap ([`MAX_PLACED_GLYPHS`]) bounds the glyphs.
pub(crate) const STEPS_PER_GLYPH: usize = 3_000;

/// A frame's work budget, in glyph steps, for a text of `glyphs`
/// glyphs: [`TRACK_BUDGET`], or [`STEPS_PER_GLYPH`] a glyph where that
/// is more. At most `STEPS_PER_GLYPH × MAX_PLACED_GLYPHS`.
pub(crate) fn frame_budget(glyphs: usize) -> usize {
    #[cfg(test)]
    if let Some(budget) = BUDGET_OVERRIDE.with(Cell::get) {
        return budget;
    }
    #[cfg(test)]
    let floor = FLOOR_OVERRIDE.with(Cell::get).unwrap_or(TRACK_BUDGET);
    #[cfg(not(test))]
    let floor = TRACK_BUDGET;
    floor.max(STEPS_PER_GLYPH.saturating_mul(glyphs.min(MAX_PLACED_GLYPHS)))
}

// What a frame's work is counted as, in glyph steps (rulings R46.1,
// R47.3). Each weight is the most one unit of that work took under the
// ASan fuzz build, divided by 16 µs — about the most a glyph step of
// [`track`] took — and rounded up, so that one counted step takes about
// the same time whatever work it counts. Measured at e301e041, at a load
// average of 1 to 2: each call of the work timed by a temporary
// instrumentation of the engine, never committed, over every committed
// flow fuzz finding and seed and the heaviest frames a sweep of 2,600
// seeded structured inputs drew, and an upper envelope fitted to each
// call's time over its call, its glyphs and its faces (315 shapings,
// 381 inkings). The PR body of #294 has the table.

/// A [`track`], past its clusters (a step each): its setup, and the
/// lay's look at its ends round it — 11.5 µs a call ÷ 16 µs, rounded up.
pub(crate) const TRACK_CALL_STEPS: usize = 1;

/// Shaping a run, or reshaping one (`reshape`, `shape_run` in
/// `side_run`), a call: 435 µs ÷ 16 µs = 27.2, rounded up.
pub(crate) const SHAPE_CALL_STEPS: usize = 28;

/// A glyph shaped: 261 µs ÷ 16 µs = 16.3, rounded up.
pub(crate) const SHAPE_GLYPH_STEPS: usize = 17;

/// A face a shaped run's glyphs were found in: 5.2 ms ÷ 16 µs = 326,
/// rounded up to 330.
pub(crate) const SHAPE_FACE_STEPS: usize = 330;

/// A glyph shaped that no face holds (`.notdef`, glyph 0), which sends
/// the shaper through every face it has: 34.8 ms ÷ 16 µs = 2,173,
/// rounded up to 2,200. (That cost is a defect of its own, #304.)
pub(crate) const SHAPE_NOTDEF_STEPS: usize = 2_200;

/// Inking a run ([`inked`], each glyph's outline read uncached), a call:
/// 1.41 ms ÷ 16 µs = 88.2, rounded up.
pub(crate) const INK_CALL_STEPS: usize = 89;

/// A glyph inked: 83 µs ÷ 16 µs = 5.2, rounded up.
pub(crate) const INK_GLYPH_STEPS: usize = 6;

/// A face a run's inked glyphs are drawn in: 1.66 ms ÷ 16 µs = 103.8,
/// rounded up.
pub(crate) const INK_FACE_STEPS: usize = 104;

/// Ink boxes one glyph step weighs where two inks are looked at for a
/// touch ([`touching`]): some 0.4 µs a box, the sort and the sweep, so
/// 16 µs ÷ 0.4 µs = 40 boxes, rounded down to 32.
pub(crate) const BOXES_A_STEP: usize = 32;

/// Clusters one glyph step walks in R27's arithmetic lays
/// (`lines::Lines`): some 1.3 µs a cluster along a path, so 16 µs ÷
/// 1.3 µs = 12 clusters, rounded down to 8.
pub(crate) const CLUSTERS_A_STEP: usize = 8;

/// A frame's own setup, past what it counts — its halves' tables, its
/// partition, its runs cloned — some 1 ms ÷ 16 µs, rounded up.
pub(crate) const FRAME_STEPS: usize = 64;

/// The distinct faces `run`'s glyphs are drawn in, and how many of its
/// glyphs no face holds.
fn faces_and_notdefs(run: &ShapedRun) -> (usize, usize) {
    let mut faces = Vec::new();
    let mut notdefs = 0;
    for glyph in run.layout_glyphs() {
        notdefs += usize::from(glyph.glyph_id == 0);
        if !faces.contains(&glyph.font_id) {
            faces.push(glyph.font_id);
        }
    }
    (faces.len(), notdefs)
}

/// The glyph steps shaping `run` counts (ruling R46.1).
pub(crate) fn shaping_steps(run: &ShapedRun) -> usize {
    let (faces, notdefs) = faces_and_notdefs(run);
    SHAPE_CALL_STEPS
        .saturating_add(SHAPE_GLYPH_STEPS.saturating_mul(run.glyphs.len()))
        .saturating_add(SHAPE_FACE_STEPS.saturating_mul(faces))
        .saturating_add(SHAPE_NOTDEF_STEPS.saturating_mul(notdefs))
}

/// The glyph steps inking `run` counts (ruling R46.1).
pub(crate) fn inking_steps(run: &ShapedRun) -> usize {
    let (faces, _) = faces_and_notdefs(run);
    INK_CALL_STEPS
        .saturating_add(INK_GLYPH_STEPS.saturating_mul(run.glyphs.len()))
        .saturating_add(INK_FACE_STEPS.saturating_mul(faces))
}

/// How many [`track`]s of every glyph a frame may take past its spent
/// budget: a lay of each line — the slide's and the solve's probe that
/// notices the budget is spent, and the check at the font's gap — and
/// the look at whether its lines fit (ruling R44.2; pinned by
/// `test_flow_halves_a_spent_budget_is_overshot_by_a_lay_at_most`).
pub(crate) const OVERSHOOT_TRACKS: usize = 8;

/// The most glyph steps one build of a frame's text counts in shaping
/// and inking (ruling R47.2), for a text shaped as `text` and, with
/// sides, an ornament shaped alone as `ornament`: the text reshaped at a
/// size and inked; with sides, the ornament and an `x` shaped and inked
/// to size it, and shaped and inked again beside the text, which is
/// shaped again with it (`side_run`). Each run's own faces and
/// `.notdef`s are counted, as the build counts them.
pub(crate) fn build_steps(text: &ShapedRun, ornament: Option<&ShapedRun>) -> usize {
    let alone = shaping_steps(text).saturating_add(inking_steps(text));
    let Some(ornament) = ornament else {
        return alone;
    };
    let x = SHAPE_CALL_STEPS
        + SHAPE_GLYPH_STEPS
        + SHAPE_FACE_STEPS
        + INK_CALL_STEPS
        + INK_GLYPH_STEPS
        + INK_FACE_STEPS;
    let beside = shaping_steps(ornament).saturating_add(inking_steps(ornament));
    alone
        .saturating_add(shaping_steps(text))
        .saturating_add(x)
        .saturating_add(beside.saturating_mul(2))
}

/// The most glyph steps a frame of `text` may take past its budget
/// (ruling R47.2): what runs before its budget is first read — its own
/// setup ([`FRAME_STEPS`]) and, for a `Fit` frame (`sized`), R27's
/// reshape at the size it fits — one build's shaping and inking
/// ([`build_steps`]), and a lay of each line ([`OVERSHOOT_TRACKS`]
/// `track`s of every glyph). A budget is read before each build, each
/// reshape of R27's fit and each lay, so no more than one of each runs
/// past it; [`untouched`] runs within the budget, which holds its share
/// back.
pub(crate) fn overshoot_steps(text: &ShapedRun, ornament: Option<&ShapedRun>, sized: bool) -> usize {
    let r27 = if sized { shaping_steps(text) } else { 0 };
    FRAME_STEPS
        .saturating_add(r27)
        .saturating_add(build_steps(text, ornament))
        .saturating_add(OVERSHOOT_TRACKS.saturating_mul(text.glyphs.len() + TRACK_CALL_STEPS))
}

/// The most glyph steps a frame of `text` may take in all: its budget
/// ([`frame_budget`]) and the most it may take past it
/// ([`overshoot_steps`]). What the flow fuzzers hold an input's frames
/// to, at their cost a step under ASan (ruling R45.1).
pub(crate) fn frame_work_steps(text: &ShapedRun, ornament: Option<&ShapedRun>, sized: bool) -> usize {
    frame_budget(text.glyphs.len()).saturating_add(overshoot_steps(text, ornament, sized))
}

/// The glyph steps a look at `boxes` ink boxes for a touch counts.
fn weighing_steps(boxes: usize) -> usize {
    boxes.div_ceil(BOXES_A_STEP).max(1)
}

/// The glyph steps [`untouched`] may take over `boxes` inks: a
/// bisection of looks at them, each with its lines cut again — what a
/// frame's budget holds back for it (ruling R46.1).
pub(crate) fn untouched_steps(boxes: usize) -> usize {
    let probes = (usize::BITS - boxes.leading_zeros()) as usize + 1;
    probes.saturating_mul(2 * weighing_steps(boxes))
}

/// The most glyphs a frame places, its text and its sides together
/// ([`MAX_PLACED_GLYPHS`], ruling R44.1).
fn glyph_cap() -> usize {
    #[cfg(test)]
    if let Some(cap) = CAP_OVERRIDE.with(Cell::get) {
        return cap;
    }
    MAX_PLACED_GLYPHS
}

/// How far apart the outward normals of two lines' end inks at a
/// junction must turn, as a cosine, for the two to be lines stacked
/// across a thin outline rather than lines meeting there (ruling R35.1):
/// 135°.
const STACKED: f32 = -std::f32::consts::FRAC_1_SQRT_2;

/// How far from the outline's normal, as a cosine, the way from the
/// outline to an ink off it must turn for the ink to lie past a sharp
/// corner ([`Layout::normals`]): a degree. On a smooth outline the way
/// from the point nearest an ink to the ink is the normal there.
const CORNER: f32 = 0.999_847_7;

/// How far apart two inks must be not to touch, in points: past
/// [`INK_TOLERANCE`], what the pairwise rule sets inks apart to.
const TOUCH: f32 = INK_TOLERANCE;

/// The most times a frame's lines are cut and the clearance found
/// again: a cut line, stretched over its half at whole, has wider word
/// gaps and so wants more clearance, which a round more settles.
const CUT_ROUNDS: usize = 8;

/// The outline's two halves (ruling R33): each its path, the part of it
/// between its junctions, and its apex.
struct Halves {
    /// The outline walked from the middle of its left side, clockwise
    /// (the first half's) and counterclockwise (the second's).
    paths: [ArcPath; 2],
    /// Each half's junctions along its path, top-left then bottom-right,
    /// the second past the first.
    junctions: [(f32, f32); 2],
    /// Each half's apex along its path, between its junctions: the
    /// outline's support point straight up for the first, straight down
    /// for the second — where R27 centers a line.
    apexes: [f32; 2],
    /// The outline's length.
    length: f32,
}

impl Halves {
    fn new(outline: &Outline, spacing: f32) -> Self {
        let left = StartPoint {
            edge: Edge::Left,
            at: 0.5,
        };
        let paths = [
            ArcPath::closed(outline, left, Direction::Clockwise, spacing),
            ArcPath::closed(outline, left, Direction::CounterClockwise, spacing),
        ];
        let length = paths[0].length();
        let top_left = outline.support(Vec2::new(-1.0, -1.0));
        let bottom_right = outline.support(Vec2::new(1.0, 1.0));
        let tops = [
            outline.support(Vec2::new(0.0, -1.0)),
            outline.support(Vec2::new(0.0, 1.0)),
        ];
        let mut junctions = [(0.0, 0.0); 2];
        let mut apexes = [0.0; 2];
        for k in 0..2 {
            // A support that is not a number — an outline with a side
            // that is not one — is nowhere along the path, though the
            // point nearest it would be somewhere: its place is not a
            // number either, so the halves read as unmeasured.
            // A support is a point of the outline: found from the path's
            // own geometry, at no lookup (ruling R43).
            let along = |p: Vec2| {
                if p.is_finite() {
                    paths[k].locate(p).unwrap_or_else(|| paths[k].nearest(p))
                } else {
                    f32::NAN
                }
            };
            let start = along(top_left);
            let end = unwrap(along(bottom_right), start, length);
            junctions[k] = (start, end);
            apexes[k] = unwrap(along(tops[k]), start, length);
        }
        Halves {
            paths,
            junctions,
            apexes,
            length,
        }
    }

    /// Where along half `k` the point nearest `p` lies, past its start.
    fn along(&self, k: usize, p: Vec2) -> f32 {
        unwrap(self.paths[k].nearest(p), self.junctions[k].0, self.length)
    }
}

/// `s` moved by whole turns of `length` to lie at or past `from`.
fn unwrap(s: f32, from: f32, length: f32) -> f32 {
    if length > 0.0 {
        from + (s - from).rem_euclid(length)
    } else {
        s
    }
}

/// The share of a half that the first of two clearances, `a` and `b`
/// (neither negative), claims where together they pass it: `a / (a + b)`,
/// in `[0, 1]`. Taken over the larger of the two, so that neither their
/// sum nor its product with the half's length can overflow: a clearance
/// grown toward a break its line's gaps make wider than the outline —
/// a line tracked over an outline some 10²⁶ long — is many times the
/// half, and the product once took the meeting point to infinity, where
/// a line's ends are not a number (the 300 s flow_structured run's
/// finding on #294). An infinite clearance claims the whole of the half
/// against a finite one, and two claim half each.
fn share(a: f32, b: f32) -> f32 {
    match (a.is_infinite(), b.is_infinite()) {
        (true, true) => 0.5,
        (true, false) => 1.0,
        (false, true) => 0.0,
        (false, false) => {
            let larger = a.max(b);
            if larger > 0.0 {
                let (a, b) = (a / larger, b / larger);
                a / (a + b)
            } else {
                0.0
            }
        }
    }
}

/// The distance between two boxes, `(min, max)` each: 0 where they meet.
fn box_gap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    (b.0 - a.1).max(a.0 - b.1).max(Vec2::ZERO).length()
}

/// How far apart two boxes, `(min, max)` each, are: their distance
/// ([`box_gap`]) where they are apart, and where they overlap how deep,
/// as a negative — the overlap along the axis it is least on, how far
/// one must move to clear the other — so that two inks drawn apart out
/// of an overlap measure how far they have still to go (ruling R43).
fn separation(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    let apart = (b.0 - a.1).max(a.0 - b.1);
    if apart.x > 0.0 || apart.y > 0.0 {
        apart.max(Vec2::ZERO).length()
    } else {
        apart.x.max(apart.y)
    }
}

/// Which way the ends of a part `(sa, sb)` of `path` move as its
/// clearance grows ([`Line::retreats`]): its start on along the path
/// after it, its end back along the path before it — at a sharp corner
/// the tangent at the end itself is the next side's, and an end drawn
/// back from a corner moves along the side before it.
///
/// Cost: two path lookups.
fn retreats(path: &ArcPath, (sa, sb): (f32, f32)) -> [Vec2; 2] {
    [path.at(sa).1, -path.at(sb - resolved(INK_TOLERANCE, sb)).1]
}

/// How far along a path from where an ink is set the point of the path
/// nearest it may lie ([`ArcPath::nearest_near`]): twice its diagonal, and
/// a point more — an ink is pushed off a convex outline by no more than
/// its half size.
fn near_reach(ink: InkBox) -> f32 {
    2.0 * (ink.1 - ink.0).length() + 1.0
}

/// The center of a box, `(min, max)`.
fn middle(b: (Vec2, Vec2)) -> Vec2 {
    Vec2::new(b.0.x.midpoint(b.1.x), b.0.y.midpoint(b.1.y))
}

/// `run` with every glyph's [`GlyphMetrics::ink`] measured but a space's:
/// what tracking a line, and its side ornament, step by. A glyph with
/// no outline to measure — a bitmap emoji, a face with no outlines — is
/// inked as its box, its advance by the line height, so a line keeps it
/// and steps it like any other letter.
///
/// Cost: one outline scale per glyph that is not a space.
fn inked(run: &ShapedRun, fs: &mut FontSystem) -> ShapedRun {
    #[cfg(test)]
    INKED_STEPS.with(|count| count.set(count.get() + inking_steps(run)));
    let mut out = run.clone();
    let mut cache = SwashCache::new();
    let laid: Vec<_> = run.layout_glyphs().cloned().collect();
    let (baseline, height) = (run.baseline(), run.line_height);
    for (glyph, laid) in out.glyphs.iter_mut().zip(&laid) {
        if !glyph.blank {
            let boxed = Ink {
                min: Vec2::new(0.0, -baseline),
                max: Vec2::new(glyph.advance.max(0.0), height - baseline),
            };
            glyph.ink =
                Some(glyph_outline_ink(fs, &mut cache, laid).map_or(boxed, |(min, max)| Ink { min, max }));
        }
    }
    out
}

/// `run` with no glyph's ink: what placement returns, a run's ink being
/// an ornament's own ([`GlyphMetrics::ink`]).
fn uninked(mut run: ShapedRun) -> ShapedRun {
    for glyph in &mut run.glyphs {
        glyph.ink = None;
    }
    run
}

thread_local! {
    /// Test counter (#294): the lines [`track`] has laid on this thread,
    /// what a frame's work is counted in.
    #[cfg(test)]
    pub(crate) static TRACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps the weights
    /// give every shaping on this thread (`shape_run`, `reshape`),
    /// counted where it is shaped, not where a frame counts it.
    #[cfg(test)]
    pub(crate) static SHAPED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps the weights
    /// give every inking on this thread ([`inked`]), counted there.
    #[cfg(test)]
    pub(crate) static INKED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps every [`track`]
    /// on this thread is worth — its clusters and its call — counted
    /// there.
    #[cfg(test)]
    pub(crate) static TRACKED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294): the times a frame's text has been built at a
    /// size on this thread — shaped, inked and its lines prepared —
    /// whether or not its clearance was then grown.
    #[cfg(test)]
    pub(crate) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test seam (#294, ruling R44.2): a frame budget to lay with in
    /// place of [`frame_budget`]'s — R27's sizing pass included, which
    /// lays under the same budget — so a test can spend one.
    #[cfg(test)]
    pub(crate) static BUDGET_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#294, rulings R44.1, R44.2): the least budget a frame
    /// lays with in place of [`TRACK_BUDGET`], so a test can put a long
    /// text's pressure on a shorter one.
    #[cfg(test)]
    pub(crate) static FLOOR_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#294, ruling R44.1): the glyphs a frame may place in
    /// place of [`MAX_PLACED_GLYPHS`], so a test can reach the cap with a
    /// short text.
    #[cfg(test)]
    pub(crate) static CAP_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test counter (#294, ruling R46.1): the times R27's fit has
    /// reshaped a frame's text on this thread (`lines::fit_reshape`).
    #[cfg(test)]
    pub(crate) static FIT_RESHAPES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R47.2): the glyph steps frames on this
    /// thread have taken while their budget was already spent
    /// ([`Work::spent_all`]), but for [`untouched`]'s, which run within
    /// the share of the budget held back for it: what a frame does past
    /// its budget once it has read that it is spent.
    #[cfg(test)]
    pub(crate) static PAST_BUDGET: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.1): the most glyph steps one
    /// [`untouched`] has taken on this thread, and the inks it was given.
    #[cfg(test)]
    pub(crate) static UNTOUCHED_TAKEN: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#294, rulings R44.3, R46.1): the pairs of boxes
    /// [`touching`] has looked at on this thread, each a [`box_gap`]
    /// weighed or a pair it passed over.
    #[cfg(test)]
    pub(crate) static BOXES_WEIGHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One line laid with one tracking gap ([`track`]).
#[derive(Clone)]
struct Tracked {
    /// Its glyphs, in text order.
    glyphs: Vec<PlacedGlyph>,
    /// How far before its part's end its last ink's center falls, by arc
    /// length: positive with room left, negative past it.
    short: f32,
    /// Its first and last ink boxes.
    ends: Option<((Vec2, Vec2), (Vec2, Vec2))>,
    /// Where along its half its first and last inks are set: what the
    /// point of the outline nearest each is looked for near
    /// ([`ArcPath::nearest_near`]).
    centers: Option<(f32, f32)>,
    /// Each inked cluster's ink box, in text order.
    boxes: Vec<InkBox>,
    /// How many of [`glyphs`](Self::glyphs) come before each box's: where
    /// the line is cut to keep only the inks before it.
    marks: Vec<usize>,
}

/// An ink as a frame line's pairwise rule sets it ([`pair`]).
#[derive(Copy, Clone, Debug)]
struct Set {
    /// Its center on the canvas, and its half size.
    at: Vec2,
    half: Vec2,
    /// Where along the path it is set.
    center: f32,
    /// The way it moves there as its center moves on
    /// ([`PathWalk::ink_point`]).
    moves: Vec2,
}

/// The most inks [`pair`] tries for one letter, each a
/// [`PathWalk::ink_point`] — three path lookups at most — so that no
/// glyph step of a frame's line takes more than 64 (ruling R43): shared
/// between its neighbor and the inks before it ([`EARLIER`]).
const PAIR_ROUNDS: usize = 21;

/// How much nearer than owed [`pair`] may set a letter, in points: a
/// quarter of [`INK_TOLERANCE`], which it may set it further — so that
/// a gap measured afresh from the inks as placed, with the float error
/// of their positions, is not read as nearer than the line's by a
/// tolerance.
const NEARER: f32 = 0.25 * INK_TOLERANCE;

/// How many inks before a letter's neighbor it is held clear of too
/// ([`track`]): round a sharp corner inward a letter set at the corner
/// lets the next wrap back toward the one before it.
const EARLIER: usize = 3;

/// The least and most a letter's step along the path is taken as the
/// last letter's share of the straight path's answer ([`track`]'s warm
/// start): the bend of the path and the ink's push off it change that
/// share from one letter to the next only a little.
const PAIR_RATIO: (f32, f32) = (0.5, 2.0);

/// The rounds of bisection [`along`] closes its model in: a bracket of
/// some points to some millionths of one, all arithmetic.
const ALONG_ROUNDS: usize = 24;

/// The most times [`along`] doubles its bracket: past `2^24` times a
/// glyph's reach no step is modeled.
const ALONG_DOUBLINGS: usize = 24;

/// `clusters` laid along `walk`'s path over the part `[sa, sb]` with
/// tracking `g`: the first ink centered at `sa`; every letter after it
/// set by the pairwise rule (ruling R31, [`pair`]) `g` from the letter
/// before, a zero gap included; a space owing its advance and `g` more.
/// A space draws nothing and is placed past the last ink's box, where
/// the box rule would start its step. [`Tracked::short`] is how far
/// before `sb` the last ink's center falls, by arc length: it shrinks as
/// `g` grows, a zero gap included, so the end condition can be solved.
///
/// Each letter's first center tried is warm-started from the one before
/// (ruling R43): the straight path's answer along the way the last ink
/// moves, scaled by the share of it the last letter's step took.
///
/// Cost: O(clusters): a letter one [`PathWalk::ink_point`] where the
/// warm start holds, a few where the path bends, [`PAIR_ROUNDS`] at
/// most; a space one path lookup.
fn track(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    (sa, sb): (f32, f32),
    g: f32,
) -> Tracked {
    #[cfg(test)]
    TRACKS.with(|count| count.set(count.get() + 1));
    #[cfg(test)]
    TRACKED_STEPS.with(|count| count.set(count.get() + clusters.len() + TRACK_CALL_STEPS));
    let mut lay = InkLay::new(walk, glyphs, sa, 0.5 * g);
    for cluster in clusters {
        lay.cluster(cluster, g);
    }
    let (short, ends, centers) = match (lay.first, lay.last) {
        (Some(a), Some(b)) => (
            sb - b.center,
            Some(((a.at - a.half, a.at + a.half), (b.at - b.half, b.at + b.half))),
            Some((a.center, b.center)),
        ),
        _ => (sb - sa, None, None),
    };
    Tracked {
        glyphs: lay.out,
        short,
        ends,
        centers,
        boxes: lay.boxes,
        marks: lay.marks,
    }
}

/// A run of clusters being laid along a path by the pairwise rule, a
/// frame's line ([`track`]) or a side ornament's copies
/// ([`SideArc::lay`]): what is placed, and what the next letter is set
/// from.
struct InkLay<'w> {
    walk: &'w PathWalk<'w>,
    glyphs: &'w [GlyphMetrics],
    out: Vec<PlacedGlyph>,
    /// Each ink's box, in order, and how many glyphs come before it.
    boxes: Vec<InkBox>,
    marks: Vec<usize>,
    first: Option<Set>,
    last: Option<Set>,
    /// The inks before the last, nearest first.
    earlier: [Option<Set>; EARLIER],
    /// How far the next ink is owed from the last.
    owed: f32,
    /// Where a space is placed: past the last ink's box.
    s: f32,
    /// The share of the straight path's answer the last letter's step
    /// took ([`track`]'s warm start).
    ratio: f32,
}

impl<'w> InkLay<'w> {
    /// A lay along `walk` whose first ink is centered at `s`, unless one
    /// is set before it ([`after`](Self::after)), and owed `owed` from
    /// whatever comes before it.
    fn new(walk: &'w PathWalk<'w>, glyphs: &'w [GlyphMetrics], s: f32, owed: f32) -> Self {
        InkLay {
            walk,
            glyphs,
            out: Vec::new(),
            boxes: Vec::new(),
            marks: Vec::new(),
            first: None,
            last: None,
            earlier: [None; EARLIER],
            owed,
            s,
            ratio: 1.0,
        }
    }

    /// The lay set on from the ink `before`, as though it were the last
    /// laid — a text ink a side ornament's copies start past — which is
    /// neither placed nor among [`boxes`](Self::boxes).
    fn after(mut self, before: Set) -> Self {
        self.s = before.center + extent(before.moves, before.half.x * 2.0, before.half.y * 2.0) * 0.5;
        self.last = Some(before);
        self
    }

    /// Lay `cluster` at gap `g`: a space owing its advance and `g` more,
    /// placed where the box rule would start its step; a letter set by
    /// the pairwise rule ([`pair`]) as far from the last ink as owed —
    /// and at least `g` from the few before it ([`EARLIER`]) — or, the
    /// first, at the lay's start. Every letter after owes `g`.
    ///
    /// Cost: a space one path lookup; a letter one [`PathWalk::ink_point`]
    /// where the warm start holds, [`PAIR_ROUNDS`] at most.
    fn cluster(&mut self, cluster: &Cluster, g: f32) {
        #[cfg(test)]
        let before = crate::flow::path::PATH_LOOKUPS.with(Cell::get);
        let walk = self.walk;
        if cluster.blank {
            let at = walk.path.at(self.s).0;
            for index in cluster.glyphs.clone() {
                self.out.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: at - Vec2::new(0.5 * cluster.advance, walk.baseline),
                });
            }
            self.owed += cluster.advance + g;
            self.s += cluster.advance;
            #[cfg(test)]
            note_step(before);
            return;
        }
        let Body::Inked { advance, ink } = walk.body_of(cluster) else {
            return;
        };
        let size = ink.max - ink.min;
        let half = size * 0.5;
        let set = match self.last {
            Some(before) => {
                let owed = self.owed.max(0.0);
                let straight = along(Vec2::ZERO, before.moves, before.half + half, owed);
                let guess = if straight.is_finite() {
                    before.center + straight * self.ratio
                } else {
                    f32::NAN
                };
                let mut rounds = PAIR_ROUNDS;
                let mut set = pair(walk, before, half, guess, owed, ink, &mut rounds);
                // Clear of the inks before its neighbor too, by its gap —
                // and past touching, at no gap — moved on to the nearest
                // center the rule finds that far from the one it is too
                // near: a letter is never nearer than its gap to any
                // letter but its neighbors (rulings R34, R38.1). Near is
                // what `pair` would move: short of its acceptance, so a
                // round never solves for the center it already has.
                let clear = g.max(2.0 * TOUCH);
                for _ in 0..EARLIER {
                    let near = |e: &Set| {
                        ((set.at - e.at).abs() - (set.half + e.half))
                            .max(Vec2::ZERO)
                            .length()
                            < clear - NEARER
                    };
                    let Some(e) = self.earlier.iter().flatten().find(|e| near(e)).copied() else {
                        break;
                    };
                    if rounds == 0 {
                        break;
                    }
                    #[cfg(test)]
                    let was = set.center;
                    set = pair(walk, e, half, set.center, clear, ink, &mut rounds);
                    #[cfg(test)]
                    EARLIER_ROUNDS.with(|count| {
                        let (moved, idle) = count.get();
                        count.set(if set.center == was {
                            (moved, idle + 1)
                        } else {
                            (moved + 1, idle)
                        });
                    });
                }
                let taken = (set.center - before.center) / straight;
                self.ratio = if taken.is_finite() {
                    taken.clamp(PAIR_RATIO.0, PAIR_RATIO.1)
                } else {
                    1.0
                };
                set
            }
            // The first ink centered at the lay's start.
            None => {
                let (at, moves) = walk.ink_point(self.s, ink);
                Set {
                    at,
                    half,
                    center: self.s,
                    moves,
                }
            }
        };
        let corner = set.at - (ink.min + ink.max) * 0.5 - Vec2::new(0.0, walk.baseline);
        self.marks.push(self.out.len());
        for index in cluster.glyphs.clone() {
            self.out.push(PlacedGlyph {
                glyph: index as u32,
                pos: corner + Vec2::new(self.glyphs[index].x - cluster.left, 0.0),
            });
        }
        self.first.get_or_insert(set);
        self.boxes.push((set.at - half, set.at + half));
        self.earlier.rotate_right(1);
        self.earlier[0] = self.last;
        self.s = set.center + 0.5 * extent(set.moves, size.x, size.y) + (advance - ink.max.x);
        self.last = Some(set);
        self.owed = g;
        #[cfg(test)]
        note_step(before);
    }
}

thread_local! {
    /// Test counter (#294, ruling R43): the most path lookups one glyph
    /// step of [`track`] has taken on this thread.
    #[cfg(test)]
    pub(crate) static MOST_STEP_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R43): the glyph steps [`track`] has
    /// taken on this thread, and the path lookups they took, what
    /// [`MOST_STEP_LOOKUPS`] is the most of.
    #[cfg(test)]
    pub(crate) static STEP_LOOKUPS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#294, ruling R44.7): the rounds a letter has been
    /// moved clear of an ink before its neighbor ([`EARLIER`]) on this
    /// thread, and of them the rounds that left it where it was.
    #[cfg(test)]
    pub(crate) static EARLIER_ROUNDS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Test counter (#294, ruling R43): one glyph step of [`track`] taken,
/// the path lookups since `before` its cost.
#[cfg(test)]
fn note_step(before: usize) {
    let spent = crate::flow::path::PATH_LOOKUPS.with(Cell::get) - before;
    MOST_STEP_LOOKUPS.with(|most| most.set(most.get().max(spent)));
    STEP_LOOKUPS.with(|counts| {
        let (steps, lookups) = counts.get();
        counts.set((steps + 1, lookups + spent));
    });
}

/// Where to set a letter whose ink is `ink` (half size `half`) after the
/// ink `last` so that the two are `owed` apart, box to box (ruling R31):
/// the first center tried whose gap is no more than [`NEARER`] short of
/// that and no more than [`INK_TOLERANCE`] past it,
/// more than none apart — boxes no gap apart may overlap, so a zero gap is
/// never near enough, though nothing is owed — or, where none is, the
/// nearest tried past it at which the ink is at least `owed` away: a
/// line's letters are never nearer than its gap (rulings R31, R34). With
/// nothing tried that far, the furthest tried.
///
/// `guess` is tried first ([`track`]'s warm start). Each next center is
/// where the two would be `owed` apart were the path straight on from the
/// last center tried, along the way its ink moves there ([`along`]): exact
/// where it is, and close where the path bends. It is held inside the
/// bracket the centers tried have closed — one too near, one far enough
/// — by regula falsi (Illinois) where it falls outside, and, while
/// nothing tried is far enough, to a stride on. The gap is continuous in
/// the center, so the bracket holds a root, and the line's length moves
/// continuously with its gap wherever the root it closes on does: what
/// [`solve_tracking`] solves (ruling R43).
///
/// Cost: one [`PathWalk::ink_point`] where `guess` holds, a few where the
/// path bends, [`PAIR_ROUNDS`] at most.
fn pair(
    walk: &PathWalk<'_>,
    last: Set,
    half: Vec2,
    guess: f32,
    owed: f32,
    ink: Ink,
    rounds: &mut usize,
) -> Set {
    let reach = last.half + half;
    let set_at = |c: f32| {
        let (at, moves) = walk.ink_point(c, ink);
        Set {
            at,
            half,
            center: c,
            moves,
        }
    };
    let gap_of = |set: &Set| ((set.at - last.at).abs() - reach).max(Vec2::ZERO).length();
    let stride = owed + 2.0 * reach.max_element() + INK_TOLERANCE;
    // Too near: the last ink's own center, where the gap is none; then
    // the furthest tried that is too near, with its miss.
    let (mut low, mut low_miss) = (last.center, -owed);
    // Far enough: the nearest tried that is, with its miss.
    let mut high: Option<(Set, f32)> = None;
    // Which end the last center tried moved: an end kept twice running
    // has the other's miss halved (Illinois).
    let mut kept: Option<bool> = None;
    // Whether the model still steers: while each step it takes at least
    // halves the miss. Round a sharp corner outward an ink swings round
    // the corner as its center crosses it, which the straight path does
    // not model, and the model's steps fall either side of the root.
    let mut modeled = true;
    let mut last_miss = f32::INFINITY;
    // The bracket's width when it last halved.
    let mut width = f32::INFINITY;
    let mut tried: Option<Set> = None;
    let mut c = if guess > low && guess.is_finite() {
        guess
    } else {
        low + stride
    };
    while *rounds > 0 {
        *rounds -= 1;
        let set = set_at(c);
        let gap = gap_of(&set);
        if gap > 0.0 && gap >= owed - NEARER && gap <= owed + INK_TOLERANCE {
            return set;
        }
        tried = Some(set);
        // A gap that is not a number is not far enough.
        let miss = gap - owed;
        if miss > 0.0 {
            if high.is_none_or(|(h, _)| c < h.center) {
                if kept == Some(true) {
                    low_miss *= 0.5;
                }
                kept = Some(true);
                high = Some((set, miss));
            }
        } else if c > low && high.is_none_or(|(h, _)| c < h.center) {
            if kept == Some(false) {
                if let Some((_, far)) = high.as_mut() {
                    *far *= 0.5;
                }
            }
            kept = Some(false);
            (low, low_miss) = (c, if miss.is_nan() { -owed } else { miss });
        }
        if !miss.abs().lt(&(0.5 * last_miss)) {
            modeled = false;
        }
        last_miss = miss.abs();
        let model = c + along(set.at - last.at, set.moves, reach, owed);
        let next = match high {
            None if model > low => model.min(low + stride),
            None => low + stride,
            Some((h, h_miss)) => {
                let now = h.center - low;
                let halved = now <= 0.5 * width;
                if halved {
                    width = now;
                }
                // The line through the bracket's ends.
                let crossing = low + now * (-low_miss) / (h_miss - low_miss);
                let inside = |x: f32| x > low && x < h.center;
                if !halved && width.is_finite() {
                    low.midpoint(h.center)
                } else if modeled && inside(model) {
                    model
                } else if inside(crossing) {
                    crossing
                } else {
                    low.midpoint(h.center)
                }
            }
        };
        // A bracket closed to a float, or a step that leaves the center
        // where it was, finds nothing more.
        if !next.gt(&low) || next.to_bits() == c.to_bits() || high.is_some_and(|(h, _)| next >= h.center) {
            break;
        }
        c = next;
    }
    match (high, tried) {
        (Some((h, _)), _) => h,
        (None, Some(t)) => t,
        (None, None) => set_at(c),
    }
}

/// The step along unit `dir` from where two inks' centers are `delta`
/// apart, their half sizes summing to `reach`, after which their boxes
/// are `owed` apart: negative where they are further apart than that
/// already — the pairwise rule's model of a path straight on from a
/// center tried ([`pair`]), solved by bisection on its own arithmetic,
/// with no path lookup. `NaN` where no step on or back sets them so.
///
/// Cost: O(1): [`ALONG_DOUBLINGS`] and [`ALONG_ROUNDS`] rounds at most.
fn along(delta: Vec2, dir: Vec2, reach: Vec2, owed: f32) -> f32 {
    let miss = |d: f32| ((delta + dir * d).abs() - reach).max(Vec2::ZERO).length() - owed;
    let at = miss(0.0);
    if !at.is_finite() {
        return f32::NAN;
    }
    // Too near, the step is on; too far, back. Boxes no gap apart may
    // overlap: none apart is too near.
    let near = |m: f32| m < 0.0 || (m <= 0.0 && owed <= 0.0);
    let on = near(at);
    // A bracket from here, doubled until it holds the step.
    let unit = (owed + reach.max_element()).max(INK_TOLERANCE);
    let sign = if on { 1.0 } else { -1.0 };
    let (mut inside, mut outside) = (0.0_f32, sign * unit);
    let mut doublings = 0;
    while near(miss(outside)) == on {
        if doublings == ALONG_DOUBLINGS || !outside.is_finite() {
            return f32::NAN;
        }
        inside = outside;
        outside *= 2.0;
        doublings += 1;
    }
    for _ in 0..ALONG_ROUNDS {
        let middle = inside.midpoint(outside);
        if near(miss(middle)) == on {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    outside
}

/// One line of the frame before it is laid: the half it takes, its
/// glyphs and what tracking it needs of them.
#[derive(Clone)]
struct Prepared {
    /// The half it takes: 0 the upper-right, 1 the lower-left.
    half: usize,
    /// Its clusters, in text order.
    clusters: Vec<Cluster>,
    /// Its letters' mean ink half size: how far they sit off the outline.
    size: Vec2,
    /// Its font's own gap: its letters' mean side bearings.
    natural_gap: f32,
    /// Its widest space's advance.
    space: f32,
    /// Its natural ends along its half ([`LineSpan::NATURAL`]): its first
    /// and last ink centers at its font's own gap, centered on its half's
    /// apex.
    natural: (f32, f32),
    /// Whether R27 cut its end.
    cut: bool,
}

/// One line along its half ([`Layout::lay`]).
#[derive(Clone)]
struct Line {
    /// The half it takes.
    half: usize,
    /// Its glyphs, as [`track`] laid them at its gap.
    tracked: Tracked,
    /// Its tracking gap.
    gap: f32,
    /// Its word gap, a space's advance and two gaps: `None` for a line
    /// with no space, which has no word gap.
    word: Option<f32>,
    /// The part of its half it covers.
    part: (f32, f32),
    /// Its letters' mean ink half size.
    size: Vec2,
    /// Whether its half less the clearance was too short to hold it at
    /// its font's own gap, so it was laid tighter.
    pressed: bool,
    /// Whether it has two letters or more, and so a gap to track: a line
    /// of one sits centered on its part, which its clearance moves
    /// nowhere in particular.
    tracks: bool,
    /// How many gaps it has ([`solve_tracking`]'s `steps`).
    steps: usize,
    /// The outward normal of the outline at its first and last inks, as
    /// they lie ([`Layout::normals`]): what the 135° test weighs.
    normals: [Vec2; 2],
    /// Which way its ends move as its clearance grows at the top-left
    /// and bottom-right junctions: on along its half at its start, back
    /// at its end — along the path before its end, which at a sharp
    /// corner is not the way the path runs on from it.
    retreats: [Vec2; 2],
    /// How much more clearance its part can give before it is shorter
    /// than the line at its font's own gap, by arc length, never
    /// negative: the room its part leaves, and its gaps past its font's
    /// own. A line of one letter, which nothing presses, has no end of
    /// it.
    slack: f32,
}

/// A frame's lines being laid along its halves.
struct Layout<'a> {
    halves: &'a Halves,
    outward: bool,
    /// The work the frame has taken, and the most it may take.
    work: Work,
}

/// A frame's work (rulings R44.2, R46.1): the glyph steps all its work
/// has taken — its tracking, its side lays, its shaping and inking, its
/// looks for a touch, R27's lays — the glyphs of its text its budget is
/// scaled by, the most it may take ([`frame_budget`]), and what of that
/// is held back for [`untouched`].
pub(super) struct Work {
    spent: Cell<usize>,
    glyphs: Cell<usize>,
    budget: Cell<usize>,
    reserve: Cell<usize>,
    /// Whether [`untouched`] is running, within the share held back for
    /// it: its steps are not past the budget ([`PAST_BUDGET`]).
    #[cfg(test)]
    reserved: Cell<bool>,
}

impl Work {
    /// A frame's work, nothing taken yet, its budget scaled by `glyphs`.
    pub(super) fn new(glyphs: usize) -> Self {
        let work = Work {
            spent: Cell::new(FRAME_STEPS),
            glyphs: Cell::new(0),
            budget: Cell::new(TRACK_BUDGET),
            reserve: Cell::new(0),
            #[cfg(test)]
            reserved: Cell::new(false),
        };
        work.scale(glyphs);
        work
    }

    /// Count `steps` glyph steps taken.
    pub(super) fn take(&self, steps: usize) {
        #[cfg(test)]
        if self.spent_all() && !self.reserved.get() {
            PAST_BUDGET.with(|past| past.set(past.get().saturating_add(steps)));
        }
        self.spent.set(self.spent.get().saturating_add(steps));
    }

    /// Count shaping `run` (ruling R46.1).
    pub(super) fn shaped(&self, run: &ShapedRun) {
        self.take(shaping_steps(run));
    }

    /// Count inking `run` (ruling R46.1).
    fn inked(&self, run: &ShapedRun) {
        self.take(inking_steps(run));
    }

    /// Count walking `clusters` clusters in R27's arithmetic.
    pub(super) fn walked(&self, clusters: usize) {
        self.take(clusters.div_ceil(CLUSTERS_A_STEP).max(1));
    }

    /// Whether the frame has taken its budget, less what is held back
    /// for [`untouched`], which runs within it (ruling R46.1).
    pub(super) fn spent_all(&self) -> bool {
        self.spent.get().saturating_add(self.reserve.get()) >= self.budget.get()
    }

    /// Scale the budget by the frame's text, `glyphs` glyphs once cut to
    /// the glyph cap (ruling R44.2): its sides' lays count against it,
    /// and scale it not (ruling R44.3), so a frame's work is bounded by
    /// its text alone. As much of it as [`untouched`] may take over that
    /// many inks is held back for it.
    fn scale(&self, glyphs: usize) {
        self.glyphs.set(glyphs);
        self.budget.set(frame_budget(glyphs));
        self.reserve.set(untouched_steps(glyphs));
    }
}

/// The lines [`Layout::lay`] has laid in one clearance search.
#[derive(Default)]
struct LayMemo {
    /// By line, part of its half it may cover, and span, all bit for
    /// bit: a round that grows one line's clearance lays the other where
    /// it lay.
    laid: FxHashMap<(usize, u32, u32, u32), Line>,
    /// Each line's last lay at a span, by line and span's bits: its
    /// part's length and its gap, what its next lay's tracking is
    /// warm-started from ([`solve_tracking`], ruling R43) — a round moves
    /// a line's part a little, and its gap with it.
    warm: FxHashMap<(usize, u32), (f32, f32)>,
}

/// One clearance search's state ([`Layout::lay_clear`]): the lines it
/// has laid, and how many times a round that would press a line is
/// halved in it.
struct Search {
    memo: LayMemo,
    backtracks: usize,
}

impl<'a> Layout<'a> {
    fn new(halves: &'a Halves, outward: bool, work: Work) -> Self {
        Layout {
            halves,
            outward,
            work,
        }
    }
}

impl Layout<'_> {
    /// [`track`], counted against the frame's budget ([`Work`]).
    fn track(
        &self,
        walk: &PathWalk<'_>,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        part: (f32, f32),
        g: f32,
    ) -> Tracked {
        self.work.take(clusters.len() + TRACK_CALL_STEPS);
        track(walk, glyphs, clusters, part, g)
    }

    /// Whether the frame has taken its budget ([`Work`]).
    fn spent_all(&self) -> bool {
        self.work.spent_all()
    }

    /// The walk a line on half `k` is tracked along.
    fn walk<'s>(&'s self, k: usize, run: &ShapedRun) -> PathWalk<'s> {
        line_walk(&self.halves.paths[k], run, self.outward)
    }

    /// How long `clusters` are at gap `g` from their first ink center at
    /// `from` to their last, by arc length along half `k`.
    fn reach(&self, k: usize, run: &ShapedRun, clusters: &[Cluster], from: f32, g: f32) -> f32 {
        -self
            .track(&self.walk(k, run), &run.glyphs, clusters, (from, from), g)
            .short
    }

    /// The line of `run` (inked) of `clusters` on half `k`, ready to lay:
    /// `None` when it holds no glyph that is not a space. Its natural
    /// ends are centered on the apex ([`falsi`]): the arc from its first
    /// ink center to the apex equals the arc from the apex to its last,
    /// to [`CENTER_TOLERANCE`].
    ///
    /// Cost: one [`track`] at the apex and [`CENTER_ROUNDS`] at most, one
    /// on a straight path.
    fn prepare_clusters(
        &self,
        run: &ShapedRun,
        k: usize,
        clusters: Vec<Cluster>,
        cut: bool,
    ) -> Option<Prepared> {
        let (size, natural_gap) = letters_of(&clusters)?;
        let space = clusters
            .iter()
            .filter(|c| c.blank)
            .map(|c| c.advance)
            .fold(0.0_f32, f32::max);
        let apex = self.halves.apexes[k];
        let reach = |from: f32| self.reach(k, run, &clusters, from, natural_gap);
        // Off-center by `before - after`: positive while the start is too
        // far back. It falls as the start moves on, by about twice as
        // much, the line's length changing only with the bend of the path
        // under it — so the line through the bracket's ends, at the
        // apex less the whole line and at the apex, crosses zero at the
        // straight path's answer, half the line back, and that is the
        // first probe. A miss that is not a number reads as too far
        // back: the start moves on, toward the apex, where the line is
        // no longer than it was.
        let miss = |na: f32| {
            let long = reach(na);
            let m = (apex - na) - (na + long - apex);
            (if m.is_nan() { f32::INFINITY } else { m }, long)
        };
        let whole = reach(apex).max(0.0);
        let (na, long) = falsi(
            miss,
            (apex - whole, whole, whole),
            (apex, -whole, whole),
            resolved(CENTER_TOLERANCE, apex),
            resolved(CENTER_TOLERANCE, apex),
            CENTER_ROUNDS,
            &|| self.spent_all(),
        );
        Some(Prepared {
            half: k,
            natural: (na, na + long),
            clusters,
            size,
            natural_gap,
            space,
            cut,
        })
    }

    /// The part of half `half` a line may cover with `clear` kept at
    /// each junction (top-left, bottom-right): `(lo, hi)`, never
    /// inverted and never off the half. Clearances that together pass
    /// the whole half meet where they divide it in their ratio
    /// ([`share`]): the line is then a point, pressed.
    fn room(&self, half: usize, clear: [f32; 2]) -> (f32, f32) {
        let (ja, jb) = self.halves.junctions[half];
        let (lo, hi) = (ja + clear[0], jb - clear[1]);
        if lo <= hi {
            (lo, hi)
        } else {
            let mid = ja + (jb - ja) * share(clear[0], clear[1]);
            (mid, mid)
        }
    }

    /// The natural line `p` slid clear of the clearance, at its font's
    /// own gap, within `(lo, hi)` when that holds it: slid on from the
    /// top-left clearance, slid back from the bottom-right one. A line
    /// longer than `hi − lo` at its gap starts at `lo` and ends at `hi`.
    fn slid(&self, run: &ShapedRun, p: &Prepared, (lo, hi): (f32, f32)) -> (f32, f32) {
        let reach = |from: f32| self.reach(p.half, run, &p.clusters, from, p.natural_gap);
        let (mut na, mut nb) = p.natural;
        if na < lo {
            na = lo;
            nb = lo + reach(lo);
        }
        if nb > hi {
            // The start whose line ends at `hi`: where the miss, the
            // line's end past `hi`, is zero. It moves with the start
            // nearly one for one, bent only by the path under the line,
            // so each probe is where the line through the last two
            // crosses zero (ruling R38.3) — the first, the start the
            // line's length from `hi` gives.
            let mut long = reach(na);
            let mut last: Option<(f32, f32)> = None;
            let mut next = hi - long;
            for _ in 0..SLIDE_ROUNDS {
                if self.spent_all() {
                    break;
                }
                na = next;
                long = reach(na);
                let miss = na + long - hi;
                if miss.abs() <= resolved(SLIDE_TOLERANCE, hi) {
                    break;
                }
                let secant = last.map(|(at, was)| na - miss * (na - at) / (miss - was));
                next = match secant {
                    Some(x) if x.is_finite() => x,
                    _ => hi - long,
                };
                if next == na {
                    break;
                }
                last = Some((na, miss));
            }
            if na < lo {
                na = lo;
                long = reach(lo);
            }
            nb = (na + long).min(hi);
        }
        (na, nb)
    }

    /// The lines of `run` (inked), each moved by `span` from its natural
    /// place (slid clear of the clearance, [`slid`](Self::slid)) to its
    /// whole half less its `clear` (top-left, bottom-right), by arc length at
    /// each end (ruling R33), and laid at the gap that centers its end
    /// inks on its part's ends.
    ///
    /// A line laid before in this search with the same part and span
    /// is read from `memo`: the line laid then. A line laid afresh has
    /// its tracking warm-started from its last lay at the span.
    fn lay(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        clear: Clear,
        span: f32,
        memo: &mut LayMemo,
    ) -> Vec<Line> {
        lines
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let room = self.room(p.half, clear[p.half]);
                let key = (i, room.0.to_bits(), room.1.to_bits(), span.to_bits());
                if let Some(line) = memo.laid.get(&key) {
                    return line.clone();
                }
                let warm = memo.warm.get(&(i, span.to_bits())).copied();
                let line = self.lay_one(run, p, room, span, warm);
                if line.tracks && !line.pressed {
                    memo.warm
                        .insert((i, span.to_bits()), (line.part.1 - line.part.0, line.gap));
                }
                memo.laid.insert(key, line.clone());
                line
            })
            .collect()
    }

    /// [`lay`](Self::lay)'s line `p` over `(lo, hi)`, its tracking
    /// warm-started from `warm` — the part's length and the gap of the
    /// line's last lay at this span — when there is one.
    fn lay_one(
        &self,
        run: &ShapedRun,
        p: &Prepared,
        (lo, hi): (f32, f32),
        span: f32,
        warm: Option<(f32, f32)>,
    ) -> Line {
        let walk = self.walk(p.half, run);
        let (na, nb) = self.slid(run, p, (lo, hi));
        // `lo ≤ na` and `nb ≤ hi` after the slide, so each end
        // moves only outward as the span grows.
        let sa = na + (lo - na) * span;
        let sb = nb + (hi - nb) * span;
        // A line of one letter has no gap to track: it sits at
        // its font's gap, centered on its part.
        let letters = p.clusters.iter().filter(|c| !c.blank).count();
        let steps = letters.saturating_sub(1) + p.clusters.iter().filter(|c| c.blank).count();
        let (gap, tracked) = if letters < 2 {
            let mid = sa.midpoint(sb);
            let gap = p.natural_gap;
            (gap, self.track(&walk, &run.glyphs, &p.clusters, (mid, mid), gap))
        } else {
            let (gap, tracked) = solve_tracking(
                |g| self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), g),
                steps,
                sb,
                // The last gap moved by the part's change in length,
                // shared among the gaps, as on a straight path.
                warm.map(|(long, gap)| gap + ((sb - sa) - long) / steps.max(1) as f32),
                &|| self.spent_all(),
            );
            if gap < p.natural_gap - GAP_TOLERANCE {
                // A part its line fits at its font's gap is not one that
                // presses it, where the slide that set it fell short of
                // its end round a bend (ruling R35.2), or the solve came
                // to rest below the font's gap by no more than the line's
                // precision: it is laid at the font's gap from its start.
                let natural = self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), p.natural_gap);
                if natural.short >= -line_tolerance(steps, sb) {
                    (p.natural_gap, natural)
                } else {
                    (gap, tracked)
                }
            } else {
                (gap, tracked)
            }
        };
        Line {
            half: p.half,
            normals: self.normals(p.half, &tracked, (sa, sb)),
            retreats: retreats(&self.halves.paths[p.half], (sa, sb)),
            tracked,
            gap,
            word: (p.space > 0.0).then_some(p.space + 2.0 * gap),
            part: (sa, sb),
            size: p.size,
            pressed: letters >= 2 && gap < p.natural_gap - GAP_TOLERANCE,
            tracks: letters >= 2,
            steps,
            // The room the part leaves, and what its gaps hold past the
            // font's own, measured where the line lies: on a bend an
            // upright letter takes more of the path than at the apex.
            slack: if letters >= 2 {
                ((hi - lo) - (sb - sa) + steps as f32 * (gap - p.natural_gap)).max(0.0)
            } else {
                f32::INFINITY
            },
        }
    }

    /// The outward normal of the outline at the point nearest each end
    /// ink of a line `tracked` on half `k` over `part`, first then last:
    /// the normal of the ink as it lies, which on a sharp tip turns away
    /// from the normal where its center is set along the path. Past a
    /// sharp corner, where the point nearest the ink is the corner and
    /// the outline has no one normal there, the way from the corner to
    /// the ink: an ink pushed round a corner off the end of a side faces
    /// between the two sides, where the side after the corner's normal
    /// would face it the other side's way (a frame round a 10:1
    /// rectangle once took a junction so for lines stacked across it). A
    /// line with no ink takes the normals at its part's ends. Found once
    /// a line is laid ([`Line::normals`]).
    ///
    /// An ink is pushed off the outline by no more than its own half
    /// size, so the point of the outline nearest it is within its
    /// diagonal of where it is set, by arc length within twice that on a
    /// convex outline: it is looked for there (ruling R43).
    ///
    /// Cost: two [`ArcPath::nearest_near`]s.
    fn normals(&self, k: usize, tracked: &Tracked, part: (f32, f32)) -> [Vec2; 2] {
        let path = &self.halves.paths[k];
        let normal = |s: f32| path.outward(path.at(s).1);
        match (tracked.ends, tracked.centers) {
            (Some((first, last)), Some((a, b))) => [(first, a), (last, b)].map(|(ink, set)| {
                let p = middle(ink);
                let (q, tangent) = path.at(path.nearest_near(p, set, near_reach(ink)));
                let along = path.outward(tangent);
                let off = p - q;
                let way = off.normalize_or_zero();
                // Off the outline by more than a hundredth of the ink's
                // size, in a way its normal there is not: past a corner.
                if off.length() > 0.01 * (ink.1 - ink.0).length() && way.dot(along) < CORNER {
                    way
                } else {
                    along
                }
            }),
            _ => [normal(part.0), normal(part.1)],
        }
    }

    /// How far apart the lines' end inks are at each junction (top-left,
    /// bottom-right), box to box, how far apart they must be — two of the lines' mean
    /// word gap (ruling R33.1), over the lines that have one: a line of
    /// one word has no word gap to count, the span stretching its letter
    /// gaps alone; with no word gap on any line, two of a line's letter
    /// gaps stand for one; and for two end inks whose outward normals
    /// turn more than 135° apart, lines stacked across a thin outline
    /// rather than lines meeting, only not to touch (ruling R35.1) — and,
    /// for each line, how much drawing its
    /// end back at that junction parts the two: the cosine between the
    /// way its end moves as its clearance grows and the way away from the
    /// other end. A single line is measured against the junction point
    /// on its own center line, owing half that.
    fn apart(&self, lines: &[Line]) -> Junctions {
        let words: Vec<f32> = lines.iter().filter_map(|l| l.word).collect();
        let word = if words.is_empty() {
            lines.iter().map(|l| 2.0 * l.gap).sum::<f32>() / lines.len().max(1) as f32
        } else {
            words.iter().sum::<f32>() / words.len() as f32
        };
        // Which way line `l`'s end at junction `j` moves as its
        // clearance there grows: on along its half at the top-left, back
        // at the bottom-right.
        let retreat = |l: &Line, j: usize| l.retreats[j];
        // Ends that touch have no way apart to measure against: each
        // line's drawing back parts them, as at the junction itself.
        let helps = |l: &Line, j: usize, own: (Vec2, Vec2), other: (Vec2, Vec2)| {
            if !l.tracks {
                0.0
            } else if box_gap(own, other) > 0.0 {
                retreat(l, j).dot((middle(own) - middle(other)).normalize_or_zero())
            } else {
                1.0
            }
        };
        let mut out = Junctions {
            apart: [f32::INFINITY; 2],
            depth: [f32::INFINITY; 2],
            want: [0.0; 2],
            helps: [[0.0; 2]; 2],
        };
        match lines {
            [one, two, ..] => {
                out.want = [JUNCTION_WORD_GAPS * word; 2];
                if let (Some((a1, b1)), Some((a2, b2))) = (one.tracked.ends, two.tracked.ends) {
                    for (j, (e1, e2)) in [(a1, a2), (b1, b2)].into_iter().enumerate() {
                        if one.normals[j].dot(two.normals[j]) < STACKED {
                            out.want[j] = TOUCH;
                        }
                        out.apart[j] = box_gap(e1, e2);
                        out.depth[j] = separation(e1, e2);
                        out.helps[one.half][j] = helps(one, j, e1, e2);
                        out.helps[two.half][j] = helps(two, j, e2, e1);
                    }
                }
            }
            [one] => {
                out.want = [0.5 * JUNCTION_WORD_GAPS * word; 2];
                let walk = PathWalk {
                    path: &self.halves.paths[one.half],
                    memo: None,
                    line_height: 0.0,
                    outward: self.outward,
                    limit: f32::INFINITY,
                    level: false,
                    ornament: true,
                    baseline: 0.0,
                    push: None,
                };
                let mean = Ink {
                    min: -one.size,
                    max: one.size,
                };
                let (ja, jb) = self.halves.junctions[one.half];
                let points = [walk.ink_at(ja, mean), walk.ink_at(jb, mean)];
                if let Some((a, b)) = one.tracked.ends {
                    for (j, (e, p)) in [(a, points[0]), (b, points[1])].into_iter().enumerate() {
                        out.apart[j] = box_gap(e, (p, p));
                        out.depth[j] = separation(e, (p, p));
                        out.helps[one.half][j] = helps(one, j, e, (p, p));
                    }
                }
            }
            [] => {}
        }
        out
    }
}

/// Each junction's measure ([`Layout::apart`]): its lines' end inks'
/// distance, the distance wanted, and how much each half's line parts
/// them by drawing back.
struct Junctions {
    apart: [f32; 2],
    /// [`apart`](Self::apart) where the inks are apart, and how deep
    /// they overlap, as a negative, where they do ([`separation`]): what
    /// a round's step is measured from, so that two inks drawn out of an
    /// overlap are short by its depth too, and show it (ruling R43).
    depth: [f32; 2],
    want: [f32; 2],
    /// `[half][junction]`.
    helps: [[f32; 2]; 2],
}

/// The clearance each half's line keeps from each junction, by arc
/// length: `[half][junction]`, top-left then bottom-right.
type Clear = [[f32; 2]; 2];

/// An ink box on the canvas, `(min, max)`.
type InkBox = (Vec2, Vec2);
/// A frame's side ornament as laid round a junction (ruling R33.1): the
/// walk it is laid along — the first half's path, its inks pushed out
/// by the text's mean ink so they sit on the text's center line — and
/// one copy's clusters.
struct SideOrnament<'a> {
    walk: PathWalk<'a>,
    glyphs: &'a [GlyphMetrics],
    clusters: Vec<Cluster>,
    /// The frame's work, which its lays count against (rulings R44.2,
    /// R44.3).
    work: &'a Work,
}

impl<'a> SideOrnament<'a> {
    /// The side ornament of `run` (inked) over `range`, laid round the
    /// halves of `layout` beside text whose letters' mean ink half size
    /// is `size`.
    fn new(layout: &'a Layout<'_>, run: &'a ShapedRun, range: Range<usize>, size: Vec2) -> SideOrnament<'a> {
        SideOrnament {
            walk: PathWalk {
                push: layout.outward.then_some(size),
                ..line_walk(&layout.halves.paths[0], run, layout.outward)
            },
            glyphs: &run.glyphs,
            clusters: clusters_of(&run.glyphs, range),
            work: &layout.work,
        }
    }

    /// Count `copies` copies laid against the frame's glyph steps.
    fn count(&self, copies: usize) {
        self.work.take(copies.saturating_mul(self.clusters.len().max(1)));
    }

    /// The arc of this side between two text inks, `None` without an
    /// ink in the ornament.
    fn arc(&self, from: ArcEnd, to: ArcEnd) -> Option<SideArc<'_>> {
        SideArc::new(self, from, to)
    }

    /// About how long one copy and its two joins are along a path:
    /// what a junction's clearance grows by while it holds none.
    fn room(&self) -> f32 {
        let advance: f32 = self.clusters.iter().map(|c| c.advance).sum();
        let floor = self.arc_floor().unwrap_or(0.0);
        advance + 2.0 * floor
    }

    /// The least gap a copy is laid at: its first ink's side bearings,
    /// never less than [`JOIN_FLOOR`].
    fn arc_floor(&self) -> Option<f32> {
        let (advance, ink) = self
            .clusters
            .iter()
            .find(|c| !c.blank)
            .and_then(|c| c.ink.map(|i| (c.advance, i)))?;
        Some((advance - (ink.max.x - ink.min.x)).max(JOIN_FLOOR))
    }
}

/// The arc of a side between the text ink it starts past (`from`) and
/// the one it ends before (`to`), along the side's path.
struct SideArc<'s> {
    side: &'s SideOrnament<'s>,
    /// The text ink the arc starts past, as the pairwise rule sets the
    /// first copy from it.
    start: Set,
    to: (Vec2, Vec2),
    /// The least gap a copy is laid at ([`SideOrnament::arc_floor`]).
    floor: f32,
    /// Where `from`'s and `to`'s centers lie along the path, `cb` past
    /// `ca`.
    ca: f32,
    cb: f32,
}

impl<'s> SideArc<'s> {
    fn new(side: &'s SideOrnament<'s>, (from, near_a): ArcEnd, (to, near_b): ArcEnd) -> Option<Self> {
        side.clusters.iter().find(|c| !c.blank && c.ink.is_some())?;
        let path = side.walk.path;
        // The point of the path nearest each ink, looked for near where
        // it is set (ruling R43).
        let ca = path.nearest_near(middle(from), near_a, near_reach(from));
        let cb = unwrap(
            path.nearest_near(middle(to), near_b, near_reach(to)),
            ca,
            path.length(),
        );
        Some(SideArc {
            side,
            start: Set {
                at: middle(from),
                half: 0.5 * (from.1 - from.0),
                center: ca,
                moves: path.at(ca).1,
            },
            to,
            floor: side.arc_floor()?,
            ca,
            cb,
        })
    }

    /// `n` copies laid at `gap` by the pairwise rule ([`InkLay`]): the
    /// first ink `gap` from `from`'s, each next ink `gap` from the last, a
    /// space owing its advance and `gap` more. Returns the lay, how far
    /// the last ink is from `to` and where along the path its center lies,
    /// and whether all `n` copies were laid: the frame's budget is read
    /// before each copy after the first (rulings R44.2, R47.2), the
    /// first's read by the caller, and a lay that finds it spent stops.
    ///
    /// Cost: a [`PathWalk::ink_point`] or a few a glyph.
    fn lay(&self, n: usize, gap: f32) -> (InkLay<'_>, (f32, f32), bool) {
        let mut lay = InkLay::new(&self.side.walk, self.side.glyphs, self.ca, gap).after(self.start);
        for k in 0..n {
            if k > 0 && self.side.work.spent_all() {
                let end = self.end_of(&lay);
                return (lay, end, false);
            }
            for cluster in &self.side.clusters {
                lay.cluster(cluster, gap);
            }
            self.side.count(1);
        }
        let end = self.end_of(&lay);
        (lay, end, true)
    }

    /// How far the last ink `lay` set is from `to`, and where along the
    /// path its center lies.
    fn end_of(&self, lay: &InkLay<'_>) -> (f32, f32) {
        lay.last
            .map(|l| (box_gap((l.at - l.half, l.at + l.half), self.to), l.center))
            .unwrap_or((f32::INFINITY, self.ca))
    }

    /// Whether `n` copies at `gap` fit: the last ink's center short of
    /// `to`'s and its ink at least `gap` from `to`'s. A miss that is not
    /// a number does not fit.
    fn fits(&self, n: usize, gap: f32) -> bool {
        let (_, (end, at), whole) = self.lay(n, gap);
        whole && at < self.cb && end >= gap
    }

    /// How much room `n` copies laid by `lay` leave: how far the last
    /// ink is from `to`'s past the gap — at least none where they fit
    /// ([`fits`](Self::fits)) — or, its center past `to`'s, as far past
    /// as that and the gap, a negative. A room that is not a number is
    /// none at all.
    fn room_of(&self, lay: &InkLay<'_>, gap: f32) -> f32 {
        let (end, at) = self.end_of(lay);
        let v = if at < self.cb {
            end - gap
        } else {
            -(gap + (at - self.cb))
        };
        if v.is_nan() {
            f32::NEG_INFINITY
        } else {
            v
        }
    }

    /// Copies laid at the floor's gap, one after another, up to `most` or
    /// the first that does not fit: what they place, and for each `k`
    /// copies that fit the room they leave and how many glyphs they
    /// place. Copies are laid in a row, each set on from the last, so the
    /// first `k` of any row are `k` copies laid on their own: this one lay
    /// tells every copy count's room at the floor (ruling R44.3). `None`
    /// when the frame's budget is spent before the row is laid: the
    /// budget is read before each copy (rulings R44.2, R47.2).
    ///
    /// Cost: one lay of the copies that fit and the one that does not.
    fn floor_row(&self, most: usize) -> Option<(Vec<PlacedGlyph>, Vec<(f32, usize)>)> {
        let gap = self.floor;
        let mut lay = InkLay::new(&self.side.walk, self.side.glyphs, self.ca, gap).after(self.start);
        let mut rooms = Vec::new();
        while rooms.len() < most.min(MAX_SIDE_COPIES) {
            if self.side.work.spent_all() {
                return None;
            }
            for cluster in &self.side.clusters {
                lay.cluster(cluster, gap);
            }
            self.side.count(1);
            let room = self.room_of(&lay, gap);
            if room.is_nan() || room < 0.0 {
                break;
            }
            rooms.push((room, lay.out.len()));
        }
        Some((lay.out, rooms))
    }

    /// How many gaps `n` copies have: one before each ink of every copy,
    /// and the join to `to`.
    fn gaps(&self, n: usize) -> f32 {
        let inked = self.side.clusters.iter().filter(|c| !c.blank).count().max(1);
        (n * inked + 1) as f32
    }

    /// The gap at which `n` copies would fill the arc were its path
    /// straight, from the room `room` they leave at the floor: the room
    /// shared among their gaps.
    fn straight(&self, n: usize, room: f32) -> f32 {
        self.floor + room / self.gaps(n)
    }

    /// The widest gap at which `n` copies fit with every gap and both
    /// joins equal, to [`SIDE_PRECISION`], and the copies laid at it,
    /// given the room `at_floor` they leave at the floor and what they
    /// place there ([`floor_row`](Self::floor_row)): where the last copy's
    /// ink is the gap from `to`'s. That room falls with the gap by about
    /// its gaps' count a point — one before each ink of every copy, and
    /// the join to `to` — bent only by the path, so it is solved as a
    /// line's tracking is (ruling R44.3, [`solve_tracking`]): the straight
    /// path's root ([`straight`](Self::straight)), aimed a hair past it,
    /// widened until it brackets the root, and closed by [`falsi`]. A copy
    /// past `to`'s center reads as past the end. The frame's budget spent,
    /// the widest gap found to fit so far.
    ///
    /// Cost: a lay of `n` copies a widening, and [`falsi`]'s, at most
    /// [`SIDE_ROUNDS`]; none at all where the floor is the answer.
    fn solve(&self, n: usize, at_floor: f32, floor_out: Vec<PlacedGlyph>) -> (f32, Vec<PlacedGlyph>) {
        let gaps = self.gaps(n);
        // As precise as the copies' inks are set: each to within the
        // pairwise rule's tolerance, as a line's end is ([`line_tolerance`]).
        let tolerance = resolved(SIDE_PRECISION, self.cb).max(gaps * INK_TOLERANCE);
        if at_floor <= tolerance {
            return (self.floor, floor_out);
        }
        // A room of none fits, and reads as positive to `falsi`. A miss
        // whose last ink touches `to`'s, or is set past its center, reads
        // as at least twice the tolerance short, so `falsi` never takes it
        // for the answer: copies a hair short of `to` close with a join a
        // hair narrower than their gaps, as a line's end a hair past its
        // part's is laid, but copies that touch the text are no answer.
        let room = |gap: f32| {
            let (lay, _, whole) = self.lay(n, gap);
            // A lay the frame's spent budget stopped is no answer: read as
            // past every gap, and the search, reading the budget spent,
            // keeps the widest gap found to fit.
            if !whole {
                return (f32::NEG_INFINITY, lay.out);
            }
            let v = self.room_of(&lay, gap);
            let (end, at) = self.end_of(&lay);
            let v = if v >= 0.0 {
                v.max(f32::MIN_POSITIVE)
            } else if at >= self.cb || end.is_nan() || end <= TOUCH {
                v.min(-2.0 * tolerance)
            } else {
                v
            };
            (v, lay.out)
        };
        let stop = || self.side.work.spent_all();
        let mut low = (self.floor, at_floor, floor_out);
        let mut g = self.straight(n, at_floor) + (at_floor / gaps) * 1e-3 + f32::MIN_POSITIVE;
        for _ in 0..TRACK_DOUBLINGS {
            if !g.is_finite() || stop() {
                break;
            }
            let (v, out) = room(g);
            if v < 0.0 {
                return falsi(
                    room,
                    low,
                    (g, v, out),
                    tolerance,
                    tolerance / gaps,
                    SIDE_ROUNDS,
                    &stop,
                );
            }
            if v <= tolerance {
                return (g, out);
            }
            let next = (g + v / gaps * (1.0 + 1e-3)).max(g * 1.125);
            low = (g, v, out);
            g = next;
        }
        (low.0, low.2)
    }

    /// Fill the arc (ruling R33.1): the whole number of copies whose
    /// equal gap is nearest `target`, at least one and at most
    /// [`MAX_SIDE_COPIES`] or as many as `budget` glyphs hold — weighing
    /// the most that fit at the target and one more, whose gap is
    /// narrower — every gap and both joins equal. Nothing when not even
    /// one copy fits, or the budget holds none. Nothing, and `false`, when
    /// the frame's work budget is spent before the floor's row is laid
    /// (rulings R44.2, R47.2): the arc is left empty.
    ///
    /// The two counts weighed are those whose gaps bracket the target:
    /// guessed from the floor's row by the straight path, then moved a
    /// count at a time until the gaps solved bracket it, so a guess the
    /// path's bend puts off by one costs a solve, never the answer.
    ///
    /// Cost (ruling R44.3): the floor's row, and two solves where the
    /// guess holds — a lay of every copy each widening and each of
    /// [`falsi`]'s rounds, the copies chosen kept as laid.
    fn fill(&self, target: f32, budget: usize, out: &mut Vec<PlacedGlyph>) -> bool {
        let per_copy = self
            .side
            .clusters
            .iter()
            .map(|c| c.glyphs.len())
            .sum::<usize>()
            .max(1);
        let most = MAX_SIDE_COPIES.min(budget / per_copy);
        if most == 0 {
            return true;
        }
        let Some((row, rooms)) = self.floor_row(most) else {
            return false;
        };
        let fit = rooms.len();
        if fit == 0 {
            return true;
        }
        let mut solved: Vec<Option<(f32, Vec<PlacedGlyph>)>> = vec![None; fit + 1];
        let mut solve = |n: usize| -> f32 {
            if solved[n].is_none() {
                let (room, len) = rooms[n - 1];
                solved[n] = Some(self.solve(n, room, row[..len].to_vec()));
            }
            solved[n].as_ref().map_or(self.floor, |s| s.0)
        };
        // The most copies that fit at the target, by the straight path: the
        // count whose gap is the last at or past it.
        let guess = (1..=fit)
            .take_while(|&k| self.straight(k, rooms[k - 1].0) >= target)
            .count()
            .clamp(1, fit);
        let mut n = guess;
        // Down while `n` copies do not reach the target, up while one more
        // still does: the pair whose gaps bracket it, or an end. A spent
        // budget leaves the guess as it stands, its gaps solved no wider
        // than found to fit.
        let spent = || self.side.work.spent_all();
        while n > 1 && solve(n) < target && !spent() {
            n -= 1;
        }
        while n < fit && solve(n + 1) >= target && !spent() {
            n += 1;
        }
        let mut best = (n, solve(n));
        if n < fit {
            let wider = solve(n + 1);
            if (wider - target).abs() < (best.1 - target).abs() {
                best = (n + 1, wider);
            }
        }
        if let Some((_, glyphs)) = solved[best.0].take() {
            out.extend(glyphs);
        }
        true
    }
}

/// A text ink a side arc runs from or to ([`side_arcs`]): its box, and
/// where along the first half's path it is set.
type ArcEnd = (InkBox, f32);

/// The text inks each arc of a side runs between, and which junctions
/// (top-left, bottom-right) the arc lies round: with two lines one arc
/// per junction; with one, one arc round both, over the half it leaves.
///
/// Each end ink comes with where it is set along the first half's path:
/// exactly, for the first half's line; mirrored, for the second's, whose
/// path runs round the other way from the same start ([`Halves`]).
/// `length` is the outline's.
fn side_arcs(lines: &[Line], length: f32) -> Vec<([bool; 2], ArcEnd, ArcEnd)> {
    let ends: Vec<(usize, (ArcEnd, ArcEnd))> = lines
        .iter()
        .filter_map(|l| {
            let (a, b) = l.tracked.ends?;
            let (first, last) = l.tracked.centers?;
            let on_first = |s: f32| if l.half == 0 { s } else { length - s };
            Some((l.half, ((a, on_first(first)), (b, on_first(last)))))
        })
        .collect();
    match ends.as_slice() {
        [(_, (a1, b1)), (_, (a2, b2)), ..] => vec![([true, false], *a2, *a1), ([false, true], *b1, *b2)],
        // The first half's line leaves the second: clockwise from its
        // last ink round to its first.
        [(0, (a, b))] => vec![([true, true], *b, *a)],
        // The second half's line leaves the first: clockwise from its
        // first ink over the top to its last.
        [(_, (a, b))] => vec![([true, true], *a, *b)],
        [] => Vec::new(),
    }
}

/// The lines' letters' mean ink half size: what pushes a side ornament
/// onto the text's center line.
fn mean_size(lines: &[Line]) -> Vec2 {
    lines.iter().map(|l| l.size).sum::<Vec2>() / lines.len().max(1) as f32
}

impl Layout<'_> {
    /// Whether junction `j` (top-left 0, bottom-right 1) of `laid` is a
    /// break (ruling R33.1): the lines' end inks there at least as far
    /// apart as [`apart`](Self::apart) wants — a distance that is not a
    /// number is not — and, with `side` (the ornament's run and glyphs),
    /// one whole copy fitting between them at its own floor gap: the
    /// test [`SideArc::solve`] makes before it solves anything, so the
    /// two cannot disagree, and all this asks — whether, not how wide.
    /// With it, the junction's measure.
    fn met(&self, laid: &[Line], side: Option<(&ShapedRun, &Range<usize>)>, j: usize) -> (bool, Junctions) {
        let measured = self.apart(laid);
        let words = measured.apart[j] >= measured.want[j] * CLEAR_MARGIN;
        let copy = side.is_none_or(|(drawn, range)| {
            let side = SideOrnament::new(self, drawn, range.clone(), mean_size(laid));
            side_arcs(laid, self.halves.length)
                .into_iter()
                .filter(|(at, _, _)| at[j])
                .all(|(_, from, to)| side.arc(from, to).is_none_or(|arc| arc.fits(1, arc.floor)))
        });
        (words && copy, measured)
    }

    /// The lines laid at `span`, each line's clearance from each
    /// junction grown from `seed` at the whole span (ruling R33.1) and
    /// then, below whole, grown on at `span` until each junction is a
    /// break there too (ruling R35.1): so a line at a narrower span lies
    /// within where it lies at whole. Returns the lines, their clearance,
    /// and whether each junction (top-left, bottom-right) is a break.
    fn lay_clear(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        seed: Clear,
        backtracks: usize,
    ) -> Cleared {
        let mut search = Search {
            memo: LayMemo::default(),
            backtracks,
        };
        let whole = self.grow(run, lines, side, 1.0, seed, &mut search);
        if span >= 1.0 {
            return whole;
        }
        self.grow(run, lines, side, span, whole.clear, &mut search)
    }

    /// The lines laid at `span` with each line's clearance grown from
    /// `clear` until each junction is a break ([`met`](Self::met)).
    ///
    /// A junction's clearance grows, round by round, while it is not a
    /// break ([`met`](Self::met)). Each round shares what is missing —
    /// the words' shortfall, ends that overlap short by how deep they do
    /// too, or for a copy of the side ornament the room along the path it
    /// lacks — among the lines whose drawing back parts their ends there,
    /// by how much it does, measured afresh each round: where both halves
    /// run on side by side, as on a long flat outline, only the line whose
    /// end moves away is drawn back. Drawing a line back narrows its
    /// tracking, and so the word gap the break is measured in: the step is
    /// damped by how much that eases the break wanted, or, where the last
    /// round grew the junction alone and by all it asked, aimed by what
    /// that round gave — a secant on the junction's own measure (ruling
    /// R43). A round that would press a line it left unpressed is halved
    /// until it does not ([`BACKTRACKS`] times at most), so the rounds
    /// close on the least clearance from below rather than overshoot. A
    /// junction stops growing when no line's drawing back parts its ends,
    /// when a line is pressed, or after [`CLEAR_ROUNDS`] rounds: the bound
    /// on its work.
    fn grow(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        mut clear: Clear,
        search: &mut Search,
    ) -> Cleared {
        let backtracks = search.backtracks;
        let memo = &mut search.memo;
        let mut laid = self.lay(run, lines, clear, span, memo);
        // Each junction's depth before the round that grew it, if the last
        // round did; how much that round grew it; whether it grew it
        // alone — growth at the other junction narrows both lines' gaps,
        // and moves this one's ends with them, either way — and by all its
        // step asked, none held to a line's slack nor halved for a press:
        // what its next step is aimed by.
        let mut depth_before = [f32::NEG_INFINITY; 2];
        let mut grown_by = [0.0_f32; 2];
        let mut alone = [false; 2];
        let mut free = [false; 2];
        let mut stuck = [false; 2];
        let mut met_now: Option<[bool; 2]> = None;
        for _ in 0..CLEAR_ROUNDS {
            // A pressed line ends the search: the frame goes to its last
            // resorts from here — a shrink, a cut — and they grow the
            // clearance again for what they leave, so rounds that grow it
            // round a line already too long for its room are work spent
            // on nothing (ruling R38.3).
            if self.spent_all() || laid.iter().any(|l| l.pressed) {
                break;
            }
            let mut grow: Clear = [[0.0; 2]; 2];
            let mut grown = false;
            // Whether a line's slack held back this round's growth at
            // each junction.
            let mut held = [false; 2];
            let mut met_here = [false; 2];
            let mut measured_helps = [[0.0_f32; 2]; 2];
            let measures = [0, 1].map(|j| self.met(&laid, side, j));
            for j in 0..2 {
                let (met, measured) = &measures[j];
                let met = *met;
                met_here[j] = met;
                for (row, helps) in measured_helps.iter_mut().zip(measured.helps) {
                    row[j] = helps[j].max(0.0);
                }
                if met || stuck[j] {
                    continue;
                }
                let apart = measured.apart[j];
                let depth = measured.depth[j];
                let aim = measured.want[j] * CLEAR_MARGIN;
                let helps = [0, 1].map(|k| measured.helps[k][j].max(0.0));
                let total = helps[0] + helps[1];
                if total.partial_cmp(&0.0) != Some(Ordering::Greater) {
                    stuck[j] = true;
                    continue;
                }
                // Ends whose distance is not a number are short of it.
                let by_words = matches!(apart.partial_cmp(&aim), None | Some(Ordering::Less));
                let shortfall = if by_words {
                    // Ends that overlap are short by how deep they do too.
                    aim - if depth.is_finite() {
                        depth.min(apart)
                    } else {
                        apart
                    }
                } else {
                    // Room for a copy of the side ornament is room along
                    // the path between the end inks: the arcs from the
                    // junction to each line's end ink, less the inks'
                    // own halves.
                    let room = side.map_or(0.0, |(drawn, range)| {
                        SideOrnament::new(self, drawn, range.clone(), mean_size(&laid)).room()
                    });
                    let arc: f32 = laid
                        .iter()
                        .map(|l| {
                            let (ja, jb) = self.halves.junctions[l.half];
                            let from = if j == 0 { l.part.0 - ja } else { jb - l.part.1 };
                            from - l.size.x
                        })
                        .sum();
                    // The arc measured on the lines' paths is not quite
                    // the room the ornament's own walk finds: while the
                    // copy still does not fit, it grows by a quarter of a
                    // copy at the least.
                    (room - if laid.len() > 1 { arc } else { apart }).max(0.25 * room)
                };
                // Drawing a line back narrows its tracking, and so the word
                // gap the break is measured in: by twice its gap's fall,
                // which is the span's share of the clearance spread over
                // its gaps. A step that counts only the inks' parting
                // overshoots by that much, so it is cut to the share of
                // the closing the parting makes (ruling R38.3); with no
                // word gap in the measure — a break of a copy's room, or
                // of not touching — nothing eases.
                let eases = [0, 1].map(|k| {
                    let words = laid.iter().filter(|l| l.word.is_some()).count();
                    laid.iter()
                        .find(|l| l.half == k)
                        .filter(|l| {
                            by_words
                                && l.tracks
                                && l.gap > 0.0
                                && measured.want[j] > TOUCH
                                && (words == 0 || l.word.is_some())
                        })
                        .map_or(0.0, |l| {
                            let among = if words == 0 { laid.len() } else { words };
                            // A single line owes half the break.
                            let owed = if laid.len() > 1 { 1.0 } else { 0.5 };
                            owed * JUNCTION_WORD_GAPS * 2.0 * span / (l.steps.max(1) * among.max(1)) as f32
                        })
                });
                let shares = helps.map(|h| h / total.max(1.0));
                let parting: f32 = (0..2).map(|k| helps[k] * shares[k]).sum();
                let closing: f32 = (0..2).map(|k| (helps[k] + eases[k]) * shares[k]).sum();
                let damped = if closing > 0.0 {
                    shortfall * parting / closing
                } else {
                    shortfall
                };
                // A break wider than any `f32` — two word gaps of a line
                // tracked over an outline as long as an `f32` — is short
                // by an infinite amount, and an infinite step shared by
                // a share of zero, or held to a line's slack, is not a
                // number: the step is held to the largest `f32`, past
                // which a clearance presses its line to a point however
                // much more it is (the probe in
                // `test_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely`).
                // A step that is not a number — a shortfall that is not
                // one — grows by the least.
                let mut step = if damped.is_nan() {
                    MIN_GROWTH
                } else {
                    damped.clamp(MIN_GROWTH, f32::MAX)
                };
                // Where the last round grew this junction alone, by all its
                // step asked, the step is aimed by what that round gave: the
                // shortfall over the depth it gained a point of growth, at
                // most four times the last step — a secant on the junction's
                // own measure (ruling R43), where the damping guesses how
                // much drawing back eases the break, and closes on it a
                // share a round.
                let gained = (depth - depth_before[j]) / grown_by[j];
                if by_words && alone[j] && free[j] && gained.is_finite() && gained > 0.0 {
                    let secant = (aim - depth) / gained;
                    if secant.is_finite() {
                        step = secant.clamp(MIN_GROWTH, (4.0 * grown_by[j]).max(MIN_GROWTH));
                    }
                }
                for k in 0..2 {
                    // Two lines that both draw back part by about the
                    // sum: each gives its share.
                    grow[k][j] = step * shares[k];
                }
                grown = true;
            }
            // A line drawn back past its slack is pressed, where the
            // other line at the junction may have room to give: each
            // line's growth is held to its slack, what is cut handed to
            // the other line where it parts the ends there too and has
            // room (rulings R34, R38.1). Where no line that parts a
            // junction has room the round grows as it would, and the line
            // it presses takes the last resorts.
            for k in 0..2 {
                let Some(line) = laid.iter().find(|l| l.half == k) else {
                    continue;
                };
                let wanted = grow[k][0] + grow[k][1];
                if wanted <= line.slack || wanted <= 0.0 {
                    continue;
                }
                let other = 1 - k;
                let mut room = laid
                    .iter()
                    .find(|l| l.half == other)
                    .map_or(0.0, |l| (l.slack - grow[other][0] - grow[other][1]).max(0.0));
                let keep = line.slack / wanted;
                for j in 0..2 {
                    let cut = grow[k][j] * (1.0 - keep);
                    let theirs = measured_helps[other][j];
                    let mine = measured_helps[k][j];
                    let handed = if theirs > 0.0 {
                        (cut * mine / theirs).min(room)
                    } else {
                        0.0
                    };
                    if line.slack >= MIN_GROWTH || handed > 0.0 {
                        held[j] |= cut > 0.0;
                        grow[k][j] -= cut;
                        grow[other][j] += handed;
                        room -= handed;
                    }
                }
            }
            let grew = [0, 1].map(|j| grow[0][j] > 0.0 || grow[1][j] > 0.0);
            for j in 0..2 {
                depth_before[j] = if grew[j] {
                    measures[j].1.depth[j]
                } else {
                    f32::NEG_INFINITY
                };
                alone[j] = grew[j] && !grew[1 - j];
            }
            met_now = Some(met_here);
            if !grown {
                break;
            }
            let pressed: Vec<bool> = laid.iter().map(|l| l.pressed).collect();
            // A round that draws a line back past what slack it has,
            // there being no other line with room to take it, presses it
            // knowingly: halving it would only close on the same press in
            // smaller steps (ruling R38.3).
            let foreseen = laid
                .iter()
                .any(|l| !l.pressed && l.slack < MIN_GROWTH && grow[l.half][0] + grow[l.half][1] > l.slack);
            let backtracks = if foreseen { 0 } else { backtracks };
            let mut scale = 1.0;
            for _ in 0..=backtracks {
                // The frame's budget spent: the round not taken, the lines
                // as the last round laid them (ruling R44.2).
                if self.spent_all() {
                    break;
                }
                let mut next = clear;
                for k in 0..2 {
                    for j in 0..2 {
                        next[k][j] += grow[k][j] * scale;
                    }
                }
                let tried = self.lay(run, lines, next, span, memo);
                let presses = tried.iter().zip(&pressed).any(|(l, was)| l.pressed && !was);
                if !presses || scale < 0.5_f32.powi(backtracks as i32) * 1.5 {
                    clear = next;
                    laid = tried;
                    met_now = None;
                    free = held.map(|h| !h && scale == 1.0);
                    grown_by = [0, 1].map(|j| (grow[0][j] + grow[1][j]) * scale);
                    break;
                }
                scale *= 0.5;
            }
        }
        let met = met_now.unwrap_or_else(|| [0, 1].map(|j| self.met(&laid, side, j).0));
        Cleared {
            lines: laid,
            clear,
            met,
        }
    }
}

/// A frame's lines laid with their clearance ([`Layout::lay_clear`]).
struct Cleared {
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction, top-left and bottom-right, is a break.
    met: [bool; 2],
}

/// Whether two of `lines`' inks touch (ruling R35.2): any two of
/// different lines, or of one line but not neighbors — two neighbors
/// are as far apart as the line's tracking sets them, a font with no
/// side bearings' zero gap included. Sorted by their left edges, each
/// box is held only against those that start before it ends.
///
/// Cost: O(n log n) in the lines' inks, and a pair per two that share
/// some of their width.
fn overlapping(lines: &[Line], work: &Work) -> bool {
    let boxes = lines
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.tracked.boxes.iter().enumerate().map(move |(i, b)| ((k, i), *b)))
        .collect();
    touching(boxes, |(k, i), (l, j)| k == l && i.abs_diff(j) <= 1, work)
}

/// Whether two of `boxes` touch — no more than [`TOUCH`] apart
/// ([`box_gap`]) — but for the pairs `exempt` excuses, each box known by
/// its key ([`touch_scan`]): each box and each pair of them looked at
/// counted against `work` (ruling R46.1).
fn touching<K: Copy>(boxes: Vec<(K, InkBox)>, exempt: impl Fn(K, K) -> bool, work: &Work) -> bool {
    let mut looked = 0;
    let touches = touch_scan(&boxes, &exempt, &mut looked);
    #[cfg(test)]
    BOXES_WEIGHED.with(|count| count.set(count.get() + looked));
    work.take(weighing_steps(boxes.len().saturating_add(looked)));
    touches
}

/// How much wider than the widest box and [`TOUCH`] a cell of
/// [`touch_scan`]'s grid is: enough that rounding a box's corner to its
/// cell never puts two boxes that touch two cells apart.
const CELL_MARGIN: f64 = 1.0 + 1.0 / (1u64 << 20) as f64;

/// Whether two of `boxes` touch, as [`touching`] asks, the pairs of them
/// looked at added to `looked`.
///
/// The pairs weighed are a sweep's: sorted by where each box starts along
/// the axis the boxes spread further on, ties in their order, a box is
/// weighed against each after it that starts no further than [`TOUCH`]
/// past where it ends, a box that is not a number ending the scan. Of
/// those pairs only the ones that can touch are looked at. Boxes whose
/// corners are numbers are bucketed by their least corner in a grid whose
/// cells are as wide as the widest box and [`TOUCH`] and as tall as the
/// tallest and [`TOUCH`] ([`CELL_MARGIN`]): two that touch lie in one
/// cell or in neighboring ones. A box with a corner that is not a number,
/// which [`box_gap`] can read as touching anything, is weighed as the
/// sweep weighs it: against each box its scan reaches, and each box whose
/// scan reaches it.
///
/// Cost: a sort, and a look at each pair of boxes in neighboring cells:
/// round any outline, the few inks either side of each. A sweep along one
/// axis looked at every pair that shared some of its width, quadratic in
/// a side ornament's copies down a straight side (R44.3's 200-byte
/// ornament round a sharp square, some 5,000 pairs a glyph). A box that
/// is not a number adds the boxes its scan reaches.
fn touch_scan<K: Copy>(boxes: &[(K, InkBox)], exempt: &impl Fn(K, K) -> bool, looked: &mut usize) -> bool {
    let spread = |axis: usize| {
        let (lo, hi) = boxes
            .iter()
            .map(|(_, b)| b.0[axis])
            .filter(|v| v.is_finite())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            });
        hi - lo
    };
    let axis = if spread(1) > spread(0) { 1 } else { 0 };
    let start = |i: usize| boxes[i].1 .0[axis];
    // Whether the sweep weighs box `i` with `j`, after it — `j` starting
    // no further than `TOUCH` past where `i` ends — and the two touch.
    let weigh = |i: usize, j: usize| {
        let ((k, a), (l, b)) = (boxes[i], boxes[j]);
        b.0[axis].le(&(a.1[axis] + TOUCH)) && !exempt(k, l) && box_gap(a, b) <= TOUCH
    };
    let regular = |i: usize| boxes[i].1 .0.is_finite() && boxes[i].1 .1.is_finite();

    // The boxes not numbers, weighed as the sweep weighs them.
    if !(0..boxes.len()).all(regular) {
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        order.sort_by(|&i, &j| start(i).total_cmp(&start(j)));
        let odd: Vec<usize> = (0..order.len()).filter(|&at| !regular(order[at])).collect();
        for (at, &i) in order.iter().enumerate() {
            let reach = boxes[i].1 .1[axis] + TOUCH;
            // A box that is a number: the odd boxes its scan reaches, the
            // boxes after it starting in order until one starts past
            // `reach` — so the odd ones in order until one does.
            let after: Box<dyn Iterator<Item = usize>> = if regular(i) {
                Box::new(odd[odd.partition_point(|&o| o <= at)..].iter().map(|&o| order[o]))
            } else {
                Box::new(order[at + 1..].iter().copied())
            };
            for j in after {
                *looked += 1;
                if !start(j).le(&reach) {
                    break;
                }
                if weigh(i, j) {
                    return true;
                }
            }
        }
    }

    // The boxes that are numbers, by cell: sorted by column, then row.
    let size = [0, 1].map(|d| {
        let widest = (0..boxes.len())
            .filter(|&i| regular(i))
            .map(|i| boxes[i].1 .1[d] - boxes[i].1 .0[d])
            .fold(0.0_f32, f32::max);
        (f64::from(widest) + f64::from(TOUCH)) * CELL_MARGIN
    });
    // A cell past `i64`'s range is its end's, shared by every box there.
    let cell_of = |i: usize| {
        let corner = boxes[i].1 .0;
        let at = |d: usize| (f64::from(corner[d]) / size[d]).floor() as i64;
        (at(0), at(1))
    };
    let mut cells: Vec<((i64, i64), usize)> = (0..boxes.len())
        .filter(|&i| regular(i))
        .map(|i| (cell_of(i), i))
        .collect();
    cells.sort_unstable();
    // Which of a pair the sweep weighs first.
    let pair = |i: usize, j: usize| match start(i).total_cmp(&start(j)).then(i.cmp(&j)) {
        Ordering::Less => weigh(i, j),
        _ => weigh(j, i),
    };
    // Each pair once: a box with the boxes after it in its cell and the
    // cell above, then with the three cells of the next column beside.
    let mut next = 0;
    for (n, &((column, row), i)) in cells.iter().enumerate() {
        let above = row.saturating_add(1);
        for &((c, r), j) in &cells[n + 1..] {
            if c != column || r > above {
                break;
            }
            *looked += 1;
            if pair(i, j) {
                return true;
            }
        }
        let Some(beside) = column.checked_add(1) else {
            continue;
        };
        let from = (beside, row.saturating_sub(1));
        while next < cells.len() && cells[next].0 < from {
            next += 1;
        }
        for &((c, r), j) in &cells[next..] {
            if c != beside || r > above {
                break;
            }
            *looked += 1;
            if pair(i, j) {
                return true;
            }
        }
    }
    false
}

/// `lines` cut to the longest run of their inks, the first line's then
/// the second's, of which no two touch ([`overlapping`]), and the halves
/// of the lines cut: the cheapest cut that leaves no ink touching, for a
/// frame whose work budget is spent before its last resorts held it
/// apart (rulings R38.1, R44.2). A line cut to no ink is left out.
///
/// Cost: a bisection of [`overlapping`]s over the frame's inks.
fn untouched(lines: &[Line], work: &Work) -> (Vec<Line>, Vec<usize>) {
    let total: usize = lines.iter().map(|l| l.tracked.boxes.len()).sum();
    #[cfg(test)]
    let (spent, reserved) = (work.spent.get(), work.reserved.replace(true));
    let cut_to = |m: usize| {
        let mut left = m;
        let mut out = Vec::new();
        for line in lines {
            if left == 0 {
                break;
            }
            let keep = left.min(line.tracked.boxes.len());
            left -= keep;
            let mut line = line.clone();
            if keep < line.tracked.boxes.len() {
                line.tracked.glyphs.truncate(line.tracked.marks[keep]);
                line.tracked.boxes.truncate(keep);
                line.tracked.marks.truncate(keep);
            }
            out.push(line);
        }
        out
    };
    // Apart with no ink, touching with all; the longest run apart lies
    // between.
    let (mut apart, mut over) = (0, total);
    while over - apart > 1 {
        let mid = apart + (over - apart) / 2;
        // Each look cuts the lines again: as much work as weighing them.
        work.take(weighing_steps(mid));
        if overlapping(&cut_to(mid), work) {
            over = mid;
        } else {
            apart = mid;
        }
    }
    let kept = cut_to(apart);
    #[cfg(test)]
    {
        work.reserved.set(reserved);
        let taken = work.spent.get() - spent;
        UNTOUCHED_TAKEN.with(|most| {
            if taken > most.get().0 {
                most.set((taken, total));
            }
        });
    }
    let cut = lines
        .iter()
        .enumerate()
        .filter(|(k, l)| {
            kept.get(*k)
                .is_none_or(|c| c.tracked.boxes.len() < l.tracked.boxes.len())
        })
        .map(|(_, l)| l.half)
        .collect();
    (kept, cut)
}

/// One end of a bracket [`falsi`] closes: where, the function's value
/// there, and what the function made there.
type End<T> = (f32, f32, T);

/// The root of a function falling through a bracket — positive at
/// `low`, not positive at `high`, each an [`End`] — by the Illinois
/// regula falsi: each probe where the line through the bracket's ends
/// crosses zero, the value at an end kept twice running halved, so a
/// bend in the function cannot hold one end still. A probe that would
/// not fall strictly inside the bracket — an end that is not finite, a
/// line that crosses outside — is taken at the middle instead. It stops
/// at a probe within `tolerance` of zero, at a bracket no probe can
/// narrow (the middle is an end: no change) or no wider than `width` —
/// a function with a step in it, as a line's length is where a letter
/// crosses a corner, need never come within tolerance — or after `rounds` probes,
/// and returns the probe within tolerance, or else the low end: the
/// root approached from where the function is still positive.
///
/// `f` returns a value that is a number: the caller says which side a
/// value that is not one reads as.
///
/// Cost: `rounds` calls of `f` at most, and as few as one where the
/// function is near a line — what a line's length is in its gap, and
/// its centering in its start, on any path but a sharp bend.
fn falsi<T>(
    mut f: impl FnMut(f32) -> (f32, T),
    mut low: End<T>,
    mut high: End<T>,
    tolerance: f32,
    width: f32,
    rounds: usize,
    stop: &dyn Fn() -> bool,
) -> (f32, T) {
    // The values the line is drawn through: an end's own, halved while
    // the other end moves and it does not.
    let (mut at_low, mut at_high) = (low.1, high.1);
    let mut kept: Option<bool> = None;
    for _ in 0..rounds {
        // The frame's work budget spent: the bracket as it stands
        // (ruling R44.2).
        if stop() {
            break;
        }
        let mid = low.0.midpoint(high.0);
        if !(mid > low.0 && mid < high.0) || high.0 - low.0 <= width {
            break;
        }
        let crossing = low.0 + (high.0 - low.0) * at_low / (at_low - at_high);
        let x = if at_low.is_finite() && at_high.is_finite() && crossing > low.0 && crossing < high.0 {
            crossing
        } else {
            mid
        };
        let (value, made) = f(x);
        if value > 0.0 {
            let near = value <= tolerance;
            low = (x, value, made);
            at_low = value;
            if kept == Some(true) {
                at_high *= 0.5;
            }
            kept = Some(true);
            if near {
                return (low.0, low.2);
            }
        } else {
            let near = -value <= tolerance;
            high = (x, value, made);
            at_high = value;
            if kept == Some(false) {
                at_low *= 0.5;
            }
            kept = Some(false);
            if near {
                return (high.0, high.2);
            }
        }
    }
    (low.0, low.2)
}

/// How near its part's end a line's last ink is laid, by arc length, in
/// points: [`solve_tracking`]'s tolerance, at most a millionth of a
/// point of tracking per gap on a line of a hundred.
const SHORT_TOLERANCE: f32 = 1e-4;

/// How near its part's end, in points, a line of `steps` gaps whose
/// part ends at `end` is laid: [`SHORT_TOLERANCE`] where the path's
/// floats resolve it ([`resolved`]), but no finer than the line's letters
/// are set — each to within the pairwise rule's tolerance, so the line's
/// length moves by up to that many times its gaps as its gap moves, and a
/// solve held finer chases that round after round (ruling R43).
fn line_tolerance(steps: usize, end: f32) -> f32 {
    resolved(SHORT_TOLERANCE, end).max(steps.max(1) as f32 * INK_TOLERANCE)
}

/// How narrow a line's tracking bracket closes to, in points of gap: a
/// tenth of [`GAP_TOLERANCE`], what the gap a line has is judged to.
const GAP_PRECISION: f32 = 0.1 * GAP_TOLERANCE;

/// The least gap at which `track` lays a line's last ink at its part's
/// end — [`Tracked::short`] no longer positive, the room it leaves
/// shrinking as the gap grows — and the line laid at it: 0 where the
/// line fills its part at no gap, and at most about `2^TRACK_DOUBLINGS`.
/// `steps` is how many gaps the line has, each letter's but the first and
/// each space's — what its length grows by per point of gap on a
/// straight path.
///
/// The line's length is its inks and spaces and `steps` times its gap,
/// bent only by the path under it: so the first probe is that line's
/// root, its room at no gap shared among its gaps, aimed a hair past it
/// so it brackets the root, and [`falsi`] closes the bracket from there
/// (ruling R38.3). `end` is where the part ends, what the tolerance on
/// the line's end is resolved against ([`resolved`]).
///
/// With `warm`, a gap near the root — the line's last lay's, moved by
/// its part's change in length (ruling R43) — that is probed first, and
/// then the gap the straight path's slope from it gives, aimed a hair
/// past the root, toward it: where the two bracket the root, [`falsi`]
/// closes it from there, and the line is laid in two or three tracks;
/// where they do not, the solve starts afresh.
///
/// Cost: one [`track`] at no gap, one per bracket widening (none on a
/// straight path), and [`falsi`]'s, at most [`TRACK_ROUNDS`]; two more
/// with `warm`, where they do not bracket the root.
fn solve_tracking(
    track: impl Fn(f32) -> Tracked,
    steps: usize,
    end: f32,
    warm: Option<f32>,
    stop: &dyn Fn() -> bool,
) -> (f32, Tracked) {
    let per_gap = steps.max(1) as f32;
    // The bracket closes no finer than the line's precision spread over
    // its gaps: past it, a narrower bracket moves the line's end by less
    // than its letters are set to.
    let tolerance = line_tolerance(steps, end);
    let width = GAP_PRECISION.max(tolerance / per_gap);
    // A room that is not a number reads as past the end.
    let short = |t: &Tracked| {
        if t.short.is_nan() {
            f32::NEG_INFINITY
        } else {
            t.short
        }
    };
    if let Some(g0) = warm.filter(|g| g.is_finite() && *g > 0.0) {
        let first = track(g0);
        let v0 = short(&first);
        if v0.abs() <= tolerance {
            return (g0, first);
        }
        // On along the straight path's slope, a hair past the root.
        let toward = g0 + v0 / per_gap;
        let g1 = if v0 > 0.0 {
            toward * (1.0 + 1e-3) + f32::MIN_POSITIVE
        } else {
            toward * (1.0 - 1e-3)
        };
        if g1.is_finite() && g1 > 0.0 {
            let second = track(g1);
            let v1 = short(&second);
            if v1.abs() <= tolerance {
                return (g1, second);
            }
            let bracket = match (v0 > 0.0, v1 > 0.0) {
                (true, false) => Some(((g0, v0, first), (g1, v1, second))),
                (false, true) => Some(((g1, v1, second), (g0, v0, first))),
                _ => None,
            };
            if let Some((low, high)) = bracket {
                return falsi(
                    |g| {
                        let t = track(g);
                        (short(&t), t)
                    },
                    low,
                    high,
                    tolerance,
                    width,
                    TRACK_ROUNDS,
                    stop,
                );
            }
        }
    }
    let at_zero = track(0.0);
    // No room even at no gap, or a room that is not a number: no gap.
    if at_zero.short.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return (0.0, at_zero);
    }
    let aim = |g: f32| g * (1.0 + 1e-3) + f32::MIN_POSITIVE;
    let mut low = (0.0_f32, at_zero.short, at_zero);
    let mut g = aim(low.1 / per_gap);
    for round in 0..TRACK_DOUBLINGS {
        if !g.is_finite() || stop() {
            break;
        }
        let tracked = track(g);
        let value = short(&tracked);
        if value <= 0.0 {
            if -value <= tolerance {
                return (g, tracked);
            }
            return falsi(
                |g| {
                    let t = track(g);
                    (short(&t), t)
                },
                low,
                (g, value, tracked),
                tolerance,
                width,
                TRACK_ROUNDS,
                stop,
            );
        }
        if value <= tolerance {
            return (g, tracked);
        }
        // Still short: on along the line through the last two probes,
        // at least an eighth further, at least twice as far once a few
        // probes have not reached it, at most four times.
        let fall = (low.1 - value) / (g - low.0);
        let line = if fall > 0.0 {
            g + value / fall
        } else {
            f32::INFINITY
        };
        let least = if round < 4 { 1.125 } else { 2.0 };
        let next = aim(line).clamp(g * least, g * 4.0);
        low = (g, value, tracked);
        g = next;
    }
    (low.0, low.2)
}

/// A line's letters' mean ink half size and their font's own gap — their
/// mean side bearings; `None` without a letter.
fn letters_of(clusters: &[Cluster]) -> Option<(Vec2, f32)> {
    let letters: Vec<(f32, Ink)> = clusters
        .iter()
        .filter(|c| !c.blank)
        .filter_map(|c| c.ink.map(|i| (c.advance, i)))
        .collect();
    if letters.is_empty() {
        return None;
    }
    let count = letters.len() as f32;
    let half = letters.iter().map(|(_, i)| (i.max - i.min) * 0.5).sum::<Vec2>() / count;
    let gap = letters.iter().map(|(a, i)| a - (i.max.x - i.min.x)).sum::<f32>() / count;
    Some((half, gap))
}

/// The walk a line is tracked along on `path`.
fn line_walk<'p>(path: &'p ArcPath, run: &ShapedRun, outward: bool) -> PathWalk<'p> {
    PathWalk {
        path,
        memo: None,
        line_height: run.line_height,
        outward,
        limit: f32::INFINITY,
        level: false,
        ornament: true,
        baseline: run.baseline(),
        push: None,
    }
}

/// The frame's side ornament (ruling R33.1): its run (`run` and one
/// shaped copy of the ornament at the size the x-height cap sets, a line
/// of its own) and the ornament's glyphs in it. `None` without sides,
/// or when the ornament shapes to nothing. Its shaping and inking are
/// counted in `work` (ruling R46.1).
fn side_run(
    run: &ShapedRun,
    sides: &Option<(String, Rgba)>,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    work: &Work,
) -> Option<(ShapedRun, Range<usize>)> {
    let (content, color) = sides.as_ref()?;
    let size = run
        .glyphs
        .iter()
        .filter(|g| g.span != FILLER_SPAN)
        .map(|g| g.size)
        .fold(0.0_f32, f32::max);
    if size.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return None;
    }
    let at = |text: &str, size: f32, fs: &mut FontSystemWriteGuard| {
        let style = ResolvedText {
            size: FontSize::Pt(size),
            ..*flow
        };
        let shaped = shape_run(
            &[FlowSpan {
                text,
                style: &style,
                link: None,
            }],
            1.0,
            fs,
        );
        work.shaped(&shaped);
        work.inked(&shaped);
        let fsys: &mut FontSystem = fs;
        inked(&shaped, fsys)
            .glyphs
            .iter()
            .filter_map(|g| g.ink.map(|i| i.max.y - i.min.y))
            .fold(0.0_f32, f32::max)
    };
    let x_height = at("x", size, fs);
    let tall = at(content, size, fs);
    let ornament = if tall > x_height && tall > 0.0 && x_height > 0.0 {
        size * x_height / tall
    } else {
        size
    };
    let style = ResolvedText {
        size: FontSize::Pt(ornament),
        color: *color,
        ..*flow
    };
    let fsys: &mut FontSystem = fs;
    let with = reshape(run, 1.0, Some((content.as_str(), &style)), fsys);
    work.shaped(&with);
    let start = with.glyphs.iter().position(|g| g.span == FILLER_SPAN)?;
    let range = start..with.glyphs.len();
    Some((with, range))
}

impl Layout<'_> {
    /// Line `p` cut at its end to the longest prefix of whole clusters
    /// that its half less `clear` holds at its font's own gap — its
    /// trailing spaces left out, at least its first cluster kept — or
    /// `None` when it holds the whole line (ruling R34: a `Once` frame
    /// never cuts text its half holds).
    ///
    /// Cost: a bisection of [`track`]s over the line's clusters.
    fn cut(&self, run: &ShapedRun, p: &Prepared, clear: [f32; 2]) -> Option<Prepared> {
        let kept = self.held(run, p.half, &p.clusters, p.natural_gap, clear)?;
        self.prepare_clusters(run, p.half, kept, true)
    }

    /// The longest prefix of whole `clusters` that half `half` less
    /// `clear` holds at gap `gap`, its trailing spaces left out and at
    /// least its first cluster kept: [`cut`](Self::cut)'s, before the
    /// line is prepared. `None` when it holds them all, or there is
    /// nothing to cut.
    fn held(
        &self,
        run: &ShapedRun,
        half: usize,
        clusters: &[Cluster],
        gap: f32,
        clear: [f32; 2],
    ) -> Option<Vec<Cluster>> {
        let (lo, hi) = self.room(half, clear);
        let fits = |k: usize| {
            let kept = trimmed(&clusters[..k]);
            kept.is_empty() || self.reach(half, run, kept, lo, gap) <= hi - lo + resolved(SLIDE_TOLERANCE, hi)
        };
        let n = clusters.len();
        if fits(n) {
            return None;
        }
        // `fits` holds for no clusters and fails for all of them; the
        // longest prefix it holds for lies between.
        let (mut held, mut over) = (0, n);
        while over - held > 1 && !self.spent_all() {
            let mid = held + (over - held) / 2;
            if fits(mid) {
                held = mid;
            } else {
                over = mid;
            }
        }
        let kept = trimmed(&clusters[..held.max(1)]).to_vec();
        let kept = if kept.is_empty() {
            clusters[..1].to_vec()
        } else {
            kept
        };
        // Nothing left to cut: a line of one cluster its half cannot
        // hold is laid pressed.
        (kept.len() != n).then_some(kept)
    }
}

impl Layout<'_> {
    /// Line `i` of `lines` cut at its end to the longest prefix of whole
    /// clusters with which no two of the frame's inks touch, its
    /// clearance found again ([`lay_clear`](Self::lay_clear)) — `None`
    /// for the line when only leaving it out does so — or `None` when
    /// even leaving it out leaves two inks touching (ruling R38.1).
    ///
    /// The prefix is bisected on a cheap look (ruling R42.1): the frame
    /// with the line cut to it, laid once against the clearance `held`,
    /// and whether two of its inks touch. The clearance search runs once,
    /// on the prefix chosen; only where its inks touch even so is the
    /// prefix bisected again below it on whole searches.
    ///
    /// Cost: a bisection of [`lay`](Self::lay)s over the line's clusters
    /// and one clearance search; where that search's inks touch, a
    /// bisection of clearance searches below the prefix.
    fn cut_apart(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        i: usize,
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        held: Clear,
    ) -> Option<(Option<Prepared>, Cleared)> {
        let p = &lines[i];
        // The frame with line `i` cut to its first `k` clusters: the
        // line, `None` when it is left out, and the frame's lines.
        let frame_of = |k: usize| {
            let kept = trimmed(&p.clusters[..k]).to_vec();
            let line = if kept.is_empty() {
                None
            } else {
                self.prepare_clusters(run, p.half, kept, true)
            };
            let mut frame = lines.to_vec();
            match &line {
                Some(shorter) => frame[i] = shorter.clone(),
                None => {
                    frame.remove(i);
                }
            }
            (line, frame)
        };
        // That frame, its clearance found again, when no two of its inks
        // touch.
        let apart = |k: usize| {
            let (line, frame) = frame_of(k);
            let cleared = self.lay_clear(run, &frame, side, span, [[0.0; 2]; 2], BACKTRACKS);
            (!overlapping(&cleared.lines, &self.work)).then_some((line, cleared))
        };
        // That frame laid once against the clearance held: whether no two
        // of its inks touch.
        let looks_apart = |k: usize| {
            let (_, frame) = frame_of(k);
            !overlapping(
                &self.lay(run, &frame, held, span, &mut LayMemo::default()),
                &self.work,
            )
        };
        // Apart with none of the line's clusters, touching with all of
        // them; the longest prefix apart lies between.
        let (mut chosen, mut over) = (0, p.clusters.len());
        while over - chosen > 1 && !self.spent_all() {
            let mid = chosen + (over - chosen) / 2;
            if looks_apart(mid) {
                chosen = mid;
            } else {
                over = mid;
            }
        }
        if let Some(found) = apart(chosen) {
            return Some(found);
        }
        if chosen == 0 {
            return None;
        }
        // The look misled: the prefix is bisected below it on whole
        // clearance searches.
        let mut best = apart(0)?;
        let (mut held_k, mut over) = (0, chosen);
        while over - held_k > 1 && !self.spent_all() {
            let mid = held_k + (over - held_k) / 2;
            match apart(mid) {
                Some(found) => (held_k, best) = (mid, found),
                None => over = mid,
            }
        }
        Some(best)
    }
}

/// A frame's text laid at one size ([`place_halves`]).
struct Built {
    /// The run as drawn, inked.
    drawn: ShapedRun,
    /// The side ornament's glyphs in it.
    side: Option<Range<usize>>,
    prepared: Vec<Prepared>,
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction is a break ([`Layout::met`]).
    met: [bool; 2],
    /// Whether two of its inks touch ([`overlapping`]).
    overlap: bool,
}

/// What a frame's `lines` leave unheld (ruling R35.2), with whether
/// each junction is a break and whether two inks touch: a line its half
/// less the clearance cannot hold at its font's gap, a junction left
/// short of a break, two inks touching.
fn unheld_of(lines: &[Line], met: [bool; 2], overlap: bool) -> Unheld {
    Unheld {
        pressed: lines.iter().filter(|l| l.pressed).map(|l| l.half).collect(),
        junctions: [0, 1].into_iter().filter(|&j| !met[j]).collect(),
        overlap,
        sides: Vec::new(),
    }
}

impl Built {
    /// Whether its halves hold it: nothing left unheld (ruling R35.2).
    fn holds(&self) -> bool {
        unheld_of(&self.lines, self.met, self.overlap).is_empty()
    }

    /// Its clearance as a seed for the text at a larger size
    /// ([`SEED_SHARE`] of it).
    fn seed(&self) -> Clear {
        self.clear.map(|half| half.map(|c| c * SEED_SHARE))
    }
}

/// The ink box on the canvas of `glyph`, placed from `run` (inked):
/// `None` for one with no ink, a space.
fn glyph_box(run: &ShapedRun, glyph: &PlacedGlyph) -> Option<InkBox> {
    let ink = run.glyphs.get(glyph.glyph as usize)?.ink?;
    let pen = glyph.pos + Vec2::new(0.0, run.baseline());
    Some((pen + ink.min, pen + ink.max))
}

/// `clusters` without their trailing spaces.
fn trimmed(clusters: &[Cluster]) -> &[Cluster] {
    let end = clusters.iter().rposition(|c| !c.blank).map_or(0, |i| i + 1);
    &clusters[..end]
}

/// `clusters` cut to the glyphs `left` still holds, at a cluster — a
/// cluster's glyphs are placed together or not at all — its end's blanks
/// left out with what follows them, and `left` less what is kept: a
/// frame's text held to [`MAX_PLACED_GLYPHS`] (ruling R44.1). Returns
/// what is kept, and whether anything was cut.
fn to_cap(mut clusters: Vec<Cluster>, left: &mut usize) -> (Vec<Cluster>, bool) {
    let mut placed = 0;
    let fits = clusters
        .iter()
        .take_while(|c| {
            placed += c.glyphs.len();
            placed <= *left
        })
        .count();
    let cut = fits < clusters.len();
    if cut {
        clusters.truncate(fits);
        let end = trimmed(&clusters).len();
        clusters.truncate(end);
    }
    *left -= clusters.iter().map(|c| c.glyphs.len()).sum::<usize>();
    (clusters, cut)
}

/// Test seam (#294, ruling R44.1): `run`'s glyphs `range` cut to `left`
/// glyphs at a cluster ([`to_cap`]): the glyphs kept, as a range of the
/// run's, whether it was cut, and the glyphs left after.
#[cfg(test)]
pub(crate) fn to_cap_seam(run: &ShapedRun, range: Range<usize>, left: usize) -> (Range<usize>, bool, usize) {
    let mut left = left;
    let (kept, cut) = to_cap(clusters_of(&run.glyphs, range.clone()), &mut left);
    let end = kept.last().map_or(range.start, |c| c.glyphs.end);
    (range.start..end, cut, left)
}

/// Place `run` as a text frame around `outline` (rulings R27, R32, R33,
/// R33.1, R34): its two lines laid at `layout.span` along the diagonal
/// halves, each junction a break, `layout.sides` closing what the lines
/// leave. The placement lists the first half's line's glyphs, then the
/// second's, in text order, then the side ornament's copies. Returns the
/// run the placement refers to when it is not `run` — the rescaled run
/// for `Fit`, with the side ornament as a line of its own — and the
/// placement.
///
/// R27 parts the text and sizes a `Fit` frame ([`place_lines`]); its
/// lines, cut where R27 cut them at `min`, are laid here. A `Once` or
/// `Spread` frame is laid at its own size, its lines parted as R27 parts
/// them ([`line_partition`]) but never cut to R27's level span: a line
/// is cut at its end only where its half less the clearance cannot hold
/// it at its font's own gap (ruling R34), and the placement then
/// reports [`PlacedRun::truncated`]. A `Once` filler is not drawn: the
/// resolver gives it to the frame as its `sides` when `sides` is unset
/// ([`resolve_ring`](crate::document::resolve::resolve_ring)).
///
/// Cost: [`place_lines`]; per line a few [`track`]s to center it
/// ([`falsi`]) and a few per lay ([`solve_tracking`]), each O(its
/// glyphs); the junction clearance [`CLEAR_ROUNDS`] rounds at most, at
/// whole and again below it, each laying again only the lines whose
/// part it moved, with a one-copy fit per side arc; a shrink
/// [`SHRINK_ROUNDS`] probes, each one [`track`] a line and, where they
/// fit, a clearance search; a cut a bisection of [`track`]s more and the
/// clearance again, [`CUT_ROUNDS`] times at most; a cut apart a
/// bisection of clearance searches per line; a side arc a lay of its
/// copies at the floor, and a few lays per copy count weighed
/// ([`falsi`]); every touch check a sweep; a build's shaping and inking.
/// In all, at most the frame's budget of glyph steps ([`frame_budget`]:
/// [`STEPS_PER_GLYPH`] a glyph of its text cut to [`MAX_PLACED_GLYPHS`],
/// at least [`TRACK_BUDGET`]), every kind of work counted at its weight
/// (ruling R46.1) and the budget read before every lay, build and
/// reshape, and [`overshoot_steps`] past it at most (ruling R47.2);
/// where two inks still touch, a bisection of overlap checks over its
/// inks ([`untouched`]) runs within the share of the budget held back
/// for it.
#[allow(clippy::too_many_arguments)] // `place_on_closed`'s, and the trace.
pub(super) fn place_halves(
    run: &ShapedRun,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let span = layout.span.get();
    let sized = matches!(fit, FitMode::Fit { .. });
    // The frame's work, all of it counted (rulings R44.2, R46.1): its
    // budget scaled by its text, cut to the glyph cap, until R27 has
    // parted it, then by the lines it parted.
    let work = Work::new(run.glyphs.len().min(glyph_cap()));
    // R27's placement, laid in full only for a frame that keeps it.
    let keep_r27 = |trace: Option<&mut Trace>, fs: &mut FontSystemWriteGuard, work: &Work| {
        let mut natural_trace = Trace::default();
        let placed = place_lines_in(
            run,
            outline,
            fit,
            outward,
            flow,
            fs,
            Some(&mut natural_trace),
            work,
        );
        if let Some(trace) = trace {
            *trace = natural_trace;
            trace.spent = work.spent.get();
            trace.budget = work.budget.get();
            trace.glyphs = work.glyphs.get();
        }
        placed
    };
    // What a `Fit` frame reads of R27: its size, its parting and its cuts
    // (ruling R43: not where it centers each line, which the halves lay
    // again). A `Once` or `Spread` frame parts its lines itself, below.
    let mut natural_trace = Trace::default();
    let (fitted, natural) = if sized {
        frame_lines(
            run,
            outline,
            fit,
            outward,
            flow,
            fs,
            Some(&mut natural_trace),
            &work,
        )
    } else {
        (
            None,
            PlacedRun {
                glyphs: Vec::new(),
                truncated: false,
            },
        )
    };
    if sized && natural_trace.lines.is_empty() {
        return keep_r27(trace, fs, &work);
    }
    let halves = Halves::new(outline, table_spacing(run, fit, flow));
    // An outline whose halves cannot be measured — one with a side that
    // is not a number — keeps R27's placement, which holds every
    // position finite there. (`place_on_closed` shrinks an outline too
    // large for its length to be an `f32` before it reaches here.)
    let ends = [
        halves.junctions[0],
        halves.junctions[1],
        (halves.apexes[0], halves.apexes[1]),
    ];
    if !(halves.length.is_finite() && ends.iter().all(|(a, b)| a.is_finite() && b.is_finite())) {
        return keep_r27(trace, fs, &work);
    }
    let lay = Layout::new(&halves, outward, work);

    // The run the lines are laid from, and each line: its half, its
    // glyphs, and whether R27 cut it.
    let (base, specs): (ShapedRun, Vec<(usize, Range<usize>, bool)>) = if sized {
        let base = fitted.clone().unwrap_or_else(|| run.clone());
        let specs = natural_trace
            .lines
            .iter()
            .filter_map(|line| {
                let ids = natural.glyphs[line.glyphs.clone()]
                    .iter()
                    .map(|g| g.glyph as usize);
                Some((line.half, ids.clone().min()?..ids.max()? + 1, line.cut))
            })
            .collect();
        (base, specs)
    } else {
        lay.work.walked(run.glyphs.len());
        let specs = line_partition(run, outline, fit, flow)
            .into_iter()
            .enumerate()
            .filter(|(_, range)| !range.is_empty())
            .map(|(k, range)| (k, range, false))
            .collect();
        (run.clone(), specs)
    };

    // The text at `scale` times its size: the run as drawn — the text,
    // and the side ornament on a line of its own — its lines, laid with
    // their clearance grown from `seed`. With `cheap`, `None` when a line
    // does not fit its half less the seed at its font's gap (one `track`
    // a line), before any clearance is grown: what a shrink's probe
    // looks at first (ruling R35.3).
    let build = |scale: f32, seed: Clear, cheap: bool, fs: &mut FontSystemWriteGuard| {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        let scaled = if scale == 1.0 {
            base.clone()
        } else {
            let fsys: &mut FontSystem = fs;
            let scaled = reshape(&base, scale, None, fsys);
            lay.work.shaped(&scaled);
            scaled
        };
        let (drawn, side) = match side_run(&scaled, &layout.sides, flow, fs, &lay.work) {
            Some((with, range)) => (with, Some(range)),
            None => (scaled, None),
        };
        lay.work.inked(&drawn);
        let fsys: &mut FontSystem = fs;
        let drawn = inked(&drawn, fsys);
        // The glyphs the text may still place (ruling R44.1): a frame's
        // text and its sides together place at most `MAX_PLACED_GLYPHS`,
        // so a text longer is cut at a cluster, its end left out.
        let mut left = glyph_cap();
        let prepared: Vec<Prepared> = specs
            .iter()
            .filter_map(|(k, range, cut)| {
                let (clusters, capped) = to_cap(clusters_of(&drawn.glyphs, range.clone()), &mut left);
                let cut = &(*cut || capped);
                // A `Once` or `Spread` line its whole half cannot hold at
                // its font's gap is cut to what that holds before it is
                // prepared or any clearance is grown: a clearance only
                // narrows its room, so the cut would take that much and
                // more in the end (ruling R34), and a line centered and
                // laid whole round after round is work spent on text that
                // is never drawn (ruling R38.3).
                let kept = (!sized)
                    .then(|| letters_of(&clusters))
                    .flatten()
                    .and_then(|(_, gap)| lay.held(&drawn, *k, &clusters, gap, [0.0; 2]));
                match kept {
                    Some(kept) => lay.prepare_clusters(&drawn, *k, kept, true),
                    None => lay.prepare_clusters(&drawn, *k, clusters, *cut),
                }
            })
            .collect();
        let too_long = |p: &Prepared| {
            let (lo, hi) = lay.room(p.half, seed[p.half]);
            p.clusters.iter().filter(|c| !c.blank).count() >= 2
                && !lay
                    .reach(p.half, &drawn, &p.clusters, lo, p.natural_gap)
                    .le(&(hi - lo + resolved(SLIDE_TOLERANCE, hi)))
        };
        if cheap && prepared.iter().any(too_long) {
            return None;
        }
        let backtracks = if cheap { PROBE_BACKTRACKS } else { BACKTRACKS };
        let cleared = lay.lay_clear(
            &drawn,
            &prepared,
            side.as_ref().map(|r| (&drawn, r)),
            span,
            seed,
            backtracks,
        );
        let overlap = overlapping(&cleared.lines, &lay.work);
        Some(Built {
            drawn,
            side,
            prepared,
            lines: cleared.lines,
            clear: cleared.clear,
            met: cleared.met,
            overlap,
        })
    };
    // The frame's budget scales with its text, cut to the glyph cap
    // (rulings R44.1, R44.2).
    let glyphs = specs
        .iter()
        .map(|(_, range, _)| range.len())
        .sum::<usize>()
        .min(glyph_cap());
    lay.work.scale(glyphs);
    let zero: Clear = [[0.0; 2]; 2];
    let Some(mut built) = build(1.0, zero, false, fs) else {
        unreachable!("only a cheap build is refused")
    };
    let mut scale = 1.0_f32;
    // A `Fit` frame that its halves do not hold at R27's size — a line
    // its half less the clearance cannot hold at its font's gap, a
    // junction left short of a break, two inks touching: a flat outline,
    // its half barely longer than the level span R27 sized the text for —
    // is shrunk, as the last resort, to the largest size within its
    // bounds at which they hold (§6.2 step 6, rulings R34 and R35.2).
    // Each probe looks first at whether its lines fit their halves less
    // a clearance seeded from the last size held, and grows that
    // clearance only if they do (ruling R35.3).
    if let (false, false, FitMode::Fit { min, .. }) = (built.holds(), lay.spent_all(), fit) {
        let fitted_size = base.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let declared = run.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let floor = if fitted_size > 0.0 && declared > 0.0 {
            (min * declared / fitted_size).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let Some(at_floor) = build(floor, zero, false, fs) else {
            unreachable!("only a cheap build is refused")
        };
        if at_floor.holds() {
            let (mut held, mut over) = (floor, 1.0_f32);
            let mut best = at_floor;
            for _ in 0..SHRINK_ROUNDS {
                // Sub-percent precision in a last resort buys nothing a
                // reader can see (ruling R42.2): the bracket within 1% of
                // the scale held, or a size step under a quarter point,
                // ends the shrink.
                let step = over - held;
                let precise = step <= SHRINK_PRECISION * held || step * fitted_size < SHRINK_LEAST_STEP_PT;
                if lay.spent_all() || precise {
                    break;
                }
                let mid = 0.5 * (held + over);
                match build(mid, best.seed(), true, fs) {
                    Some(probe) if probe.holds() => {
                        (held, best) = (mid, probe);
                    }
                    _ => over = mid,
                }
            }
            (built, scale) = (best, held);
        } else {
            (built, scale) = (at_floor, floor);
        }
    }
    let Built {
        drawn,
        side,
        mut prepared,
        mut lines,
        mut clear,
        mut met,
        mut overlap,
    } = built;
    let side_of = side.as_ref().map(|range| (&drawn, range));

    // A frame its halves hold even so by nothing is cut (rulings R34,
    // R35.2): a line its half less the clearance cannot hold at its
    // font's gap — a `Once` or `Spread` line at its size, a `Fit` line at
    // its least — to what it can. A line its half holds is never cut
    // (ruling R34): a junction short of a break, or two inks touching,
    // where no line is pressed, is reported in the trace instead.
    let mut truncated = (natural.truncated && sized) || prepared.iter().any(|p| p.cut);
    for _ in 0..CUT_ROUNDS {
        let holds = !lines.iter().any(|l| l.pressed) && met.iter().all(|&m| m) && !overlap;
        if holds || lay.spent_all() {
            break;
        }
        let mut changed = false;
        for (p, line) in prepared.iter_mut().zip(&lines) {
            if !line.pressed {
                continue;
            }
            if let Some(shorter) = lay.cut(&drawn, p, clear[p.half]) {
                *p = shorter;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        truncated = true;
        // The clearance grown again from a share of the last: a cut
        // line, stretched wider, wants as much or more (ruling R38.3).
        let seed = clear.map(|half| half.map(|c| c * SEED_SHARE));
        let cleared = lay.lay_clear(&drawn, &prepared, side_of, span, seed, BACKTRACKS);
        (lines, clear, met) = (cleared.lines, cleared.clear, cleared.met);
        overlap = overlapping(&lines, &lay.work);
    }
    // Two inks that still touch, every line its half holds: an outline
    // too thin to hold the lines apart — inward round a flat outline,
    // the two lines facing across it. The line that cannot be held
    // beside the others is cut, the last first, to the longest prefix
    // that touches nothing, or left out (ruling R38.1); a `Once` frame
    // is not shrunk.
    let mut cut_apart = Vec::new();
    for i in (0..prepared.len()).rev() {
        if !overlap || lay.spent_all() {
            break;
        }
        if let Some((kept, cleared)) = lay.cut_apart(&drawn, &prepared, i, side_of, span, clear) {
            cut_apart.push(prepared[i].half);
            match kept {
                Some(p) => prepared[i] = p,
                None => {
                    prepared.remove(i);
                }
            }
            (lines, met) = (cleared.lines, cleared.met);
            overlap = false;
            truncated = true;
        }
    }
    // A frame whose budget was spent while two of its inks still touch is
    // cut to the inks that touch nothing (rulings R38.1, R44.2): never
    // overlap wins over never cut, and a spent budget is no exception.
    if overlap {
        let (kept, halves_cut) = untouched(&lines, &lay.work);
        lines = kept;
        prepared.truncate(lines.len());
        cut_apart.extend(halves_cut);
        met = [0, 1].map(|j| lay.met(&lines, side_of, j).0);
        overlap = overlapping(&lines, &lay.work);
        truncated = true;
    }
    let mut unheld = unheld_of(&lines, met, overlap);

    let mut text = Vec::new();
    let mut line_ranges = Vec::new();
    for line in &lines {
        line_ranges.push(text.len()..text.len() + line.tracked.glyphs.len());
        text.extend(line.tracked.glyphs.iter().copied());
    }

    // The side ornament round the junctions.
    let mut sides = Vec::new();
    if let Some(range) = &side {
        let side = SideOrnament::new(&lay, &drawn, range.clone(), mean_size(&lines));
        let target = lines.iter().map(|l| l.gap).sum::<f32>() / lines.len().max(1) as f32;
        let text_boxes: Vec<InkBox> = lines
            .iter()
            .flat_map(|l| l.tracked.boxes.iter().copied())
            .collect();
        for (at, from, to) in side_arcs(&lines, halves.length) {
            let budget = glyph_cap().saturating_sub(text.len() + sides.len());
            if let Some(arc) = side.arc(from, to) {
                let before = sides.len();
                // An arc the frame's budget is spent before is left
                // empty, and reported (rulings R44.2, R47.2).
                if !arc.fill(target, budget, &mut sides) {
                    unheld.sides.extend((0..2).filter(|&j| at[j]));
                    continue;
                }
                // An arc whose copies would touch the text, the copies
                // already laid, or one another but for neighbors — round
                // an outline thinner than its lines, inward, where an arc
                // folds back across itself — is left empty, and reported
                // (ruling R35.2).
                let boxes: Vec<InkBox> = sides[before..]
                    .iter()
                    .filter_map(|g| glyph_box(&drawn, g))
                    .collect();
                let laid: Vec<InkBox> = sides[..before]
                    .iter()
                    .filter_map(|g| glyph_box(&drawn, g))
                    .collect();
                // Keyed by which they are — the text's, the copies already
                // laid, this arc's — and where among them.
                let keyed = text_boxes
                    .iter()
                    .map(|b| (0, *b))
                    .chain(laid.iter().map(|b| (1, *b)))
                    .enumerate()
                    .map(|(i, (from, b))| ((from, i), b))
                    .chain(boxes.iter().enumerate().map(|(i, b)| ((2, i), *b)))
                    .collect();
                let touches = touching(
                    keyed,
                    |(a, i), (b, j)| {
                        if a == 2 && b == 2 {
                            i.abs_diff(j) <= 1
                        } else {
                            a != 2 && b != 2
                        }
                    },
                    &lay.work,
                );
                if touches {
                    sides.truncate(before);
                    unheld.sides.extend((0..2).filter(|&j| at[j]));
                }
            }
        }
    }

    if let Some(trace) = trace {
        *trace = Trace::default();
        trace.lines = lines
            .iter()
            .zip(&prepared)
            .zip(&line_ranges)
            .map(|((line, p), glyphs)| TracedLine {
                half: line.half,
                glyphs: glyphs.clone(),
                from: line.part.0,
                to: line.part.1,
                cut: p.cut,
            })
            .collect();
        trace.unheld = unheld;
        trace.cut_apart = cut_apart;
        trace.clear = clear;
        trace.spent = lay.work.spent.get();
        trace.budget = lay.work.budget.get();
        trace.glyphs = lay.work.glyphs.get();
    }
    let changed = (sized && fitted.is_some()) || side.is_some() || scale != 1.0;
    let mut glyphs = text;
    glyphs.extend(sides);
    let placed = PlacedRun { glyphs, truncated };
    (changed.then(|| uninked(drawn)), placed)
}

/// Test seam (#294, ruling R46.1): whether a frame's work, its budget
/// scaled by `glyphs` glyphs and then set to `budget`, reads as spent once
/// it has taken `steps` glyph steps.
#[cfg(test)]
pub(crate) fn spent_seam(glyphs: usize, budget: usize, steps: usize) -> bool {
    let work = Work::new(glyphs);
    work.budget.set(budget);
    work.spent.set(0);
    work.take(steps);
    work.spent_all()
}

/// Test seam (#291, ruling R32): where along `outline`'s first half the
/// last ink of `run` (one line, every glyph) ends, laid over the whole
/// half with each of `gaps` — how [`track`]'s length moves with its
/// gap, a zero gap included.
pub(crate) fn track_seam(run: &ShapedRun, outline: &Outline, gaps: &[f32], fs: &mut FontSystem) -> Vec<f32> {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let run = inked(run, fs);
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let walk = line_walk(&halves.paths[0], &run, true);
    let (sa, sb) = halves.junctions[0];
    gaps.iter()
        .map(|&g| {
            let tracked = track(&walk, &run.glyphs, &clusters, (sa, sb), g);
            tracked.ends.map_or(sa, |(_, last)| halves.along(0, middle(last)))
        })
        .collect()
}

/// Test seam (#294, ruling R43): the geometry of a line's ends on
/// `outline`'s half `half` — the outward normal the 135° test weighs for
/// an end ink `ink` set at `set` along the half ([`Layout::normals`]),
/// and the way the end of a part ending at `end` draws back
/// ([`retreats`]).
pub(crate) fn end_seam(
    outline: &Outline,
    half: usize,
    ink: (Vec2, Vec2),
    set: f32,
    end: f32,
) -> (Vec2, Vec2) {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0));
    let tracked = Tracked {
        glyphs: Vec::new(),
        short: 0.0,
        ends: Some((ink, ink)),
        centers: Some((set, set)),
        boxes: vec![ink],
        marks: vec![0],
    };
    let normal = lay.normals(half, &tracked, (set, set))[0];
    (normal, retreats(&halves.paths[half], (set, end))[1])
}

/// Test seam (#291, ruling R33.1): `n` copies of `ornament` laid as a
/// junction lays them at their floor gap ([`SideOrnament::arc_floor`]),
/// along `outline`'s first half from a point at its top-left junction,
/// beside `run`'s text: the run they are shaped into, the copies, and the
/// floor. `None` when the ornament shapes to nothing or has no ink.
pub(crate) fn side_floor_seam(
    run: &ShapedRun,
    outline: &Outline,
    ornament: &str,
    n: usize,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> Option<(ShapedRun, Vec<PlacedGlyph>, f32)> {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0));
    let sides = Some((ornament.to_owned(), Rgba::opaque(0, 0, 0)));
    let (with, range) = side_run(run, &sides, flow, fs, &lay.work)?;
    let fsys: &mut FontSystem = fs;
    let drawn = inked(&with, fsys);
    let (size, _) = letters_of(&clusters_of(&drawn.glyphs, 0..range.start))?;
    let side = SideOrnament::new(&lay, &drawn, range, size);
    let (ja, jb) = halves.junctions[0];
    let point = |s: f32| {
        let p = halves.paths[0].at(s).0;
        ((p, p), s)
    };
    let arc = side.arc(point(ja), point(jb))?;
    let glyphs = arc.lay(n, arc.floor).0.out;
    let floor = arc.floor;
    Some((uninked(drawn), glyphs, floor))
}

/// Test seam (#294, ruling R44.3): `n` copies of `ornament` solved for
/// their equal gap ([`SideArc::solve`]) along the arc
/// [`side_floor_seam`] lays them on, between `outline`'s first half's
/// junctions, from the floor's row: the gap, and whether the copies laid
/// at it touch the junction's ink or pass its center. `None` when `n`
/// copies do not fit at the floor.
#[cfg(test)]
pub(crate) fn side_solve_seam(
    run: &ShapedRun,
    outline: &Outline,
    ornament: &str,
    n: usize,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> Option<(f32, bool)> {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0));
    let sides = Some((ornament.to_owned(), Rgba::opaque(0, 0, 0)));
    let (with, range) = side_run(run, &sides, flow, fs, &lay.work)?;
    let fsys: &mut FontSystem = fs;
    let drawn = inked(&with, fsys);
    let (size, _) = letters_of(&clusters_of(&drawn.glyphs, 0..range.start))?;
    let side = SideOrnament::new(&lay, &drawn, range, size);
    let (ja, jb) = halves.junctions[0];
    let point = |s: f32| {
        let p = halves.paths[0].at(s).0;
        ((p, p), s)
    };
    let arc = side.arc(point(ja), point(jb))?;
    let (row, rooms) = arc.floor_row(n)?;
    let &(room, len) = rooms.get(n.checked_sub(1)?)?;
    let (gap, _) = arc.solve(n, room, row[..len].to_vec());
    let (_, (end, at), _) = arc.lay(n, gap);
    Some((gap, at >= arc.cb || end.is_nan() || end <= TOUCH))
}

/// Test seam (#291, ruling R34): the part of `outline`'s half `half` a
/// line may cover with `clear` kept at its junctions, and the half's
/// junctions — [`Layout::room`], which never inverts.
pub(crate) fn room_seam(outline: &Outline, half: usize, clear: [f32; 2]) -> ((f32, f32), (f32, f32)) {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0));
    (lay.room(half, clear), halves.junctions[half])
}

/// Test seam (#291, ruling R33): `outline`'s two halves, each as its
/// junctions and apex along its path — from the middle of the left
/// side, clockwise for the first, counterclockwise for the second — and
/// the outline's length.
pub(crate) fn halves_seam(outline: &Outline) -> ([(f32, f32); 2], [f32; 2], f32) {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    (halves.junctions, halves.apexes, halves.length)
}
