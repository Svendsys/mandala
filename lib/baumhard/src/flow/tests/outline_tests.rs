// SPDX-License-Identifier: MPL-2.0

//! Outlines and paths: the corner radius (D11), offsets and ring
//! distances, the perimeter parameterization start points use, and the
//! arc-length walk of ellipses and Béziers.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

use glam::Vec2;

use crate::document::values::{Direction, Edge, Percent, Shape, StartPoint};
use crate::flow::path::{direction_of, length_of, ArcPath};
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
fn test_flow_a_rect_grown_past_f32_saturates_and_covers_what_it_reaches() {
    do_flow_a_rect_grown_past_f32_saturates_and_covers_what_it_reaches();
}

/// One axis of `origin`/`size` grown by `d`, in `f64`, which holds every
/// width an `f32` grows to: its near and far edges, from the origin and
/// the far corner, the near one held to `f32::MIN` as no `f32` reaches
/// past it, and both the center where a shrink passes zero. A finite
/// origin and a `d` that is a number.
fn reach(origin: f32, size: f32, d: f32) -> (f64, f64) {
    let (origin, size, d) = (f64::from(origin), f64::from(size), f64::from(d));
    if size + 2.0 * d < 0.0 {
        let center = origin + size / 2.0;
        return (center, center);
    }
    ((origin - d).max(f64::from(f32::MIN)), origin + size + d)
}

