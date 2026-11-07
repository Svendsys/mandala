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
pub(crate) fn drawn_rect(position: Vec2, min_size: Option<Vec2>, extents: impl Iterator<Item = Vec2>) -> Rect {
    let mut size = usable_size(min_size);
    for far in extents {
        if far.is_finite() {
            size = size.max(far);
        }
    }
    Rect::new(usable_point(position), size.max(Vec2::ZERO))
}

/// The midpoints of `rect`'s top, right, bottom and left sides: where a
/// path attaches. Each lies on the node's outline whatever its shape —
/// a rounded rectangle's straight sides and an inscribed ellipse both
/// pass through them. O(1).
pub(crate) fn side_midpoints(rect: Rect) -> [Vec2; 4] {
    let c = rect.center();
    let max = rect.max();
    [
        Vec2::new(c.x, rect.origin.y),
        Vec2::new(max.x, c.y),
        Vec2::new(c.x, max.y),
        Vec2::new(rect.origin.x, c.y),
    ]
}

/// Where a path attaches to the node drawn in `rect` (#241 §2): the
/// midpoint of the side `anchor` names, or for [`Anchor::Auto`] the side
/// midpoint nearest the other end's center (the first on a tie, top
/// before right, bottom and left). O(1).
pub(crate) fn anchor_point(rect: Rect, anchor: Anchor, other_center: Vec2) -> Vec2 {
    let [top, right, bottom, left] = side_midpoints(rect);
    match anchor {
        Anchor::Top => top,
        Anchor::Right => right,
        Anchor::Bottom => bottom,
        Anchor::Left => left,
        Anchor::Auto => {
            let mut best = top;
            let mut best_d = top.distance_squared(other_center);
            for point in [right, bottom, left] {
                let d = point.distance_squared(other_center);
                if d < best_d {
                    best = point;
                    best_d = d;
                }
            }
            best
        }
    }
}

/// The curve of a path from the node drawn in `from` to the node drawn
/// in `to` (#241 §2): from the `from` end's anchor point to the `to`
/// end's, a line with no control points, a quadratic Bézier with one —
/// an offset from `from`'s center — and a cubic with two — the first an
/// offset from `from`'s center, the second from `to`'s. `None` for more
/// than two, which no loaded document has. O(1).
pub(crate) fn curve_between(
    from: Rect,
    to: Rect,
    anchors: (Anchor, Anchor),
    control_points: &[Vec2],
) -> Option<Curve> {
    let start = anchor_point(from, anchors.0, to.center());
    let end = anchor_point(to, anchors.1, from.center());
    Some(match *control_points {
        [] => Curve::Line(start, end),
        [c] => Curve::Quadratic(start, from.center() + usable_point(c), end),
        [c1, c2] => Curve::Cubic(
            start,
            from.center() + usable_point(c1),
            to.center() + usable_point(c2),
            end,
        ),
        _ => return None,
    })
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
pub(crate) fn label_center(curve: &Curve, t: f32, offset: f32) -> Vec2 {
    let arc = ArcPath::open(curve, POINT_TABLE_SPACING);
    let t = if t.is_nan() { 0.5 } else { t.clamp(0.0, 1.0) };
    let (point, tangent) = arc.at(t * arc.length());
    let offset = if offset.is_finite() { offset } else { 0.0 };
    point + left_normal(tangent) * offset
}

/// Where a portal end sits on `outline` (#241 §2), and the outline's
/// outward normal there: at `border` — a fraction of the perimeter from
/// the top side's start, clockwise — or, for [`BorderPoint::Auto`],
/// facing `toward`, the other end's center: where the ray from the
/// outline's center through it leaves the outline. The point is moved
/// `offset` along the normal. Cost: one arc-length walk of the outline (a
/// table for an ellipse) and a lookup, and for `Auto` a nearest-point
/// search.
pub(crate) fn portal_point(outline: &Outline, border: BorderPoint, toward: Vec2, offset: f32) -> (Vec2, Vec2) {
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    let arc = ArcPath::closed(outline, start, Direction::Clockwise, POINT_TABLE_SPACING);
    let s = match border {
        BorderPoint::At(t) => {
            let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
            t * arc.length()
        }
        BorderPoint::Auto => arc.nearest(ray_exit(outline, toward)),
    };
    let offset = if offset.is_finite() { offset } else { 0.0 };
    let point = arc.offset_at(s, offset);
    (point.point, point.normal)
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
