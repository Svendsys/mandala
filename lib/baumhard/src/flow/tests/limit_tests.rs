// SPDX-License-Identifier: MPL-2.0

//! The longest path placement lays (`MAX_PATH_LENGTH`, #363, ruling
//! B42.1): a path within it is laid, one past it lays nothing and says
//! why, by one name on every kind of placement — a frame, a ring, a
//! directional ring, a placement along an open path and a pitched one.
//!
//! How long an outline or a curve is, is read here off its own numbers
//! in `f64` — a rectangle's sides and corners, an ellipse's polygon at
//! half-angle tangents, a Bézier's polyline by de Casteljau's construction
//! and its control polygon — never off the path the engine builds
//! (TEST_CONVENTIONS §T13, shape 6).

use glam::{DVec2, Vec2};

use crate::document::element::DirectionalGlyphs;
use crate::document::values::{Direction, Edge, FitMode, Rgba, StartPoint};
use crate::flow::place::pitch::place_pitched_capped;
use crate::flow::place::ring::RingLoss;
use crate::flow::place::{place_on_closed_capped, place_on_open_capped, Trace};
use crate::flow::shape::Ink;
use crate::flow::tests::support::{real_run, style, synthetic, with_fonts};
use crate::flow::{Curve, Outline, PathEnd, PlacedRun, Rect, RingLayout, ShapedRun, MAX_PATH_LENGTH};

/// The limit, in `f64`.
const LIMIT: f64 = MAX_PATH_LENGTH as f64;

/// The most glyphs a ring or a placement along a path lays here: the flow
/// fuzzers' cap.
const CAP: usize = 160;

/// An end of a pitched path with nothing beside it.
const NO_END: PathEnd<'static> = PathEnd {
    inks: &[],
    back: 0.0,
    hold: 0.0,
};

