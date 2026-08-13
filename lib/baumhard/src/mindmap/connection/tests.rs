// SPDX-License-Identifier: MPL-2.0

//! Tests for anchor resolution, path sampling, Bezier math, and edge
//! hit-test. Includes performance regression guards for the long-edge
//! code paths whose drag-frame cost is governed by the invariant
//! "sample count scales linearly with path length, independently of
//! the arc-length subdivision table size".

use super::*;
use crate::mindmap::model::ControlPoint;

#[test]
fn test_anchor_top() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "top", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(140.0, 200.0));
}

#[test]
fn test_anchor_right() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "right", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(180.0, 220.0));
}

#[test]
fn test_anchor_bottom() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "bottom", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(140.0, 240.0));
}

#[test]
fn test_anchor_left() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "left", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(100.0, 220.0));
}

#[test]
fn test_anchor_auto_picks_nearest() {
    let pos = Vec2::new(0.0, 0.0);
    let size = Vec2::new(100.0, 50.0);
    // Other node is far to the right -- should pick right edge midpoint
    let other = Vec2::new(500.0, 25.0);
    let pt = resolve_anchor_point(pos, size, "auto", other);
    assert_eq!(pt, Vec2::new(100.0, 25.0)); // right edge midpoint
}

#[test]
fn test_anchor_auto_picks_top() {
    let pos = Vec2::new(0.0, 100.0);
    let size = Vec2::new(100.0, 50.0);
    // Other node is above
    let other = Vec2::new(50.0, -500.0);
    let pt = resolve_anchor_point(pos, size, "auto", other);
    assert_eq!(pt, Vec2::new(50.0, 100.0)); // top edge midpoint
}

#[test]
fn test_build_straight_path() {
    let path = build_connection_path(
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "right", // from: right anchor
        Vec2::new(200.0, 0.0),
        Vec2::new(100.0, 50.0),
        "left", // to: left anchor
        &[],
    );
    match path {
        ConnectionPath::Straight { start, end } => {
            assert_eq!(start, Vec2::new(100.0, 25.0));
            assert_eq!(end, Vec2::new(200.0, 25.0));
        }
        _ => panic!("Expected Straight path"),
    }
}

#[test]
fn test_build_cubic_path() {
    let cps = vec![
        ControlPoint { x: 50.0, y: 0.0 },
        ControlPoint { x: -50.0, y: 0.0 },
    ];
    let path = build_connection_path(
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "right",
        Vec2::new(300.0, 0.0),
        Vec2::new(100.0, 50.0),
        "left",
        &cps,
    );
    match path {
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => {
            assert_eq!(start, Vec2::new(100.0, 25.0));
            assert_eq!(end, Vec2::new(300.0, 25.0));
            // control1 = from_center + offset = (50,25) + (50,0) = (100, 25)
            assert_eq!(control1, Vec2::new(100.0, 25.0));
            // control2 = to_center + offset = (350,25) + (-50,0) = (300, 25)
            assert_eq!(control2, Vec2::new(300.0, 25.0));
        }
        _ => panic!("Expected CubicBezier path"),
    }
}

#[test]
fn test_straight_path_length() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let len = path_length(&path);
    assert!((len - 100.0).abs() < 0.01);
}

#[test]
fn test_straight_sampling() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 11); // 0, 10, 20, ..., 100
                                  // First point at start
    assert!((points[0].position.x - 0.0).abs() < 0.01);
    // Last point at or near 100
    assert!((points[10].position.x - 100.0).abs() < 0.01);
    // All y should be 0
    for p in &points {
        assert!((p.position.y).abs() < 0.01);
    }
}

#[test]
fn test_straight_sampling_diagonal() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(30.0, 40.0), // length = 50
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 6); // 0, 10, 20, 30, 40, 50
}

#[test]
fn test_collinear_bezier_length() {
    // Control points on the line -> arc length should equal straight distance
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let len = path_length(&path);
    assert!((len - 100.0).abs() < 0.5, "Expected ~100, got {}", len);
}

#[test]
fn test_curved_bezier_longer_than_straight() {
    // Control points perpendicular -> arc length > straight distance
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.0, 100.0),
        control2: Vec2::new(67.0, -100.0),
        end: Vec2::new(100.0, 0.0),
    };
    let straight_dist = 100.0f32;
    let arc_len = path_length(&path);
    assert!(
        arc_len > straight_dist,
        "Arc length {} should exceed straight {}",
        arc_len,
        straight_dist
    );
}

#[test]
fn test_curved_bezier_sampling() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.0, 100.0),
        control2: Vec2::new(67.0, -100.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    // Curved path is longer than 100, so should have more than 11 points
    assert!(points.len() > 11, "Expected >11 points, got {}", points.len());
    // First point near start
    assert!(points[0].position.distance(Vec2::ZERO) < 1.0);
}

