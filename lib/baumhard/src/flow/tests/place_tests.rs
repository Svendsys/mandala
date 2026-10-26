// SPDX-License-Identifier: MPL-2.0

//! Placement geometry (#239 §5): step lengths on lines of every
//! direction, glyph centers on the outline (and outside it by half
//! their extent), start points and directions, ring offsets, and
//! point placement. Runs here are synthetic, so every expected position
//! is a literal or a closed-form expression of the fixture's numbers.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::tests::support::{center, close, style, synthetic, with_fonts};
use crate::flow::{
    offset, outline, place_at_point, place_on_closed, place_on_open, ring_offset, Curve, Outline, PlacedRun,
    Rect, ShapedRun,
};

const ONCE: FitMode = FitMode::Once { filler: None };

/// `run` placed along `curve` in a mode that places the run it is given.
fn open(run: &ShapedRun, curve: Curve, fit: &FitMode, spacing: f32) -> PlacedRun {
    let (rescaled, placed) = with_fonts(|fs| place_on_open(run, &curve, fit, spacing, &style(12.0), fs));
    assert!(rescaled.is_none(), "these modes place the run they are given");
    placed
}

/// `run` placed around `path` with `fit`, from `start` in `direction`.
fn closed(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    outward: bool,
) -> PlacedRun {
    let (rescaled, placed) =
        with_fonts(|fs| place_on_closed(run, path, start, direction, fit, outward, &style(12.0), fs));
    assert!(rescaled.is_none(), "these modes place the run they are given");
    placed
}

/// Every placed glyph's box center.
fn centers(run: &ShapedRun, placed: &PlacedRun) -> Vec<Vec2> {
    placed
        .glyphs
        .iter()
        .map(|g| center(g.pos, run.glyphs[g.glyph as usize].advance, run.line_height))
        .collect()
}

