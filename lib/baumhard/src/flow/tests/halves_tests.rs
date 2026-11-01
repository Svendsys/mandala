// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33 and R33.1): even ink tracking, every steep run
//! reading down, each junction a visible break, the span moving the
//! lines' ends, the side ornament round the junctions, and a `repeat`
//! ring's spacing. Runs are shaped for real; every ink is measured here
//! from the glyph outlines, unhinted, the ink placement steps by, and
//! every expectation is a property of the positions.

use glam::Vec2;

use crate::document::resolve::{ResolvedRing, ResolvedSides};
use crate::document::values::{Direction, Edge, FitMode, LineSpan, Percent, Rgba, Shape, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::place::halves::{halves_seam, room_seam, side_floor_seam, track_seam};
use crate::flow::place::{place_lines, place_on_closed_traced, Trace, INK_TOLERANCE, MIN_TABLE_SPACING};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedGlyph, PlacedRun, Rect, RingLayout, ShapedRun, FILLER_SPAN};
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

/// `text` at `points` laid round `outline` as a `Fit` frame, outward,
/// with `layout`.
fn frame(text: &str, points: f32, outline: &Outline, layout: &RingLayout) -> (ShapedRun, PlacedRun, Trace) {
    frame_with(text, points, outline, &FIT, layout, true)
}

/// `text` at `points` laid round `outline` as a frame with `fit` and
/// `layout`, outward or inward.
fn frame_with(
    text: &str,
    points: f32,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
) -> (ShapedRun, PlacedRun, Trace) {
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
            fit,
            layout,
            outward,
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
        for span in [LineSpan::WHOLE, LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE)] {
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
/// the mean of the lines that have one, a word gap measured across a
/// space — at the top-left junction and the bottom-right.
pub fn do_flow_halves_junctions_are_breaks() {
    for (name, shape, text, points) in scenes() {
        let (run, placed, trace) = frame(text, points, &shape, &RingLayout::whole());
        let inks = inks(&run, &placed);
        let lines: Vec<LineGaps> = trace
            .lines
            .iter()
            .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
            .collect();
        let word = word_gap(&lines);
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

/// The lines' mean word gap, over the lines that have one; with none on
/// any line, two of a line's letter gaps, of the lines' mean.
fn word_gap(lines: &[LineGaps]) -> f32 {
    let words: Vec<f32> = lines
        .iter()
        .filter(|l| !l.words.is_empty())
        .map(|l| mean(&l.words))
        .collect();
    if words.is_empty() {
        mean(&lines.iter().map(|l| 2.0 * mean(&l.letters)).collect::<Vec<_>>())
    } else {
        mean(&words)
    }
}

#[test]
fn test_flow_halves_a_one_word_line_keeps_its_neighbor_whole() {
    do_flow_halves_a_one_word_line_keeps_its_neighbor_whole();
}

/// The cosine between `outline`'s outward normals at the points nearest
/// the centers of the ink boxes `a` and `b`.
fn outward_turn(outline: &Outline, a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    let left = StartPoint {
        edge: Edge::Left,
        at: 0.5,
    };
    let path = ArcPath::closed(outline, left, Direction::Clockwise, MIN_TABLE_SPACING);
    let normal = |p: Vec2| {
        let tangent = path.at(path.nearest(p)).1;
        Vec2::new(tangent.y, -tangent.x)
    };
    normal(center(a)).dot(normal(center(b)))
}

/// A line of one word, or of glyphs with no space between them, has no
/// word gap: spread over its half at span whole, its letter gaps are
/// the span's, and the junction break is owed in the other line's word
/// gaps (ruling R33.1), not in a word gap the one-word line does not
/// have. So the other line keeps its room: a `Once` frame cuts nothing,
/// a `Fit` frame keeps the size R27 gives it, and each junction is two
/// of the other line's word gaps wide. With no word gap on either line,
/// each junction is two of the lines' doubled letter gaps wide — or, its
/// ends drawn back until they are stacked across the outline (their
/// normals turned past 135°, rulings R35.1 and R38.2), clear.
pub fn do_flow_halves_a_one_word_line_keeps_its_neighbor_whole() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(820.0, 330.0)), Shape::Ellipse);
    for (text, spaced) in [
        ("and a time to every purpose under the\nheaven", true),
        ("Infinity\nEternity", false),
    ] {
        one_word_line(&shape, text, spaced);
    }
}

