// SPDX-License-Identifier: MPL-2.0

//! The projection's geometry (design §5.5, #241 §2): a node's drawn
//! rectangle, where a path attaches to its ends, a ring's size in
//! points, where a label and a portal end sit. Pure functions over plain
//! values; `format/projection.md` is the spelling of every rule here.

use glam::{DVec2, Vec2};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::document::resolve::BorderPoint;
use crate::document::values::{Anchor, Direction, Edge, FontSize, Shape, StartPoint};
use crate::flow::{ArcPath, Curve, Local, Outline, Rect};
use crate::font::fonts::{clamp_font_metric, MIN_FONT_SIZE_PT};
use crate::gfx_structs::shape::NodeShape;

/// The table spacing of the arc-length walks a label and a portal end
/// are found on, in canvas units: one point is asked of each, so a
/// table finer than a canvas unit would cost samples for nothing.
const POINT_TABLE_SPACING: f32 = 1.0;

/// A stored size read as a length per axis: an axis that is not finite,
/// or negative, reads as `0` (#279's note: a `min_size` that is
/// non-finite or negative reads as 0). `None` is `(0, 0)`. O(1).
pub(crate) fn usable_size(size: Option<Vec2>) -> Vec2 {
    let axis = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    size.map_or(Vec2::ZERO, |s| Vec2::new(axis(s.x), axis(s.y)))
}

/// A stored canvas point, or the origin when an axis is not finite. O(1).
pub(crate) fn usable_point(point: Vec2) -> Vec2 {
    if point.is_finite() {
        point
    } else {
        Vec2::ZERO
    }
}

/// A node's drawn rectangle (D5, §5.5): at `position`, as large as the
/// larger of its minimum size and the extent of its sections' boxes
/// measured from its position — each section's `offset + size` — and at
/// least `(0, 0)`. `extents` are those far corners, relative to the
/// position. O(sections).
pub(crate) fn drawn_rect(
    position: Vec2,
    min_size: Option<Vec2>,
    extents: impl Iterator<Item = Vec2>,
) -> Rect {
    let mut size = usable_size(min_size);
    for far in extents {
        if far.is_finite() {
            size = size.max(far);
        }
    }
    Rect::new(usable_point(position), size.max(Vec2::ZERO))
}

/// The share of the distance between a cubic's two ends its handles
/// reach along the end directions: 0.5523, at which one cubic draws a
/// quarter circle between two perpendicular sides as closely as one can
/// (#296). A path between two facing sides is then an S, and one between
/// a side and the side turned a quarter from it a round bend.
const HANDLE_SHARE: f32 = 0.5523;

/// How far a path end may turn from its side's normal to keep clear of
/// the other paths that leave the same side (#296, rule 4).
pub(crate) const SPLAY_CAP: f32 = 40.0 * std::f32::consts::PI / 180.0;

/// The turn between two paths that leave one point at which their first
/// glyphs clear each other by `gap` (#296, rule 4), for glyphs `size`
/// across whose first sits its end gap, `gap`, from the point: its
/// center `gap + size / 2` out, and two such centers `2 r sin(θ / 2)`
/// apart at a turn of `θ`, which is to be `gap + size`. Radians;
/// a half turn where no turn clears them, or the sizes are not finite.
/// O(1).
pub(crate) fn spread(gap: f32, size: f32) -> f32 {
    let (gap, size) = (gap.max(0.0), size.max(0.0));
    let ratio = (gap + size) / (2.0 * gap + size);
    if ratio.is_finite() && ratio < 1.0 {
        2.0 * ratio.asin()
    } else {
        std::f32::consts::PI
    }
}

/// One path of a fan ([`fan`]): the least turn between it and a
/// neighbor at which their first glyphs clear each other by its own gap
/// (`spread`), how far apart their first glyphs' centers must be to clear
/// side by side or across a slope (`clear`), and, for a straight path,
/// the turn from the normal its one segment leaves in, which no fan
/// changes (`fixed`); and the straight line from its end here to its
/// other end (`chord`), a quarter of which is as far as it can be held
/// out, as a lead is ([`path_through`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FanMember {
    pub(crate) spread: f32,
    pub(crate) clear: f32,
    pub(crate) fixed: Option<f32>,
    pub(crate) chord: f32,
}

/// How far a fan of `members` holds its first glyphs out when none of
/// them is straight ([`fan`]): the hold its turns' order is read from,
/// which needs no order. No fan is laid. O(members); no allocation.
pub(crate) fn fan_hold(members: &[FanMember]) -> f32 {
    if members.len() < 2 {
        return 0.0;
    }
    let (widest, clear) = widest_spread_and_clear(members);
    held_out(
        widest.min(2.0 * SPLAY_CAP / (members.len() - 1) as f32),
        widest,
        clear,
    )
}

/// The widest spread and the widest clear of `members`. O(members).
fn widest_spread_and_clear(members: &[FanMember]) -> (f32, f32) {
    members.iter().fold((0.0_f32, 0.0_f32), |(spread, clear), m| {
        (spread.max(m.spread), clear.max(m.clear))
    })
}

/// How far neighbors `step` apart are held out to clear each other by
/// `clear`: nowhere at the widest spread or wider, else the clear over
/// twice the sine of half the step. 0 where that is not finite. O(1).
fn held_out(step: f32, widest: f32, clear: f32) -> f32 {
    let hold = if step < widest {
        clear / (2.0 * (0.5 * step).sin())
    } else {
        0.0
    };
    if hold.is_finite() {
        hold
    } else {
        0.0
    }
}

