// SPDX-License-Identifier: MPL-2.0

//! Whole glyphs between two ends (#296, rule 5): a pattern along an open
//! path — a connector's or a line link's `repeat` ornament — laid as a
//! whole number of glyphs, its slack spread evenly over every gap,
//! the two at its ends included.
//!
//! A `repeat` walk ([`place_on_open`](crate::flow::place_on_open)) lays
//! its pattern from the start and cuts the last copy where the path
//! ends, so the gap at the far end is whatever the cut leaves, and at the
//! near end whatever the path's start leaves. Between two frames that
//! reads as a connector stopping short of one and running into the
//! other. Here each glyph's ink is a box of its own, and the gaps are
//! measured between boxes:
//!
//! - **The gaps are the pattern's own, ink to ink**, whichever way the
//!   path runs: two consecutive inks are as far apart, box to box, as the
//!   font sets them on a line — the first's right bearing, every space's
//!   advance between them, the second's left bearing — plus `spacing` for
//!   each step. Each ink is placed where its box lies that far from the
//!   one before it, so a wide glyph such as `~` steps further along a level
//!   stretch than down a steep one, and the gaps read even round a bend.
//! - **The count is whole.** As many inks as bring every gap nearest its
//!   natural one, and the difference spread over them all: each gap, the
//!   ends' too, grows or shrinks by the same amount, the slack. It is
//!   solved for, so the inks fill the path from end gap to end gap.
//! - **The ends are measured.** The gap from the first ink to the boxes
//!   at the start ([`PathEnd::inks`], a frame's inks or a node's box) is
//!   the end gap, the mean of the pattern's natural gaps plus the slack:
//!   the path starts where its end sits, and where the boxes lie further
//!   back or further on than the path's start, the glyph moves back or on
//!   to keep the gap — back at most [`PathEnd::back`], along the path's
//!   first direction. Likewise at the far end. An end held out
//!   ([`PathEnd::hold`]) keeps its first ink that far out instead.
//!
//! Nothing is reversed: an ornament has no reading direction (ruling
//! R29).

use std::cell::Cell;

use glam::Vec2;

use crate::flow::curve::Curve;
use crate::flow::outline::Rect;
use crate::flow::path::ArcPath;
use crate::flow::place::halves::{resolved, BOXES_A_STEP};
use crate::flow::place::ring::{ring_glyphs, steps_of, RingLoss, RingWork, GLYPHS_A_STEP};
use crate::flow::place::{
    cluster_ink, clusters_of, Cluster, PlacedGlyph, PlacedRun, Trace, MAX_PLACED_GLYPHS, MIN_TABLE_SPACING,
};
use crate::flow::shape::{Ink, ShapedRun};

/// How many times an ink's place is refined from its first guess, along
/// the secant of its gap. Fixed, so that a place moves smoothly with the
/// gap asked of it, and the whole run's reach with the slack: what the
/// slack is solved by. On a straight stretch the first guess is exact.
pub(crate) const REFINE: usize = 4;

/// The most rounds the slack is solved in. Each round moves it by what
/// the run fell short by over its gaps, which is exact on a straight path,
/// so a round or two settles it. A round that leaves the slack where it
/// was stops it, as does a bracket closed to adjacent floats; and on a
/// path whose float steps are wider than the pattern's gaps, a round the
/// lay does not answer ([`STALLED`]), the third at the latest (#333).
pub(crate) const SLACK_ROUNDS: usize = 16;

/// How near its end gap, in points, the slack lays the last ink, at the
/// least: what a round is solved to, but where the path's positions are
/// wider apart than this ([`resolved`]).
const SLACK_TOLERANCE: f32 = 1e-3;

/// The least share of what a round's move of the slack asks of the lay —
/// that move over each of its gaps — that the lay must move by for the
/// solve to go on: below it, the lay does not answer the slack (#333). On
/// a straight path it moves by all of it.
const STALLED: f32 = 1e-3;

/// The most strides a bracketed solve ([`Track::bracketed`]) takes out
/// from its first guess, each twice the last, to bracket the place it
/// seeks: from a point, past 10⁹ points.
pub(crate) const BRACKET_STRIDES: usize = 32;

/// The rounds a bracketed solve bisects its bracket in.
pub(crate) const BRACKET_ROUNDS: usize = 32;