#[test]
fn test_sample_path_zero_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 0.0, super::MAX_PATH_SAMPLES);
    assert!(points.is_empty());
}

#[test]
fn test_sample_path_degenerate() {
    // Zero-length path
    let path = ConnectionPath::Straight {
        start: Vec2::new(50.0, 50.0),
        end: Vec2::new(50.0, 50.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 1);
}

#[test]
fn test_distance_to_straight_on_path() {
    // Point lying exactly on a horizontal segment -> distance 0
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 0.0), &path);
    assert!(d.abs() < 0.01);
}

#[test]
fn test_distance_to_straight_perpendicular() {
    // Perpendicular offset of 5 above the path midpoint
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 5.0), &path);
    assert!((d - 5.0).abs() < 0.01, "expected ~5, got {}", d);
}

#[test]
fn test_distance_to_straight_past_endpoint() {
    // Point beyond `end`: distance should be to the end, not the
    // infinite line.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(110.0, 0.0), &path);
    assert!((d - 10.0).abs() < 0.01, "expected ~10, got {}", d);
}

#[test]
fn test_distance_to_straight_diagonal() {
    // Diagonal segment (0,0)->(30,40), length 50.
    // Point at (0,50): expected distance from segment ~ ?
    // Perpendicular foot on segment is at t = (0*30 + 50*40)/2500 = 0.8
    // -> closest = (24, 32), distance = sqrt(24^2 + 18^2) = sqrt(576+324) = 30
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(30.0, 40.0),
    };
    let d = distance_to_path(Vec2::new(0.0, 50.0), &path);
    assert!((d - 30.0).abs() < 0.01, "expected 30, got {}", d);
}

#[test]
fn test_distance_to_zero_length_path() {
    // Degenerate (zero-length) segment: distance is point-to-point
    let path = ConnectionPath::Straight {
        start: Vec2::new(50.0, 50.0),
        end: Vec2::new(50.0, 50.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 60.0), &path);
    assert!((d - 10.0).abs() < 0.01);
}

#[test]
fn test_distance_to_cubic_bezier_on_curve() {
    // A straight-ish cubic: control points collinear with endpoints
    // means the "curve" is effectively a line from (0,0) to (100,0).
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 0.0), &path);
    assert!(d < 0.5, "expected ~0, got {}", d);
}

#[test]
fn test_distance_to_cubic_bezier_perpendicular() {
    // Point 5 units above the midpoint of a collinear bezier (straight
    // line in practice): distance should be ~5
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 5.0), &path);
    assert!((d - 5.0).abs() < 0.5, "expected ~5, got {}", d);
}

#[test]
fn test_build_quadratic_promotion() {
    let cps = vec![ControlPoint { x: 0.0, y: 100.0 }];
    let path = build_connection_path(
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "auto",
        Vec2::new(200.0, 0.0),
        Vec2::new(100.0, 50.0),
        "auto",
        &cps,
    );
    match path {
        ConnectionPath::CubicBezier { .. } => { /* promoted correctly */ }
        _ => panic!("Expected CubicBezier from quadratic promotion"),
    }
}

// -----------------------------------------------------------------
// Performance regression guards
//
// These tests do not assert wall-clock timings (flaky under CI load).
// They assert the behavioural invariant the drag-frame sampler
// relies on: long paths must emit a sample count proportional to
// `length / spacing`, not capped at the arc-length subdivision
// table size. Breaking this reintroduces the long-connection drag
// stutter the sampler was tuned to avoid.
// -----------------------------------------------------------------

/// A 20,000-unit straight path sampled at spacing 15 must produce a
/// sample count proportional to length/spacing, not capped at
/// `ARC_LENGTH_SUBDIVISIONS` (256). Guards against a regression that
/// clamped sample count to the arc-length table size.
#[test]
fn test_sample_long_straight_scales_linearly_with_length() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(20_000.0, 0.0),
    };
    let points = sample_path(&path, 15.0, super::MAX_PATH_SAMPLES);
    // Expected: floor(20000/15) + 1 = 1334.
    assert_eq!(points.len(), 1334);
    // Way above the 256-subdivision table size — proves no clamp.
    assert!(
        points.len() > 1000,
        "sample count {} should scale with length, not subdivisions",
        points.len()
    );
}

