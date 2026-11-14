// SPDX-License-Identifier: MPL-2.0

//! Directional rings (#324, `flow::place::directional`): a `repeat` ring
//! laying its spans where the outline runs level, its vertical text where
//! it runs steep and its corners where it turns as a box does. Every
//! expectation is computed from the outline's geometry and the glyphs'
//! own inks, never through the placement's own runs (TEST_CONVENTIONS
//! §T13, shape 6): which way the outline runs at a glyph is read off the
//! rectangle or the ellipse itself.

use glam::Vec2;

use crate::document::element::DirectionalGlyphs;
use crate::document::values::{Direction, Edge, FitMode, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::place::{place_on_closed_traced, Trace};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{
    offset_keeping_corners, Ink, Outline, PlacedRun, Rect, RingLayout, ShapedRun, FILLER_SPAN,
};

/// The size the rings are laid at.
const POINTS: f32 = 16.0;

/// The light box-drawing set: horizontal, vertical, and the corners
/// clockwise from the top left.
const LIGHT: (&str, &str, [&str; 4]) = ("─", "│", ["┌", "┐", "┘", "└"]);

/// What a placed glyph draws.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Part {
    /// The ring's spans: a level run's.
    Level,
    /// Its vertical text.
    Steep,
    /// A corner, `0` the top left, clockwise.
    Corner(usize),
}

/// One placed glyph: what it draws and its ink on the canvas.
#[derive(Copy, Clone, Debug)]
struct Laid {
    part: Part,
    ink: Rect,
}

/// The directional glyphs of a set.
fn glyphs((_, v, corners): (&str, &str, [&str; 4]), with_corners: bool) -> DirectionalGlyphs {
    DirectionalGlyphs {
        vertical: Some(v.to_owned()),
        corners: with_corners.then(|| corners.map(str::to_owned)),
    }
}

/// `set`'s ring laid round `path`, its trace, and each glyph by part and
/// ink: a filler glyph's part read from the bytes the vertical text and
/// the corners occupy in the line they were shaped into, a space apart.
fn lay(set: (&str, &str, [&str; 4]), with_corners: bool, path: &Outline) -> (Vec<Laid>, Trace, ShapedRun) {
    lay_at(set, with_corners, path, POINTS)
}

/// [`lay`], its text at `points`.
fn lay_at(
    set: (&str, &str, [&str; 4]),
    with_corners: bool,
    path: &Outline,
    points: f32,
) -> (Vec<Laid>, Trace, ShapedRun) {
    let text = style(points);
    let directional = glyphs(set, with_corners);
    let layout = RingLayout {
        directional: directional.clone(),
        ..RingLayout::whole()
    };
    let mut trace = Trace::default();
    let (run, placed) = with_fonts(|fs| {
        let input = real_run(set.0, &text, fs);
        let (replaced, placed) = place_on_closed_traced(
            &input,
            path,
            StartPoint {
                edge: Edge::Top,
                at: 0.0,
            },
            Direction::Clockwise,
            &FitMode::Repeat,
            &layout,
            true,
            &text,
            fs,
            Some(&mut trace),
        );
        (replaced.unwrap_or(input), placed)
    });
    (parts(&run, &placed, set, with_corners), trace, run)
}

/// A ring of `spans` laid round `path` with `layout`, at [`POINTS`]: the
/// run the placement refers to, and the placement.
fn lay_layout(spans: &str, layout: &RingLayout, path: &Outline) -> (ShapedRun, PlacedRun) {
    let text = style(POINTS);
    with_fonts(|fs| {
        let input = real_run(spans, &text, fs);
        let (replaced, placed) = place_on_closed_traced(
            &input,
            path,
            StartPoint {
                edge: Edge::Top,
                at: 0.0,
            },
            Direction::Clockwise,
            &FitMode::Repeat,
            layout,
            true,
            &text,
            fs,
            None,
        );
        (replaced.unwrap_or(input), placed)
    })
}

