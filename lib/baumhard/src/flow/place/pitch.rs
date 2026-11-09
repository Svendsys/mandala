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
//! - **The pitch is the pattern's own**, whichever way the path runs:
//!   two consecutive inks' centers are as far apart along the path as the
//!   font sets them on a line — the first's half width and right bearing,
//!   every space's advance between them, the second's left bearing and
//!   half width — plus `spacing` for each step. So the connectors of one
//!   style keep one pitch, level, steep or round a bend (rule 5), and a
//!   round glyph one gap.
//! - **The count is whole.** As many inks as bring every gap nearest its
//!   natural one, and the difference spread over them all: each gap,
//!   the ends' too, grows or shrinks by the same amount. So the pitch is
//!   within half a glyph's share of its natural one, whatever the path's
//!   length.
//! - **The ends are measured.** The gap from the first ink to the boxes
//!   at the start ([`PathEnd::inks`], a frame's inks or a node's box) is
//!   held to the gap its inks are laid at — the median of the gaps
//!   between neighbors, box to box, as the path bends them: the path
//!   starts where its end sits, and where the boxes lie further back or
//!   further on than the path's start, the glyphs move back or on to keep
//!   the gap — back at most [`PathEnd::back`], along the path's first
//!   direction. Likewise at the far end. An end held out
//!   ([`PathEnd::hold`]) keeps its first ink that far out instead.
//!
//! Nothing is reversed: an ornament has no reading direction (ruling
//! R29).

use glam::Vec2;

use crate::flow::curve::Curve;
use crate::flow::outline::Rect;
use crate::flow::path::ArcPath;
use crate::flow::place::{
    cluster_ink, clusters_of, Cluster, PlacedGlyph, PlacedRun, MAX_PLACED_GLYPHS, MIN_TABLE_SPACING,
};
use crate::flow::shape::{Ink, ShapedRun};

/// How many rounds the ends are measured and moved in. Each round moves
/// an end by what its gap missed by; the gap moves with it one for one
/// where the boxes face the path, so a round or two settles it, and the
/// rest only take up a turn the path makes near its end.
const END_ROUNDS: usize = 8;

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

