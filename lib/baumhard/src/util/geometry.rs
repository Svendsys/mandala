// SPDX-License-Identifier: MPL-2.0

//! Small-scale 2D geometry helpers: rotation around a pivot,
//! epsilon-aware float comparisons, and pixel-space ordering.

use glam::{Mat3, Vec2};

/// Rotate `a` clockwise by `degrees` around `pivot`, returning the
/// transformed point. Uses `glam::Mat3::from_rotation_z` internally;
/// O(1).
pub fn clockwise_rotation_around_pivot(a: Vec2, pivot: Vec2, degrees: f32) -> Vec2 {
    let translated = a - pivot;

    let radians = -degrees.to_radians();
    let rotation = Mat3::from_rotation_z(radians);

    let result = rotation.transform_point2(translated) + pivot;

    result
}

const ERROR_TOLERANCE_ALMOST_EQUAL: f32 = 1e-5;

/// `|a - b| <= 1e-5`. The baumhard-wide epsilon for "close enough"
/// between two `f32`s. Cost: O(1).
pub fn almost_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= ERROR_TOLERANCE_ALMOST_EQUAL
}

/// `true` iff `f` is a non-NaN, non-infinite, strictly-positive
/// `f32`. The canonical predicate for "is this a valid pixel /
/// zoom / scale / font-size value coming from user input?" —
/// rejects `NaN`, `±∞`, and zero-or-negative numbers.
///
/// Used on the parse → mutation boundary (every console verb
/// that accepts a numeric pt size, every edge-style setter
/// that accepts a positive measurement). Cost: O(1).
pub fn is_positive_finite(f: f32) -> bool {
    f.is_finite() && f > 0.0
}

/// `true` iff `f` is a non-NaN, non-infinite, non-negative
/// `f64`. The canonical predicate for "is this a coordinate /
/// size safe to feed into layout math?" — rejects `NaN`,
/// `±∞`, and negative numbers, but allows zero (a zero-width
/// node or zero-coord position is a valid layout input).
///
/// Used by `document::mutations::*` layout helpers that walk
/// `MindNode.size` / `position` (both `f64`) before applying
/// derived math; a non-finite or negative coord fed into
/// trigonometry produces NaN that propagates into every
/// downstream row, so the safe-coord check stops the cascade
/// at the source. Cost: O(1).
pub fn is_non_negative_finite_f64(f: f64) -> bool {
    f.is_finite() && f >= 0.0
}

/// Logical inverse of [`almost_equal`]. Named "pretty" because it
/// treats within-epsilon pairs as equal rather than fighting with raw
/// `!=` comparisons at float boundaries. Cost: O(1).
pub fn pretty_inequal(a: f32, b: f32) -> bool {
    !almost_equal(a, b)
}

/// Pixel-reading-order `>=` on `(x, y)` pairs: y-dominant, x as
/// tie-breaker, using [`almost_equal`] for the equality test.
/// Cost: O(1).
pub fn pixel_greater_or_equal(a_greater_or: (f32, f32), equal_b: (f32, f32)) -> bool {
    pixel_greater_than(a_greater_or, equal_b)
        || (almost_equal(a_greater_or.0, equal_b.0) && almost_equal(a_greater_or.1, equal_b.1))
}

/// Pixel-reading-order `>` on `(x, y)` pairs: if the y components are
/// almost-equal, compare x; otherwise compare y. Matches how a cursor
/// walks a page of glyphs. Cost: O(1).
pub fn pixel_greater_than(a_greater: (f32, f32), than_b: (f32, f32)) -> bool {
    if almost_equal(a_greater.1, than_b.1) {
        a_greater.0 > than_b.0
    } else {
        a_greater.1 > than_b.1
    }
}

/// Pixel-reading-order `<=` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_or_equal`]. Cost: O(1).
pub fn pixel_less_or_equal(a_less_or: (f32, f32), equal_b: (f32, f32)) -> bool {
    pixel_lesser_than(a_less_or, equal_b)
        || (almost_equal(a_less_or.0, equal_b.0) && almost_equal(a_less_or.1, equal_b.1))
}

/// Pixel-reading-order `<` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_than`]. Cost: O(1).
pub fn pixel_lesser_than(a_lesser: (f32, f32), than_b: (f32, f32)) -> bool {
    if almost_equal(a_lesser.1, than_b.1) {
        a_lesser.0 < than_b.0
    } else {
        a_lesser.1 < than_b.1
    }
}

/// Area of the rectangle whose width / height are the x / y
/// components of `vec`. O(1).
pub fn vec2_area(vec: Vec2) -> f32 {
    vec.x * vec.y
}

/// AABB centre from a top-left position + size pair. O(1), no
/// heap. Equivalent to [`crate::mindmap::model::MindNode::center_vec2`]
/// for the case where only the geometry is in scope (anchor
/// resolution paths, scene-builder portal-pair midpoint compute);
/// where a `MindNode` is in scope, prefer the method.
#[inline]
pub fn aabb_center(pos: Vec2, size: Vec2) -> Vec2 {
    pos + size * 0.5
}

/// Component-wise [`pretty_inequal`] on two vectors: true if either
/// component pair is outside the `almost_equal` tolerance.
/// Cost: O(1).
pub fn pretty_inequal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
    pretty_inequal(vec1.x, vec2.x) || pretty_inequal(vec1.y, vec2.y)
}

/// Component-wise [`almost_equal`] on two vectors: true iff both
/// component pairs are within tolerance. Cost: O(1).
pub fn almost_equal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
    almost_equal(vec1.x, vec2.x) && almost_equal(vec1.y, vec2.y)
}

/// Option-aware [`almost_equal`]: `(None, None) -> true`,
/// `(Some(a), Some(b)) -> almost_equal(a, b)`, mismatched-tag
/// pairs (`Some` vs `None`) -> false.
///
/// Used by per-axis edge / portal-label setters to short-circuit
/// "no-op writes" when the user re-types a value that's
/// within-epsilon of the stored one. Centralised here so the
/// setter no-op contract is single-sourced across edges/style.rs,
/// edge_label_drag, portal_label_drag, and the connection cache's
/// font-size diff. Cost: O(1).
pub fn option_almost_equal(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => almost_equal(x, y),
        _ => false,
    }
}
