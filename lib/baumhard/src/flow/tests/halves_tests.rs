// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33 and R33.1): even ink tracking, every steep run
//! reading down, each junction a visible break, the span moving the
//! lines' ends, the side ornament round the junctions, and a `repeat`
//! ring's spacing. Runs are shaped for real; every ink is measured here
//! from the glyph outlines, unhinted, the ink placement steps by, and
//! every expectation is a property of the positions.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Rgba, Shape, Span, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::place::halves::track_seam;
use crate::flow::place::{place_lines, place_on_closed_traced, Trace, MIN_TABLE_SPACING};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, RingLayout, ShapedRun, FILLER_SPAN};
use crate::font::fonts::glyph_outline_ink;
use crate::font::{FontSystem, SwashCache};

/// The README's Blake quatrain.
const QUATRAIN: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower, \
     Hold Infinity in the palm of your hand, And Eternity in an hour. ";

/// The psalm the README sets inside it.
const PSALM: &str = "The heavens declare the glory of God; and the firmament sheweth his handywork. ";

/// The Ecclesiastes center frame.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

/// The frames scene's rounded rectangle.
const ROUNDED: &str = "A frame of text around a rounded rectangle, shaped once and placed glyph by glyph. ";

/// The README's outlines, each with the text and size it holds there.
fn scenes() -> Vec<(&'static str, Outline, &'static str, f32)> {
    let rect = |w: f32, h: f32| Rect::new(Vec2::ZERO, Vec2::new(w, h));
    vec![
        (
            "quatrain ellipse",
            outline(rect(1360.0, 720.0), Shape::Ellipse),
            QUATRAIN,
            30.0,
        ),
        (
            "psalm ellipse",
            outline(rect(860.0, 380.0), Shape::Ellipse),
            PSALM,
            30.0,
        ),
        (
            "center ellipse",
            outline(rect(820.0, 330.0), Shape::Ellipse),
            ECCLESIASTES,
            38.0,
        ),
        (
            "rounded rect",
            outline(
                rect(260.0, 170.0),
                Shape::Rectangle {
                    radius: Percent::new(60),
                },
            ),
            ROUNDED,
            22.0,
        ),
        (
            "pill",
            outline(
                rect(600.0, 200.0),
                Shape::Rectangle {
                    radius: Percent::new(100),
                },
            ),
            ECCLESIASTES,
            22.0,
        ),
    ]
}

/// How far an end may seem to move back between two spans, in points:
/// what projecting an ink's center onto the outline is precise to.
const SLACK: f32 = 0.5;

const FIT: FitMode = FitMode::Fit { min: 0.5, max: 2.0 };

/// `text` at `points` laid round `outline` as a frame, with `layout`.
fn frame(text: &str, points: f32, outline: &Outline, layout: &RingLayout) -> (ShapedRun, PlacedRun, Trace) {
    let flow = style(points);
    let mut trace = Trace::default();
    let (rescaled, placed) = with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        let start = StartPoint {
            edge: Edge::Top,
            at: 0.0,
        };
        let (rescaled, placed) = place_on_closed_traced(
            &run,
            outline,
            start,
            Direction::Clockwise,
            &FIT,
            layout,
            true,
            &flow,
            fs,
            Some(&mut trace),
        );
        (rescaled.unwrap_or(run), placed)
    });
    (rescaled, placed, trace)
}

/// Each placed glyph's ink box on the canvas, measured here from its
/// outline; `None` for a space.
fn inks(run: &ShapedRun, placed: &PlacedRun) -> Vec<Option<(Vec2, Vec2)>> {
    with_fonts(|fs| {
        let fs: &mut FontSystem = fs;
        let mut cache = SwashCache::new();
        let laid: Vec<_> = run.layout_glyphs().cloned().collect();
        placed
            .glyphs
            .iter()
            .map(|g| {
                let (min, max) = glyph_outline_ink(fs, &mut cache, &laid[g.glyph as usize])?;
                let pen = g.pos + Vec2::new(0.0, run.baseline());
                Some((pen + min, pen + max))
            })
            .collect()
    })
}

/// The distance between two boxes, 0 where they meet.
fn gap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    (b.0 - a.1).max(a.0 - b.1).max(Vec2::ZERO).length()
}

fn center(b: (Vec2, Vec2)) -> Vec2 {
    0.5 * (b.0 + b.1)
}