/// `Rect::grown` past what an `f32` holds (#364): grown by a `d` that
/// takes its width or its near edge past `f32` — 1.8·10³⁸, `f32::MAX`
/// and +∞ from a box near the origin, 10³⁸ from one at the low edge of
/// `f32` — no corner is no number: the origin is the near edge it
/// reaches, held to `f32::MIN`, and the far corner the far edge, or +∞
/// where the width or that edge is past `f32`. So it covers what it
/// reaches, each edge read in `f64` here, not from the box. Grown within
/// `f32` — by 0, 5, 10³⁷ and 1.6·10³⁸, whose width an `f32` still holds,
/// and by 10³⁸ from a box at the high edge, whose far corner reaches +∞
/// — its edges are where `f64` puts them; shrunk by 3, 10³⁰, `f32::MAX`
/// and +∞, a side stops at zero about the center; and by a `d` that is
/// no number it is empty at its center. A rectangle with a finite origin
/// and a center past `f32` — reaching +∞ as the box grown by 1.8·10³⁸
/// does, grown again by 0, 5, −10³⁸ and −1.8·10³⁸; one with a size of
/// +∞ grown by 5; one at 3·10³⁸, 10³⁸ wide, grown by 0, 5 and 1.8·10³⁸ —
/// is held the same way, its edges from its origin and its far corner. A
/// box whose origin is ±∞ or no number, as the flow is handed, has a
/// size never negative.
///
/// Controls: with `grown` as it was, its origin `center − size / 2`, red
/// on the box grown by 1.8·10³⁸, whose origin is −∞; with that check
/// taken out, on its far corner, −∞ + +∞, no number. With the rows near
/// the origin taken out too, red on the box at the low edge of `f32`,
/// whose origin overflows to −∞, on its near edge; with that taken out,
/// on its far corner, −∞, short of the far edge it reaches. Saturated to
/// an origin of `f32::MIN` and a size of `f32::MAX`, or of +∞, red on the
/// near edge of the box grown by 1.8·10³⁸, which it reaches at −1.8·10³⁸.
/// Saturating an axis whose origin is −∞ from its center too, red on the
/// box with an origin of −∞ grown by 5, whose size is then −∞ (which
/// panicked the flow's `f32::clamp` of an offset's radius). Saturating
/// only an axis whose center is finite, as this first did, red on the
/// rectangle reaching +∞ grown by 0, whose origin is then +∞ less a width
/// of +∞; and with a shrink read as `size + 2·d`, which overflows, red on
/// it shrunk by 1.8·10³⁸, emptied at +∞ where it reaches from 0 on.
pub fn do_flow_a_rect_grown_past_f32_saturates_and_covers_what_it_reaches() {
    let (max, inf, nan) = (f32::MAX, f32::INFINITY, f32::NAN);
    let r = rect(10.0, -20.0, 30.0, 4.0);
    // Literal rows: grown within `f32`, as it always was.
    assert_eq!(r.grown(0.0), r, "grown by 0");
    assert_eq!(r.grown(5.0), rect(5.0, -25.0, 40.0, 14.0), "grown by 5");
    assert_eq!(r.grown(-3.0), rect(13.0, -18.0, 24.0, 0.0), "shrunk by 3");
    let center = r.center();
    for d in [-1.0e30, -max, -inf, nan] {
        assert_eq!(
            r.grown(d),
            Rect::new(center, Vec2::ZERO),
            "grown by {d}: empty at its center"
        );
    }
    let edge = rect(-max, -max, 1.0e38, 1.0e38);
    let high = rect(max - 1.0e38, max - 1.0e38, 1.0e38, 1.0e38);
    // A rectangle reaching +inf, as grown past f32 gives one: its origin
    // finite and its center past f32.
    let reaching = r.grown(1.8e38);
    let unbounded = rect(0.0, 0.0, inf, 10.0);
    let overflowing = rect(3.0e38, 0.0, 1.0e38, 10.0);
    let rows = [
        ("near the origin", r, 0.0, false),
        ("near the origin", r, 5.0, false),
        ("near the origin", r, 1.0e37, false),
        ("near the origin", r, 1.6e38, false),
        ("near the origin", r, 1.8e38, true),
        ("near the origin", r, max, true),
        ("near the origin", r, inf, true),
        ("near the origin", r, -3.0, false),
        ("near the origin", r, -max, false),
        ("at the low edge of f32", edge, 1.0e38, true),
        ("at the high edge of f32", high, 1.0e38, false),
        ("reaching +inf", reaching, 0.0, true),
        ("reaching +inf", reaching, 5.0, true),
        ("reaching +inf", reaching, -1.0e38, true),
        ("reaching +inf", reaching, -1.8e38, true),
        ("with a size of +inf", unbounded, 5.0, true),
        ("whose center is past f32", overflowing, 0.0, true),
        ("whose center is past f32", overflowing, 5.0, true),
        ("whose center is past f32", overflowing, 1.8e38, true),
    ];
    for (what, r, d, past) in rows {
        let plain = (r.size + Vec2::splat(2.0 * d)).max(Vec2::ZERO);
        assert_eq!(
            !(plain.is_finite() && (r.center() - plain * 0.5).is_finite()),
            past,
            "precondition: a box {what} grown by {d} is past f32 on an axis: {past}"
        );
        let g = r.grown(d);
        for (axis, k) in [("x", 0), ("y", 1)] {
            let (origin, size, far) = (g.origin[k], g.size[k], g.max()[k]);
            let (near_at, far_at) = reach(r.origin[k], r.size[k], d);
            let at = format!("a box {what} grown by {d}, on {axis}: {g:?}");
            assert!(origin.is_finite(), "{at}: its origin is a number on the canvas");
            assert!(size >= 0.0, "{at}: its size is a number, not negative");
            assert!(!far.is_nan(), "{at}: its far corner is a number");
            let close = |got: f64, want: f64| (got - want).abs() <= want.abs() * 1.0e-6 + 1.0e-6;
            assert!(
                close(f64::from(origin), near_at),
                "{at}: its origin is the near edge it reaches, {near_at}"
            );
            if far_at > f64::from(max) || far_at - near_at > f64::from(max) {
                assert_eq!(far, inf, "{at}: its far corner is +inf, past f32");
            } else {
                assert!(
                    close(f64::from(far), far_at),
                    "{at}: its far corner is the far edge it reaches, {far_at}"
                );
            }
        }
    }
    // A box whose origin is not finite on an axis, which the flow is
    // handed: its size is never negative, whatever `d`.
    let unfinite = [
        ("an origin of -inf", rect(-inf, 0.0, 10.0, 10.0)),
        ("an origin of +inf", rect(inf, 0.0, 10.0, 10.0)),
        ("an origin that is no number", rect(nan, 0.0, 10.0, 10.0)),
    ];
    for (what, r) in unfinite {
        for d in [5.0, -5.0, 1.8e38, max, inf, -inf, nan] {
            let g = r.grown(d);
            assert!(
                !(g.size.x < 0.0 || g.size.y < 0.0),
                "a box with {what} grown by {d}: its size is not negative, {g:?}"
            );
        }
    }
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
fn test_flow_table_spacing_has_a_floor() {
    do_flow_table_spacing_has_a_floor();
}

/// A tiny `Fit.min` cannot sample a table finer than any glyph can be
/// placed: the spacing is held at a quarter of the line height of the
/// shaper's smallest size, 0.25 · 0.5 pt · 1.2 = 0.15 pt. Counted, not
/// timed: a cubic of three 50 pt legs (speed bound 150) is tabled in
/// 1000 steps, 1001 samples, where `min = 1e-6` on a 14.4 pt line would
/// ask for 0.25 · 14.4 · 1e-6 ≈ 3.6e-6 pt and hit the 2²⁰ ceiling; an
/// ellipse of radii 100 and 50 in ⌈100 · 2π / 0.15⌉ = 4189 steps.
pub fn do_flow_table_spacing_has_a_floor() {
    use crate::document::values::FitMode;
    use crate::flow::place::{table_spacing, MIN_TABLE_SPACING};
    use crate::flow::tests::support::{style, synthetic};
    const ONCE_PLAIN: FitMode = FitMode::Once { filler: None };

    close(MIN_TABLE_SPACING, 0.15, 1e-6, "the floor");
    let run = synthetic(&[5.0], 14.4);
    let tiny = FitMode::Fit { min: 1e-6, max: 2.0 };
    let spacing = table_spacing(&run, &tiny, &style(12.0));
    close(spacing, 0.15, 1e-6, "a tiny min is held at the floor");
    let usual = FitMode::Fit { min: 0.5, max: 2.0 };
    close(
        table_spacing(&run, &usual, &style(12.0)),
        1.8,
        1e-5,
        "0.25 · 14.4 · 0.5",
    );
    let hairline = synthetic(&[1.0], 0.4);
    close(
        table_spacing(&hairline, &ONCE_PLAIN, &style(12.0)),
        0.15,
        1e-6,
        "a line under the floor",
    );

    let cubic = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(30.0, 40.0),
        Vec2::new(60.0, 0.0),
        Vec2::new(90.0, 40.0),
    );
    close(cubic.speed_bound(), 150.0, 1e-3, "three legs of 50");
    let samples = ArcPath::open(&cubic, spacing).table_samples();
    assert!((1000..=1002).contains(&samples), "cubic: {samples} samples");

    let ellipse = outline(rect(0.0, 0.0, 200.0, 100.0), Shape::Ellipse);
    let path = ArcPath::closed(&ellipse, start(Edge::Top, 0.0), Direction::Clockwise, spacing);
    let samples = path.table_samples();
    assert!((4189..=4191).contains(&samples), "ellipse: {samples} samples");
}

/// Outlines from the small to the largest `f32`, round and not, centered
/// at the origin and far from it, with radii and sizes that are not
/// numbers, two of the sizes either side of the longest path placement
/// lays (a square 2 × 10¹⁸ a side is 8 × 10¹⁸ round, one 3 × 10¹⁸ a side
/// 1.2 × 10¹⁹): what
/// [`do_flow_a_closed_paths_table_samples_are_known_before_it_is_built`]
/// and [`do_flow_a_closed_path_is_within_the_limit_or_too_long`] walk.
fn closed_outlines() -> Vec<Outline> {
    let sizes = [0.5, 200.0, 1e5, 2e18, 3e18, 1e30, 1.5e38, f32::MAX, f32::NAN];
    let mut outlines = Vec::new();
    for (i, &w) in sizes.iter().enumerate() {
        for (j, &h) in sizes.iter().enumerate() {
            for (k, &at) in [0.0, 1e30, 3e38].iter().enumerate() {
                let r = rect(at - w * 0.5, at - h * 0.5, w, h);
                outlines.push(outline(r, Shape::Ellipse));
                outlines.push(outline(r, rounded([0, 30, 100][(i + j + k) % 3])));
            }
        }
    }
    outlines
}