/// A kind of placement round a closed outline: its name, its text, its
/// fit, its layout, and whether it has a glyph to lay.
type ClosedKind = (String, &'static str, FitMode, RingLayout, bool);

/// Every kind of placement round a closed outline, a ring's spaced
/// `spacing`: a frame in each fit, with and without sides; a ring of
/// text and one of an ornament; a directional ring of its vertical text
/// alone, with spans, and with corners — and, with no glyph to lay, a
/// frame of no text in each fit, with sides and a `Once`'s filler, a
/// ring of no text, and a directional ring whose vertical text is empty
/// and whose spans are none.
fn closed_kinds(spacing: f32) -> Vec<ClosedKind> {
    let mut kinds: Vec<ClosedKind> = Vec::new();
    let sided = |sides: bool| RingLayout {
        sides: sides.then(|| ("─═".to_owned(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    for (name, fit) in [
        ("Once", FitMode::Once { filler: None }),
        ("Spread", FitMode::Spread),
        ("Fit", FitMode::Fit { min: 0.5, max: 2.0 }),
    ] {
        for sides in [false, true] {
            kinds.push((
                format!("a frame laid {name}{}", if sides { " with sides" } else { "" }),
                "To see a World in a Grain of Sand",
                fit.clone(),
                sided(sides),
                true,
            ));
        }
    }
    for (name, fit) in [
        (
            "Once with a filler",
            FitMode::Once {
                filler: Some("─".to_owned()),
            },
        ),
        ("Spread", FitMode::Spread),
        ("Fit", FitMode::Fit { min: 0.5, max: 2.0 }),
    ] {
        kinds.push((
            format!("a frame of no text laid {name} with sides"),
            "",
            fit,
            sided(true),
            false,
        ));
    }
    let directional = |vertical: &str, corners: bool| DirectionalGlyphs {
        vertical: Some(vertical.to_owned()),
        corners: corners.then(|| ["┌", "┐", "┘", "└"].map(str::to_owned)),
    };
    for (name, text, glyphs) in [
        ("a ring of text", "abc", DirectionalGlyphs::default()),
        ("a ring of an ornament", "─", DirectionalGlyphs::default()),
        ("a ring of no text", "", DirectionalGlyphs::default()),
        (
            "a directional ring of its vertical text alone",
            "",
            directional("a", false),
        ),
        (
            "a directional ring of spans and a vertical text",
            "─",
            directional("│", false),
        ),
        ("a directional ring with corners", "abc", directional("│", true)),
        (
            "a directional ring of an empty vertical text and no spans",
            "",
            directional("", false),
        ),
    ] {
        let lays = !text.is_empty()
            || glyphs
                .vertical
                .as_deref()
                .is_some_and(|vertical| !vertical.is_empty())
            || glyphs.corners.is_some();
        kinds.push((
            format!("{name} spaced {spacing:e}"),
            text,
            FitMode::Repeat,
            RingLayout {
                spacing,
                directional: glyphs,
                ..RingLayout::whole()
            },
            lays,
        ));
    }
    kinds
}

/// `text` laid round `path` as `fit` and `layout` say, from `at` of the
/// way along its top, and its trace.
fn lay_closed(
    path: &Outline,
    text: &str,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    at: f32,
) -> (PlacedRun, Trace) {
    let plain = style(16.0);
    with_fonts(|fs| {
        let run = real_run(text, &plain, fs);
        let mut trace = Trace::default();
        let (_, placed) = place_on_closed_capped(
            &run,
            path,
            StartPoint { edge: Edge::Top, at },
            Direction::Clockwise,
            fit,
            layout,
            outward,
            &plain,
            fs,
            CAP,
            Some(&mut trace),
        );
        (placed, trace)
    })
}

/// A run of `n` dots, each an ornament's glyph 6 wide whose ink is 2
/// square: what a pitched placement lays.
fn dots(n: usize) -> ShapedRun {
    let mut run = synthetic(&vec![6.0; n], 12.0);
    for glyph in &mut run.glyphs {
        glyph.letter = false;
        glyph.ink = Some(Ink {
            min: Vec2::new(2.0, -3.0),
            max: Vec2::new(4.0, -1.0),
        });
    }
    run
}

/// Every kind of placement along the open `path`, spaced `spacing`: its
/// name, whether it has a glyph to lay, what it laid and its trace. Text,
/// an ornament and no text in each fit, and no text as a `Once` with a
/// filler, which lays the filler; and a pitched pattern with nothing at
/// its ends and with a box at each, and one of no ink.
fn lay_open_kinds(path: &[Curve], spacing: f32) -> Vec<(String, bool, PlacedRun, Trace)> {
    let plain = style(16.0);
    let mut laid = Vec::new();
    with_fonts(|fs| {
        for (name, fit) in [
            ("Once", FitMode::Once { filler: None }),
            ("Repeat", FitMode::Repeat),
            ("Spread", FitMode::Spread),
            ("Fit", FitMode::Fit { min: 0.5, max: 2.0 }),
            (
                "Once with a filler",
                FitMode::Once {
                    filler: Some("─".to_owned()),
                },
            ),
        ] {
            for text in ["abc", "─", ""] {
                let run = real_run(text, &plain, fs);
                let lays = !text.is_empty() || matches!(fit, FitMode::Once { filler: Some(_) });
                let mut trace = Trace::default();
                let (_, placed) =
                    place_on_open_capped(&run, path, &fit, spacing, &plain, fs, CAP, Some(&mut trace));
                laid.push((
                    format!("{text:?} laid {name} spaced {spacing:e}"),
                    lays,
                    placed,
                    trace,
                ));
            }
        }
    });
    let (Some(first), Some(last)) = (path.first(), path.last()) else {
        return laid;
    };
    let side = Vec2::splat(12.0);
    let boxes = [
        [Rect::new(first.point(0.0) - side * 1.5, side)],
        [Rect::new(last.point(1.0) + side * 0.5, side)],
    ];
    let beside = [
        PathEnd {
            inks: &boxes[0],
            back: 6.0,
            hold: 0.0,
        },
        PathEnd {
            inks: &boxes[1],
            back: 6.0,
            hold: 0.0,
        },
    ];
    for (name, ends) in [
        ("nothing at its ends", [NO_END; 2]),
        ("a box at each end", beside),
    ] {
        let mut trace = Trace::default();
        let placed = place_pitched_capped(&dots(2), path, spacing, ends, CAP, Some(&mut trace));
        laid.push((
            format!("a pitched pattern spaced {spacing:e}, {name}"),
            true,
            placed,
            trace,
        ));
    }
    let mut trace = Trace::default();
    let placed = place_pitched_capped(
        &synthetic(&[6.0; 2], 12.0),
        path,
        spacing,
        [NO_END; 2],
        CAP,
        Some(&mut trace),
    );
    laid.push((
        format!("a pitched pattern of no ink spaced {spacing:e}"),
        false,
        placed,
        trace,
    ));
    laid
}

/// What a placement along a path past the limit owes, and does not give:
/// nothing laid, cut, and the reason by its name — where it `lays`, has a
/// glyph to lay; one with none lays nothing and says nothing, as round any
/// outline. Empty where it gives all it owes.
fn not_refused(what: &str, lays: bool, placed: &PlacedRun, trace: &Trace) -> Option<String> {
    let says = placed.truncated || trace.ring == Some(RingLoss::TooLong);
    (!placed.glyphs.is_empty() || says != lays || (lays && trace.ring != Some(RingLoss::TooLong))).then(
        || {
            format!(
                "{what}: {} glyphs, truncated {}, reported {:?}",
                placed.glyphs.len(),
                placed.truncated,
                trace.ring
            )
        },
    )
}

/// What a placement along a path within the limit owes, and does not
/// give: something laid where it `lays`, has a glyph to lay, and nothing
/// where it has none; every position finite; and no word of a path too
/// long. Empty where it gives all three.
fn not_laid(what: &str, lays: bool, placed: &PlacedRun, trace: &Trace) -> Option<String> {
    let finite = placed.glyphs.iter().all(|g| g.pos.is_finite());
    (placed.glyphs.is_empty() == lays || !finite || trace.ring == Some(RingLoss::TooLong)).then(|| {
        format!(
            "{what}: {} glyphs, every position finite {finite}, reported {:?}",
            placed.glyphs.len(),
            trace.ring
        )
    })
}

/// Fail with every fault found, one a line.
fn assert_none(faults: &[String]) {
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

/// A rounded rectangle's perimeter from its own numbers: its sides, less
/// the squares its corners cut, plus their arcs.
fn rectangle_round(w: f64, h: f64, r: f64) -> f64 {
    2.0 * (w + h) - 8.0 * r + std::f64::consts::TAU * r
}

/// An ellipse's perimeter from its radii: the polygon through 65,536 of
/// its points, each found from the tangent of its half angle — within a
/// part in 10⁸ of the ellipse, from under it.
fn ellipse_round(a: f64, b: f64) -> f64 {
    let n = 65_536;
    let at = |k: usize| {
        let u = (std::f64::consts::PI * k as f64 / n as f64).tan();
        // The half turn's point is where the tangent is not a number.
        if k * 2 == n {
            DVec2::new(-a, 0.0)
        } else {
            DVec2::new(a * (1.0 - u * u) / (1.0 + u * u), b * 2.0 * u / (1.0 + u * u))
        }
    };
    (0..n).map(|k| (at(k + 1) - at(k)).length()).sum()
}

/// The outlines the limit is held on, each `scale` times as long round as
/// the limit, to the float: a box, a thin box, a rounded box, a pill, an
/// ellipse and a circle, at the origin's corner and centered on it. Each
/// with its perimeter, from its own numbers.
fn outlines_round(scale: f64) -> Vec<(String, Outline, f64)> {
    let mut outlines = Vec::new();
    // Each shape by its height and its radius as shares of its width, and
    // its perimeter a width.
    let shapes: [(&str, f64, Option<f64>); 6] = [
        ("a box", 0.6, Some(0.0)),
        ("a thin box", 1e-18, Some(0.0)),
        ("a rounded box", 0.6, Some(0.1)),
        ("a pill", 1.0, Some(0.25)),
        ("an ellipse", 0.6, None),
        ("a circle", 1.0, None),
    ];
    for (name, tall, radius) in shapes {
        let unit = match radius {
            Some(r) => rectangle_round(1.0, tall, r),
            None => ellipse_round(0.5, 0.5 * tall),
        };
        // The width, as the `f32` the outline holds, and what it is round.
        let w = (scale * LIMIT / unit) as f32;
        let h = (f64::from(w) * tall) as f32;
        let (wide, high) = (f64::from(w), f64::from(h));
        for centered in [false, true] {
            let size = Vec2::new(w, h);
            let origin = if centered { -0.5 * size } else { Vec2::ZERO };
            let rect = Rect::new(origin, size);
            let (outline, round) = match radius {
                Some(r) => {
                    let r = (wide * r) as f32;
                    (
                        Outline::RoundedRect { rect, radius: r },
                        rectangle_round(wide, high, f64::from(r)),
                    )
                }
                None => (Outline::Ellipse { rect }, ellipse_round(0.5 * wide, 0.5 * high)),
            };
            outlines.push((
                format!(
                    "{name}{} {round:e} round",
                    if centered { " about the origin" } else { "" }
                ),
                outline,
                round,
            ));
        }
    }
    outlines
}

#[test]
fn test_flow_limit_is_half_the_first_float_past_the_root_of_the_largest_f32() {
    do_flow_limit_is_half_the_first_float_past_the_root_of_the_largest_f32();
}

/// The limit is the number its derivation gives (`MAX_PATH_LENGTH`'s
/// doc): 2⁶³, half of 2⁶⁴, the first float past the root of the largest
/// `f32`. So the distance between two points of a path that long, each a
/// quarter of it off the path, has an `f32` square, and one float further
/// than twice the limit has none; and two places along it sum to a
/// number.
///
/// Controls: the constant at twice what it is, and at half, each red
/// ("the limit is 2⁶³"): the rest is that number's arithmetic.
pub fn do_flow_limit_is_half_the_first_float_past_the_root_of_the_largest_f32() {
    assert_eq!(MAX_PATH_LENGTH, 2.0_f32.powi(63), "the limit is 2⁶³");
    let root = f32::MAX.sqrt();
    assert_eq!(
        root.next_up(),
        2.0 * MAX_PATH_LENGTH,
        "the limit is half of 2⁶⁴, the first float past the root of the largest f32"
    );
    assert!((root * root).is_finite() && (root.next_up() * root.next_up()).is_infinite());
    let apart = MAX_PATH_LENGTH + 2.0 * (0.25 * MAX_PATH_LENGTH);
    assert!(
        (apart * apart).is_finite() && Vec2::new(apart, 0.0).length() == apart,
        "two points a path's length apart, each a quarter of it off the path: a distance with a square"
    );
    assert!(
        (MAX_PATH_LENGTH + MAX_PATH_LENGTH).is_finite(),
        "two places along it sum to a place"
    );
}

#[test]
fn test_flow_limit_a_square_at_the_limit_is_laid_and_a_float_past_it_is_not() {
    do_flow_limit_a_square_at_the_limit_is_laid_and_a_float_past_it_is_not();
}

/// The limit is held to the float, and a path as long as it is laid: a
/// sharp square 2⁶¹ a side is exactly 2⁶³ round, however it is walked —
/// from the middle of its top, as a ring; from its top's start, as a
/// directional ring; from the middle of its left side, as a frame's
/// halves — and every kind of placement lays round it. A side one float
/// longer makes it 2⁴⁰ too long, and every kind lays nothing and says the
/// path is too long.
///
/// Controls: the limit held with `>=`, red on the square at the limit;
/// held one float up (`> limit.next_up()`), red on the square past it;
/// not held at all, red on the square past it.
pub fn do_flow_limit_a_square_at_the_limit_is_laid_and_a_float_past_it_is_not() {
    let side = 0.25 * MAX_PATH_LENGTH;
    assert_eq!(
        4.0 * f64::from(side),
        LIMIT,
        "precondition: four sides of 2⁶¹ are the limit exactly"
    );
    let square = |side: f32| Outline::RoundedRect {
        rect: Rect::new(Vec2::ZERO, Vec2::splat(side)),
        radius: 0.0,
    };
    let mut found = Vec::new();
    for outward in [false, true] {
        for (name, text, fit, layout, lays) in closed_kinds(0.0) {
            let (placed, trace) = lay_closed(&square(side), text, &fit, &layout, outward, 0.5);
            found.extend(not_laid(
                &format!("{name} round the square at the limit"),
                lays,
                &placed,
                &trace,
            ));
            let (placed, trace) = lay_closed(&square(side.next_up()), text, &fit, &layout, outward, 0.5);
            found.extend(not_refused(
                &format!("{name} round the square a float past the limit"),
                lays,
                &placed,
                &trace,
            ));
        }
    }
    assert_none(&found);
}

#[test]
fn test_flow_limit_an_outline_past_the_limit_lays_nothing_and_says_so() {
    do_flow_limit_an_outline_past_the_limit_lays_nothing_and_says_so();
}

/// Round an outline longer than the limit nothing is laid, and every kind
/// of placement says why by the one name: boxes, thin and rounded, a
/// pill, ellipses and circles, a thousandth past the limit, twice it,
/// 10³⁰ round, three quarters of the largest `f32` and nearly twice the
/// largest `f32`, at no spacing and spaced 10³⁷, inward and outward. Among
/// them are the four faults of #363, each of which this was: a ring round a
/// box 1.5 × 10³⁸ by 10³⁸ (laid at `NaN`, or part of the way round with
/// nothing said); a directional ring round a rounded box 8 × 10³⁷ a side
/// (three runs in four at `NaN`); a ring of `─` spaced 10²⁵ round a box
/// that wide (four glyphs where its spacing leaves room for one); a frame
/// round an ellipse 10²² across (half its size from where it lies).
/// Precondition: each outline is longer than the limit, by its own
/// numbers.
///
/// Controls: the limit not held, red on every row that was a fault
/// (positions that are not finite; a frame that is laid); a frame's
/// report left out (`place_halves`), red on every frame; a ring's
/// (`place`), red on every ring, a directional ring's among them: one
/// with no run to lay is laid as a ring of its spans alone, and says so
/// there.
pub fn do_flow_limit_an_outline_past_the_limit_lays_nothing_and_says_so() {
    let mut outlines: Vec<(String, Outline, f64)> = Vec::new();
    let most = f64::from(f32::MAX);
    for scale in [
        1.001,
        2.0,
        1.0e30 / LIMIT,
        0.75 * most / LIMIT,
        1.9 * most / LIMIT,
    ] {
        outlines.extend(outlines_round(scale));
    }
    let boxed = |w: f32, h: f32, r: f32| Outline::RoundedRect {
        rect: Rect::new(Vec2::ZERO, Vec2::new(w, h)),
        radius: r,
    };
    for (name, outline, round) in [
        (
            "the box of the first fault",
            boxed(1.5e38, 1.0e38, 0.0),
            rectangle_round(1.5e38, 1.0e38, 0.0),
        ),
        (
            "the rounded box of the second",
            boxed(8.0e37, 8.0e37, 2.0e37),
            rectangle_round(8.0e37, 8.0e37, 2.0e37),
        ),
        (
            "the box of the third",
            boxed(1.0e25, 0.6e25, 0.0),
            rectangle_round(1.0e25, 0.6e25, 0.0),
        ),
        (
            "the ellipse of the fourth",
            Outline::Ellipse {
                rect: Rect::new(Vec2::ZERO, Vec2::new(1.0e22, 0.6e22)),
            },
            ellipse_round(0.5e22, 0.3e22),
        ),
    ] {
        outlines.push((name.to_owned(), outline, round));
    }
    let mut found = Vec::new();
    let mut laid = 0;
    for (name, outline, round) in &outlines {
        assert!(
            *round > LIMIT * 1.0005,
            "precondition: {name} is longer than the limit"
        );
        for spacing in [0.0_f32, 1.0e37] {
            for outward in [false, true] {
                for (kind, text, fit, layout, lays) in closed_kinds(spacing) {
                    let (placed, trace) = lay_closed(outline, text, &fit, &layout, outward, 0.37);
                    found.extend(not_refused(
                        &format!("{kind} round {name}, outward {outward}"),
                        lays,
                        &placed,
                        &trace,
                    ));
                    laid += 1;
                }
            }
        }
    }
    assert_eq!(laid, (5 * 12 + 4) * 2 * 2 * 16, "every placement is laid");
    assert_none(&found);
}

#[test]
fn test_flow_limit_an_outline_just_within_the_limit_is_laid() {
    do_flow_limit_an_outline_just_within_the_limit_is_laid();
}

/// Round an outline a thousandth shorter than the limit every kind of
/// placement lays something, at finite positions, with no word of a path
/// too long: the same boxes, pill, ellipses and circles as past it, at no
/// spacing and spaced a tenth of the limit. (Not the thin box: one
/// thinner than a line holds no ink of a vertical text, however long it
/// is.) Precondition: each outline is shorter than the limit, by its own
/// numbers.
///
/// Controls: the limit held at half what it is, red on every outline; an
/// ellipse's table counted toward the limit as eight of its longer radius,
/// more than its perimeter, red on the ellipses.
pub fn do_flow_limit_an_outline_just_within_the_limit_is_laid() {
    let mut found = Vec::new();
    for (name, outline, round) in outlines_round(0.999) {
        if name.contains("thin") {
            continue;
        }
        assert!(
            round < LIMIT * 0.9995 && round > LIMIT * 0.99,
            "precondition: {name} is just shorter than the limit"
        );
        for spacing in [0.0_f32, 0.1 * MAX_PATH_LENGTH] {
            for outward in [false, true] {
                for (kind, text, fit, layout, lays) in closed_kinds(spacing) {
                    let (placed, trace) = lay_closed(&outline, text, &fit, &layout, outward, 0.37);
                    found.extend(not_laid(
                        &format!("{kind} round {name}, outward {outward}"),
                        lays,
                        &placed,
                        &trace,
                    ));
                }
            }
        }
    }
    assert_none(&found);
}

/// A Bézier's length from its control points, in `f64`: the polyline
/// through 4,096 of its points, each by de Casteljau's construction on
/// the control points' offsets from the first. No longer than the curve.
fn polyline(points: &[Vec2]) -> f64 {
    let offsets: Vec<DVec2> = points
        .iter()
        .map(|p| p.as_dvec2() - points[0].as_dvec2())
        .collect();
    let at = |t: f64| {
        let mut row = offsets.clone();
        while row.len() > 1 {
            row = row.windows(2).map(|pair| pair[0].lerp(pair[1], t)).collect();
        }
        row[0]
    };
    let n = 4096;
    (0..n)
        .map(|k| (at((k + 1) as f64 / n as f64) - at(k as f64 / n as f64)).length())
        .sum()
}

/// A curve's control polygon, in `f64`: no shorter than the curve.
fn polygon(points: &[Vec2]) -> f64 {
    points
        .windows(2)
        .map(|pair| (pair[1].as_dvec2() - pair[0].as_dvec2()).length())
        .sum()
}

/// The control points of a curve, in order.
fn controls(curve: &Curve) -> Vec<Vec2> {
    match *curve {
        Curve::Line(a, b) => vec![a, b],
        Curve::Quadratic(a, c, b) => vec![a, c, b],
        Curve::Cubic(a, c1, c2, b) => vec![a, c1, c2, b],
    }
}

#[test]
fn test_flow_limit_an_open_path_past_the_limit_lays_nothing_and_says_so() {
    do_flow_limit_an_open_path_past_the_limit_lays_nothing_and_says_so();
}

/// Along an open path longer than the limit nothing is laid, and every
/// kind of placement says why: text and an ornament in every fit, and a
/// pitched pattern. A line a float longer than the limit, and one as long
/// as an `f32` reaches; a line whose ends are too far apart for its
/// length to be an `f32`, which was a path of no length with nothing
/// said; two lines and three that together pass the limit, and the
/// largest `f32`; a quadratic and a cubic, each longer than the limit by
/// its own polyline; and a cubic that doubles back, whose polyline is
/// shorter than the limit and whose control polygon is longer — it is
/// counted by its polygon, which no chord of it exceeds. And along each a
/// little within the limit, something is laid. Precondition: each path is
/// as long as it says, by its own numbers.
///
/// Controls: the limit not held, red (positions that are not finite
/// along the chains past the largest `f32`; glyphs laid along the rest);
/// a Bézier counted by its table and not its polygon, red on the cubic
/// that doubles back; a stretch too long for an `f32` left uncounted, red
/// on the line whose ends are too far apart; `place_pitched`'s report
/// left out, red on every pitched pattern.
pub fn do_flow_limit_an_open_path_past_the_limit_lays_nothing_and_says_so() {
    let l = MAX_PATH_LENGTH;
    let line = |from: Vec2, to: Vec2| Curve::Line(from, to);
    // Each path, with how long it is at the least and at the most.
    let measured = |path: &[Curve]| -> (f64, f64) {
        path.iter().fold((0.0, 0.0), |(least, most), curve| {
            let points = controls(curve);
            (least + polyline(&points), most + polygon(&points))
        })
    };
    let back = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(0.55 * l, 0.0),
        Vec2::new(0.55 * l, 0.001 * l),
        Vec2::new(0.0, 0.001 * l),
    );
    let past: Vec<(&str, Vec<Curve>)> = vec![
        (
            "a line a float past the limit",
            vec![line(Vec2::ZERO, Vec2::new(l.next_up(), 0.0))],
        ),
        (
            "a slant twice the limit",
            vec![line(Vec2::ZERO, Vec2::new(1.2 * l, 1.6 * l))],
        ),
        (
            "a line as long as an f32 reaches",
            vec![line(Vec2::new(-1.7e38, 0.0), Vec2::new(1.7e38, 0.0))],
        ),
        (
            "a line too long for an f32",
            vec![line(Vec2::new(-3.0e38, 0.0), Vec2::new(3.0e38, 0.0))],
        ),
        (
            "two lines of 0.6 of the limit",
            vec![
                line(Vec2::ZERO, Vec2::new(0.6 * l, 0.0)),
                line(Vec2::new(0.6 * l, 0.0), Vec2::new(0.6 * l, 0.6 * l)),
            ],
        ),
        (
            "three lines past the largest f32",
            vec![
                line(Vec2::new(-1.5e38, -1.5e38), Vec2::new(1.5e38, -1.5e38)),
                line(Vec2::new(1.5e38, -1.5e38), Vec2::new(1.5e38, 1.5e38)),
                line(Vec2::new(1.5e38, 1.5e38), Vec2::new(-1.5e38, 1.5e38)),
            ],
        ),
        (
            "a quadratic",
            vec![Curve::Quadratic(
                Vec2::ZERO,
                Vec2::new(l, 2.0 * l),
                Vec2::new(2.0 * l, 0.0),
            )],
        ),
        (
            "a cubic",
            vec![Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(0.5 * l, l),
                Vec2::new(l, -l),
                Vec2::new(1.5 * l, 0.0),
            )],
        ),
        ("a cubic that doubles back", vec![back]),
    ];
    let (least, most) = measured(&[back]);
    assert!(
        least < LIMIT * 0.9 && most > LIMIT * 1.05,
        "precondition: the cubic that doubles back is shorter than the limit, {least:e}, and its control polygon longer, {most:e}"
    );
    let mut found = Vec::new();
    for (name, path) in &past {
        let (least, most) = measured(path);
        assert!(
            most > LIMIT && (least > LIMIT || *name == "a cubic that doubles back"),
            "precondition: {name} is longer than the limit ({least:e} to {most:e})"
        );
        for spacing in [0.0_f32, 1.0e37] {
            for (kind, lays, placed, trace) in lay_open_kinds(path, spacing) {
                found.extend(not_refused(
                    &format!("{kind} along {name}"),
                    lays,
                    &placed,
                    &trace,
                ));
            }
        }
    }
    let within: Vec<(&str, Vec<Curve>)> = vec![
        (
            "a line as long as the limit",
            vec![line(Vec2::ZERO, Vec2::new(l, 0.0))],
        ),
        (
            "two lines of 0.49 of the limit",
            vec![
                line(Vec2::ZERO, Vec2::new(0.49 * l, 0.0)),
                line(Vec2::new(0.49 * l, 0.0), Vec2::new(0.49 * l, 0.49 * l)),
            ],
        ),
        (
            "a cubic whose control polygon is 0.99 of the limit",
            vec![Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(0.33 * l, 0.0),
                Vec2::new(0.33 * l, 0.33 * l),
                Vec2::new(0.66 * l, 0.33 * l),
            )],
        ),
    ];
    for (name, path) in &within {
        let (_, most) = measured(path);
        assert!(
            most <= LIMIT,
            "precondition: {name} is no longer than the limit ({most:e})"
        );
        for (kind, lays, placed, trace) in lay_open_kinds(path, 0.0) {
            found.extend(not_laid(&format!("{kind} along {name}"), lays, &placed, &trace));
        }
    }
    assert_none(&found);
}