/// Flat ellipses, grown by 6 as a ring's inset grows them: wide and tall,
/// so flat that round their tips they turn through 45° within a line.
fn flat_ellipses() -> [(&'static str, Outline); 3] {
    [(300.0, 60.0), (600.0, 24.0), (60.0, 300.0)].map(|(w, h)| {
        let rect = Rect::new(Vec2::new(-w * 0.5, -h * 0.5), Vec2::new(w, h));
        let name = if w > h {
            "a wide flat ellipse"
        } else {
            "a tall flat ellipse"
        };
        (name, offset_keeping_corners(&Outline::Ellipse { rect }, 6.0))
    })
}

/// How far the ink farthest off `path` lies from it.
fn farthest(path: &Outline, laid: &[Laid]) -> f32 {
    let arc = ArcPath::closed(
        path,
        StartPoint {
            edge: Edge::Top,
            at: 0.0,
        },
        Direction::Clockwise,
        1.0,
    );
    let points: Vec<Vec2> = (0..4096)
        .map(|k| arc.at(arc.length() * k as f32 / 4096.0).0)
        .collect();
    laid.iter()
        .map(|l| {
            points
                .iter()
                .map(|&p| (l.ink.origin - p).max(p - l.ink.max()).max(Vec2::ZERO).length())
                .fold(f32::INFINITY, f32::min)
        })
        .fold(0.0, f32::max)
}

/// Each placed glyph's part and ink.
fn parts(run: &ShapedRun, placed: &PlacedRun, set: (&str, &str, [&str; 4]), with_corners: bool) -> Vec<Laid> {
    // The vertical text, then the corners, a space apart.
    let mut starts = vec![(0, Part::Steep)];
    let mut at = set.1.len() + 1;
    if with_corners {
        for (i, corner) in set.2.iter().enumerate() {
            starts.push((at, Part::Corner(i)));
            at += corner.len() + 1;
        }
    }
    let baseline = run.baseline();
    placed
        .glyphs
        .iter()
        .map(|g| {
            let metrics = &run.glyphs[g.glyph as usize];
            let part = if metrics.span == FILLER_SPAN {
                starts
                    .iter()
                    .rev()
                    .find(|(start, _)| *start <= metrics.bytes.start)
                    .map(|&(_, part)| part)
                    .expect("a filler glyph is the vertical text's or a corner's")
            } else {
                Part::Level
            };
            let ink: Ink = metrics.ink.expect("a box-drawing glyph has ink");
            let origin = g.pos + Vec2::new(0.0, baseline);
            Laid {
                part,
                ink: Rect::new(origin + ink.min, ink.max - ink.min),
            }
        })
        .collect()
}

/// A sharp rectangle, a rounded one and an ellipse, 300 by 140, grown by
/// 6 as a ring's inset grows them.
fn outlines() -> [(&'static str, Outline); 3] {
    let rect = Rect::new(Vec2::new(-150.0, -70.0), Vec2::new(300.0, 140.0));
    [
        ("rectangle", Outline::RoundedRect { rect, radius: 0.0 }),
        ("rounded", Outline::RoundedRect { rect, radius: 28.0 }),
        ("ellipse", Outline::Ellipse { rect }),
    ]
    .map(|(name, o)| (name, offset_keeping_corners(&o, 6.0)))
}

#[test]
fn test_flow_directional_lays_a_box() {
    do_flow_directional_lays_a_box();
}

/// A ring of light box-drawing glyphs round a sharp rectangle reads as a
/// box, and so does one round a rectangle rounded no wider than a glyph:
/// every glyph of its spans along the top or the bottom, every vertical
/// one down a side, each corner once at its own corner, and each corner's
/// strokes run on in its neighbors' — its ink's outer edges on the level
/// glyphs' row and on the vertical glyphs' column.
pub fn do_flow_directional_lays_a_box() {
    for set in [LIGHT, HEAVY, ("─", "│", ["╭", "╮", "╯", "╰"])] {
        lays_a_box(set);
    }
}

/// [`do_flow_directional_lays_a_box`] in `set`.
fn lays_a_box(set: (&str, &str, [&str; 4])) {
    let rect = Rect::new(Vec2::new(-150.0, -70.0), Vec2::new(300.0, 140.0));
    // Sharp, and rounded at radii about a corner's reach, where the row
    // and the column a corner joins are read on its arc if not on its
    // sides.
    // At two sizes and insets, the second the owner's fixture's, where a
    // side's last vertical glyph once ran on past its corner.
    for (radius, points, inset) in [
        (0.0, POINTS, 6.0),
        (10.0, POINTS, 6.0),
        (14.0, POINTS, 6.0),
        (0.0, 16.8, 8.0),
        (14.0, 16.8, 8.0),
    ] {
        let path = offset_keeping_corners(&Outline::RoundedRect { rect, radius }, inset);
        let (laid, _, _) = lay_at(set, true, &path, points);
        let of = |part: Part| laid.iter().filter(move |l| l.part == part);
        assert!(
            of(Part::Level).count() >= 40,
            "precondition: the top and the bottom are laid (radius {radius})"
        );
        assert!(
            of(Part::Steep).count() >= 8,
            "precondition: the sides are laid (radius {radius})"
        );
        let grown = path.rect();
        let center = grown.center();
        for l in of(Part::Level) {
            assert!(
                (l.ink.center().y - center.y).abs() > grown.size.y * 0.5,
                "a level glyph off the top and the bottom (radius {radius}): {l:?}"
            );
        }
        // Every vertical glyph down a side, between the rows: none past
        // the top or the bottom row's outer edge.
        let (top, bottom) = (level_row(&laid, true), level_row(&laid, false));
        for l in of(Part::Steep) {
            assert!(
                (l.ink.center().x - center.x).abs() > grown.size.x * 0.5,
                "a vertical glyph off the sides (radius {radius}): {l:?}"
            );
            assert!(
                l.ink.origin.y >= top - 0.01 && l.ink.max().y <= bottom + 0.01,
                "a vertical glyph past the rows {top}..{bottom} (radius {radius}): {l:?}"
            );
        }
        for (i, sign) in [
            (0, Vec2::new(-1.0, -1.0)),
            (1, Vec2::new(1.0, -1.0)),
            (2, Vec2::new(1.0, 1.0)),
            (3, Vec2::new(-1.0, 1.0)),
        ] {
            let corners: Vec<&Laid> = of(Part::Corner(i)).collect();
            assert_eq!(corners.len(), 1, "corner {i} laid once (radius {radius})");
            let ink = corners[0].ink;
            let d = ink.center() - center;
            assert!(
                d.x * sign.x > 0.0 && d.y * sign.y > 0.0,
                "corner {i} at its own corner (radius {radius}): {ink:?}"
            );
            let (outer, row_outer) = if sign.y < 0.0 {
                (ink.origin.y, level_row(&laid, true))
            } else {
                (ink.max().y, level_row(&laid, false))
            };
            assert!(
                (outer - row_outer).abs() < 0.01,
                "corner {i}'s stroke on the row (radius {radius}): {outer} vs {row_outer}"
            );
            let (side, column_outer) = if sign.x < 0.0 {
                (ink.origin.x, steep_column(&laid, true))
            } else {
                (ink.max().x, steep_column(&laid, false))
            };
            assert!(
                (side - column_outer).abs() < 0.01,
                "corner {i}'s stroke on the column (radius {radius}): {side} vs {column_outer}"
            );
        }
    }
}

/// The outer edge of the level glyphs' inks along the top (or bottom).
fn level_row(laid: &[Laid], top: bool) -> f32 {
    let ys =
        laid.iter()
            .filter(|l| l.part == Part::Level)
            .map(|l| if top { l.ink.origin.y } else { l.ink.max().y });
    if top {
        ys.fold(f32::INFINITY, f32::min)
    } else {
        ys.fold(f32::NEG_INFINITY, f32::max)
    }
}

/// The outer edge of the vertical glyphs' inks down the left (or right).
fn steep_column(laid: &[Laid], left: bool) -> f32 {
    let xs =
        laid.iter()
            .filter(|l| l.part == Part::Steep)
            .map(|l| if left { l.ink.origin.x } else { l.ink.max().x });
    if left {
        xs.fold(f32::INFINITY, f32::min)
    } else {
        xs.fold(f32::NEG_INFINITY, f32::max)
    }
}

/// The heavy box-drawing set.
const HEAVY: (&str, &str, [&str; 4]) = ("━", "┃", ["┏", "┓", "┛", "┗"]);

/// Whether the outline runs level — within 45° of horizontal — at the
/// point of it nearest `p`, read off its own geometry: a rounded
/// rectangle's sides and corner arcs, an ellipse's normal there. Also how
/// near 45° it runs there, `|n.y| − |n.x|` of the normal.
fn level_at(path: &Outline, p: Vec2) -> (bool, f32) {
    let rect = path.rect();
    let center = rect.center();
    let normal = match *path {
        Outline::RoundedRect { .. } => {
            let r = path.corner_radius();
            let half = rect.size * 0.5 - Vec2::splat(r);
            let d = p - center;
            let clamped = d.clamp(-half, half);
            let n = d - clamped;
            if n.length() > 1e-3 {
                n.normalize()
            } else {
                // On the straight part: the side it is nearer.
                let gap = rect.size * 0.5 - d.abs();
                if gap.y < gap.x {
                    Vec2::new(0.0, d.y.signum())
                } else {
                    Vec2::new(d.x.signum(), 0.0)
                }
            }
        }
        Outline::Ellipse { .. } => {
            let radii = rect.size * 0.5;
            let d = (p - center) / radii;
            (d / radii).normalize()
        }
    };
    (normal.y.abs() >= normal.x.abs(), normal.y.abs() - normal.x.abs())
}

/// The runs of a placement: its glyphs split wherever the part changes,
/// in the order laid, each with its part.
fn runs_of(laid: &[Laid]) -> Vec<(Part, Vec<Rect>)> {
    let mut runs: Vec<(Part, Vec<Rect>)> = Vec::new();
    for l in laid {
        match runs.last_mut() {
            Some((part, inks)) if *part == l.part => inks.push(l.ink),
            _ => runs.push((l.part, vec![l.ink])),
        }
    }
    runs
}

/// The gap from ink `a` to ink `b` along `axis`, the way a run reads:
/// signed, below 0 by how much they overlap.
fn axis_gap(axis: Vec2, a: &Rect, b: &Rect) -> f32 {
    let span = |r: &Rect| {
        let (p, q) = (axis.dot(r.origin), axis.dot(r.max()));
        (p.min(q), p.max(q))
    };
    span(b).0 - span(a).1
}

/// The way a run of `part` reads at the ink `first` of it, round
/// `center`: rightward over the top, down the right side, leftward along
/// the bottom, up the left side.
fn axis_of(part: Part, first: &Rect, center: Vec2) -> Vec2 {
    let d = first.center() - center;
    match part {
        Part::Level if d.y < 0.0 => Vec2::X,
        Part::Level => Vec2::NEG_X,
        _ if d.x > 0.0 => Vec2::Y,
        _ => Vec2::NEG_Y,
    }
}

/// The spans' own gap as a line sets two of the glyph: the font's, below
/// 0 where the face overhangs its advance.
fn natural_gap(run: &ShapedRun) -> f32 {
    let h = run
        .glyphs
        .iter()
        .find(|g| g.span != FILLER_SPAN)
        .expect("a spans glyph");
    let ink = h.ink.expect("ink");
    (h.advance - ink.max.x) + ink.min.x
}

#[test]
fn test_flow_directional_chooses_by_the_way_the_outline_runs() {
    do_flow_directional_chooses_by_the_way_the_outline_runs();
}

/// Each glyph is the one its place calls for: the spans' where the
/// outline, read off its own geometry, runs within 45° of level, the
/// vertical text's where it runs steeper — round a rectangle, a rounded
/// one and an ellipse, light and heavy.
pub fn do_flow_directional_chooses_by_the_way_the_outline_runs() {
    for set in [LIGHT, HEAVY] {
        for (name, path) in outlines() {
            let (laid, _, _) = lay(set, true, &path);
            let mut checked = 0;
            for l in &laid {
                let (level, margin) = level_at(&path, l.ink.center());
                // Within a few degrees of 45° the glyph's own push off the
                // outline decides which side of it its center lands.
                if margin.abs() < 0.2 || matches!(l.part, Part::Corner(_)) {
                    continue;
                }
                checked += 1;
                match l.part {
                    Part::Level => assert!(level, "{name}: a level glyph where it runs steep: {l:?}"),
                    Part::Steep => assert!(!level, "{name}: a vertical glyph where it runs level: {l:?}"),
                    Part::Corner(_) => {}
                }
            }
            assert!(checked > 30, "precondition: {name}'s ring is laid ({checked})");
        }
    }
}

#[test]
fn test_flow_directional_changes_its_choice_four_times_round() {
    do_flow_directional_changes_its_choice_four_times_round();
}

/// The choice cannot flicker: round each outline it changes four times,
/// at its four turns, and nowhere else.
pub fn do_flow_directional_changes_its_choice_four_times_round() {
    for set in [LIGHT, HEAVY] {
        for (name, path) in outlines() {
            let (laid, _, _) = lay(set, false, &path);
            let kinds: Vec<Part> = laid.iter().map(|l| l.part).collect();
            let changes = (0..kinds.len())
                .filter(|&i| kinds[i] != kinds[(i + 1) % kinds.len()])
                .count();
            assert_eq!(
                changes, 4,
                "{name}: the choice changes at the four turns and nowhere else"
            );
        }
    }
}

/// The line height the rings are laid at: a span of `POINTS` points.
const LINE: f32 = POINTS * crate::flow::LINE_HEIGHT_FACTOR;

#[test]
fn test_flow_directional_hysteresis_holds_a_curve_to_three_lines_and_a_box_to_none() {
    do_flow_directional_hysteresis_holds_a_curve_to_three_lines_and_a_box_to_none();
}

/// The hysteresis, on both sides of its threshold. A steep run that ends
/// on curves is a run only from three line heights: a rectangle rounded
/// wider than a glyph whose sides, arcs to their 45° points included, are
/// 2.9 line heights lays no vertical glyph there, and one of 3.1 does. A
/// side of a box is a side however short: a sharp rectangle with sides of
/// 2.9 line heights, or of 1.6, keeps its sides' runs and its four
/// corners, so it still reads as a box. A flat ellipse's tips, taller
/// than a line, are a curve's: no runs, its ring along its curve.
pub fn do_flow_directional_hysteresis_holds_a_curve_to_three_lines_and_a_box_to_none() {
    let steep = |laid: &[Laid]| laid.iter().filter(|l| l.part == Part::Steep).count();
    let corners = |laid: &[Laid]| laid.iter().filter(|l| matches!(l.part, Part::Corner(_))).count();
    // A radius of 30: its box's corners lie 12.4 off its turns, more than
    // half a line, so no turn of it is a box's; its steep run is its
    // straight side and a quarter of each arc, `h − 2r + πr/2`.
    let radius = 30.0;
    let arcs = std::f32::consts::FRAC_PI_2 * radius;
    for (lines, holds) in [(2.9, false), (3.1, true)] {
        let height = lines * LINE - arcs + 2.0 * radius;
        let path = Outline::RoundedRect {
            rect: Rect::new(Vec2::new(-150.0, -height * 0.5), Vec2::new(300.0, height)),
            radius,
        };
        let (laid, _, _) = lay(LIGHT, true, &path);
        assert!(
            laid.iter().filter(|l| l.part == Part::Level).count() > 40,
            "precondition: laid ({lines} lines)"
        );
        assert_eq!(corners(&laid), 0, "precondition: a curve's turns are no box's");
        assert_eq!(
            steep(&laid) > 0,
            holds,
            "sides of {lines} line heights round curves lay vertical glyphs: {holds}"
        );
    }
    for lines in [1.6, 2.9] {
        let height = lines * LINE;
        let path = Outline::RoundedRect {
            rect: Rect::new(Vec2::new(-150.0, -height * 0.5), Vec2::new(300.0, height)),
            radius: 0.0,
        };
        let (laid, _, _) = lay(LIGHT, true, &path);
        assert_eq!(
            corners(&laid),
            4,
            "a box of sides {lines} line heights keeps its corners"
        );
        // Its sides are drawn: a corner's vertical stroke, or a vertical
        // glyph, reaches past the middle of each side's half.
        let grown = path.rect();
        for right in [false, true] {
            let x = if right { grown.max().x } else { grown.origin.x };
            let drawn = laid
                .iter()
                .filter(|l| l.part != Part::Level && (l.ink.center().x - x).abs() < LINE)
                .map(|l| l.ink.size.y)
                .sum::<f32>();
            assert!(
                drawn >= grown.size.y * 0.75,
                "the {} side of a box of {lines} line heights is drawn: {drawn} of {}",
                if right { "right" } else { "left" },
                grown.size.y
            );
        }
    }
    for (name, path) in flat_ellipses() {
        let (laid, _, _) = lay(LIGHT, true, &path);
        let wide = path.rect().size.x > path.rect().size.y;
        let across = if wide { Part::Steep } else { Part::Level };
        assert!(laid.len() > 10, "precondition: {name} is laid");
        assert_eq!(corners(&laid), 0, "{name}'s tips are no box's turns");
        assert_eq!(
            laid.iter().filter(|l| l.part == across).count(),
            0,
            "{name}'s tips are no runs of their own"
        );
        let off = farthest(&path, &laid);
        assert!(off <= POINTS, "{name}'s ring lies along it: an ink {off} off");
    }
}

#[test]
fn test_flow_directional_lays_corners_only_where_it_turns_as_a_box() {
    do_flow_directional_lays_corners_only_where_it_turns_as_a_box();
}

/// A corner is laid where the outline turns as a box does — a sharp
/// rectangle's corners, and arcs narrower than a glyph — and not round a
/// rounded rectangle's wider arcs or an ellipse, flat or not, where it
/// would lie off the outline.
pub fn do_flow_directional_lays_corners_only_where_it_turns_as_a_box() {
    for set in [LIGHT, HEAVY] {
        let [rectangle, rounded, ellipse] = outlines();
        let corners = |path: &Outline| {
            lay(set, true, path)
                .0
                .iter()
                .filter(|l| matches!(l.part, Part::Corner(_)))
                .count()
        };
        assert_eq!(
            corners(&rectangle.1),
            4,
            "a sharp rectangle turns as a box at its four corners"
        );
        assert_eq!(
            corners(&rounded.1),
            0,
            "a rounded rectangle's arcs, wider than a glyph, are no box's turn"
        );
        assert_eq!(corners(&ellipse.1), 0, "an ellipse turns round a curve");
        for (name, path) in flat_ellipses() {
            let (laid, _, _) = lay(set, true, &path);
            assert!(laid.len() > 10, "precondition: {name} is laid");
            let off = farthest(&path, &laid);
            assert!(off <= POINTS, "no ink off {name}: {off} off");
            assert_eq!(
                laid.iter().filter(|l| matches!(l.part, Part::Corner(_))).count(),
                0,
                "{name} turns round a curve at its tips"
            );
        }
        let tight = offset_keeping_corners(
            &Outline::RoundedRect {
                rect: Rect::new(Vec2::new(-150.0, -70.0), Vec2::new(300.0, 140.0)),
                radius: 3.0,
            },
            0.0,
        );
        assert_eq!(
            corners(&tight),
            4,
            "corners round arcs narrower than a glyph are a box's"
        );
        // A side too short to keep its two corners apart sets none.
        for (w, h) in [(300.0, 10.0), (4.0, 300.0), (10.0, 10.0)] {
            let path = Outline::RoundedRect {
                rect: Rect::new(Vec2::new(-w * 0.5, -h * 0.5), Vec2::new(w, h)),
                radius: 0.0,
            };
            assert_eq!(corners(&path), 0, "a {w} by {h} box sets no corner over another");
        }
    }
}

#[test]
fn test_flow_directional_spaces_each_run_evenly_and_never_apart() {
    do_flow_directional_spaces_each_run_evenly_and_never_apart();
}

#[test]
fn test_flow_directional_lays_its_vertical_all_round_where_its_spans_draw_nothing() {
    do_flow_directional_lays_its_vertical_all_round_where_its_spans_draw_nothing();
}

/// Spans with nothing to draw lay the vertical text all round: over the
/// top and along the bottom as down the sides, its gaps with the ring's
/// `spacing`. With no vertical text either, the corners alone are laid,
/// one at each of the box's corners, and round a curve nothing.
pub fn do_flow_directional_lays_its_vertical_all_round_where_its_spans_draw_nothing() {
    let (_, path) = &outlines()[0];
    let (laid, _, _) = lay((" ", LIGHT.1, LIGHT.2), false, path);
    assert!(!laid.is_empty(), "precondition: the ring is laid");
    assert!(
        laid.iter().all(|l| l.part == Part::Steep),
        "only the vertical text is laid: {laid:?}"
    );
    let rect = path.rect();
    let (center, size) = (rect.center(), rect.size);
    let over = |l: &&Laid, top: bool| {
        let d = l.ink.center() - center;
        d.x.abs() < 0.4 * size.x
            && (if top {
                d.y < -0.4 * size.y
            } else {
                d.y > 0.4 * size.y
            })
    };
    for top in [true, false] {
        assert!(
            laid.iter().filter(|l| over(l, top)).count() > 10,
            "the vertical text laid along the {}: {laid:?}",
            if top { "top" } else { "bottom" }
        );
    }
    // Its gaps take the ring's spacing.
    let laid_at = |spacing: f32| {
        let layout = RingLayout {
            directional: glyphs((" ", LIGHT.1, LIGHT.2), false),
            spacing,
            ..RingLayout::whole()
        };
        lay_layout(" ", &layout, path).1.glyphs.len()
    };
    let (close, spaced) = (laid_at(0.0), laid_at(10.0));
    assert!(
        spaced * 3 < close * 2,
        "a spacing of 10 sets the vertical text apart: {spaced} glyphs, against {close} at 0"
    );
    // No vertical text: the corners alone.
    let layout = RingLayout {
        directional: DirectionalGlyphs {
            vertical: None,
            corners: Some(LIGHT.2.map(str::to_owned)),
        },
        ..RingLayout::whole()
    };
    let (_, placed) = lay_layout(" ", &layout, path);
    assert_eq!(
        placed.glyphs.len(),
        4,
        "with no text to run beside, its four corners"
    );
    // Round a curve it turns as no box does: nothing.
    for (name, curve) in outlines().into_iter().skip(1) {
        let (_, placed) = lay_layout(" ", &layout, &curve);
        assert_eq!(placed.glyphs.len(), 0, "corners alone round the {name}: none");
    }
    let box_corners = [
        rect.origin,
        Vec2::new(rect.max().x, rect.origin.y),
        rect.max(),
        Vec2::new(rect.origin.x, rect.max().y),
    ];
    let mut at: Vec<usize> = placed
        .glyphs
        .iter()
        .map(|g| {
            (0..4)
                .min_by(|&a, &b| {
                    g.pos
                        .distance(box_corners[a])
                        .total_cmp(&g.pos.distance(box_corners[b]))
                })
                .expect("four corners")
        })
        .collect();
    at.sort_unstable();
    assert_eq!(at, [0, 1, 2, 3], "one corner at each of the box's corners");
    for g in &placed.glyphs {
        let near = box_corners
            .iter()
            .map(|&c| g.pos.distance(c))
            .fold(f32::INFINITY, f32::min);
        assert!(near < 2.0 * LINE, "a corner beside its box's corner: {near} off");
    }
}

/// Each run's gaps, along the way it reads, are one gap, none looser than
/// the font sets two of its spans' glyph, so box-drawing strokes never
/// part — and so are the gaps at its ends: to the corner beside it, round
/// a box, sharp, rounded up to about a glyph or with short sides; to
/// where the outline turns, round a rounded rectangle's wider arcs and
/// an ellipse, where the runs meet with no corner.
pub fn do_flow_directional_spaces_each_run_evenly_and_never_apart() {
    let rect = Rect::new(Vec2::new(-150.0, -70.0), Vec2::new(300.0, 140.0));
    let mut fixtures: Vec<(String, Outline, bool)> = outlines()
        .into_iter()
        .map(|(name, path)| (name.to_owned(), path, name == "rectangle"))
        .collect();
    // Rounded up to and past about a glyph, the first three a box's, and
    // round wider curves whose arms run short.
    for radius in [10.0, 16.0, 20.0, 24.0, 27.0, 29.0, 30.0, 32.0, 35.0, 40.0] {
        let path = offset_keeping_corners(&Outline::RoundedRect { rect, radius }, 6.0);
        fixtures.push((format!("radius {radius}"), path, radius <= 20.0));
    }
    let short = Rect::new(Vec2::new(-150.0, -20.0), Vec2::new(300.0, 40.0));
    let short = offset_keeping_corners(
        &Outline::RoundedRect {
            rect: short,
            radius: 0.0,
        },
        6.0,
    );
    fixtures.push(("short sides".to_owned(), short, true));
    for set in [LIGHT, HEAVY] {
        for (name, path, boxed) in &fixtures {
            let boxed = *boxed;
            let (laid, _, run) = lay(set, true, path);
            let natural = natural_gap(&run);
            let center = path.rect().center();
            let runs = runs_of(&laid);
            assert!(runs.len() >= 4, "precondition: {name} is laid in runs");
            let corners = laid.iter().filter(|l| matches!(l.part, Part::Corner(_))).count();
            assert_eq!(corners == 4, boxed, "precondition: {name} is a box: {boxed}");
            for (k, (part, inks)) in runs.iter().enumerate() {
                if matches!(part, Part::Corner(_)) {
                    continue;
                }
                let axis = axis_of(*part, &inks[0], center);
                let (before_part, before) = &runs[(k + runs.len() - 1) % runs.len()];
                let (after_part, after) = &runs[(k + 1) % runs.len()];
                let (first, last) = (&inks[0], inks.last().expect("a run holds an ink"));
                // Its ends: to a corner beside it, or, with nothing beside
                // it, to where the outline turns, its supports at 45°
                // either side of the way it faces.
                let out = Vec2::new(axis.y, -axis.x);
                let turn = |d: Vec2| Rect::new(path.support(d), Vec2::ZERO);
                let ends = [
                    if matches!(before_part, Part::Corner(_)) {
                        axis_gap(axis, before.last().expect("a corner's ink"), first)
                    } else {
                        axis_gap(axis, &turn(out - axis), first)
                    },
                    if matches!(after_part, Part::Corner(_)) {
                        axis_gap(axis, last, &after[0])
                    } else {
                        axis_gap(axis, last, &turn(out + axis))
                    },
                ];
                let gaps: Vec<f32> = inks.windows(2).map(|w| axis_gap(axis, &w[0], &w[1])).collect();
                let all: Vec<f32> = gaps.iter().chain(&ends).copied().collect();
                let (lo, hi) = all
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &g| {
                        (lo.min(g), hi.max(g))
                    });
                // A box's sides are straight, its places exact; round a
                // bend an ink's place is bisected to well under that.
                let even = 0.01;
                assert!(
                    hi - lo < even,
                    "{name}, {part:?}: a run's gaps, and its ends', are one gap: {gaps:?}, ends {ends:?}"
                );
                assert!(
                    hi <= natural + 0.01,
                    "{name}, {part:?}: no gap looser than the font's ({natural}): {gaps:?}, ends {ends:?}"
                );
            }
        }
    }
}

/// The budget and the cap: they read the frame's test seams and the flow
/// fuzzer's own checks, so they are no bench bodies.
#[cfg(test)]
mod held {
    use super::*;
    use crate::flow::place::place_on_closed_capped;
    use crate::flow::place::ring::RingLoss;
    use crate::flow::place::setup_table_samples;
    use crate::flow::tests::flow_fuzz::InputWork;

    /// A big rectangle, whose ring at [`LIGHT`]'s 16 points is some 420 glyphs
    /// and over ten thousand glyph steps.
    fn big() -> Outline {
        Outline::RoundedRect {
            rect: Rect::new(Vec2::new(-1500.0, -700.0), Vec2::new(3000.0, 1400.0)),
            radius: 0.0,
        }
    }

    /// `set`'s ring round `path` at most `cap` glyphs: the placement, the
    /// trace, and the flow fuzzer's own check of a ring.
    fn capped(
        set: (&str, &str, [&str; 4]),
        with_corners: bool,
        path: &Outline,
        cap: usize,
    ) -> (Vec<Laid>, PlacedRun, Trace, Result<(), String>) {
        let text = style(POINTS);
        let layout = RingLayout {
            directional: glyphs(set, with_corners),
            ..RingLayout::whole()
        };
        let mut trace = Trace::default();
        let (input, replaced, placed) = with_fonts(|fs| {
            let input = real_run(set.0, &text, fs);
            let (replaced, placed) = place_on_closed_capped(
                &input,
                path,
                StartPoint {
                    edge: Edge::Top,
                    at: 0.0,
                },
                Direction::Clockwise,
                &FitMode::Repeat,
                &layout,
                true,
                &text,
                fs,
                cap,
                Some(&mut trace),
            );
            (input.clone(), replaced.unwrap_or(input), placed)
        });
        let tables = setup_table_samples(&input, path, &FitMode::Repeat, &text);
        let checked = InputWork::default().ring(&input, cap, tables, &placed, &trace, "a directional ring");
        (
            parts(&replaced, &placed, set, with_corners),
            placed,
            trace,
            checked,
        )
    }

    #[test]
    fn test_flow_directional_holds_to_its_cap_and_says_so() {
        // The rectangle's ring holds some 420 glyphs; each cap holds it
        // short, and it is laid all round within it.
        for with_corners in [true, false] {
            for cap in [3, 10, 40, 154] {
                let (_, placed, trace, checked) = capped(LIGHT, with_corners, &big(), cap);
                assert!(
                    placed.glyphs.len() <= cap,
                    "{} glyphs past the cap of {cap} (corners {with_corners})",
                    placed.glyphs.len()
                );
                assert_eq!(
                    trace.ring,
                    Some(RingLoss::Short),
                    "a ring its cap holds says so (cap {cap})"
                );
                assert_eq!(checked, Ok(()), "the flow fuzzer's check of a ring (cap {cap})");
            }
        }
        // Its corners are all set or none: a cap of 3 holds none of four.
        let (laid, _, _, _) = capped(LIGHT, true, &big(), 3);
        assert_eq!(
            laid.iter().filter(|l| matches!(l.part, Part::Corner(_))).count(),
            0,
            "a cap that cannot hold all four corners holds none"
        );
        // A cap that holds the whole ring lays it whole, and it is not
        // short: a run that lays less than its share passes the rest on.
        let rect = big().rect();
        for (with_corners, path) in [
            (true, big()),
            (false, big()),
            (true, Outline::RoundedRect { rect, radius: 200.0 }),
            (true, Outline::Ellipse { rect }),
        ] {
            let (whole, _, _) = lay(LIGHT, with_corners, &path);
            for cap in [whole.len(), whole.len() + 1] {
                let (laid, _, trace, checked) = capped(LIGHT, with_corners, &path, cap);
                assert_eq!(
                    laid.len(),
                    whole.len(),
                    "a cap of {cap} holds the whole ring of {} (corners {with_corners})",
                    whole.len()
                );
                assert_eq!(
                    trace.ring, None,
                    "a ring its cap holds whole is not short (cap {cap})"
                );
                assert_eq!(checked, Ok(()), "the flow fuzzer's check of a ring (cap {cap})");
            }
        }
        // At a cap that holds every run some ink, every side is drawn.
        let (laid, _, _, _) = capped(LIGHT, false, &big(), 154);
        let center = big().rect().center();
        for (dir, what) in [
            (Vec2::NEG_Y, "top"),
            (Vec2::X, "right"),
            (Vec2::Y, "bottom"),
            (Vec2::NEG_X, "left"),
        ] {
            assert!(
                laid.iter()
                    .any(|l| (l.ink.center() - center).normalize().dot(dir) > 0.9),
                "a ring its cap holds is laid on its {what} too"
            );
        }
    }

    #[test]
    fn test_flow_directional_lays_no_less_for_more_budget() {
        use crate::flow::place::halves::BUDGET_OVERRIDE;
        // A cap that cuts the ring, and budgets from its setup's to its
        // whole: a run its share cut is laid again only where it lays
        // more, so more budget never lays fewer glyphs.
        let path = Outline::RoundedRect {
            rect: big().rect(),
            radius: 200.0,
        };
        let (whole, trace, _) = lay(LIGHT, true, &path);
        let cap = whole.len() - 1;
        BUDGET_OVERRIDE.with(|b| b.set(Some(1)));
        let (_, setup, _) = lay(LIGHT, true, &path);
        let mut was = 0;
        for step in 0..=40 {
            let budget = setup.spent + (trace.spent - setup.spent) * step / 40;
            BUDGET_OVERRIDE.with(|b| b.set(Some(budget)));
            let (laid, _, _, _) = capped(LIGHT, true, &path, cap);
            assert!(
                laid.len() >= was,
                "a budget of {budget} lays {} glyphs, fewer than the {was} a smaller one laid",
                laid.len()
            );
            was = laid.len();
        }
        BUDGET_OVERRIDE.with(|b| b.set(None));
        assert_eq!(
            was, cap,
            "precondition: the whole budget lays the ring to its cap"
        );
    }

    #[test]
    fn test_flow_directional_holds_to_its_budget() {
        // An ellipse, whose runs read its path at every ink, and a box,
        // whose sides are straight lines, each counted as a read.
        for path in [Outline::Ellipse { rect: big().rect() }, big()] {
            held_to_its_budget(path);
        }
    }

    /// A ring of wide clusters — a letter under 240 marks, an ink of its
    /// runs 241 glyphs — held to its budget by construction (#313, ruling
    /// B18.1 §4): a run's count and each round of its spacing hold back
    /// what its last lays take and the glyphs of the inks it would then
    /// hold, which are counted once it is laid, as a pitched placement's
    /// do, so no run takes more after its last go than that go held back
    /// (`RUN_EDGE`), at any budget from its setup's to its whole, and no
    /// budget past the setup is passed. Before, its four lays' reads held
    /// back for the glyphs too: their slack covers some 136 glyphs an ink
    /// at a lay's most reads — `LIGHT` round a box and an ellipse never
    /// passed its budget at any of 2,001 budgets with no glyph term at all
    /// (review 5902086644) — but not an ink wider than that.
    ///
    /// Control: see the pull request of #313.
    #[test]
    fn test_flow_directional_holds_wide_clusters_to_its_budget() {
        use crate::flow::place::directional::RUN_EDGE;
        use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
        const MARKS: &str = "\u{0300}\u{0301}\u{0302}\u{0303}\u{0304}\u{0306}\u{0307}\u{0308}\u{030A}\u{030B}\u{030C}\u{0323}\u{0324}\u{0325}\u{0330}";
        let level = format!("o{}", MARKS.repeat(16));
        let steep = format!("l{}", MARKS.repeat(16));
        let set: (&str, &str, [&str; 4]) = (&level, &steep, LIGHT.2);
        let wide = with_fonts(|fs| {
            let run = real_run(&level, &style(POINTS), fs);
            assert_eq!(
                run.glyphs.iter().filter(|g| g.grapheme_start).count(),
                1,
                "precondition: the letter and its marks are one cluster"
            );
            run.glyphs.len()
        });
        assert!(wide > 136, "precondition: an ink of the ring is {wide} glyphs");
        // Its glyphs, and its trace: its marks draw no ink of their own, so
        // it is not parted as `lay` parts a ring.
        let lay = |set: (&str, &str, [&str; 4]), path: &Outline| {
            let text = style(POINTS);
            let layout = RingLayout {
                directional: glyphs(set, true),
                ..RingLayout::whole()
            };
            let mut trace = Trace::default();
            let placed = with_fonts(|fs| {
                let input = real_run(set.0, &text, fs);
                place_on_closed_traced(
                    &input,
                    path,
                    StartPoint {
                        edge: Edge::Top,
                        at: 0.0,
                    },
                    Direction::Clockwise,
                    &FitMode::Repeat,
                    &layout,
                    true,
                    &text,
                    fs,
                    Some(&mut trace),
                )
                .1
            });
            (placed.glyphs, trace)
        };
        for path in [Outline::Ellipse { rect: big().rect() }, big()] {
            RUN_EDGE.with(|edge| edge.set(None));
            let (whole, trace) = lay(set, &path);
            BUDGET_OVERRIDE.with(|b| b.set(Some(1)));
            let (_, setup) = lay(set, &path);
            BUDGET_OVERRIDE.with(|b| b.set(None));
            assert!(
                trace.spent > 2 * setup.spent && !whole.is_empty(),
                "precondition: the ring's work, {} steps, is more than twice its setup's, {}",
                trace.spent,
                setup.spent
            );
            let mut stopped = 0;
            for step in 1..=100 {
                let budget = setup.spent + (trace.spent - setup.spent) * step / 100;
                PAST_BUDGET.with(|past| past.set(0));
                BUDGET_OVERRIDE.with(|b| b.set(Some(budget)));
                let (_, held) = lay(set, &path);
                BUDGET_OVERRIDE.with(|b| b.set(None));
                let past = PAST_BUDGET.with(std::cell::Cell::get);
                assert_eq!(
                    past, 0,
                    "a budget of {budget} past the setup's {} is passed by {past}",
                    setup.spent
                );
                stopped += usize::from(held.ring == Some(RingLoss::Short));
            }
            assert!(
                stopped >= 25,
                "precondition: the budgets stop the ring ({stopped} of 100)"
            );
            let edge = RUN_EDGE
                .with(std::cell::Cell::get)
                .expect("precondition: its runs are laid");
            assert!(
                edge <= 0,
                "a run took {edge} glyph steps past what its last go held back for"
            );
        }
    }

    /// A ring round `path` held to its budget: never past it once it
    /// covers its setup, and nothing laid past its setup.
    fn held_to_its_budget(path: Outline) {
        use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
        let (whole, trace, _) = lay(LIGHT, true, &path);
        assert!(trace.spent <= trace.budget, "an ordinary ring within its budget");
        assert_eq!(trace.ring, None, "an ordinary ring is not stopped");
        // A budget past the setup but short of the ring: what is held back
        // for its lays keeps it within, to the step.
        PAST_BUDGET.with(|past| past.set(0));
        BUDGET_OVERRIDE.with(|b| b.set(Some(trace.spent / 2)));
        let (laid, short, _) = lay(LIGHT, true, &path);
        BUDGET_OVERRIDE.with(|b| b.set(None));
        let past = PAST_BUDGET.with(std::cell::Cell::get);
        assert_eq!(past, 0, "a budget past the setup is never passed");
        assert_eq!(
            short.ring,
            Some(RingLoss::Short),
            "a spent budget is reported, never silent"
        );
        assert!(
            laid.len() < whole.len(),
            "precondition: the budget stopped the ring"
        );
        // A budget its setup alone spends: no run is laid past it.
        BUDGET_OVERRIDE.with(|b| b.set(Some(1)));
        let (bare, setup, _) = lay(LIGHT, true, &path);
        BUDGET_OVERRIDE.with(|b| b.set(None));
        assert!(
            trace.spent > 2 * setup.spent,
            "precondition: the ring's work, {} steps, is more than twice its setup's, {}, \
             so half of it is past the setup",
            trace.spent,
            setup.spent
        );
        assert_eq!(setup.ring, Some(RingLoss::Short), "a spent budget is reported");
        assert_eq!(
            bare.iter().filter(|l| !matches!(l.part, Part::Corner(_))).count(),
            0,
            "nothing is laid past a budget its setup spends but its corners"
        );
    }
}