fn one_word_line(shape: &Outline, text: &str, spaced: bool) {
    let size = |run: &ShapedRun| run.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
    let sized = with_fonts(|fs| {
        let flow = style(38.0);
        let run = real_run(text, &flow, fs);
        let (rescaled, _) = place_lines(&run, shape, &FIT, true, &flow, fs, None);
        size(&rescaled.unwrap_or(run))
    });
    for fit in [FIT, FitMode::Once { filler: None }] {
        let (run, placed, trace) = frame_with(text, 38.0, shape, &fit, &RingLayout::whole(), true);
        assert_eq!(trace.lines.len(), 2, "precondition: {text:?} {fit:?}: two lines");
        let inks = inks(&run, &placed);
        let lines: Vec<LineGaps> = trace
            .lines
            .iter()
            .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
            .collect();
        assert!(
            lines[1].words.is_empty() && lines[0].words.is_empty() != spaced,
            "precondition: {text:?} {fit:?}: the second line is one word, the first {}",
            if spaced { "more" } else { "too" }
        );
        assert!(!placed.truncated, "{text:?} {fit:?}: nothing cut");
        if matches!(fit, FitMode::Fit { .. }) {
            assert!(
                (size(&run) - sized).abs() < 1e-3,
                "{text:?} {fit:?}: sized {} from R27's {sized}",
                size(&run)
            );
        }
        let word = word_gap(&lines);
        let (a1, b1) = lines[0].ends;
        let (a2, b2) = lines[1].ends;
        for (junction, a, b) in [("top-left", a1, a2), ("bottom-right", b1, b2)] {
            let apart = gap(a, b);
            // Ends drawn back until their outward normals turn past 135°
            // are stacked across the outline, not meeting at it (rulings
            // R35.1, R38.2): they need only not touch.
            let stacked = outward_turn(shape, a, b) < (135.0_f32).to_radians().cos();
            assert!(
                apart >= 2.0 * word || (stacked && apart > 0.0),
                "{text:?} {fit:?} {junction}: the lines' ends {apart} apart, the word gap {word}"
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
            let span = LineSpan::new(step as f32 / 4.0).unwrap_or(LineSpan::WHOLE);
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

/// With `sides` (ruling R33.1), each junction holds at least one whole
/// copy of the ornament, its ink no taller than the text's x-height,
/// nothing touching the text or another copy — a copy of several
/// glyphs, one with spaces round it, one with no side bearings included
/// — and, for an ornament of one glyph and no spaces, every gap between
/// its copies and both its joins to the text within 10% of each other,
/// at span whole and at span ½.
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
    let half = LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE);
    // One-glyph ornaments, at whole and ½; then copies of several glyphs
    // (`"✦ ✦"`, `"<·>"`), one with spaces round it (`" · "`) and one with
    // no side bearings (`"─"`), at whole.
    let ornaments: [(&str, &[LineSpan]); 7] = [
        ("·", &[LineSpan::WHOLE, half]),
        ("✦", &[LineSpan::WHOLE, half]),
        ("✦ ✦", &[LineSpan::WHOLE]),
        ("<·>", &[LineSpan::WHOLE]),
        (" · ", &[LineSpan::WHOLE]),
        ("─", &[LineSpan::WHOLE]),
        ("──────", &[LineSpan::WHOLE]),
    ];
    for (ornament, spans) in ornaments {
        let per_copy = ornament.chars().filter(|c| !c.is_whitespace()).count();
        // Equal gaps and joins are owed between inks; an ornament with
        // spaces of its own keeps them between its copies.
        let one_glyph = ornament.chars().count() == 1;
        for (name, shape, text, points) in scenes() {
            for &span in spans {
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
                for (i, a) in sides.iter().enumerate() {
                    for b in &sides[i + 1..] {
                        assert!(gap(*a, *b) > 0.0, "{what}: two ornaments touch");
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
                    assert!(
                        held.len() >= per_copy,
                        "{what} {junction}: {} inks, not a whole copy",
                        held.len()
                    );
                    if !one_glyph {
                        continue;
                    }
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
/// slid clear rather than pressed — and it does: its natural end there
/// is its end at whole. A natural line that is not slid is centered on
/// its half's apex, the arc from its first ink's center to the apex and
/// from the apex to its last within half a point of each other.
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
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        ..RingLayout::whole()
    };
    let mut slid = 0;
    for (name, shape, text, points) in scenes() {
        let fitted = with_fonts(|fs| {
            let flow = style(points);
            let run = real_run(text, &flow, fs);
            let (rescaled, _) = place_lines(&run, &shape, &FIT, true, &flow, fs, None);
            size(&rescaled.unwrap_or(run))
        });
        let mut parts: Vec<Vec<(usize, f32, f32)>> = Vec::new();
        for (label, layout) in [
            ("natural", RingLayout::natural()),
            ("½", half.clone()),
            ("whole", RingLayout::whole()),
            ("whole with sides", sides.clone()),
        ] {
            let what = format!("{name} at {label}");
            let (run, placed, trace) = frame(text, points, &shape, &layout);
            parts.push(trace.lines.iter().map(|l| (l.half, l.from, l.to)).collect());
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
        // Natural against whole: a line whose natural end is its end at
        // whole was slid there; one with both ends short of whole's is
        // centered on its apex.
        let (_, apexes, _) = halves_seam(&shape);
        for (&(half, from, to), &(_, whole_from, whole_to)) in parts[0].iter().zip(&parts[2]) {
            if (to - whole_to).abs() <= 1e-2 || (from - whole_from).abs() <= 1e-2 {
                slid += 1;
            } else {
                let apex = apexes[half];
                assert!(
                    ((apex - from) - (to - apex)).abs() <= 0.5,
                    "{name} line on half {half}: {} before its apex, {} after",
                    apex - from,
                    to - apex
                );
            }
        }
        if name == "center ellipse" {
            assert!(
                (parts[0][1].2 - parts[2][1].2).abs() <= 1e-2,
                "precondition: the center's second line is slid, its natural end {} at its end at whole {}",
                parts[0][1].2,
                parts[2][1].2
            );
        }
    }
    assert!(slid >= 1, "precondition: a natural line is slid ({slid})");
}

#[test]
fn test_flow_halves_a_repeat_ring_takes_its_spacing() {
    do_flow_halves_a_repeat_ring_takes_its_spacing();
}

/// A `repeat` ring's `spacing` is added between every two of its glyph
/// steps, ink to ink (ruling R34): with 6 points of it a `"✦"` ring's
/// copies are 6 points further apart, a `"✦ "` ring's 12 (two steps a
/// copy, the space's and the star's) — each within the slack R29 shares
/// among a ring's gaps, at most one copy's length over the copies — and
/// it holds fewer of them. A spacing as large as an `f32` goes keeps
/// every position finite and lays one copy at least.
pub fn do_flow_halves_a_repeat_ring_takes_its_spacing() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(400.0, 220.0)), Shape::Ellipse);
    let ring = |pattern: &str, spacing: f32| {
        let flow = style(16.0);
        let layout = RingLayout {
            spacing,
            ..RingLayout::whole()
        };
        with_fonts(|fs| {
            let run = real_run(pattern, &flow, fs);
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
        })
    };
    let perimeter = ArcPath::closed(
        &shape,
        StartPoint {
            edge: Edge::Top,
            at: 0.0,
        },
        Direction::Clockwise,
        MIN_TABLE_SPACING,
    )
    .length();
    let measure = |pattern: &str, spacing: f32| {
        let (run, placed) = ring(pattern, spacing);
        let inks: Vec<(Vec2, Vec2)> = inks(&run, &placed).into_iter().flatten().collect();
        let mut gaps: Vec<f32> = inks.windows(2).map(|w| gap(w[0], w[1])).collect();
        gaps.push(gap(inks[inks.len() - 1], inks[0]));
        (inks.len(), mean(&gaps))
    };
    let (run, placed) = ring("✦ ", f32::MAX);
    assert!(!placed.glyphs.is_empty(), "the largest spacing lays a copy");
    for g in &placed.glyphs {
        assert!(
            g.pos.is_finite(),
            "the largest spacing places {} at {}",
            g.glyph,
            g.pos
        );
        assert!((g.glyph as usize) < run.glyphs.len());
    }
    for (pattern, steps) in [("✦", 1.0_f32), ("✦ ", 2.0)] {
        let (none_count, none_gap) = measure(pattern, 0.0);
        let (some_count, some_gap) = measure(pattern, 6.0);
        assert!(
            some_count < none_count,
            "{pattern:?}: {some_count} copies with spacing, {none_count} without"
        );
        let slack = perimeter / (some_count as f32).powi(2) + perimeter / (none_count as f32).powi(2);
        let grew = some_gap - none_gap;
        assert!(
            (grew - 6.0 * steps).abs() <= slack,
            "{pattern:?}: copies {grew} further apart with 6 of spacing, {} owed, within {slack}",
            6.0 * steps
        );
    }
}

#[test]
fn test_flow_halves_a_resolved_ring_lays_as_it_says() {
    do_flow_halves_a_resolved_ring_lays_as_it_says();
}

/// A resolved ring's span, sides and spacing are the layout it is
/// placed with (`RingLayout::of`): each field to its own, and no sides
/// to none.
pub fn do_flow_halves_a_resolved_ring_lays_as_it_says() {
    let color = Rgba::opaque(1, 2, 3);
    let ring = ResolvedRing {
        fit: FitMode::Repeat,
        span: LineSpan::new(0.25).unwrap_or(LineSpan::WHOLE),
        sides: Some(ResolvedSides {
            content: "✦".to_owned(),
            color,
        }),
        spacing: 3.0,
    };
    assert_eq!(
        RingLayout::of(&ring),
        RingLayout {
            span: LineSpan::new(0.25).unwrap_or(LineSpan::WHOLE),
            sides: Some(("✦".to_owned(), color)),
            spacing: 3.0,
        }
    );
    let bare = ResolvedRing { sides: None, ..ring };
    assert_eq!(RingLayout::of(&bare).sides, None);
}

#[test]
fn test_flow_halves_hold_on_every_aspect() {
    do_flow_halves_hold_on_every_aspect();
}

/// The lines this thread has tracked (`halves::TRACKS`, a test
/// counter): none counted in a build that is not a test's — a bench's,
/// where this tree is compiled too.
fn tracks_so_far() -> usize {
    #[cfg(test)]
    return crate::flow::place::halves::TRACKS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    0
}

/// The halves' guarantees hold from a circle to an outline fifty times
/// wider than tall, an ellipse and rectangles sharp and rounded, and on
/// a 600 × 12 rectangle with 22 pt text, taller than the outline:
/// a short text and one that fills its level span (`QUATRAIN`, which
/// `Fit` sizes to fill it, the crowded case), outward and inward, `Fit`
/// and `Once`, at span natural, ½ and whole, with sides and without
/// (outward; rulings R34, R35). No two inks touch, text or ornament; no letter
/// gap is below its line's font's own; no line's part runs backward; a
/// line at a narrower span lies within its part at whole; and at every
/// span each junction is a break, box to box (ruling R35.1): its two end
/// inks at least two word gaps apart — or, where their outward normals
/// turn more than 135° apart, lines stacked across the thin outline,
/// only not touching. What a frame's last resorts cannot hold it
/// reports in its trace rather than leave silent (ruling R35.2): only a
/// frame laid inward whose two lines' inks are together taller than the
/// outline may, and the rest must hold. Only such a frame may have a
/// line cut apart (ruling R38.1), and some here are.
pub fn do_flow_halves_hold_on_every_aspect() {
    /// The most lines a frame here may track (rulings R35.3, R38.3): the
    /// most any does is 10,534, the crowded text shrunk round a flat
    /// outline (17,031 when a line's gap was bisected).
    const MOST_TRACKS: usize = 12_000;
    let report = std::env::var_os("ZZ").is_some();
    let rect = |w: f32, h: f32| Rect::new(Vec2::ZERO, Vec2::new(w, h));
    // Each outline, its height, and the size its text is set at.
    let mut outlines: Vec<(String, Outline, f32, f32)> = Vec::new();
    for aspect in [1.0_f32, 10.0, 50.0] {
        let (w, h) = (600.0, 600.0 / aspect);
        outlines.push((
            format!("ellipse {aspect}:1"),
            outline(rect(w, h), Shape::Ellipse),
            h,
            18.0,
        ));
        for radius in [0, 50] {
            outlines.push((
                format!("rect r{radius} {aspect}:1"),
                outline(
                    rect(w, h),
                    Shape::Rectangle {
                        radius: Percent::new(radius),
                    },
                ),
                h,
                18.0,
            ));
        }
    }
    outlines.push((
        "600×12 sharp".to_string(),
        outline(
            rect(600.0, 12.0),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        12.0,
        22.0,
    ));
    let mut faults: Vec<String> = Vec::new();
    // How many frames reached a last resort: a `Fit` frame shrunk below
    // R27's size, a `Once` frame cut, and one that reports what is left
    // unheld.
    let (mut shrunk, mut cut, mut reported, mut apart) = (0, 0, 0, 0);
    // The most lines one frame here tracks: its work, bounded (ruling
    // R35.3).
    let mut most_tracks = 0;
    let size = |run: &ShapedRun| {
        run.glyphs
            .iter()
            .filter(|g| g.span != FILLER_SPAN)
            .map(|g| g.size)
            .fold(0.0_f32, f32::max)
    };
    let left = StartPoint {
        edge: Edge::Left,
        at: 0.5,
    };
    for (name, shape, height, points) in &outlines {
        // The outline's outward normal nearest `p`.
        let path = ArcPath::closed(shape, left, Direction::Clockwise, MIN_TABLE_SPACING);
        let normal = |p: Vec2| {
            let tangent = path.at(path.nearest(p)).1;
            Vec2::new(tangent.y, -tangent.x)
        };
        for text in [ECCLESIASTES, QUATRAIN] {
            let (fitted, r27_cut) = with_fonts(|fs| {
                let flow = style(*points);
                let run = real_run(text, &flow, fs);
                let (rescaled, placed) = place_lines(&run, shape, &FIT, true, &flow, fs, None);
                (size(&rescaled.unwrap_or(run)), placed.truncated)
            });
            for outward in [true, false] {
                for fit in [FIT, FitMode::Once { filler: None }] {
                    // Inward, sides round an outline its lines fill are
                    // left out (and reported); outward they are laid.
                    let sides_tried = if outward { 2 } else { 1 };
                    for sides in [None, Some(("·".to_owned(), Rgba::opaque(9, 9, 9)))]
                        .into_iter()
                        .take(sides_tried)
                    {
                        let mut whole: (Vec<(f32, f32)>, f32, bool) = (Vec::new(), 0.0, false);
                        for span in [
                            LineSpan::WHOLE,
                            LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
                            LineSpan::NATURAL,
                        ] {
                            let what = format!(
                                "{name} {:.8} {} {fit:?} span {} sides {}",
                                text,
                                if outward { "outward" } else { "inward" },
                                span.get(),
                                sides.is_some()
                            );
                            let layout = RingLayout {
                                span,
                                sides: sides.clone(),
                                ..RingLayout::whole()
                            };
                            let before = tracks_so_far();
                            let (run, placed, trace) =
                                frame_with(text, *points, shape, &fit, &layout, outward);
                            let tracks = tracks_so_far() - before;
                            most_tracks = most_tracks.max(tracks);
                            if tracks > MOST_TRACKS {
                                faults.push(format!("{what}: {tracks} lines tracked"));
                            }
                            if matches!(fit, FitMode::Fit { .. }) && size(&run) < fitted * 0.999 {
                                shrunk += 1;
                            }
                            // A `Fit` frame outward is held by its shrink:
                            // what R27 does not cut, it does not.
                            if outward && matches!(fit, FitMode::Fit { .. }) && placed.truncated && !r27_cut {
                                faults.push(format!("{what}: cut, where a shrink holds it"));
                            }
                            if matches!(fit, FitMode::Once { .. }) && placed.truncated {
                                cut += 1;
                            }
                            let inks = inks(&run, &placed);
                            // Inward, two lines whose inks are together
                            // taller than the outline cannot both lie
                            // inside it untouched: the one case a frame
                            // may report what it leaves unheld.
                            let tallest = |l: &crate::flow::place::TracedLine| {
                                l.glyphs
                                    .clone()
                                    .filter_map(|i| inks[i])
                                    .map(|(a, b)| b.y - a.y)
                                    .fold(0.0_f32, f32::max)
                            };
                            // With a line left out (ruling R38.1), the one
                            // left stands for both.
                            let tall = trace.lines.iter().map(tallest).fold(0.0_f32, f32::max);
                            let thin = !outward && 2.0 * tall > *height;
                            if !trace.cut_apart.is_empty() {
                                apart += 1;
                                if !thin {
                                    faults.push(format!("{what}: cut {:?} apart", trace.cut_apart));
                                }
                            }
                            let unheld = &trace.unheld;
                            if !unheld.is_empty() {
                                reported += 1;
                                if !thin {
                                    faults.push(format!("{what}: reports {unheld:?}"));
                                }
                            }
                            let boxes: Vec<(Vec2, Vec2)> = inks.iter().flatten().copied().collect();
                            if !unheld.overlap {
                                'touch: for (i, a) in boxes.iter().enumerate() {
                                    for b in &boxes[i + 1..] {
                                        if gap(*a, *b) <= 0.0 {
                                            faults.push(format!(
                                                "{what}: two inks touch at {:?} and {:?}",
                                                center(*a),
                                                center(*b)
                                            ));
                                            break 'touch;
                                        }
                                    }
                                }
                            }
                            let lines: Vec<LineGaps> = trace
                                .lines
                                .iter()
                                .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
                                .collect();
                            for (k, line) in trace.lines.iter().enumerate() {
                                if line.from > line.to {
                                    faults.push(format!(
                                        "{what} line {k}: its part runs backward {}..{}",
                                        line.from, line.to
                                    ));
                                }
                                let own: Vec<f32> = line
                                    .glyphs
                                    .clone()
                                    .filter(|&i| !run.glyphs[placed.glyphs[i].glyph as usize].blank)
                                    .filter_map(|i| {
                                        inks[i].map(|(a, b)| {
                                            run.glyphs[placed.glyphs[i].glyph as usize].advance - (b.x - a.x)
                                        })
                                    })
                                    .collect();
                                let own = mean(&own);
                                let (least, _) = spread(&lines[k].letters);
                                // The pairwise rule sets each gap to within
                                // `INK_TOLERANCE` of the line's, and the
                                // line's is solved to a ten-thousandth.
                                if !lines[k].letters.is_empty()
                                    && least < own - INK_TOLERANCE - 2e-4
                                    && !unheld.pressed.contains(&line.half)
                                {
                                    faults.push(format!(
                                        "{what} line {k}: letters {least} apart, its font's gap {own}"
                                    ));
                                }
                            }
                            let parts: Vec<(f32, f32)> = trace.lines.iter().map(|l| (l.from, l.to)).collect();
                            // Held against whole only at whole's size and
                            // cut: a last resort can end elsewhere.
                            if span == LineSpan::WHOLE {
                                whole = (parts, size(&run), placed.truncated);
                            } else if parts.len() == whole.0.len()
                                && size(&run) == whole.1
                                && placed.truncated == whole.2
                            {
                                for (k, (part, at_whole)) in parts.iter().zip(&whole.0).enumerate() {
                                    if part.0 < at_whole.0 - 1e-3 || part.1 > at_whole.1 + 1e-3 {
                                        faults.push(format!(
                                            "{what} line {k}: its part {part:?} passes its part at whole {at_whole:?}"
                                        ));
                                    }
                                }
                            }
                            if let [one, two] = lines.as_slice() {
                                let words: Vec<f32> = [one, two]
                                    .iter()
                                    .filter(|l| !l.words.is_empty())
                                    .map(|l| mean(&l.words))
                                    .collect();
                                let word = if words.is_empty() { 0.0 } else { mean(&words) };
                                for (j, (junction, a, b)) in [
                                    ("top-left", one.ends.0, two.ends.0),
                                    ("bottom-right", one.ends.1, two.ends.1),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    if unheld.junctions.contains(&j) {
                                        continue;
                                    }
                                    let apart = gap(a, b);
                                    let turn = normal(center(a)).dot(normal(center(b)));
                                    // Stacked past 135°: only not touching.
                                    // Between 130° and 135°, where the
                                    // engine's ink and this projection may
                                    // differ, not touching is enough too.
                                    let owed = if turn < (130.0_f32).to_radians().cos() {
                                        0.0
                                    } else {
                                        2.0 * word - 1e-2
                                    };
                                    if apart <= 0.0 || apart < owed {
                                        faults.push(format!(
                                            "{what} {junction}: ends {apart} apart, owed {owed} (turn {:.0}°)",
                                            turn.clamp(-1.0, 1.0).acos().to_degrees()
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if report {
        eprintln!(
            "ZZSWEEP shrunk {shrunk} cut {cut} reported {reported} apart {apart} most tracks {most_tracks}\n{}",
            faults.join("\n")
        );
    }
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
    // The sweep reaches the last resorts it holds to account: the
    // crowded text's frames are shrunk.
    assert!(shrunk > 0, "precondition: a frame here is shrunk");
    // And the inward frames round outlines too thin for their two lines
    // are cut apart (ruling R38.1).
    assert!(apart > 0, "precondition: a frame here is cut apart");
}

#[test]
fn test_flow_halves_a_crowded_frame_shrinks_or_cuts() {
    do_flow_halves_a_crowded_frame_shrinks_or_cuts();
}

/// A frame whose halves less their clearance cannot hold its lines at
/// their font's gap — a long side ornament takes most of the rounded
/// rectangle's halves — reaches its last resort, and every letter gap
/// stays at least the font's own: a `Fit` frame is shrunk below the size
/// R27 gives it, within its bounds, and nothing is cut; a `Once` frame
/// keeps its size and is cut, and says so.
pub fn do_flow_halves_a_crowded_frame_shrinks_or_cuts() {
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(260.0, 170.0)),
        Shape::Rectangle {
            radius: Percent::new(60),
        },
    );
    let size = |run: &ShapedRun| {
        run.glyphs
            .iter()
            .filter(|g| g.span != FILLER_SPAN)
            .map(|g| g.size)
            .fold(0.0_f32, f32::max)
    };
    let layout = RingLayout {
        sides: Some((
            "✦ · ✦ · ✦ · ✦ · ✦ · ✦ · ✦ · ✦ · ✦ · ✦ · ✦".to_owned(),
            Rgba::opaque(9, 9, 9),
        )),
        ..RingLayout::whole()
    };
    let fitted = with_fonts(|fs| {
        let flow = style(22.0);
        let run = real_run(ROUNDED, &flow, fs);
        let (rescaled, _) = place_lines(&run, &shape, &FIT, true, &flow, fs, None);
        size(&rescaled.unwrap_or(run))
    });
    for fit in [FIT, FitMode::Once { filler: None }] {
        let (run, placed, trace) = frame_with(ROUNDED, 22.0, &shape, &fit, &layout, true);
        let inks = inks(&run, &placed);
        match fit {
            FitMode::Fit { .. } => {
                assert!(
                    size(&run) < fitted * 0.999,
                    "{fit:?}: shrunk from {fitted} to {}",
                    size(&run)
                );
                assert!(size(&run) >= 0.5 * 22.0 - 1e-3, "{fit:?}: within its bounds");
                assert!(!placed.truncated, "{fit:?}: nothing cut");
            }
            _ => {
                assert!((size(&run) - 22.0).abs() < 1e-3, "{fit:?}: its own size");
                assert!(placed.truncated, "{fit:?}: cut, and says so");
            }
        }
        for (k, line) in trace.lines.iter().enumerate() {
            let own: Vec<f32> = line
                .glyphs
                .clone()
                .filter(|&i| !run.glyphs[placed.glyphs[i].glyph as usize].blank)
                .filter_map(|i| {
                    inks[i].map(|(a, b)| run.glyphs[placed.glyphs[i].glyph as usize].advance - (b.x - a.x))
                })
                .collect();
            let (least, _) = spread(&line_gaps(&run, &placed, &inks, line.glyphs.clone()).letters);
            assert!(
                least >= mean(&own) - 1e-3,
                "{fit:?} line {k}: letters {least} apart, the font's {}",
                mean(&own)
            );
        }
    }
}

#[test]
fn test_flow_halves_span_half_is_the_mean_of_natural_and_whole() {
    do_flow_halves_span_half_is_the_mean_of_natural_and_whole();
}

/// Span ½ lays each end halfway between its natural place and its place
/// at whole (ruling R33): by arc length, the part's ends — where along
/// the path the end inks are set — exactly so; and each end ink's
/// center, projected onto the outline, between its natural and whole
/// projections and within a twentieth of the way between them, plus
/// `SLACK`, of their mean (an ink is pushed off the path along a
/// chord, not the outline's normal, so its projection is not quite its
/// place). Precondition: each scene's clearance is not nothing — a
/// line's end at whole is short of its junction — so the ½ end is not
/// also halfway to the junction.
pub fn do_flow_halves_span_half_is_the_mean_of_natural_and_whole() {
    for (name, shape, text, points) in scenes() {
        let (junctions, _, length) = halves_seam(&shape);
        let left = StartPoint {
            edge: Edge::Left,
            at: 0.5,
        };
        let paths = [
            ArcPath::closed(&shape, left, Direction::Clockwise, MIN_TABLE_SPACING),
            ArcPath::closed(&shape, left, Direction::CounterClockwise, MIN_TABLE_SPACING),
        ];
        let at = |span: LineSpan| {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) = frame(text, points, &shape, &layout);
            let inks = inks(&run, &placed);
            trace
                .lines
                .iter()
                .map(|l| {
                    let g = line_gaps(&run, &placed, &inks, l.glyphs.clone());
                    let along = |p: Vec2| {
                        let start = junctions[l.half].0;
                        start + (paths[l.half].nearest(p) - start).rem_euclid(length)
                    };
                    (
                        l.half,
                        (l.from, l.to),
                        (along(center(g.ends.0)), along(center(g.ends.1))),
                    )
                })
                .collect::<Vec<_>>()
        };
        let (natural, half, whole) = (
            at(LineSpan::NATURAL),
            at(LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE)),
            at(LineSpan::WHOLE),
        );
        for ((n, h), w) in natural.iter().zip(&half).zip(&whole) {
            let what = format!("{name} half {}", n.0);
            let (ja, jb) = junctions[w.0];
            assert!(
                w.1 .0 > ja + 1.0 && w.1 .1 < jb - 1.0,
                "precondition, {what}: a clearance at whole {:?} within {ja}..{jb}",
                w.1
            );
            for (end, (nat, mid, all)) in [
                ("start", (n.1 .0, h.1 .0, w.1 .0)),
                ("end", (n.1 .1, h.1 .1, w.1 .1)),
            ] {
                assert!(
                    (mid - 0.5 * (nat + all)).abs() <= 1e-2,
                    "{what} {end}: its part's end at {mid}, natural {nat}, whole {all}"
                );
            }
            for (end, (nat, mid, all)) in [
                ("first ink", (n.2 .0, h.2 .0, w.2 .0)),
                ("last ink", (n.2 .1, h.2 .1, w.2 .1)),
            ] {
                let (low, high) = (nat.min(all), nat.max(all));
                assert!(
                    mid >= low - SLACK
                        && mid <= high + SLACK
                        && (mid - 0.5 * (nat + all)).abs() <= 0.05 * (high - low) + SLACK,
                    "{what} {end}: at {mid}, natural {nat}, whole {all}"
                );
            }
        }
    }
}

#[test]
fn test_flow_halves_a_line_keeps_its_half() {
    do_flow_halves_a_line_keeps_its_half();
}

/// A text that opens with a line break has an empty first line, and its
/// one line takes the second half (ruling R27's parting): the trace says
/// half 1, and every ink lies below the chord from the top-left junction
/// to the bottom-right one, on the lower-left side. Control: the same
/// text without the break takes the first half.
pub fn do_flow_halves_a_line_keeps_its_half() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(820.0, 330.0)), Shape::Ellipse);
    let (tl, br) = (
        shape.support(Vec2::new(-1.0, -1.0)),
        shape.support(Vec2::new(1.0, 1.0)),
    );
    let below = |p: Vec2| {
        let d = br - tl;
        d.x * (p.y - tl.y) - d.y * (p.x - tl.x) > 0.0
    };
    for (fit, (text, half)) in [FIT, FitMode::Once { filler: None }]
        .into_iter()
        .flat_map(|fit| [(fit.clone(), ("\nheavens", 1)), (fit, ("heavens", 0))])
    {
        let text: &str = text;
        let (run, placed, trace) = frame_with(text, 38.0, &shape, &fit, &RingLayout::whole(), true);
        assert_eq!(trace.lines.len(), 1, "{text:?}: one line");
        assert_eq!(trace.lines[0].half, half, "{text:?}: its half");
        let inks = inks(&run, &placed);
        let on_lower = inks.iter().flatten().filter(|b| below(center(**b))).count();
        let all = inks.iter().flatten().count();
        assert_eq!(
            on_lower,
            if half == 1 { all } else { 0 },
            "{text:?}: {on_lower} of {all} inks lower-left"
        );
    }
}

#[test]
fn test_flow_halves_a_glyph_with_no_outline_is_laid() {
    do_flow_halves_a_glyph_with_no_outline_is_laid();
}

/// A glyph with no outline to measure — a color emoji — is laid with
/// its line, stepped as a box: every glyph of the frame's text but the
/// whitespace the lines part at is placed, nothing cut, and its box, its
/// advance by the line height, touches no letter's ink. So is a line of
/// nothing but such glyphs, which has no letter to size its tracking
/// by but its boxes. Precondition: the text holds a glyph that is not a
/// space and has no outline ink.
pub fn do_flow_halves_a_glyph_with_no_outline_is_laid() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(820.0, 330.0)), Shape::Ellipse);
    for text in [
        "a time \u{1F338} to every purpose \u{1F338} under the heaven",
        "a time to every purpose under the heaven\n\u{1F338}\u{1F338}\u{1F338}",
    ] {
        laid_without_outlines(text, &shape);
    }
}

fn laid_without_outlines(text: &str, shape: &Outline) {
    let (run, placed, trace) = frame_with(
        text,
        30.0,
        shape,
        &FitMode::Once { filler: None },
        &RingLayout::whole(),
        true,
    );
    let every = PlacedRun {
        glyphs: (0..run.glyphs.len())
            .map(|i| PlacedGlyph {
                glyph: i as u32,
                pos: Vec2::ZERO,
            })
            .collect(),
        truncated: false,
    };
    let outline_less = inks(&run, &every)
        .iter()
        .enumerate()
        .filter(|(i, ink)| ink.is_none() && !run.glyphs[*i].blank)
        .count();
    assert!(
        outline_less > 0,
        "precondition: {text:?}: the text holds a glyph with no outline ink"
    );
    let inks = inks(&run, &placed);
    assert!(!placed.truncated, "{text:?}: nothing cut");
    let parted: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
    let drawing = run.glyphs.iter().filter(|g| !g.blank).count();
    let laid_drawing = placed
        .glyphs
        .iter()
        .filter(|g| !run.glyphs[g.glyph as usize].blank)
        .count();
    assert_eq!(
        laid_drawing, drawing,
        "{text:?}: every glyph that draws is laid ({parted} in the lines)"
    );
    // Stepped as its box: an outline-less glyph's box, its advance by the
    // line height, touches no letter's ink.
    for (k, g) in placed.glyphs.iter().enumerate() {
        let metrics = &run.glyphs[g.glyph as usize];
        if inks[k].is_some() || metrics.blank {
            continue;
        }
        let boxed = (g.pos, g.pos + Vec2::new(metrics.advance, run.line_height));
        for ink in inks.iter().flatten() {
            assert!(
                gap(boxed, *ink) > 0.0,
                "{text:?}: glyph {} overlaps a letter's ink",
                g.glyph
            );
        }
    }
}

#[test]
fn test_flow_halves_spread_and_once_and_inward() {
    do_flow_halves_spread_and_once_and_inward();
}

/// Through `place_on_closed`: a `Spread` frame is laid as a `Once` one,
/// glyph for glyph; a `Once` frame with a filler draws no filler glyph,
/// its text where it is without one; and an inward ring tracks evenly
/// and reads down as an outward one does (R32, R33).
pub fn do_flow_halves_spread_and_once_and_inward() {
    let once = FitMode::Once { filler: None };
    for (name, shape, text, points) in scenes() {
        let (_, plain, _) = frame_with(text, points, &shape, &once, &RingLayout::whole(), true);
        let (_, spreads, _) = frame_with(text, points, &shape, &FitMode::Spread, &RingLayout::whole(), true);
        assert_eq!(spreads.glyphs, plain.glyphs, "{name}: Spread lays as Once");
        let filler = FitMode::Once {
            filler: Some("·".to_owned()),
        };
        let (run, filled, _) = frame_with(text, points, &shape, &filler, &RingLayout::whole(), true);
        assert!(
            filled
                .glyphs
                .iter()
                .all(|g| run.glyphs[g.glyph as usize].span != FILLER_SPAN),
            "{name}: no filler glyph drawn"
        );
        assert_eq!(
            filled.glyphs, plain.glyphs,
            "{name}: the text where it is without a filler"
        );
        let (run, placed, trace) = frame_with(text, points, &shape, &once, &RingLayout::whole(), false);
        let inks = inks(&run, &placed);
        for (k, line) in trace.lines.iter().enumerate() {
            let gaps = line_gaps(&run, &placed, &inks, line.glyphs.clone());
            let (least, most) = spread(&gaps.letters);
            assert!(
                least > 0.0 && most <= EVEN * least,
                "{name} inward line {k}: letter gaps {least} to {most}"
            );
            let mut run_dy: Vec<f32> = Vec::new();
            for w in gaps.centers.windows(2) {
                let d = w[1] - w[0];
                if d.y.abs() > d.x.abs() {
                    run_dy.push(d.y);
                } else {
                    if run_dy.len() >= 2 {
                        assert!(
                            run_dy.iter().all(|&dy| dy > 0.0),
                            "{name} inward line {k}: a steep run goes up"
                        );
                    }
                    run_dy.clear();
                }
            }
        }
    }
}

#[test]
fn test_flow_halves_once_holds_what_r27_cuts() {
    do_flow_halves_once_holds_what_r27_cuts();
}

/// A `Once` frame's text that R27 cuts at its level span, but that its
/// halves hold at its font's gap, is laid whole (ruling R34): no glyph
/// that draws left out, nothing reported cut, every letter gap at least
/// the font's own. Precondition: R27 cuts it.
pub fn do_flow_halves_once_holds_what_r27_cuts() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(300.0, 300.0)), Shape::Ellipse);
    let once = FitMode::Once { filler: None };
    let r27 = with_fonts(|fs| {
        let flow = style(22.0);
        let run = real_run(ECCLESIASTES, &flow, fs);
        place_lines(&run, &shape, &once, true, &flow, fs, None)
            .1
            .truncated
    });
    assert!(r27, "precondition: R27 cuts the text at its level span");
    let (run, placed, trace) = frame_with(ECCLESIASTES, 22.0, &shape, &once, &RingLayout::whole(), true);
    assert!(!placed.truncated, "nothing cut");
    let drawing = run.glyphs.iter().filter(|g| !g.blank).count();
    let laid = placed
        .glyphs
        .iter()
        .filter(|g| !run.glyphs[g.glyph as usize].blank)
        .count();
    assert_eq!(laid, drawing, "every glyph that draws is laid");
    let inks = inks(&run, &placed);
    for (k, line) in trace.lines.iter().enumerate() {
        let own: Vec<f32> = line
            .glyphs
            .clone()
            .filter(|&i| !run.glyphs[placed.glyphs[i].glyph as usize].blank)
            .filter_map(|i| {
                inks[i].map(|(a, b)| run.glyphs[placed.glyphs[i].glyph as usize].advance - (b.x - a.x))
            })
            .collect();
        let (least, _) = spread(&line_gaps(&run, &placed, &inks, line.glyphs.clone()).letters);
        assert!(
            least >= mean(&own) - 1e-3,
            "line {k}: letters {least} apart, the font's {}",
            mean(&own)
        );
    }
}

#[test]
fn test_flow_halves_a_junction_holds_at_most_its_copies() {
    do_flow_halves_a_junction_holds_at_most_its_copies();
}

/// A junction holds at most `MAX_SIDE_COPIES` (256) copies of its
/// ornament: a short text at span natural on a large ellipse leaves arcs
/// that would take many more. Precondition: one junction reaches the
/// cap.
pub fn do_flow_halves_a_junction_holds_at_most_its_copies() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(6000.0, 3000.0)), Shape::Ellipse);
    let layout = RingLayout {
        span: LineSpan::NATURAL,
        sides: Some(("·".to_owned(), Rgba::opaque(9, 9, 9))),
        ..RingLayout::whole()
    };
    let (run, placed, _) = frame_with(
        "a time and a time",
        8.0,
        &shape,
        &FitMode::Once { filler: None },
        &layout,
        true,
    );
    let copies = placed
        .glyphs
        .iter()
        .filter(|g| run.glyphs[g.glyph as usize].span == FILLER_SPAN)
        .count();
    assert!(copies <= 2 * 256, "{copies} copies round two junctions");
    assert!(
        copies >= 256,
        "precondition: a junction reaches the cap ({copies} copies)"
    );
}