/// One line's letter gaps (between neighboring inks of a word) and word
/// gaps (across a space), and its first and last ink.
struct LineGaps {
    letters: Vec<f32>,
    words: Vec<f32>,
    ends: ((Vec2, Vec2), (Vec2, Vec2)),
    centers: Vec<Vec2>,
}

fn line_gaps(
    run: &ShapedRun,
    placed: &PlacedRun,
    inks: &[Option<(Vec2, Vec2)>],
    range: std::ops::Range<usize>,
) -> LineGaps {
    let (mut letters, mut words, mut centers) = (Vec::new(), Vec::new(), Vec::new());
    let mut previous: Option<(Vec2, Vec2)> = None;
    let mut spaced = false;
    let mut first = None;
    for k in range {
        if run.glyphs[placed.glyphs[k].glyph as usize].blank {
            spaced = true;
            continue;
        }
        let Some(ink) = inks[k] else { continue };
        if let Some(p) = previous {
            if spaced {
                words.push(gap(p, ink));
            } else {
                letters.push(gap(p, ink));
            }
        }
        first.get_or_insert(ink);
        centers.push(center(ink));
        previous = Some(ink);
        spaced = false;
    }
    let first = first.expect("a line with ink");
    LineGaps {
        letters,
        words,
        ends: (first, previous.expect("a line with ink")),
        centers,
    }
}

fn spread(v: &[f32]) -> (f32, f32) {
    v.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &x| {
        (a.min(x), b.max(x))
    })
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

#[test]
fn test_flow_halves_track_evenly_and_read_down() {
    do_flow_halves_track_evenly_and_read_down();
}

/// How unevenly a line's letters may be tracked: the most two of its
/// letter gaps may differ by, as a ratio. The placement steps by the ink
/// it is measured with here, so its gaps agree to the solve's tolerance;
/// a gap off by a hinted raster's pixel at either edge — the E7
/// checker's 1.31, on the psalm's 3.7 points — would fail it.
const EVEN: f32 = 1.01;

/// Laid over its whole diagonal half (rulings R32, R33), every line of
/// the README's frames tracks evenly — its letters' inks, measured from
/// their outlines, within `EVEN` of each other's gaps, where it runs
/// level and where it runs steep — at span whole and at span ½, and
/// every run of three or more letters steeper than 45° goes down the
/// canvas: the first line takes the upper-right half, the second the
/// lower-left.
pub fn do_flow_halves_track_evenly_and_read_down() {
    for (name, shape, text, points) in scenes() {
        for span in [Span::WHOLE, Span::new(0.5).unwrap_or(Span::WHOLE)] {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) = frame(text, points, &shape, &layout);
            let inks = inks(&run, &placed);
            assert_eq!(trace.lines.len(), 2, "{name}: two lines");
            for (k, line) in trace.lines.iter().enumerate() {
                let what = format!("{name} span {} line {k}", span.get());
                let gaps = line_gaps(&run, &placed, &inks, line.glyphs.clone());
                let (least, most) = spread(&gaps.letters);
                assert!(
                    least > 0.0 && most <= EVEN * least,
                    "{what}: letter gaps from {least} to {most}"
                );
                let mut run_dy: Vec<f32> = Vec::new();
                let check = |run_dy: &mut Vec<f32>| {
                    if run_dy.len() >= 2 {
                        assert!(
                            run_dy.iter().all(|&dy| dy > 0.0),
                            "{what}: a steep run goes up: {run_dy:?}"
                        );
                    }
                    run_dy.clear();
                };
                for w in gaps.centers.windows(2) {
                    let d = w[1] - w[0];
                    if d.y.abs() > d.x.abs() {
                        run_dy.push(d.y);
                    } else {
                        check(&mut run_dy);
                    }
                }
                check(&mut run_dy);
            }
        }
    }
}

#[test]
fn test_flow_halves_junctions_are_breaks() {
    do_flow_halves_junctions_are_breaks();
}

