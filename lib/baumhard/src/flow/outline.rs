// SPDX-License-Identifier: MPL-2.0

//! Outlines: the closed paths a frame's rings run around (design §6.3,
//! D11), and the rectangle they are derived from.
//!
//! A node's outline is its drawn rectangle shaped by its
//! [`Shape`]: a rounded rectangle whose corner radius is a percentage
//! of half the rectangle's shorter side, or the inscribed ellipse.
//! Ring *k* runs around the outline grown outward by [`ring_offset`].

use glam::Vec2;

use crate::document::values::Shape;

/// An axis-aligned rectangle on the canvas: its top-left corner and its
/// size. The node's drawn rectangle — position plus fitted or minimum
/// size, which the projection computes — is one of these.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    /// The top-left corner.
    pub origin: Vec2,
    /// The width and height, each non-negative.
    pub size: Vec2,
}

impl Rect {
    /// The rectangle with top-left corner `origin` and `size`. O(1).
    pub const fn new(origin: Vec2, size: Vec2) -> Self {
        Rect { origin, size }
    }

    /// The center. O(1).
    pub fn center(&self) -> Vec2 {
        self.origin + self.size * 0.5
    }

    /// The bottom-right corner. O(1).
    pub fn max(&self) -> Vec2 {
        self.origin + self.size
    }

    /// The smallest rectangle holding both. O(1).
    pub fn union(&self, other: &Rect) -> Rect {
        let origin = self.origin.min(other.origin);
        Rect::new(origin, self.max().max(other.max()) - origin)
    }

    /// The rectangle grown by `d` on every side, about the same center.
    /// A negative `d` shrinks it, and a side shrunk past zero stays at
    /// zero, centered; a `d` that is no number shrinks it so too.
    ///
    /// **Past what an `f32` holds** (#364), an axis whose center is a
    /// finite number saturates rather than leave a far corner that is no
    /// number (an origin of −∞ plus a size of +∞). Its origin is the near
    /// edge it reaches, held to `f32::MIN`; its size runs from there to
    /// the far edge it reaches, `+∞` where that width is past `f32`. Every
    /// corner is then a number, the far one `+∞` at most, and the box
    /// covers every point it reaches: on an axis of width `+∞`, every
    /// point from its near edge on — past its far edge too, as no finite
    /// width can reach that edge from the near one. It is a rectangle
    /// reaching `+∞`, which the indices take as a rectangle that far
    /// (`hit::is_rectangle`). Grown by `+∞` it runs from `f32::MIN` to
    /// `+∞`: every finite point.
    ///
    /// An axis that grows within `f32`, and one whose center is itself
    /// past `f32` or no number, is computed as it always was, to the bit:
    /// its size never negative. O(1).
    pub fn grown(&self, d: f32) -> Rect {
        let size = (self.size + Vec2::splat(2.0 * d)).max(Vec2::ZERO);
        let center = self.center();
        let origin = center - size * 0.5;
        if origin.is_finite() && size.is_finite() {
            return Rect { origin, size };
        }
        // An axis past `f32`: its near edge from the half width, which an
        // `f32` holds wherever `d` does, and its size from the near edge
        // to the far one, `+∞` where the far edge is past `f32` too. Its
        // center finite, the near edge is at most the far one.
        let half = (self.size * 0.5 + Vec2::splat(d)).max(Vec2::ZERO);
        let axis = |origin: f32, size: f32, center: f32, half: f32| {
            if (origin.is_finite() && size.is_finite()) || !center.is_finite() {
                (origin, size)
            } else {
                let near = (center - half).max(f32::MIN);
                (near, (center + half) - near)
            }
        };
        let (x, width) = axis(origin.x, size.x, center.x, half.x);
        let (y, height) = axis(origin.y, size.y, center.y, half.y);
        Rect {
            origin: Vec2::new(x, y),
            size: Vec2::new(width, height),
        }
    }
}

/// A closed outline: what a frame's ring is placed around.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Outline {
    /// A rectangle with circular corners of `radius`, between `0` (sharp)
    /// and half the shorter side (a circle on a square, a pill on a
    /// longer rectangle).
    RoundedRect {
        /// The rectangle the outline is drawn on.
        rect: Rect,
        /// The corner radius, in canvas units.
        radius: f32,
    },
    /// The ellipse inscribed in `rect`.
    Ellipse {
        /// The rectangle the ellipse is inscribed in.
        rect: Rect,
    },
}

impl Outline {
    /// The rectangle the outline is drawn on. O(1).
    pub fn rect(&self) -> Rect {
        match *self {
            Outline::RoundedRect { rect, .. } | Outline::Ellipse { rect } => rect,
        }
    }

    /// The corner radius actually drawn: `radius` held to
    /// `0..=min(w, h)/2`, so a hand-built outline cannot ask for corners
    /// that overlap. `0` for an ellipse. O(1).
    pub fn corner_radius(&self) -> f32 {
        match *self {
            Outline::RoundedRect { rect, radius } => {
                let limit = rect.size.x.min(rect.size.y) * 0.5;
                if radius.is_nan() {
                    0.0
                } else {
                    radius.clamp(0.0, limit.max(0.0))
                }
            }
            Outline::Ellipse { .. } => 0.0,
        }
    }

