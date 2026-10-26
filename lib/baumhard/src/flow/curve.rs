// SPDX-License-Identifier: MPL-2.0

//! Open paths: the curve a connector or a line link runs along.
//!
//! A connector or link has 0, 1 or 2 control points
//! ([`MAX_CONTROL_POINTS`](crate::document::bounds::MAX_CONTROL_POINTS)),
//! so its path is a line, a quadratic Bézier or a cubic Bézier. Where
//! the ends sit and which side of a node they attach to is the
//! projection's to resolve; this module takes the resolved curve.

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

impl Curve {
    /// The curve built from its two ends and `control_points`, the
    /// shape a connector stores (0, 1 or 2 points). More than two is
    /// invalid ([`check_control_points`](crate::document::bounds::check_control_points)
    /// refuses it at load), so `None`. O(1).
    pub fn from_points(start: Vec2, control_points: &[Vec2], end: Vec2) -> Option<Curve> {
        match *control_points {
            [] => Some(Curve::Line(start, end)),
            [c] => Some(Curve::Quadratic(start, c, end)),
            [c1, c2] => Some(Curve::Cubic(start, c1, c2, end)),
            _ => None,
        }
    }

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
    pub fn speed_bound(&self) -> f32 {
        match *self {
            Curve::Line(a, b) => (b - a).length(),
            Curve::Quadratic(a, c, b) => 2.0 * (c - a).length().max((b - c).length()),
            Curve::Cubic(a, c1, c2, b) => {
                3.0 * (c1 - a).length().max((c2 - c1).length()).max((b - c2).length())
            }
        }
    }
}