/// A long cubic bezier's sample count is linear in path length, not in
/// the subdivision table size. If the arc-length lookup regressed to
/// walking the table per sample (O(N·subdivisions)) instead of binary
/// search, the test would still pass — the invariant we care about
/// here is that sample count itself tracks length, and that's what
/// this guards.
#[test]
fn test_sample_long_bezier_count_bounded_by_length() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(5_000.0, 800.0),
        control2: Vec2::new(15_000.0, -800.0),
        end: Vec2::new(20_000.0, 0.0),
    };
    let length = path_length(&path);
    let spacing = 15.0;
    let points = sample_path(&path, spacing, super::MAX_PATH_SAMPLES);
    let expected_floor = (length / spacing) as usize;
    // Sampler emits `floor(length/spacing) + 1` points. Allow a window
    // of ±2 to tolerate FP drift at the endpoint.
    assert!(
        points.len() >= expected_floor,
        "expected at least {}, got {}",
        expected_floor,
        points.len()
    );
    assert!(
        points.len() <= expected_floor + 2,
        "expected at most {}, got {}",
        expected_floor + 2,
        points.len()
    );
    // Sanity: we're in the "long edge" regime the sample-count
    // invariant targets.
    assert!(points.len() > 1000);
}

/// On a straight path, successive samples must be ordered along the
/// path direction. Catches an off-by-one or reversed loop in the arc
/// length → t conversion.
#[test]
fn test_sample_path_monotonic_along_straight() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(1000.0, 0.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert!(points.len() > 2);
    for pair in points.windows(2) {
        assert!(
            pair[1].position.x >= pair[0].position.x - 1e-4,
            "samples not monotonic: {:?} -> {:?}",
            pair[0].position,
            pair[1].position
        );
    }
}

/// Consecutive sample distances on a straight path should match the
/// requested spacing within floating-point tolerance. Catches any
/// accumulated FP drift regression from a naive refactor (e.g.
/// `current += spacing` instead of `i * spacing`).
#[test]
fn test_sample_path_even_spacing_within_tolerance() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(500.0, 0.0),
    };
    let spacing = 10.0;
    let points = sample_path(&path, spacing, super::MAX_PATH_SAMPLES);
    // All pairs except possibly the last must be within tolerance of
    // the requested spacing. The last pair can be shorter because the
    // tail is clamped to t=1.
    let n = points.len();
    assert!(n >= 3);
    for i in 0..(n - 2) {
        let d = points[i + 1].position.distance(points[i].position);
        assert!(
            (d - spacing).abs() < 0.01,
            "sample spacing {} at i={} deviates from {}",
            d,
            i,
            spacing
        );
    }
}

/// Negative spacing must not produce an infinite loop or a panic.
/// Current behaviour: empty Vec (matches the existing zero-spacing
/// behaviour). WASM crash guard.
#[test]
fn test_sample_path_rejects_negative_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, -1.0, super::MAX_PATH_SAMPLES);
    assert!(
        points.is_empty(),
        "negative spacing must return empty, got {} points",
        points.len()
    );
}

/// NaN spacing must not panic (NaN comparisons are always false, so
/// `spacing <= 0.0` is false — we rely on downstream guards to still
/// produce a sane result). WASM crash guard.
#[test]
fn test_sample_path_rejects_nan_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    // Must not panic — we do not care about the exact return value,
    // only that we get back to this line without an abort. A non-panic
    // outcome is the WASM-reliability invariant.
    let _ = sample_path(&path, f32::NAN, super::MAX_PATH_SAMPLES);
}

/// Spacing larger than the path length should return exactly one
/// sample (the start point). This guards the `count = 0` edge case.
#[test]
fn test_sample_path_huge_spacing_returns_start_only() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10_000.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].position, Vec2::new(0.0, 0.0));
}

/// Two calls to `sample_path` with the same inputs must produce
/// bit-identical output. Guards against a future accidental
/// randomisation (jitter, thread-local state, HashMap iteration
/// order leaking into the output).
#[test]
fn test_sample_path_deterministic_across_calls() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(100.0, 200.0),
        control2: Vec2::new(300.0, -200.0),
        end: Vec2::new(400.0, 0.0),
    };
    let a = sample_path(&path, 5.0, super::MAX_PATH_SAMPLES);
    let b = sample_path(&path, 5.0, super::MAX_PATH_SAMPLES);
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(b.iter()) {
        assert_eq!(pa.position, pb.position);
    }
}

/// `distance_to_path` on a long cubic bezier must return a finite,
/// non-NaN value. Guards against a hypothetical exponential
/// subdivision regression or NaN propagation from the sampler.
#[test]
fn test_distance_to_path_on_long_bezier_is_finite() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(25_000.0, 10_000.0),
        control2: Vec2::new(75_000.0, -10_000.0),
        end: Vec2::new(100_000.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50_000.0, 50_000.0), &path);
    assert!(d.is_finite(), "distance should be finite, got {}", d);
    assert!(d >= 0.0, "distance should be non-negative, got {}", d);
    // And the point is visibly off the curve, so non-zero.
    assert!(d > 1.0);
}

// point_at_t for label positioning along edges.

