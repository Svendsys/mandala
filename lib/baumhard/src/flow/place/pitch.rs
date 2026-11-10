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

use glam::Vec2;

use crate::flow::curve::Curve;
use crate::flow::outline::Rect;
use crate::flow::path::ArcPath;
use crate::flow::place::halves::resolved;
use crate::flow::place::{
    cluster_ink, clusters_of, Cluster, PlacedGlyph, PlacedRun, MAX_PLACED_GLYPHS, MIN_TABLE_SPACING,
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
struct Mark {
    cluster: usize,
    ink: Ink,
    gap: f32,
}

/// The gap between two canvas boxes, signed: how far apart they are
/// where they are apart, and less than 0 by the least they overlap by
/// where they overlap. Continuous, and growing as they move apart.
fn signed_gap(a: &Rect, b: &Rect) -> f32 {
    let d = (b.origin - a.max()).max(a.origin - b.max());
    if d.x > 0.0 || d.y > 0.0 {
        d.max(Vec2::ZERO).length()
    } else {
        d.max_element()
    }
}

/// The pattern's inked clusters, in order, each with the natural gap
/// from its ink to the next one's round the pattern: its right bearing,
/// the advances of the clusters before the next ink, that ink's left
/// bearing, and `spacing` for each step between them — at least 0. Empty
/// for a run with no ink.
fn marks_of(run: &ShapedRun, clusters: &[Cluster], spacing: f32) -> Vec<Mark> {
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
        marks[j].gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
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
    let size = marks
        .iter()
        .map(|m| (m.ink.max - m.ink.min).max_element())
        .fold(0.0, f32::max);
    Some((gap, size))
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
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let marks = marks_of(run, &clusters, spacing);
    if marks.is_empty() {
        return PlacedRun::default();
    }
    let count = marks.len();
    let end_gap = marks.iter().map(|m| m.gap).sum::<f32>() / count as f32;
    // The least slack: no gap below 0.
    let floor = -marks.iter().map(|m| m.gap).fold(end_gap, f32::min);
    let arc = ArcPath::open(path, (0.25 * run.line_height).max(MIN_TABLE_SPACING));
    let length = arc.length();
    if !length.is_finite() || length <= 0.0 {
        return PlacedRun::default();
    }
    let curves = path.as_ref();
    let (first, last) = (curves.first(), curves.last());
    let start_dir = first
        .map_or(Vec2::X, |c| c.derivative(0.0))
        .try_normalize()
        .unwrap_or(Vec2::X);
    let end_dir = last
        .map_or(Vec2::X, |c| c.derivative(1.0))
        .try_normalize()
        .unwrap_or(Vec2::X);
    // A point `s` along the path, and the way it runs there, run on
    // straight past either end.
    let at = |s: f32| -> (Vec2, Vec2) {
        if s < 0.0 {
            (arc.at(0.0).0 + start_dir * s, start_dir)
        } else if s > length {
            (arc.at(length).0 + end_dir * (s - length), end_dir)
        } else {
            arc.at(s)
        }
    };
    let mark_of = |k: usize| &marks[k % count];
    let size_of = |k: usize| {
        let ink = &mark_of(k).ink;
        ink.max - ink.min
    };
    let ink_box = |k: usize, s: f32| {
        let size = size_of(k);
        Rect::new(at(s).0 - size * 0.5, size)
    };
    // How far along `dir` two boxes' centers lie for the boxes to be
    // `want` apart, box to box, their sizes summed `sizes`: apart across
    // one axis, as far on as that takes; apart across both, where the
    // distance between their corners is `want`.
    let apart_by = |sizes: Vec2, dir: Vec2, want: f32| -> f32 {
        let (half, d, want) = (sizes * 0.5, dir.abs(), want.max(0.0));
        let across = |h: f32, d: f32, other: f32, other_d: f32| {
            let t = (h + want) / d;
            (d > 1e-6 && t * other_d <= other).then_some(t)
        };
        across(half.x, d.x, half.y, d.y)
            .or_else(|| across(half.y, d.y, half.x, d.x))
            .unwrap_or_else(|| {
                let p = d.dot(half);
                p + (p * p - half.length_squared() + want * want).max(0.0).sqrt()
            })
    };
    // How far from an ink's center its box's edge lies along `dir`.
    let edge = |k: usize, dir: Vec2| {
        let half = size_of(k) * 0.5;
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
    };
    let nearest = |b: &Rect, inks: &[Rect]| {
        inks.iter()
            .map(|i| signed_gap(b, i))
            .fold(f32::INFINITY, f32::min)
    };
    // The place at which `gap`, growing with the place, is `want`: from
    // `guess`, refined [`REFINE`] times along the secant.
    let solve = |gap: &dyn Fn(f32) -> f32, want: f32, guess: f32| -> f32 {
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
            solve(&|s| nearest(&ink_box(0, s), ends[0].inks), gap, reach + gap)
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
                &|u| nearest(&ink_box(k, length - u), ends[1].inks),
                gap,
                reach + gap,
            )
        };
        length - back.max(reach - backs[1]).max(holds[1])
    };
    // Where ink `k + 1` sits, `want` on from ink `k` at `s`, box to box:
    // where the path runs straight on, exactly its edges and the gap on;
    // where it turns, that refined. Kept in `f64`, as the places add up:
    // thousands of `f32` steps along a long path drift by whole points,
    // and the slack is solved to a thousandth.
    let next_at = |k: usize, s: f64, want: f32| -> f64 {
        let (here, dir) = (ink_box(k, s as f32), at(s as f32).1);
        let guess = s + f64::from(apart_by(size_of(k) + size_of(k + 1), dir, want));
        if at(guess as f32).1.dot(dir) > 1.0 - 1e-5 {
            return guess;
        }
        let gap = |t: f32| signed_gap(&here, &ink_box(k + 1, t));
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
    // The most inks a placement holds: whole clusters, within the cap.
    let widest = marks
        .iter()
        .map(|m| clusters[m.cluster].glyphs.len())
        .max()
        .unwrap_or(1);
    let cap = MAX_PLACED_GLYPHS / widest.max(1);
    // How many inks fit at their natural gaps: each placed until one
    // passes where it would sit as the last.
    let lasts: Vec<f32> = (0..count).map(|k| last_at(k, end_gap)).collect();
    let mut fitted = 0;
    let mut s = f64::from(first_at(end_gap));
    while fitted < cap && s <= f64::from(lasts[fitted % count]) {
        s = next_at(fitted, s, mark_of(fitted).gap);
        fitted += 1;
    }
    // The slack that lays `n` inks from end gap to end gap, if any does.
    let mut centers: Vec<f32> = Vec::new();
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
        // A miss is met to the float steps of the path's positions.
        let tolerance = resolved(SLACK_TOLERANCE, length);
        let mut last: Option<(f32, f32)> = None;
        for _ in 0..SLACK_ROUNDS {
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
            // the floats resolve, a lay that does not answer one move may
            // answer a later, larger one, and the solve goes on.
            if let Some((was_slack, was_missed)) = last {
                let moved = slack - was_slack;
                let unresolved = resolved(0.0, length) > end_gap;
                let unanswered = (missed - was_missed).abs() < STALLED * ((n + 1) as f32 * moved).abs();
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
        Some(best.1)
    };
    let best = [fitted, fitted + 1]
        .into_iter()
        .filter(|&n| n > 0 && n <= cap)
        .filter_map(|n| slack_for(n).map(|slack| (n, slack)))
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
    let Some((n, slack)) = best else {
        return PlacedRun::default();
    };
    short(n, slack, &mut centers);
    let baseline = run.baseline();
    let mut glyphs = Vec::with_capacity(n * run.glyphs.len().min(4));
    for (k, &s) in centers.iter().enumerate() {
        let mark = mark_of(k);
        let cluster = &clusters[mark.cluster];
        let corner = at(s).0 - (mark.ink.min + mark.ink.max) * 0.5 - Vec2::new(0.0, baseline);
        for index in cluster.glyphs.clone() {
            glyphs.push(PlacedGlyph {
                glyph: index as u32,
                pos: corner + Vec2::new(run.glyphs[index].x - cluster.left, 0.0),
            });
        }
    }
    PlacedRun {
        glyphs,
        truncated: fitted >= cap,
    }
}