/// The turns and holds of the paths that leave one point of a side, in
/// order (#296, rule 4): each two neighbors apart by the widest spread
/// among them, and none more than [`SPLAY_CAP`] from the normal — the
/// fan narrowed to hold it. Narrowed, its first glyphs are held out from
/// the point to where neighbors that far apart clear each other: their
/// centers the widest `clear` apart — the gap and the glyph's diagonal, as
/// the spread is reckoned, since two neighbors turned off the normal
/// stand side by side across a slope — so the widest clear over twice
/// the sine of half the step. A path alone leaves along the normal, held
/// nowhere.
///
/// A straight path keeps the turn its segment leaves in (`fixed`), and
/// the others fan round it. Those before the first straight path and
/// after the last take the widest step, up to the widest spread, at
/// which they keep that far from their neighbors within the cap — or,
/// past a straight path, within an even capped fan's step of room for
/// each path beyond it, where that reaches further. Those between two
/// straight paths share the room between them, a step apart where it
/// holds them. Each takes the even fan's turn, as near as that allows.
///
/// Each path is held out as far as its own neighbors need: as far as the
/// step needs, and further only for a neighbor nearer than the step —
/// two straight paths may leave nearer each other than that, and the
/// paths between them are as near as their room makes them — but never
/// past a quarter of its own `chord`, as far as it can carry. So a path
/// keeps its glyphs whatever its neighbors do, and its hold grows
/// continuously as two directions meet. Two straight paths that leave
/// in exactly one direction cannot be told apart: they overlap.
///
/// Cost: O(members) with no straight path; with one, O(members) for each
/// of a fixed number of trial steps. One allocation, three with a
/// straight path.
pub(crate) fn fan(members: &[FanMember]) -> Vec<(f32, f32)> {
    if members.len() < 2 {
        return members.iter().map(|m| (m.fixed.unwrap_or(0.0), 0.0)).collect();
    }
    let last = (members.len() - 1) as f32;
    let (widest, clear) = widest_spread_and_clear(members);
    // The hold that neighbors `gap` apart need, at most a quarter of
    // `chord`: none at the widest spread or wider, without bound as the
    // gap closes — so capped, and the cap reached continuously.
    let held = |gap: f32, chord: f32| {
        let cap = (0.25 * chord).max(0.0);
        let need = if gap >= widest {
            0.0
        } else if gap > 0.0 {
            clear / (2.0 * (0.5 * gap).sin())
        } else {
            f32::INFINITY
        };
        if need.is_nan() {
            0.0
        } else {
            need.min(cap)
        }
    };
    if members.iter().all(|m| m.fixed.is_none()) {
        let step = widest.min(2.0 * SPLAY_CAP / last);
        return members
            .iter()
            .enumerate()
            .map(|(rank, m)| ((rank as f32 - 0.5 * last) * step, held(step, m.chord)))
            .collect();
    }
    let fixed = |rank: usize| members[rank].fixed.filter(|t| t.is_finite());
    // The room the paths that are not straight may turn in: the cap, or
    // past a straight path, as far as an even capped fan's step for each
    // path beyond it reaches, if that is further.
    let even = 2.0 * SPLAY_CAP / last;
    let mut low = -SPLAY_CAP;
    let mut high = SPLAY_CAP;
    for rank in 0..members.len() {
        if let Some(turn) = fixed(rank) {
            low = low.min(turn - rank as f32 * even);
            high = high.max(turn + (last - rank as f32) * even);
        }
    }
    // The step each path keeps from the one before it, at `step`: between
    // two straight paths, no more than their room shares out.
    let pins: Vec<usize> = (0..members.len()).filter(|&r| fixed(r).is_some()).collect();
    let apart = |rank: usize, step: f32| -> f32 {
        let below = pins.iter().rev().find(|&&p| p < rank);
        let above = pins.iter().find(|&&p| p >= rank);
        match (below, above) {
            (Some(&a), Some(&b)) => {
                let room = fixed(b).unwrap_or(0.0) - fixed(a).unwrap_or(0.0);
                step.min((room / (b - a) as f32).max(0.0))
            }
            _ => step,
        }
    };
    // The turns packed as far toward `low` (`up`) or `high` (`!up`) as
    // the steps allow, each straight path at its own; `None` where the
    // paths that are not straight do not fit.
    let packed = |step: f32, up: bool| -> Option<Vec<f32>> {
        let mut turns = vec![0.0_f32; members.len()];
        let mut before: Option<(usize, f32)> = None;
        let ranks: Vec<usize> = if up {
            (0..members.len()).collect()
        } else {
            (0..members.len()).rev().collect()
        };
        for rank in ranks {
            let turn = match fixed(rank) {
                Some(turn) => turn,
                None if up => {
                    let turn = before.map_or(low, |(_, b)| (b + apart(rank, step)).max(low));
                    (turn <= high + 1e-6).then_some(turn)?
                }
                None => {
                    let turn = before.map_or(high, |(b_rank, b)| (b - apart(b_rank, step)).min(high));
                    (turn >= low - 1e-6).then_some(turn)?
                }
            };
            turns[rank] = turn;
            before = Some((rank, turn));
        }
        Some(turns)
    };
    // The widest step that fits: the widest spread if it does, else found
    // by halving the interval it lies in.
    let fits = |step: f32| {
        let (Some(a), Some(b)) = (packed(step, true), packed(step, false)) else {
            return false;
        };
        // Each path not straight between its two packings.
        a.iter().zip(&b).all(|(a, b)| *a <= *b + 1e-5)
    };
    let step = if fits(widest) {
        widest
    } else {
        let (mut lo, mut hi) = (0.0_f32, widest);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let turns: Vec<f32> = match (packed(step, true), packed(step, false)) {
        // The even fan's turn, held between the two packings: each two
        // neighbors stay their step apart, both packings being so.
        (Some(least), Some(most)) => (0..members.len())
            .map(|rank| {
                let even = (rank as f32 - 0.5 * last) * step;
                fixed(rank).unwrap_or(even.clamp(least[rank], most[rank].max(least[rank])))
            })
            .collect(),
        _ => (0..members.len())
            .map(|rank| fixed(rank).unwrap_or((rank as f32 - 0.5 * last) * step))
            .collect(),
    };
    // Each held out as far as the step and its own two neighbors need.
    (0..members.len())
        .map(|rank| {
            let gap = |other: usize| (turns[other] - turns[rank]).abs();
            let nearest = [
                rank.checked_sub(1),
                (rank + 1 < members.len()).then_some(rank + 1),
            ]
            .into_iter()
            .flatten()
            .map(gap)
            .fold(step, f32::min);
            (turns[rank], held(nearest, members[rank].chord))
        })
        .collect()
}

/// The side of `rect` a path attaches to: `anchor`'s, or for
/// [`Anchor::Auto`] the side the ray from `rect`'s center toward
/// `toward` leaves it through (#296, rule 2): left or right where
/// `|dx| / half-width` is the larger, top or bottom otherwise — on a tie,
/// and for a ray of no length or not finite, top or bottom by `dy`'s sign,
/// top at 0. A box of no width and no height compares `|dx|` and `|dy|`.
/// Never [`Anchor::Auto`]. O(1).
pub(crate) fn side_toward(rect: Rect, anchor: Anchor, toward: Vec2) -> Anchor {
    if anchor != Anchor::Auto {
        return anchor;
    }
    let d = toward - rect.center();
    let d = if d.is_finite() { d } else { Vec2::ZERO };
    let half = rect.size * 0.5;
    let (across, along) = if half == Vec2::ZERO {
        (d.x.abs(), d.y.abs())
    } else {
        (d.x.abs() * half.y, d.y.abs() * half.x)
    };
    match (across > along, d.x > 0.0, d.y > 0.0) {
        (true, true, _) => Anchor::Right,
        (true, false, _) => Anchor::Left,
        (false, _, true) => Anchor::Bottom,
        (false, _, false) => Anchor::Top,
    }
}

/// The midpoint of `side` of `rect`, and the side's outward normal.
/// [`Anchor::Auto`] reads as the top. O(1).
pub(crate) fn side_point(rect: Rect, side: Anchor) -> (Vec2, Vec2) {
    let (c, max) = (rect.center(), rect.max());
    match side {
        Anchor::Right => (Vec2::new(max.x, c.y), Vec2::X),
        Anchor::Bottom => (Vec2::new(c.x, max.y), Vec2::Y),
        Anchor::Left => (Vec2::new(rect.origin.x, c.y), -Vec2::X),
        Anchor::Top | Anchor::Auto => (Vec2::new(c.x, rect.origin.y), -Vec2::Y),
    }
}

/// The signed angle from `normal` to `toward`, radians in `-π..=π`:
/// positive turning as [`Vec2::from_angle`] turns, clockwise on the
/// screen. What the paths leaving one side are ordered by. `0` for a
/// direction of no length. O(1).
pub(crate) fn turn_to(normal: Vec2, toward: Vec2) -> f32 {
    let a = normal.perp_dot(toward).atan2(normal.dot(toward));
    if a.is_finite() {
        a
    } else {
        0.0
    }
}

/// The curves of a path (#296, rule 3): from `start` to `end`, each a
/// point, the direction the path leaves it in — at `end`, the
/// direction it leaves *backward*, the end side's outward normal for a
/// path that arrives straight on — and its lead, through each of `via`.
/// Point `k` of `n` is the point `k / (n + 1)` of the way along the
/// straight line from end to end, moved by `via[k - 1]`; one that is not
/// finite moves it nothing.
///
/// Each end first runs straight along its direction for its own lead —
/// its first two glyphs, and as far as the paths that share its point
/// hold their glyphs out, so they step square to their side however the
/// path then bends — but never more than a quarter of the straight line
/// from end to end. Between the two leads, one cubic between each
/// two points: at a lead's end along its direction, at a point between
/// along the line through the points on either side of it (a Catmull-Rom
/// spline), so the curves join without a kink. A lead's handle reaches
/// [`HANDLE_SHARE`] of how far its curve's other end lies along its
/// direction, and at least a third of the curve's chord, so a curve
/// between facing sides is an S as deep as the sides are apart and one
/// between a side and the side turned a quarter from it a round bend; a
/// handle between is a third of the tangent. `handles` scales the two
/// leads' handles, from `1`: less, the path turns toward its other end
/// sooner after each lead, as the paths of a crowded junction must to
/// keep clear of each other ([`untangle`]); still C1.
///
/// Cost: O(`via`); one allocation, `via.len() + 3` curves.
pub(crate) fn path_through(
    start: (Vec2, Vec2, f32),
    end: (Vec2, Vec2, f32),
    via: &[Vec2],
    handles: f32,
) -> Vec<Curve> {
    let handles = if handles.is_finite() {
        handles.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let (p0, p1) = (usable_point(start.0), usable_point(end.0));
    let room = 0.25 * p0.distance(p1);
    let lead = |l: f32| if l.is_finite() { l.clamp(0.0, room) } else { 0.0 };
    let (l0, l1) = (lead(start.2), lead(end.2));
    let (q0, q1) = (p0 + start.1 * l0, p1 + end.1 * l1);
    let n = via.len();
    let knot = |k: usize| match k {
        0 => q0,
        k if k > n => q1,
        k => through(p0, p1, via, k),
    };
    // A lead's handle: along `dir`, as far as the curve reaches that way.
    let handle = |from: Vec2, dir: Vec2, to: Vec2| {
        let chord = to - from;
        dir * (handles * HANDLE_SHARE * chord.dot(dir).abs().max(chord.length() / 3.0))
    };
    // The tangent at knot `k`, as a handle's reach: out of it for the
    // curve after, into it for the curve before.
    let tangent = |k: usize| -> Vec2 {
        if k == 0 {
            handle(q0, start.1, knot(1))
        } else if k > n {
            -handle(q1, end.1, knot(n))
        } else {
            (knot(k + 1) - knot(k - 1)) / 6.0
        }
    };
    let mut curves = Vec::with_capacity(n + 3);
    if l0 > 0.0 {
        curves.push(Curve::Line(p0, q0));
    }
    curves.extend((0..=n).map(|k| {
        let (a, b) = (knot(k), knot(k + 1));
        Curve::Cubic(a, a + tangent(k), b - tangent(k + 1), b)
    }));
    if l1 > 0.0 {
        curves.push(Curve::Line(q1, p1));
    }
    curves
}

/// The point a path from `p0` to `p1` passes through for control point
/// `k - 1` of `via` (D34), `k` in `1..=via.len()`: the point `k / (n + 1)`
/// of the way along the straight line from end to end, moved by it; one
/// that is not finite moves it nothing. O(1).
fn through(p0: Vec2, p1: Vec2, via: &[Vec2], k: usize) -> Vec2 {
    p0 + (p1 - p0) * (k as f32 / (via.len() + 1) as f32) + usable_point(via[k - 1])
}

/// Every point a path from `start` to `end` through `via` passes through,
/// in order ([`path_through`]): its two ends, and between them the point
/// of each control point. O(`via`); one allocation.
pub(crate) fn knots(start: Vec2, end: Vec2, via: &[Vec2]) -> Vec<Vec2> {
    let (p0, p1) = (usable_point(start), usable_point(end));
    let mut out = Vec::with_capacity(via.len() + 2);
    out.push(p0);
    out.extend((1..=via.len()).map(|k| through(p0, p1, via, k)));
    out.push(p1);
    out
}

/// How many rounds [`untangle`] takes, at most: each halves the handles
/// of the paths that come too near another, down to [`LEAST_HANDLES`].
pub(crate) const UNTANGLE_ROUNDS: usize = 14;

/// How much nearer than its width [`untangle`] keeps another path's
/// curve: a quarter more, as two curves' glyphs are boxes upright on
/// them, whose corners reach further across a slope than a glyph's own
/// size.
pub(crate) const CLEARANCE: f32 = 1.25;

/// The least share of their handles [`untangle`] leaves a path: all but
/// straight from lead to lead.
const LEAST_HANDLES: f32 = 1.0 / 4096.0;

/// One path of a junction, as [`untangle`] reads it.
#[derive(Clone, Debug)]
pub(crate) struct Lane {
    /// The junction's point, the way the path leaves it, turned by its
    /// fan, and how far it runs straight before it bends: its first two
    /// glyphs' reach and its fan's hold.
    pub(crate) start: (Vec2, Vec2, f32),
    /// Where its first glyph is held out to: inside it, the fan keeps the
    /// paths apart.
    pub(crate) hold: f32,
    /// Its far end: the point, the way it leaves backward, its lead.
    pub(crate) end: (Vec2, Vec2, f32),
    /// Its through points.
    pub(crate) via: Vec<Vec2>,
    /// How near another path's curve may come to its own: its own gap and
    /// a glyph.
    pub(crate) width: f32,
    /// Whether it is one straight segment from end to end, which its
    /// handles do not bend.
    pub(crate) straight: bool,
}

/// The share of its handles each path of a junction keeps, so that no two
/// come nearer each other than their widths along their whole length
/// (#296, rule 4: "no glyph overlaps", as ruled on #326): `lanes` in fan
/// order, their curves sampled every quarter width past their holds. The
/// paths that come too near another turn toward their far ends sooner,
/// their handles halved, a round at a time, at most [`UNTANGLE_ROUNDS`];
/// a path that keeps clear keeps its handles. Of the rounds, the one with
/// the fewest paths too near, and of those the fullest handles, is kept:
/// two paths whose far ends lie on one line from the junction meet
/// however straight they run, and keep the curves they had.
///
/// The rounds stop early when it is certain that no later round can keep
/// fewer paths too near: when the paths too near another in every round
/// to come, as read from the samples no halving moves, are as many as the
/// fewest so far (`always_near`) — two paths that arrive at one point
/// along one lead, say. What is kept is what all the rounds would keep.
/// Paths that meet only along the curves a halving moves give no such
/// certainty, and may take every round: all of them while they stay too
/// near, fewer when a halving parts them.
///
/// Cost: at most [`UNTANGLE_ROUNDS`] rounds. The first samples every
/// lane's curves, each to at most the sampling's cap (`sample`), and each
/// after it those of the lanes it halved alone; each round grids them and
/// measures them against the points of each lane near them
/// ([`NearGrid`]); and once, the samples that no halving moves
/// (`always_near`).
pub(crate) fn untangle(lanes: &[Lane]) -> Vec<f32> {
    let mut handles = vec![1.0_f32; lanes.len()];
    if lanes.len() < 2 {
        return handles;
    }
    // Once the fuzz harness's ceiling has cut a check short, what is laid
    // out is discarded: no samples are taken for it.
    #[cfg(any(test, fuzzing))]
    if UNTANGLE_CUT.with(std::cell::Cell::get) {
        return handles;
    }
    let widths: Vec<f32> = lanes.iter().map(|lane| lane.width).collect();
    let lay = |lane: &Lane, h: f32| {
        let curves = if lane.straight {
            vec![Curve::Line(lane.start.0, lane.end.0)]
        } else {
            path_through(lane.start, lane.end, &lane.via, h)
        };
        let taken = sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width));
        #[cfg(test)]
        UNTANGLE_SAMPLED.with(|count| count.set(count.get() + taken.len()));
        taken
    };
    // Whether the fuzz harness's ceiling leaves room for a lane's samples,
    // taken from it lane by lane, before they are gridded: past it, no
    // further lane is gridded and no further round taken. Always, in a
    // shipped build, which has no ceiling.
    #[cfg(any(test, fuzzing))]
    let within = |taken: &Vec<Vec2>| within_ceiling(taken.len());
    #[cfg(not(any(test, fuzzing)))]
    let within = |_: &Vec<Vec2>| true;
    let mut samples: Vec<Vec<Vec2>> = Vec::with_capacity(lanes.len());
    for (lane, &h) in lanes.iter().zip(&handles) {
        let taken = lay(lane, h);
        if !within(&taken) {
            return handles;
        }
        samples.push(taken);
    }
    // A round measures again only the lanes whose curves the last one
    // changed, against every lane's points, which stay in one grid from
    // round to round: two lanes neither of which changed are as near, or
    // as clear, as they were ([`NearGrid`]).
    let mut grid = NearGrid::new(&widths);
    for (k, points) in samples.iter().enumerate() {
        #[cfg(test)]
        UNTANGLE_GRIDDED.with(|count| count.set(count.get() + points.len()));
        grid.insert(k, points);
    }
    let mut fresh = vec![true; lanes.len()];
    let mut pairs = rustc_hash::FxHashSet::default();
    let mut best = (usize::MAX, handles.clone());
    let mut always: Option<usize> = None;
    for round in 0..UNTANGLE_ROUNDS {
        if round > 0 {
            let mut cut = false;
            for (k, lane) in lanes.iter().enumerate().filter(|&(k, _)| fresh[k]) {
                grid.remove(k, &samples[k]);
                samples[k] = lay(lane, handles[k]);
                if !within(&samples[k]) {
                    cut = true;
                    break;
                }
                #[cfg(test)]
                UNTANGLE_GRIDDED.with(|count| count.set(count.get() + samples[k].len()));
                grid.insert(k, &samples[k]);
            }
            if cut {
                break;
            }
        }
        #[cfg(test)]
        UNTANGLE_GRIDS.with(|count| count.set(count.get() + 1));
        pairs = grid.near_pairs(&samples, &fresh, &pairs);
        let mut near = vec![false; lanes.len()];
        for &(k, j) in &pairs {
            near[k] = true;
            near[j] = true;
        }
        let count = near.iter().filter(|&&n| n).count();
        if count < best.0 {
            best = (count, handles.clone());
        }
        if count == 0 || *always.get_or_insert_with(|| always_near(lanes, &widths, &samples)) >= best.0 {
            break;
        }
        for (k, h) in handles.iter_mut().enumerate() {
            // A straight lane is fixed: its segment follows no handles,
            // so it keeps them.
            let halved = if near[k] && !lanes[k].straight {
                (*h * 0.5).max(LEAST_HANDLES)
            } else {
                *h
            };
            fresh[k] = halved != *h;
            *h = halved;
        }
    }
    best.1
}