/// The gap between two canvas boxes: `0` where they meet or overlap.
pub(crate) fn box_gap(a: &Rect, b: &Rect) -> f32 {
    let d = (b.origin - a.max()).max(a.origin - b.max()).max(Vec2::ZERO);
    d.length()
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
/// repeated in order, as many as bring every gap nearest the pattern's
/// own, both end gaps measured to `ends`. The count is taken again for
/// the room the measured ends leave, and the ends measured again for a
/// count that changed; a count that would change a second time is not
/// taken, so the glyphs are always laid at a count their ends were
/// measured for. `spacing` is added to every step, as a `repeat` walk
/// adds it; one that is not finite or not positive adds nothing.
///
/// A run with no ink — text, or only spaces — has no pitch, and is
/// placed empty: [`place_on_open`](crate::flow::place_on_open) lays it.
/// So is a path that is not finite, and one too short to hold one ink.
/// Never more than [`MAX_PLACED_GLYPHS`] glyphs; a placement held to it
/// is `truncated`, its gaps spread over the inks it holds.
///
/// Cost: O(clusters) to read the pattern, and a few rounds of O(inks) path
/// lookups and O(`ends`' boxes) gap measures; the path is a table per
/// curve, as [`place_on_open`](crate::flow::place_on_open)'s. Allocates
/// the placement and one scratch vector of positions.
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
    // A point `s` along the path, run on straight past either end.
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
    let width = |k: usize| {
        let ink = &mark_of(k).ink;
        ink.max.x - ink.min.x
    };
    // The most inks a placement holds: whole clusters, within the cap.
    let widest = marks
        .iter()
        .map(|m| clusters[m.cluster].glyphs.len())
        .max()
        .unwrap_or(1);
    let cap = MAX_PLACED_GLYPHS / widest.max(1);
    // How many inks `room` holds: as many as fit at their natural gaps, or
    // one more, whichever needs the smaller change of gap — and whether
    // the cap held it. `used` is the length the inks fitted take so, the
    // end gaps included; `more`, one more's.
    let count_in = |room: f32| -> (usize, bool) {
        let (mut used, mut fitted) = (2.0 * end_gap, 0usize);
        let more = loop {
            let between = if fitted > 0 { mark_of(fitted - 1).gap } else { 0.0 };
            let more = used + width(fitted) + between;
            if more > room || fitted >= cap {
                break more;
            }
            used = more;
            fitted += 1;
        };
        let change = |n: usize, need: f32| (room - need) / (n + 1) as f32;
        let n = if fitted == 0 {
            // One ink, if the room holds it with its end gaps no less than 0.
            usize::from(change(1, more) + end_gap >= 0.0)
        } else if fitted < cap && change(fitted + 1, more).abs() < change(fitted, used).abs() {
            fitted + 1
        } else {
            fitted
        };
        (n, fitted >= cap)
    };
    // The length `n` inks take at their natural gaps, the ends' included.
    let natural = |n: usize| {
        2.0 * end_gap
            + (0..n).map(width).sum::<f32>()
            + (0..n.saturating_sub(1)).map(|k| mark_of(k).gap).sum::<f32>()
    };
    // Each of `n` inks' centers along the path, between `from` and `to`,
    // every gap its natural one and the slack spread over them all, the
    // ends' too: one pitch whichever way the path runs.
    let lay = |n: usize, from: f32, to: f32, centers: &mut Vec<f32>| {
        centers.resize(n, 0.0);
        let delta = ((to - from) - natural(n)) / (n + 1) as f32;
        let mut s = from + end_gap + delta;
        for (k, center) in centers.iter_mut().enumerate() {
            *center = s + 0.5 * width(k);
            s += width(k) + mark_of(k).gap + delta;
        }
        end_gap + delta
    };
    let ink_box = |k: usize, s: f32| {
        let ink = &mark_of(k).ink;
        let size = ink.max - ink.min;
        Rect::new(at(s).0 - size * 0.5, size)
    };
    let nearest = |b: &Rect, inks: &[Rect]| inks.iter().map(|i| box_gap(b, i)).fold(f32::INFINITY, f32::min);
    // The gap the inks are laid at, as their boxes are apart where the
    // path bends them: the median of the gaps between neighbors.
    let laid_gap = |centers: &[f32]| {
        let mut gaps: Vec<f32> = centers
            .windows(2)
            .enumerate()
            .map(|(k, w)| box_gap(&ink_box(k, w[0]), &ink_box(k + 1, w[1])))
            .collect();
        gaps.sort_by(f32::total_cmp);
        gaps.get(gaps.len() / 2).copied()
    };
    let backs = ends.map(|e| if e.back > 0.0 { e.back } else { 0.0 });
    let holds = ends.map(|e| if e.hold > 0.0 { e.hold } else { 0.0 });
    let (mut from, mut to) = (0.0_f32, length);
    let (mut n, mut capped) = count_in(length);
    let mut centers: Vec<f32> = Vec::new();
    // The ends measured, a few rounds; then the count again for the room
    // they left, and the ends again if it changed — once.
    for pass in 0..2 {
        if n == 0 {
            return PlacedRun::default();
        }
        // The ends that came nearest their gaps: a frame's inks lie apart,
        // so the gap to the nearest jumps as a glyph passes from one to the
        // next, and the rounds may then step back and forth over it.
        let mut best: Option<(f32, f32, f32)> = None;
        for round in 0..END_ROUNDS {
            let laid = lay(n, from, to, &mut centers);
            let (laid_from, laid_to) = (from, to);
            let (first, last) = (centers[0], centers[n - 1]);
            let want = laid_gap(&centers).unwrap_or(laid);
            // After the first round, half of what each end missed by.
            let share = if round == 0 { 1.0 } else { 0.5 };
            let mut missed = 0.0;
            // A held end's glyph lies at its hold, or further out where its
            // frame's inks reach past it.
            let reach = [holds[0] - first, last - (length - holds[1])];
            if !ends[0].inks.is_empty() {
                let gap = nearest(&ink_box(0, first), ends[0].inks);
                if gap.is_finite() && (holds[0] <= 0.0 || gap < want) {
                    missed += (gap - want).abs();
                    from = (from + share * (want - gap)).max(-backs[0] - laid);
                }
            }
            if !ends[1].inks.is_empty() {
                let gap = nearest(&ink_box(n - 1, last), ends[1].inks);
                if gap.is_finite() && (holds[1] <= 0.0 || gap < want) {
                    missed += (gap - want).abs();
                    to = (to - share * (want - gap)).min(length + backs[1] + laid);
                }
            }
            if holds[0] > 0.0 {
                missed += reach[0].max(0.0);
                from = from.max(laid_from + reach[0]);
            }
            if holds[1] > 0.0 {
                missed += reach[1].max(0.0);
                to = to.min(laid_to - reach[1]);
            }
            if best.is_none_or(|b| missed < b.0) {
                best = Some((missed, laid_from, laid_to));
            }
            if missed <= 0.01 * want.max(0.01)
                && (from - laid_from).abs() <= 1e-3
                && (to - laid_to).abs() <= 1e-3
            {
                break;
            }
        }
        if let Some((_, f, t)) = best {
            (from, to) = (f, t);
        }
        // No end's ink past its limit, into the node: the end moved back
        // as far as it reached past, twice, the gaps' share moving with it.
        for _ in 0..2 {
            lay(n, from, to, &mut centers);
            // How far an ink reaches along the path's end direction.
            let half = |k: usize, dir: Vec2| {
                let size = mark_of(k).ink.max - mark_of(k).ink.min;
                0.5 * (dir.x.abs() * size.x + dir.y.abs() * size.y)
            };
            let over = [
                -backs[0] - (centers[0] - half(0, start_dir)),
                centers[n - 1] + half(n - 1, end_dir) - (length + backs[1]),
            ];
            from += over[0].max(0.0);
            to -= over[1].max(0.0);
        }
        let recount = count_in(to - from);
        if recount.0 == n || pass == 1 {
            break;
        }
        (n, capped) = recount;
    }
    if n == 0 {
        return PlacedRun::default();
    }
    lay(n, from, to, &mut centers);
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
        truncated: capped,
    }
}