/// The place at which `gap`, growing with the place, is `want`: bracketed
/// from `guess` in strides doubling from a point ([`BRACKET_STRIDES`]),
/// then bisected [`BRACKET_ROUNDS`] times. Where no stride brackets it,
/// the end of the strides that came nearest.
///
/// Cost: at most `2 + BRACKET_STRIDES + BRACKET_ROUNDS` calls of `gap`.
fn bisect(gap: &dyn Fn(f32) -> f32, want: f32, guess: f32) -> f32 {
    let below = |t: f32| gap(t) < want;
    let (mut lo, mut hi) = (guess, guess);
    let mut stride = 1.0_f32;
    if below(guess) {
        for _ in 0..BRACKET_STRIDES {
            hi = guess + stride;
            if !below(hi) {
                break;
            }
            lo = hi;
            stride *= 2.0;
        }
    } else {
        for _ in 0..BRACKET_STRIDES {
            lo = guess - stride;
            if below(lo) {
                break;
            }
            hi = lo;
            stride *= 2.0;
        }
    }
    for _ in 0..BRACKET_ROUNDS {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            break;
        }
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// What the last [`place_pitched`] on this thread solved its slack to,
/// for the count of inks it laid. Test-only: what holds the solve's stops
/// (#333).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Solved {
    /// The count of inks laid.
    pub count: usize,
    /// The slack they were laid at.
    pub slack: f32,
    /// How far the last ink fell short there of where its end gap puts
    /// it.
    pub missed: f32,
    /// How many of the count's rounds moved the slack and the lay did not
    /// answer ([`STALLED`]), on any path: the solve stops at the first of
    /// them only on a path whose float steps are wider than the pattern's
    /// gaps.
    pub unanswered: usize,
}

#[cfg(test)]
thread_local! {
    /// The last [`place_pitched`]'s [`Solved`] on this thread.
    pub(crate) static SOLVED: std::cell::Cell<Option<Solved>> = const { std::cell::Cell::new(None) };
}

/// What a [`place_pitched`] placement keeps its glyphs clear of at one
/// end of its path.
#[derive(Clone, Copy, Debug)]
pub struct PathEnd<'a> {
    /// The boxes the end's glyph is held a gap from, on the canvas: a
    /// frame's inks, or the node's box. Empty, the end is not measured:
    /// its gap is laid from the path's end.
    pub inks: &'a [Rect],
    /// How far off the path, along its direction at the end — back from
    /// its start, on past its end — the end glyph's ink may reach to keep
    /// its gap: into a frame whose inks lie back from where the path ends,
    /// never into the node. Read as `0` when not positive.
    pub back: f32,
    /// How far from the path's end the end glyph's ink is held at the
    /// least, its center along the path, whatever its gap: where the paths
    /// that leave one point clear each other (#296). Read as `0` when not
    /// positive.
    pub hold: f32,
}

/// One inked cluster of the pattern: which cluster, its ink in its box's
/// coordinates (`x` from the box's left edge, `y` from the baseline), and
/// the natural gap from its ink to the next inked cluster's.
#[derive(Clone, Debug)]
pub(super) struct Mark {
    pub(super) cluster: usize,
    pub(super) ink: Ink,
    pub(super) gap: f32,
}

impl Mark {
    /// Its ink's size.
    pub(super) fn size(&self) -> Vec2 {
        self.ink.max - self.ink.min
    }
}

/// The gap between two canvas boxes, signed: how far apart they are
/// where they are apart, and less than 0 by the least they overlap by
/// where they overlap. Continuous, and growing as they move apart.
pub(super) fn signed_gap(a: &Rect, b: &Rect) -> f32 {
    let d = (b.origin - a.max()).max(a.origin - b.max());
    if d.x > 0.0 || d.y > 0.0 {
        d.max(Vec2::ZERO).length()
    } else {
        d.max_element()
    }
}

/// The pattern's inked clusters, in order, each with the natural gap
/// from its ink to the next one's round the pattern as the font sets
/// them on a line: its right bearing, the advances of the clusters
/// before the next ink, that ink's left bearing, and `spacing` for each
/// step between them. Below 0 where the font sets two inks overlapping,
/// as a box-drawing face's overhang does; `0` where a gap is not finite.
/// Empty for a run with no ink.
pub(super) fn natural_marks(run: &ShapedRun, clusters: &[Cluster], spacing: f32) -> Vec<Mark> {
    let spacing = if spacing.is_finite() {
        spacing.max(0.0)
    } else {
        0.0
    };
    let mut marks: Vec<Mark> = clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.blank)
        .filter_map(|(k, c)| {
            cluster_ink(&run.glyphs, c, |_| 1.0).map(|ink| Mark {
                cluster: k,
                ink,
                gap: 0.0,
            })
        })
        .collect();
    let count = marks.len();
    for j in 0..count {
        let (here, next) = (&marks[j], &marks[(j + 1) % count]);
        let steps = (next.cluster + clusters.len() - here.cluster - 1) % clusters.len() + 1;
        let between: f32 = (1..steps)
            .map(|d| clusters[(here.cluster + d) % clusters.len()].advance)
            .sum();
        let gap = clusters[here.cluster].advance - here.ink.max.x
            + between
            + next.ink.min.x
            + spacing * steps as f32;
        marks[j].gap = if gap.is_finite() { gap } else { 0.0 };
    }
    marks
}

