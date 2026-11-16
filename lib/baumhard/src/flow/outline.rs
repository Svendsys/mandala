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
    /// **Past what an `f32` holds** (#364), an axis saturates rather than
    /// leave a corner that is no number (an origin of −∞ plus a size of
    /// +∞, or +∞ minus +∞). Its origin is the near edge it reaches, held
    /// to `f32::MIN`; its size runs from there to the far edge it reaches,
    /// `+∞` where that width or that edge is past `f32`. Every corner is
    /// then a number, the far one `+∞` at most, and the box covers every
    /// point it reaches: on an axis of width `+∞`, every point from its
    /// near edge on — past its far edge too, wherever the width it reaches
    /// is more than `f32::MAX`, as no finite width reaches that edge from
    /// the near one. It is a rectangle reaching `+∞`, which the indices
    /// take as a rectangle that far (`hit::is_rectangle`). Grown by `+∞`
    /// it runs from `f32::MIN` to `+∞`: every finite point. The edges come
    /// from the center where an `f32` holds it, and otherwise — a size of
    /// `+∞`, as this saturation gives, or a far corner past `f32` — from
    /// the origin and the far corner. So `grown` of a rectangle, by any
    /// `d`, is a rectangle: grown again, it covers what it reaches.
    ///
    /// Computed as it always was, to the bit:
    ///
    /// - an axis that grows within `f32`, which is every axis of a
    ///   loadable map;
    /// - a shrink to nothing, or a `d` that is no number, which leaves the
    ///   axis empty at its center — `+∞` where the center is past `f32`,
    ///   as a box reaching `+∞` shrunk by `+∞` is;
    /// - an axis that is no rectangle — an origin or a size that is no
    ///   number, or a negative size — as there is nothing it reaches.
    ///
    /// An axis at ±∞ (an origin of ±∞, which covers no finite point)
    /// keeps its origin and grows its size as it always did, except where
    /// the far corner would be no number: grown to a width of `+∞` at −∞,
    /// it stays a point there, its width `0`, and at +∞ its origin stays
    /// +∞ whatever its width. O(1).
    pub fn grown(&self, d: f32) -> Rect {
        let size = (self.size + Vec2::splat(2.0 * d)).max(Vec2::ZERO);
        let center = self.center();
        let origin = center - size * 0.5;
        if origin.is_finite() && size.is_finite() {
            return Rect { origin, size };
        }
        let (x, width) = grown_axis(self.origin.x, self.size.x, d, (origin.x, size.x), center.x);
        let (y, height) = grown_axis(self.origin.y, self.size.y, d, (origin.y, size.y), center.y);
        Rect {
            origin: Vec2::new(x, y),
            size: Vec2::new(width, height),
        }
    }
}

/// One axis of [`Rect::grown`] that the plain arithmetic does not hold:
/// `origin` and `size` grown by `d`, given what the plain arithmetic gave
/// (`plain`, its origin and size) and the axis's `center`. See
/// [`Rect::grown`] for what each case gives and why. O(1).
fn grown_axis(origin: f32, size: f32, d: f32, plain: (f32, f32), center: f32) -> (f32, f32) {
    let (plain_origin, plain_size) = plain;
    if plain_origin.is_finite() && plain_size.is_finite() {
        // This axis grew within `f32`; the other did not.
        return plain;
    }
    if origin.is_nan() || !(size >= 0.0) {
        // No rectangle on this axis: as it always was.
        return plain;
    }
    if origin.is_infinite() {
        // At ±∞: the origin kept, the width as it always grew, but none
        // that would make the far corner −∞ + +∞.
        return if (origin + plain_size).is_nan() {
            (origin, 0.0)
        } else {
            (origin, plain_size)
        };
    }
    if center.is_finite() {
        // The near edge from the half width, which an `f32` holds wherever
        // `d` does; the size from the near edge to the far one, `+∞` where
        // that width is past `f32`. A finite center lies between them.
        let half = (size * 0.5 + d).max(0.0);
        let near = (center - half).max(f32::MIN);
        return (near, (center + half) - near);
    }
    // The origin finite and the center past `f32`: a size more than twice
    // the room from the origin to `f32::MAX`, `+∞` among them, as
    // `origin + size / 2` overflows. Shrunk to nothing, or by a `d` that
    // is no number, it is empty at its center, `+∞`, as it always was —
    // as is a size of `+∞` shrunk by `+∞`, which reaches no width a
    // number gives. The half width is read, not `size + 2·d`, as `2·d`
    // overflows where the shrink it is does not empty a size of `+∞`.
    if !(size * 0.5 + d >= 0.0) {
        return plain;
    }
    // Otherwise the edges from the origin and the far corner: the near one
    // held to `f32::MIN`, the far one past `f32` (it lies past the center),
    // so the size is `+∞` — or nothing, at `+∞`, where the near edge is
    // past `f32` too.
    let near = (origin - d).max(f32::MIN);
    if near == f32::INFINITY {
        return (near, 0.0);
    }
    (near, (origin + size + d) - near)
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
