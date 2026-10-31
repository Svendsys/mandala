// SPDX-License-Identifier: MPL-2.0

//! Placement geometry (#239 §5): step lengths on lines of every
//! direction, glyph centers on the outline (and outside it by half
//! their extent), start points and directions, ring offsets, and
//! point placement. Runs here are synthetic, so every expected position
//! is a literal or a closed-form expression of the fixture's numbers.

use glam::Vec2;

use crate::flow::path::ArcPath;
use crate::flow::place::box_step;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::tests::support::{
    center, close, de_casteljau, positioned, real_run, style, synthetic, with_fonts,
};
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
fn test_flow_place_step_follows_the_chord_at_a_corner() {
    do_flow_place_step_follows_the_chord_at_a_corner();
}

/// At a sharp corner a step's direction is the chord it spans (#239 §5
/// step 2, ruling R9.2). On a sharp 100 × 100 square from x = 75 on the
/// top side, the second glyph (advance 20, height 10) starts at x = 95.
/// Worked by hand:
///
/// 1. seed: the direction at 95 is level, so 20;
/// 2. the chord over 20, from (95, 0) to (100, 15), gives
///    `(5·20 + 15·10)/√(5² + 15²) = 15.811`;
/// 3. the chord over that, to (100, 10.811), gives
///    `(5·20 + 10.811·10)/√(5² + 10.811²) = 17.472`.
///
/// So the center is 8.736 on, at (100, 3.736): between the corner, where
/// the tangent taken half a step on would put it, and (100, 5), where the
/// tangent at the glyph's start alone would. The first glyph, entirely
/// on the top side, steps its advance either way. The steps are taken
/// with the walk's own rule (`box_step`) on the outline's path, cluster
/// after cluster from the start with no gap: a frame ring's placement
/// adds its even spacing (ruling R29) on top of them.
pub fn do_flow_place_step_follows_the_chord_at_a_corner() {
    let path = ArcPath::closed(
        &sharp(0.0, 0.0, 100.0, 100.0),
        start(Edge::Top, 0.75),
        Direction::Clockwise,
        1.0,
    );
    let (first, _) = box_step(&path, 0.0, 20.0, 10.0);
    let (second, _) = box_step(&path, first, 20.0, 10.0);
    let c = [path.at(first * 0.5).0, path.at(first + second * 0.5).0];
    assert!(c[0].distance(Vec2::new(85.0, 0.0)) < 1e-3, "{}", c[0]);
    assert!(c[1].distance(Vec2::new(100.0, 3.7358)) < 1e-3, "{}", c[1]);
}

#[test]
fn test_flow_place_corner_moves_continuously() {
    do_flow_place_corner_moves_continuously();
}

/// A glyph walked past a sharp corner in hundredth-of-a-point steps
/// never jumps: between consecutive start points its center moves by
/// little more than the start point does. A step taken from the tangent
/// alone jumps by up to half the difference of advance and line height
/// (here 25) as its middle crosses the corner. The steps are the walk's
/// own (`box_step`), from the start with no gap, as in
/// `test_flow_place_step_follows_the_chord_at_a_corner`.
pub fn do_flow_place_corner_moves_continuously() {
    let square = sharp(0.0, 0.0, 100.0, 100.0);
    let mut previous: Option<Vec2> = None;
    for i in 0..=2000 {
        let at = 0.5 + 0.5 * i as f32 / 2000.0;
        let path = ArcPath::closed(&square, start(Edge::Top, at), Direction::Clockwise, 1.0);
        let (first, _) = box_step(&path, 0.0, 10.0, 10.0);
        let (second_step, _) = box_step(&path, first, 60.0, 10.0);
        let second = path.at(first + second_step * 0.5).0;
        if let Some(previous) = previous {
            let moved = second.distance(previous);
            assert!(moved < 0.5, "at {at}: the second glyph jumped {moved}");
        }
        previous = Some(second);
    }
}

#[test]
fn test_flow_place_keeps_marks_with_their_letters() {
    do_flow_place_keeps_marks_with_their_letters();
}