/// How many of `lanes` come too near another in every round of
/// [`untangle`], whatever their handles: those with samples that no
/// halving moves nearer another's such samples than [`CLEARANCE`] times
/// the wider of their `widths` — a straight lane's every one, as it was
/// `taken` in the first round and in each since, and a curved lane's
/// `fixed_samples`. A round can keep fewer paths too near than this no
/// more than any other. O(the curved lanes' leads' samples, and the
/// samples measured as a round measures them).
fn always_near(lanes: &[Lane], widths: &[f32], taken: &[Vec<Vec2>]) -> usize {
    let fixed: Vec<std::borrow::Cow<'_, [Vec2]>> = lanes
        .iter()
        .zip(taken)
        .map(|(lane, taken)| {
            if lane.straight {
                std::borrow::Cow::Borrowed(&taken[..])
            } else {
                std::borrow::Cow::Owned(fixed_samples(lane))
            }
        })
        .collect();
    let mut near = vec![false; lanes.len()];
    for (k, j) in near_pairs(&fixed, widths, &vec![true; lanes.len()], &Default::default()) {
        near[k] = true;
        near[j] = true;
    }
    near.iter().filter(|&&n| n).count()
}

/// The samples of curved lane `lane` that [`untangle`] takes in every
/// round, whatever its handles, each exactly as a round takes it: those
/// along its two leads, which run straight whatever its handles
/// (`path_through`) — the first lead's from where a round's are taken on,
/// and the last lead's when the curves before it reach so far that a
/// round takes them all.
///
/// The curves before the last lead start and end where they do whatever the
/// handles; only their control points move. A round's distance along a
/// curve it walks, summed from its samples, is at least the curve's chord
/// less how far its last sample falls short of its end: at most its
/// greatest speed over its samples, three times its control polygon over
/// their count, so three of its steps, or of its polygon over the
/// sampling's cap, whichever is more. A handle reaches no further than at
/// its full share, so the polygon at full handles, twice over and the
/// chord, bounds it at any share.
///
/// A round walks a curve only where its reach is finite ([`sample`]): of
/// one with a leg whose distance is not, it takes no sample and adds no
/// distance, and the chord says nothing of it. So the last lead is read
/// only of a lane no curve of which a round can pass over, at any share:
/// each curve's polygon at full handles is at most three units in the last
/// place under `f32::MAX.sqrt()`. Every leg at every share is at most that
/// polygon, in exact arithmetic. As reckoned, the polygon may read two and
/// a half units short (five roundings: a distance's three and two
/// additions); a leg's squares may round up by three quarters of a unit of
/// its length (three roundings of a square); and `f32::MAX.sqrt()` is
/// itself a quarter of a unit under the root. So far under, every leg's
/// squares are finite, its distance is at most `2⁶⁴`, and the three sum
/// to a finite reach; a float nearer, the argument does not show it. A
/// polygon that is not a number is not at most the bound, and its lane's
/// last lead is not read.
///
/// Of a curve a round walks, the bound above holds in exact arithmetic. A
/// margin is kept back from the chords' sum for the roundings between it
/// and what runs: `2·k + 7` units in its last place (`f32::EPSILON`), `k`
/// the curves. What it covers, in those units:
///
/// - **A curve's last sample's parameter**, `(n − 1) / n` as an `f32`: up
///   to `2⁻²⁵` short of it, which sets the sample back by up to three of
///   the curve's reach, its control polygon's length, times that. Where
///   the chord is half the reach or more, that is under `6·2⁻²⁵` of the
///   chord: 1.5. Where it is less, the legs before the polygon's last are
///   a quarter of the reach or more, which holds the sample a ninth of the
///   three steps nearer the end — from three samples on, below which the
///   parameter is exact: fifty times this at the cap's count, and more
///   below it.
/// - **A round's own sums**, in `f64` ([`sample`]): its offsets and
///   chords, under `2⁻⁵` where the chord is half the reach or more and
///   far within what the three steps leave over where it is less; and
///   their running sum, over at most the cap's samples a curve, `k·2⁻¹⁴`.
/// - **This condition's own arithmetic**, in `f32`, a rounding half a
///   unit: `k + 5`, of `2·k + 10` roundings. The chords' sum rounds once
///   a curve after its first, whose addition to nothing is exact, and so
///   does the three steps' (`2·k − 2`). A curve's three steps are a
///   product, off by half a unit of itself, so all of them by half a unit
///   of their sum (one). A chord is a distance (three). A round counts
///   its samples from a reach found as the polygon is (six: a distance's
///   three, two additions and a division). The margin's product and the
///   difference are one each.
///
/// Under `k·(1 + 2⁻¹⁴) + 6.6` in all: `2·k + 7` is each of the two
/// rounded up to a whole number, and holds for any number of curves. No
/// term is kept for the coordinates' own rounding, as one was while a
/// round summed its distance between its points (until #356): measured in
/// each curve's own frame, the distance reads no point's coordinates.
///
/// What the tests hold of this, and what they do not:
/// - Lanes near the length where the condition decides need more than the
///   chord alone
///   (`test_projection_a_lanes_fixed_samples_are_in_every_round`).
/// - A lane of one cubic whose first three control points meet, which
///   moves fastest into its end, needs the three steps and the margin both
///   (`test_projection_fixed_samples_keeps_its_rounding_margin_back`).
/// - A lane of many curves, each chord rounding the `f32` sum up as it
///   joins it, needs the margin's part that grows with the curves
///   (`test_projection_fixed_samples_keeps_a_margin_for_each_curve`). That
///   holds that the part is needed. Its coefficient, and the constant's
///   size, are upper bounds by the argument above, which no test reaches.
/// - A lane with a leg too long for a finite distance has no last lead
///   read, and a junction of two such answers as all its rounds do
///   (`test_projection_fixed_samples_reads_no_last_lead_past_a_skipped_curve`,
///   `test_projection_untangle_answers_as_its_rounds_beside_a_skipped_curve`).
/// - The bound on the polygon is held from both sides: a lane whose
///   polygon is the bound, which every round walks, has its last lead
///   read, and one whose polygon reads `2⁶⁴`, seven floats on, which the
///   rounds at full and at half handles pass over, has not
///   (`test_projection_fixed_samples_holds_its_bound_on_a_curves_polygon`).
///   The six floats between rest on the argument above.
/// - That a polygon that is not a number is refused by the comparison
///   itself, no test can show. Such a polygon has a control point that is
///   not finite; the curve's ends as [`Curve::point`] reckons them, each
///   control point times its weight, are then not numbers, nor is the
///   chord, and the condition fails on the chord however the polygon is
///   compared.
///
/// Each errs toward the last lead not being read: a lane that loses its
/// last lead's samples so is only measured for more rounds, while one that
/// keeps a sample some round does not take would change what is drawn.
/// O(the leads' samples, and the curves).
pub(crate) fn fixed_samples(lane: &Lane) -> Vec<Vec2> {
    let (step, skip) = (0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width));
    let curves = path_through(lane.start, lane.end, &lane.via, 1.0);
    let mut fixed = Vec::new();
    if let Some(first @ Curve::Line(..)) = curves.first() {
        fixed.extend(sample(&[*first], step, skip));
    }
    let Some((last @ Curve::Line(..), before)) = curves.split_last() else {
        return fixed;
    };
    let step = step.max(1e-2);
    // The longest polygon no leg of which, at any share, a round can find
    // too long for a finite distance.
    let walked = (1.0 - 3.0 * f32::EPSILON) * f32::MAX.sqrt();
    // Per curve before the last lead: its chord, and how far a round's
    // sum along it can fall short of that.
    let (mut chord, mut short) = (0.0_f32, 0.0_f32);
    for curve in before {
        let (a, d) = (curve.point(0.0), curve.point(1.0));
        let polygon = match *curve {
            Curve::Cubic(_, b, c, _) => 2.0 * (a.distance(b) + c.distance(d)) + a.distance(d),
            _ => a.distance(d),
        };
        // At most: a polygon that is not a number is not walked either.
        let walked_at_every_share = polygon <= walked;
        if !walked_at_every_share {
            return fixed;
        }
        chord += a.distance(d);
        short += 3.0 * step.max(polygon / SAMPLE_CAP as f32);
    }
    let rounding = (2.0 * before.len() as f32 + 7.0) * f32::EPSILON;
    if !before.is_empty() && (1.0 - rounding) * chord - short >= skip {
        fixed.extend(sample(&[*last], step, 0.0));
    }
    fixed
}

