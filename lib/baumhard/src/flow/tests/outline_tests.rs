// SPDX-License-Identifier: MPL-2.0

//! Outlines and paths: the corner radius (D11), offsets and ring
//! distances, the perimeter parameterization start points use, and the
//! arc-length walk of ellipses and Béziers.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

use glam::Vec2;

use crate::document::values::{Direction, Edge, Percent, Shape, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::tests::support::{close, de_casteljau, de_casteljau_derivative};
use crate::flow::{offset, outline, ring_offset, Curve, Outline, Rect};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(Vec2::new(x, y), Vec2::new(w, h))
}

fn rounded(percent: u8) -> Shape {
    Shape::Rectangle {
        radius: Percent::new(percent),
    }
}

fn start(edge: Edge, at: f32) -> StartPoint {
    StartPoint { edge, at }
}

#[test]
fn test_flow_outline_corner_radius_is_a_share_of_half_the_shorter_side() {
    do_flow_outline_corner_radius_is_a_share_of_half_the_shorter_side();
}

/// D11: the radius is `percent / 100 × min(w, h) / 2` — on a 200 × 100
/// rectangle 0% is sharp, 50% is 25 and 100% is 50 (a pill) — and an
/// ellipse is inscribed in the same rectangle.
pub fn do_flow_outline_corner_radius_is_a_share_of_half_the_shorter_side() {
    let r = rect(10.0, 20.0, 200.0, 100.0);
    for (percent, radius) in [(0, 0.0), (50, 25.0), (100, 50.0)] {
        assert_eq!(
            outline(r, rounded(percent)),
            Outline::RoundedRect { rect: r, radius },
            "{percent}%"
        );
    }
    let tall = rect(0.0, 0.0, 40.0, 300.0);
    assert_eq!(
        outline(tall, rounded(100)),
        Outline::RoundedRect {
            rect: tall,
            radius: 20.0
        },
        "the shorter side is the width here"
    );
    assert_eq!(outline(r, Shape::Ellipse), Outline::Ellipse { rect: r });
}

#[test]
fn test_flow_offset_grows_the_rect_and_the_radius() {
    do_flow_offset_grows_the_rect_and_the_radius();
}

/// An offset rounded rectangle grows by `d` on every side and its radius
/// by `d`; an ellipse's rectangle grows by `d`. Shrinking past nothing
/// stops at an empty rectangle and a zero radius.
pub fn do_flow_offset_grows_the_rect_and_the_radius() {
    let r = rect(0.0, 0.0, 100.0, 60.0);
    assert_eq!(
        offset(&outline(r, rounded(0)), 5.0),
        Outline::RoundedRect {
            rect: rect(-5.0, -5.0, 110.0, 70.0),
            radius: 5.0
        }
    );
    assert_eq!(
        offset(&outline(r, rounded(50)), 5.0),
        Outline::RoundedRect {
            rect: rect(-5.0, -5.0, 110.0, 70.0),
            radius: 20.0
        }
    );
    assert_eq!(
        offset(&Outline::Ellipse { rect: r }, 3.0),
        Outline::Ellipse {
            rect: rect(-3.0, -3.0, 106.0, 66.0)
        }
    );
    assert_eq!(
        offset(&outline(r, rounded(50)), -40.0),
        Outline::RoundedRect {
            rect: rect(40.0, 30.0, 20.0, 0.0),
            radius: 0.0
        }
    );
}

#[test]
fn test_flow_ring_offset_sums_inner_rings_and_gaps() {
    do_flow_ring_offset_sums_inner_rings_and_gaps();
}

/// Ring *k* runs at `inset + Σ h_j + k · gap`: the innermost at the
/// inset, the third past two rings and two gaps.
pub fn do_flow_ring_offset_sums_inner_rings_and_gaps() {
    assert_eq!(ring_offset(2.0, 3.0, &[]), 2.0);
    assert_eq!(ring_offset(2.0, 3.0, &[10.0]), 15.0);
    assert_eq!(ring_offset(2.0, 3.0, &[10.0, 20.0]), 38.0);
}