#[test]
fn point_at_t_straight_endpoints_and_midpoint() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 50.0),
    };
    assert_eq!(point_at_t(&path, 0.0), Vec2::new(0.0, 0.0));
    assert_eq!(point_at_t(&path, 1.0), Vec2::new(100.0, 50.0));
    let mid = point_at_t(&path, 0.5);
    assert!((mid.x - 50.0).abs() < 1e-5);
    assert!((mid.y - 25.0).abs() < 1e-5);
}

#[test]
fn point_at_t_cubic_bezier_endpoints() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(25.0, 100.0),
        control2: Vec2::new(75.0, 100.0),
        end: Vec2::new(100.0, 0.0),
    };
    // A cubic curve at t = 0 hits the start, at t = 1 hits the end.
    let p0 = point_at_t(&path, 0.0);
    let p1 = point_at_t(&path, 1.0);
    assert!((p0.x - 0.0).abs() < 1e-5 && (p0.y - 0.0).abs() < 1e-5);
    assert!((p1.x - 100.0).abs() < 1e-5 && (p1.y - 0.0).abs() < 1e-5);
    // And t = 0.5 sits between the control points vertically, well
    // above the straight-line midpoint.
    let mid = point_at_t(&path, 0.5);
    assert!(mid.y > 50.0, "midpoint y={} should be curved above", mid.y);
}

#[test]
fn point_at_t_clamps_out_of_range() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    // Values outside [0, 1] are clamped.
    assert_eq!(point_at_t(&path, -10.0), Vec2::new(0.0, 0.0));
    assert_eq!(point_at_t(&path, 99.0), Vec2::new(100.0, 0.0));
}

// ── Bezier math (bezier.rs) ────────────────────────────────────────

use super::bezier::{cubic_bezier_length, cubic_bezier_point, sample_cubic_bezier};
use crate::util::geometry::almost_equal;

#[test]
fn test_bezier_point_at_endpoints() {
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 50.0);
    let p2 = Vec2::new(90.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);

    let start = cubic_bezier_point(0.0, p0, p1, p2, p3);
    assert!(almost_equal(start.x, 0.0), "t=0 should return p0.x");
    assert!(almost_equal(start.y, 0.0), "t=0 should return p0.y");

    let end = cubic_bezier_point(1.0, p0, p1, p2, p3);
    assert!(almost_equal(end.x, 100.0), "t=1 should return p3.x");
    assert!(almost_equal(end.y, 0.0), "t=1 should return p3.y");
}

#[test]
fn test_bezier_point_at_midpoint_is_influenced_by_controls() {
    let p0 = Vec2::new(0.0, 0.0);
    let p3 = Vec2::new(100.0, 0.0);

    // Straight line (controls on the segment)
    let mid_straight = cubic_bezier_point(0.5, p0, p0, p3, p3);
    assert!(almost_equal(mid_straight.x, 50.0), "straight midpoint x");
    assert!(almost_equal(mid_straight.y, 0.0), "straight midpoint y");

    // Curved (controls above the line)
    let p1 = Vec2::new(10.0, 80.0);
    let p2 = Vec2::new(90.0, 80.0);
    let mid_curved = cubic_bezier_point(0.5, p0, p1, p2, p3);
    assert!(
        mid_curved.y > 30.0,
        "curved midpoint should be pulled up by control points; got {}",
        mid_curved.y
    );
}

#[test]
fn test_bezier_length_straight_line() {
    let a = Vec2::new(0.0, 0.0);
    let b = Vec2::new(100.0, 0.0);
    // Controls on the line make it degenerate into a straight segment
    let length = cubic_bezier_length(a, a, b, b);
    assert!(
        (length - 100.0).abs() < 1.0,
        "straight-line bezier should have length ~100; got {}",
        length,
    );
}

#[test]
fn test_bezier_sample_produces_points() {
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 50.0);
    let p2 = Vec2::new(90.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let spacing = 10.0;

    let samples = sample_cubic_bezier(p0, p1, p2, p3, spacing, super::MAX_PATH_SAMPLES);
    assert!(
        samples.len() > 5,
        "a 100-unit curve at spacing 10 should produce >5 samples; got {}",
        samples.len()
    );

    // First sample should be at p0
    assert!(almost_equal(samples[0].position.x, 0.0));
    assert!(almost_equal(samples[0].position.y, 0.0));
}

#[test]
fn test_bezier_sample_degenerate_returns_single_point() {
    // A zero-length curve (all points identical)
    let pt = Vec2::new(42.0, 42.0);
    let samples = sample_cubic_bezier(pt, pt, pt, pt, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(samples.len(), 1, "degenerate curve should produce single sample");
    assert!(almost_equal(samples[0].position.x, 42.0));
}

// ---- tangent / normal helpers ----

#[test]
fn tangent_at_t_straight_path_returns_endpoint_direction() {
    // For a straight path, the tangent is the normalised
    // end-minus-start vector regardless of `t`.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(10.0, 0.0),
    };
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let tangent = tangent_at_t(&path, t);
        assert!(almost_equal(tangent.x, 1.0), "t={t} x should be 1");
        assert!(almost_equal(tangent.y, 0.0), "t={t} y should be 0");
    }
}

