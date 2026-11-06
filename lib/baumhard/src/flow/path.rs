// SPDX-License-Identifier: MPL-2.0

//! Arc-length parameterization: a closed outline from a start point in
//! a direction, or an open curve from its source end, as a function of
//! the distance `s` walked along it (#239 §5 step 1).
//!
//! Straight stretches and circular arcs are walked exactly. An ellipse
//! that is not a circle, and a Bézier, have no closed-form arc length,
//! so they are walked through a table: samples of the curve's own
//! parameter close enough together that no two neighbors are more than
//! `spacing` apart — placement asks for a quarter of the run's line
//! height — each paired with the distance walked to reach it. A
//! distance is turned back into a parameter by interpolating between
//! the two samples around it, and the point and direction are then
//! evaluated **on the curve itself** at that parameter, so a point is
//! always exactly on the curve and only its distance carries the
//! table's (second-order) error.

use glam::Vec2;
use std::cell::Cell;

use crate::document::values::{Direction, Edge, StartPoint};
use crate::flow::curve::Curve;
use crate::flow::outline::Outline;

/// The most samples an arc-length table holds (ruling R6, reading 9).
/// A path long enough to need more at the asked spacing — a canvas-sized
/// ellipse under a hairline font — is sampled more coarsely instead,
/// rather than allocating without bound.
pub const MAX_TABLE_SAMPLES: usize = 1 << 20;

/// The fewest samples a table holds, so a tiny curve is still walked
/// along its shape rather than along one chord.
const MIN_TABLE_SAMPLES: usize = 8;

/// A curve with a parameter, walked through a table.
#[derive(Copy, Clone, Debug)]
enum Param {
    /// `center + (radii.x · cos φ, radii.y · sin φ)`; `y` grows downward,
    /// so a growing `φ` runs clockwise on screen.
    Ellipse { center: Vec2, radii: Vec2 },
    /// A Bézier (or a line) over `t` in `0..=1`.
    Curve(Curve),
}

impl Param {
    fn point(&self, t: f32) -> Vec2 {
        match *self {
            Param::Ellipse { center, radii } => center + Vec2::new(radii.x * t.cos(), radii.y * t.sin()),
            Param::Curve(curve) => curve.point(t),
        }
    }

    fn derivative(&self, t: f32) -> Vec2 {
        match *self {
            Param::Ellipse { radii, .. } => Vec2::new(-radii.x * t.sin(), radii.y * t.cos()),
            Param::Curve(curve) => curve.derivative(t),
        }
    }
}

/// One piece of a path, walked from its own `s = 0`.
#[derive(Clone, Debug)]
enum Stretch {
    /// From `from`, along the unit vector `dir`, for `len`.
    Line { from: Vec2, dir: Vec2, len: f32 },
    /// Around `center` at `radius`, starting at `angle` (radians) and
    /// turning by `turn` (`+1` clockwise on screen, `-1` counter-clockwise)
    /// for `len`.
    Arc {
        center: Vec2,
        radius: f32,
        angle: f32,
        turn: f32,
        len: f32,
    },
    /// Through a table: the parameter at each sample (`params`), and the
    /// distance walked to reach it (`distances`, from 0, ascending).
    Table {
        param: Param,
        params: Vec<f32>,
        distances: Vec<f32>,
        /// The sample the last lookup fell after ([`at_or_below`]).
        cursor: Cell<usize>,
    },
}

/// The last index of `sorted` (ascending) whose value is at or below
/// `s`, or 0 when none is: `partition_point(|d| d <= s) − 1`, saturating.
/// `cursor`, the index the last lookup found, is tried first, and the
/// one after it: a walk goes forward a sample at a time, so each lookup
/// of a walk is found there. A lookup a few samples off — the chord a
/// frame's ink is pushed off along, either side of where it is set — is
/// found by galloping from the cursor, the bracket doubled each probe and
/// then searched; only a jump goes far. The same index every way, so a
/// lookup answers what a search would (ruling R43: walk the arc table
/// with a cursor everywhere).
///
/// Cost: O(1) at the cursor or the next index; else O(log d) for an
/// index `d` samples from the cursor.
fn at_or_below(sorted: &[f32], s: f32, cursor: &Cell<usize>) -> usize {
    let holds = |i: usize| sorted[i] <= s;
    let is_last = |i: usize| i < sorted.len() && holds(i) && (i + 1 == sorted.len() || !holds(i + 1));
    let hint = cursor.get();
    let found = if is_last(hint) {
        hint
    } else if is_last(hint + 1) {
        hint + 1
    } else {
        #[cfg(test)]
        PATH_SEARCHES.with(|count| count.set(count.get() + 1));
        gallop(sorted, s, hint)
    };
    cursor.set(found);
    found
}