/// The grapheme cluster is the unit of placement (ruling R9.1). Down a
/// vertical line, a combining mark, a stack of two, a Hebrew point (RTL,
/// listed before its letter), and the ZWJ of an unligated sequence all
/// sit in their base's row at their pen position, and each cluster is
/// one line below the last: as many rows as clusters. A glyph's place
/// in its cluster's box is its pen position as shaped, relative to the
/// cluster's first (leftmost) glyph; the Hebrew point shares its
/// letter's pen position.
pub fn do_flow_place_keeps_marks_with_their_letters() {
    for text in [
        "x\u{301}y",
        "q\u{323}\u{302}z",
        "\u{5D0}\u{5B8}\u{5D1}",
        "a\u{200D}b",
        "\u{1F469}\u{200D}\u{1F4BB}e",
    ] {
        let (run, placed) = with_fonts(|fs| {
            let run = real_run(text, &style(12.0), fs);
            let (_, placed) = place_on_open(
                &run,
                &Curve::Line(Vec2::ZERO, Vec2::new(0.0, 500.0)),
                &ONCE,
                0.0,
                &style(12.0),
                fs,
            );
            (run, placed)
        });
        let clusters = run.glyphs.iter().filter(|g| g.grapheme_start).count();
        assert!(
            clusters < run.glyphs.len() || text.starts_with('\u{1F469}'),
            "{text:?}: the fixture needs a glyph inside a cluster"
        );
        assert_eq!(placed.glyphs.len(), run.glyphs.len(), "{text:?}");
        let mut row = 0usize;
        let mut first = placed.glyphs[0];
        let mut first_x = run.glyphs[first.glyph as usize].x;
        for placed_glyph in &placed.glyphs {
            let glyph = &run.glyphs[placed_glyph.glyph as usize];
            if glyph.grapheme_start && placed_glyph.glyph != placed.glyphs[0].glyph {
                close(
                    placed_glyph.pos.y - first.pos.y,
                    run.line_height,
                    1e-3,
                    "the next cluster is a line down",
                );
                first = *placed_glyph;
                first_x = glyph.x;
                row += 1;
            }
            close(
                placed_glyph.pos.y,
                first.pos.y,
                1e-3,
                "a mark keeps its base's row",
            );
            close(
                placed_glyph.pos.x - first.pos.x,
                glyph.x - first_x,
                1e-3,
                "at its pen position as shaped",
            );
        }
        assert_eq!(row + 1, clusters, "{text:?}: one row per cluster");
    }
}

#[test]
fn test_flow_place_sets_glyphs_at_their_pen_positions() {
    do_flow_place_sets_glyphs_at_their_pen_positions();
}