#[test]
fn tangent_at_t_zero_length_straight_path_falls_back_to_x_axis() {
    // Coincident endpoints produce a zero-length raw tangent;
    // the fallback keeps callers from dividing by zero.
    let pt = Vec2::new(5.0, 5.0);
    let path = ConnectionPath::Straight { start: pt, end: pt };
    let tangent = tangent_at_t(&path, 0.5);
    assert_eq!(tangent, Vec2::X);
}

#[test]
fn tangent_at_t_cubic_bezier_at_endpoints_uses_analytical_derivative() {
    // At t = 0: derivative = 3(p1 - p0). At t = 1: derivative =
    // 3(p3 - p2). Normalised.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 0.0);
    let p2 = Vec2::new(20.0, 10.0);
    let p3 = Vec2::new(30.0, 10.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // t = 0: tangent ∝ (p1 - p0) = (10, 0) → normalised (1, 0).
    let t0 = tangent_at_t(&path, 0.0);
    assert!(almost_equal(t0.x, 1.0));
    assert!(almost_equal(t0.y, 0.0));
    // t = 1: tangent ∝ (p3 - p2) = (10, 0) → (1, 0).
    let t1 = tangent_at_t(&path, 1.0);
    assert!(almost_equal(t1.x, 1.0));
    assert!(almost_equal(t1.y, 0.0));
}

#[test]
fn normal_at_t_is_orthogonal_to_tangent() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(7.0, 3.0),
    };
    let tangent = tangent_at_t(&path, 0.5);
    let normal = normal_at_t(&path, 0.5);
    // Dot product of orthogonal unit vectors is 0.
    assert!(tangent.dot(normal).abs() < 1.0e-5);
    // And length is 1.
    assert!((normal.length() - 1.0).abs() < 1.0e-5);
}

#[test]
fn normal_at_t_rotates_canvas_90_clockwise_into_screen_space() {
    // A tangent pointing +X in canvas space rotates to (-0, +1)
    // by the `(x, y) → (-y, x)` formula, i.e. +Y — which on a
    // Y-down canvas lands *below* the path (the right-hand side
    // of travel in screen space). Pin the behaviour so a future
    // flip of the formula or a coordinate-system change breaks
    // this test instead of silently inverting label positioning.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(10.0, 0.0),
    };
    let normal = normal_at_t(&path, 0.5);
    assert!(almost_equal(normal.x, 0.0));
    assert!(almost_equal(normal.y, 1.0));
}

// ---- cubic_bezier_tangent analytical derivative ----

#[test]
fn cubic_bezier_tangent_matches_finite_difference() {
    // Spot-check the analytical derivative against a finite
    // difference — a single bug in the coefficients (e.g. writing
    // `2.0 * u * t` instead of `6.0 * u * t` for the middle term)
    // would break this test.
    use crate::mindmap::connection::bezier::{cubic_bezier_point, cubic_bezier_tangent};
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(1.0, 5.0);
    let p2 = Vec2::new(4.0, -2.0);
    let p3 = Vec2::new(6.0, 3.0);
    // h = 1e-3 balances truncation error (O(h² · |f‴|) ≈ 1e-4
    // on this cubic) against f32 cancellation in the central
    // difference (rounding amplification ≈ 1/(2h)). Smaller h
    // would sink under cancellation noise; larger h would let
    // truncation dominate.
    let h = 1.0e-3;
    for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
        let analytical = cubic_bezier_tangent(t, p0, p1, p2, p3);
        let fwd = cubic_bezier_point(t + h, p0, p1, p2, p3);
        let back = cubic_bezier_point(t - h, p0, p1, p2, p3);
        let fd = (fwd - back) / (2.0 * h);
        // Tolerance 1e-3 sits comfortably above the combined
        // truncation + f32 cancellation floor while still
        // catching a single-coefficient bug — e.g. a missing
        // factor of 2 or a `u*t` → `u+t` typo produces errors
        // of order 1-10.
        assert!(
            (analytical.x - fd.x).abs() < 1.0e-3,
            "t={t} x analytical {} vs fd {}",
            analytical.x,
            fd.x
        );
        assert!(
            (analytical.y - fd.y).abs() < 1.0e-3,
            "t={t} y analytical {} vs fd {}",
            analytical.y,
            fd.y
        );
    }
}

// ---- closest_point_on_path ----