#[test]
fn test_flow_halves_a_part_never_inverts() {
    do_flow_halves_a_part_never_inverts();
}

/// A line's part of its half with a clearance kept at each junction
/// (ruling R34): the half less both clearances while they leave any of
/// it, and where they divide the half in their ratio once together they
/// pass it — never inverted, never off the half.
pub fn do_flow_halves_a_part_never_inverts() {
    let rect = Rect::new(Vec2::ZERO, Vec2::new(400.0, 160.0));
    for shape in [
        Shape::Ellipse,
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    ] {
        let shape = outline(rect, shape);
        for half in 0..2 {
            let ((ja, jb), _) = room_seam(&shape, half, [0.0; 2]);
            let len = jb - ja;
            let cases = [
                ([0.25 * len, 0.25 * len], (ja + 0.25 * len, jb - 0.25 * len)),
                (
                    [0.7 * len, 0.6 * len],
                    (ja + len * 0.7 / 1.3, ja + len * 0.7 / 1.3),
                ),
                ([2.0 * len, 0.0], (jb, jb)),
                ([0.0, 3.0 * len], (ja, ja)),
            ];
            for (clear, (lo, hi)) in cases {
                let ((a, b), junctions) = room_seam(&shape, half, clear);
                assert_eq!(junctions, (ja, jb), "the half's junctions");
                assert!(
                    a <= b && ja <= a && b <= jb,
                    "half {half} {clear:?}: {a}..{b} of {ja}..{jb}"
                );
                assert!(
                    (a - lo).abs() <= 1e-3 && (b - hi).abs() <= 1e-3,
                    "half {half} {clear:?}: {a}..{b}, owed {lo}..{hi}"
                );
            }
        }
    }
}

