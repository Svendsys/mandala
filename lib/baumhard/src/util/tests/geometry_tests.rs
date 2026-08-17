// SPDX-License-Identifier: MPL-2.0

use crate::util::geometry::{
    aabb_contains, almost_equal, almost_equal_vec2, clockwise_rotation_around_pivot,
    is_non_negative_finite_f64, is_positive_finite, option_almost_equal, pixel_greater_or_equal,
    pixel_greater_than, pixel_less_or_equal, pixel_lesser_than,
};
use glam::Vec2;

#[test]
fn test_90_deg_rotation() {
    do_90_deg_rotation();
}

pub fn do_90_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
    let expected = Vec2::new(0.0, -1.0);
    assert!(almost_equal_vec2(rotated, expected));
}

#[test]
fn test_180_deg_rotation() {
    do_180_deg_rotation();
}

pub fn do_180_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 180.0);
    let expected = Vec2::new(-1.0, 0.0);
    assert!(almost_equal_vec2(rotated, expected));
}

#[test]
fn test_non_origin_pivot_rotation() {
    do_non_origin_pivot_rotation();
}

pub fn do_non_origin_pivot_rotation() {
    let point = Vec2::new(2.0, 2.0);
    let pivot = Vec2::new(1.0, 1.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
    let expected = Vec2::new(2.0, 0.0);
    assert_eq!(rotated, expected);
}

#[test]
fn test_0_deg_rotation() {
    do_0_deg_rotation();
}

pub fn do_0_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 0.0);
    assert_eq!(rotated, point);
}

#[test]
fn test_pixel_functions() {
    do_pixel_functions();
}