#[test]
fn test_flow_pill_has_straight_sides_of_w_minus_h() {
    do_flow_pill_has_straight_sides_of_w_minus_h();
}

/// Radius 100% on a 2:1 rectangle is a pill: from the start of the top
/// side, the path runs straight and level for exactly `w − h` and then
/// turns, and the whole perimeter is `2(w − h) + π·h`.
pub fn do_flow_pill_has_straight_sides_of_w_minus_h() {
    let (w, h) = (200.0, 100.0);
    let pill = outline(rect(0.0, 0.0, w, h), rounded(100));
    let path = ArcPath::closed(&pill, start(Edge::Top, 0.0), Direction::Clockwise, 1.0);
    close(path.length(), 2.0 * (w - h) + PI * h, 1e-3, "perimeter");
    let (origin, _) = path.at(0.0);
    assert_eq!(origin, Vec2::new(50.0, 0.0));
    for s in [0.0, 50.0, w - h - 1e-3] {
        let (point, tangent) = path.at(s);
        close(point.y, 0.0, 1e-4, "on the top side");
        close(tangent.x, 1.0, 1e-6, "level");
    }
    let (_, turning) = path.at(w - h + 1.0);
    assert!(turning.y > 0.01, "past the straight it turns down: {turning}");
}

#[test]
fn test_flow_start_points_on_a_rounded_rect() {
    do_flow_start_points_on_a_rounded_rect();
}

/// A start sits at `at` along its side's straight stretch, each side
/// measured clockwise: the top from the left, the right from the top,
/// the bottom from the right, the left from the bottom. With radius
/// 100% on a square there is no straight stretch, so every `at` is the
/// side's beginning.
pub fn do_flow_start_points_on_a_rounded_rect() {
    let sharp = outline(rect(0.0, 0.0, 100.0, 60.0), rounded(0));
    let begins = |o: &Outline, edge, at| {
        ArcPath::closed(o, start(edge, at), Direction::Clockwise, 1.0)
            .at(0.0)
            .0
    };
    assert_eq!(begins(&sharp, Edge::Top, 0.25), Vec2::new(25.0, 0.0));
    assert_eq!(begins(&sharp, Edge::Right, 0.5), Vec2::new(100.0, 30.0));
    assert_eq!(begins(&sharp, Edge::Bottom, 0.25), Vec2::new(75.0, 60.0));
    assert_eq!(begins(&sharp, Edge::Left, 0.25), Vec2::new(0.0, 45.0));
    let round = outline(rect(0.0, 0.0, 80.0, 80.0), rounded(100));
    for at in [0.0, 0.5, 1.0] {
        assert_eq!(begins(&round, Edge::Right, at), Vec2::new(80.0, 40.0), "at {at}");
    }
}

#[test]
fn test_flow_start_points_on_an_ellipse() {
    do_flow_start_points_on_an_ellipse();
}

/// On an ellipse a start is the parametric angle `at` of the way through
/// its side's quarter (top 225°–315°, right 315°–405°, bottom 45°–135°,
/// left 135°–225°), with `y` downward so a growing angle is clockwise.
pub fn do_flow_start_points_on_an_ellipse() {
    let ellipse = Outline::Ellipse {
        rect: rect(0.0, 0.0, 200.0, 100.0),
    };
    let begins = |edge, at| ArcPath::closed(&ellipse, start(edge, at), Direction::Clockwise, 1.0).at(0.0);
    let near = |a: Vec2, b: Vec2, what: &str| assert!(a.distance(b) < 1e-3, "{what}: {a} vs {b}");
    near(begins(Edge::Top, 0.5).0, Vec2::new(100.0, 0.0), "top middle");
    near(begins(Edge::Right, 0.5).0, Vec2::new(200.0, 50.0), "right middle");
    near(
        begins(Edge::Bottom, 0.5).0,
        Vec2::new(100.0, 100.0),
        "bottom middle",
    );
    near(begins(Edge::Left, 0.5).0, Vec2::new(0.0, 50.0), "left middle");
    near(
        begins(Edge::Top, 0.0).0,
        Vec2::new(100.0 - 100.0 * FRAC_1_SQRT_2, 50.0 - 50.0 * FRAC_1_SQRT_2),
        "top's start, 225°",
    );
    // Clockwise from the top middle is rightward; counter-clockwise,
    // leftward.
    assert!(begins(Edge::Top, 0.5).1.x > 0.99);
    let ccw = ArcPath::closed(&ellipse, start(Edge::Top, 0.5), Direction::CounterClockwise, 1.0).at(0.0);
    assert!(ccw.1.x < -0.99);
}