#[test]
fn test_flow_halves_a_copy_at_its_floor_clears_its_neighbors() {
    do_flow_halves_a_copy_at_its_floor_clears_its_neighbors();
}

/// A side ornament's copies laid at their least gap (ruling R33.1) are
/// set by the pairwise ink rule at that gap: a one-glyph ornament's
/// copies each that floor from the ink before, within the rule's
/// tolerance and a tenth, and every ink clear of the one before by more
/// than the tolerance, so no rounding of the rule lets two touch — an
/// ornament with no side bearings, whose floor is all that keeps it
/// under the rule rather than where its box falls, included.
/// Precondition: `"─"` has no side bearings, its ink as wide as its
/// advance or wider.
pub fn do_flow_halves_a_copy_at_its_floor_clears_its_neighbors() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(820.0, 330.0)), Shape::Ellipse);
    for ornament in ["─", "──────", "·", "✦ ✦"] {
        let copies = 4;
        let laid = with_fonts(|fs| {
            let flow = style(38.0);
            let run = real_run(ECCLESIASTES, &flow, fs);
            side_floor_seam(&run, &shape, ornament, copies, &flow, fs)
        });
        let Some((run, glyphs, floor)) = laid else {
            panic!("{ornament:?}: no copy laid");
        };
        let placed = PlacedRun {
            glyphs,
            truncated: false,
        };
        let inks = inks(&run, &placed);
        let per_copy = ornament.chars().filter(|c| !c.is_whitespace()).count();
        let drawn: Vec<(usize, (Vec2, Vec2))> = placed
            .glyphs
            .iter()
            .zip(&inks)
            .filter_map(|(g, ink)| Some((g.glyph as usize, (*ink)?)))
            .collect();
        assert_eq!(
            drawn.len(),
            copies * per_copy,
            "precondition: {ornament:?}: every copy laid"
        );
        if ornament == "─" {
            let (i, (a, b)) = drawn[0];
            let bearings = run.glyphs[i].advance - (b.x - a.x);
            assert!(
                bearings < INK_TOLERANCE,
                "precondition: \"─\" has side bearings of {bearings}, which would hold its copies apart"
            );
        }
        let one_glyph = ornament.chars().count() == 1;
        for w in drawn.windows(2) {
            let apart = gap(w[0].1, w[1].1);
            assert!(
                apart > INK_TOLERANCE,
                "{ornament:?}: at its floor {floor}, two inks {apart} apart"
            );
            if one_glyph {
                assert!(
                    (apart - floor).abs() <= 0.1 * floor + INK_TOLERANCE,
                    "{ornament:?}: two copies {apart} apart, the floor {floor}"
                );
            }
        }
    }
}