#[test]
fn test_flow_a_closed_paths_table_samples_are_known_before_it_is_built() {
    do_flow_a_closed_paths_table_samples_are_known_before_it_is_built();
}

/// `ArcPath::closed_table_samples` answers, without building it, the
/// samples `ArcPath::closed`'s path is built with
/// (`ArcPath::table_samples`) — what a frame's setup charges its tables
/// at (#309, ruling B16.3a) and its bound is computed from — to the
/// sample, over `closed_outlines` at the finest spacing, a font's and
/// a spacing past every perimeter, from every edge both ways. Among them
/// are tables at their cap, tables between it and their floor, circles
/// and rounded rectangles of none, and ellipses that are not numbers.
///
/// Control: `closed_table_samples` without the `+ 1` for a table's
/// closing sample, red on every tabled outline.
pub fn do_flow_a_closed_paths_table_samples_are_known_before_it_is_built() {
    use crate::flow::place::MIN_TABLE_SPACING;
    use crate::flow::MAX_TABLE_SAMPLES;
    let (mut capped, mut between, mut none) = (0, 0, 0);
    let edges = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];
    for (n, path) in closed_outlines().iter().enumerate() {
        for (m, spacing) in [MIN_TABLE_SPACING, 4.8, 1e30].into_iter().enumerate() {
            let known = ArcPath::closed_table_samples(path, spacing);
            let direction = if (n + m) % 2 == 0 {
                Direction::Clockwise
            } else {
                Direction::CounterClockwise
            };
            let built =
                ArcPath::closed(path, start(edges[(n + m) % 4], 0.3), direction, spacing).table_samples();
            assert_eq!(known, built, "{path:?} at {spacing}");
            match known {
                0 => none += 1,
                k if k == MAX_TABLE_SAMPLES + 1 => capped += 1,
                _ => between += 1,
            }
        }
    }
    assert!(
        capped > 0 && between > 0 && none > 0,
        "capped {capped}, between {between}, none {none}"
    );
}

#[test]
fn test_flow_a_closed_path_is_within_the_limit_or_too_long() {
    do_flow_a_closed_path_is_within_the_limit_or_too_long();
}

/// A closed path is no longer than `MAX_PATH_LENGTH`, or it is too long
/// and has no length (#363, ruling B42.1): over `closed_outlines`, from
/// every edge both ways, the path `ArcPath::closed` builds has a finite
/// length within the limit, and says it is too long exactly where its
/// outline is. Which outline is, is read off the outline's own numbers in
/// `f64`, none of it the path's: a rounded rectangle's perimeter, its
/// arcs and its sides between them — each as far as its corners lie apart
/// as their floats hold them, to sixteen of their spacing, which far from
/// the origin is all a side is; an ellipse's, which is more than four of
/// its longer radius and less than a turn of it. Outlines whose corners
/// are not finite, and those between their two bounds, are held to the
/// first claim alone.
///
/// Ellipses so far out that their radii are below their floats' spacing
/// there, whose points all round onto their center, have their perimeter,
/// measured from their center (#356): until their tables were, the
/// rounding was all the table summed, and they had none.
///
/// Controls: the limit not held (`from_stretches` keeping every path),
/// red on a length that is not finite; the limit held at half and at
/// twice what it is, each red on an outline between the two (a box 6 ×
/// 10¹⁸ round, and a rounded one 9.5 × 10¹⁸ round); an
/// ellipse's table measured between its points as `f32`, red on the
/// collapsed ones.
pub fn do_flow_a_closed_path_is_within_the_limit_or_too_long() {
    use crate::flow::MAX_PATH_LENGTH;
    let limit = f64::from(MAX_PATH_LENGTH);
    let (mut too_long, mut laid, mut collapsed) = (0, 0, 0);
    let edges = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];
    for (n, path) in closed_outlines().iter().enumerate() {
        let direction = if n % 2 == 0 {
            Direction::Clockwise
        } else {
            Direction::CounterClockwise
        };
        let from = start(edges[n % 4], 0.7);
        let built = ArcPath::closed(path, from, direction, 1e30);
        assert!(
            built.length().is_finite() && built.length() <= MAX_PATH_LENGTH,
            "{path:?}: a length of {}",
            built.length()
        );
        assert!(
            !built.too_long() || built.length() == 0.0,
            "{path:?}: too long, and {} long",
            built.length()
        );
        too_long += usize::from(built.too_long());
        laid += usize::from(built.length() > 0.0);
        // How long the outline is at the least and at the most, from its
        // own numbers: `None` where its corners are not finite.
        let rect = path.rect();
        let (w, h) = (f64::from(rect.size.x), f64::from(rect.size.y));
        let cornered = rect.origin.is_finite() && rect.max().is_finite() && rect.center().is_finite();
        let bounds = match path {
            _ if !cornered || !(w >= 0.0 && h >= 0.0) => None,
            Outline::RoundedRect { radius, .. } => {
                let r = if radius.is_nan() {
                    0.0
                } else {
                    f64::from(*radius).clamp(0.0, 0.5 * w.min(h))
                };
                let across = rect.max().as_dvec2() - rect.origin.as_dvec2();
                let sides = (across.x - 2.0 * r).abs() + (across.y - 2.0 * r).abs();
                let perimeter = 2.0 * sides + std::f64::consts::TAU * r;
                // The spacing of the floats its corners are set in.
                let reach = rect.origin.abs().max(rect.max().abs()).max_element();
                let grain = 16.0 * f64::from(reach.next_up() - reach);
                Some((perimeter - grain, perimeter + grain))
            }
            Outline::Ellipse { .. } => {
                let longer = 0.5 * w.max(h);
                Some((4.0 * longer, std::f64::consts::TAU * longer))
            }
        };
        if let Some((least, most)) = bounds {
            if least > limit * (1.0 + 1e-6) {
                assert!(
                    built.too_long(),
                    "{path:?}: at least {least:e} round, and not too long"
                );
            }
            if most < limit * (1.0 - 1e-6) {
                assert!(
                    !built.too_long(),
                    "{path:?}: at most {most:e} round, and too long"
                );
            }
        }
        if let Outline::Ellipse { rect } = path {
            // Its points all one float: its radii below the spacing of the
            // floats about its center.
            let center = rect.center();
            let radii = rect.size * 0.5;
            let spacing = |v: f32| f32::from_bits(v.abs().to_bits() + 1) - v.abs();
            if built.table_samples() > 0
                && !built.too_long()
                && radii.x < spacing(center.x)
                && radii.y < spacing(center.y)
            {
                assert!(built.length() > 0.0, "{path:?}: its perimeter, not none");
                collapsed += 1;
            }
        }
    }
    assert!(
        too_long > 0 && laid > 0 && collapsed > 0,
        "too long {too_long}, laid {laid}, collapsed {collapsed}"
    );
}

