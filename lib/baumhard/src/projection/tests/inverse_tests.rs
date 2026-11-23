// SPDX-License-Identifier: MPL-2.0

//! The inverses a label drag and a portal-end drag write from (V1-C7,
//! #248): `geometry::label_at` undoes `geometry::label_center`, and
//! `geometry::portal_at` undoes where the projection draws a portal end
//! whose `border_t` is set — `geometry::portal_point` at the place
//! `geometry::portal_along` gives it, and the reach past it that the
//! marker or the label is centered at. A drag stages the pair the inverse
//! reads off the pointer, so a label or an end the projection then
//! places sits under the pointer. Each is checked where it is *drawn*:
//! the pair it returns, placed forward again, against the point it was
//! handed. Also `geometry::knots`, the points `geometry::path_through`
//! passes through, which `Projector::through_points` reads.

use glam::Vec2;

use crate::document::resolve::BorderPoint;
use crate::document::values::{Percent, Shape};
use crate::flow::{outline, Curve, Outline, Rect};
use crate::projection::geometry::{
    knots, label_at, label_center, path_through, portal_along, portal_at, portal_point, reach,
};

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

/// The three outlines a node draws round the rectangle `(40, 20)` to
/// `(240, 120)`: sharp corners, rounded ones and an ellipse.
fn outlines() -> Vec<(Shape, Outline)> {
    let rect = Rect::new(Vec2::new(40.0, 20.0), Vec2::new(200.0, 100.0));
    [
        Shape::Rectangle {
            radius: Percent::new(0),
        },
        Shape::Rectangle {
            radius: Percent::new(50),
        },
        Shape::Ellipse,
    ]
    .into_iter()
    .map(|shape| (shape, outline(rect, shape)))
    .collect()
}

/// How far past a portal end's place what a drag holds is drawn, for the
/// normal there.
type Reach = Box<dyn Fn(Vec2) -> f32>;

/// The reaches a drag takes off past the end's place: none, a marker's
/// (a box of 14 by 18, its reach turning with the normal), and a label's
/// past it (the marker's twice, an offset of 3 and a 40 by 16 label's
/// reach) — what `Projector::portal_place_at` hands the inverse.
fn reaches() -> Vec<(&'static str, Reach)> {
    let marker = Vec2::new(14.0, 18.0);
    let label = Vec2::new(40.0, 16.0);
    vec![
        ("none", Box::new(|_| 0.0)),
        ("marker", Box::new(move |n| reach(n, marker))),
        (
            "label",
            Box::new(move |n| 2.0 * reach(n, marker) + 3.0 + reach(n, label)),
        ),
    ]
}

/// Where the projection draws what is `reach` past a portal end at
/// `(border_t, offset)` round `line` (`stages::portal_place`, for an end
/// whose `border_t` is set).
fn drawn(line: &Outline, (border_t, offset): (f32, f32), reach: &dyn Fn(Vec2) -> f32) -> Vec2 {
    let (s, _) = portal_along(line, BorderPoint::At(border_t), Vec2::ZERO);
    let (point, normal) = portal_point(line, s, offset);
    point + normal * reach(normal)
}

/// **What the projection draws for the pair the inverse reads is under
/// the point it read**, round each outline and for each reach: at places
/// all round — on sides, near and at the corners, where a sharp corner's
/// offset curve sweeps — on the outline and outside it, the point drawn
/// for `(border_t, offset)`, read back and drawn again, is the same
/// point; and a re-read of that is the same pair, so a re-grab that does
/// not move moves nothing.
///
/// Control: the point moved inward reads a smaller offset (the sign is
/// the inverse's, not a coincidence). Seen red: the inverse with no solve
/// for an offset inward (`geometry::solve_offset`), at the places where
/// the offset curve folds round a sharp corner.
#[test]
fn test_portal_at_draws_the_end_where_it_was_read() {
    for (shape, line) in outlines() {
        for (name, reach) in reaches() {
            for t in [0.0_f32, 0.07, 0.124, 0.13, 0.3, 0.375, 0.55, 0.62, 0.9, 0.995] {
                for offset in [-6.0_f32, 0.0, 5.0, 12.0, 40.0] {
                    let target = drawn(&line, (t, offset), reach.as_ref());
                    let read = portal_at(&line, target, reach.as_ref());
                    let again = drawn(&line, read, reach.as_ref());
                    assert!(
                        again.distance(target) < 0.05,
                        "{name} at ({t}, {offset}) round {shape:?}: drawn at {again}, read at {target} ({read:?})"
                    );
                    let reread = portal_at(&line, again, reach.as_ref());
                    assert!(
                        drawn(&line, reread, reach.as_ref()).distance(again) < 0.05,
                        "{name}: a re-read of ({t}, {offset}) round {shape:?} moved"
                    );
                }
            }
        }
        let (s, _) = portal_along(&line, BorderPoint::At(0.3), Vec2::ZERO);
        let (on, normal) = portal_point(&line, s, 0.0);
        let (_, inside) = portal_at(&line, on - normal * 6.0, |_| 0.0);
        assert!((inside + 6.0).abs() < 0.5, "inside reads negative, got {inside}");
    }
}

