// SPDX-License-Identifier: MPL-2.0

//! The projection's geometry (design §5.5, #241 §2): a node's drawn
//! rectangle, where a path attaches to its ends, a ring's size in
//! points, where a label and a portal end sit. Pure functions over plain
//! values; `format/projection.md` is the spelling of every rule here.

use glam::Vec2;

use crate::document::resolve::BorderPoint;
use crate::document::values::{Anchor, Direction, Edge, FontSize, Shape, StartPoint};
use crate::flow::{ArcPath, Curve, Outline, Rect};
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

/// The turns and holds of the paths that leave one point of a side, in
/// order, each given as `(spread, clear)` (#296, rule 4): each two
/// neighbors apart by the widest spread among them, and none more than
/// [`SPLAY_CAP`] from the normal — the fan narrowed to hold it. Narrowed,
/// its first glyphs are held out from the point to where neighbors that
/// far apart clear each other: their centers the widest `clear` apart —
/// the gap and the glyph's diagonal, as the spread is reckoned, since
/// two neighbors turned off the normal stand side by side across a
/// slope — so the widest clear over twice the sine of half the step. A
/// path alone leaves along the normal, held nowhere. O(members); one
/// allocation.
pub(crate) fn fan(members: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if members.len() < 2 {
        return vec![(0.0, 0.0); members.len()];
    }
    let last = (members.len() - 1) as f32;
    let widest = members.iter().map(|m| m.0).fold(0.0_f32, f32::max);
    let step = widest.min(2.0 * SPLAY_CAP / last);
    let hold = if step < widest {
        let clear = members.iter().map(|m| m.1).fold(0.0_f32, f32::max);
        clear / (2.0 * (0.5 * step).sin())
    } else {
        0.0
    };
    let hold = if hold.is_finite() { hold } else { 0.0 };
    (0..members.len())
        .map(|rank| ((rank as f32 - 0.5 * last) * step, hold))
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
/// path that arrives straight on — and its lead, through each of `via`. Point `k` of
/// `n` is the point `k / (n + 1)` of the way along the straight line
/// from end to end, moved by `via[k - 1]`; one that is not finite moves
/// it nothing.
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
        k => p0 + (p1 - p0) * (k as f32 / (n + 1) as f32) + usable_point(via[k - 1]),
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
/// Cost: O(each lane's length over its width) samples, gridded and
/// measured, in the first round, and in each after it those of the lanes
/// it halved alone ([`NearGrid`]).
pub(crate) fn untangle(lanes: &[Lane]) -> Vec<f32> {
    let mut handles = vec![1.0_f32; lanes.len()];
    if lanes.len() < 2 {
        return handles;
    }
    let widths: Vec<f32> = lanes.iter().map(|lane| lane.width).collect();
    let lay = |lane: &Lane, h: f32| {
        let curves = if lane.straight {
            vec![Curve::Line(lane.start.0, lane.end.0)]
        } else {
            path_through(lane.start, lane.end, &lane.via, h)
        };
        sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width))
    };
    let mut samples: Vec<Vec<Vec2>> = lanes
        .iter()
        .zip(&handles)
        .map(|(lane, &h)| lay(lane, h))
        .collect();
    // A round measures again only the lanes whose curves the last one
    // changed, against every lane's points, which stay in one grid from
    // round to round: two lanes neither of which changed are as near, or
    // as clear, as they were ([`NearGrid`]).
    let mut grid = NearGrid::new(&widths);
    for (k, points) in samples.iter().enumerate() {
        grid.insert(k, points);
    }
    let mut fresh = vec![true; lanes.len()];
    let mut pairs = rustc_hash::FxHashSet::default();
    let mut best = (usize::MAX, handles.clone());
    for round in 0..UNTANGLE_ROUNDS {
        if round > 0 {
            for (k, lane) in lanes.iter().enumerate().filter(|&(k, _)| fresh[k]) {
                grid.remove(k, &samples[k]);
                samples[k] = lay(lane, handles[k]);
                grid.insert(k, &samples[k]);
            }
        }
        pairs = grid.near_pairs(&samples, &fresh, &pairs);
        let mut near = vec![false; lanes.len()];
        for &(k, j) in &pairs {
            near[k] = true;
            near[j] = true;
        }
        let count = near.iter().filter(|&&n| n).count();
        if count <= best.0 {
            best = (count, handles.clone());
        }
        if count == 0 {
            break;
        }
        for (k, h) in handles.iter_mut().enumerate() {
            let halved = if near[k] {
                (*h * 0.5).max(LEAST_HANDLES)
            } else {
                *h
            };
            // A straight lane's curve does not follow its handles.
            fresh[k] = halved != *h && !lanes[k].straight;
            *h = halved;
        }
    }
    best.1
}

