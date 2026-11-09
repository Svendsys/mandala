// SPDX-License-Identifier: MPL-2.0

//! Open paths: the curves a connector or a line link runs along.
//!
//! An open path is one curve — a line, a quadratic or a cubic Bézier —
//! or a chain of them, each starting where the one before it ends: the
//! projection draws a connector as cubics through its control points
//! (#296). Where the ends sit and which side of a node they attach to is
//! the projection's to resolve; this module takes the resolved curves,
//! as a slice (`&[Curve]`), or as one curve, which is a chain of one.

use glam::Vec2;

/// A resolved open path, from its source end to its target end.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Curve {
    /// A straight line: no control points.
    Line(Vec2, Vec2),
    /// A quadratic Bézier: start, one control point, end.
    Quadratic(Vec2, Vec2, Vec2),
    /// A cubic Bézier: start, two control points, end.
    Cubic(Vec2, Vec2, Vec2, Vec2),
}

/// One curve is a chain of one: every function that walks an open path
/// takes `impl AsRef<[Curve]>`, so a lone curve and a chain are passed
/// alike.
impl AsRef<[Curve]> for Curve {
    fn as_ref(&self) -> &[Curve] {
        std::slice::from_ref(self)
    }
}

/// The point at `t` in `0.0..=1.0` of the chain `curves`, each curve one
/// even share of `t`: the middle of a two-curve chain is where its curves
/// meet. `None` for an empty chain. O(1).
pub fn chain_point(curves: &[Curve], t: f32) -> Option<Vec2> {
    let last = curves.len().checked_sub(1)?;
    let at = t.clamp(0.0, 1.0) * curves.len() as f32;
    let index = (at.floor() as usize).min(last);
    Some(curves[index].point(at - index as f32))
}

impl Curve {
    /// The point at parameter `t` in `0.0..=1.0`. O(1).
    pub fn point(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        match *self {
            Curve::Line(a, b) => a * u + b * t,
            Curve::Quadratic(a, c, b) => a * (u * u) + c * (2.0 * u * t) + b * (t * t),
            Curve::Cubic(a, c1, c2, b) => {
                a * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + b * (t * t * t)
            }
        }
    }

    /// The derivative with respect to `t` at `t`: the direction of
    /// travel, scaled by the parameter's speed. Zero where the curve
    /// stalls (a control point on an end). O(1).
    pub fn derivative(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        match *self {
            Curve::Line(a, b) => b - a,
            Curve::Quadratic(a, c, b) => (c - a) * (2.0 * u) + (b - c) * (2.0 * t),
            Curve::Cubic(a, c1, c2, b) => {
                (c1 - a) * (3.0 * u * u) + (c2 - c1) * (6.0 * u * t) + (b - c2) * (3.0 * t * t)
            }
        }
    }

    /// An upper bound on `|derivative(t)|` over `0..=1`: the degree
    /// times the longest leg of the control polygon (the derivative is a
    /// convex combination of the legs, scaled by the degree). Two points
    /// `Δt` apart are then at most `speed_bound() · Δt` apart, which is
    /// how an arc-length table picks its sample count. O(1).
    ///
    /// `NaN` when any leg's length is not finite — a control point that
    /// is not finite, or a leg longer than the largest `f32` — since such
    /// a curve has no bound. (`f32::max` would drop a `NaN` leg and bound the rest.)
    /// Every leg finite, the bound saturates at the largest `f32`: a
    /// quadratic whose legs are each within it can still be twice as fast,
    /// and its path no longer for that (seed 1021 of the structured flow
    /// generator: a leg of 2.4 × 10³⁸, a path that long, whose bound of
    /// twice that was infinite, so that it built no table and was placed
    /// as a path of no length). The bound only picks a table's sample
    /// count, which [`MAX_TABLE_SAMPLES`](crate::flow::path::MAX_TABLE_SAMPLES)
    /// caps far below it; a path longer than the largest `f32` is still
    /// dropped as not walkable (`ArcPath::from_stretches`).
    pub fn speed_bound(&self) -> f32 {
        let bound = |degree: f32, legs: &[Vec2]| {
            let mut longest = 0.0_f32;
            for leg in legs {
                let length = crate::flow::path::length_of(*leg);
                if !length.is_finite() {
                    return f32::NAN;
                }
                longest = longest.max(length);
            }
            (degree * longest).min(f32::MAX)
        };
        match *self {
            Curve::Line(a, b) => bound(1.0, &[b - a]),
            Curve::Quadratic(a, c, b) => bound(2.0, &[c - a, b - c]),
            Curve::Cubic(a, c1, c2, b) => bound(3.0, &[c1 - a, c2 - c1, b - c2]),
        }
    }
}