/// **In a sharp corner's wedge the end is drawn under the pointer, by
/// the first estimate**: for pointers below and right of the bottom-right
/// corner, right of the top-right one and above and left of the top-left
/// one — none on a side's normal, every one past the reach — what the
/// projection draws for the pair read is where the pointer is, for each
/// reach, and the corner's swept normal and the place found on the
/// offset curve put it there with no solve (`geometry::PORTAL_SOLVES`):
/// the solve is for an offset inward, where the curve folds.
///
/// Controls: the inverse that ignores the corner — the outline's place
/// nearest the pointer and its distance along that place's normal, less
/// the reach (the shape #395's review measured 88 units off) — misses
/// by more than a unit, so these pointers are where the corner matters;
/// and seen red, the corner-blind normal (the place's own) and the place
/// on the outline (not the offset curve) each leave the end to the solve.
#[test]
fn test_portal_at_in_a_sharp_corners_wedge_is_under_the_pointer() {
    use crate::projection::geometry::PORTAL_SOLVES;
    let (_, line) = outlines().remove(0);
    // Each pointer with the corner it is beside, by its `border_t` round
    // the 600-unit perimeter clockwise from the top-left corner.
    let pointers = [
        (Vec2::new(240.0 + 40.0, 120.0 + 60.0), 300.0 / 600.0),
        (Vec2::new(240.0 + 90.0, 120.0 + 15.0), 300.0 / 600.0),
        (Vec2::new(240.0 + 50.0, 20.0 - 50.0), 200.0 / 600.0),
        (Vec2::new(40.0 - 45.0, 20.0 - 70.0), 0.0),
    ];
    for (name, reach) in reaches() {
        for (pointer, corner_t) in pointers {
            let solves = PORTAL_SOLVES.with(|count| count.get());
            let read = portal_at(&line, pointer, reach.as_ref());
            let at = drawn(&line, read, reach.as_ref());
            assert!(
                at.distance(pointer) < 0.05,
                "{name}: drawn at {at}, the pointer at {pointer} ({read:?})"
            );
            assert_eq!(
                PORTAL_SOLVES.with(|count| count.get()),
                solves,
                "{name}: the first estimate drew the end at {pointer}, with no solve"
            );
            // Every point of the wedge is nearest the corner itself.
            let (s, _) = portal_along(&line, BorderPoint::At(corner_t), Vec2::ZERO);
            let (corner, normal) = portal_point(&line, s, 0.0);
            let blind = (corner_t, (pointer - corner).dot(normal) - reach(normal));
            let missed = drawn(&line, blind, reach.as_ref());
            assert!(
                missed.distance(pointer) > 1.0,
                "control ({name}): the corner-blind inverse lands at {missed}, the pointer at {pointer}"
            );
        }
    }
}

/// An outline of no length — a node drawn with no area — and a pointer
/// that is not finite read the top side's start, on the outline; a reach
/// that is not finite is none.
///
/// Controls: a real outline and a finite pointer read somewhere else.
#[test]
fn test_portal_at_on_no_outline_or_no_pointer_is_the_start() {
    let none = outline(Rect::new(Vec2::new(10.0, 10.0), Vec2::ZERO), Shape::Ellipse);
    let flat = outline(
        Rect::new(Vec2::new(10.0, 10.0), Vec2::ZERO),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let point = Vec2::new(300.0, 50.0);
    assert_eq!(portal_at(&none, point, |_| 0.0), (0.0, 0.0));
    assert_eq!(portal_at(&flat, point, |_| 0.0), (0.0, 0.0));
    let (_, line) = outlines().remove(0);
    for bad in [Vec2::new(f32::NAN, 4.0), Vec2::new(4.0, f32::INFINITY)] {
        assert_eq!(portal_at(&line, bad, |_| 0.0), (0.0, 0.0), "{bad}");
    }
    assert_ne!(portal_at(&line, point, |_| 0.0), (0.0, 0.0), "control");
    assert_eq!(
        portal_at(&line, point, |_| f32::NAN),
        portal_at(&line, point, |_| 0.0),
        "a reach that is not finite is none"
    );
}

/// **`knots` is where `path_through` passes**: its two ends, and each
/// control point's point — the end of the chain's body curve before it,
/// whatever leads the chain has — for chains with leads, with none and
/// with several control points.
///
/// Control: the control point's offset itself (not added to its place on
/// the chord) is no point of the chain.
#[test]
fn test_knots_are_the_points_path_through_passes() {
    let (start, end) = (Vec2::new(0.0, 0.0), Vec2::new(300.0, 60.0));
    for (via, lead) in [
        (vec![], 0.0_f32),
        (vec![Vec2::new(0.0, -40.0)], 12.0),
        (vec![Vec2::new(10.0, 30.0), Vec2::new(-20.0, -50.0)], 0.0),
        (
            vec![Vec2::new(5.0, 5.0), Vec2::new(0.0, 80.0), Vec2::new(-5.0, 0.0)],
            20.0,
        ),
    ] {
        let curves = path_through(
            (start, Vec2::new(1.0, 0.0), lead),
            (end, Vec2::new(-1.0, 0.0), lead),
            &via,
            1.0,
        );
        let points = knots(start, end, &via);
        assert_eq!(points.len(), via.len() + 2);
        assert_eq!(points[0], curves[0].point(0.0));
        assert_eq!(
            *points.last().expect("an end"),
            curves.last().expect("a curve").point(1.0)
        );
        let ends: Vec<Vec2> = curves.iter().map(|c| c.point(1.0)).collect();
        for (k, point) in points[1..=via.len()].iter().enumerate() {
            assert!(
                ends.iter().any(|e| e.distance(*point) < 1e-3),
                "knot {k} {point} is no curve's end in {curves:?}"
            );
            if via[k].length() > 1.0 {
                assert!(
                    !ends.iter().any(|e| e.distance(via[k]) < 1e-3),
                    "control: the bare offset {} is no point of the chain",
                    via[k]
                );
            }
        }
    }
}