/// Points along `curves` at most `step` apart, from `skip` along on.
fn sample(curves: &[Curve], step: f32, skip: f32) -> Vec<Vec2> {
    let step = step.max(1e-2);
    let mut out = Vec::new();
    let mut along = 0.0_f32;
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
        let n = ((reach / step).ceil() as usize).clamp(1, 1 << 16);
        let mut last = a;
        for i in 0..n {
            let p = curve.point(i as f32 / n as f32);
            along += p.distance(last);
            last = p;
            if along >= skip {
                out.push(p);
            }
        }
    }
    out
}

/// A cell of a [`NearGrid`], by its column and row.
type CellKey = (i64, i64);

/// The points in one cell of a [`NearGrid`], each with its lane.
type CellPoints = Vec<(usize, Vec2)>;

/// Every lane's points of a junction, in cells as wide as the widest
/// distance two lanes must keep ([`CLEARANCE`] times the wider of their
/// widths), so a point is measured to the points in the nine cells round
/// it alone. It is kept from round to round of [`untangle`], a lane's
/// points taken out and put back as its curve changes.
struct NearGrid<'a> {
    widths: &'a [f32],
    cell: f32,
    cells: rustc_hash::FxHashMap<CellKey, CellPoints>,
}

impl<'a> NearGrid<'a> {
    /// An empty grid for lanes of `widths`. O(1).
    fn new(widths: &'a [f32]) -> Self {
        let widest = widths.iter().copied().fold(0.0_f32, f32::max);
        NearGrid {
            widths,
            cell: (CLEARANCE * widest).max(1e-3),
            cells: Default::default(),
        }
    }