/// The lengths of the polygon through the ellipse of `radii` about the
/// origin at the angles `from + (to − from)·(i / count)`, each an `f32`
/// as a table's are, from its first point to each, for `i` from 0 to
/// `count`: the oracle an ellipse's table is held to (#356). It shares no
/// arithmetic with the engine's table: a point is found from the tangent
/// of its half angle, `u = tan(φ/2)`, as `(1 − u², 2u) / (1 + u²)` scaled
/// by the radii — or from its cotangent where the half angle nears a
/// quarter turn — in `f64`, where the table takes the cosine and sine.
fn ellipse_polygon(radii: glam::DVec2, from: f32, to: f32, count: usize) -> Vec<f64> {
    let point = |phi: f64| {
        let half = 0.5 * phi;
        let (c, s) = if half.cos().abs() >= half.sin().abs() {
            let u = half.tan();
            ((1.0 - u * u) / (1.0 + u * u), 2.0 * u / (1.0 + u * u))
        } else {
            let v = 1.0 / half.tan();
            ((v * v - 1.0) / (v * v + 1.0), 2.0 * v / (v * v + 1.0))
        };
        glam::DVec2::new(radii.x * c, radii.y * s)
    };
    let phi = |i: usize| f64::from(from + (to - from) * (i as f32 / count as f32));
    let mut along = Vec::with_capacity(count + 1);
    let mut walked = 0.0_f64;
    along.push(walked);
    for i in 0..count {
        walked += (point(phi(i + 1)) - point(phi(i))).length();
        along.push(walked);
    }
    along
}

/// The angle, in radians as an `f32`, at which an ellipse's outline starts
/// from `start`, by the rule `ArcPath::closed` states: its side's quarter
/// (top 225°–315°, right 315°–405°, bottom 45°–135°, left 135°–225°),
/// `start.at` of the way along it.
fn ellipse_start(start: StartPoint) -> f32 {
    let (from, to) = match start.edge {
        Edge::Top => (225.0_f32, 315.0_f32),
        Edge::Right => (315.0, 405.0),
        Edge::Bottom => (45.0, 135.0),
        Edge::Left => (135.0, 225.0),
    };
    (from + (to - from) * start.at).to_radians()
}

#[test]
fn test_flow_an_ellipses_length_is_its_ellipses_at_every_magnitude() {
    do_flow_an_ellipses_length_is_its_ellipses_at_every_magnitude();
}

/// An ellipse's length is the length of the ellipse its center and radii
/// make, at every distance from the origin and at the origin, to the bound
/// `ArcPath::closed` states (#356), and so is each distance along it its
/// table holds. The centers: the origin, 10⁶ to 10²² from it on the
/// diagonal, and five of mixed signs from 10⁶ to 10²². The ellipses:
///
/// - at every center, 200 and 1,600 across, a:b = 0.6, a flat 0.05 and a
///   tall 3, each way round, from the top's start, from a right side 0.37
///   of the way along and from a left side 0.81 of it; and 25,600 across,
///   the flat one counter-clockwise from the left and the tall one
///   clockwise from the right;
/// - at the centers on the diagonal, a:b = 0.6 at every doubling from 400
///   to 409,600 across but 1,600, the direction and the start turning
///   with the size.
///
/// Each is held to `ellipse_polygon` through it at the angles its start
/// and direction give by the rule `ArcPath::closed` states
/// (`ellipse_start`) — its length and its distances a quarter, a half and
/// three quarters of its samples in, each within `2⁻²⁴` of that polygon's
/// length to there, and `2⁻⁴⁷` of its larger radius and of that length a
/// sample. Far out, where an `f32` sets its points apart by more than its
/// radii, it still has its perimeter.
///
/// Controls, each red: its table's distances summed between its points as
/// `f32`, where they lie; its distances summed in `f32`; the cosine and
/// sine taken in `f32` and widened; a counter-clockwise outline's angle
/// running up, as a clockwise one's does; the start edge's quarter one
/// side on.
pub fn do_flow_an_ellipses_length_is_its_ellipses_at_every_magnitude() {
    let diagonal: Vec<Vec2> = std::iter::once(Vec2::ZERO)
        .chain((6..=22).map(|k| Vec2::splat(10f32.powi(k))))
        .collect();
    let mixed: Vec<Vec2> = (6..=22)
        .step_by(4)
        .map(|k| Vec2::new(-10f32.powi(k), 0.3 * 10f32.powi(k)))
        .collect();
    let starts = [
        start(Edge::Top, 0.0),
        start(Edge::Right, 0.37),
        start(Edge::Left, 0.81),
    ];
    let directions = [Direction::Clockwise, Direction::CounterClockwise];
    // (across, a:b, direction, start)
    let mut everywhere = Vec::new();
    for across in [200.0_f32, 1_600.0] {
        for aspect in [0.6_f32, 0.05, 3.0] {
            for direction in directions {
                for from_start in starts {
                    everywhere.push((across, aspect, direction, from_start));
                }
            }
        }
    }
    everywhere.push((25_600.0, 0.05, directions[1], starts[2]));
    everywhere.push((25_600.0, 3.0, directions[0], starts[1]));
    let doublings: Vec<_> = [1_usize, 2, 4, 5, 6, 7, 8, 9, 10, 11]
        .into_iter()
        .map(|d| {
            (
                200.0 * (1u32 << d) as f32,
                0.6_f32,
                directions[d % 2],
                starts[d % 3],
            )
        })
        .collect();
    let mut checked = 0;
    for (k, &center) in diagonal.iter().chain(&mixed).enumerate() {
        let on_diagonal = k < diagonal.len();
        for &(across, aspect, direction, from_start) in
            everywhere.iter().chain(doublings.iter().filter(|_| on_diagonal))
        {
            let size = Vec2::new(across, aspect * across);
            let path = Outline::Ellipse {
                rect: Rect::new(center - size * 0.5, size),
            };
            let Outline::Ellipse { rect } = path else { unreachable!() };
            let radii = rect.size * 0.5;
            let from = ellipse_start(from_start);
            let turn = match direction {
                Direction::Clockwise => 1.0,
                Direction::CounterClockwise => -1.0,
            };
            let to = from + turn * std::f32::consts::TAU;
            let built = ArcPath::closed(&path, from_start, direction, 1.0);
            let samples = built.table_samples();
            assert!(samples > 1, "{path:?} is tabled");
            let (_, distances) = built.table(0).expect("an ellipse's one table");
            let exact = ellipse_polygon(radii.as_dvec2(), from, to, samples - 1);
            let reach = f64::from(radii.max_element());
            let within = |i: usize, measured: f64| {
                let bound = exact[i] * 2f64.powi(-24) + (i + 1) as f64 * 2f64.powi(-47) * (reach + exact[i]);
                assert!(
                    (measured - exact[i]).abs() <= bound,
                    "{path:?} from {from_start:?} {direction:?}: {measured} at sample {i}, \
                     its ellipse {}: past {bound}",
                    exact[i]
                );
            };
            within(samples - 1, f64::from(built.length()));
            let count = samples - 1;
            for i in [count / 4, count / 2, 3 * count / 4] {
                within(i, f64::from(distances[i]));
            }
            assert!(built.length() > 0.0, "{path:?} has its perimeter");
            checked += 1;
        }
    }
    assert_eq!(checked, 23 * 38 + 18 * 10, "every ellipse is checked");
}