#[test]
fn closest_point_on_path_straight_cursor_above_midpoint() {
    // Horizontal segment; cursor directly above the midpoint at
    // y = -5. Expected: t = 0.5, perp_offset = -5 (perp direction
    // is `(-y, x)` rotation of `(1, 0)` tangent = `(0, 1)`, so
    // cursor at `(x_mid, -5)` relative to `(x_mid, 0)` has
    // `to_cursor = (0, -5)`, perp = dot((0,-5), (0,1)) = -5).
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(50.0, -5.0));
    assert!((t - 0.5).abs() < 1.0e-5, "t={t}");
    assert!((perp - -5.0).abs() < 1.0e-5, "perp={perp}");
}

#[test]
fn closest_point_on_path_straight_cursor_beyond_end_clamps_to_1() {
    // Cursor past the `end` endpoint clamps `t` into `[0, 1]`.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let (t, _perp) = closest_point_on_path(&path, Vec2::new(200.0, 0.0));
    assert!((t - 1.0).abs() < 1.0e-5, "t={t}");
}

#[test]
fn closest_point_on_path_straight_cursor_behind_start_clamps_to_0() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(10.0, 10.0),
        end: Vec2::new(50.0, 10.0),
    };
    let (t, _perp) = closest_point_on_path(&path, Vec2::new(-40.0, 10.0));
    assert!(t < 1.0e-5, "t={t}");
}

#[test]
fn closest_point_on_path_straight_zero_length_returns_defaults() {
    // Coincident endpoints: no meaningful direction. Contract is
    // `(0, 0)` so the caller can rely on the tuple without a
    // `NaN` check.
    let pt = Vec2::new(5.0, 5.0);
    let path = ConnectionPath::Straight { start: pt, end: pt };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(20.0, 20.0));
    assert_eq!(t, 0.0);
    assert_eq!(perp, 0.0);
}

#[test]
fn closest_point_on_path_cubic_cursor_on_curve_returns_zero_perp() {
    // A cursor sitting exactly on the curve at a known `t` must
    // recover `t` (approximately) with near-zero perp.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(33.0, 50.0);
    let p2 = Vec2::new(66.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // Pick a known t, evaluate the curve there, and ask the
    // closest-point solver for that point.
    let true_t = 0.37;
    let on_curve = crate::mindmap::connection::bezier::cubic_bezier_point(true_t, p0, p1, p2, p3);
    let (t, perp) = closest_point_on_path(&path, on_curve);
    assert!(
        (t - true_t).abs() < 1.0e-3,
        "Newton should recover t within 1e-3; got {t} vs {true_t}"
    );
    assert!(
        perp.abs() < 1.0e-3,
        "perp should be ~0 when cursor is on the curve; got {perp}"
    );
}

#[test]
fn closest_point_on_path_cubic_offset_cursor_produces_signed_perp() {
    // Offset the on-curve point along the path normal by a known
    // signed distance; the solver should recover the same signed
    // perp value.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(33.0, 50.0);
    let p2 = Vec2::new(66.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    let true_t = 0.42;
    let on_curve = crate::mindmap::connection::bezier::cubic_bezier_point(true_t, p0, p1, p2, p3);
    let normal = normal_at_t(&path, true_t);
    let offset = 12.0_f32;
    let cursor = on_curve + normal * offset;
    let (t, perp) = closest_point_on_path(&path, cursor);
    assert!((t - true_t).abs() < 1.0e-2, "t={t} vs {true_t}");
    assert!((perp - offset).abs() < 1.0e-1, "perp={perp} vs {offset}");
}

#[test]
fn closest_point_on_path_cubic_degenerate_all_coincident() {
    // All four control points coincident: every evaluation of
    // B(t) returns the same point, tangent is zero everywhere.
    // Contract: `t=0`, `perp=0` — callers get a deterministic
    // fallback rather than NaN.
    let pt = Vec2::new(7.0, 11.0);
    let path = ConnectionPath::CubicBezier {
        start: pt,
        control1: pt,
        control2: pt,
        end: pt,
    };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(50.0, 50.0));
    assert!(t.is_finite());
    assert!(perp.is_finite());
    // The closest point on a degenerate curve is the coincident
    // point; perpendicular from cursor to it projected on the
    // zero tangent is zero by convention.
    assert!(
        (0.0..=1.0).contains(&t),
        "t stayed in [0,1] under degenerate curve"
    );
    assert_eq!(perp, 0.0);
}

#[test]
fn closest_point_on_path_cubic_near_inflection_never_worse_than_seed() {
    // Cubic with an inflection point — B''(t) changes sign near
    // the middle of the curve, which can send a naive Newton
    // step past the true minimum. The divergence guard compares
    // Newton's refined dist² against the sampling seed's and
    // falls back when Newton diverged. Any cursor position
    // must produce a path point whose distance is ≤ the best
    // sample's distance.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 100.0);
    let p2 = Vec2::new(90.0, -100.0); // inflection between p1 and p2
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // Cursor at a position that exercises the inflection region.
    let cursors = [
        Vec2::new(50.0, 0.0),
        Vec2::new(50.0, 10.0),
        Vec2::new(50.0, -10.0),
        Vec2::new(25.0, 30.0),
        Vec2::new(75.0, -30.0),
    ];
    for cursor in cursors {
        // Compute the sampling-only seed distance for the same
        // cursor by replicating the 32-sample sweep.
        let mut seed_best = f32::MAX;
        for i in 0..=32 {
            let t = i as f32 / 32.0;
            let d = (crate::mindmap::connection::bezier::cubic_bezier_point(t, p0, p1, p2, p3) - cursor)
                .length_squared();
            if d < seed_best {
                seed_best = d;
            }
        }
        let (t, _perp) = closest_point_on_path(&path, cursor);
        let point = crate::mindmap::connection::bezier::cubic_bezier_point(t, p0, p1, p2, p3);
        let refined_dist_sq = (point - cursor).length_squared();
        assert!(
            refined_dist_sq <= seed_best + 1.0e-3,
            "Newton must never be worse than seed: refined={} seed_best={} t={} cursor={:?}",
            refined_dist_sq,
            seed_best,
            t,
            cursor
        );
    }
}

/// **The allocation the sampler used to commit to unconditionally.**
///
/// The sample count is `path length / spacing`, and both terms come
/// out of the document — endpoints are node positions, spacing
/// derives from an authored font size. Uncapped, a long path at a
/// fine spacing asked `Vec::with_capacity` for billions of points,
/// which fails as an allocator abort rather than a catchable panic.
///
/// The loader now rejects the coordinates that get here in the first
/// place; this pins the second wall, so a path reaching the sampler
/// by some other route still cannot ask for terabytes.
#[test]
fn test_sample_path_caps_hostile_geometry() {
    let far = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(1.0e9, 0.0),
    };
    // Equality, not `<=`: the cap must *bind* here. A sampler that
    // bailed to a single point on any large quotient — an easy
    // mistake, since `sample_count` already returns 1 on four guard
    // paths — would satisfy `<=` while silently drawing nothing.
    let samples = sample_path(&far, 0.001, super::MAX_PATH_SAMPLES);
    assert_eq!(
        samples.len(),
        MAX_PATH_SAMPLES,
        "a long path at fine spacing must saturate the cap, not fall through it"
    );

    let curved = ConnectionPath::CubicBezier {
        start: Vec2::ZERO,
        control1: Vec2::new(1.0e8, 1.0e8),
        control2: Vec2::new(2.0e8, -1.0e8),
        end: Vec2::new(1.0e9, 0.0),
    };
    assert_eq!(
        sample_path(&curved, 0.001, super::MAX_PATH_SAMPLES).len(),
        MAX_PATH_SAMPLES
    );
}