/// Test seam (#294, ruling R44.7): [`at_or_below`] on `sorted` from a
/// cursor at `hint` — what a path's lookup searches its stretches' starts
/// and a table's distances with — and where it leaves the cursor.
#[cfg(test)]
pub(crate) fn at_or_below_seam(sorted: &[f32], s: f32, hint: usize) -> (usize, usize) {
    let cursor = Cell::new(hint);
    let found = at_or_below(sorted, s, &cursor);
    (found, cursor.get())
}

/// [`at_or_below`]'s search from `hint`: the bracket round the last index
/// at or below `s` doubled from `hint` the way it lies, then searched —
/// the predicate `d <= s` is monotone in an ascending slice, so this is
/// `partition_point(|d| d <= s) − 1`, saturating, found nearer.
///
/// Cost: O(log d) for an answer `d` from `hint`.
fn gallop(sorted: &[f32], s: f32, hint: usize) -> usize {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    let holds = |i: usize| sorted[i] <= s;
    let hint = hint.min(n - 1);
    // `low` holds (or is the slice's start) and `high` does not (or is
    // its end): the answer's partition point lies in `low..=high`.
    let (low, high) = if holds(hint) {
        let (mut low, mut step) = (hint, 1usize);
        loop {
            let probe = low.saturating_add(step);
            if probe >= n {
                break (low, n);
            }
            if !holds(probe) {
                break (low, probe);
            }
            low = probe;
            step = step.saturating_mul(2);
        }
    } else {
        let (mut high, mut step) = (hint, 1usize);
        loop {
            if high == 0 {
                break (0, 0);
            }
            let probe = high.saturating_sub(step);
            if holds(probe) {
                break (probe, high);
            }
            high = probe;
            step = step.saturating_mul(2);
        }
    };
    (low + sorted[low..high].partition_point(|&d| d <= s)).saturating_sub(1)
}

impl Stretch {
    /// Whether every point the stretch answers is finite: its geometry is
    /// finite and it has a finite, positive length. A table's distances
    /// sum the distances between its samples, so a sample that is not
    /// finite makes its length not finite.
    fn is_walkable(&self) -> bool {
        let len = self.len();
        let geometry = match self {
            Stretch::Line { from, dir, .. } => from.is_finite() && dir.is_finite(),
            Stretch::Arc {
                center,
                radius,
                angle,
                turn,
                ..
            } => center.is_finite() && radius.is_finite() && angle.is_finite() && turn.is_finite(),
            Stretch::Table { params, .. } => params.iter().all(|t| t.is_finite()),
        };
        geometry && len > 0.0 && len.is_finite()
    }

    fn len(&self) -> f32 {
        match self {
            Stretch::Line { len, .. } | Stretch::Arc { len, .. } => *len,
            Stretch::Table { distances, .. } => distances.last().copied().unwrap_or(0.0),
        }
    }