#[test]
fn test_flow_an_ellipse_whose_points_pass_the_largest_f32_is_too_long() {
    do_flow_an_ellipse_whose_points_pass_the_largest_f32_is_too_long();
}

/// An ellipse whose center and radii together pass the largest `f32`,
/// though each is finite and its table is built, would set points past
/// it. It is too long (#363): a point passes the largest `f32` only where
/// a radius is half the floats' spacing there or more, and its table,
/// measured from its center and all finite (#356), is then far past the
/// limit. So its path has no stretch to set a point along, and answers
/// its origin, the middle of the canvas for one that is not finite.
///
/// Control: the limit not held, red (a perimeter of some 2.3 × 10³⁸ along
/// points not all finite).
pub fn do_flow_an_ellipse_whose_points_pass_the_largest_f32_is_too_long() {
    let center = Vec2::splat(3.3e38);
    let radii = Vec2::new(5e37, 2e37);
    let path = Outline::Ellipse {
        rect: Rect::new(center - radii, radii * 2.0),
    };
    let built = ArcPath::closed(&path, start(Edge::Top, 0.5), Direction::Clockwise, 1e30);
    assert!(built.table_samples() > 0, "precondition: its table is built");
    assert!(
        (center + radii).x.is_infinite(),
        "precondition: its points pass the largest f32"
    );
    assert!(built.too_long(), "{path:?}: too long");
    assert_eq!(built.length(), 0.0, "{path:?}: no length");
    for s in [0.0, 1.0e30, f32::MAX] {
        assert!(built.at(s).0.is_finite(), "{path:?}: a finite point at {s}");
    }
}

#[test]
fn test_flow_a_curve_faster_than_the_largest_f32_is_too_long() {
    do_flow_a_curve_faster_than_the_largest_f32_is_too_long();
}

/// A curve whose every control point is finite but whose speed bound —
/// the degree times its longest leg — is past the largest `f32`, or which
/// has a leg longer than it, is tabled by no count: it builds no table,
/// and its path says it is too long (#363), where it was a path of no
/// length with nothing said. Seed 1021 of the structured flow generator:
/// a quadratic from (1, 1) by (−2.4 × 10³⁸, −587) to (−10³⁰, 1), whose
/// bound of twice its first leg is infinite and whose `Spread` was cut to
/// nothing though its text fit. And a cubic between control points
/// 6 × 10³⁸ apart, whose leg has no `f32` length.
///
/// Control: `table` answering no stretch for a curve it cannot count
/// samples for, red: a path of no length that is not too long.
pub fn do_flow_a_curve_faster_than_the_largest_f32_is_too_long() {
    let fast = Curve::Quadratic(
        Vec2::new(1.0, 1.0),
        Vec2::new(-2.409_018_9e38, -587.0),
        Vec2::new(-1e30, 1.0),
    );
    assert_eq!(
        fast.speed_bound(),
        f32::INFINITY,
        "its bound is past the largest f32"
    );
    let long = Curve::Cubic(
        Vec2::new(-3.0e38, 0.0),
        Vec2::new(3.0e38, 10.0),
        Vec2::new(3.0e38, 20.0),
        Vec2::new(-3.0e38, 30.0),
    );
    assert!(long.speed_bound().is_nan(), "a leg of it has no length");
    for (name, curve) in [("the fast quadratic", fast), ("the long cubic", long)] {
        let path = ArcPath::open(&curve, 4.8);
        assert_eq!(path.table_samples(), 0, "{name} builds no table");
        assert!(path.too_long(), "{name} is too long");
        assert_eq!(path.length(), 0.0, "{name} has no length");
    }
}

#[test]
fn test_flow_non_finite_curves_build_no_table() {
    do_flow_non_finite_curves_build_no_table();
}