/// A non-finite length or spacing must not reach the allocation
/// either. Float-to-integer casts saturate, so an infinite quotient
/// would land on `usize::MAX` and overflow the `+ 1`; a `NaN` casts
/// to zero. Both resolve to a single point instead.
#[test]
fn test_sample_path_survives_non_finite_geometry() {
    let infinite = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(f32::INFINITY, 0.0),
    };
    // Asserted as an exact count, not `<= MAX_PATH_SAMPLES`. The
    // bound is satisfied by *any* result including an empty one, so
    // it holds whether or not `sample_count`'s guards exist and
    // certifies nothing. One point is the documented outcome.
    assert_eq!(
        sample_path(&infinite, 1.0, super::MAX_PATH_SAMPLES).len(),
        1,
        "an infinite length must resolve to a single point"
    );

    let nan = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(f32::NAN, 0.0),
    };
    assert_eq!(
        sample_path(&nan, 1.0, super::MAX_PATH_SAMPLES).len(),
        1,
        "a NaN length must resolve to a single point"
    );

    // A non-finite spacing is the same hazard from the other side.
    let ordinary = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(100.0, 0.0),
    };
    assert_eq!(
        sample_path(&ordinary, f32::NAN, super::MAX_PATH_SAMPLES).len(),
        1,
        "a NaN spacing must resolve to a single point"
    );
    assert_eq!(
        sample_path(&ordinary, f32::INFINITY, super::MAX_PATH_SAMPLES).len(),
        1,
        "an infinite spacing must resolve to a single point"
    );
    // A non-positive spacing is refused by `sample_path` itself,
    // before `sample_count` is consulted — so the answer is no
    // samples at all rather than the single point the non-finite
    // cases resolve to. Asserted because the two guards are easy to
    // conflate: `sample_count`'s own `spacing <= 0.0` arm is
    // unreachable through this entry point.
    assert!(
        sample_path(&ordinary, 0.0, super::MAX_PATH_SAMPLES).is_empty(),
        "a zero spacing yields no samples, not one"
    );
    assert!(
        sample_path(&ordinary, -5.0, super::MAX_PATH_SAMPLES).is_empty(),
        "a negative spacing yields no samples, not one"
    );

    // The ordinary case still samples the way it always did.
    assert_eq!(sample_path(&ordinary, 10.0, super::MAX_PATH_SAMPLES).len(), 11);
}