/// The most points [`sample`] takes along one curve.
const SAMPLE_CAP: usize = 1 << 16;

/// Points along `curves` at most `step` apart, from `skip` along on. A
/// curve whose reach is not finite — a leg of its control polygon past
/// the square root of the largest `f32`, or a control point that is not
/// finite — gives no points and adds no distance.
///
/// How far along a point lies is measured in its curve's own frame
/// ([`Local`]): the chords between its samples' offsets from the curve's
/// start, in `f64`, summed in `f64` (#356). Summed between the points
/// themselves, in coordinates as large as where the curve is, it counted
/// their rounding as distance: far from the origin `skip` was reached
/// early, and the samples kept moved with where the junction lies, not
/// with its shape. The points are each curve's own, as before, so a
/// sample is the same point, to the bit, in every call that takes it
/// ([`fixed_samples`]).
///
/// The bound: a curve's `n` samples are its points at `t = i / n` (each an
/// `f32`), `i` from 0 below `n`, and the distance it adds is the polyline
/// through them, from its start to its last sample — the chord on to its
/// end is not walked, as it was not before. Against `L`, that polyline's
/// length in exact arithmetic, it is within `(n + 1)·2⁻⁴⁷·(R + L)`, `R`
/// its control points' reach from its start — each offset a few `f64`
/// roundings of `R`, each chord and partial sum one of their own — at
/// most [`SAMPLE_CAP`] samples, so just over `2⁻³¹·(R + L)`; and `along`
/// is never rounded to `f32`. `connector_tests`' agreement test holds it,
/// wherever the curve lies. That is a curve sampled alone, or the first of
/// `curves`: a later one's partial sums are of the path so far, and round
/// at its size — `2⁻⁵³` of the distance along, a sample.
pub(crate) fn sample(curves: &[Curve], step: f32, skip: f32) -> Vec<Vec2> {
    let step = step.max(1e-2);
    let mut out = Vec::new();
    let skip = f64::from(skip);
    let mut along = 0.0_f64;
    for curve in curves {
        let (a, d) = (curve.point(0.0), curve.point(1.0));
        let reach = match *curve {
            Curve::Cubic(_, b, c, _) => a.distance(b) + b.distance(c) + c.distance(d),
            Curve::Quadratic(_, b, _) => a.distance(b) + b.distance(d),
            Curve::Line(..) => a.distance(d),
        };
        if !reach.is_finite() {
            continue;
        }
        let n = ((reach / step).ceil() as usize).clamp(1, SAMPLE_CAP);
        let (_, local) = Local::of(*curve);
        let mut last = DVec2::ZERO;
        for i in 0..n {
            let t = i as f32 / n as f32;
            let here = local.at(t);
            along += (here - last).length();
            last = here;
            if along >= skip {
                out.push(curve.point(t));
            }
        }
    }
    out
}

/// A cell of a [`NearGrid`], by its column and row.
type CellKey = (i64, i64);

/// The cell of `side` that holds `p`.
fn cell_of(p: Vec2, side: f32) -> CellKey {
    ((p.x / side).floor() as i64, (p.y / side).floor() as i64)
}

/// `points` in cells of `side`, consecutive points in one cell, as
/// samples a fraction of a cell apart mostly are, put in with one lookup
/// between them: each run of them passed to `put` with its cell. O(points).
fn runs(points: &[Vec2], side: f32, mut put: impl FnMut(CellKey, &[Vec2])) {
    let mut from = 0;
    for k in 1..=points.len() {
        let at = cell_of(points[from], side);
        if k == points.len() || cell_of(points[k], side) != at {
            put(at, &points[from..k]);
            from = k;
        }
    }
}

/// Every lane's points of a junction, in **coarse** cells as wide as the
/// widest distance two lanes must keep ([`CLEARANCE`] times the widest
/// width), each holding its points by lane: the lanes that may come near a
/// point are those with points in the nine coarse cells round it. It is
/// kept from round to round of [`untangle`], a lane's points taken out and
/// put back as its curve changes.
///
/// Where two lanes meet in a coarse cell, the thinner one's points there
/// are measured against the wider one's in the nine **fine** cells round
/// each: cells of the wider lane's own distance, [`CLEARANCE`] times its
/// width, which is the distance the pair must keep. A lane sampled every
/// quarter of its width along a straight line puts at most six points in
/// a fine cell of its own each time it crosses one along an axis, and at
/// most eight across a diagonal; where a curve slows its samples, even in
/// its parameter, bunch closer, and a cell holds more. So a point is
/// measured against the few points each lane near it has there, however
/// much thinner it is than the widest lane of the junction, and against
/// no lane that is not near it. A lane's fine cells are filled the first
/// time a pair needs them, and emptied when its points change. Measured
/// in the coarse cells alone, a lane a twenty-fourth as wide as another
/// measured each of its points against hundreds of its own, and of every
/// lane beside it (#346).
struct NearGrid<'a> {
    widths: &'a [f32],
    /// The side of a coarse cell.
    cell: f32,
    /// Each coarse cell's points, by lane.
    cells: FxHashMap<CellKey, FxHashMap<usize, Vec<Vec2>>>,
    /// Each lane's points in fine cells of its own, once a pair has
    /// needed them since its points last changed.
    fine: Vec<std::cell::OnceCell<FxHashMap<CellKey, Vec<Vec2>>>>,
}

impl<'a> NearGrid<'a> {
    /// An empty grid for lanes of `widths`. O(the lanes).
    fn new(widths: &'a [f32]) -> Self {
        let widest = widths.iter().copied().fold(0.0_f32, f32::max);
        NearGrid {
            widths,
            cell: (CLEARANCE * widest).max(1e-3),
            cells: Default::default(),
            fine: vec![Default::default(); widths.len()],
        }
    }

    /// Put lane `lane`'s `points` in. O(points).
    fn insert(&mut self, lane: usize, points: &[Vec2]) {
        let cells = &mut self.cells;
        runs(points, self.cell, |at, run| {
            cells
                .entry(at)
                .or_default()
                .entry(lane)
                .or_default()
                .extend_from_slice(run);
        });
        self.fine[lane] = Default::default();
    }

    /// Take lane `lane`'s points, `points`, out again. O(points and the
    /// cells they lie in).
    fn remove(&mut self, lane: usize, points: &[Vec2]) {
        let cells = &mut self.cells;
        runs(points, self.cell, |at, _| {
            if let Some(cell) = cells.get_mut(&at) {
                cell.remove(&lane);
                if cell.is_empty() {
                    cells.remove(&at);
                }
            }
        });
        self.fine[lane] = Default::default();
    }