/// A curve with a control point that is not finite has no speed bound —
/// `speed_bound` is `NaN`, not the bound of its finite legs — and its
/// path builds no table at all (not one built and then dropped) and has
/// no length; so does an ellipse with a radius that is not a number.
pub fn do_flow_non_finite_curves_build_no_table() {
    let nan_cubic = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(f32::NAN, 10.0),
        Vec2::new(20.0, 10.0),
        Vec2::new(30.0, 0.0),
    );
    let inf_quadratic = Curve::Quadratic(Vec2::ZERO, Vec2::new(10.0, f32::INFINITY), Vec2::new(20.0, 0.0));
    for (name, curve) in [("NaN cubic", nan_cubic), ("infinite quadratic", inf_quadratic)] {
        assert!(curve.speed_bound().is_nan(), "{name}: {}", curve.speed_bound());
        let path = ArcPath::open(&curve, 1.0);
        assert_eq!(path.table_samples(), 0, "{name}");
        assert_eq!(path.length(), 0.0, "{name}");
    }
    let ellipse = outline(rect(0.0, 0.0, f32::NAN, 100.0), Shape::Ellipse);
    let path = ArcPath::closed(&ellipse, start(Edge::Top, 0.0), Direction::Clockwise, 1.0);
    assert_eq!(path.table_samples(), 0, "NaN ellipse");
    assert_eq!(path.length(), 0.0, "NaN ellipse");
}

#[test]
fn test_flow_outline_support_faces_its_direction() {
    do_flow_outline_support_faces_its_direction();
}

/// `Outline::support` (#291, ruling R33), against values worked out
/// here: on a 400 × 200 ellipse the point `c + (a², b²)/√(a² + b²)` for
/// `(1, 1)` — where its tangent runs at 45° — and in every direction a
/// point on the ellipse whose normal, `(x/a², y/b²)`, is parallel to the
/// direction; on a rounded rectangle its corner arc's midpoint; on a
/// sharp one its corner; on a pill the end of its cap; and its center
/// for a direction of no length or one not a number.
pub fn do_flow_outline_support_faces_its_direction() {
    let near = |a: Vec2, b: Vec2| a.distance(b) <= 1e-3;
    let (a, b) = (200.0_f32, 100.0_f32);
    let ellipse = outline(rect(0.0, 0.0, 2.0 * a, 2.0 * b), Shape::Ellipse);
    let c = Vec2::new(a, b);
    let expected = c + Vec2::new(a * a, b * b) / (a * a + b * b).sqrt();
    assert!(
        near(ellipse.support(Vec2::ONE), expected),
        "{}",
        ellipse.support(Vec2::ONE)
    );
    for dir in [
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(3.0, 1.0),
        Vec2::new(-1.0, 4.0),
    ] {
        let p = ellipse.support(dir) - c;
        assert!(
            ((p.x / a).powi(2) + (p.y / b).powi(2) - 1.0).abs() < 1e-4,
            "{dir}: {p} on the ellipse"
        );
        let normal = Vec2::new(p.x / (a * a), p.y / (b * b)).normalize();
        assert!(
            normal.dot(dir.normalize()) > 1.0 - 1e-5,
            "{dir}: the normal {normal} faces it"
        );
    }
    // A rounded rectangle 400 × 200 at 50%: corner radius 50.
    let round = outline(rect(0.0, 0.0, 400.0, 200.0), rounded(50));
    let corner = Vec2::new(350.0, 150.0) + Vec2::splat(50.0 * FRAC_1_SQRT_2);
    assert!(
        near(round.support(Vec2::ONE), corner),
        "{}",
        round.support(Vec2::ONE)
    );
    let top_left = Vec2::new(50.0, 50.0) - Vec2::splat(50.0 * FRAC_1_SQRT_2);
    assert!(near(round.support(-Vec2::ONE), top_left));
    let sharp = outline(rect(10.0, 20.0, 400.0, 200.0), rounded(0));
    assert!(near(sharp.support(-Vec2::ONE), Vec2::new(10.0, 20.0)));
    assert!(near(sharp.support(Vec2::ONE), Vec2::new(410.0, 220.0)));
    let pill = outline(rect(0.0, 0.0, 400.0, 200.0), rounded(100));
    assert!(
        near(pill.support(Vec2::X), Vec2::new(400.0, 100.0)),
        "{}",
        pill.support(Vec2::X)
    );
    assert!(near(
        pill.support(Vec2::ONE),
        Vec2::new(300.0, 100.0) + Vec2::splat(100.0 * FRAC_1_SQRT_2)
    ));
    for nothing in [Vec2::ZERO, Vec2::new(f32::NAN, 1.0)] {
        assert_eq!(ellipse.support(nothing), c, "{nothing}");
        assert_eq!(round.support(nothing), Vec2::new(200.0, 100.0), "{nothing}");
    }
}

#[test]
fn test_flow_path_nearest_finds_its_own_points() {
    do_flow_path_nearest_finds_its_own_points();
}

/// `ArcPath::nearest` on a closed ellipse finds, for a point a little
/// off the path at `s`, that `s` — round the path, a point just before
/// the seam included, which sits as near the sample at the seam's start
/// as at its end — to within a hundredth of a point, measured round the
/// path.
pub fn do_flow_path_nearest_finds_its_own_points() {
    let shape = outline(rect(0.0, 0.0, 400.0, 200.0), Shape::Ellipse);
    let path = ArcPath::closed(&shape, start(Edge::Left, 0.5), Direction::CounterClockwise, 0.25);
    let length = path.length();
    for s in [
        0.0,
        1.0,
        0.25 * length,
        0.5 * length,
        0.9 * length,
        length - 0.1,
        length - 0.01,
    ] {
        let (p, tangent) = path.at(s);
        let off = p + path.outward(tangent) * 3.0;
        let found = path.nearest(off);
        let apart = (found - s).rem_euclid(length);
        let apart = apart.min(length - apart);
        assert!(apart <= 1e-2, "at {s} of {length}: found {found}");
    }
}

#[test]
fn test_flow_path_measures_a_path_as_long_as_placement_lays() {
    do_flow_path_measures_a_path_as_long_as_placement_lays();
}