/// Each junction is a visible break (ruling R33.1): at span whole, the
/// two lines' nearest end inks are at least two word gaps apart — of
/// the two lines' mean, a word gap measured across a space — at the
/// top-left junction and the bottom-right.
pub fn do_flow_halves_junctions_are_breaks() {
    for (name, shape, text, points) in scenes() {
        let (run, placed, trace) = frame(text, points, &shape, &RingLayout::whole());
        let inks = inks(&run, &placed);
        let lines: Vec<LineGaps> = trace
            .lines
            .iter()
            .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
            .collect();
        let word = mean(&lines.iter().map(|l| mean(&l.words)).collect::<Vec<_>>());
        let (a1, b1) = lines[0].ends;
        let (a2, b2) = lines[1].ends;
        for (junction, apart) in [("top-left", gap(a1, a2)), ("bottom-right", gap(b1, b2))] {
            assert!(
                apart >= 2.0 * word,
                "{name} {junction}: the lines' ends {apart} apart, the word gap {word}"
            );
        }
    }
}

#[test]
fn test_flow_halves_span_moves_the_ends_monotonically() {
    do_flow_halves_span_moves_the_ends_monotonically();
}

/// The span moves each line's ends from their natural places toward its
/// half's junctions (ruling R32): as it grows from natural to whole, each
/// line's first ink moves along its half no nearer its middle and its
/// last ink no nearer either — measured by arc length along the outline,
/// each ink's center projected onto it — and the arc its inks cover
/// never shrinks, within `SLACK` for the projection's precision.
pub fn do_flow_halves_span_moves_the_ends_monotonically() {
    for (name, shape, text, points) in scenes() {
        let left = StartPoint {
            edge: Edge::Left,
            at: 0.5,
        };
        let paths = [
            ArcPath::closed(&shape, left, Direction::Clockwise, MIN_TABLE_SPACING),
            ArcPath::closed(&shape, left, Direction::CounterClockwise, MIN_TABLE_SPACING),
        ];
        let length = paths[0].length();
        // Along half `k`, from its top-left junction.
        let along = |k: usize, p: Vec2| {
            let start = paths[k].nearest(shape.support(Vec2::new(-1.0, -1.0)));
            (paths[k].nearest(p) - start).rem_euclid(length)
        };
        let mut previous: Option<Vec<(f32, f32)>> = None;
        for step in 0..=4 {
            let span = Span::new(step as f32 / 4.0).unwrap_or(Span::WHOLE);
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) = frame(text, points, &shape, &layout);
            let inks = inks(&run, &placed);
            let lines: Vec<(f32, f32)> = trace
                .lines
                .iter()
                .enumerate()
                .map(|(k, l)| {
                    let g = line_gaps(&run, &placed, &inks, l.glyphs.clone());
                    (along(k, center(g.ends.0)), along(k, center(g.ends.1)))
                })
                .collect();
            if let Some(before) = &previous {
                for (k, (now, then)) in lines.iter().zip(before).enumerate() {
                    let what = format!("{name} line {k} at span {}", span.get());
                    assert!(
                        now.0 <= then.0 + SLACK,
                        "{what}: its start moved back toward its middle: {then:?} → {now:?}"
                    );
                    assert!(
                        now.1 >= then.1 - SLACK,
                        "{what}: its end moved back toward its middle: {then:?} → {now:?}"
                    );
                    assert!(
                        now.1 - now.0 >= then.1 - then.0 - SLACK,
                        "{what}: it covers less: {then:?} → {now:?}"
                    );
                }
            }
            previous = Some(lines);
        }
    }
}

#[test]
fn test_flow_halves_tracking_grows_with_the_gap() {
    do_flow_halves_tracking_grows_with_the_gap();
}

/// A line's length grows with its tracking gap from zero on (ruling R32):
/// every letter after the first is set by the pairwise rule, a zero gap
/// included, so no gap lays it longer than a wider one — which is what
/// lets the gap be solved by bisection.
pub fn do_flow_halves_tracking_grows_with_the_gap() {
    let gaps = [0.0, 0.05, 0.25, 0.5, 1.0, 2.0, 4.0];
    for (name, shape, text, points) in scenes() {
        let ends = with_fonts(|fs| {
            let run = real_run(&text[..text.len().min(40)], &style(points), fs);
            let fs: &mut FontSystem = fs;
            track_seam(&run, &shape, &gaps, fs)
        });
        for (k, pair) in ends.windows(2).enumerate() {
            assert!(
                pair[1] > pair[0],
                "{name}: the line ends at {} with gap {}, and at {} with gap {}",
                pair[0],
                gaps[k],
                pair[1],
                gaps[k + 1]
            );
        }
    }
}