#[test]
fn test_flow_halves_the_break_holds_at_the_span_laid() {
    do_flow_halves_the_break_holds_at_the_span_laid();
}

/// The phase-2 review's table on #294 (ruling R35.1): the Ecclesiastes
/// text, `Fit`, outward, round a 600 × 12 ellipse at 18 pt and a 600 × 12
/// sharp rectangle at 22 pt, at span ½ and 0.9, the rectangle at natural
/// too, a 600 × 60 ellipse at 0.9, and the README's 820 × 330 center
/// ellipse at ½. The review measured the bottom-right break, box to box,
/// at 8.78 and 12.00 points at ½ against some 41 owed, because the
/// clearance was found at whole and only kept below it. Now it is held
/// at the span laid: at each junction the two end inks are at least two
/// word gaps apart, or — where the outline's outward normals under them
/// turn more than 135° apart, lines stacked across it — do not touch.
/// Held at ½, the bottom-right ends there are drawn back until they are
/// stacked across the outline (17.9 and 18.6 points apart, not touching),
/// at 0.9 as well, and on the 10:1 ellipse at 0.9 (32 points apart);
/// on the center ellipse they meet, two word gaps apart.
/// Precondition: the table holds both a meeting and lines stacked.
/// Control: the clearance kept from whole below it, the ½ rows fail.
pub fn do_flow_halves_the_break_holds_at_the_span_laid() {
    let rect = |w: f32, h: f32| Rect::new(Vec2::ZERO, Vec2::new(w, h));
    let ellipse = outline(rect(600.0, 12.0), Shape::Ellipse);
    let ten = outline(rect(600.0, 60.0), Shape::Ellipse);
    let readme = outline(rect(820.0, 330.0), Shape::Ellipse);
    let sharp = outline(
        rect(600.0, 12.0),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let left = StartPoint {
        edge: Edge::Left,
        at: 0.5,
    };
    let span = |s: f32| LineSpan::new(s).unwrap_or(LineSpan::WHOLE);
    // Whether a junction here meets, and whether one is stacked.
    let mut kinds = [false, false];
    for (name, shape, points, spans) in [
        ("ellipse 50:1", &ellipse, 18.0, vec![span(0.5), span(0.9)]),
        ("ellipse 10:1", &ten, 18.0, vec![span(0.9)]),
        ("the center ellipse", &readme, 38.0, vec![span(0.5)]),
        (
            "600×12",
            &sharp,
            22.0,
            vec![span(0.5), span(0.9), LineSpan::NATURAL],
        ),
    ] {
        let path = ArcPath::closed(shape, left, Direction::Clockwise, MIN_TABLE_SPACING);
        let normal = |p: Vec2| {
            let tangent = path.at(path.nearest(p)).1;
            Vec2::new(tangent.y, -tangent.x)
        };
        for span in spans {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) = frame_with(ECCLESIASTES, points, shape, &FIT, &layout, true);
            let inks = inks(&run, &placed);
            let lines: Vec<LineGaps> = trace
                .lines
                .iter()
                .map(|l| line_gaps(&run, &placed, &inks, l.glyphs.clone()))
                .collect();
            assert_eq!(
                lines.len(),
                2,
                "precondition: {name} span {}: two lines",
                span.get()
            );
            assert!(
                trace.unheld.is_empty(),
                "{name} span {}: {:?}",
                span.get(),
                trace.unheld
            );
            let word = word_gap(&lines);
            for (junction, a, b) in [
                ("top-left", lines[0].ends.0, lines[1].ends.0),
                ("bottom-right", lines[0].ends.1, lines[1].ends.1),
            ] {
                let apart = gap(a, b);
                let turn = normal(center(a))
                    .dot(normal(center(b)))
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                let what = format!(
                    "{name} span {} {junction}: ends {apart} apart, turn {turn:.0}°, word gap {word}",
                    span.get()
                );
                if turn > 135.0 {
                    assert!(apart > 0.0, "{what}: stacked lines touch");
                    kinds[1] = true;
                } else {
                    assert!(apart >= 2.0 * word - 1e-2, "{what}: no break");
                    kinds[0] = true;
                }
            }
        }
    }
    assert!(
        kinds == [true, true],
        "precondition: the table holds both a meeting and lines stacked ({kinds:?})"
    );
}