/// A line, a curve and an outline nearly as long as placement lays
/// ([`MAX_PATH_LENGTH`](crate::flow::MAX_PATH_LENGTH), 2⁶³) measure their
/// lengths, each against the same length worked in `f64`, and walk to
/// their ends; the same four times the size, past it, have no length and
/// are too long. And a vector longer than `√f32::MAX` (about `1.8e19`) —
/// whose squared length passes every `f32`, and which no path placement
/// lays holds — still measures its length and its direction
/// (`length_of`, `direction_of`): what a stretch is measured by before
/// its path is found too long.
///
/// Controls: with `length_of` answering `Vec2::length`, red ("a vector":
/// infinite); with the limit held at half what it is, red ("a line is
/// within the limit, and not too long").
pub fn do_flow_path_measures_a_path_as_long_as_placement_lays() {
    let near = |got: f32, want: f64, what: &str| {
        assert!(
            ((got as f64) - want).abs() <= want * 1e-5,
            "{what}: {got:e}, owed {want:e}"
        );
    };
    near(length_of(Vec2::new(3.0e30, -4.0e30)), 5.0e30, "a vector");
    near(length_of(Vec2::new(3.0, 4.0)), 5.0, "a short vector");
    let dir = direction_of(Vec2::new(3.0e30, -4.0e30)).expect("a direction");
    assert!(dir.distance(Vec2::new(0.6, -0.8)) <= 1e-6, "its direction {dir}");
    assert_eq!(length_of(Vec2::ZERO), 0.0);
    for v in [
        Vec2::ZERO,
        Vec2::new(f32::NAN, 1.0),
        Vec2::new(f32::INFINITY, 0.0),
    ] {
        assert_eq!(direction_of(v), None, "{v} has no direction");
    }
    // Below the overflow, `try_normalize`'s answer exactly: `None` for a
    // vector whose length underflows to zero, as for zero itself.
    for v in [
        Vec2::new(1.0e-38, 0.0),
        Vec2::new(3.0e-25, -4.0e-25),
        Vec2::new(0.3, -0.4),
        Vec2::new(3.0e18, 4.0e18),
    ] {
        assert_eq!(
            direction_of(v),
            v.try_normalize(),
            "{v}: try_normalize's direction"
        );
    }
    assert_eq!(
        direction_of(Vec2::new(1.0e-38, 0.0)),
        None,
        "an underflowing vector has no direction"
    );
    let limit = f64::from(crate::flow::MAX_PATH_LENGTH);
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    for (scale, laid) in [(1.0_f32, true), (4.0, false)] {
        let held = |path: &ArcPath, want: f64, what: &str| {
            assert_eq!(
                want <= limit,
                laid,
                "precondition: {what} at {scale}×, {want:e} long, is {} the limit",
                if laid { "within" } else { "past" }
            );
            if laid {
                assert!(!path.too_long(), "{what} is within the limit, and not too long");
                near(path.length(), want, what);
            } else {
                assert!(path.too_long(), "{what} at {scale}× is too long");
                assert_eq!(path.length(), 0.0, "{what} at {scale}× has no length");
            }
        };
        // A line from -4e18 to 4e18, and a diagonal one.
        let (a, b) = (Vec2::new(-4.0e18, 0.0) * scale, Vec2::new(4.0e18, 0.0) * scale);
        let line = ArcPath::open(&Curve::Line(a, b), 1.0);
        held(&line, 8.0e18 * f64::from(scale), "a line");
        if laid {
            assert!(
                line.at(line.length()).0.distance(b) <= 1.0e13,
                "the line walks to its end"
            );
        }
        let (a, b) = (Vec2::splat(-3.0e18) * scale, Vec2::splat(3.0e18) * scale);
        held(
            &ArcPath::open(&Curve::Line(a, b), 1.0),
            6.0e18 * std::f64::consts::SQRT_2 * f64::from(scale),
            "a diagonal line",
        );
        // A curve with a control point far out: its length lies between
        // its chord and its control polygon, and it counts toward the
        // limit by its polygon.
        let (a, c, b) = (
            Vec2::ZERO,
            Vec2::new(4.0e18, 0.0) * scale,
            Vec2::new(0.0, 8.0e17) * scale,
        );
        let curve = ArcPath::open(&Curve::Quadratic(a, c, b), 1.0);
        let polygon = (c - a).as_dvec2().length() + (b - c).as_dvec2().length();
        let chord = (b - a).as_dvec2().length();
        assert_eq!(
            polygon <= limit,
            laid,
            "precondition: the curve's control polygon at {scale}×, {polygon:e}"
        );
        assert_eq!(curve.too_long(), !laid, "a curve at {scale}×");
        if laid {
            assert!(
                (curve.length() as f64) >= chord && (curve.length() as f64) <= polygon * (1.0 + 1e-5),
                "a curve {:e}, between {chord:e} and {polygon:e}",
                curve.length()
            );
        }
        // A rounded rectangle 4e18 wide.
        let (w, h, r) = (4.0e18_f64 * f64::from(scale), 40.0_f64, 10.0_f64);
        let rounded = outline(
            Rect::new(Vec2::ZERO, Vec2::new(w as f32, h as f32)),
            Shape::Rectangle {
                radius: Percent::new(50),
            },
        );
        let path = ArcPath::closed(&rounded, start, Direction::Clockwise, 1.0);
        held(
            &path,
            2.0 * (w - 2.0 * r) + 2.0 * (h - 2.0 * r) + 2.0 * PI as f64 * r,
            "an outline",
        );
    }
}