fn sharp(x: f32, y: f32, w: f32, h: f32) -> Outline {
    outline(
        Rect::new(Vec2::new(x, y), Vec2::new(w, h)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    )
}

fn start(edge: Edge, at: f32) -> StartPoint {
    StartPoint { edge, at }
}

#[test]
fn test_flow_place_horizontal_steps_by_advance() {
    do_flow_place_horizontal_steps_by_advance();
}

/// Along a horizontal line each glyph steps its advance plus the
/// spacing, and its box is centered on the line: advances 5, 7, 9 with
/// spacing 3 from x = 10 put the boxes at 10, 18 and 28.
pub fn do_flow_place_horizontal_steps_by_advance() {
    let run = synthetic(&[5.0, 7.0, 9.0], 12.0);
    let placed = open(
        &run,
        Curve::Line(Vec2::new(10.0, 50.0), Vec2::new(500.0, 50.0)),
        &ONCE,
        3.0,
    );
    assert!(!placed.truncated);
    let pos: Vec<Vec2> = placed.glyphs.iter().map(|g| g.pos).collect();
    let expected = [
        Vec2::new(10.0, 44.0),
        Vec2::new(18.0, 44.0),
        Vec2::new(28.0, 44.0),
    ];
    assert_eq!(pos.len(), 3);
    for (got, want) in pos.iter().zip(expected) {
        assert!(got.distance(want) < 1e-3, "{got} vs {want}");
    }
}

#[test]
fn test_flow_place_vertical_steps_by_line_height() {
    do_flow_place_vertical_steps_by_line_height();
}

/// Along a vertical line each glyph steps the line height plus the
/// spacing, whatever its advance — glyphs stack a line apart — and its
/// box is centered on the line.
pub fn do_flow_place_vertical_steps_by_line_height() {
    let run = synthetic(&[5.0, 7.0, 9.0], 12.0);
    let placed = open(
        &run,
        Curve::Line(Vec2::new(20.0, 0.0), Vec2::new(20.0, 500.0)),
        &ONCE,
        2.0,
    );
    let pos: Vec<Vec2> = placed.glyphs.iter().map(|g| g.pos).collect();
    let expected = [Vec2::new(17.5, 0.0), Vec2::new(16.5, 14.0), Vec2::new(15.5, 28.0)];
    assert_eq!(pos.len(), 3);
    for (got, want) in pos.iter().zip(expected) {
        assert!(got.distance(want) < 1e-3, "{got} vs {want}");
    }
}

#[test]
fn test_flow_place_diagonal_step_blends_advance_and_height() {
    do_flow_place_diagonal_step_blends_advance_and_height();
}

/// On a 45° line a glyph's step is `(w + h)/√2`: consecutive centers
/// are that far apart, and the first center is half of it from the
/// start, on the line.
pub fn do_flow_place_diagonal_step_blends_advance_and_height() {
    let (w, h) = (10.0, 14.0);
    let run = synthetic(&[w; 4], h);
    let placed = open(&run, Curve::Line(Vec2::ZERO, Vec2::new(400.0, 400.0)), &ONCE, 0.0);
    let step = (w + h) / 2.0_f32.sqrt();
    let c = centers(&run, &placed);
    assert_eq!(c.len(), 4);
    close(c[0].length(), step / 2.0, 1e-3, "first center");
    close(c[0].x, c[0].y, 1e-3, "on the line");
    for pair in c.windows(2) {
        close(pair[0].distance(pair[1]), step, 1e-3, "step");
    }
}

#[test]
fn test_flow_place_step_is_taken_at_its_middle() {
    do_flow_place_step_is_taken_at_its_middle();
}

/// A glyph's step is resolved at its middle, not at its start (#239 §5
/// step 2, the second round). On a sharp 100 × 100 square from x = 75
/// on the top side, the second glyph (advance 20, height 10) starts at
/// x = 95, still on the top side, but the middle of a 20-step would be
/// 5 down the right side: re-taken there, its step is the line height,
/// 10, so its center is half of that on, exactly at the corner
/// (100, 0). Taking θ only at the glyph's start would give a step of 20
/// and a center at (100, 5).
pub fn do_flow_place_step_is_taken_at_its_middle() {
    let run = synthetic(&[20.0, 20.0], 10.0);
    let path = sharp(0.0, 0.0, 100.0, 100.0);
    let placed = closed(
        &run,
        &path,
        start(Edge::Top, 0.75),
        Direction::Clockwise,
        &ONCE,
        false,
    );
    let c = centers(&run, &placed);
    assert!(c[0].distance(Vec2::new(85.0, 0.0)) < 1e-3, "{}", c[0]);
    assert!(c[1].distance(Vec2::new(100.0, 0.0)) < 1e-3, "{}", c[1]);
}

#[test]
fn test_flow_place_sharp_rect_touches_every_side() {
    do_flow_place_sharp_rect_touches_every_side();
}

/// Around a sharp rectangle, glyph centers (not pushed outward) lie on
/// the rectangle's boundary, and some lie on each of its four sides.
pub fn do_flow_place_sharp_rect_touches_every_side() {
    let run = synthetic(&[8.0; 4], 10.0);
    let path = sharp(0.0, 0.0, 200.0, 100.0);
    let placed = closed(
        &run,
        &path,
        start(Edge::Top, 0.0),
        Direction::Clockwise,
        &FitMode::Repeat,
        false,
    );
    let c = centers(&run, &placed);
    assert!(c.len() > 20);
    let on = |v: f32, line: f32| (v - line).abs() < 1e-3;
    let inside = |v: f32, high: f32| (0.0..=high).contains(&v);
    for p in &c {
        let boundary = (on(p.y, 0.0) || on(p.y, 100.0)) && inside(p.x, 200.0)
            || (on(p.x, 0.0) || on(p.x, 200.0)) && inside(p.y, 100.0);
        assert!(boundary, "{p} is off the rectangle");
    }
    assert!(c.iter().any(|p| on(p.y, 0.0) && p.x > 1.0 && p.x < 199.0), "top");
    assert!(
        c.iter().any(|p| on(p.x, 200.0) && p.y > 1.0 && p.y < 99.0),
        "right"
    );
    assert!(
        c.iter().any(|p| on(p.y, 100.0) && p.x > 1.0 && p.x < 199.0),
        "bottom"
    );
    assert!(c.iter().any(|p| on(p.x, 0.0) && p.y > 1.0 && p.y < 99.0), "left");
}

#[test]
fn test_flow_place_round_square_is_a_circle() {
    do_flow_place_round_square_is_a_circle();
}

/// Radius 100% on a square is a circle. Placed around the outline
/// offset by 7 without the outward push, every glyph center is on the
/// offset circle; with it, every center is further out by exactly
/// `|n.x|·w/2 + |n.y|·h/2` along its radial direction `n`.
pub fn do_flow_place_round_square_is_a_circle() {
    let (w, h) = (6.0, 10.0);
    let run = synthetic(&[w; 5], h);
    let square = outline(
        Rect::new(Vec2::ZERO, Vec2::splat(100.0)),
        Shape::Rectangle {
            radius: Percent::new(100),
        },
    );
    let ring = offset(&square, 7.0);
    let origin = Vec2::splat(50.0);
    let from = start(Edge::Right, 0.2);
    let on = centers(
        &run,
        &closed(&run, &ring, from, Direction::Clockwise, &FitMode::Repeat, false),
    );
    assert!(on.len() > 30);
    for p in &on {
        close(p.distance(origin), 57.0, 0.5, "on the offset circle");
    }
    let out = centers(
        &run,
        &closed(&run, &ring, from, Direction::Clockwise, &FitMode::Repeat, true),
    );
    assert!(out.len() > 30);
    for p in &out {
        let n = (*p - origin).normalize();
        let push = n.x.abs() * w / 2.0 + n.y.abs() * h / 2.0;
        close(
            p.distance(origin),
            57.0 + push,
            0.5,
            "pushed outward by half its extent",
        );
    }
}

#[test]
fn test_flow_place_start_and_direction() {
    do_flow_place_start_and_direction();
}

/// From the top-left corner, clockwise text runs rightward along the
/// top; counter-clockwise it runs downward along the left side.
pub fn do_flow_place_start_and_direction() {
    let run = synthetic(&[8.0; 3], 10.0);
    let path = sharp(0.0, 0.0, 200.0, 100.0);
    let top_left = start(Edge::Top, 0.0);
    let cw = centers(
        &run,
        &closed(&run, &path, top_left, Direction::Clockwise, &ONCE, false),
    );
    assert!(cw[1].x > cw[0].x + 1.0, "rightward: {:?}", &cw[..2]);
    close(cw[1].y, cw[0].y, 1e-3, "level");
    close(cw[0].y, 0.0, 1e-3, "on the top side");
    let ccw = centers(
        &run,
        &closed(&run, &path, top_left, Direction::CounterClockwise, &ONCE, false),
    );
    assert!(ccw[1].y > ccw[0].y + 1.0, "downward: {:?}", &ccw[..2]);
    close(ccw[1].x, ccw[0].x, 1e-3, "plumb");
    close(ccw[0].x, 0.0, 1e-3, "on the left side");
}

#[test]
fn test_flow_place_rings_stack_outward() {
    do_flow_place_rings_stack_outward();
}

/// Ring 1 runs outside ring 0 by `h₀ + gap`, measured from the outline
/// to glyph centers, for rings of one line height; ring 0's centers sit
/// `inset + h/2` off the outline.
pub fn do_flow_place_rings_stack_outward() {
    let h = 10.0;
    let run = synthetic(&[6.0; 3], h);
    let path = sharp(0.0, 0.0, 200.0, 100.0);
    let (inset, gap) = (4.0, 3.0);
    let ring = |inner: &[f32]| {
        let path = offset(&path, ring_offset(inset, gap, inner));
        let placed = closed(
            &run,
            &path,
            start(Edge::Top, 0.5),
            Direction::Clockwise,
            &ONCE,
            true,
        );
        centers(&run, &placed)[0]
    };
    let first = ring(&[]);
    let second = ring(&[h]);
    close(-first.y, inset + h / 2.0, 1e-3, "ring 0 off the outline");
    close(first.y - second.y, h + gap, 1e-3, "ring 1 outside ring 0");
}

#[test]
fn test_flow_place_on_an_ellipse() {
    do_flow_place_on_an_ellipse();
}

/// Around an ellipse, every glyph center (not pushed outward) is on the
/// ellipse.
pub fn do_flow_place_on_an_ellipse() {
    let run = synthetic(&[7.0, 5.0, 9.0], 12.0);
    let (a, b) = (150.0, 60.0);
    let ellipse = Outline::Ellipse {
        rect: Rect::new(Vec2::new(-a, -b), Vec2::new(2.0 * a, 2.0 * b)),
    };
    let c = centers(
        &run,
        &closed(
            &run,
            &ellipse,
            start(Edge::Bottom, 0.1),
            Direction::CounterClockwise,
            &FitMode::Repeat,
            false,
        ),
    );
    assert!(c.len() > 40);
    for p in &c {
        close((p.x / a).powi(2) + (p.y / b).powi(2), 1.0, 1e-3, "on the ellipse");
    }
}

#[test]
fn test_flow_place_on_a_bezier() {
    do_flow_place_on_a_bezier();
}

/// Along a quadratic, every glyph center is on the curve: the nearest
/// of 20 000 dense samples is within a twentieth of a point.
pub fn do_flow_place_on_a_bezier() {
    let run = synthetic(&[7.0, 5.0, 9.0], 12.0);
    let curve = Curve::Quadratic(Vec2::ZERO, Vec2::new(150.0, -200.0), Vec2::new(300.0, 0.0));
    let placed = open(&run, curve, &FitMode::Repeat, 1.0);
    let c = centers(&run, &placed);
    assert!(c.len() > 20);
    let dense: Vec<Vec2> = (0..=20_000).map(|i| curve.point(i as f32 / 20_000.0)).collect();
    for p in &c {
        let nearest = dense.iter().map(|q| q.distance(*p)).fold(f32::MAX, f32::min);
        assert!(nearest < 0.05, "{p} is {nearest} off the curve");
    }
}

#[test]
fn test_flow_place_at_point_centers_the_line() {
    do_flow_place_at_point_centers_the_line();
}

/// A run placed at a point is one line centered there: advances 4 and 6
/// around (100, 50) at line height 10 put the boxes at (95, 45) and
/// (99, 45).
pub fn do_flow_place_at_point_centers_the_line() {
    let run = synthetic(&[4.0, 6.0], 10.0);
    let placed = place_at_point(&run, Vec2::new(100.0, 50.0));
    let pos: Vec<Vec2> = placed.glyphs.iter().map(|g| g.pos).collect();
    assert_eq!(pos, vec![Vec2::new(95.0, 45.0), Vec2::new(99.0, 45.0)]);
    assert!(!placed.truncated);
}