#[test]
fn test_flow_halves_an_outline_whose_halves_are_not_numbers_keeps_r27s_lines() {
    do_flow_halves_an_outline_whose_halves_are_not_numbers_keeps_r27s_lines();
}

/// An outline whose halves cannot be measured keeps R27's placement
/// (ruling R34): a rounded rectangle 8.9e37 on a side — the size
/// `place_on_closed` shrinks the far outline of the flow_structured
/// finding `spread_gap_past_the_largest_f32` to — has a length, 2.8e38,
/// but its bottom-right junction, a turn past its top-left one, is past
/// every `f32`. Its frame's lines are where `place_lines` lays them,
/// glyph for glyph, every position finite. Precondition: a junction is
/// not finite, and R27 places glyphs there. Control: without the guard
/// the halves are laid from that junction, and the glyphs differ.
pub fn do_flow_halves_an_outline_whose_halves_are_not_numbers_keeps_r27s_lines() {
    let shape = Outline::RoundedRect {
        rect: Rect::new(Vec2::splat(-4.4540537e37), Vec2::splat(8.908107e37)),
        radius: 4.4540537e37,
    };
    let (junctions, _, length) = halves_seam(&shape);
    assert!(
        !length.is_finite() || junctions.iter().any(|(a, b)| !a.is_finite() || !b.is_finite()),
        "precondition: the halves are not a number"
    );
    for fit in [FitMode::Spread, FitMode::Fit { min: 1.0, max: 1.0 }] {
        let (_, placed, trace) = frame_with(ECCLESIASTES, 18.0, &shape, &fit, &RingLayout::whole(), true);
        let (_, natural) = with_fonts(|fs| {
            let flow = style(18.0);
            let run = real_run(ECCLESIASTES, &flow, fs);
            place_lines(&run, &shape, &fit, true, &flow, fs, None)
        });
        assert!(
            !natural.glyphs.is_empty(),
            "precondition: {fit:?}: R27 places glyphs"
        );
        assert_eq!(placed.glyphs, natural.glyphs, "{fit:?}: R27's lines");
        assert!(
            placed.glyphs.iter().all(|g| g.pos.is_finite()),
            "{fit:?}: every position finite"
        );
        assert!(trace.unheld.is_empty(), "{fit:?}: nothing reported unheld");
    }
}