/// **The clamp that must not panic on a window the cascade built
/// backwards.**
///
/// `f32::clamp` panics when its bounds cross, and a size window is
/// not assembled from one place: a label may set only
/// `min_font_size_pt` and inherit `max` from its edge. Validating
/// each struct's own pair at load therefore cannot see every window
/// the cascade can produce — the halves are individually valid and
/// jointly inverted — so the ordering is enforced where the window
/// is used.
#[test]
fn test_font_window_tolerates_an_inverted_cascade() {
    use crate::font::fonts::{clamp_to_font_window, MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};

    // Inverted: read as the window the author described.
    assert_eq!(clamp_to_font_window(50.0, 40.0, 8.0), 40.0);
    assert_eq!(clamp_to_font_window(4.0, 40.0, 8.0), 8.0);
    assert_eq!(clamp_to_font_window(20.0, 40.0, 8.0), 20.0);

    // Ordinary window is untouched.
    assert_eq!(clamp_to_font_window(20.0, 8.0, 40.0), 20.0);
    assert_eq!(clamp_to_font_window(100.0, 8.0, 40.0), 40.0);

    // Non-finite bounds are pulled into the shaper's domain first,
    // so neither the clamp nor the shaper ever sees one. A window
    // with nothing usable in it widens to the whole domain rather
    // than collapsing, so an ordinary size passes through.
    assert_eq!(clamp_to_font_window(20.0, f32::NAN, f32::NAN), 20.0);
    assert_eq!(clamp_to_font_window(0.0, f32::NAN, f32::NAN), MIN_FONT_SIZE_PT);
    assert_eq!(clamp_to_font_window(1.0e9, f32::NAN, f32::NAN), MAX_FONT_SIZE_PT);
    assert!(clamp_to_font_window(1.0e9, 8.0, f32::INFINITY) <= MAX_FONT_SIZE_PT);

    // A NaN size takes the floor rather than propagating.
    assert_eq!(clamp_to_font_window(f32::NAN, 8.0, 40.0), 8.0);
}

/// **The per-path cap bounded a path and bounded nothing.**
///
/// Each sample becomes a glyph area in the scene arena, and an edge
/// costs about 120 bytes in the file. A 73 KB document with 200 edges
/// reached 2 000 000 samples, a 2 000 201-node arena and 1 642 MiB
/// resident; with the budget it reaches 500 000 and 416 MiB, and no
/// document can exceed that however many edges it declares. The
/// per-path constant's own doc had said the aggregate was not bounded.
///
/// The share is equal per edge rather than first-come, so the outcome
/// cannot depend on iteration order and no edge renders while a later
/// one vanishes.
#[test]
fn test_the_scene_wide_glyph_budget_is_shared_equally_and_never_zero() {
    use super::{per_path_sample_budget, MAX_PATH_SAMPLES, MAX_TOTAL_PATH_SAMPLES};

    // Few edges: the per-path cap still governs, unchanged behavior.
    assert_eq!(per_path_sample_budget(1), MAX_PATH_SAMPLES);
    assert_eq!(
        per_path_sample_budget(0),
        MAX_PATH_SAMPLES,
        "no edges must not divide by zero"
    );
    assert_eq!(
        per_path_sample_budget(MAX_TOTAL_PATH_SAMPLES / MAX_PATH_SAMPLES),
        MAX_PATH_SAMPLES,
        "at the crossover the two ceilings agree"
    );

    // Many edges: the aggregate governs, and the product stays inside
    // the budget — which is the whole property.
    for edges in [200usize, 1_000, 50_000, 5_000_000] {
        let each = per_path_sample_budget(edges);
        assert!(
            each >= 1,
            "{edges} edges: an edge that renders nothing looks like a missing edge"
        );
        assert!(
            each <= MAX_PATH_SAMPLES,
            "{edges} edges: the per-path cap must still apply"
        );
        // `each * edges` can exceed the budget only through the
        // never-zero floor, which is the deliberate trade.
        if each > 1 {
            assert!(
                each.saturating_mul(edges) <= MAX_TOTAL_PATH_SAMPLES,
                "{edges} edges x {each} samples exceeds the scene budget"
            );
        }
    }

    // Real maps must be untouched by this: the repository's heaviest is
    // `stress_long_edges` at 46 290 samples over 124 edges.
    assert!(
        per_path_sample_budget(124) >= 46_290 / 124 + 1,
        "the budget must not thin the repository's own worst-case map"
    );
}