#[test]
fn test_flow_ellipse_walk_stays_on_the_ellipse() {
    do_flow_ellipse_walk_stays_on_the_ellipse();
}

/// Every point of an ellipse's arc-length walk is on the ellipse (its
/// points are evaluated on the curve, not on the table's chords), its
/// direction is the ellipse's tangent there, and its length is the
/// ellipse's perimeter — checked against Ramanujan's second
/// approximation, which is exact to far below a pixel at this
/// eccentricity.
pub fn do_flow_ellipse_walk_stays_on_the_ellipse() {
    let (a, b) = (150.0_f32, 60.0_f32);
    let ellipse = Outline::Ellipse {
        rect: rect(-a, -b, 2.0 * a, 2.0 * b),
    };
    let path = ArcPath::closed(&ellipse, start(Edge::Right, 0.3), Direction::Clockwise, 2.0);
    let hh = ((a - b) / (a + b)).powi(2);
    let ramanujan = PI * (a + b) * (1.0 + 3.0 * hh / (10.0 + (4.0 - 3.0 * hh).sqrt()));
    close(path.length(), ramanujan, ramanujan * 1e-4, "perimeter");
    for i in 0..200 {
        let s = path.length() * i as f32 / 200.0;
        let (p, t) = path.at(s);
        close((p.x / a).powi(2) + (p.y / b).powi(2), 1.0, 1e-4, "on the ellipse");
        // The tangent is perpendicular to the ellipse's gradient there.
        let gradient = Vec2::new(p.x / (a * a), p.y / (b * b)).normalize();
        close(t.dot(gradient), 0.0, 1e-3, "tangent");
    }
}

#[test]
fn test_flow_bezier_walk_measures_arc_length() {
    do_flow_bezier_walk_measures_arc_length();
}

/// A cubic's arc-length walk measures its length — against a Simpson
/// integral of its speed, the curve evaluated here by de Casteljau
/// rather than through `Curve` — and every point it yields is on the
/// curve: the nearest of 20 000 dense de Casteljau samples is within a
/// hundredth of a point. Distance is also uniform: two points `Δs` apart
/// along the walk are at most `Δs` apart in the plane and nearly that
/// on a gentle curve.
pub fn do_flow_bezier_walk_measures_arc_length() {
    let points = [
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, -80.0),
        Vec2::new(220.0, 160.0),
        Vec2::new(320.0, 40.0),
    ];
    let curve = Curve::Cubic(points[0], points[1], points[2], points[3]);
    let path = ArcPath::open(&curve, 1.5);
    let steps = 20_000;
    let speed = |t: f32| de_casteljau_derivative(&points, t).length();
    let mut integral = speed(0.0) + speed(1.0);
    for i in 1..steps {
        let t = i as f32 / steps as f32;
        integral += speed(t) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    integral /= 3.0 * steps as f32;
    close(path.length(), integral, integral * 1e-4, "length");

    let dense: Vec<Vec2> = (0..=steps)
        .map(|i| de_casteljau(&points, i as f32 / steps as f32))
        .collect();
    for i in 0..=100 {
        let s = path.length() * i as f32 / 100.0;
        let (p, _) = path.at(s);
        let nearest = dense.iter().map(|q| q.distance(p)).fold(f32::MAX, f32::min);
        assert!(nearest < 0.01, "s = {s}: {nearest} off the curve");
    }
    let (p0, _) = path.at(100.0);
    let (p1, _) = path.at(110.0);
    let chord = p0.distance(p1);
    assert!(
        chord <= 10.0 + 1e-3 && chord > 9.9,
        "10 along the walk is {chord} in the plane"
    );
}