pub fn do_pixel_functions() {
    assert!(pixel_greater_than((100.0, 100.0), (200.0, 90.0)));
    assert!(!pixel_greater_than((100.0, 100.0), (200.0, 110.0)));
    assert!(pixel_greater_than((105.0, 100.0), (100.0, 100.0)));
    assert!(pixel_greater_than((101.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_greater_than((100.0, 100.0), (100.0, 100.0)));
    assert!(pixel_greater_or_equal((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_greater_or_equal((100.0, 100.0), (100.0, 101.0)));
    assert!(pixel_greater_or_equal((100.0, 105.0), (100.0, 101.0)));
    assert!(pixel_greater_or_equal((100.0, 105.0), (100.0, 105.0)));
    assert!(pixel_greater_or_equal((101.0, 105.0), (100.0, 105.0)));
    assert!(!pixel_greater_or_equal((101.0, 105.0), (102.0, 105.0)));
    assert!(!pixel_lesser_than((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_lesser_than((100.0, 100.0), (200.0, 99.0)));
    assert!(pixel_lesser_than((100.0, 100.0), (200.0, 100.0)));
    assert!(pixel_lesser_than((100.0, 100.0), (100.0, 101.0)));
    assert!(pixel_lesser_than((200.0, 10.0), (100.0, 101.0)));
    assert!(pixel_less_or_equal((200.0, 10.0), (100.0, 101.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_less_or_equal((101.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_less_or_equal((100.0, 101.0), (100.0, 100.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (101.0, 100.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (100.0, 101.0)));
}

#[test]
fn test_almost_equal() {
    do_almost_equal();
}

pub fn do_almost_equal() {
    // Test positive cases
    assert!(almost_equal(0.000001f32, 0.000002f32));
    assert!(almost_equal(1.000001f32, 1.000002f32));
    assert!(almost_equal(-1.000001f32, -1.000002f32));

    // Test negative cases
    assert!(!almost_equal(0.1f32, 0.2f32));
    assert!(!almost_equal(1.1f32, 1.2f32));
    assert!(!almost_equal(-1.1f32, -1.2f32));
    assert!(!almost_equal(95.0, 105.0));
    assert!(!almost_equal(105.0, 95.0));
}

#[test]
fn test_option_almost_equal() {
    do_option_almost_equal();
}

pub fn do_option_almost_equal() {
    // Both `None` is equal — the "value not set on either side"
    // case the per-axis edge / portal-label setters short-circuit on.
    assert!(option_almost_equal(None, None));

    // Both `Some` and within tolerance.
    assert!(option_almost_equal(Some(1.0), Some(1.000001)));
    assert!(option_almost_equal(Some(-0.5), Some(-0.500001)));

    // Both `Some` and clearly outside tolerance.
    assert!(!option_almost_equal(Some(0.0), Some(0.1)));
    assert!(!option_almost_equal(Some(1.0), Some(2.0)));

    // Mismatched tags — never equal.
    assert!(!option_almost_equal(None, Some(0.0)));
    assert!(!option_almost_equal(Some(0.0), None));
}
#[test]
fn test_almost_equal_vec2() {
    do_almost_equal_vec2();
}

pub fn do_almost_equal_vec2() {
    // Test positive cases
    assert!(almost_equal_vec2(
        Vec2::new(0.000001f32, 0.000003f32),
        Vec2::new(0.000002f32, 0.000004f32)
    ));

    assert!(almost_equal_vec2(
        Vec2::new(1.000001f32, 1.000003f32),
        Vec2::new(1.000002f32, 1.000004f32)
    ));

    assert!(almost_equal_vec2(
        Vec2::new(-1.000001f32, -1.000003f32),
        Vec2::new(-1.000002f32, -1.000004f32)
    ));

    // Test negative cases
    assert!(!almost_equal_vec2(
        Vec2::new(0.1f32, 0.2f32),
        Vec2::new(0.2f32, 0.3f32)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(95.0, 150.0),
        Vec2::new(105.0, 150.0)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(1.1f32, 1.2f32),
        Vec2::new(1.2f32, 1.3f32)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(-1.1f32, -1.2f32),
        Vec2::new(-1.2f32, -1.3f32)
    ));
}

#[test]
fn test_is_positive_finite() {
    do_is_positive_finite();
}

pub fn do_is_positive_finite() {
    // Strictly positive finite values pass.
    assert!(is_positive_finite(0.000001));
    assert!(is_positive_finite(1.0));
    assert!(is_positive_finite(1e30));

    // Zero, negative, NaN, ±∞ all reject.
    assert!(!is_positive_finite(0.0));
    assert!(!is_positive_finite(-0.0));
    assert!(!is_positive_finite(-0.000001));
    assert!(!is_positive_finite(-1.0));
    assert!(!is_positive_finite(f32::NAN));
    assert!(!is_positive_finite(f32::INFINITY));
    assert!(!is_positive_finite(f32::NEG_INFINITY));
}

#[test]
fn test_is_non_negative_finite_f64() {
    do_is_non_negative_finite_f64();
}

pub fn do_is_non_negative_finite_f64() {
    // Zero passes — the predicate is `>= 0.0`, distinct from
    // `is_positive_finite`'s strict `> 0.0`.
    assert!(is_non_negative_finite_f64(0.0));
    assert!(is_non_negative_finite_f64(-0.0));
    assert!(is_non_negative_finite_f64(0.000001));
    assert!(is_non_negative_finite_f64(1.0));
    assert!(is_non_negative_finite_f64(1e300));

    // Negative + non-finite all reject.
    assert!(!is_non_negative_finite_f64(-0.000001));
    assert!(!is_non_negative_finite_f64(-1.0));
    assert!(!is_non_negative_finite_f64(f64::NAN));
    assert!(!is_non_negative_finite_f64(f64::INFINITY));
    assert!(!is_non_negative_finite_f64(f64::NEG_INFINITY));
}

#[test]
fn test_aabb_contains_includes_every_boundary() {
    do_aabb_contains_includes_every_boundary();
}

/// The interval is closed on all four sides, and that is the whole
/// point of the function: a click landing exactly on a node's right
/// or bottom edge has to hit the node rather than fall through to
/// whatever is beneath it.
///
/// The input that makes this fail is a half-open `<` on either
/// upper bound — the shape a reader reaching for "width" instead of
/// "max" writes — which drops the right and bottom edges while
/// leaving the left and top ones working, so all four corners are
/// checked rather than one. The degenerate box is the same claim at
/// zero size: it must contain its single point, which a half-open
/// test says is empty.
pub fn do_aabb_contains_includes_every_boundary() {
    let min = Vec2::new(-3.0, 5.0);
    let max = Vec2::new(7.0, 11.0);

    assert!(aabb_contains(Vec2::new(2.0, 8.0), min, max), "interior");
    for corner in [
        Vec2::new(min.x, min.y),
        Vec2::new(max.x, min.y),
        Vec2::new(min.x, max.y),
        Vec2::new(max.x, max.y),
    ] {
        assert!(
            aabb_contains(corner, min, max),
            "corner {corner:?} is inside a closed box"
        );
    }
    assert!(aabb_contains(Vec2::new(max.x, 8.0), min, max), "right edge");
    assert!(aabb_contains(Vec2::new(2.0, max.y), min, max), "bottom edge");

    let point = Vec2::new(4.0, -1.0);
    assert!(
        aabb_contains(point, point, point),
        "a zero-size box contains its own point"
    );
}

#[test]
fn test_aabb_contains_rejects_on_each_axis_independently() {
    do_aabb_contains_rejects_on_each_axis_independently();
}

/// Outside on one axis is outside, even when the other axis is
/// comfortably inside. All four directions are checked because an
/// implementation that drops one axis's pair of comparisons
/// entirely still passes every case on the other axis: without the
/// `y` clauses `(2.0, 4.999)` and `(2.0, 11.001)` both come back
/// inside, and without the `x` clauses `(-3.001, 8.0)` does.
///
/// A *transposed* compare — `point.y` against `max.x` — is not this
/// test's to catch, and could not be: every assertion here is a
/// rejection, and a transposition only makes the predicate
/// stricter. `test_aabb_contains_includes_every_boundary` is where
/// it turns red, on the corners.
///
/// The `max < min` case is the last direction: a box whose bounds
/// are inverted contains nothing, which the closed comparisons give
/// without a guard of their own. An implementation that "helpfully"
/// sorted its bounds would report the point inside.
pub fn do_aabb_contains_rejects_on_each_axis_independently() {
    let min = Vec2::new(-3.0, 5.0);
    let max = Vec2::new(7.0, 11.0);

    assert!(!aabb_contains(Vec2::new(-3.001, 8.0), min, max), "left of min.x");
    assert!(!aabb_contains(Vec2::new(7.001, 8.0), min, max), "right of max.x");
    assert!(!aabb_contains(Vec2::new(2.0, 4.999), min, max), "above min.y");
    assert!(!aabb_contains(Vec2::new(2.0, 11.001), min, max), "below max.y");

    assert!(
        !aabb_contains(Vec2::new(2.0, 8.0), max, min),
        "an inverted box contains nothing, including its own former interior"
    );
}
