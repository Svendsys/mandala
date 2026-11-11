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
    let text = style(POINTS);
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
/// box: every glyph of its spans along the top or the bottom, every
/// vertical one down a side, each corner once at its own corner, and each
/// corner's strokes run on in its neighbors' — its ink's edge on the
/// level glyphs' ink, across, and on the vertical glyphs', along.
pub fn do_flow_directional_lays_a_box() {
    let path = outlines()[0].1;
    let (laid, _, _) = lay(LIGHT, true, &path);
    let of = |part: Part| laid.iter().filter(move |l| l.part == part);
    assert!(
        of(Part::Level).count() >= 40,
        "precondition: the top and the bottom are laid"
    );
    assert!(of(Part::Steep).count() >= 8, "precondition: the sides are laid");
    let rect = path.rect();
    let center = rect.center();
    for l in of(Part::Level) {
        assert!(
            (l.ink.center().y - center.y).abs() > rect.size.y * 0.5,
            "a level glyph off the top and the bottom: {l:?}"
        );
    }
    for l in of(Part::Steep) {
        assert!(
            (l.ink.center().x - center.x).abs() > rect.size.x * 0.5,
            "a vertical glyph off the sides: {l:?}"
        );
    }
    let level = of(Part::Level).next().expect("a level glyph").ink;
    let steep = of(Part::Steep).next().expect("a vertical glyph").ink;
    for (i, sign) in [
        (0, Vec2::new(-1.0, -1.0)),
        (1, Vec2::new(1.0, -1.0)),
        (2, Vec2::new(1.0, 1.0)),
        (3, Vec2::new(-1.0, 1.0)),
    ] {
        let corners: Vec<&Laid> = of(Part::Corner(i)).collect();
        assert_eq!(corners.len(), 1, "corner {i} laid once");
        let ink = corners[0].ink;
        let d = ink.center() - center;
        assert!(
            d.x * sign.x > 0.0 && d.y * sign.y > 0.0,
            "corner {i} at its own corner: {ink:?}"
        );
        // Its horizontal stroke is the level glyphs' row: its ink's edge
        // on the outside is theirs, across.
        let (outer, row_outer) = if sign.y < 0.0 {
            (ink.origin.y, level_row(&laid, true))
        } else {
            (ink.max().y, level_row(&laid, false))
        };
        assert!(
            (outer - row_outer).abs() < 0.01,
            "corner {i}'s stroke on the row: {outer} vs {row_outer}"
        );
        let (side, column_outer) = if sign.x < 0.0 {
            (ink.origin.x, steep_column(&laid, true))
        } else {
            (ink.max().x, steep_column(&laid, false))
        };
        assert!(
            (side - column_outer).abs() < 0.01,
            "corner {i}'s stroke on the column: {side} vs {column_outer}"
        );
    }
    assert!(
        level.size.x > level.size.y && steep.size.y > steep.size.x,
        "precondition: the strokes lie their ways"
    );
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

/// The heavy one.
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

/// The gaps between consecutive inks of one part along `axis`, in the
/// order laid, starting a new run wherever the part changes.
fn gaps(laid: &[Laid], part: Part, axis: impl Fn(&Laid, &Laid) -> f32) -> Vec<Vec<f32>> {
    let mut runs: Vec<Vec<f32>> = Vec::new();
    let mut last: Option<&Laid> = None;
    for l in laid {
        if l.part != part {
            last = None;
            continue;
        }
        match (last, runs.last_mut()) {
            (Some(prev), Some(run)) => run.push(axis(prev, l)),
            _ => runs.push(Vec::new()),
        }
        last = Some(l);
    }
    runs.retain(|r| !r.is_empty());
    runs
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

#[test]
fn test_flow_directional_hysteresis_leaves_out_a_short_run() {
    do_flow_directional_hysteresis_leaves_out_a_short_run();
}

/// A run shorter than three line heights is no run of its own: a flat
/// ellipse lays its spans all round, a tall one its vertical text, with
/// no corner at either.
pub fn do_flow_directional_hysteresis_leaves_out_a_short_run() {
    // A flat ellipse: its steep tips are each shorter than three line
    // heights, so they make no run, and the level runs meet there with
    // no corner between them.
    let flat = offset_keeping_corners(
        &Outline::Ellipse {
            rect: Rect::new(Vec2::new(-300.0, -12.0), Vec2::new(600.0, 24.0)),
        },
        0.0,
    );
    let (laid, _, _) = lay(LIGHT, true, &flat);
    assert!(
        laid.iter().filter(|l| l.part == Part::Level).count() > 50,
        "precondition: laid"
    );
    assert!(
        laid.iter().all(|l| l.part == Part::Level),
        "a flat ellipse's tips lay no vertical glyph and no corner"
    );
    // A tall one: its level tips make no run.
    let tall = Outline::Ellipse {
        rect: Rect::new(Vec2::new(-12.0, -300.0), Vec2::new(24.0, 600.0)),
    };
    let (laid, _, _) = lay(LIGHT, true, &tall);
    assert!(
        laid.iter().filter(|l| l.part == Part::Steep).count() > 20,
        "precondition: laid"
    );
    assert!(
        laid.iter().all(|l| l.part == Part::Steep),
        "a tall ellipse's tips lay no level glyph and no corner"
    );
}

#[test]
fn test_flow_directional_lays_corners_only_where_it_turns_as_a_box() {
    do_flow_directional_lays_corners_only_where_it_turns_as_a_box();
}

/// A corner is laid where the outline turns as a box does — a sharp
/// rectangle's corners, and arcs narrower than the corner — and not round
/// a rounded rectangle's wider arcs or an ellipse.
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
            "a rounded rectangle's arcs, wider than a corner, are no box's turn"
        );
        assert_eq!(corners(&ellipse.1), 0, "an ellipse turns round a curve");
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
            "corners round arcs narrower than a corner are a box's"
        );
    }
}