#[test]
fn test_flow_halves_sides_hold_the_junctions() {
    do_flow_halves_sides_hold_the_junctions();
}

/// With `sides` (ruling R33.1), each junction holds at least one copy of
/// the ornament, every gap between its copies and both its joins to the
/// text within 10% of each other, its ink no taller than the text's
/// x-height, nothing touching the text — at span whole and at span ½.
pub fn do_flow_halves_sides_hold_the_junctions() {
    let x_height = |points: f32| {
        with_fonts(|fs| {
            let run = real_run("x", &style(points), fs);
            let fs: &mut FontSystem = fs;
            let mut cache = SwashCache::new();
            let laid: Vec<_> = run.layout_glyphs().cloned().collect();
            glyph_outline_ink(fs, &mut cache, &laid[0]).map_or(0.0, |(min, max)| max.y - min.y)
        })
    };
    for ornament in ["·", "✦"] {
        for (name, shape, text, points) in scenes() {
            for span in [Span::WHOLE, Span::new(0.5).unwrap_or(Span::WHOLE)] {
                let what = format!("{name} {ornament:?} span {}", span.get());
                let layout = RingLayout {
                    span,
                    sides: Some((ornament.to_owned(), Rgba::opaque(200, 200, 200))),
                    ..RingLayout::whole()
                };
                let (run, placed, trace) = frame(text, points, &shape, &layout);
                let inks = inks(&run, &placed);
                let is_side = |k: usize| run.glyphs[placed.glyphs[k].glyph as usize].span == FILLER_SPAN;
                let sides: Vec<(Vec2, Vec2)> = (0..placed.glyphs.len())
                    .filter(|&k| is_side(k))
                    .filter_map(|k| inks[k])
                    .collect();
                let text_inks: Vec<(Vec2, Vec2)> = (0..placed.glyphs.len())
                    .filter(|&k| !is_side(k))
                    .filter_map(|k| inks[k])
                    .collect();
                let size = run
                    .glyphs
                    .iter()
                    .filter(|g| g.span != FILLER_SPAN)
                    .map(|g| g.size)
                    .fold(0.0_f32, f32::max);
                let cap = x_height(size);
                for side in &sides {
                    assert!(
                        side.1.y - side.0.y <= cap + 0.05,
                        "{what}: an ornament {} tall, the x-height {cap}",
                        side.1.y - side.0.y
                    );
                    for t in &text_inks {
                        assert!(gap(*side, *t) > 0.0, "{what}: an ornament touches the text");
                    }
                }
                let lines: Vec<LineGaps> = trace
                    .lines
                    .iter()
                    .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
                    .collect();
                let (a1, b1) = lines[0].ends;
                let (a2, b2) = lines[1].ends;
                for (junction, from, to) in [("top-left", a2, a1), ("bottom-right", b1, b2)] {
                    // The copies between the two end inks.
                    let mid = 0.5 * (center(from) + center(to));
                    let radius = 0.5 * center(from).distance(center(to));
                    let mut held: Vec<(Vec2, Vec2)> = sides
                        .iter()
                        .copied()
                        .filter(|s| center(*s).distance(mid) < radius)
                        .collect();
                    assert!(!held.is_empty(), "{what} {junction}: no ornament");
                    held.sort_by(|a, b| {
                        center(*a)
                            .distance(center(from))
                            .total_cmp(&center(*b).distance(center(from)))
                    });
                    let mut gaps = vec![gap(from, held[0]), gap(held[held.len() - 1], to)];
                    gaps.extend(held.windows(2).map(|w| gap(w[0], w[1])));
                    let (least, most) = spread(&gaps);
                    assert!(
                        most <= 1.1 * least + 0.05,
                        "{what} {junction}: gaps and joins from {least} to {most}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_flow_halves_no_line_is_laid_tighter_than_its_font() {
    do_flow_halves_no_line_is_laid_tighter_than_its_font();
}

/// A frame keeps the size R27 gives it, and no line of it is laid
/// tighter than its font — its letters' mean side bearings, the gap R27
/// leaves between their inks on a level line: at span natural every
/// letter gap is its font's own, and at span ½ and whole, sides or none,
/// none is less. Where a natural line reaches into a junction's clearance
/// (the Ecclesiastes center's second line, at the bottom-right) it is
/// slid clear rather than pressed. (Measured once by projection from far
/// along a line, the end test pointed the wrong way and shrank the
/// center frame to a tenth.)
pub fn do_flow_halves_no_line_is_laid_tighter_than_its_font() {
    let size = |run: &ShapedRun| {
        run.glyphs
            .iter()
            .filter(|g| g.span != FILLER_SPAN)
            .map(|g| g.size)
            .fold(0.0_f32, f32::max)
    };
    let sides = RingLayout {
        sides: Some(("✦".to_owned(), Rgba::opaque(200, 200, 200))),
        ..RingLayout::whole()
    };
    let half = RingLayout {
        span: Span::new(0.5).unwrap_or(Span::WHOLE),
        ..RingLayout::whole()
    };
    for (name, shape, text, points) in scenes() {
        let fitted = with_fonts(|fs| {
            let flow = style(points);
            let run = real_run(text, &flow, fs);
            let (rescaled, _) = place_lines(&run, &shape, &FIT, true, &flow, fs, None);
            size(&rescaled.unwrap_or(run))
        });
        for (label, layout) in [
            ("natural", RingLayout::natural()),
            ("½", half.clone()),
            ("whole", RingLayout::whole()),
            ("whole with sides", sides.clone()),
        ] {
            let what = format!("{name} at {label}");
            let (run, placed, trace) = frame(text, points, &shape, &layout);
            assert!(
                (size(&run) - fitted).abs() <= 1e-3 * fitted,
                "{what}: sized {} from R27's {fitted}",
                size(&run)
            );
            let inks = inks(&run, &placed);
            for (k, line) in trace.lines.iter().enumerate() {
                let own: Vec<f32> = line
                    .glyphs
                    .clone()
                    .filter(|&i| !run.glyphs[placed.glyphs[i].glyph as usize].blank)
                    .filter_map(|i| {
                        inks[i]
                            .map(|(a, b)| run.glyphs[placed.glyphs[i].glyph as usize].advance - (b.x - a.x))
                    })
                    .collect();
                let own = mean(&own);
                let (least, most) = spread(&line_gaps(&run, &placed, &inks, line.glyphs.clone()).letters);
                assert!(
                    least >= own - 1e-3,
                    "{what} line {k}: letters {least} apart, its font's gap {own}"
                );
                if label == "natural" {
                    assert!(
                        most <= own + 1e-2,
                        "{what} line {k}: letters {most} apart, its font's gap {own}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_flow_halves_a_repeat_ring_takes_its_spacing() {
    do_flow_halves_a_repeat_ring_takes_its_spacing();
}

/// A `repeat` ring's `spacing` (ruling R31) parts its copies by that much
/// more ink to ink: a `"✦ "` ring with 6 points of spacing holds fewer
/// copies than with none, each more than 6 points further from the next
/// (a gap follows each of a copy's two steps).
pub fn do_flow_halves_a_repeat_ring_takes_its_spacing() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(400.0, 220.0)), Shape::Ellipse);
    let ring = |spacing: f32| {
        let flow = style(16.0);
        let layout = RingLayout {
            spacing,
            ..RingLayout::whole()
        };
        let (run, placed) = with_fonts(|fs| {
            let run = real_run("✦ ", &flow, fs);
            let start = StartPoint {
                edge: Edge::Top,
                at: 0.0,
            };
            let (r, p) = place_on_closed_traced(
                &run,
                &shape,
                start,
                Direction::Clockwise,
                &FitMode::Repeat,
                &layout,
                true,
                &flow,
                fs,
                None,
            );
            (r.unwrap_or(run), p)
        });
        let inks: Vec<(Vec2, Vec2)> = inks(&run, &placed).into_iter().flatten().collect();
        let mut gaps: Vec<f32> = inks.windows(2).map(|w| gap(w[0], w[1])).collect();
        gaps.push(gap(inks[inks.len() - 1], inks[0]));
        (inks.len(), mean(&gaps))
    };
    let (none_count, none_gap) = ring(0.0);
    let (some_count, some_gap) = ring(6.0);
    assert!(
        some_count < none_count,
        "{some_count} copies with spacing, {none_count} without"
    );
    assert!(
        some_gap > none_gap + 6.0,
        "copies {none_gap} apart without spacing, {some_gap} with"
    );
}