    fn key(&self, p: Vec2) -> CellKey {
        ((p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64)
    }

    /// Put lane `lane`'s `points` in. O(points); consecutive points in one
    /// cell, as samples a quarter of a width apart mostly are, cost one
    /// lookup between them.
    fn insert(&mut self, lane: usize, points: &[Vec2]) {
        #[cfg(test)]
        UNTANGLE_GRIDDED.with(|count| count.set(count.get() + points.len()));
        let mut run: Option<(CellKey, CellPoints)> = None;
        for &p in points {
            let at = self.key(p);
            match &mut run {
                Some((k, held)) if *k == at => held.push((lane, p)),
                _ => {
                    if let Some((k, held)) = run.take() {
                        self.cells.entry(k).or_default().extend(held);
                    }
                    run = Some((at, vec![(lane, p)]));
                }
            }
        }
        if let Some((k, held)) = run {
            self.cells.entry(k).or_default().extend(held);
        }
    }

    /// Take lane `lane`'s points, `points`, out again. O(points and the
    /// cells they lie in).
    fn remove(&mut self, lane: usize, points: &[Vec2]) {
        let mut last = None;
        for &p in points {
            let at = self.key(p);
            if last == Some(at) {
                continue;
            }
            last = Some(at);
            if let Some(cell) = self.cells.get_mut(&at) {
                cell.retain(|&(l, _)| l != lane);
            }
        }
    }

    /// The pairs of lanes, each `(lower, higher)`, whose points come
    /// nearer each other than [`CLEARANCE`] times the wider of their
    /// widths, the grid holding every lane's `samples`: of the pairs with
    /// a `fresh` lane, those measured now, from the fresh lanes' points;
    /// of the pairs of two lanes not fresh, those in `before`, whose
    /// points have not moved since it was measured.
    ///
    /// Cost: per fresh point, the points in the nine cells round it; one
    /// stamp a lane.
    fn near_pairs(
        &self,
        samples: &[Vec<Vec2>],
        fresh: &[bool],
        before: &rustc_hash::FxHashSet<(usize, usize)>,
    ) -> rustc_hash::FxHashSet<(usize, usize)> {
        #[cfg(test)]
        UNTANGLE_GRIDS.with(|count| count.set(count.get() + 1));
        let mut pairs: rustc_hash::FxHashSet<(usize, usize)> = before
            .iter()
            .copied()
            .filter(|&(k, j)| !fresh[k] && !fresh[j])
            .collect();
        // The fresh lane each lane was last found near: a pair carried
        // from `before` is of two lanes not fresh, so never met in the loop
        // below, and a pair of two fresh lanes was decided while the lower
        // one's points were measured, nearness being symmetric. One stamp
        // a lane, not a table of pairs.
        let mut seen = vec![usize::MAX; samples.len()];
        #[cfg(test)]
        UNTANGLE_SCRATCH.with(|count| count.set(count.get() + seen.len()));
        let mut around: Vec<&CellPoints> = Vec::with_capacity(9);
        for (k, points) in samples.iter().enumerate().filter(|&(k, _)| fresh[k]) {
            let mut around_of = None;
            for &p in points {
                let (x, y) = self.key(p);
                if around_of != Some((x, y)) {
                    around.clear();
                    for dx in -1..=1 {
                        for dy in -1..=1 {
                            around.extend(self.cells.get(&(x + dx, y + dy)));
                        }
                    }
                    around_of = Some((x, y));
                }
                for &(j, q) in around.iter().copied().flatten() {
                    if j == k || (fresh[j] && j < k) || seen[j] == k {
                        continue;
                    }
                    if q.distance(p) < CLEARANCE * self.widths[k].max(self.widths[j]) {
                        seen[j] = k;
                        pairs.insert((k.min(j), k.max(j)));
                    }
                }
            }
        }
        pairs
    }
}

/// The pairs of lanes, each `(lower, higher)`, whose points `samples`
/// come nearer each other than [`CLEARANCE`] times the wider of their
/// `widths`: of the pairs with a `fresh` lane, those measured now; of the
/// pairs of two lanes not fresh, those in `before` ([`NearGrid`], built
/// for the one question).
///
/// Cost: O(the points) to grid them, and per fresh point the points in
/// the nine cells round it.
#[cfg(test)]
pub(crate) fn near_pairs(
    samples: &[Vec<Vec2>],
    widths: &[f32],
    fresh: &[bool],
    before: &rustc_hash::FxHashSet<(usize, usize)>,
) -> rustc_hash::FxHashSet<(usize, usize)> {
    let mut grid = NearGrid::new(widths);
    for (k, points) in samples.iter().enumerate() {
        grid.insert(k, points);
    }
    grid.near_pairs(samples, fresh, before)
}

#[cfg(test)]
thread_local! {
    /// Test counter: the rounds of measuring a junction's lanes on this
    /// thread ([`NearGrid::near_pairs`]), one for each round of
    /// [`untangle`].
    pub(crate) static UNTANGLE_GRIDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the points put in a [`NearGrid`] on this thread:
    /// every lane's once, and after that a lane's again only when its
    /// curve changed.
    pub(crate) static UNTANGLE_GRIDDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter: the entries of the tables [`NearGrid::near_pairs`]
    /// has allocated on this thread to know which pairs it has found.
    pub(crate) static UNTANGLE_SCRATCH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