#[test]
fn test_flow_curve_point_and_derivative_by_hand() {
    do_flow_curve_point_and_derivative_by_hand();
}

/// `Curve`'s point, derivative and speed bound against values worked by
/// hand, so the curve tests that compare against de Casteljau are not
/// the only witness:
///
/// - the cubic (0,0) (100,−80) (220,160) (320,40) at ½ is
///   `(P0 + 3P1 + 3P2 + P3)/8 = (160, 35)`, its derivative
///   `¾(P3 + P2 − P1 − P0) = (330, 210)`, and its speed bound three
///   times its longest leg, `3·√(120² + 240²)`;
/// - the quadratic (0,0) (150,−200) (300,0) at ½ is `(150, −100)`, its
///   derivative `(C − A) + (B − C) = (300, 0)`;
/// - the line (0,0) (4,8) at ¼ is (1, 2), its derivative (4, 8).
pub fn do_flow_curve_point_and_derivative_by_hand() {
    let near = |a: Vec2, b: Vec2| assert!(a.distance(b) < 1e-3, "{a} vs {b}");
    let cubic = Curve::Cubic(
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, -80.0),
        Vec2::new(220.0, 160.0),
        Vec2::new(320.0, 40.0),
    );
    near(cubic.point(0.5), Vec2::new(160.0, 35.0));
    near(cubic.derivative(0.5), Vec2::new(330.0, 210.0));
    close(
        cubic.speed_bound(),
        3.0 * 72_000.0_f32.sqrt(),
        1e-3,
        "cubic speed bound",
    );
    let quadratic = Curve::Quadratic(Vec2::ZERO, Vec2::new(150.0, -200.0), Vec2::new(300.0, 0.0));
    near(quadratic.point(0.5), Vec2::new(150.0, -100.0));
    near(quadratic.derivative(0.5), Vec2::new(300.0, 0.0));
    let line = Curve::Line(Vec2::ZERO, Vec2::new(4.0, 8.0));
    near(line.point(0.25), Vec2::new(1.0, 2.0));
    near(line.derivative(0.25), Vec2::new(4.0, 8.0));
}

#[test]
fn test_flow_open_path_holds_its_ends() {
    do_flow_open_path_holds_its_ends();
}

/// An open path walked past either end stays at that end, keeping its
/// direction, where a closed one goes round again.
pub fn do_flow_open_path_holds_its_ends() {
    let path = ArcPath::open(&Curve::Line(Vec2::ZERO, Vec2::new(0.0, 50.0)), 1.0);
    assert_eq!(path.at(80.0), (Vec2::new(0.0, 50.0), Vec2::Y));
    assert_eq!(path.at(-5.0), (Vec2::ZERO, Vec2::Y));
    let square = outline(rect(0.0, 0.0, 10.0, 10.0), rounded(0));
    let closed = ArcPath::closed(&square, start(Edge::Top, 0.0), Direction::Clockwise, 1.0);
    assert_eq!(closed.at(45.0).0, Vec2::new(5.0, 0.0), "a closed path goes round");
}

#[test]
fn test_flow_curve_from_points() {
    do_flow_curve_from_points();
}

/// Zero, one and two control points are a line, a quadratic and a
/// cubic; a third is invalid.
pub fn do_flow_curve_from_points() {
    let (a, c, d, b) = (Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE);
    assert_eq!(Curve::from_points(a, &[], b), Some(Curve::Line(a, b)));
    assert_eq!(Curve::from_points(a, &[c], b), Some(Curve::Quadratic(a, c, b)));
    assert_eq!(Curve::from_points(a, &[c, d], b), Some(Curve::Cubic(a, c, d, b)));
    assert_eq!(Curve::from_points(a, &[c, d, c], b), None);
}