/// [`natural_marks`], every gap at least 0: what a connector's pattern
/// is laid by.
fn marks_of(run: &ShapedRun, clusters: &[Cluster], spacing: f32) -> Vec<Mark> {
    let mut marks = natural_marks(run, clusters, spacing);
    for mark in &mut marks {
        mark.gap = mark.gap.max(0.0);
    }
    marks
}

/// A pattern's own gap and size, as [`place_pitched`] lays it: the mean
/// of its natural gaps between inks, and the largest extent, across or
/// along, of any of its inks. `None` for a run with no ink, which
/// [`place_pitched`] does not lay. What the paths leaving one side of a
/// node are fanned by (#296).
///
/// Cost: O(clusters).
pub fn pattern_gap(run: &ShapedRun, spacing: f32) -> Option<(f32, f32)> {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let marks = marks_of(run, &clusters, spacing);
    if marks.is_empty() {
        return None;
    }
    let gap = marks.iter().map(|m| m.gap).sum::<f32>() / marks.len() as f32;
    let size = marks.iter().map(|m| m.size().max_element()).fold(0.0, f32::max);
    Some((gap, size))
}

/// What a pitched lay ([`pitch`]) sets its inks along: a path from `0`
/// to its length, run on past either end, and where an ink set at a
/// point of it is centered.
pub(super) trait Track {
    /// Its length.
    fn length(&self) -> f32;
    /// The point `s` along it and the way it runs there; past either end,
    /// as the track runs on.
    fn at(&self, s: f32) -> (Vec2, Vec2);
    /// Where an ink of `size` set at `s` is centered.
    fn center(&self, s: f32, size: Vec2) -> Vec2;
    /// The way it runs at its start, and at its end.
    fn ends(&self) -> (Vec2, Vec2);
    /// How large its positions are, what a tolerance on them is resolved
    /// against ([`resolved`]).
    fn magnitude(&self) -> f32;
    /// The gap between two inks' boxes `a` and `b`, `b` the further on,
    /// as the lay measures and keeps it — between two of its inks, and
    /// to what lies at either end: signed, less than 0 by how much they
    /// overlap, and growing as `b` moves on. Box to box ([`signed_gap`])
    /// unless the track says otherwise.
    fn gap(&self, a: &Rect, b: &Rect) -> f32 {
        signed_gap(a, b)
    }
    /// Whether the lay holds no gap looser than its natural one: of the
    /// two counts nearest, the one whose slack is not above 0, its inks
    /// pressed together rather than parted. Otherwise the one whose slack
    /// is nearest 0.
    fn tight(&self) -> bool {
        false
    }
    /// Whether an ink's place is found by bisection, its gap bracketed
    /// round its first guess ([`BRACKET_STRIDES`], [`BRACKET_ROUNDS`]),
    /// rather than refined [`REFINE`] times along the secant: for a track
    /// whose gaps grow with the place all along it, where the secant,
    /// round a bend that slows the gap's growth, can step back past where
    /// it began.
    fn bracketed(&self) -> bool {
        false
    }
    /// How far from the center of an ink of `size` its box's edge lies
    /// along `dir`, the way the track runs at an end: how far the ink
    /// reaches past where it is set, which an end with nothing beside it
    /// keeps short of the end. Box to box, where `dir` leaves the box.
    fn reach(&self, size: Vec2, dir: Vec2) -> f32 {
        let half = size * 0.5;
        let along = |h: f32, d: f32| {
            if d.abs() > 1e-6 {
                h / d.abs()
            } else {
                f32::INFINITY
            }
        };
        along(half.x, dir.x)
            .min(along(half.y, dir.y))
            .min(half.max_element())
    }
}

/// An open path: run on straight past either end, along its first and
/// last directions; an ink centered on it.
struct Open {
    arc: ArcPath,
    length: f32,
    start_dir: Vec2,
    end_dir: Vec2,
    /// The gaps between two boxes measured along it ([`Track::gap`]),
    /// each a unit of a placement's work past its path's reads (#313).
    gaps: Cell<usize>,
}

impl Track for Open {
    fn length(&self) -> f32 {
        self.length
    }

    fn at(&self, s: f32) -> (Vec2, Vec2) {
        if s < 0.0 {
            (self.arc.at(0.0).0 + self.start_dir * s, self.start_dir)
        } else if s > self.length {
            (
                self.arc.at(self.length).0 + self.end_dir * (s - self.length),
                self.end_dir,
            )
        } else {
            self.arc.at(s)
        }
    }