#[test]
fn test_flow_halves_a_side_that_would_touch_is_left_out() {
    do_flow_halves_a_side_that_would_touch_is_left_out();
}

/// A side ornament whose copies would touch the text, or fold back
/// across themselves round a thin tip, is left out of its break and
/// reported (ruling R35.2): the quatrain laid inward round a 600 × 12
/// ellipse with `"·"` sides, at span ½, where each tip's arc wraps the
/// tip inside an outline thinner than its two lines. No two inks touch,
/// text or ornament. Precondition: the trace reports a break left
/// without its ornament. Control: with every arc kept, copies touch.
pub fn do_flow_halves_a_side_that_would_touch_is_left_out() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)), Shape::Ellipse);
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("·".to_owned(), Rgba::opaque(9, 9, 9))),
        ..RingLayout::whole()
    };
    let (run, placed, trace) = frame_with(QUATRAIN, 18.0, &shape, &FIT, &layout, false);
    assert!(
        !trace.unheld.sides.is_empty(),
        "precondition: a break is left without its ornament ({:?})",
        trace.unheld
    );
    let boxes: Vec<(Vec2, Vec2)> = inks(&run, &placed).into_iter().flatten().collect();
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            assert!(
                gap(*a, *b) > 0.0,
                "two inks touch at {:?} and {:?}",
                center(*a),
                center(*b)
            );
        }
    }
}