    /// The point `s` along the stretch and the unit direction of travel
    /// there.
    fn at(&self, s: f32) -> (Vec2, Vec2) {
        match self {
            Stretch::Line { from, dir, .. } => (*from + *dir * s, *dir),
            Stretch::Arc {
                center,
                radius,
                angle,
                turn,
                ..
            } => {
                let a = angle + turn * s / radius;
                let (sin, cos) = a.sin_cos();
                (
                    *center + Vec2::new(cos, sin) * *radius,
                    Vec2::new(-sin, cos) * *turn,
                )
            }
            Stretch::Table {
                param,
                params,
                distances,
                cursor,
            } => {
                let last = distances.len().saturating_sub(2);
                let i = at_or_below(distances, s, cursor).min(last);
                let (d0, d1) = (distances[i], distances[i + 1]);
                let (t0, t1) = (params[i], params[i + 1]);
                let frac = if d1 > d0 {
                    ((s - d0) / (d1 - d0)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let t = t0 + (t1 - t0) * frac;
                let forward = if t1 >= t0 { 1.0 } else { -1.0 };
                let tangent = direction_of(param.derivative(t) * forward)
                    .or_else(|| direction_of(param.point(t1) - param.point(t0)))
                    .unwrap_or(Vec2::X);
                (param.point(t), tangent)
            }
        }
    }
}

thread_local! {
    /// Test counter (#294): the points [`ArcPath::at`] has looked up on
    /// this thread.
    #[cfg(test)]
    pub(crate) static PATH_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294): the binary searches [`at_or_below`] has made
    /// on this thread, a lookup its cursor did not answer.
    #[cfg(test)]
    pub(crate) static PATH_SEARCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A path walked by arc length.
#[derive(Clone, Debug)]
pub(crate) struct ArcPath {
    stretches: Vec<Stretch>,
    /// The distance at which each stretch begins.
    starts: Vec<f32>,
    length: f32,
    /// Where the path begins: what a path of no length answers. Always
    /// finite.
    origin: Vec2,
    /// `+1` for a closed path walked clockwise, `-1` counter-clockwise,
    /// `0` for an open path, which has no outside.
    winding: f32,
    /// How many samples were built into arc-length tables for this
    /// path, kept or dropped as not walkable.
    table_samples: usize,
    /// The stretch the last lookup fell in ([`at_or_below`]).
    cursor: Cell<usize>,
    /// Its sharp corners, in order along it: where two straight
    /// stretches meet at an angle, as a rounded rectangle of no radius
    /// turns ([`offset_at`](Self::offset_at), #308). None on an open path.
    corners: Vec<Corner>,
}

/// A bend of a closed path between two straight sides (#308): a sharp
/// corner, where one side ends and the next begins at an angle and the
/// path has no one normal, or a run of arcs about one center, a rounded
/// corner ([`ArcPath::offset_at`]).
#[derive(Copy, Clone, Debug)]
struct Corner {
    /// Where along the path it begins, in `0.0..length`.
    at: f32,
    /// How long it is along the path: none for a sharp corner, its arcs'
    /// for a rounded one.
    bend: f32,
    /// The corner's point, or its arcs' center.
    point: Vec2,
    /// Its arcs' radius: none for a sharp corner.
    radius: f32,
    /// The direction of travel along the side before it and the side
    /// after it.
    before: Vec2,
    after: Vec2,
    /// How far the direction of travel turns there, in radians, signed:
    /// positive turning clockwise on screen.
    turn: f32,
    /// How long the straight sides before and after it are.
    sides: (f32, f32),
}

/// The point at `s` of a closed path's offset curve (#308,
/// [`ArcPath::offset_at`]).
#[derive(Copy, Clone, Debug)]
pub(crate) struct OffsetPoint {
    /// The point, `offset` along the path's outward normal from it.
    pub(crate) point: Vec2,
    /// The unit direction the point moves in as `s` grows: the path's
    /// direction of travel there, turned back where the offset sweeps a
    /// sharp corner from inside it.
    pub(crate) moves: Vec2,
    /// The path's outward normal the point is offset along: at a sharp
    /// corner, the one the sweep has reached.
    pub(crate) normal: Vec2,
}

impl ArcPath {
    /// Collect stretches, dropping those that have no length or answer a
    /// point that is not finite, so every point the path answers is
    /// finite: an outline or curve with no finite stretch is an empty
    /// path, and one with some keeps only those. An `origin` that is not
    /// finite is replaced by `Vec2::ZERO`. Geometry is checked at load;
    /// this keeps a hand-built or fuzzed one from placing glyphs at `NaN`.
    fn from_stretches(stretches: impl IntoIterator<Item = Stretch>, origin: Vec2, winding: f32) -> Self {
        let mut path = ArcPath {
            stretches: Vec::new(),
            starts: Vec::new(),
            length: 0.0,
            origin: if origin.is_finite() { origin } else { Vec2::ZERO },
            winding,
            table_samples: 0,
            cursor: Cell::new(0),
            corners: Vec::new(),
        };
        for stretch in stretches {
            if let Stretch::Table { params, .. } = &stretch {
                path.table_samples += params.len();
            }
            let len = stretch.len();
            if stretch.is_walkable() {
                path.starts.push(path.length);
                path.length += len;
                path.stretches.push(stretch);
            }
        }
        path.corners = path.find_corners();
        path
    }

    /// The sharp corners of a closed path ([`Corner`]): each meeting of
    /// a straight stretch with the next straight one, across the seam
    /// too, where the direction of travel turns by more than
    /// [`CORNER_TURN`]. A straight side met by a corner arc turns
    /// smoothly into it, however small its radius. Each corner's
    /// [`sides`](Corner::sides) are its sides' whole lengths: every
    /// straight stretch in line with the one beside it, the seam's two
    /// included, where a path started part way along a side cuts it in
    /// two (#308: round a sharp rectangle started at the middle of its
    /// left side, as a frame's halves are, a side read as its stretch
    /// alone was half its length, and the curve inside each of its
    /// corners stepped by as much as the cap fell short).
    ///
    /// Cost: O(stretches), and O(stretches) more for each corner's sides.
    fn find_corners(&self) -> Vec<Corner> {
        let n = self.stretches.len();
        if self.winding == 0.0 || n < 2 || !self.length.is_finite() {
            return Vec::new();
        }
        // The length of the straight side stretch `k` lies on, running
        // on from it `step` stretches at a time (`n − 1` back, `1` on)
        // over every straight stretch in line with it, `stop` excluded.
        let side_through = |k: usize, step: usize, stop: usize| {
            let Stretch::Line { dir, len, .. } = &self.stretches[k] else {
                return 0.0;
            };
            let mut long = *len;
            let mut at = k;
            for _ in 1..n {
                at = (at + step) % n;
                if at == stop || at == k {
                    break;
                }
                match &self.stretches[at] {
                    Stretch::Line { dir: d, len, .. }
                        if d.perp_dot(*dir).atan2(d.dot(*dir)).abs() <= CORNER_TURN =>
                    {
                        long += len;
                    }
                    _ => break,
                }
            }
            long
        };
        let mut corners = Vec::new();
        for i in 0..n {
            let Stretch::Line {
                from,
                dir: before,
                len: side_before,
            } = &self.stretches[i]
            else {
                continue;
            };
            // The arcs after the side, about one center, and the side
            // after them.
            let mut j = (i + 1) % n;
            let (mut bend, mut center, mut radius, mut arced) = (0.0_f32, None::<Vec2>, 0.0_f32, 0.0_f32);
            while let Stretch::Arc {
                center: c,
                radius: r,
                turn,
                len,
                ..
            } = &self.stretches[j]
            {
                if center.is_some_and(|was| was.distance(*c) > CORNER_TURN * r.max(1.0)) || j == i {
                    break;
                }
                (bend, center, radius) = (bend + len, Some(*c), *r);
                arced += turn * len / r;
                j = (j + 1) % n;
            }
            let Stretch::Line { dir: after, .. } = &self.stretches[j] else {
                continue;
            };
            let sharp = center.is_none();
            let turn = if sharp {
                before.perp_dot(*after).atan2(before.dot(*after))
            } else {
                arced
            };
            if turn.is_finite() && (!sharp || turn.abs() > CORNER_TURN) {
                let start = (i + 1) % n;
                corners.push(Corner {
                    at: if start == 0 { 0.0 } else { self.starts[start] },
                    bend,
                    point: center.unwrap_or(*from + *before * *side_before),
                    radius,
                    before: *before,
                    after: *after,
                    turn,
                    sides: (side_through(i, n - 1, j), side_through(j, 1, i)),
                });
            }
        }
        corners.sort_by(|a, b| a.at.total_cmp(&b.at));
        corners
    }

    /// `outline` walked once around from `start` in `direction`, its
    /// curved stretches tabled no coarser than `spacing` (#239 §3).
    ///
    /// A rounded rectangle is four sides, each a straight stretch
    /// followed clockwise by the corner after it; the start sits at `at`
    /// along its side's straight stretch (top runs left to right, right
    /// top to bottom, bottom right to left, left bottom to top). An
    /// ellipse starts at the parametric angle `at` of the way through
    /// its side's quarter (top 225°–315°, right 315°–405°, bottom
    /// 45°–135°, left 135°–225°).
    ///
    /// Cost: O(1) for a rounded rectangle or a circle; one table of
    /// `O(perimeter / spacing)` samples, at most [`MAX_TABLE_SAMPLES`],
    /// for an ellipse.
    pub(crate) fn closed(outline: &Outline, start: StartPoint, direction: Direction, spacing: f32) -> Self {
        let turn = match direction {
            Direction::Clockwise => 1.0,
            Direction::CounterClockwise => -1.0,
        };
        let at = if start.at.is_nan() {
            0.0
        } else {
            start.at.clamp(0.0, 1.0)
        };
        let side = match start.edge {
            Edge::Top => 0,
            Edge::Right => 1,
            Edge::Bottom => 2,
            Edge::Left => 3,
        };
        match *outline {
            Outline::RoundedRect { rect, .. } => {
                let r = outline.corner_radius();
                let (min, max) = (rect.origin, rect.max());
                let sides = [
                    (Vec2::new(min.x + r, min.y), Vec2::new(max.x - r, min.y)),
                    (Vec2::new(max.x, min.y + r), Vec2::new(max.x, max.y - r)),
                    (Vec2::new(max.x - r, max.y), Vec2::new(min.x + r, max.y)),
                    (Vec2::new(min.x, max.y - r), Vec2::new(min.x, min.y + r)),
                ];
                let half_pi = std::f32::consts::FRAC_PI_2;
                // The corner after side `i`, and the angle it starts at
                // clockwise.
                let corners = [
                    (Vec2::new(max.x - r, min.y + r), -half_pi),
                    (Vec2::new(max.x - r, max.y - r), 0.0),
                    (Vec2::new(min.x + r, max.y - r), half_pi),
                    (Vec2::new(min.x + r, min.y + r), std::f32::consts::PI),
                ];
                let line = |from: Vec2, to: Vec2| Stretch::Line {
                    from,
                    dir: direction_of(to - from).unwrap_or(Vec2::X),
                    len: length_of(to - from),
                };
                let corner = |i: usize, clockwise: bool| {
                    let (center, angle) = corners[i];
                    Stretch::Arc {
                        center,
                        radius: r,
                        angle: if clockwise { angle } else { angle + half_pi },
                        turn: if clockwise { 1.0 } else { -1.0 },
                        len: r * half_pi,
                    }
                };
                let (a, b) = sides[side];
                let point = a + (b - a) * at;
                let mut stretches = Vec::with_capacity(9);
                if turn > 0.0 {
                    stretches.push(line(point, b));
                    for k in 0..4 {
                        stretches.push(corner((side + k) % 4, true));
                        if k < 3 {
                            let (from, to) = sides[(side + k + 1) % 4];
                            stretches.push(line(from, to));
                        }
                    }
                    stretches.push(line(a, point));
                } else {
                    stretches.push(line(point, a));
                    for k in 1..=4 {
                        stretches.push(corner((side + 4 - k) % 4, false));
                        if k < 4 {
                            let (from, to) = sides[(side + 4 - k) % 4];
                            stretches.push(line(to, from));
                        }
                    }
                    stretches.push(line(b, point));
                }
                ArcPath::from_stretches(stretches, point, turn)
            }
            Outline::Ellipse { rect } => {
                let center = rect.center();
                let radii = rect.size * 0.5;
                let quarter = [
                    (225.0_f32, 315.0_f32),
                    (315.0, 405.0),
                    (45.0, 135.0),
                    (135.0, 225.0),
                ][side];
                let phi = (quarter.0 + (quarter.1 - quarter.0) * at).to_radians();
                let param = Param::Ellipse { center, radii };
                let origin = param.point(phi);
                let tau = std::f32::consts::TAU;
                let largest = radii.x.max(radii.y);
                let stretch = if !(center.is_finite() && radii.is_finite()) {
                    // No table of points that are not finite is built.
                    None
                } else if (radii.x - radii.y).abs() <= largest * 1e-6 {
                    Some(Stretch::Arc {
                        center,
                        radius: largest,
                        angle: phi,
                        turn,
                        len: largest * tau,
                    })
                } else {
                    table(param, phi, phi + turn * tau, largest * tau, spacing)
                };
                ArcPath::from_stretches(stretch, origin, turn)
            }
        }
    }

    /// `curve` walked from its source end, tabled no coarser than
    /// `spacing` unless it is a straight line.
    ///
    /// Cost: O(1) for a line; one table of `O(length / spacing)` samples,
    /// at most [`MAX_TABLE_SAMPLES`], for a Bézier.
    pub(crate) fn open(curve: &Curve, spacing: f32) -> Self {
        let origin = curve.point(0.0);
        let stretch = match *curve {
            Curve::Line(a, b) => Some(Stretch::Line {
                from: a,
                dir: direction_of(b - a).unwrap_or(Vec2::X),
                len: length_of(b - a),
            }),
            _ => table(Param::Curve(*curve), 0.0, 1.0, curve.speed_bound(), spacing),
        };
        ArcPath::from_stretches(stretch, origin, 0.0)
    }

    /// The path's length, `P`.
    pub(crate) fn length(&self) -> f32 {
        self.length
    }

    /// How many samples were built into arc-length tables for this
    /// path, all told — those dropped as not walkable included: 0 for a
    /// path of lines and arcs, and for a curve that is not finite, which
    /// builds no table. What a test reads to hold the table spacing to its
    /// floor, and a non-finite curve to no work, without timing anything.
    pub(crate) fn table_samples(&self) -> usize {
        self.table_samples
    }

    /// The point `s` along the path and the unit direction of travel
    /// there. A closed path wraps around (`s` past its length is a
    /// second lap); an open path holds its ends, so walking off one
    /// keeps its end direction.
    ///
    /// Cost: two binary searches (the stretch, then a table's sample).
    pub(crate) fn at(&self, s: f32) -> (Vec2, Vec2) {
        #[cfg(test)]
        PATH_LOOKUPS.with(|count| count.set(count.get() + 1));
        if self.stretches.is_empty() {
            return (self.origin, Vec2::X);
        }
        let s = if self.winding != 0.0 {
            s.rem_euclid(self.length)
        } else {
            s.clamp(0.0, self.length)
        };
        let index = at_or_below(&self.starts, s, &self.cursor);
        self.stretches[index].at(s - self.starts[index])
    }

    /// Where along the path the point nearest `p` lies: a sampling every
    /// [`NEAREST_SAMPLES`]th of the path, then a golden-section search
    /// between the best sample's neighbors — across the seam of a closed
    /// path, the result in `0.0..length` — on an open one within its
    /// ends. On a convex path whose nearest point is unique, that point;
    /// ties go to the first. `0.0` for an empty path.
    ///
    /// Cost: [`NEAREST_SAMPLES`] path lookups, and [`NEAREST_ROUNDS`] more.
    pub(crate) fn nearest(&self, p: Vec2) -> f32 {
        // A length that is not a number is no length either.
        if self.stretches.is_empty() || self.length.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return 0.0;
        }
        let step = self.length / NEAREST_SAMPLES as f32;
        // Not squared: on an outline some 10³⁷ across every squared
        // distance is infinite, and the first sample was taken for all.
        let distance = |s: f32| length_of(self.at(s).0 - p);
        let mut best = (f32::INFINITY, 0.0_f32);
        for k in 0..=NEAREST_SAMPLES {
            let s = step * k as f32;
            let d = distance(s);
            if d < best.0 {
                best = (d, s);
            }
        }
        // On a closed path the bracket runs across the seam, which `at`
        // wraps: a nearest point just before it is found from a best
        // sample at its start.
        let closed = self.winding != 0.0;
        let (mut low, mut high) = if closed {
            (best.1 - step, best.1 + step)
        } else {
            ((best.1 - step).max(0.0), (best.1 + step).min(self.length))
        };
        let ratio = 0.5 * (5.0_f32.sqrt() - 1.0);
        for _ in 0..NEAREST_ROUNDS {
            let a = high - ratio * (high - low);
            let b = low + ratio * (high - low);
            if distance(a) <= distance(b) {
                high = b;
            } else {
                low = a;
            }
        }
        let found = 0.5 * (low + high);
        if closed {
            found.rem_euclid(self.length)
        } else {
            found
        }
    }

    /// Where along the path the point `p` lies, for a point of the
    /// outline the path was built from — one its own geometry answers,
    /// such as a support point ([`Outline::support`]) — found from each
    /// stretch's geometry: its projection on a side, its angle on a
    /// corner or a circle, its parameter on an ellipse's table, held to
    /// the stretch's ends; of the stretches, the one it lies nearest,
    /// ties to the first. At no path lookup, where
    /// [`nearest`](Self::nearest) samples the whole path (ruling R43).
    /// `None` where a stretch has no such form — a Bézier's table — or
    /// `p` is not finite, for `nearest` to answer instead; `0.0` for an
    /// empty path, as `nearest` answers.
    ///
    /// Cost: O(stretches), and a binary search of an ellipse's table.
    pub(crate) fn locate(&self, p: Vec2) -> Option<f32> {
        if !p.is_finite() {
            return None;
        }
        if self.stretches.is_empty() {
            return Some(0.0);
        }
        let tau = std::f32::consts::TAU;
        let mut best: Option<(f32, f32)> = None;
        for (stretch, &start) in self.stretches.iter().zip(&self.starts) {
            let (local, at) = match stretch {
                Stretch::Line { from, dir, len } => {
                    let t = (p - *from).dot(*dir).clamp(0.0, *len);
                    (t, *from + *dir * t)
                }
                Stretch::Arc {
                    center,
                    radius,
                    angle,
                    turn,
                    len,
                } => {
                    let a = (p.y - center.y).atan2(p.x - center.x);
                    let swept = ((a - angle) * turn).rem_euclid(tau) * radius;
                    // Past the arc's end: its nearer end.
                    let t = if swept <= *len {
                        swept
                    } else if tau * radius - swept < swept - len {
                        0.0
                    } else {
                        *len
                    };
                    (t, stretch.at(t).0)
                }
                Stretch::Table {
                    param: Param::Ellipse { center, radii },
                    params,
                    distances,
                    ..
                } => {
                    let (Some(&first), Some(&last)) = (params.first(), params.last()) else {
                        continue;
                    };
                    let rising = last >= first;
                    let phi = ((p.y - center.y) / radii.y).atan2((p.x - center.x) / radii.x);
                    // The parameter's turn the table walks.
                    let sign = if rising { 1.0 } else { -1.0 };
                    let phi = first + ((phi - first) * sign).rem_euclid(tau) * sign;
                    let below = |t: f32| if rising { t <= phi } else { t >= phi };
                    let i = params
                        .partition_point(|&t| below(t))
                        .saturating_sub(1)
                        .min(params.len().saturating_sub(2));
                    let (t0, t1) = (params[i], params[(i + 1).min(params.len() - 1)]);
                    let (d0, d1) = (distances[i], distances[(i + 1).min(distances.len() - 1)]);
                    let frac = if t1 != t0 {
                        ((phi - t0) / (t1 - t0)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let local = d0 + (d1 - d0) * frac;
                    let point = Param::Ellipse {
                        center: *center,
                        radii: *radii,
                    }
                    .point(t0 + (t1 - t0) * frac);
                    (local, point)
                }
                Stretch::Table { .. } => return None,
            };
            // Measured so that an outline some 10³⁷ across, whose squared
            // distances pass every `f32`, still tells near from far.
            let d = length_of(at - p);
            if best.is_none_or(|(near, _)| d < near) {
                best = Some((d, start + local));
            }
        }
        best.map(|(_, s)| s)
    }

    /// The outward unit normal beside `tangent`, for a closed path: the
    /// direction of travel turned a quarter toward the outside. Zero for
    /// an open path, which has no outside.
    pub(crate) fn outward(&self, tangent: Vec2) -> Vec2 {
        Vec2::new(tangent.y, -tangent.x) * self.winding
    }

    /// The point at `s` of the path's offset curve at `offset` along its
    /// outward normal (inward for a negative `offset`, #308): `p(s) +
    /// offset · n(s)`, exactly, wherever the path has one normal and the
    /// offset does not pass the radius of a bend inward — its sides, its
    /// arcs, an ellipse's table, whose points and directions are the
    /// curve's own. An offset inward past an ellipse's radius of curvature,
    /// at the tips of one fifty times as wide as tall, folds back there as
    /// a parallel curve does.
    ///
    /// **Sharp corners, outward.** At a sharp corner ([`Corner`]) the path
    /// has no one normal, and the curve sweeps round the corner at `offset`
    /// from it, from the side before's normal to the side after's. That
    /// sweep has no length along the path, so it is given some: the stretch
    /// within `w` of the corner — `w` half the sweep's own length, at most
    /// half of either side — walks the offset curve's side before, its
    /// sweep and its side after at one speed, `(2w + |offset|·|turn|) /
    /// 2w`. So the point moves continuously as `s` moves through a corner,
    /// never jumping from one side's offset to the other's, and every point
    /// lies on the curve.
    ///
    /// **Bends, inward.** Inside a corner — a sharp one, or a rounded one
    /// whose radius the offset passes — the two sides' offsets cross before
    /// they reach it, at the corner of the curve inside, `X`, and what lies
    /// past it of either, and round the bend between, folds back over the
    /// outline: the stretch of the path from where either side's offset
    /// reaches `X`, `|radius + offset|·tan(|turn| / 2)` before the bend and
    /// after it, at most half of either side, is the point `X` throughout,
    /// its normal turning from the side before's to the side after's. A
    /// bend that turns by half a turn or more, the end of an outline thinner
    /// than twice the offset, has no such corner, and folds.
    ///
    /// With no offset it is the path itself.
    ///
    /// Every bend of a closed outline turns with its winding: it is
    /// convex, both [`Outline`] kinds being so. A concave corner would want
    /// the other branches — outward offsets crossing there, inward ones
    /// sweeping — and is not handled.
    ///
    /// Cost: one path lookup; within a corner's stretch none, but its
    /// arithmetic, counted as one.
    pub(crate) fn offset_at(&self, s: f32, offset: f32) -> OffsetPoint {
        if self.winding != 0.0 && offset != 0.0 && self.length > 0.0 {
            let s = s.rem_euclid(self.length);
            // Signed, across the seam, from `at`.
            let from_at = |at: f32| {
                let from = s - at;
                if from > 0.5 * self.length {
                    from - self.length
                } else if from < -0.5 * self.length {
                    from + self.length
                } else {
                    from
                }
            };
            for corner in &self.corners {
                let half_turn = 0.5 * corner.turn.abs();
                let inside = corner.radius + offset;
                if offset < 0.0 && inside < 0.0 && half_turn < INSIDE_TURN {
                    let t = (-inside * half_turn.tan())
                        .min(0.5 * corner.sides.0)
                        .min(0.5 * corner.sides.1);
                    let reach = 0.5 * corner.bend + t;
                    let from = from_at(corner.at + 0.5 * corner.bend);
                    if reach > 0.0 && from.abs() < reach && reach.is_finite() {
                        #[cfg(test)]
                        PATH_LOOKUPS.with(|count| count.set(count.get() + 1));
                        return self.inside(corner, inside, half_turn, (from + reach) / (2.0 * reach));
                    }
                    continue;
                }
                if corner.radius > 0.0 || offset < 0.0 {
                    continue;
                }
                let sweep = offset * corner.turn.abs();
                let w = (0.5 * sweep).min(0.5 * corner.sides.0).min(0.5 * corner.sides.1);
                if !(w > 0.0 && sweep.is_finite()) {
                    continue;
                }
                let from = from_at(corner.at);
                if from.abs() >= w {
                    continue;
                }
                #[cfg(test)]
                PATH_LOOKUPS.with(|count| count.set(count.get() + 1));
                return self.swept(corner, offset, w, sweep, from);
            }
        }
        let (point, tangent) = self.at(s);
        let normal = self.outward(tangent);
        OffsetPoint {
            point: point + normal * offset,
            moves: tangent,
            normal,
        }
    }

    /// [`offset_at`](Self::offset_at) inside `corner`, its offset past the
    /// bend's radius by `inside` (negative), `frac` of the way through the
    /// stretch that is the inside curve's corner `X`.
    fn inside(&self, corner: &Corner, inside: f32, half_turn: f32, frac: f32) -> OffsetPoint {
        let (n1, n2) = (self.outward(corner.before), self.outward(corner.after));
        let bisector = (n1 + n2).normalize_or_zero();
        let point = corner.point + bisector * (inside / half_turn.cos());
        OffsetPoint {
            point,
            moves: (corner.before + corner.after).normalize_or_zero(),
            normal: n1.lerp(n2, frac.clamp(0.0, 1.0)).normalize_or_zero(),
        }
    }

    /// [`offset_at`](Self::offset_at) within `w` of the sharp `corner`
    /// outward, `from` past it (negative before it), its sweep `sweep`
    /// long.
    fn swept(&self, corner: &Corner, offset: f32, w: f32, sweep: f32, from: f32) -> OffsetPoint {
        // How far along the offset curve's stretch round the corner.
        let along = (from + w) / (2.0 * w) * (2.0 * w + sweep);
        let side = |dir: Vec2, by: f32| {
            let normal = self.outward(dir);
            OffsetPoint {
                point: corner.point + dir * by + normal * offset,
                moves: dir,
                normal,
            }
        };
        if along <= w {
            side(corner.before, along - w)
        } else if along >= w + sweep {
            side(corner.after, along - w - sweep)
        } else {
            let (sin, cos) = (corner.turn * (along - w) / sweep).sin_cos();
            let turned = Vec2::new(
                corner.before.x * cos - corner.before.y * sin,
                corner.before.x * sin + corner.before.y * cos,
            );
            let normal = self.outward(turned);
            OffsetPoint {
                point: corner.point + normal * offset,
                moves: turned,
                normal,
            }
        }
    }
}

/// How far, in radians, the direction of travel must turn where two
/// straight stretches meet for the meeting to be a sharp corner
/// ([`ArcPath::offset_at`]): a millionth, past the float error of two
/// stretches of one straight side.
const CORNER_TURN: f32 = 1e-6;

/// The most half a bend's turn may be, in radians, for the offsets of its
/// sides inside it to cross at a corner of the inside curve
/// ([`ArcPath::offset_at`]): a hair short of a quarter turn, past which
/// the two sides run back side by side and their offsets inside meet far
/// off or never.
const INSIDE_TURN: f32 = 0.49 * std::f32::consts::PI;

/// `v`'s length, finite wherever it is: [`Vec2::length`], or, for a
/// vector longer than `√f32::MAX` (about `1.8e19`) — whose squared
/// length passes every `f32`, so `Vec2::length` answers infinity — the
/// length measured scaled down by its largest component. Without it, a
/// path drawn that far had no length and placed nothing. Every shorter
/// vector measures as `Vec2::length` measures it, to the bit.
///
/// Cost: O(1).
pub(crate) fn length_of(v: Vec2) -> f32 {
    let length = v.length();
    let largest = v.abs().max_element();
    if length.is_finite() || !(largest > 0.0 && largest.is_finite()) {
        length
    } else {
        largest * (v / largest).length()
    }
}

/// `v`'s unit direction: [`Vec2::try_normalize`], or, for a vector too
/// long for it — whose [`Vec2::length`] is not finite though `v` is —
/// its direction scaled first as [`length_of`] scales it. `None` for a
/// zero vector, one that is not finite, and one so short its length
/// underflows to zero, as `try_normalize` answers: every vector it has
/// an answer for gets that answer, to the bit.
///
/// Cost: O(1).
pub(crate) fn direction_of(v: Vec2) -> Option<Vec2> {
    let largest = v.abs().max_element();
    if v.length().is_finite() || !(largest > 0.0 && largest.is_finite()) {
        v.try_normalize()
    } else {
        (v / largest).try_normalize()
    }
}

/// How many samples [`ArcPath::nearest`] takes along a path.
pub(crate) const NEAREST_SAMPLES: usize = 2048;

/// How many rounds of golden-section search [`ArcPath::nearest`] refines
/// its best sample with: a bracket of two samples closes by a factor of
/// about 10⁷.
pub(crate) const NEAREST_ROUNDS: usize = 34;

/// A table walking `param` from `from` to `to`, its samples close
/// enough that no neighbors are more than `spacing` apart, given that the
/// curve moves at most `speed` per unit of the full parameter range.
///
/// `None`, building nothing, when `speed` or the parameter range is not
/// finite: a curve with no finite speed bound has a control point that is
/// not finite, so every sample would be too.
fn table(param: Param, from: f32, to: f32, speed: f32, spacing: f32) -> Option<Stretch> {
    if !(speed.is_finite() && from.is_finite() && to.is_finite()) {
        return None;
    }
    let spacing = if spacing > 0.0 { spacing } else { 1.0 };
    let wanted = (speed / spacing).ceil();
    let count = if wanted.is_finite() {
        (wanted as usize).clamp(MIN_TABLE_SAMPLES, MAX_TABLE_SAMPLES)
    } else {
        MIN_TABLE_SAMPLES
    };
    let mut params = Vec::with_capacity(count + 1);
    let mut distances = Vec::with_capacity(count + 1);
    let mut previous = param.point(from);
    let mut walked = 0.0_f32;
    for i in 0..=count {
        let t = from + (to - from) * (i as f32 / count as f32);
        let point = param.point(t);
        walked += length_of(point - previous);
        previous = point;
        params.push(t);
        distances.push(walked);
    }
    Some(Stretch::Table {
        param,
        params,
        distances,
        cursor: Cell::new(0),
    })
}