    /// The outline's support point in direction `dir`: its point farthest
    /// along `dir` — on an ellipse where the outward normal is `dir`, on
    /// a rounded rectangle the point of its corner arc facing `dir` (a
    /// sharp rectangle's corner). Where a frame's two lines meet (#291,
    /// ruling R33): the supports in `(−1, −1)` and `(1, 1)`. The center
    /// for a `dir` of no length, or one not a number. O(1).
    pub fn support(&self, dir: Vec2) -> Vec2 {
        let rect = self.rect();
        let center = rect.center();
        let Some(unit) = dir.try_normalize() else {
            return center;
        };
        match *self {
            Outline::RoundedRect { .. } => {
                let r = self.corner_radius();
                let half = (rect.size * 0.5 - Vec2::splat(r)).max(Vec2::ZERO);
                let corner = center + Vec2::new(half.x * unit.x.signum0(), half.y * unit.y.signum0());
                corner + unit * r
            }
            Outline::Ellipse { .. } => {
                let radii = rect.size * 0.5;
                let stretched = radii * radii * unit;
                let reach = (radii * unit).length();
                if reach > 0.0 {
                    center + stretched / reach
                } else {
                    center
                }
            }
        }
    }
}

/// `-1`, `0` or `1` by the sign of a number, `0` for zero and a number
/// that is not one: which side of a corner a support direction faces.
trait Signum0 {
    fn signum0(self) -> f32;
}

impl Signum0 for f32 {
    fn signum0(self) -> f32 {
        if self > 0.0 {
            1.0
        } else if self < 0.0 {
            -1.0
        } else {
            0.0
        }
    }
}

/// The outline of a node drawn in `rect` with `shape` (D11).
///
/// `Rectangle { radius }` gives a rounded rectangle whose corner radius
/// is `radius / 100 × min(w, h) / 2`: 0% is sharp, 100% a circle on a
/// square and a pill on a longer rectangle (the percentage is capped at
/// 100 by [`Percent`](crate::document::values::Percent) itself).
/// `Ellipse` gives the ellipse inscribed in `rect`. O(1).
pub fn outline(rect: Rect, shape: Shape) -> Outline {
    match shape {
        Shape::Rectangle { radius } => Outline::RoundedRect {
            rect,
            radius: f32::from(radius.get()) / 100.0 * rect.size.x.min(rect.size.y) * 0.5,
        },
        Shape::Ellipse => Outline::Ellipse { rect },
    }
}

/// `o` moved outward by `d` (inward for a negative `d`).
///
/// A rounded rectangle's rectangle grows by `d` on every side and its
/// radius by `d` — the exact offset curve of a rounded rectangle, so a
/// sharp rectangle's outer rings have corners of radius `d`, and the
/// radius stays within half the grown rectangle's shorter side. An
/// ellipse's rectangle grows by `d` (an ellipse again, as #239 §3
/// specifies, rather than the exact offset curve of an ellipse, which is
/// not one). Shrinking stops at an empty rectangle and a zero radius.
/// O(1).
pub fn offset(o: &Outline, d: f32) -> Outline {
    match *o {
        Outline::RoundedRect { rect, .. } => {
            let grown = rect.grown(d);
            let radius = (o.corner_radius() + d).clamp(0.0, grown.size.x.min(grown.size.y) * 0.5);
            Outline::RoundedRect { rect: grown, radius }
        }
        Outline::Ellipse { rect } => Outline::Ellipse { rect: rect.grown(d) },
    }
}

/// `o` moved outward by `d` (inward for a negative `d`) with its corners
/// kept: a rounded rectangle's rectangle grows by `d` on every side and
/// its radius stays, so a sharp rectangle grows into a sharp rectangle;
/// an ellipse grows as [`offset`] grows it. Where a directional ring runs
/// (#324): its corners are where its sides' strokes meet, as a box's are.
/// The radius stays within half the grown rectangle's shorter side, and
/// shrinking stops at an empty rectangle. O(1).
pub fn offset_keeping_corners(o: &Outline, d: f32) -> Outline {
    match *o {
        Outline::RoundedRect { rect, .. } => {
            let grown = rect.grown(d);
            let radius = o.corner_radius().clamp(0.0, grown.size.x.min(grown.size.y) * 0.5);
            Outline::RoundedRect { rect: grown, radius }
        }
        Outline::Ellipse { .. } => offset(o, d),
    }
}

/// How far outside the outline ring *k* runs, where *k* is the length
/// of `inner_line_heights`, the line heights of the rings inside it,
/// innermost first: `inset + Σ inner_line_heights + k · gap` (#239 §5).
///
/// Ring *k* is placed on `offset(outline, ring_offset(...))`. A ring
/// rescaled by `Fit` contributes the line height it was placed at — the
/// run placement returned — so each ring clears the one inside it as
/// drawn.
///
/// Cost: O(k).
pub fn ring_offset(inset: f32, gap: f32, inner_line_heights: &[f32]) -> f32 {
    inset + inner_line_heights.iter().sum::<f32>() + inner_line_heights.len() as f32 * gap
}