/// Each glyph of a cluster sits in the cluster's box at its pen position
/// as shaped, not at its predecessors' advances laid end to end (ruling
/// R9.1). A letter of advance 8 at pen 0 with a mark after it that
/// shares its pen position — as a right-to-left mark does — puts the
/// mark at the letter's x, not 8 to its right; a mark at pen 8 sits 8
/// to the right. Down a vertical line both share their letter's row.
pub fn do_flow_place_sets_glyphs_at_their_pen_positions() {
    let shared = positioned(&[(8.0, 0.0, true), (0.0, 0.0, false), (6.0, 8.0, true)], 10.0);
    let after = positioned(&[(8.0, 0.0, true), (0.0, 8.0, false), (6.0, 8.0, true)], 10.0);
    let down = Curve::Line(Vec2::ZERO, Vec2::new(0.0, 500.0));
    let shared = open(&shared, down, &ONCE, 0.0);
    let after = open(&after, down, &ONCE, 0.0);
    close(
        shared.glyphs[1].pos.x,
        shared.glyphs[0].pos.x,
        1e-3,
        "a mark sharing its letter's pen",
    );
    close(
        after.glyphs[1].pos.x,
        after.glyphs[0].pos.x + 8.0,
        1e-3,
        "a mark after its letter",
    );
    for placed in [&shared, &after] {
        close(
            placed.glyphs[1].pos.y,
            placed.glyphs[0].pos.y,
            1e-3,
            "the mark's row",
        );
        close(
            placed.glyphs[2].pos.y - placed.glyphs[0].pos.y,
            10.0,
            1e-3,
            "the next cluster's row",
        );
    }
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

/// From the top-left corner, clockwise a `Repeat` ring runs rightward
/// along the top; counter-clockwise it runs downward along the left side.
/// (`start` and `direction` belong to `Repeat` rings: a text frame reads
/// in two lines, over the top and along the bottom — ruling R27.)
pub fn do_flow_place_start_and_direction() {
    let run = synthetic(&[8.0; 3], 10.0);
    let path = sharp(0.0, 0.0, 200.0, 100.0);
    let top_left = start(Edge::Top, 0.0);
    let cw = centers(
        &run,
        &closed(
            &run,
            &path,
            top_left,
            Direction::Clockwise,
            &FitMode::Repeat,
            false,
        ),
    );
    assert!(cw[1].x > cw[0].x + 1.0, "rightward: {:?}", &cw[..2]);
    close(cw[1].y, cw[0].y, 1e-3, "level");
    close(cw[0].y, 0.0, 1e-3, "on the top side");
    let ccw = centers(
        &run,
        &closed(
            &run,
            &path,
            top_left,
            Direction::CounterClockwise,
            &FitMode::Repeat,
            false,
        ),
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
            &FitMode::Repeat,
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
/// of 20 000 dense samples, evaluated here by de Casteljau rather than
/// through `Curve`, is within a twentieth of a point.
pub fn do_flow_place_on_a_bezier() {
    let run = synthetic(&[7.0, 5.0, 9.0], 12.0);
    let points = [Vec2::ZERO, Vec2::new(150.0, -200.0), Vec2::new(300.0, 0.0)];
    let curve = Curve::Quadratic(points[0], points[1], points[2]);
    let placed = open(&run, curve, &FitMode::Repeat, 1.0);
    let c = centers(&run, &placed);
    assert!(c.len() > 20);
    let dense: Vec<Vec2> = (0..=20_000)
        .map(|i| de_casteljau(&points, i as f32 / 20_000.0))
        .collect();
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

#[test]
fn test_flow_place_keeps_every_position_finite() {
    do_flow_place_keeps_every_position_finite();
}

/// Every position placement returns is finite, whatever the path
/// (geometry is checked at load; this is the fuzz target's guarantee,
/// #278). A curve with a point that is not finite has no finite
/// stretch, so it is an empty path that answers the origin; a rounded
/// rectangle with a side that is not a number keeps only its finite
/// stretches. Two marks with no base step zero along a horizontal
/// tangent, so they fit even an empty path and are placed — at the
/// origin, not at `NaN` — and `"hello"` is cut, or placed on what is
/// finite. Every mode, and a point placement at a center that is not
/// finite.
pub fn do_flow_place_keeps_every_position_finite() {
    let (marks, hello) = with_fonts(|fs| {
        (
            real_run("\u{301}\u{302}", &style(12.0), fs),
            real_run("hello", &style(12.0), fs),
        )
    });
    let nan = f32::NAN;
    let inf = f32::INFINITY;
    let curves = [
        Curve::Line(Vec2::ZERO, Vec2::new(nan, 3.0)),
        Curve::Line(Vec2::ZERO, Vec2::new(inf, 3.0)),
        Curve::Cubic(
            Vec2::ZERO,
            Vec2::new(nan, 10.0),
            Vec2::new(20.0, 10.0),
            Vec2::new(30.0, 0.0),
        ),
        Curve::Quadratic(Vec2::ZERO, Vec2::new(10.0, inf), Vec2::new(20.0, 0.0)),
    ];
    let modes = [
        ONCE,
        FitMode::Once {
            filler: Some("-".to_string()),
        },
        FitMode::Repeat,
        FitMode::Spread,
        FitMode::Fit { min: 0.5, max: 2.0 },
    ];
    let finite = |placed: &PlacedRun, what: &str| {
        for glyph in &placed.glyphs {
            assert!(glyph.pos.is_finite(), "{what}: placed at {}", glyph.pos);
        }
    };
    for (c, curve) in curves.iter().enumerate() {
        for fit in &modes {
            for (name, run) in [("marks", &marks), ("hello", &hello)] {
                let (_, placed) = with_fonts(|fs| place_on_open(run, curve, fit, 0.0, &style(12.0), fs));
                finite(&placed, &format!("curve {c}, {fit:?}, {name}"));
                if name == "marks" && *fit == ONCE {
                    assert_eq!(
                        placed.glyphs.len(),
                        marks.glyphs.len(),
                        "curve {c}: the fixture needs the marks placed"
                    );
                }
            }
        }
    }
    let broken = outline(
        Rect::new(Vec2::ZERO, Vec2::new(nan, 10.0)),
        Shape::Rectangle {
            radius: Percent::new(100),
        },
    );
    let top = StartPoint {
        edge: Edge::Top,
        at: 0.5,
    };
    let mut placed_on_broken = 0;
    for fit in &modes {
        for (name, run) in [("marks", &marks), ("hello", &hello)] {
            let (_, placed) = with_fonts(|fs| {
                place_on_closed(
                    run,
                    &broken,
                    top,
                    Direction::Clockwise,
                    fit,
                    true,
                    &style(12.0),
                    fs,
                )
            });
            finite(&placed, &format!("NaN rect, {fit:?}, {name}"));
            placed_on_broken += placed.glyphs.len();
        }
    }
    assert!(
        placed_on_broken > 0,
        "the fixture needs glyphs placed on the NaN rect"
    );
    finite(&place_at_point(&hello, Vec2::new(nan, 4.0)), "point at NaN");
    finite(&place_at_point(&hello, Vec2::splat(inf)), "point at infinity");
}
