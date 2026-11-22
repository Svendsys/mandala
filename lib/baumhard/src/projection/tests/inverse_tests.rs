// SPDX-License-Identifier: MPL-2.0

//! The inverses a label drag and a portal-end drag write from (V1-C7,
//! #248): `geometry::label_at` undoes `geometry::label_center`, and
//! `geometry::portal_at` undoes `geometry::portal_point` at the place
//! `geometry::portal_along` gives a set `border_t`. A drag stages the
//! pair the inverse reads off the pointer, so a label or an end the
//! projection then places sits under the pointer.

use glam::Vec2;

use crate::document::resolve::BorderPoint;
use crate::document::values::{Percent, Shape};
use crate::flow::{outline, Curve, Rect};
use crate::projection::geometry::{label_at, label_center, portal_along, portal_at, portal_point};

/// A gentle S between two facing sides, and a two-curve chain with a
/// bend: the shapes a connector draws.
fn paths() -> Vec<Vec<Curve>> {
    vec![
        vec![Curve::Cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(80.0, 0.0),
            Vec2::new(120.0, 90.0),
            Vec2::new(200.0, 90.0),
        )],
        vec![
            Curve::Quadratic(Vec2::new(0.0, 0.0), Vec2::new(60.0, -30.0), Vec2::new(120.0, 0.0)),
            Curve::Line(Vec2::new(120.0, 0.0), Vec2::new(120.0, 140.0)),
        ],
    ]
}

/// **A label placed at `(t, offset)` reads back as `(t, offset)`.** For
/// every place on each path — offsets either side of it and on it — the
/// point the projection centers the label at, handed to the inverse,
/// gives the place again: so a drag that stages what the inverse reads
/// off the pointer puts the label's center under the pointer.
///
/// Control: the point moved by the curve's right normal instead reads a
/// different offset (the sign is the inverse's, not a coincidence).
#[test]
fn test_label_at_reads_back_the_place_label_center_puts_a_label() {
    for path in paths() {
        for t in [0.1_f32, 0.35, 0.5, 0.8] {
            for offset in [-9.0_f32, 0.0, 6.5] {
                let point = label_center(&path, t, offset);
                let (t2, offset2) = label_at(&path, point);
                assert!((t2 - t).abs() < 0.01, "t {t} read back as {t2} on {path:?}");
                assert!(
                    (offset2 - offset).abs() < 0.5,
                    "offset {offset} at t {t} read back as {offset2}"
                );
            }
        }
        let on = label_center(&path, 0.5, 0.0);
        let left = label_center(&path, 0.5, 8.0);
        let right = on - (left - on);
        let (_, read) = label_at(&path, right);
        assert!(
            (read + 8.0).abs() < 0.5,
            "the right side reads a negative offset, got {read}"
        );
    }
}

/// A path of no length reads the place the projection reads a `t` that
/// is not a number as: the middle, on the path.
#[test]
fn test_label_at_on_an_empty_path_is_the_middle() {
    assert_eq!(label_at(&[], Vec2::new(4.0, 5.0)), (0.5, 0.0));
    // Control: a real path reads somewhere else for the same point.
    assert_ne!(label_at(&paths()[0], Vec2::new(4.0, 5.0)), (0.5, 0.0));
}

/// **A portal end placed at `(border_t, offset)` reads back as
/// `(border_t, offset)`**, round a sharp rectangle, a rounded one and an
/// ellipse, on the outline, outside it and inside it, at places away
/// from the corners (where two places of the outline are equally near a
/// point outside the corner and either is right).
///
/// Control: a point moved inward reads a negative offset.
#[test]
fn test_portal_at_reads_back_the_place_portal_point_puts_an_end() {
    let rect = Rect::new(Vec2::new(40.0, 20.0), Vec2::new(200.0, 100.0));
    for shape in [
        Shape::Rectangle { radius: Percent::new(0) },
        Shape::Rectangle { radius: Percent::new(50) },
        Shape::Ellipse,
    ] {
        let line = outline(rect, shape);
        for t in [0.07_f32, 0.3, 0.55, 0.9] {
            let (s, _) = portal_along(&line, BorderPoint::At(t), Vec2::ZERO);
            for offset in [0.0_f32, 5.0, 12.0] {
                let (point, _) = portal_point(&line, s, offset);
                let (t2, offset2) = portal_at(&line, point);
                assert!(
                    (t2 - t).abs() < 0.005,
                    "border_t {t} read back as {t2} round {shape:?}"
                );
                assert!(
                    (offset2 - offset).abs() < 0.5,
                    "offset {offset} read back as {offset2} round {shape:?}"
                );
            }
        }
        let (s, _) = portal_along(&line, BorderPoint::At(0.3), Vec2::ZERO);
        let (on, normal) = portal_point(&line, s, 0.0);
        let (_, inside) = portal_at(&line, on - normal * 6.0);
        assert!((inside + 6.0).abs() < 0.5, "inside reads negative, got {inside}");
    }
}
