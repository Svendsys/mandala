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
/// numbers: what [`do_flow_a_closed_paths_table_samples_are_known_before_it_is_built`]
/// and [`do_flow_a_closed_paths_length_is_finite_as_its_build_says`] walk.
fn closed_outlines() -> Vec<Outline> {
    let sizes = [0.5, 200.0, 1e5, 1e30, 1.5e38, f32::MAX, f32::NAN];
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
/// sample, over [`closed_outlines`] at the finest spacing, a font's and
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
fn test_flow_a_closed_paths_length_is_finite_as_its_build_says() {
    do_flow_a_closed_paths_length_is_finite_as_its_build_says();
}

/// `ArcPath::closed_length_is_finite` answers whether `ArcPath::closed`'s
/// path has a finite length as the path built says, from every edge both
/// ways, over [`closed_outlines`]: an ellipse's always is, so none is
/// built for it (#309) — the frame's probe that asked was the largest
/// table of its setup, at the finest spacing, and the answer always yes.
/// Rounded rectangles answer both ways (sides of 1.5 × 10³⁸ sum past
/// `f32::MAX`), and ellipses so far out that their table is built and
/// dropped, its points not finite, leave paths of no length.
///
/// Control: `closed_length_is_finite` answering `true` for a rounded
/// rectangle too, red on the overflowing ones.
pub fn do_flow_a_closed_paths_length_is_finite_as_its_build_says() {
    let (mut infinite, mut dropped) = (0, 0);
    let edges = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];
    for (n, path) in closed_outlines().iter().enumerate() {
        let direction = if n % 2 == 0 {
            Direction::Clockwise
        } else {
            Direction::CounterClockwise
        };
        let from = start(edges[n % 4], 0.7);
        let built = ArcPath::closed(path, from, direction, 1e30);
        assert_eq!(
            ArcPath::closed_length_is_finite(path, from, direction),
            built.length().is_finite(),
            "{path:?}"
        );
        infinite += usize::from(!built.length().is_finite());
        if matches!(path, Outline::Ellipse { .. }) {
            assert!(built.length().is_finite(), "{path:?}: {}", built.length());
            dropped += usize::from(built.table_samples() > 0 && built.length() == 0.0);
        }
    }
    assert!(
        infinite > 0 && dropped > 0,
        "infinite {infinite}, dropped {dropped}"
    );
}

#[test]
fn test_flow_a_curve_faster_than_the_largest_f32_is_tabled() {
    do_flow_a_curve_faster_than_the_largest_f32_is_tabled();
}

/// A curve whose every control point and leg is finite but whose speed
/// bound — the degree times its longest leg — is past the largest `f32`
/// still builds its table: the bound saturates there, and its path has
/// its length. Seed 1021 of the structured flow generator: a quadratic
/// from (1, 1) by (−2.4 × 10³⁸, −587) to (−10³⁰, 1), 2.4 × 10³⁸ long,
/// whose bound of twice its first leg was infinite, so that it built no
/// table and its `Spread` was cut to nothing though its text fit.
///
/// Control: with the bound not saturated, no table and no length.
pub fn do_flow_a_curve_faster_than_the_largest_f32_is_tabled() {
    let curve = Curve::Quadratic(
        Vec2::new(1.0, 1.0),
        Vec2::new(-2.409_018_9e38, -587.0),
        Vec2::new(-1e30, 1.0),
    );
    assert_eq!(
        curve.speed_bound(),
        f32::MAX,
        "the bound saturates at the largest f32"
    );
    let path = ArcPath::open(&curve, 4.8);
    assert!(path.table_samples() > 0, "the curve builds its table");
    assert!(
        path.length().is_finite() && path.length() > 2.4e38,
        "its path is as long as it is: {}",
        path.length()
    );
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
fn test_flow_path_measures_past_the_root_of_the_largest_f32() {
    do_flow_path_measures_past_the_root_of_the_largest_f32();
}

/// A vector, a line, a curve and an outline longer than `√f32::MAX`
/// (about `1.8e19`) — whose squared length passes every `f32` — measure
/// their lengths, each against the same length worked in `f64`, and
/// walk to their ends. Control: measured by `Vec2::length`, each is
/// infinite or not walked at all.
pub fn do_flow_path_measures_past_the_root_of_the_largest_f32() {
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
    // A line from -1e20 to 1e20, and a diagonal one.
    let (a, b) = (Vec2::new(-1.0e20, 0.0), Vec2::new(1.0e20, 0.0));
    let line = ArcPath::open(&Curve::Line(a, b), 1.0);
    near(line.length(), 2.0e20, "a line");
    assert!(
        line.at(line.length()).0.distance(b) <= 1.0e15,
        "the line walks to its end"
    );
    let (a, b) = (Vec2::splat(-1.0e37), Vec2::splat(1.0e37));
    near(
        ArcPath::open(&Curve::Line(a, b), 1.0).length(),
        2.0e37 * std::f64::consts::SQRT_2,
        "a diagonal line",
    );
    // A curve with a control point far out: its length lies between its
    // chord and its control polygon.
    let (a, c, b) = (
        Vec2::new(0.0, 0.0),
        Vec2::new(5.0e25, 0.0),
        Vec2::new(0.0, 1.0e25),
    );
    let curve = ArcPath::open(&Curve::Quadratic(a, c, b), 1.0);
    let polygon = (c - a).as_dvec2().length() + (b - c).as_dvec2().length();
    let chord = (b - a).as_dvec2().length();
    assert!(
        (curve.length() as f64) >= chord && (curve.length() as f64) <= polygon * (1.0 + 1e-5),
        "a curve {:e}, between {chord:e} and {polygon:e}",
        curve.length()
    );
    // A rounded rectangle 1e30 wide.
    let (w, h, r) = (1.0e30_f64, 40.0_f64, 10.0_f64);
    let rounded = outline(
        Rect::new(Vec2::ZERO, Vec2::new(w as f32, h as f32)),
        Shape::Rectangle {
            radius: Percent::new(50),
        },
    );
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    let path = ArcPath::closed(&rounded, start, Direction::Clockwise, 1.0);
    near(
        path.length(),
        2.0 * (w - 2.0 * r) + 2.0 * (h - 2.0 * r) + 2.0 * PI as f64 * r,
        "an outline",
    );
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