    fn center(&self, s: f32, _size: Vec2) -> Vec2 {
        self.at(s).0
    }

    fn ends(&self) -> (Vec2, Vec2) {
        (self.start_dir, self.end_dir)
    }

    fn magnitude(&self) -> f32 {
        self.length
    }

    fn gap(&self, a: &Rect, b: &Rect) -> f32 {
        self.gaps.set(self.gaps.get().saturating_add(1));
        signed_gap(a, b)
    }
}

/// What a [`pitch`] laid: the places its inks are set at along its
/// track, in order, ink `k` the pattern's mark `k` round it; and whether
/// it was held to its cap.
pub(super) struct Pitched {
    pub(super) centers: Vec<f32>,
    pub(super) truncated: bool,
}

/// Place `run` along the open `path` as whole glyphs, evenly gapped from
/// end to end (see the module docs): the pattern's inked clusters
/// repeated in order, each ink placed where its box lies its natural gap
/// plus the slack from the one before it, box to box; as many as bring
/// the slack nearest 0, and the slack solved for so that both end gaps,
/// measured to `ends`, are the mean natural gap plus the slack too.
/// `spacing` is added to every natural gap, as a `repeat` walk adds it;
/// one that is not finite or not positive adds nothing.
///
/// A run with no ink — text, or only spaces — has no pitch, and is
/// placed empty: [`place_on_open`](crate::flow::place_on_open) lays it.
/// So is a path that is not finite, and one too short to hold one ink
/// with no gap at all. Never more than [`MAX_PLACED_GLYPHS`] glyphs; a
/// placement held to it is `truncated`, its gaps spread over the inks it
/// holds.
///
/// Cost: O(clusters) to read the pattern; O(inks) path lookups to count
/// the inks, and as many again for each of a few rounds of the slack for
/// each of the two counts nearest, each ink a few gap measures where the path bends; and
/// O(`ends`' boxes) per end measure. Allocates the placement and one
/// scratch vector of positions.
pub fn place_pitched(
    run: &ShapedRun,
    path: &(impl AsRef<[Curve]> + ?Sized),
    spacing: f32,
    ends: [PathEnd<'_>; 2],
) -> PlacedRun {
    place_pitched_capped(run, path, spacing, ends, MAX_PLACED_GLYPHS, None)
}

/// The most path reads a pitched lay along an open path ([`pitch`]'s
/// `next_at`) takes an ink (#313): its box, the way the path runs there
/// and at its first guess, and its gap's first measure and [`REFINE`]
/// refinements, a box each.
const INK_READS: usize = 4 + REFINE;

/// The most gaps between two boxes ([`Track::gap`]) a pitched lay
/// measures an ink: its gap's first measure and its refinements.
const INK_GAPS: usize = 1 + REFINE;

/// The most path reads, and the gaps to each of an end's boxes, an end
/// ink's place takes ([`pitch`]'s `first_at` and `last_at`): its gap's
/// first measure and its refinements, a box each.
const END_MEASURES: usize = 1 + REFINE;

/// The lays of a pitched placement held back for whenever its count or
/// its slack asks whether it may go on (#313), as a directional ring's
/// runs are: the two counts its slack is solved for each begin with a lay
/// the budget is not asked about, a round of the slack is one more, and
/// the lay at the slack found one more.
const LAYS_HELD: usize = 4;

/// The glyph steps `gaps` gaps between two boxes count (#313): as many as
/// a touch check's sweep weighs, [`BOXES_A_STEP`] to a step.
fn gap_steps(gaps: usize) -> usize {
    gaps.div_ceil(BOXES_A_STEP)
}

/// The glyph steps a pitched lay of `inks` inks along an open path takes
/// at most (#313), its ends each beside at most `boxes` boxes: its path's
/// reads ([`INK_READS`] an ink, and [`END_MEASURES`] an end) and its gaps
/// measured ([`INK_GAPS`] an ink, and [`END_MEASURES`] an end's boxes).
fn lay_steps(inks: usize, boxes: usize) -> usize {
    let reads = inks.saturating_mul(INK_READS).saturating_add(2 * END_MEASURES);
    let gaps = inks
        .saturating_mul(INK_GAPS)
        .saturating_add((2 * END_MEASURES).saturating_mul(boxes));
    steps_of(reads).saturating_add(gap_steps(gaps))
}

/// The glyph steps a pitched placement of a pattern of `marks` inks takes
/// before it first asks its budget (#313): where each ink would sit as
/// the last, and where the first sits, each an end's place
/// ([`END_MEASURES`] reads and gaps to each of `boxes` boxes).
fn setup_steps(marks: usize, boxes: usize) -> usize {
    let places = marks.saturating_add(1).saturating_mul(END_MEASURES);
    steps_of(places).saturating_add(gap_steps(places.saturating_mul(boxes.max(1))))
}

/// [`place_pitched`], laying at most `cap` glyphs, filling `trace` with
/// its work (#313).
///
/// **Its budget.** A pitched placement is budgeted as a `repeat` ring is
/// ([`RingWork`]; rulings B17.1a, B17.2a): by the glyphs it will lay, the
/// path's length over one round of the pattern's pitch — each ink's least
/// extent and its gap, so that a path that runs steep is not estimated
/// short — rounded up, times a round's glyphs, at most `cap`. Its work is
/// its path's reads ([`READS_A_STEP`](super::ring::READS_A_STEP) to a
/// step), the glyphs it places, and the gaps between boxes it measures
/// ([`gap_steps`]). Before it lays anything it asks whether it can afford
/// where its inks would end ([`setup_steps`]) and [`LAYS_HELD`] lays of
/// one ink; and [`pitch`] asks before each ink it counts and each round
/// of its slack whether it can afford [`LAYS_HELD`] lays of one ink more
/// ([`lay_steps`]), and the glyphs of the inks it would then hold, which
/// are counted as they are placed. A placement its budget stops is laid
/// at the count and the slack found so far, or, stopped before its first
/// ink, laid empty, and it is short ([`RingLoss::Short`]) — as is one its
/// cap holds. So it takes no more than its budget.
pub(crate) fn place_pitched_capped(
    run: &ShapedRun,
    path: &(impl AsRef<[Curve]> + ?Sized),
    spacing: f32,
    ends: [PathEnd<'_>; 2],
    cap: usize,
    trace: Option<&mut Trace>,
) -> PlacedRun {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let marks = marks_of(run, &clusters, spacing);
    if marks.is_empty() {
        return PlacedRun::default();
    }
    let arc = ArcPath::open(path, (0.25 * run.line_height).max(MIN_TABLE_SPACING));
    let length = arc.length();
    if !length.is_finite() || length <= 0.0 {
        return PlacedRun::default();
    }
    let curves = path.as_ref();
    let direction = |curve: Option<&Curve>, t: f32| {
        curve
            .map_or(Vec2::X, |c| c.derivative(t))
            .try_normalize()
            .unwrap_or(Vec2::X)
    };
    // Its budget, scaled by the glyphs a round of its pattern's pitch along
    // the path holds (#313).
    let cap = cap.min(MAX_PLACED_GLYPHS);
    let round: usize = marks.iter().map(|m| clusters[m.cluster].glyphs.len()).sum();
    let pitch_of: f64 = marks
        .iter()
        .map(|m| f64::from(m.size().min_element()) + f64::from(m.gap))
        .sum();
    let estimate = ring_glyphs(round, pitch_of, length, cap);
    let work = RingWork::new(&arc, round, false, run.line_height, estimate, cap);
    let (budget, glyphs, _) = work.budget();
    let track = Open {
        arc,
        length,
        start_dir: direction(curves.first(), 0.0),
        end_dir: direction(curves.last(), 1.0),
        gaps: Cell::new(0),
    };
    let boxes = ends[0].inks.len().max(ends[1].inks.len());
    // The gaps measured so far, counted as they are asked about.
    let counted = Cell::new(0usize);
    let spent = || {
        let gaps = track.gaps.get();
        work.took(gap_steps(gaps) - gap_steps(counted.get()));
        counted.set(gaps);
        work.spent(&track.arc)
    };
    let stopped = Cell::new(false);
    let affords = |held: usize| {
        let ok = spent().saturating_add(held) < budget;
        if !ok {
            stopped.set(true);
        }
        ok
    };
    let end_gap = marks.iter().map(|m| m.gap).sum::<f32>() / marks.len() as f32;
    // The least slack: no gap below 0.
    let floor = -marks.iter().map(|m| m.gap).fold(end_gap, f32::min);
    // The most inks a placement holds: whole clusters, within the cap.
    let widest = marks
        .iter()
        .map(|m| clusters[m.cluster].glyphs.len())
        .max()
        .unwrap_or(1);
    let inks_cap = cap / widest.max(1);
    // The lays held back for with `inks` inks in hand, and the glyphs of
    // one ink more placed once the lay is done, counted then.
    let held = |inks: usize| {
        let inks = inks.saturating_add(1);
        LAYS_HELD
            .saturating_mul(lay_steps(inks, boxes))
            .saturating_add(inks.saturating_mul(widest).div_ceil(GLYPHS_A_STEP))
    };
    let go = |inks: usize| affords(held(inks));
    let pitched = if affords(setup_steps(marks.len(), boxes).saturating_add(held(0))) {
        pitch(&track, &marks, inks_cap, floor, ends, &go)
    } else {
        None
    };
    let placed = pitched.map_or_else(PlacedRun::default, |pitched| {
        let centers: Vec<(usize, Vec2)> = pitched
            .centers
            .iter()
            .enumerate()
            .map(|(k, &s)| {
                let k = k % marks.len();
                (k, track.center(s, marks[k].size()))
            })
            .collect();
        PlacedRun {
            glyphs: glyphs_at(run, &clusters, &marks, &centers),
            truncated: pitched.truncated,
        }
    });
    work.set(placed.glyphs.len());
    if stopped.get() || placed.truncated {
        work.lose(RingLoss::Short);
    }
    let spent = spent();
    if let Some(trace) = trace {
        trace.spent = spent;
        trace.tables = work.tables();
        trace.budget = budget;
        trace.glyphs = glyphs;
        trace.ring = work.loss();
    }
    #[cfg(test)]
    {
        super::halves::PAST_BUDGET.with(|past| past.set(past.get().saturating_add(spent.saturating_sub(budget))));
        LAST_PITCHED.with(|last| last.set(Some((spent, budget, work.loss()))));
    }
    placed
}

#[cfg(test)]
thread_local! {
    /// Test counter (#313): what the last [`place_pitched_capped`] on this
    /// thread spent, its budget, and how its budget left it.
    pub(crate) static LAST_PITCHED: std::cell::Cell<Option<(usize, usize, Option<RingLoss>)>> = const { std::cell::Cell::new(None) };
}

/// The glyphs of `marks[k]`'s cluster for each `(k, center)` of
/// `centers`, the cluster's ink centered there: its glyphs at their pens
/// in its box, as shaped.
///
/// Cost: O(the glyphs placed); allocates them.
pub(super) fn glyphs_at(
    run: &ShapedRun,
    clusters: &[Cluster],
    marks: &[Mark],
    centers: &[(usize, Vec2)],
) -> Vec<PlacedGlyph> {
    let baseline = run.baseline();
    let mut glyphs = Vec::with_capacity(centers.len() * run.glyphs.len().min(4));
    for &(k, center) in centers {
        let mark = &marks[k];
        let cluster = &clusters[mark.cluster];
        let corner = center - (mark.ink.min + mark.ink.max) * 0.5 - Vec2::new(0.0, baseline);
        for index in cluster.glyphs.clone() {
            glyphs.push(PlacedGlyph {
                glyph: index as u32,
                pos: corner + Vec2::new(run.glyphs[index].x - cluster.left, 0.0),
            });
        }
    }
    glyphs
}

/// Lay `marks` along `track` as whole inks, evenly gapped from end to
/// end ([`place_pitched`]'s rule): mark `k` round the pattern is ink `k`,
/// each set its natural gap plus the slack from the one before it, box to
/// box, and both end gaps, measured to `ends`, the mean natural gap plus
/// the slack; as many as bring the slack nearest 0 at no less than
/// `floor`, and the slack solved for. Never more than `cap` inks, a lay
/// held to it `truncated`.
///
/// `go(inks)` asks whether the lay may go on with `inks` inks in hand:
/// it is read before each ink the count sets, with the count that ink
/// makes, and before each round of the slack, with the count it lays. A
/// `false` stops the lay there — the count found so far, the slack
/// nearest so far — and the lay is then laid once at them, unasked: what
/// a caller with a budget holds back for; a `false` before the first ink
/// lays none. `None` then, and where no count of inks
/// fits from end gap to end gap at the floor — a track too short for one
/// ink — and for `marks` empty.
///
/// Cost: O(inks) track lookups to count the inks, and as many again for
/// each of at most [`SLACK_ROUNDS`] rounds of the slack for each of the
/// two counts nearest, each ink a few gap measures where the track bends;
/// O(`ends`' boxes) per end measure. Allocates the places.
pub(super) fn pitch(
    track: &impl Track,
    marks: &[Mark],
    cap: usize,
    floor: f32,
    ends: [PathEnd<'_>; 2],
    go: &dyn Fn(usize) -> bool,
) -> Option<Pitched> {
    let count = marks.len();
    if count == 0 {
        return None;
    }
    let end_gap = marks.iter().map(|m| m.gap).sum::<f32>() / count as f32;
    let length = track.length();
    let (start_dir, end_dir) = track.ends();
    let mark_of = |k: usize| &marks[k % count];
    let size_of = |k: usize| mark_of(k).size();
    let ink_box = |k: usize, s: f32| {
        let size = size_of(k);
        Rect::new(track.center(s, size) - size * 0.5, size)
    };
    // How far along `dir` two boxes' centers lie for the boxes to be
    // `want` apart, box to box, their sizes summed `sizes`: apart across
    // one axis, as far on as that takes; apart across both, where the
    // distance between their corners is `want`.
    let apart_by = |sizes: Vec2, dir: Vec2, want: f32| -> f32 {
        // Apart across one axis, a gap below 0 is an overlap that deep;
        // across both, the corners' distance is never below 0.
        let (half, d) = (sizes * 0.5, dir.abs());
        let across = |h: f32, d: f32, other: f32, other_d: f32| {
            let t = (h + want) / d;
            (d > 1e-6 && t * other_d <= other).then_some(t.max(0.0))
        };
        across(half.x, d.x, half.y, d.y)
            .or_else(|| across(half.y, d.y, half.x, d.x))
            .unwrap_or_else(|| {
                let (p, want) = (d.dot(half), want.max(0.0));
                p + (p * p - half.length_squared() + want * want).max(0.0).sqrt()
            })
    };
    // How far from an ink's center its box's edge lies along `dir`.
    let edge = |k: usize, dir: Vec2| track.reach(size_of(k), dir);
    // The gap from `b` to the nearest of an end's boxes, which lie before
    // it at the start and after it at the end.
    let nearest = |b: &Rect, inks: &[Rect], start: bool| {
        inks.iter()
            .map(|i| if start { track.gap(i, b) } else { track.gap(b, i) })
            .fold(f32::INFINITY, f32::min)
    };
    // The place at which `gap`, growing with the place, is `want`: from
    // `guess`, refined [`REFINE`] times along the secant, or bracketed and
    // bisected.
    let solve = |gap: &dyn Fn(f32) -> f32, want: f32, guess: f32| -> f32 {
        if track.bracketed() {
            return bisect(gap, want, guess);
        }
        let (mut s, mut miss, mut slope) = (guess, gap(guess) - want, 1.0_f32);
        for _ in 0..REFINE {
            let next = s - miss / slope;
            let missed = gap(next) - want;
            if (next - s).abs() > 1e-6 {
                slope = ((missed - miss) / (next - s)).clamp(0.05, 2.0);
            }
            (s, miss) = (next, missed);
        }
        s
    };
    let backs = ends.map(|e| if e.back > 0.0 { e.back } else { 0.0 });
    let holds = ends.map(|e| if e.hold > 0.0 { e.hold } else { f32::NEG_INFINITY });
    // Where the first ink sits for an end gap of `gap`: that far from the
    // start's boxes, its ink back from the start at most its `back`, its
    // center at least its `hold` on.
    let first_at = |gap: f32| -> f32 {
        let reach = edge(0, start_dir);
        let s = if ends[0].inks.is_empty() {
            reach + gap
        } else {
            solve(&|s| nearest(&ink_box(0, s), ends[0].inks, true), gap, reach + gap)
        };
        s.max(reach - backs[0]).max(holds[0])
    };
    // Where ink `k` sits as the last, for an end gap of `gap`; likewise.
    let last_at = |k: usize, gap: f32| -> f32 {
        let reach = edge(k, end_dir);
        let back = if ends[1].inks.is_empty() {
            reach + gap
        } else {
            solve(
                &|u| nearest(&ink_box(k, length - u), ends[1].inks, false),
                gap,
                reach + gap,
            )
        };
        length - back.max(reach - backs[1]).max(holds[1])
    };
    // Where ink `k + 1` sits, `want` on from ink `k` at `s`, box to box:
    // where the track runs straight on, exactly its edges and the gap on;
    // where it turns, that refined. Kept in `f64`, as the places add up:
    // thousands of `f32` steps along a long path drift by whole points,
    // and the slack is solved to a thousandth.
    let next_at = |k: usize, s: f64, want: f32| -> f64 {
        let (here, dir) = (ink_box(k, s as f32), track.at(s as f32).1);
        let guess = s + f64::from(apart_by(size_of(k) + size_of(k + 1), dir, want));
        // Box to box, where the track runs straight on the guess is the
        // place; a bracketed track's gap is not box to box, and is solved.
        if !track.bracketed() && track.at(guess as f32).1.dot(dir) > 1.0 - 1e-5 {
            return guess;
        }
        let gap = |t: f32| track.gap(&here, &ink_box(k + 1, t));
        f64::from(solve(&gap, want, guess as f32)).max(s)
    };
    // The places of `n` inks at `slack`, into `centers`, and how far the
    // last falls short of where its end gap puts it: falling as the slack
    // grows.
    let short = |n: usize, slack: f32, centers: &mut Vec<f32>| -> f32 {
        centers.clear();
        let mut s = f64::from(first_at(end_gap + slack));
        for k in 0..n {
            if k > 0 {
                s = next_at(k - 1, s, mark_of(k - 1).gap + slack);
            }
            centers.push(s as f32);
        }
        (f64::from(last_at(n - 1, end_gap + slack)) - s) as f32
    };
    // How many inks fit at their natural gaps: each placed until one
    // passes where it would sit as the last.
    let lasts: Vec<f32> = (0..count).map(|k| last_at(k, end_gap)).collect();
    let mut fitted = 0;
    let mut s = f64::from(first_at(end_gap));
    while fitted < cap && s <= f64::from(lasts[fitted % count]) {
        if !go(fitted + 1) {
            // Stopped before its first ink, it lays none.
            if fitted == 0 {
                return None;
            }
            break;
        }
        s = next_at(fitted, s, mark_of(fitted).gap);
        fitted += 1;
    }
    // The slack that lays `n` inks from end gap to end gap, if any does.
    let mut centers: Vec<f32> = Vec::new();
    #[cfg(test)]
    let mut unanswered_for: Vec<(usize, usize)> = Vec::new();
    let mut slack_for = |n: usize| -> Option<f32> {
        if short(n, floor, &mut centers) < 0.0 {
            return None;
        }
        let (mut lo, mut hi) = (floor, f32::INFINITY);
        let mut slack = 0.0_f32.max(floor);
        // The slack that came nearest: where the path turns, an ink's
        // place is measured to the canvas's own precision, which far out
        // on a long path is a few thousandths a step.
        let mut best = (f32::INFINITY, slack);
        // A miss is met to the float steps of the track's positions.
        let tolerance = resolved(SLACK_TOLERANCE, track.magnitude());
        let mut last: Option<(f32, f32)> = None;
        #[cfg(test)]
        let mut unanswered_rounds = 0;
        for _ in 0..SLACK_ROUNDS {
            if !go(n) {
                break;
            }
            let missed = short(n, slack, &mut centers);
            if missed.abs() < best.0 {
                best = (missed.abs(), slack);
            }
            if missed.abs() <= tolerance {
                break;
            }
            // A round that left the slack where it was leaves every round
            // after it the same. So does, on a path whose float steps are
            // wider than the pattern's own gaps — where its floats cannot
            // resolve them — one that moved the slack and moved the lay by
            // less than a thousandth of what the move asked of its `n + 1`
            // gaps: no ink moves with the slack there (#333). On a path
            // the floats resolve, where a curve steps each ink's place by
            // the canvas's float steps, a lay that does not answer one
            // move may answer the next, which lands across a step the
            // first did not reach, and the solve goes on
            // (`test_flow_pitched_solves_past_a_round_the_lay_does_not_answer`).
            if let Some((was_slack, was_missed)) = last {
                let moved = slack - was_slack;
                let unresolved = resolved(0.0, track.magnitude()) > end_gap;
                let unanswered = (missed - was_missed).abs() < STALLED * ((n + 1) as f32 * moved).abs();
                #[cfg(test)]
                {
                    unanswered_rounds += usize::from(unanswered);
                }
                if moved == 0.0 || (unresolved && unanswered) {
                    break;
                }
            }
            last = Some((slack, missed));
            if missed > 0.0 {
                lo = slack;
            } else {
                hi = slack;
            }
            // A bracket closed to adjacent floats holds no other slack.
            if hi.is_finite() && lo.next_up() >= hi {
                break;
            }
            // Each gap, and the two ends, take an equal share.
            let step = slack + missed / (n + 1) as f32;
            slack = if step > lo && step < hi {
                step
            } else if hi.is_finite() {
                0.5 * (lo + hi)
            } else {
                lo + 2.0 * (lo - floor).max(1.0)
            };
        }
        #[cfg(test)]
        unanswered_for.push((n, unanswered_rounds));
        Some(best.1)
    };
    let candidates: Vec<(usize, f32)> = [fitted, fitted + 1]
        .into_iter()
        .filter(|&n| n > 0 && n <= cap)
        .filter_map(|n| slack_for(n).map(|slack| (n, slack)))
        .collect();
    let nearest = candidates
        .iter()
        .copied()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
    // Tight, the loosest whose slack is not above 0 (a hair above it, as
    // the slack is solved, is 0).
    let tolerance = resolved(SLACK_TOLERANCE, track.magnitude());
    let best = if track.tight() {
        candidates
            .iter()
            .copied()
            .filter(|&(_, slack)| slack <= tolerance)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .or(nearest)
    } else {
        nearest
    };
    let (n, slack) = best?;
    let _missed = short(n, slack, &mut centers);
    #[cfg(test)]
    SOLVED.with(|solved| {
        solved.set(Some(Solved {
            count: n,
            slack,
            missed: _missed,
            unanswered: unanswered_for
                .iter()
                .find(|(count, _)| *count == n)
                .map_or(0, |&(_, k)| k),
        }))
    });
    Some(Pitched {
        centers,
        truncated: fitted >= cap,
    })
}