    /// The pairs of lanes, each `(lower, higher)`, whose points come
    /// nearer each other than [`CLEARANCE`] times the wider of their
    /// widths, the grid holding every lane's `samples`: of the pairs with
    /// a `fresh` lane, those measured now; of the pairs of two lanes not
    /// fresh, those in `before`, whose points have not moved since it was
    /// measured.
    ///
    /// Cost: per fresh lane, each of its coarse cells once, the lanes in
    /// the nine round it, and per such lane the thinner one's points there
    /// ([`Self::near_at`]); one stamp a lane.
    fn near_pairs(
        &self,
        samples: &[impl AsRef<[Vec2]>],
        fresh: &[bool],
        before: &rustc_hash::FxHashSet<(usize, usize)>,
    ) -> rustc_hash::FxHashSet<(usize, usize)> {
        let mut pairs: rustc_hash::FxHashSet<(usize, usize)> = before
            .iter()
            .copied()
            .filter(|&(k, j)| !fresh[k] && !fresh[j])
            .collect();
        // The fresh lane each lane was last found near: a pair carried from
        // `before` is of two lanes not fresh, so never met in the loop
        // below, and a pair of two fresh lanes is measured from the lower
        // one, nearness being symmetric. One stamp a lane, not a table of
        // pairs.
        let mut seen = vec![usize::MAX; samples.len()];
        #[cfg(test)]
        UNTANGLE_SCRATCH.with(|count| count.set(count.get() + seen.len()));
        // The cell each lane was last listed for, by a count of the cells:
        // a lane is listed once a cell in one step, not by a search of the
        // list.
        let (mut listed, mut stamp) = (vec![0_usize; samples.len()], 0);
        for (k, points) in samples
            .iter()
            .map(AsRef::as_ref)
            .enumerate()
            .filter(|&(k, _)| fresh[k])
        {
            let mut visited: FxHashSet<CellKey> = FxHashSet::default();
            // The cells of a thinner lane's points measured against `k`'s
            // already, each once.
            let mut measured: FxHashSet<(usize, CellKey)> = FxHashSet::default();
            let mut lanes: Vec<usize> = Vec::new();
            runs(points, self.cell, |at, _| {
                if !visited.insert(at) {
                    return;
                }
                // The lanes that may come near `k`'s points in `at`: those
                // with points in the nine cells round it.
                lanes.clear();
                stamp += 1;
                for (x, y) in around(at) {
                    for &j in self.cells.get(&(x, y)).into_iter().flat_map(|held| held.keys()) {
                        #[cfg(test)]
                        UNTANGLE_LISTED.with(|count| count.set(count.get() + 1));
                        if j != k && !(fresh[j] && j < k) && seen[j] != k && listed[j] != stamp {
                            listed[j] = stamp;
                            lanes.push(j);
                        }
                    }
                }
                for &j in &lanes {
                    if self.near_at(samples, at, k, j, &mut measured) {
                        seen[j] = k;
                        pairs.insert((k.min(j), k.max(j)));
                    }
                }
            });
        }
        pairs
    }

    /// Whether fresh lane `k`'s points in coarse cell `at` come nearer
    /// lane `j`'s than [`CLEARANCE`] times the wider of their widths: the
    /// thinner lane's points — `k`'s in `at`, or `j`'s in the nine cells
    /// round it but those `measured` against `k` already, which it adds
    /// to — each against the wider lane's points in the nine fine cells
    /// round it, consecutive points in one fine cell looking them up once.
    /// O(those points and the points in each one's fine cells, and the
    /// wider lane's points if its fine cells are not yet filled).
    fn near_at(
        &self,
        samples: &[impl AsRef<[Vec2]>],
        at: CellKey,
        k: usize,
        j: usize,
        measured: &mut FxHashSet<(usize, CellKey)>,
    ) -> bool {
        let need = CLEARANCE * self.widths[k].max(self.widths[j]);
        // The wider lane, whose fine cell is at least `need`: on equal
        // widths either, and of a width that is not a number the other.
        let wider = if self.widths[k].max(self.widths[j]) == self.widths[j] {
            j
        } else {
            k
        };
        let side = (CLEARANCE * self.widths[wider]).max(1e-3);
        let fine = self.fine[wider].get_or_init(|| {
            let mut fine: FxHashMap<CellKey, Vec<Vec2>> = FxHashMap::default();
            runs(samples[wider].as_ref(), side, |at, run| {
                fine.entry(at).or_default().extend_from_slice(run);
            });
            fine
        });
        let mut nearby: (Option<CellKey>, Vec<&Vec<Vec2>>) = (None, Vec::with_capacity(9));
        let mut close = |p: Vec2| {
            #[cfg(test)]
            UNTANGLE_CLOSE.with(|count| count.set(count.get() + 1));
            let cell = cell_of(p, side);
            if nearby.0 != Some(cell) {
                nearby.0 = Some(cell);
                nearby.1.clear();
                nearby.1.extend(around(cell).filter_map(|c| fine.get(&c)));
            }
            nearby.1.iter().copied().flatten().any(|q| {
                #[cfg(test)]
                UNTANGLE_MEASURED.with(|count| count.set(count.get() + 1));
                q.distance(p) < need
            })
        };
        let points = |lane: usize, cell: CellKey| self.cells.get(&cell).and_then(|held| held.get(&lane));
        if wider == j {
            points(k, at).into_iter().flatten().any(|&p| close(p))
        } else {
            around(at)
                .filter(|&cell| measured.insert((j, cell)))
                .filter_map(|cell| points(j, cell))
                .flatten()
                .any(|&p| close(p))
        }
    }
}

/// The nine cells round `at`, `at` among them. At the largest index or
/// the least, where [`cell_of`] holds every point further out, no cell
/// lies beyond: `at` stands for it, and is given more than once.
fn around((x, y): CellKey) -> impl Iterator<Item = CellKey> {
    (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| (x.saturating_add(dx), y.saturating_add(dy))))
}

/// The pairs of lanes, each `(lower, higher)`, whose points `samples`
/// come nearer each other than [`CLEARANCE`] times the wider of their
/// `widths`: of the pairs with a `fresh` lane, those measured now; of the
/// pairs of two lanes not fresh, those in `before` ([`NearGrid`], built
/// for the one question).
///
/// Cost: O(the points) to grid them, and what [`NearGrid::near_pairs`]
/// costs.
pub(crate) fn near_pairs(
    samples: &[impl AsRef<[Vec2]>],
    widths: &[f32],
    fresh: &[bool],
    before: &rustc_hash::FxHashSet<(usize, usize)>,
) -> rustc_hash::FxHashSet<(usize, usize)> {
    let mut grid = NearGrid::new(widths);
    for (k, points) in samples.iter().enumerate() {
        grid.insert(k, points.as_ref());
    }
    grid.near_pairs(samples, fresh, before)
}