#[test]
fn test_flow_directional_spaces_each_run_evenly_and_never_apart() {
    do_flow_directional_spaces_each_run_evenly_and_never_apart();
}

/// Each run's gaps, along the way it reads, are one gap, and none is
/// looser than the font sets two of its spans' glyph: box-drawing strokes
/// never part.
pub fn do_flow_directional_spaces_each_run_evenly_and_never_apart() {
    for set in [LIGHT, HEAVY] {
        let path = outlines()[0].1;
        let (laid, _, run) = lay(set, true, &path);
        // As a line sets two of the spans' glyph: the font's own gap,
        // below 0 where the face overhangs its advance.
        let h = run
            .glyphs
            .iter()
            .find(|g| g.span != FILLER_SPAN)
            .expect("a spans glyph");
        let ink = h.ink.expect("ink");
        let natural = (h.advance - ink.max.x) + ink.min.x;
        let across = |a: &Laid, b: &Laid| {
            let (a, b) = (a.ink, b.ink);
            (b.origin.x - a.max().x).max(a.origin.x - b.max().x)
        };
        let down = |a: &Laid, b: &Laid| {
            let (a, b) = (a.ink, b.ink);
            (b.origin.y - a.max().y).max(a.origin.y - b.max().y)
        };
        for (part, axis) in [
            (Part::Level, &across as &dyn Fn(&Laid, &Laid) -> f32),
            (Part::Steep, &down),
        ] {
            let runs = gaps(&laid, part, axis);
            assert_eq!(runs.len(), 2, "precondition: two runs of {part:?}");
            for gaps in runs {
                let (lo, hi) = gaps
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &g| {
                        (lo.min(g), hi.max(g))
                    });
                assert!(hi - lo < 0.01, "{part:?}: a run's gaps are one gap: {gaps:?}");
                assert!(
                    hi <= natural + 0.01,
                    "{part:?}: no gap looser than the font's ({natural}): {gaps:?}"
                );
            }
        }
    }
}

/// The budget: it reads the frame's test seams, so it is no bench body.
#[cfg(test)]
mod budget {
    use super::*;
    use crate::flow::place::ring::RingLoss;

    #[test]
    fn test_flow_directional_holds_to_its_budget() {
        use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
        let path = outlines()[2].1;
        let (_, trace, _) = lay(LIGHT, false, &path);
        assert!(trace.budget > 0 && trace.spent > 0, "precondition: counted");
        assert!(trace.spent <= trace.budget, "an ordinary ring within its budget");
        assert_eq!(trace.ring, None, "an ordinary ring is not stopped");
        PAST_BUDGET.with(|past| past.set(0));
        BUDGET_OVERRIDE.with(|b| b.set(Some(trace.spent / 2)));
        let (laid, short, _) = lay(LIGHT, false, &path);
        BUDGET_OVERRIDE.with(|b| b.set(None));
        assert_eq!(
            short.ring,
            Some(RingLoss::Short),
            "a spent budget is reported, never silent"
        );
        assert!(!laid.is_empty(), "the runs laid as far as their counts got");
        let past = PAST_BUDGET.with(std::cell::Cell::get);
        assert!(
            past < 2_000,
            "at most a setup and one ink past its budget: {past}"
        );
    }
}
