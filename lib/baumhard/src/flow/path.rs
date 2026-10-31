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
    },
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
            } => {
                let last = distances.len().saturating_sub(2);
                let i = distances.partition_point(|&d| d <= s).saturating_sub(1).min(last);
                let (d0, d1) = (distances[i], distances[i + 1]);
                let (t0, t1) = (params[i], params[i + 1]);
                let frac = if d1 > d0 {
                    ((s - d0) / (d1 - d0)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let t = t0 + (t1 - t0) * frac;
                let forward = if t1 >= t0 { 1.0 } else { -1.0 };
                let tangent = (param.derivative(t) * forward)
                    .try_normalize()
                    .or_else(|| (param.point(t1) - param.point(t0)).try_normalize())
                    .unwrap_or(Vec2::X);
                (param.point(t), tangent)
            }
        }
    }
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
        path
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
                let line = |from: Vec2, to: Vec2| {
                    let len = (to - from).length();
                    Stretch::Line {
                        from,
                        dir: (to - from).try_normalize().unwrap_or(Vec2::X),
                        len,
                    }
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
                dir: (b - a).try_normalize().unwrap_or(Vec2::X),
                len: (b - a).length(),
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
        if self.stretches.is_empty() {
            return (self.origin, Vec2::X);
        }
        let s = if self.winding != 0.0 {
            s.rem_euclid(self.length)
        } else {
            s.clamp(0.0, self.length)
        };
        let index = self.starts.partition_point(|&start| start <= s).saturating_sub(1);
        self.stretches[index].at(s - self.starts[index])
    }

    /// Where along the path the point nearest `p` lies: a sampling every
    /// [`NEAREST_SAMPLES`]th of the path, then a golden-section search
    /// between the best sample's neighbors. On a convex path whose
    /// nearest point is unique, that point; ties go to the first. `0.0`
    /// for an empty path.
    ///
    /// Cost: [`NEAREST_SAMPLES`] path lookups, and [`NEAREST_ROUNDS`] more.
    pub(crate) fn nearest(&self, p: Vec2) -> f32 {
        // A length that is not a number is no length either.
        if self.stretches.is_empty() || self.length.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return 0.0;
        }
        let step = self.length / NEAREST_SAMPLES as f32;
        let distance = |s: f32| self.at(s).0.distance_squared(p);
        let mut best = (f32::INFINITY, 0.0_f32);
        for k in 0..=NEAREST_SAMPLES {
            let s = step * k as f32;
            let d = distance(s);
            if d < best.0 {
                best = (d, s);
            }
        }
        let (mut low, mut high) = ((best.1 - step).max(0.0), (best.1 + step).min(self.length));
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
        0.5 * (low + high)
    }

    /// The outward unit normal beside `tangent`, for a closed path: the
    /// direction of travel turned a quarter toward the outside. Zero for
    /// an open path, which has no outside.
    pub(crate) fn outward(&self, tangent: Vec2) -> Vec2 {
        Vec2::new(tangent.y, -tangent.x) * self.winding
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
        walked += (point - previous).length();
        previous = point;
        params.push(t);
        distances.push(walked);
    }
    Some(Stretch::Table {
        param,
        params,
        distances,
    })
}