#[cfg(any(test, fuzzing))]
thread_local! {
    /// The fuzz harness's ceiling on the samples [`untangle`] takes on
    /// this thread, and how many are left of it: `None`, as it is unless
    /// a harness sets it, for no ceiling. A lane whose samples would take
    /// more than is left is not gridded, and [`untangle`] grids no
    /// further lane, takes no further round, and sets [`UNTANGLE_CUT`]:
    /// so it samples at most the ceiling and one lane. Test and fuzz
    /// builds only (`cfg(test)`, and the `cfg(fuzzing)` `cargo fuzz`
    /// builds with): a shipped build has no ceiling and reads none.
    pub(crate) static UNTANGLE_CEILING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Whether [`UNTANGLE_CEILING`] has cut an [`untangle`] short on this
    /// thread since it was last cleared: what was laid out then is not
    /// what the map lays out. Test and fuzz builds only.
    pub(crate) static UNTANGLE_CUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether [`UNTANGLE_CEILING`] leaves room for a lane's `points`
/// samples, taking them from it; when it does not, it is spent and
/// [`UNTANGLE_CUT`] set. Test and fuzz builds only. O(1).
#[cfg(any(test, fuzzing))]
fn within_ceiling(points: usize) -> bool {
    UNTANGLE_CEILING.with(|ceiling| match ceiling.get() {
        None => true,
        Some(left) if points <= left => {
            ceiling.set(Some(left - points));
            true
        }
        Some(_) => {
            ceiling.set(Some(0));
            UNTANGLE_CUT.with(|cut| cut.set(true));
            false
        }
    })
}

#[cfg(test)]
thread_local! {
    /// Test counter: the rounds of [`untangle`] on this thread, each
    /// measuring a junction's lanes ([`NearGrid::near_pairs`]).
    pub(crate) static UNTANGLE_GRIDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the samples [`untangle`]'s rounds have put in their
    /// grid on this thread: every lane's once, and after that a lane's
    /// again only when its curve changed.
    pub(crate) static UNTANGLE_GRIDDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the samples [`untangle`] has taken on this thread,
    /// gridded or not.
    pub(crate) static UNTANGLE_SAMPLED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the entries of the tables [`NearGrid::near_pairs`]
    /// has allocated on this thread to know which pairs it has found.
    pub(crate) static UNTANGLE_SCRATCH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the points [`NearGrid::near_pairs`] has measured
    /// another point against on this thread.
    pub(crate) static UNTANGLE_MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the points [`NearGrid::near_pairs`] has measured
    /// against another lane on this thread, each time it did.
    pub(crate) static UNTANGLE_CLOSE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the steps [`NearGrid::near_pairs`] has taken on this
    /// thread listing the lanes near a cell, one for each lane it read in
    /// the nine cells round it.
    pub(crate) static UNTANGLE_LISTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the times [`portal_at`] has had to solve for an
    /// offset on this thread, its first estimate drawing the end off the
    /// point it read ([`solve_offset`]).
    pub(crate) static PORTAL_SOLVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `size` in points, for shaping: a relative size — a fraction of the
/// node's shorter side (D10) — times `min(w, h)` of the node's drawn
/// rectangle (#241 amendment 1). Either is clamped to the shapeable
/// range, `MIN_FONT_SIZE_PT` to `MAX_FONT_SIZE_PT`: a stored relative
/// size has no meaningful floor and may be as small as
/// `f32::MIN_POSITIVE` after repeated scaling, and the stored value stays
/// as it was written (the R14.10 note on #241). O(1).
pub(crate) fn points(size: FontSize, rect: Rect) -> f32 {
    let raw = match size {
        FontSize::Pt(points) => points,
        FontSize::Rel(fraction) => fraction * rect.size.x.min(rect.size.y),
    };
    clamp_font_metric(raw, MIN_FONT_SIZE_PT)
}

/// The unit normal to the left of travel along `tangent` — up, on the
/// screen, for a path running right — where a label's positive offset
/// moves it. `(0, -1)` for a tangent of no length. O(1).
pub(crate) fn left_normal(tangent: Vec2) -> Vec2 {
    Vec2::new(tangent.y, -tangent.x)
        .try_normalize()
        .unwrap_or(Vec2::new(0.0, -1.0))
}

/// Where a path's label centers (#241 §2): at `t` — a fraction of the
/// curve's arc length, held to `0..=1`, `0.5` when not a number — moved
/// `offset` along the curve's left normal there. Cost: one arc-length
/// table of the curve (none for a line) and one lookup.
pub(crate) fn label_center(path: &[Curve], t: f32, offset: f32) -> Vec2 {
    let arc = ArcPath::open(path, POINT_TABLE_SPACING);
    let t = if t.is_nan() { 0.5 } else { t.clamp(0.0, 1.0) };
    let (point, tangent) = arc.at(t * arc.length());
    let offset = if offset.is_finite() { offset } else { 0.0 };
    point + left_normal(tangent) * offset
}

/// Where along `outline`'s perimeter a portal end sits (#241 §2), as an
/// arc length from the start of its top side, clockwise: at `border`, a
/// fraction of the perimeter, or for [`BorderPoint::Auto`] facing
/// `toward`, the other end's center — where the ray from the outline's
/// center through it leaves the outline — and the perimeter's length.
/// Cost: one arc-length walk of the outline (a table for an ellipse), and
/// for `Auto` a nearest-point search.
pub(crate) fn portal_along(outline: &Outline, border: BorderPoint, toward: Vec2) -> (f32, f32) {
    let arc = portal_arc(outline);
    let s = match border {
        BorderPoint::At(t) => {
            let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
            t * arc.length()
        }
        BorderPoint::Auto => arc.nearest(ray_exit(outline, toward)),
    };
    (s, arc.length())
}

/// The point `s` along `outline`'s perimeter ([`portal_along`]), moved
/// `offset` along the outline's outward normal there, and that normal.
/// Cost: one arc-length walk of the outline and a lookup.
pub(crate) fn portal_point(outline: &Outline, s: f32, offset: f32) -> (Vec2, Vec2) {
    let offset = if offset.is_finite() { offset } else { 0.0 };
    let point = portal_arc(outline).offset_at(s, offset);
    (point.point, point.normal)
}

/// The inverse of [`label_center`]: the label place nearest `point` on
/// `path` — the fraction of the curve's arc length at the point of it
/// nearest `point`, and `point`'s distance from there along the curve's
/// left normal, which is the offset that moves a label's center onto
/// `point` there. `(0.5, 0)` for a path of no length, the place
/// [`label_center`] reads a `t` that is not a number as. What a label
/// drag (V1-C7) writes for the pointer.
///
/// Cost: one arc-length table of the curve (none for a line) and one
/// nearest-point search ([`ArcPath::nearest`]).
pub(crate) fn label_at(path: &[Curve], point: Vec2) -> (f32, f32) {
    let arc = ArcPath::open(path, POINT_TABLE_SPACING);
    let length = arc.length();
    if length.is_nan() || length <= 0.0 || !point.is_finite() {
        return (0.5, 0.0);
    }
    let s = arc.nearest(point);
    let (on, tangent) = arc.at(s);
    (
        (s / length).clamp(0.0, 1.0),
        (point - on).dot(left_normal(tangent)),
    )
}

/// The inverse of where the projection draws a portal end whose
/// `border_t` is set ([`portal_along`], [`portal_point`]): the
/// `(border_t, offset)` — a fraction of the perimeter from the start of
/// the top side, clockwise, and a distance out along the outward normal
/// there — that centers what a drag holds on `point`, when the projection
/// centers it `reach(normal)` further out than the end's place on the
/// offset curve, for that place's normal (`Projector::portal_place_at`:
/// the marker's reach, or for its label the marker's twice, the label's
/// offset and its reach). What a portal-end drag (V1-C7) writes.
///
/// The outside of the outline is of two kinds. Beside a side or a bend
/// the place is where the normal through `point` meets the outline, and
/// the offset is how far out `point` is along it, less the reach. In a
/// **sharp corner's wedge** — between its two sides' normals, where every
/// point's nearest place on the outline is the corner itself — the
/// offset curve sweeps round the corner at the offset's distance
/// ([`ArcPath::offset_at`]), its normal turning from the side before's to
/// the side after's: there the normal is the direction from the corner
/// to `point`, and the offset its distance from the corner, less the
/// reach for that normal. Either way the place is then found on the
/// **offset curve** at that offset — the place whose drawn point is
/// nearest `point` ([`nearest_drawn`]) — not on the outline: within a
/// corner's sweep the two differ, by as much as the sweep is long, and
/// the place on the outline would draw the end round the corner from the
/// pointer; at a corner itself with no offset, where the outline has two
/// normals, it is the side whose normal draws the end at `point`.
///
/// Nearer the outline than the reach, or inside it, the offset is inward,
/// and at a sharp corner the offset curve folds onto the corner of the
/// curve inside, its normal turning there while its point stands, so the
/// first estimate may draw the end off `point`. The offset is then solved
/// for: as it grows the drawn curve moves out, so `point`'s distance out
/// from where the end is drawn nearest it, along the normal there, falls
/// as the offset grows — bracketed from the estimate by doubling steps
/// and bisected ([`solve_offset`]), the place found again at each.
/// Outside a fold the first estimate is exact and nothing is solved.
///
/// `(0, 0)` for an outline of no length, and for a `point` that is not
/// finite; a `reach` that is not finite is none.
///
/// Cost: one arc-length walk of the outline (a table for an ellipse), one
/// nearest-point search of it and one of its offset curve near there; in
/// a fold one more search a step of the solve.
pub(crate) fn portal_at(outline: &Outline, point: Vec2, reach: impl Fn(Vec2) -> f32) -> (f32, f32) {
    let arc = portal_arc(outline);
    let length = arc.length();
    if length.is_nan() || length <= 0.0 || !point.is_finite() {
        return (0.0, 0.0);
    }
    let reach = |normal: Vec2| {
        let r = reach(normal);
        if r.is_finite() {
            r
        } else {
            0.0
        }
    };
    // Where the projection draws what is held for an end at `(s, offset)`.
    let drawn = |s: f32, offset: f32| {
        let on = arc.offset_at(s, offset);
        on.point + on.normal * reach(on.normal)
    };
    let near = arc.nearest(point);
    let on = arc.offset_at(near, 0.0);
    let away = point - on.point;
    // Outside, the direction from the place to `point`: the place's own
    // normal beside a side or a bend, and in a sharp corner's wedge the
    // swept normal that reaches `point`. Inside, the place's normal.
    let normal = away
        .try_normalize()
        .filter(|&toward| toward.dot(on.normal) > -0.5)
        .unwrap_or(on.normal);
    let offset = away.dot(normal) - reach(normal);
    if !offset.is_finite() {
        return ((near / length).clamp(0.0, 1.0), 0.0);
    }
    let s = nearest_drawn(&arc, offset, point, near, &drawn);
    let (s, offset) = if drawn(s, offset).distance(point) <= REFINED {
        (s, offset)
    } else {
        solve_offset(&arc, point, offset, s, &drawn)
    };
    ((s / length).clamp(0.0, 1.0), offset)
}

/// [`portal_at`]'s solve for an offset whose first estimate `offset`, at
/// the place `s`, draws the end off `point`: the place and the offset at
/// which `point`'s distance out from the nearest drawn point, along the
/// offset curve's normal there, is none. The distance falls as the offset
/// grows; a bracket is found from `offset` by steps that double, then
/// bisected. Of every place and offset it tried, the one that draws the
/// end nearest `point`: where no offset draws it at `point` — an end
/// pulled into an outline thinner than twice the offset, whose offset
/// curve inward folds right over — the end is drawn as near as it comes.
///
/// Cost: one [`nearest_drawn`] a step, at most [`REFINE_ROUNDS`] steps of
/// each kind.
fn solve_offset(
    arc: &ArcPath,
    point: Vec2,
    offset: f32,
    s: f32,
    drawn: &impl Fn(f32, f32) -> Vec2,
) -> (f32, f32) {
    #[cfg(test)]
    PORTAL_SOLVES.with(|count| count.set(count.get() + 1));
    // `point`'s distance out from the end drawn nearest it at `offset`,
    // and the place it is drawn at.
    let out = |offset: f32, near: f32| {
        let s = nearest_drawn(arc, offset, point, near, drawn);
        let normal = arc.offset_at(s, offset).normal;
        let miss = point - drawn(s, offset);
        (miss.dot(normal), s, miss.length())
    };
    let (first, mut near, missed) = out(offset, s);
    let mut best = (missed, near, offset);
    let outward = first > 0.0;
    let (mut inner, mut outer) = (offset, offset);
    let mut step = first.abs().max(1.0);
    let mut bracketed = false;
    for _ in 0..REFINE_ROUNDS {
        let next = if outward { outer + step } else { inner - step };
        let (d, at, missed) = out(next, near);
        near = at;
        if missed < best.0 {
            best = (missed, at, next);
        }
        if outward {
            inner = outer;
            outer = next;
        } else {
            outer = inner;
            inner = next;
        }
        if (d > 0.0) != outward || d == 0.0 {
            bracketed = true;
            break;
        }
        step *= 2.0;
    }
    if bracketed {
        for _ in 0..REFINE_ROUNDS {
            let middle = 0.5 * (inner + outer);
            let (d, at, missed) = out(middle, near);
            near = at;
            if missed < best.0 {
                best = (missed, at, middle);
            }
            if missed <= REFINED {
                break;
            }
            if d > 0.0 {
                inner = middle;
            } else {
                outer = middle;
            }
        }
    }
    (best.1, best.2)
}

/// How far, in canvas units, the end [`portal_at`] reads may be drawn
/// from the pointer before it solves for the offset.
const REFINED: f32 = 1e-3;

/// How many steps of each kind [`solve_offset`] takes at most: to bracket
/// the offset, then to bisect the bracket.
const REFINE_ROUNDS: usize = 24;

/// How many places either side of where it starts [`nearest_drawn`]
/// reads before it narrows in.
const OFFSET_SAMPLES: usize = 32;

/// How many golden-section rounds [`nearest_drawn`] narrows in by.
const OFFSET_ROUNDS: usize = 34;

/// The place along `arc` at which `drawn` — where the projection draws
/// what a portal end at that place and `offset` holds — is nearest
/// `target`, searched within reach of `near`: a corner's sweep outward
/// reaches `|offset|·|turn| / 2` either side of the corner, and a fold
/// inward `|offset|·tan(|turn| / 2)` beyond its bend, so for a convex
/// outline's turns (each under half a turn) the place lies within
/// `2·|offset|` of `near`, which is within the sweep or the fold — and at
/// most half the perimeter either way. A sampling, then a golden-section
/// search between the best sample's neighbors.
///
/// Cost: twice [`OFFSET_SAMPLES`] and twice [`OFFSET_ROUNDS`] of `drawn`,
/// each one offset-curve lookup.
fn nearest_drawn(
    arc: &ArcPath,
    offset: f32,
    target: Vec2,
    near: f32,
    drawn: &impl Fn(f32, f32) -> Vec2,
) -> f32 {
    let length = arc.length();
    let window = (2.0 * offset.abs() + 1.0).min(0.5 * length);
    let distance = |s: f32| (drawn(s, offset) - target).length();
    let step = window / OFFSET_SAMPLES as f32;
    let mut best = (distance(near), near);
    for k in 0..=2 * OFFSET_SAMPLES {
        let s = near - window + step * k as f32;
        let d = distance(s);
        if d < best.0 {
            best = (d, s);
        }
    }
    let (mut low, mut high) = (best.1 - step, best.1 + step);
    let ratio = 0.5 * (5.0_f32.sqrt() - 1.0);
    for _ in 0..OFFSET_ROUNDS {
        let a = high - ratio * (high - low);
        let b = low + ratio * (high - low);
        if distance(a) <= distance(b) {
            high = b;
        } else {
            low = a;
        }
    }
    let found = 0.5 * (low + high);
    // The best sample itself, when the narrowing found no nearer: a place
    // at a corner with no offset, where the drawn point jumps between
    // the two sides' normals, is a sample's.
    let found = if distance(found) <= best.0 { found } else { best.1 };
    found.rem_euclid(length)
}

/// The box a portal end's marker run draws in: its glyphs' advances by
/// its line height, centered on its place (`stages::portal_place`). O(its
/// glyphs).
pub(crate) fn marker_size(run: &crate::flow::ShapedRun) -> Vec2 {
    Vec2::new(run.glyphs.iter().map(|g| g.advance).sum(), run.line_height)
}

/// `outline` walked once clockwise from the start of its top side: what
/// a portal end's place along the perimeter is measured on.
fn portal_arc(outline: &Outline) -> ArcPath {
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    ArcPath::closed(outline, start, Direction::Clockwise, POINT_TABLE_SPACING)
}

/// Where each of `ends` — `(along, width)`: its place along one closed
/// outline `perimeter` long and how wide its marker and label are,
/// sorted by place — is held apart from the others (#296; portal ends
/// that face one way piled up on one point): no two neighbors, round the
/// outline and across its seam, nearer center to center than half their
/// widths summed, every end as near where it faced as that allows — the
/// least squares of the moves, by pool-adjacent-violators: the ends are
/// read from the widest free stretch of the outline on, each block of
/// ends that would overlap the one before it merged with it and centered
/// on where its ends faced, until none does. An end clear of the others
/// stays where it faces. A `perimeter` that is not a positive number is
/// read as an open line, with no seam. O(ends) amortized; two allocations.
pub(crate) fn held_apart(ends: &[(f32, f32)], perimeter: f32) -> Vec<f32> {
    let n = ends.len();
    let closed = perimeter.is_finite() && perimeter > 0.0;
    // Read from just past the widest free stretch, round the seam.
    let cut = if closed && n > 1 {
        let free = |k: usize| {
            let before = if k == 0 {
                ends[n - 1].0 - perimeter
            } else {
                ends[k - 1].0
            };
            ends[k].0 - before
        };
        (0..n).max_by(|&a, &b| free(a).total_cmp(&free(b))).unwrap_or(0)
    } else {
        0
    };
    let order: Vec<usize> = (0..n).map(|i| (cut + i) % n).collect();
    let faced = |i: usize| ends[order[i]].0 + if closed && order[i] < cut { perimeter } else { 0.0 };
    // Each end's least offset from the first, the ends packed tight: what
    // its place less its offset is held in order by.
    let mut offset = vec![0.0_f32; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + 0.5 * (ends[order[i - 1]].1 + ends[order[i]].1);
    }
    let mut blocks: Vec<(f32, usize)> = Vec::with_capacity(n);
    for (i, &off) in offset.iter().enumerate() {
        let mut block = (faced(i) - off, 1usize);
        while let Some(&(sum, count)) = blocks.last() {
            if sum / count as f32 > block.0 / block.1 as f32 {
                blocks.pop();
                block = (block.0 + sum, block.1 + count);
            } else {
                break;
            }
        }
        blocks.push(block);
    }
    let mut at = vec![0.0_f32; n];
    let mut i = 0;
    for (sum, count) in blocks {
        let mean = sum / count as f32;
        for _ in 0..count {
            let place = mean + offset[i];
            at[order[i]] = if closed {
                place.rem_euclid(perimeter)
            } else {
                place
            };
            i += 1;
        }
    }
    at
}

/// A path that leaves a node, as the portal ends there keep clear of it
/// (#394): its curves as its lay draws them, and `radius`, how far its
/// ink reaches either side of them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Leaving {
    /// Its curves, end to end.
    pub(crate) curves: Vec<Curve>,
    /// How far its ink reaches either side of its curves.
    pub(crate) radius: f32,
}

/// How many steps fast a piece of a curve may run before
/// [`near_points`] halves it rather than sample it.
const NEAR_SPLIT: f32 = 16.0;

/// How many times [`near_points`] halves a curve at most.
const NEAR_DEPTH: u32 = 24;

/// The most points [`near_points`] takes of one path's curves, a bound on
/// one path's cost to [`portal_walls`]: past it, what lies further along
/// its curves is not walled. A path whose pieces near a node are sampled
/// in fewer points — some `WALL_SAMPLE_CAP` steps of its length near it,
/// less what its pieces' speed bounds overstate — is walled whole.
pub(crate) const WALL_SAMPLE_CAP: usize = 1 << 12;

/// The pieces of `curve` halved at its middle (de Casteljau). O(1).
fn halves(curve: Curve) -> (Curve, Curve) {
    let mid = |a: Vec2, b: Vec2| 0.5 * (a + b);
    match curve {
        Curve::Line(a, b) => {
            let m = mid(a, b);
            (Curve::Line(a, m), Curve::Line(m, b))
        }
        Curve::Quadratic(a, c, b) => {
            let (ac, cb) = (mid(a, c), mid(c, b));
            let m = mid(ac, cb);
            (Curve::Quadratic(a, ac, m), Curve::Quadratic(m, cb, b))
        }
        Curve::Cubic(a, c1, c2, b) => {
            let (p, q, r) = (mid(a, c1), mid(c1, c2), mid(c2, b));
            let (pq, qr) = (mid(p, q), mid(q, r));
            let m = mid(pq, qr);
            (Curve::Cubic(a, p, pq, m), Curve::Cubic(m, qr, r, b))
        }
    }
}

/// Points of `curve` no further than `step` apart along it, wherever it
/// may come within `near` — a box — onto `out`, until `out` holds
/// [`WALL_SAMPLE_CAP`]: a piece whose control points' box misses `near`
/// lies outside it (a Bézier lies in its control points' hull) and is
/// passed over; one that runs more than [`NEAR_SPLIT`] steps fast is
/// halved, at most [`NEAR_DEPTH`] times; any other is sampled evenly, as
/// many points as its speed bound is steps, and its ends. A piece with a
/// control point that is not finite is passed over. O(its points, and
/// the pieces halved).
fn near_points(curve: Curve, near: Rect, step: f32, depth: u32, out: &mut Vec<Vec2>) {
    let points: &[Vec2] = match &curve {
        Curve::Line(a, b) => &[*a, *b],
        Curve::Quadratic(a, c, b) => &[*a, *c, *b],
        Curve::Cubic(a, c1, c2, b) => &[*a, *c1, *c2, *b],
    };
    if out.len() >= WALL_SAMPLE_CAP || points.iter().any(|p| !p.is_finite()) {
        return;
    }
    let (low, high) = points.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(l, h), p| (l.min(*p), h.max(*p)),
    );
    let far = near.max();
    if high.x < near.origin.x || high.y < near.origin.y || low.x > far.x || low.y > far.y {
        return;
    }
    let speed = curve.speed_bound();
    if !speed.is_finite() {
        return;
    }
    if speed > NEAR_SPLIT * step && depth > 0 {
        let (a, b) = halves(curve);
        near_points(a, near, step, depth - 1, out);
        near_points(b, near, step, depth - 1, out);
        return;
    }
    let n = ((speed / step).ceil() as usize).clamp(1, WALL_SAMPLE_CAP);
    for i in 0..=n {
        if out.len() >= WALL_SAMPLE_CAP {
            return;
        }
        out.push(curve.point(i as f32 / n as f32));
    }
}

