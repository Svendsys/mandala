// SPDX-License-Identifier: MPL-2.0

//! `ArcPath::nearest` (#394, review of #398): the place along a path
//! nearest a point, found exactly on a side and on an arc, and from a
//! table's samples refined on an ellipse — never on a thin outline's far
//! side, which a sampling of the whole path at a fixed count found
//! wherever its samples lay further apart than the outline is thick.

use glam::Vec2;

use crate::document::values::{Direction, Edge, Percent, Shape, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::{outline, Outline, Rect};

/// `outline` walked clockwise from the start of its top side, its tables
/// a unit apart: how the projection measures a portal end's place.
fn walked(outline: &Outline) -> ArcPath {
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    ArcPath::closed(outline, start, Direction::Clockwise, 1.0)
}

/// A box at the origin, `w` by `h`, its corners rounded by `percent`.
fn boxed(w: f32, h: f32, percent: u8) -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(w, h)),
        Shape::Rectangle {
            radius: Percent::new(percent),
        },
    )
}

/// The distance between two places round a path `length` long, the
/// shorter way.
fn round(a: f32, b: f32, length: f32) -> f32 {
    let d = (a - b).rem_euclid(length);
    d.min(length - d)
}

/// **By a thin outline, the nearest place is on the near side**: on a
/// box 100 000 by 20, points 2 above its top side are nearest the top
/// side, right below them — not the bottom side 22 away, where a sampling
/// 2048 times round found every point of some 35 in each 98.
///
/// Control: a side read at its start alone is red.
#[test]
fn test_nearest_by_a_thin_outline_is_on_its_near_side() {
    let arc = walked(&boxed(100_000.0, 20.0, 0));
    for x in [30.0_f32, 45.0, 60.0, 140.0, 50_000.25] {
        let s = arc.nearest(Vec2::new(x, -2.0));
        assert!(
            (s - x).abs() < 1e-2,
            "the point 2 above the top side at {x} is nearest {s} along, {:?}",
            arc.at(s).0
        );
    }
}

/// **A point as near two sides is nearest the first of them**: the
/// center of a box 40 by 20, 10 from its top side and its bottom side,
/// is nearest the top side's middle, 20 along, not the bottom side's.
///
/// Control: ties kept to the last is red.
#[test]
fn test_nearest_ties_go_to_the_lowest_place() {
    let arc = walked(&boxed(40.0, 20.0, 0));
    let s = arc.nearest(Vec2::new(20.0, 10.0));
    assert!((s - 20.0).abs() < 1e-3, "the center is nearest {s} along, not 20");
}

/// **A point on the outline is nearest its own place**, on each kind of
/// piece: the sides of a square-cornered box, the arcs of a rounded one,
/// and the tables of an ellipse — every place a twentieth of the way
/// round, and the place of the point drawn there read back.
///
/// Controls: a side read at its start alone, an arc's angle read without
/// its turn, and a table's samples not refined are each red on their row.
#[test]
fn test_nearest_reads_back_each_place_on_each_kind_of_piece() {
    let rows = [
        ("square corners", boxed(80.0, 30.0, 0)),
        ("rounded corners", boxed(80.0, 30.0, 100)),
        (
            "an ellipse",
            Outline::Ellipse {
                rect: Rect::new(Vec2::new(5.0, -3.0), Vec2::new(70.0, 24.0)),
            },
        ),
    ];
    for (name, shape) in rows {
        let arc = walked(&shape);
        let length = arc.length();
        for k in 0..20 {
            let s = length * (k as f32 + 0.37) / 20.0;
            let back = arc.nearest(arc.at(s).0);
            assert!(
                round(back, s, length) < 1e-2,
                "{name}: the point {s} along reads back as {back}"
            );
        }
    }
}

/// A small deterministic source of numbers in `0..1`.
fn unit(mut seed: u32) -> impl FnMut() -> f32 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 1_000_000) as f32 / 1_000_000.0
    }
}

/// **Nearest is no further than the nearest of a dense sampling**: for
/// 300 points scattered round and inside each of a square-cornered box, a
/// rounded one, an ellipse, and a thin box, the place `nearest` finds is
/// no further from the point than the nearest of 40 000 places evenly
/// round the outline, less a margin of half their spacing.
///
/// Control: a side read at its start alone is red.
#[test]
fn test_nearest_is_no_further_than_a_dense_sampling_finds() {
    let rows = [
        ("square corners", boxed(90.0, 40.0, 0)),
        ("rounded corners", boxed(90.0, 40.0, 60)),
        (
            "an ellipse",
            Outline::Ellipse {
                rect: Rect::new(Vec2::ZERO, Vec2::new(90.0, 40.0)),
            },
        ),
        ("a thin box", boxed(9000.0, 6.0, 30)),
    ];
    let mut next = unit(394);
    for (name, shape) in rows {
        let arc = walked(&shape);
        let length = arc.length();
        let rect = shape.rect();
        let dense: Vec<Vec2> = (0..40_000)
            .map(|k| arc.at(length * k as f32 / 40_000.0).0)
            .collect();
        let spacing = length / 40_000.0;
        for _ in 0..300 {
            let p = rect.origin - 0.5 * rect.size + 2.0 * rect.size * Vec2::new(next(), next());
            let found = arc.at(arc.nearest(p)).0.distance(p);
            let brute = dense.iter().map(|q| q.distance(p)).fold(f32::INFINITY, f32::min);
            assert!(
                found <= brute + 0.5 * spacing + 1e-3,
                "{name}: {p} is {found} from where nearest finds, {brute} from the nearest of the samples"
            );
        }
    }
}