#[test]
fn test_flow_halves_a_frame_its_lines_can_hold_is_not_shrunk() {
    do_flow_halves_a_frame_its_lines_can_hold_is_not_shrunk();
}

/// A frame whose lines can be held at R27's size is held there, not
/// shrunk (rulings R34, R38.1): the README quatrain, at 18 pt, `Fit`,
/// at span whole, round outlines its two lines fill. Drawn back to part
/// a junction, a line with no slack left would be pressed while the
/// other line has room to give, and a pressed line sends the frame to
/// its shrink; so each line is drawn back no further than its slack, the
/// rest handed to the other line (outward, the 10:1 and 50:1 ellipses
/// and the 50:1 pill). And each round's step is damped by how much the
/// drawing back eases the break wanted, so it does not overshoot into a
/// press the clearance never needed (inward, the 10:1 rectangle).
///
/// Control: with the slack cap removed the outward frames shrink; with
/// the step undamped the inward one does.
pub fn do_flow_halves_a_frame_its_lines_can_hold_is_not_shrunk() {
    let rect = |w: f32, h: f32| Rect::new(Vec2::ZERO, Vec2::new(w, h));
    let size = |run: &ShapedRun| {
        run.glyphs
            .iter()
            .filter(|g| g.span != FILLER_SPAN)
            .map(|g| g.size)
            .fold(0.0_f32, f32::max)
    };
    let rows = [
        ("ellipse 10:1", outline(rect(600.0, 60.0), Shape::Ellipse), true),
        ("ellipse 50:1", outline(rect(600.0, 12.0), Shape::Ellipse), true),
        (
            "pill 50:1",
            outline(
                rect(600.0, 12.0),
                Shape::Rectangle {
                    radius: Percent::new(50),
                },
            ),
            true,
        ),
        (
            "rectangle 10:1, inward",
            outline(
                rect(600.0, 60.0),
                Shape::Rectangle {
                    radius: Percent::new(0),
                },
            ),
            false,
        ),
    ];
    for (name, shape, outward) in rows {
        let sized = with_fonts(|fs| {
            let flow = style(18.0);
            let run = real_run(QUATRAIN, &flow, fs);
            let (rescaled, _) = place_lines(&run, &shape, &FIT, true, &flow, fs, None);
            size(&rescaled.unwrap_or(run))
        });
        let (run, placed, trace) = frame_with(QUATRAIN, 18.0, &shape, &FIT, &RingLayout::whole(), outward);
        assert_eq!(trace.lines.len(), 2, "precondition: {name}: two lines");
        assert!(
            !placed.truncated && trace.unheld.is_empty(),
            "{name}: held whole ({:?})",
            trace.unheld
        );
        assert!(
            (size(&run) - sized).abs() < 1e-3,
            "{name}: shrunk to {} from R27's {sized}",
            size(&run)
        );
    }
}

/// An ordinary frame's work is a few lays of each line (ruling R38.3):
/// each of the README's frames, `Fit`, at span whole and natural, tracks
/// at most 160 lines — every lay of a line solving its gap from the
/// straight path's answer, a few `track`s, and the clearance found in a
/// few rounds. When a line's gap was bisected (a bracket doubled from
/// one point, then twenty rounds) the same frames tracked 192 to 303.
///
/// Control: with the tracking bisected again, the frames pass their
/// bound.
#[test]
fn test_flow_halves_an_ordinary_frame_lays_each_line_a_few_times() {
    for (name, shape, text, points) in scenes() {
        for layout in [RingLayout::whole(), RingLayout::natural()] {
            let before = tracks_so_far();
            let (_, placed, trace) = frame(text, points, &shape, &layout);
            let tracks = tracks_so_far() - before;
            assert_eq!(trace.lines.len(), 2, "precondition: {name}: two lines");
            assert!(
                !placed.truncated && trace.unheld.is_empty(),
                "precondition: {name}: held whole"
            );
            assert!(
                tracks <= 160,
                "{name} at span {}: {tracks} lines tracked, at most 160",
                layout.span.get()
            );
        }
    }
}