/// The walls the paths `leaving` a node stand on its `outline` for the
/// portal ends there that reach `depth` out from it (#394), each
/// `(along, width)` as [`portal_along`] measures an end: the places along
/// the outline at which an end would meet a path's ink — in the measure
/// [`held_apart`] holds ends apart by, a length along the outline. Each
/// path is sampled wherever it may come within its reach of the band the
/// ends take, from the outline out to `depth` ([`near_points`], a step
/// its ink's radius apart, at least [`WALL_STEP`]); each sample in that
/// band, by its distance past the outline at the place nearest it, walls
/// off that place, either way by the ink's radius and by `step / √2`:
/// two samples a step apart have nearest places on the convex outline no
/// more than a step apart, and so no more than √2 steps apart along it,
/// round a corner of at most a right angle — half of that each, so the
/// ink between two samples is walled too. The walls overlap:
/// [`held_clear`] merges them.
///
/// Cost: one arc-length walk of the outline; for each path, its pieces
/// near the band halved and sampled, at most [`WALL_SAMPLE_CAP`] points,
/// and a nearest-point search of the outline for each.
pub(crate) fn portal_walls(outline: &Outline, leaving: &[Leaving], depth: f32) -> Vec<(f32, f32)> {
    if leaving.is_empty() {
        return Vec::new();
    }
    let arc = portal_arc(outline);
    let depth = if depth.is_finite() { depth.max(0.0) } else { 0.0 };
    let rect = outline.rect();
    let mut walls = Vec::new();
    let mut points = Vec::new();
    for path in leaving {
        let radius = if path.radius.is_finite() {
            path.radius.max(0.0)
        } else {
            0.0
        };
        let step = radius.max(WALL_STEP);
        let pad = radius + step * std::f32::consts::FRAC_1_SQRT_2;
        let reach = depth + pad;
        let near = Rect::new(
            rect.origin - Vec2::splat(reach),
            rect.size + Vec2::splat(2.0 * reach),
        );
        points.clear();
        for curve in &path.curves {
            near_points(*curve, near, step, NEAR_DEPTH, &mut points);
        }
        for &q in &points {
            let s = arc.nearest(q);
            let at = arc.offset_at(s, 0.0);
            let past = (q - at.point).dot(at.normal);
            if (-pad..=reach).contains(&past) {
                walls.push((s, 2.0 * pad));
            }
        }
    }
    walls
}