/// A path looked up forward a little at a time — as a walk looks it up —
/// is answered from where the last lookup fell, not by a search of its
/// stretches and its table (#294): fewer than one search in a hundred
/// lookups. Every answer is the one a path that has not been walked
/// gives, bit for bit, a search's. Control: with no cursor, every lookup
/// searches. (No `do_*` twin: it reads test counters.)
#[test]
fn test_flow_path_walks_forward_without_searching() {
    use crate::flow::path::{at_or_below_seam, PATH_LOOKUPS, PATH_SEARCHES};
    // The cursor's search against the search it stands for (ruling
    // R44.7): `partition_point` on the raw table, for tables with runs of
    // equal values, from a cursor anywhere, for lookups that walk
    // forward, jump back — as a chord either side of an ink does — and
    // land at random, so both ways the search gallops are taken.
    let mut seed = 0x9e37_79b9_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let (mut backward, mut forward) = (0, 0);
    for len in [1usize, 2, 3, 17, 300, 4096] {
        let mut table = Vec::with_capacity(len);
        let mut v = 0.0_f32;
        for _ in 0..len {
            // A step of none now and then: equal distances.
            v += (next() % 4) as f32 * 0.5;
            table.push(v);
        }
        let top = v + 2.0;
        let mut cursor = (next() as usize) % len;
        for k in 0..2_000 {
            let s = match k % 3 {
                0 => table[cursor.min(len - 1)] + 0.25,
                1 => table[cursor.min(len - 1)] - (next() % 64) as f32,
                _ => (next() % 10_000) as f32 / 10_000.0 * (top + 2.0) - 1.0,
            };
            let want = table.partition_point(|&d| d <= s).saturating_sub(1);
            let (got, moved) = at_or_below_seam(&table, s, cursor);
            assert_eq!(
                got, want,
                "a table of {len} from cursor {cursor} at {s}: {got}, a search finds {want}"
            );
            assert_eq!(moved, got, "the cursor is left where the lookup fell");
            if got < cursor {
                backward += 1;
            } else if got > cursor + 1 {
                forward += 1;
            }
            cursor = got;
        }
    }
    assert!(
        backward > 100 && forward > 100,
        "precondition: lookups gallop both ways ({backward} back, {forward} on)"
    );
    let curve = Curve::Cubic(
        Vec2::new(0.0, 0.0),
        Vec2::new(300.0, -400.0),
        Vec2::new(500.0, 500.0),
        Vec2::new(900.0, 0.0),
    );
    let rounded = outline(
        Rect::new(Vec2::ZERO, Vec2::new(400.0, 220.0)),
        Shape::Rectangle {
            radius: Percent::new(40),
        },
    );
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    let open = || ArcPath::open(&curve, 1.0);
    let closed = || ArcPath::closed(&rounded, start, Direction::Clockwise, 1.0);
    let paths: [(&str, &dyn Fn() -> ArcPath); 2] = [("a cubic", &open), ("a rounded rectangle", &closed)];
    for (what, fresh) in paths {
        let path = fresh();
        let steps = 4000;
        let at = |k: usize| path.length() * k as f32 / steps as f32;
        for counter in [&PATH_LOOKUPS, &PATH_SEARCHES] {
            counter.with(|count| count.set(0));
        }
        let walked: Vec<(Vec2, Vec2)> = (0..=steps).map(|k| path.at(at(k))).collect();
        let [lookups, searches] =
            [&PATH_LOOKUPS, &PATH_SEARCHES].map(|counter| counter.with(std::cell::Cell::get));
        assert!(
            searches * 100 < lookups,
            "{what}: {searches} searches in {lookups} lookups"
        );
        for (k, got) in walked.iter().enumerate() {
            let want = fresh().at(at(k));
            assert!(
                got.0.to_array().map(f32::to_bits) == want.0.to_array().map(f32::to_bits)
                    && got.1.to_array().map(f32::to_bits) == want.1.to_array().map(f32::to_bits),
                "{what} at {}: {got:?}, a search finds {want:?}",
                at(k)
            );
        }
    }
}

#[test]
fn test_flow_offset_curve_holds_its_distance_round_a_corner_its_seam_splits_a_side_of() {
    do_flow_offset_curve_holds_its_distance_round_a_corner_its_seam_splits_a_side_of();
}

/// #308 (review 5801763546): a closed path's offset curve
/// (`ArcPath::offset_at`) lies its offset from the outline all round,
/// and moves on continuously, inside a sharp corner whose side the path's
/// seam cuts in two as well as anywhere: round a 200 × 20 rectangle
/// started at the middle of its left side, as a frame's halves are, and
/// at the middle of its top, sharp and with a quarter's corners, inward
/// at 6 and 9 — past a quarter of its height, short of half, so no
/// corner's stretch of the curve inside it is held to the shorter of its
/// sides — and outward at 3 and 9; sampled every 0.05 along the path,
/// each point within a thousandth of its offset from the outline
/// (`signed_distance`), and none more than five samples' length from the
/// one before: the curve moves on faster than the path round a bend
/// outward, 4.6 times round the quarter's corners, radius 2.5, at 9, and
/// a step where the curve jumps is a point or more.
///
/// Control: with each corner's sides read as the stretch beside it
/// alone, the seam's half of a side, red started mid-left inward at 6: a
/// point 1.0 off its offset, and the curve stepping 1.0 (the reviewer's
/// probe, every 0.011: 0.996 at 6 and 3.996 at 9).
pub fn do_flow_offset_curve_holds_its_distance_round_a_corner_its_seam_splits_a_side_of() {
    use crate::flow::tests::halves_tests::signed_distance;
    let rect = Rect::new(Vec2::ZERO, Vec2::new(200.0, 20.0));
    for (name, shape) in [
        (
            "sharp",
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        (
            "rounded",
            Shape::Rectangle {
                radius: Percent::new(25),
            },
        ),
    ] {
        let outline = outline(rect, shape);
        for edge in [Edge::Left, Edge::Top] {
            let path = ArcPath::closed(&outline, StartPoint { edge, at: 0.5 }, Direction::Clockwise, 0.25);
            for d in [-6.0_f32, -9.0, 3.0, 9.0] {
                let step = 0.05;
                let samples = (path.length() / step) as usize;
                let (mut off, mut jump) = (0.0_f32, 0.0_f32);
                let mut last: Option<Vec2> = None;
                for k in 0..=samples {
                    let p = path.offset_at(k as f32 * step, d).point;
                    off = off.max((signed_distance(&path, p) - d).abs());
                    if let Some(q) = last {
                        jump = jump.max(p.distance(q));
                    }
                    last = Some(p);
                }
                assert!(
                    off <= 1e-3 && jump <= 5.0 * step,
                    "{name}, started mid-{edge:?}, at {d}: a point {off} off its offset, a step of {jump} between neighbors"
                );
            }
        }
    }
}