/// The least step [`portal_walls`] samples a path at: a path whose ink
/// has no radius is still walled at this pitch.
pub(crate) const WALL_STEP: f32 = 1.0;

/// [`held_apart`], clear of `walls` (#394): where each of `ends` —
/// `(along, width)` round a closed outline `perimeter` long — sits, no
/// end nearer a wall's edge than half its own width, where the outline
/// has room for it; each wall `(along, width)` is the stretch of the
/// outline a path leaving the node covers ([`portal_walls`]).
///
/// The walls, merged where they meet, cut the outline into free
/// stretches. Each end goes to the stretch it faces into, or, facing
/// into a wall, to the stretch past the nearer of that wall's edges —
/// the later one, clockwise, on a tie. A stretch its ends' widths summed
/// overflow sheds them one at a time, greedily, not as a least squares:
/// its first end counterclockwise or its last clockwise, whichever is
/// nearer the free stretch past the wall it crosses (clockwise on a tie),
/// each carried on in that direction to the first stretch with room left
/// for it whole, past any without — so an end may cross the paths and the
/// ends between. Where no stretch has room for it whole, it goes to the
/// stretch it overflows least, the first of those, if that overflows by
/// less than taking it off its own stretch spares, and stays otherwise —
/// every move lowers the overflow summed over the stretches. A stretch left overflowing keeps its ends
/// packed tight, centered on it and over both its walls by as much: the
/// only way an end meets a wall. The stretches are read clockwise from
/// the one past the first wall from the top side's start on. Within
/// each stretch its ends are held apart as [`held_apart`] holds them on
/// an open line, then moved in from its walls: the least squares of the
/// moves, there, for ends kept in their order between two walls (an
/// isotonic fit bounded alike at every end is the unbounded one,
/// clamped).
///
/// With no walls, or none that leave the outline any room, or a
/// `perimeter` that is not a positive number, it is [`held_apart`]
/// itself, the walls set aside. O((ends + stretches) × stretches) at
/// worst — each shed, and each way found stuck, walks the stretches once
/// round — and O(ends log ends + walls log walls) when no stretch
/// overflows; a few allocations.
pub(crate) fn held_clear(ends: &[(f32, f32)], walls: &[(f32, f32)], perimeter: f32) -> Vec<f32> {
    let closed = perimeter.is_finite() && perimeter > 0.0;
    let mut arcs: Vec<(f32, f32)> = walls
        .iter()
        .filter(|w| w.0.is_finite() && w.1.is_finite() && w.1 > 0.0)
        .map(|&(center, width)| {
            let start = (center - 0.5 * width).rem_euclid(perimeter);
            (start, start + width)
        })
        .collect();
    if !closed || arcs.is_empty() || ends.is_empty() {
        return held_apart(ends, perimeter);
    }
    // The walls merged where they meet, round the seam too: each a
    // stretch of the outline from its start, in `0..perimeter`, on.
    arcs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::with_capacity(arcs.len());
    for arc in arcs {
        match merged.last_mut() {
            Some(last) if arc.0 <= last.1 => last.1 = last.1.max(arc.1),
            _ => merged.push(arc),
        }
    }
    while merged.len() > 1 {
        let (first, last) = (merged[0], merged[merged.len() - 1]);
        if last.1 < first.0 + perimeter {
            break;
        }
        let at = merged.len() - 1;
        merged[at].1 = last.1.max(first.1 + perimeter);
        merged.remove(0);
    }
    let m = merged.len();
    if merged[0].1 - merged[0].0 >= perimeter {
        return held_apart(ends, perimeter);
    }
    // Stretch `j` runs from wall `j`'s end to wall `j + 1`'s start, round
    // the seam for the last; `step[j]` is from its start to the next's.
    let wall = |j: usize| merged[j % m].1 - merged[j % m].0;
    let lo: Vec<f32> = merged.iter().map(|w| w.1).collect();
    let len: Vec<f32> = (0..m)
        .map(|j| {
            let next = if j + 1 == m {
                merged[0].0 + perimeter
            } else {
                merged[j + 1].0
            };
            next - lo[j]
        })
        .collect();
    let step: Vec<f32> = (0..m).map(|j| len[j] + wall(j + 1)).collect();
    // Each stretch's ends: (place from its start, end); and their widths
    // summed.
    let mut held: Vec<(std::collections::VecDeque<(f32, usize)>, f32)> = vec![(Default::default(), 0.0); m];
    for (k, &(along, width)) in ends.iter().enumerate() {
        let along = if along.is_finite() {
            along.rem_euclid(perimeter)
        } else {
            0.0
        };
        let after = merged.partition_point(|w| w.0 <= along);
        let (i, along) = if after == 0 {
            (m - 1, along + perimeter)
        } else {
            (after - 1, along)
        };
        let (j, place) = if along < merged[i].1 {
            let (ccw, cw) = (along - merged[i].0, merged[i].1 - along);
            if ccw < cw {
                let j = (i + m - 1) % m;
                (j, len[j] + ccw)
            } else {
                (i, -cw)
            }
        } else {
            (i, along - merged[i].1)
        };
        held[j].0.push_back((place, k));
        held[j].1 += width;
    }
    let width = |k: usize| ends[k].1;
    for h in &mut held {
        h.0.make_contiguous()
            .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    }
    // How far the ends a stretch holds overflow it, their widths summed.
    let over = |sum: f32, len: f32| (sum - len).max(0.0);
    for j in 0..m {
        // Whether the end each way, [counterclockwise, clockwise], has
        // nowhere to go that lowers the overflow.
        let mut stuck = [false, false];
        // Not empty: the sheds may leave a rounding in the sum.
        while !held[j].0.is_empty() && held[j].1 > len[j] && !(stuck[0] && stuck[1]) {
            let (front, back) = (held[j].0[0], held[j].0[held[j].0.len() - 1]);
            let ccw_near = front.0 + wall(j);
            let cw_near = len[j] - back.0 + wall(j + 1);
            let cw = !stuck[1] && (stuck[0] || cw_near <= ccw_near);
            let (mut place, k) = if cw { back } else { front };
            let w = width(k);
            // The first stretch on with room for it whole, or else the one
            // it overflows least, the first of those.
            let (mut to, mut room, mut least) = (j, None, None::<(f32, usize, f32)>);
            loop {
                if cw {
                    place -= step[to];
                    to = (to + 1) % m;
                } else {
                    to = (to + m - 1) % m;
                    place += step[to];
                }
                if to == j {
                    break;
                }
                let (sum, l) = (held[to].1, len[to]);
                if sum + w <= l {
                    room = Some((to, place));
                    break;
                }
                let added = over(sum + w, l) - over(sum, l);
                if least.is_none_or(|(a, ..)| added < a) {
                    least = Some((added, to, place));
                }
            }
            let shed = over(held[j].1, len[j]) - over(held[j].1 - w, len[j]);
            let found = room.or(least
                .filter(|&(added, ..)| added < shed)
                .map(|(_, to, place)| (to, place)));
            let Some((to, place)) = found else {
                stuck[cw as usize] = true;
                continue;
            };
            stuck[cw as usize] = false;
            if cw {
                held[j].0.pop_back();
                held[to].0.push_front((place, k));
            } else {
                held[j].0.pop_front();
                held[to].0.push_back((place, k));
            }
            held[j].1 -= width(k);
            held[to].1 += width(k);
        }
    }
    let mut at = vec![0.0_f32; ends.len()];
    for (j, (members, _)) in held.iter().enumerate() {
        let Some(&(_, last)) = members.back() else {
            continue;
        };
        let local: Vec<(f32, f32)> = members.iter().map(|&(place, k)| (place, width(k))).collect();
        let placed = held_apart(&local, 0.0);
        let mut offset = 0.0_f32;
        let mut offsets = Vec::with_capacity(members.len());
        for (i, &(_, k)) in members.iter().enumerate() {
            if i > 0 {
                offset += 0.5 * (width(members[i - 1].1) + width(k));
            }
            offsets.push(offset);
        }
        let (low, high) = (0.5 * width(members[0].1), len[j] - 0.5 * width(last) - offset);
        for (i, &(_, k)) in members.iter().enumerate() {
            let from = placed[i] - offsets[i];
            let from = if low <= high {
                from.clamp(low, high)
            } else {
                0.5 * (low + high)
            };
            at[k] = (lo[j] + from + offsets[i]).rem_euclid(perimeter);
        }
    }
    at
}

/// Where the ray from `outline`'s center toward `toward` leaves it: on an
/// ellipse exactly; on a rounded rectangle, where it leaves the
/// rectangle, which the nearest-point search then takes onto a rounded
/// corner. Straight up when `toward` is the center or not a point. O(1).
pub(crate) fn ray_exit(outline: &Outline, toward: Vec2) -> Vec2 {
    let rect = outline.rect();
    let center = rect.center();
    let half = rect.size * 0.5;
    let d = toward - center;
    let d = if d.is_finite() && d.length_squared() > 0.0 {
        d
    } else {
        Vec2::new(0.0, -1.0)
    };
    match outline {
        Outline::Ellipse { .. } => {
            if half.x <= 0.0 || half.y <= 0.0 {
                return center;
            }
            let k = ((d.x / half.x).powi(2) + (d.y / half.y).powi(2)).sqrt();
            if k > 0.0 && k.is_finite() {
                center + d / k
            } else {
                center
            }
        }
        Outline::RoundedRect { .. } => {
            let along = |extent: f32, part: f32| {
                if part == 0.0 {
                    f32::INFINITY
                } else {
                    extent / part.abs()
                }
            };
            let t = along(half.x, d.x).min(along(half.y, d.y));
            if t.is_finite() {
                center + d * t
            } else {
                center
            }
        }
    }
}

/// How far a box of `size` reaches from its center along the unit
/// `normal`: half its extent in that direction. O(1).
pub(crate) fn reach(normal: Vec2, size: Vec2) -> f32 {
    normal.x.abs() * size.x * 0.5 + normal.y.abs() * size.y * 0.5
}

/// The hit shape of a node's body (ruling B7.1): its outline's ellipse,
/// or the rectangle — a rounded rectangle is hit over its whole
/// rectangle, as the node's area has always been. O(1).
pub(crate) fn node_shape(shape: Shape) -> NodeShape {
    match shape {
        Shape::Ellipse => NodeShape::Ellipse,
        Shape::Rectangle { .. } => NodeShape::Rectangle,
    }
}
