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
use crate::flow::place::halves::Looks;
use crate::flow::place::halves::{end_seam, halves_seam, room_seam, side_floor_seam, track_seam};
use crate::flow::place::{place_lines, place_on_closed_traced, Trace, INK_TOLERANCE, MIN_TABLE_SPACING};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedGlyph, PlacedRun, Rect, RingLayout, ShapedRun, FILLER_SPAN};
use crate::font::fonts::glyph_outline_ink;
use crate::font::{FontSystem, SwashCache};

/// The README's Blake quatrain.
pub(crate) const QUATRAIN: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower, \
     Hold Infinity in the palm of your hand, And Eternity in an hour. ";

/// The psalm the README sets inside it.
const PSALM: &str = "The heavens declare the glory of God; and the firmament sheweth his handywork. ";

/// The Ecclesiastes center frame.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

/// The frames scene's rounded rectangle.
const ROUNDED: &str = "A frame of text around a rounded rectangle, shaped once and placed glyph by glyph. ";

/// The README's outlines, each with the text and size it holds there.
pub(crate) fn scenes() -> Vec<(&'static str, Outline, &'static str, f32)> {
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
pub(crate) fn frame(text: &str, points: f32, outline: &Outline, layout: &RingLayout) -> (ShapedRun, PlacedRun, Trace) {
    frame_with(text, points, outline, &FIT, layout, true)
}

/// `text` at `points` laid round `outline` as a frame with `fit` and
/// `layout`, outward or inward.
pub(crate) fn frame_with(
    text: &str,
    points: f32,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
) -> (ShapedRun, PlacedRun, Trace) {
    frame_of_spans(
        &[(text.to_string(), points)],
        points,
        outline,
        fit,
        layout,
        outward,
    )
}

/// `spans`, each its text at its size in points, shaped for real as one
/// run, laid as a frame as [`frame_with`] lays a text, its sides in the
/// style of `points`.
pub(crate) fn frame_of_spans(
    spans: &[(String, f32)],
    points: f32,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
) -> (ShapedRun, PlacedRun, Trace) {
    let flow = style(points);
    let mut trace = Trace::default();
    let (rescaled, placed) = with_fonts(|fs| {
        let run = spans_run(spans, fs);
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

/// `spans`, each its text at its size in points, shaped for real as one
/// run in [`FACE`](crate::flow::tests::support::FACE), at scale 1.
pub(crate) fn spans_run(
    spans: &[(String, f32)],
    fs: &mut crate::font::fonts::FontSystemWriteGuard,
) -> ShapedRun {
    let styles: Vec<crate::document::resolve::ResolvedText> =
        spans.iter().map(|(_, points)| style(*points)).collect();
    let flow: Vec<crate::flow::FlowSpan<'_>> = spans
        .iter()
        .zip(&styles)
        .map(|((text, _), style)| crate::flow::FlowSpan {
            text,
            style,
            link: None,
        })
        .collect();
    crate::flow::shape_run(&flow, 1.0, fs)
}

/// The reviewer's mixed-size text (review 5751828253), `copies` times
/// over: a span of `dots` `.`s at 0.5 pt, then an `x` in each of 14 spans
/// at 0.5, 1, 2, … 4,096 pt, the whole font-size window.
#[cfg(test)]
pub(crate) fn mixed_sizes(copies: usize, dots: usize) -> Vec<(String, f32)> {
    (0..copies)
        .flat_map(|_| {
            std::iter::once((".".repeat(dots), 0.5))
                .chain((0..14).map(|k| ("x".to_string(), 0.5 * (1u32 << k) as f32)))
        })
        .collect()
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

/// The x-height of [`style`]'s face a point of its size (#308): the ink
/// of an `x` shaped alone in it above the baseline, over its size —
/// measured here through the shaper and the outline, not read from the
/// engine's band, so a band the engine reads wrong does not agree with
/// itself (TEST_CONVENTIONS §T13, shape 6).
pub(crate) fn x_height_per_point() -> f32 {
    static X: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *X.get_or_init(|| {
        with_fonts(|fs| {
            let flow = style(16.0);
            let run = real_run("x", &flow, fs);
            let fs: &mut FontSystem = fs;
            let mut cache = SwashCache::new();
            let x = run.layout_glyphs().next().cloned().expect("an x is shaped");
            let (min, _) = glyph_outline_ink(fs, &mut cache, &x).expect("an x has ink");
            -min.y / x.font_size
        })
    })
}

/// Where glyph `k` of `placed` is anchored on its frame's band curve
/// (#308, ruling B15.1), its ink `ink` on the canvas: its ink's center
/// along the line and, across it, halfway between its baseline and its
/// x-height ([`x_height_per_point`], at its size) — or, a side
/// ornament's glyph, its ink's center.
pub(crate) fn band_anchor(run: &ShapedRun, placed: &PlacedRun, k: usize, ink: (Vec2, Vec2)) -> Vec2 {
    let glyph = &run.glyphs[placed.glyphs[k].glyph as usize];
    if glyph.span == FILLER_SPAN {
        return center(ink);
    }
    let baseline = placed.glyphs[k].pos.y + run.baseline();
    Vec2::new(center(ink).x, baseline - 0.5 * x_height_per_point() * glyph.size)
}

/// One line's letter gaps (between neighboring inks of a word) and word
/// gaps (across a space), and its first and last ink, with their anchors
/// on the frame's band curve ([`band_anchor`], #308).
struct LineGaps {
    letters: Vec<f32>,
    words: Vec<f32>,
    ends: ((Vec2, Vec2), (Vec2, Vec2)),
    anchors: (Vec2, Vec2),
    centers: Vec<Vec2>,
}

/// The outline's outward normal under the anchor `p` of an ink (#308),
/// `path` walked clockwise: the normal at the point of the outline
/// nearest `p` — the way from that point to a point of the frame's band
/// curve, which lies along the normal of the point it is offset from
/// — or, where `p` lies off the outline in no normal of it there, a
/// degree off or more, the way from that point to `p`: past a sharp
/// corner, where the nearest point is the corner and the outline has no
/// one normal, the way from the corner, turned outward for a point
/// inside the outline.
fn normal_under(path: &ArcPath, p: Vec2) -> Vec2 {
    let (q, tangent) = path.at(path.nearest(p));
    let along = Vec2::new(tangent.y, -tangent.x);
    let off = p - q;
    let way = off.normalize_or_zero() * off.dot(along).signum();
    if off.length() > 0.5 && way.dot(along) < 1.0_f32.to_radians().cos() {
        way
    } else {
        along
    }
}

/// How far, as a cosine, the outline's outward normals under the
/// anchors `a` and `b` of two end inks turn apart (#308): at the most,
/// over the normals within a hundredth of a point along the outline of
/// where each lies ([`normal_under`]). An anchor on the outline at a
/// sharp corner — the band curve at no offset, inward, where a line's
/// last letter can come to rest on the corner — lies where the outline
/// has no one normal, and the engine reads it as facing whichever side
/// it is set on, which a float's error in its position here would turn
/// the other way.
fn turn_cos(path: &ArcPath, a: Vec2, b: Vec2) -> f32 {
    let near = |p: Vec2| {
        let s = path.nearest(p);
        let side = |s: f32| path.outward(path.at(s).1);
        [normal_under(path, p), side(s - 1e-2), side(s + 1e-2)]
    };
    let (na, nb) = (near(a), near(b));
    na.iter()
        .flat_map(|u| nb.iter().map(move |v| u.dot(*v)))
        .fold(f32::INFINITY, f32::min)
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
    let mut anchors: Option<(Vec2, Vec2)> = None;
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
        let anchor = band_anchor(run, placed, k, ink);
        anchors = Some((anchors.map_or(anchor, |(a, _)| a), anchor));
        centers.push(center(ink));
        previous = Some(ink);
        spaced = false;
    }
    let first = first.expect("a line with ink");
    LineGaps {
        letters,
        words,
        ends: (first, previous.expect("a line with ink")),
        anchors: anchors.expect("a line with ink"),
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
    // (`"¤ ¤"`, `"<·>"`), one with spaces round it (`" · "`) and one with
    // no side bearings (`"_"`), at whole: each a glyph the compiled faces
    // hold (§T14; `✦` and `─` once, which no compiled face inks).
    let ornaments: [(&str, &[LineSpan]); 7] = [
        ("·", &[LineSpan::WHOLE, half]),
        ("¤", &[LineSpan::WHOLE, half]),
        ("¤ ¤", &[LineSpan::WHOLE]),
        ("<·>", &[LineSpan::WHOLE]),
        (" · ", &[LineSpan::WHOLE]),
        ("_", &[LineSpan::WHOLE]),
        ("______", &[LineSpan::WHOLE]),
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
        sides: Some(("¤".to_owned(), Rgba::opaque(200, 200, 200))),
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
/// steps, ink to ink (ruling R34): with 6 points of it a `"¤"` ring's
/// copies are 6 points further apart, a `"¤ "` ring's 12 (two steps a
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
        // A ring of a glyph with no outline has no ink to measure its gaps
        // by (`✦` on the compiled faces alone, which have no such glyph).
        let (Some(&first), Some(&last)) = (inks.first(), inks.last()) else {
            panic!(
                "precondition: a {pattern:?} ring at spacing {spacing} lays inked copies ({} glyphs, none inked)",
                placed.glyphs.len()
            );
        };
        let mut gaps: Vec<f32> = inks.windows(2).map(|w| gap(w[0], w[1])).collect();
        gaps.push(gap(last, first));
        (inks.len(), mean(&gaps))
    };
    let (run, placed) = ring("¤ ", f32::MAX);
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
    for (pattern, steps) in [("¤", 1.0_f32), ("¤ ", 2.0)] {
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
            content: "¤".to_owned(),
            color,
        }),
        spacing: 3.0,
    };
    assert_eq!(
        RingLayout::of(&ring),
        RingLayout {
            span: LineSpan::new(0.25).unwrap_or(LineSpan::WHOLE),
            sides: Some(("¤".to_owned(), color)),
            spacing: 3.0,
        }
    );
    let bare = ResolvedRing { sides: None, ..ring };
    assert_eq!(RingLayout::of(&bare).sides, None);
}

#[test]
fn test_flow_halves_hold_on_every_aspect() {
    the_sweep();
}

/// The aspect sweep's work, from one run of it shared by the tests that
/// read it: the sweep counts on the thread that runs it.
#[cfg(test)]
pub(crate) fn the_sweep() -> &'static SweepWork {
    static SWEEP: std::sync::OnceLock<SweepWork> = std::sync::OnceLock::new();
    SWEEP.get_or_init(do_flow_halves_hold_on_every_aspect)
}

/// The most glyph steps one frame of the aspect sweep may take (ruling
/// R43.2): the worst real frame took 306,098 at 30411a6e — the
/// quatrain inward round the 50:1 ellipse, `Fit` at span ½, re-laid
/// round after round of a clearance that could not part two inks
/// stacked across an outline thinner than they are, each lay some 20
/// `track`s chasing a tolerance finer than its floats or its letters
/// resolve. At R44 the worst, the quatrain outward round the 600 × 12
/// rectangle, `Once` at span ½, took 36,748, its sides' lays counted
/// (ruling R44.3), against a bound of 50,000. Since ruling R46 a step is
/// a unit of all of a frame's work — its shaping, inking, touch sweeps
/// and R27's walks besides its tracking — and the worst, the quatrain
/// inward round the 50:1 ellipse, `Fit`, takes 59,468: the bound is
/// restated in the new unit, some 10% over that (ruling R47.1).
pub const SWEEP_MOST_STEPS: usize = 66_000;

/// The most path lookups a glyph step of a frame may take on the mean,
/// over the aspect sweep's placements and the demo maps' frames, every
/// lookup of the placement counted (ruling R43.1): 220 at 30411a6e.
pub const MEAN_STEP_LOOKUPS: usize = 24;

/// The most path lookups any one glyph step of a frame's line may take
/// (ruling R43.1): `halves::PAIR_ROUNDS` inks tried, three lookups each.
pub const MOST_STEP_LOOKUPS: usize = 64;

/// R43.2: no frame of the aspect sweep takes more than
/// [`SWEEP_MOST_STEPS`] glyph steps (`Trace::spent`, all of a frame's
/// work counted since ruling R46; the bound restated by R47.1).
///
/// Controls: with ends that overlap short by the boxes' distance alone,
/// not their overlap's depth too, the worst frame takes 67,730; with a
/// line's end held to `SHORT_TOLERANCE` however its letters are set,
/// 62,264 (both at R44, in tracking steps alone). (No `do_*` twin: it
/// reads test counters.)
#[test]
fn test_flow_halves_no_frame_of_the_sweep_takes_more_than_66000_glyph_steps() {
    let work = the_sweep();
    assert!(
        work.frames > 300,
        "precondition: the sweep lays its frames ({})",
        work.frames
    );
    assert!(
        work.most_spent <= SWEEP_MOST_STEPS,
        "the aspect sweep's worst frame, {}, took {} glyph steps, at most {SWEEP_MOST_STEPS} (the unit counts all of a frame's work since R46)",
        work.worst,
        work.most_spent
    );
}

/// R43.3, as R47.1 restates it: the frame's [`TRACK_BUDGET`] is at
/// least six times the sweep's worst frame, so the budget cuts only
/// pathological inputs, never real text. It was ten times while a step
/// counted tracking alone; since ruling R46 it counts all of a frame's
/// work, the constant is unchanged (R45's per-input bound rests on it),
/// and the margin is the one measured: 400,000 over 59,468, ×6.7.
///
/// Controls: with the budget a tenth of what it is, red; with a
/// junction's step never aimed by what its last round gave, the worst
/// frame took 42,219, and with a line's lay never warm-started from its
/// last 41,734 (at R44, in tracking steps alone). (No `do_*` twin: it
/// reads test counters.)
#[test]
fn test_flow_halves_the_budget_is_six_times_the_sweeps_worst_frame() {
    use crate::flow::place::halves::TRACK_BUDGET;
    let work = the_sweep();
    assert!(
        work.most_spent > 0,
        "precondition: the sweep's frames are counted"
    );
    assert!(
        work.most_spent * 6 <= TRACK_BUDGET,
        "the aspect sweep's worst frame took {} glyph steps: the budget's {TRACK_BUDGET} is ×{:.1} that, at least ×6",
        work.most_spent,
        TRACK_BUDGET as f64 / work.most_spent as f64
    );
}

/// R43.3: a frame's tracking budget is 400,000 glyph steps — ten times
/// the aspect sweep's worst frame rounded up while a step counted
/// tracking alone, six times since all of a frame's work is counted
/// (rulings R46, R47.1) — not the 2^21 it was while
/// a real frame took 306,098 — so that a pathological input's frame,
/// laid to the budget at a glyph step's cost, finishes well inside the
/// fuzzers' 20 s timeout. Control: with the budget 2^21 again, red.
/// (No `do_*` twin: a constant has nothing to benchmark.)
#[test]
fn test_flow_halves_a_frames_track_budget_is_400000_glyph_steps() {
    use crate::flow::place::halves::TRACK_BUDGET;
    assert_eq!(TRACK_BUDGET, 400_000, "a frame's tracking budget, in glyph steps");
}

/// R43.1: over the aspect sweep's placements a glyph step takes at most
/// [`MEAN_STEP_LOOKUPS`] path lookups on the mean — every lookup the
/// placement makes counted, R27's sizing, the junctions' normals and
/// the side arcs included — and no glyph step of a line more than
/// [`MOST_STEP_LOOKUPS`].
///
/// Controls: with a junction's normals and a side arc's ends found by
/// sampling the whole outline (`ArcPath::nearest`), 25.9 lookups a step;
/// with a letter allowed 64 inks tried, a step takes 192. (No `do_*`
/// twin: it reads test counters.)
#[test]
fn test_flow_halves_a_glyph_step_of_the_sweep_takes_few_path_lookups() {
    let work = the_sweep();
    assert!(
        work.steps > 0 && work.most_step > 0,
        "precondition: the sweep's steps are counted"
    );
    assert!(
        work.lookups <= MEAN_STEP_LOOKUPS * work.steps,
        "the aspect sweep: {} path lookups for {} glyph steps, {:.1} a step, at most {MEAN_STEP_LOOKUPS}",
        work.lookups,
        work.steps,
        work.lookups as f64 / work.steps as f64
    );
    assert!(
        work.most_step <= MOST_STEP_LOOKUPS,
        "the aspect sweep: a glyph step took {} path lookups, at most {MOST_STEP_LOOKUPS}",
        work.most_step
    );
}

/// The aspect sweep's work, counted by its test counters: zero in a
/// build that is not a test's.
#[derive(Debug, Default)]
pub struct SweepWork {
    /// How many frames it laid.
    pub frames: usize,
    /// The most lines one frame tracked.
    pub most_tracks: usize,
    /// The most glyph steps one frame took (`Trace::spent`), and which.
    pub most_spent: usize,
    pub worst: String,
    /// The glyph steps its frames took, and the path lookups their
    /// placements made, in all.
    pub steps: usize,
    pub lookups: usize,
    /// The most path lookups one glyph step of a line took.
    pub most_step: usize,
    /// The most glyph steps a frame took a glyph of its text, rounded up
    /// (ruling R44.2).
    pub most_per_glyph: usize,
    /// The most pairs a frame's touch checks looked at a box
    /// (`Trace::looks`, rulings R48.2, R51.2).
    pub(crate) most_looks: Looks,
    /// Each frame a side arc's touch check of held more classes of height
    /// than it was held back for (ruling R51.2), checked before the frames
    /// are folded into [`SweepWork::most_looks`].
    pub(crate) over_reserved: Vec<String>,
    /// The rounds a letter was moved clear of an ink before its neighbor,
    /// and of them the rounds that left it where it was (ruling R44.7).
    pub earlier: (usize, usize),
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

/// The path lookups this thread has made (`path::PATH_LOOKUPS`, a test
/// counter): none counted in a build that is not a test's.
pub(crate) fn lookups_so_far() -> usize {
    #[cfg(test)]
    return crate::flow::path::PATH_LOOKUPS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    0
}

/// The rounds this thread's letters have been moved clear of an ink
/// before their neighbor, and of them the rounds that left a letter where
/// it was (`halves::EARLIER_ROUNDS`, a test counter): none in a build that
/// is not a test's.
pub(crate) fn earlier_rounds() -> (usize, usize) {
    #[cfg(test)]
    return crate::flow::place::halves::EARLIER_ROUNDS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    (0, 0)
}

/// The most path lookups one glyph step of a frame's line has taken on
/// this thread (`halves::MOST_STEP_LOOKUPS`, a test counter), and set it
/// back to `to`: none in a build that is not a test's.
pub(crate) fn most_step_lookups(to: usize) -> usize {
    #[cfg(test)]
    return crate::flow::place::halves::MOST_STEP_LOOKUPS.with(|most| most.replace(to));
    #[cfg(not(test))]
    {
        let _ = to;
        0
    }
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
pub fn do_flow_halves_hold_on_every_aspect() -> SweepWork {
    /// The most lines a frame here may track (rulings R35.3, R38.3): the
    /// most any does is 598 (6,164 at 30411a6e, 10,534 before the shrink
    /// stopped at a useful precision, ruling R42.2; 17,031 when a line's
    /// gap was bisected). Ruling R43's bounds, in glyph steps and path
    /// lookups, are the sweep's tests below ([`SWEEP_MOST_STEPS`]).
    const MOST_TRACKS: usize = 12_000;
    // Opt-in: the sweep's tallies, for reading the counts this test's
    // bounds are set from.
    let report = std::env::var_os("MANDALA_FLOW_SWEEP_REPORT").is_some();
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
    // The work the frames here take (ruling R43): the most glyph steps
    // one frame takes (`Trace::spent`, what `TRACK_BUDGET` bounds), and
    // the glyph steps and path lookups in all.
    let mut work = SweepWork::default();
    most_step_lookups(0);
    let earlier_before = earlier_rounds();
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
        // The outline, walked clockwise: what a junction's ends' normals
        // are read from (`turn_cos`).
        let path = ArcPath::closed(shape, left, Direction::Clockwise, MIN_TABLE_SPACING);
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
                            let looked = lookups_so_far();
                            let (run, placed, trace) =
                                frame_with(text, *points, shape, &fit, &layout, outward);
                            let tracks = tracks_so_far() - before;
                            most_tracks = most_tracks.max(tracks);
                            if tracks > MOST_TRACKS {
                                faults.push(format!("{what}: {tracks} lines tracked"));
                            }
                            work.frames += 1;
                            if report { eprintln!("HEAVY {} {} {what}", trace.spent, trace.glyphs); }
                            if trace.spent > work.most_spent {
                                (work.most_spent, work.worst) = (trace.spent, what.clone());
                            }
                            work.steps += trace.spent;
                            work.most_per_glyph =
                                work.most_per_glyph.max(trace.spent.div_ceil(trace.glyphs.max(1)));
                            if trace.looks.classes > trace.looks.reserved {
                                work.over_reserved.push(format!(
                                    "{what}: a side arc's check held {} classes, {} held back for",
                                    trace.looks.classes, trace.looks.reserved
                                ));
                            }
                            work.most_looks = work.most_looks.with(trace.looks);
                            work.lookups += lookups_so_far() - looked;
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
                                for (j, (junction, a, b, anchors)) in [
                                    ("top-left", one.ends.0, two.ends.0, (one.anchors.0, two.anchors.0)),
                                    ("bottom-right", one.ends.1, two.ends.1, (one.anchors.1, two.anchors.1)),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    if unheld.junctions.contains(&j) {
                                        continue;
                                    }
                                    let apart = gap(a, b);
                                    let turn = turn_cos(&path, anchors.0, anchors.1);
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
    work.most_tracks = most_tracks;
    work.most_step = most_step_lookups(0);
    let earlier_after = earlier_rounds();
    work.earlier = (
        earlier_after.0 - earlier_before.0,
        earlier_after.1 - earlier_before.1,
    );
    if report {
        eprintln!(
            "the aspect sweep: shrunk {shrunk} cut {cut} reported {reported} apart {apart} most tracks {most_tracks} {work:?}\n{}",
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
    work
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
            "• · • · • · • · • · • · • · • · • · • · •".to_owned(),
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
///
/// **On one band curve** (#308, ruling B15.1). Each frame solves its own
/// band curve from where its inks lie, and a line's natural ends — at its
/// font's gap, centered on its half's apex on that curve — move with the
/// curve's offset: the psalm's natural frame lies at 21.1764 and its ½
/// and whole frames at 21.1953, and its natural end on the first half
/// at 911.0593 on the one, 911.0309 on the other. So the three spans are
/// laid on the ½ frame's own curve (`OFFSET_OVERRIDE`), each frame
/// held to having laid there (a precondition), and the mean is exact.
/// Control: with the natural frame laid on its own curve again, the
/// psalm's end at 989.3153 misses the mean of 911.0593 and 1067.5997 by
/// 0.014, past the hundredth. (No `do_*` twin: it reads a test seam.)
#[test]
fn test_flow_halves_span_half_is_the_mean_of_natural_and_whole() {
    use crate::flow::place::halves::OFFSET_OVERRIDE;
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
        let half_span = LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE);
        let own = frame(
            text,
            points,
            &shape,
            &RingLayout {
                span: half_span,
                ..RingLayout::whole()
            },
        )
        .2
        .offset
        .expect("a frame on its halves has a band curve");
        let at = |span: LineSpan| {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) =
                with_seam(&OFFSET_OVERRIDE, own.to_bits() as usize, || frame(text, points, &shape, &layout));
            assert_eq!(
                trace.offset.map(f32::to_bits),
                Some(own.to_bits()),
                "precondition, {name} at {span:?}: the frame lies on the ½ frame's band curve"
            );
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
        let (natural, half, whole) = (at(LineSpan::NATURAL), at(half_span), at(LineSpan::WHOLE));
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
/// pass it — never inverted, never off the half: clearances whose
/// product with the half's length, or whose sum, is past the largest
/// `f32` included, and infinite ones, the whole half to an infinite
/// clearance against a finite one and half each to two.
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
                ([1e36, 1e36], (ja + 0.5 * len, ja + 0.5 * len)),
                ([1e36, 3e36], (ja + 0.25 * len, ja + 0.25 * len)),
                ([3e38, 3e38], (ja + 0.5 * len, ja + 0.5 * len)),
                ([f32::INFINITY, 1.0], (jb, jb)),
                ([1.0, f32::INFINITY], (ja, ja)),
                ([f32::INFINITY; 2], (ja + 0.5 * len, ja + 0.5 * len)),
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
/// Precondition: `"_"` has no side bearings, its ink as wide as its
/// advance or wider.
pub fn do_flow_halves_a_copy_at_its_floor_clears_its_neighbors() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(820.0, 330.0)), Shape::Ellipse);
    for ornament in ["_", "______", "·", "¤ ¤"] {
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
        if ornament == "_" {
            let (i, (a, b)) = drawn[0];
            let bearings = run.glyphs[i].advance - (b.x - a.x);
            assert!(
                bearings < INK_TOLERANCE,
                "precondition: \"_\" has side bearings of {bearings}, which would hold its copies apart"
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
/// Control: these rows hold even with the clearance kept from whole
/// below it (their ends are stacked); the inward frames of
/// `do_flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward`
/// do not.
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
            for (junction, a, b, anchors) in [
                ("top-left", lines[0].ends.0, lines[1].ends.0, (lines[0].anchors.0, lines[1].anchors.0)),
                ("bottom-right", lines[0].ends.1, lines[1].ends.1, (lines[0].anchors.1, lines[1].anchors.1)),
            ] {
                let apart = gap(a, b);
                let turn = turn_cos(&path, anchors.0, anchors.1)
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
fn test_flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward() {
    do_flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward();
}

/// Below whole the break is held at the span laid (ruling R35.1), and
/// that is not the clearance found at whole kept: inward round the
/// 600 × 12 ellipse, the Ecclesiastes text as a `Once` frame and the
/// quatrain as a `Fit` frame, at span ½ and natural, each line lies
/// short of where it lies at whole, and with the whole span's clearance
/// alone their top-left ends meet short of a break. The clearance grown
/// on at the span laid holds it, and nothing is left unheld.
/// Control: with the clearance kept from whole below it, each frame
/// reports its top-left junction short of a break.
pub fn do_flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward() {
    let flat = outline(Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)), Shape::Ellipse);
    let half = LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE);
    for (text, points, fit) in [
        (ECCLESIASTES, 18.0, FitMode::Once { filler: None }),
        (QUATRAIN, 18.0, FIT),
    ] {
        for span in [half, LineSpan::NATURAL] {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (_, _, trace) = frame_with(text, points, &flat, &fit, &layout, false);
            let what = format!("{:.8} {fit:?} span {}", text, span.get());
            assert_eq!(trace.lines.len(), 2, "precondition: {what}: two lines");
            assert!(
                trace.unheld.junctions.is_empty(),
                "{what}: junctions {:?} short of a break",
                trace.unheld.junctions
            );
        }
    }
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
/// ellipse with `"¤"` sides, at span ½, where each tip's arc wraps the
/// tip inside an outline thinner than its two lines. No two inks touch,
/// text or ornament. Precondition: the trace reports a break left
/// without its ornament. Control: with every arc kept, copies touch.
pub fn do_flow_halves_a_side_that_would_touch_is_left_out() {
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)), Shape::Ellipse);
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("¤".to_owned(), Rgba::opaque(9, 9, 9))),
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
/// at span whole, ½ and natural, round outlines its two lines fill. Drawn back to part
/// a junction, a line with no slack left would be pressed while the
/// other line has room to give, and a pressed line sends the frame to
/// its shrink; so each line is drawn back no further than its slack, the
/// rest handed to the other line (outward, the 10:1 and 50:1 ellipses
/// and the 50:1 pill). (Each round's step was also damped by how much
/// the drawing back eases the break wanted, which once decided the
/// inward 10:1 rectangle; since a junction's later steps are aimed by
/// what its last round gave (ruling R43), the damping guides only its
/// first, and no row here needs it.)
///
/// A line its re-solve leaves below its font's gap by no more than the
/// line's precision — each letter set to within the pairwise rule's
/// tolerance (ruling R43) — is laid at its font's gap, not pressed: the
/// 10:1 ellipse's line, at its font's gap at whole, is laid again at ½.
/// And a letter round a sharp corner inward is held clear of the
/// letters before its neighbor, else two of them touch (the 10:1
/// rectangle).
///
/// Control: with the slack cap removed the outward frames shrink; with
/// the font's-gap check held
/// to the slide's tolerance the 10:1 ellipse shrinks at ½; with a letter
/// held clear of its neighbor alone the 10:1 rectangle does.
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
        for span in [
            LineSpan::WHOLE,
            LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
            LineSpan::NATURAL,
        ] {
            let layout = RingLayout {
                span,
                ..RingLayout::whole()
            };
            let (run, placed, trace) = frame_with(QUATRAIN, 18.0, &shape, &FIT, &layout, outward);
            let name = format!("{name} at span {}", span.get());
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

#[test]
fn test_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely() {
    do_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely();
}

/// A frame round an outline as long as an `f32` is laid along its
/// halves and placed finitely: every line's ends, every clearance and
/// every glyph a number. The 10³⁸ outlines' junctions lie past half the
/// largest `f32` (asserted), so a midpoint taken as the half of a sum
/// overflows; round the 10³⁶ one a break of two word gaps, its line's
/// gaps stretched over its half, is wider than any `f32`. Found by a probe of outlines from 10³⁰ to the largest
/// `f32` long beside the flow_structured findings
/// (`test_a_frame_whose_clearances_pass_its_half_places_finitely`),
/// which the fuzzer had not reached.
///
/// Controls: with a one-letter line centered at `0.5 * (sa + sb)` again,
/// "a b" round the 10³⁸ × 10 rectangle places a glyph at `[NaN, NaN]`;
/// with the clearance step no longer held to the largest `f32`, "ab"
/// round the 10³⁶ × 10 one keeps a clearance that is not a number
/// (`[[NaN, 0.0], [NaN, 0.0]]`: an infinite step shared by a share of
/// zero, and held to a slack, is `inf * 0` and `inf - inf`); with the
/// old quotient in `Layout::room`, "ab" round the 10³⁶ × 10 rectangle
/// ends a line where no number does.
pub fn do_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely() {
    for (width, height) in [(1e36_f32, 10.0_f32), (1e38, 10.0), (1e38, 1e37)] {
        let shape = outline(
            Rect::new(Vec2::ZERO, Vec2::new(width, height)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        );
        let (junctions, apexes, length) = halves_seam(&shape);
        assert!(
            length.is_finite() && apexes.iter().chain(junctions.iter().flat_map(|(a, b)| [a, b])).all(|s| s.is_finite()),
            "precondition: the {width} × {height} outline's halves are measured, so its lines are laid along them"
        );
        if width >= 1e38 {
            assert!(
                junctions.iter().any(|(_, end)| *end > 0.5 * f32::MAX),
                "precondition: a junction of the {width} × {height} outline lies past half the largest f32, {junctions:?}"
            );
        }
        for text in ["ab", "a b", "ab cd ef gh"] {
            for fit in [FitMode::Spread, FitMode::Once { filler: None }, FIT] {
                for outward in [true, false] {
                    let what = format!("{text:?} round {width} × {height}, {fit:?}, outward {outward}");
                    let (_, placed, trace) =
                        frame_with(text, 18.0, &shape, &fit, &RingLayout::whole(), outward);
                    assert!(
                        !trace.lines.is_empty(),
                        "precondition: {what} is laid as a frame's lines"
                    );
                    for line in &trace.lines {
                        assert!(
                            line.from.is_finite() && line.to.is_finite(),
                            "{what}: line {line:?} ends where no number does"
                        );
                    }
                    assert!(
                        trace.clear.iter().flatten().all(|c| !c.is_nan()),
                        "{what}: a clearance that is not a number, {:?}",
                        trace.clear
                    );
                    for glyph in &placed.glyphs {
                        assert!(glyph.pos.is_finite(), "{what}: a glyph placed at {}", glyph.pos);
                    }
                }
            }
        }
    }
}

/// The README's demo maps' frames, laid as the demo maps' render path
/// lays them (`src/application/app/tests_gpu_demo_maps.rs`: every node a
/// box as large as its minimum size and its sections, each ring of its
/// frame on the outline offset past the rings inside it), and the work
/// each took: its trace's glyph steps, and the path lookups its
/// placement made. `Repeat` rings take no glyph step and are left out.
#[cfg(test)]
pub(crate) fn demo_map_frames() -> Vec<(String, usize, usize, Looks)> {
    use crate::arena::NodeId;
    use crate::document::element::{DocElement, DocKind};
    use crate::document::format::load;
    use crate::document::ids::Document;
    use crate::document::resolve::{resolve_frame, resolve_node, resolve_ring, resolve_text, ResolvedText};
    use crate::document::values::{Align, FontSize};
    use crate::flow::{offset, ring_offset, shape_box, shape_run, FlowSpan};

    let children = |doc: &Document, id: NodeId, kind: DocKind| -> Vec<NodeId> {
        doc.children(id)
            .filter(|&c| doc.get(c).is_some_and(|e| e.kind() == kind))
            .collect()
    };
    let points = |mut style: ResolvedText, side: f32| {
        if let FontSize::Rel(fraction) = style.size {
            style.size = FontSize::Pt(fraction * side);
        }
        style
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../maps/demos");
    let mut maps: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.to_string_lossy().ends_with(".mandala.json"))
        .collect();
    maps.sort();
    let mut out = Vec::new();
    for path in maps {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let tree = doc.tree();
        let mut nodes = Vec::new();
        let mut stack = vec![doc.root()];
        while let Some(id) = stack.pop() {
            if doc.get(id).is_some_and(|e| e.kind() == DocKind::Node) {
                nodes.push(id);
            }
            stack.extend(doc.children(id));
        }
        for node in nodes {
            let Some(DocElement::Node(data)) = doc.get(node) else {
                continue;
            };
            let spans_of = |owner: NodeId, side: f32| -> Vec<(String, ResolvedText)> {
                children(&doc, owner, DocKind::Span)
                    .into_iter()
                    .filter_map(|span| match doc.get(span) {
                        Some(DocElement::Span(s)) => {
                            Some((s.text.clone(), points(resolve_text(tree, span, &mut ()), side)))
                        }
                        _ => None,
                    })
                    .collect()
            };
            let origin = data.position;
            let (mut min, mut max) = (origin, origin + data.min_size.unwrap_or(Vec2::ZERO));
            for section in children(&doc, node, DocKind::Section) {
                let Some(DocElement::Section(s)) = doc.get(section) else {
                    continue;
                };
                let spans = spans_of(section, 0.0);
                let size = with_fonts(|fs| {
                    let flow: Vec<FlowSpan<'_>> = spans
                        .iter()
                        .map(|(text, style)| FlowSpan {
                            text,
                            style,
                            link: None,
                        })
                        .collect();
                    shape_box(&flow, s.min_size, s.align.unwrap_or(Align::Left), fs).size
                });
                let at = origin + s.offset;
                min = min.min(at);
                max = max.max(at + size);
            }
            let rect = Rect::new(min, max - min);
            let base = outline(rect, resolve_node(tree, node, &mut ()).shape);
            for frame in children(&doc, node, DocKind::Frame) {
                let f = resolve_frame(tree, frame, &mut ());
                if !f.visible {
                    continue;
                }
                let side = rect.size.x.min(rect.size.y);
                let mut heights: Vec<f32> = Vec::new();
                for (k, ring) in children(&doc, frame, DocKind::Ring).into_iter().enumerate() {
                    let spans = spans_of(ring, side);
                    if spans.is_empty() {
                        continue;
                    }
                    let path = offset(&base, ring_offset(f.inset, f.gap, &heights));
                    let style = points(resolve_text(tree, ring, &mut ()), side);
                    let resolved = resolve_ring(tree, ring, &mut ());
                    let mut trace = Trace::default();
                    let looked = lookups_so_far();
                    let height = with_fonts(|fs| {
                        let flow: Vec<FlowSpan<'_>> = spans
                            .iter()
                            .map(|(text, style)| FlowSpan {
                                text,
                                style,
                                link: None,
                            })
                            .collect();
                        let shaped = shape_run(&flow, 1.0, fs);
                        let (rescaled, _) = place_on_closed_traced(
                            &shaped,
                            &path,
                            f.start,
                            f.direction,
                            &resolved.fit,
                            &RingLayout::of(&resolved),
                            true,
                            &style,
                            fs,
                            Some(&mut trace),
                        );
                        rescaled.unwrap_or(shaped).line_height
                    });
                    let lookups = lookups_so_far() - looked;
                    heights.push(height);
                    if !matches!(resolved.fit, FitMode::Repeat) {
                        out.push((
                            format!("{name} node {} ring {k}", data.ident.0),
                            trace.spent,
                            lookups,
                            trace.looks,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Whether a frame's touch checks, `looks`, looked at no more pairs a
/// box than measured (ruling R51.2): in their sweeps and within each
/// class of height's own strips, `TOUCH_LOOKS_BASE_MEASURED` a box;
/// between a class and the finer ones, `TOUCH_LOOKS_A_CLASS_MEASURED` a
/// box for each class the check held; and so, each part twice that,
/// within `touch_looks` of its classes; and no side arc's check holding
/// more classes than it was held back for (`height_classes`). What failed, with each part's
/// ratio to its bound, where one did.
#[cfg(test)]
pub(crate) fn looks_as_measured(looks: &Looks) -> Result<(), String> {
    use crate::flow::place::halves::{
        touch_looks, TOUCH_LOOKS_A_CLASS, TOUCH_LOOKS_A_CLASS_MEASURED, TOUCH_LOOKS_BASE,
        TOUCH_LOOKS_BASE_MEASURED,
    };
    let (base, boxes) = looks.base;
    let (across, of, classes) = looks.across;
    let (worst, worst_boxes, worst_classes) = looks.worst;
    let mut broken = Vec::new();
    if base > TOUCH_LOOKS_BASE_MEASURED * boxes {
        broken.push(format!(
            "{base} looks over {boxes} boxes in a check's sweep and its classes' own strips, {:.2} a box, past the {TOUCH_LOOKS_BASE_MEASURED} measured (TOUCH_LOOKS_BASE, {TOUCH_LOOKS_BASE}, is ×{:.2} it)",
            base as f64 / boxes.max(1) as f64,
            (TOUCH_LOOKS_BASE * boxes) as f64 / base.max(1) as f64
        ));
    }
    if across > TOUCH_LOOKS_A_CLASS_MEASURED * of * classes {
        broken.push(format!(
            "{across} looks over {of} boxes of {classes} classes between its classes, {:.2} a box a class, past the {TOUCH_LOOKS_A_CLASS_MEASURED} measured (TOUCH_LOOKS_A_CLASS, {TOUCH_LOOKS_A_CLASS}, is ×{:.2} it)",
            across as f64 / (of * classes).max(1) as f64,
            (TOUCH_LOOKS_A_CLASS * of * classes) as f64 / across.max(1) as f64
        ));
    }
    if looks.reserved > 0 && looks.classes > looks.reserved {
        broken.push(format!(
            "a side arc's check held {} classes of height, past the {} it was held back for",
            looks.classes, looks.reserved
        ));
    }
    if !looks.within() {
        broken.push(format!(
            "{worst} looks over {worst_boxes} boxes of {worst_classes} classes, {:.2} a box, past touch_looks's {}",
            worst as f64 / worst_boxes.max(1) as f64,
            touch_looks(worst_classes)
        ));
    }
    if broken.is_empty() {
        Ok(())
    } else {
        Err(broken.join("; "))
    }
}

/// The ratio of each part of `looks` to its measured bound, for a
/// message: in the sweep and the classes' own strips, between classes,
/// and the check nearest `touch_looks`.
#[cfg(test)]
pub(crate) fn looks_ratios(looks: &Looks) -> String {
    use crate::flow::place::halves::{touch_looks, TOUCH_LOOKS_A_CLASS_MEASURED, TOUCH_LOOKS_BASE_MEASURED};
    let (base, boxes) = looks.base;
    let (across, of, classes) = looks.across;
    let (worst, worst_boxes, worst_classes) = looks.worst;
    format!(
        "{base} looks over {boxes} boxes in sweeps and own strips ({:.2} a box of {TOUCH_LOOKS_BASE_MEASURED} measured); {across} over {of} boxes of {classes} classes between classes ({:.2} a box a class of {TOUCH_LOOKS_A_CLASS_MEASURED}); nearest its bound, {worst} over {worst_boxes} boxes of {worst_classes} classes ({:.2} a box of {}); at most {} classes in a side arc's check, {} held back for",
        base as f64 / boxes.max(1) as f64,
        across as f64 / (of * classes).max(1) as f64,
        worst as f64 / worst_boxes.max(1) as f64,
        touch_looks(worst_classes),
        looks.classes,
        looks.reserved
    )
}

/// R48.2, R50.2, R51.2: no touch check of the aspect sweep's frames or
/// the demo maps' looks at more pairs a box than measured
/// ([`looks_as_measured`]), and what a side arc holds back for its check,
/// `touch_looks` of its classes, is twice each part. The most measured
/// over these frames is 3.22 a box in their sweeps and their classes' own
/// strips (219 looks over 68 boxes) and 0.68 a box a class between
/// classes (200 over 74 boxes of four classes); over the budget grids and
/// the pins, which hold their frames to the same parts, 4.96 and 1.50,
/// which the measured parts are rounded up from. And no frame's side arc's
/// check holds more classes of height than its arc held back for, each
/// frame held to it on its own, before the frames are folded (ruling
/// R51.2). The flow fuzz property holds every committed finding and
/// fuzzed frame to both.
///
/// Controls: with each part measured one lower, red; with each arc held
/// back for one class at most, red on the frames held to it one by one.
/// (With an arc's classes taken from the text alone, green: every
/// ornament of these frames is of classes their text holds.) (No `do_*`
/// twin: it reads the sweep's traces.)
#[test]
fn test_flow_halves_a_touch_check_looks_at_few_pairs_a_box() {
    use crate::flow::place::halves::{
        TOUCH_LOOKS_A_CLASS, TOUCH_LOOKS_A_CLASS_MEASURED, TOUCH_LOOKS_BASE, TOUCH_LOOKS_BASE_MEASURED,
    };
    let sweep = the_sweep();
    let mut over_reserved = sweep.over_reserved.clone();
    let mut most = sweep.most_looks;
    for (name, _, _, looks) in demo_map_frames() {
        // Each frame on its own, before the fold takes the most classes
        // and the most held back of different frames.
        if looks.classes > looks.reserved {
            over_reserved.push(format!(
                "{name}: a side arc's check held {} classes, {} held back for",
                looks.classes, looks.reserved
            ));
        }
        most = most.with(looks);
    }
    assert!(
        over_reserved.is_empty(),
        "side arcs' checks held more classes of height than they were held back for:\n{}",
        over_reserved.join("\n")
    );
    assert!(
        most.base.0 > 0 && most.base.1 > 0 && most.across.0 > 0,
        "precondition: the frames' touch checks are traced, and some lay boxes of more than one class: {}",
        looks_ratios(&most)
    );
    if let Err(why) = looks_as_measured(&most) {
        panic!("{why}");
    }
    assert_eq!(
        (TOUCH_LOOKS_BASE, TOUCH_LOOKS_A_CLASS),
        (2 * TOUCH_LOOKS_BASE_MEASURED, 2 * TOUCH_LOOKS_A_CLASS_MEASURED),
        "twice the most measured"
    );
}

/// R43.1 on the README's demo maps: their frames' glyph steps take at
/// most [`MEAN_STEP_LOOKUPS`] path lookups on the mean, every lookup of
/// each frame's placement counted, and none of a line more than
/// [`MOST_STEP_LOOKUPS`]; and no frame takes more than
/// [`SWEEP_MOST_STEPS`] glyph steps.
///
/// Controls: with the frame's pairwise rule solved by the ring's turns
/// again (`PathWalk::ink_gap_at`), 158.3 lookups a step; with a
/// junction's normals and a side arc's ends found by sampling the whole
/// outline (`ArcPath::nearest`), 56.4; with the halves' junctions and
/// apexes found so too, 26.1. (No `do_*` twin: it reads test counters.)
#[test]
fn test_flow_halves_a_glyph_step_of_the_demo_maps_takes_few_path_lookups() {
    most_step_lookups(0);
    let frames = demo_map_frames();
    let most_step = most_step_lookups(0);
    assert!(
        frames.len() >= 5 && frames.iter().all(|(_, spent, _, _)| *spent > 0),
        "precondition: the four demo maps' five frames are laid and counted: {frames:?}"
    );
    let steps: usize = frames.iter().map(|(_, spent, _, _)| spent).sum();
    let lookups: usize = frames.iter().map(|(_, _, lookups, _)| lookups).sum();
    assert!(
        lookups <= MEAN_STEP_LOOKUPS * steps,
        "the demo maps' frames: {lookups} path lookups for {steps} glyph steps, {:.1} a step, at most {MEAN_STEP_LOOKUPS}: {frames:?}",
        lookups as f64 / steps as f64
    );
    assert!(
        most_step <= MOST_STEP_LOOKUPS,
        "the demo maps' frames: a glyph step took {most_step} path lookups, at most {MOST_STEP_LOOKUPS}"
    );
    for (what, spent, _, _) in &frames {
        assert!(
            *spent <= SWEEP_MOST_STEPS,
            "{what}: {spent} glyph steps, at most {SWEEP_MOST_STEPS}"
        );
    }
    if std::env::var_os("MANDALA_FLOW_SWEEP_REPORT").is_some() {
        eprintln!("the demo maps' frames: {steps} glyph steps, {lookups} lookups, most a step {most_step}: {frames:?}");
    }
}

#[test]
fn test_flow_halves_an_end_past_a_sharp_corner_faces_away_from_it() {
    do_flow_halves_an_end_past_a_sharp_corner_faces_away_from_it();
}

/// A line's ends at a sharp corner (ruling R43, #308), round a 600 × 60
/// rectangle with no radius. An end ink set round the corner — here at
/// the bottom-left corner, on the second half, which runs down the left
/// side from its middle — has its baseline point on the baseline's sweep
/// round the corner, where the outline has no one normal: it faces the
/// way from the corner to it, halfway between the two sides' normals
/// where it is set at the corner itself, not the left side's way or the
/// bottom's, which the 135° test would read as a line stacked across
/// the outline from one on the top. And a part ending at a corner — the
/// first half's at the bottom-right one, down the right side — draws
/// back up the side before it, not along the bottom after it, which
/// reads as drawing back parting nothing. Each expectation is the
/// geometry's, worked here by hand.
///
/// Controls: with the normal always the outline's at the point the end
/// is set at, the end faces a side's way, `(-1, 0)`; with the retreat
/// taken from the tangent at the end itself, it runs along the bottom.
pub fn do_flow_halves_an_end_past_a_sharp_corner_faces_away_from_it() {
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 60.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    // The second half from (0, 30) down: the corner (0, 60) is 30 along,
    // and a baseline 10 off the outline sweeps round it from the left
    // side's normal to the bottom's.
    let (normal, _) = end_seam(&shape, 1, 10.0, 30.0, 630.0);
    let away = Vec2::new(-1.0, 1.0).normalize();
    assert!(
        normal.distance(away) < 1e-3,
        "an end set at the bottom-left corner faces {normal:?}, not the way from the corner to it, {away:?}"
    );
    // The first half from (0, 30) up: over the top, then down the right
    // side to the corner (600, 60), 690 along.
    let (_, retreat) = end_seam(&shape, 0, 10.0, 300.0, 690.0);
    assert!(
        retreat.distance(Vec2::new(0.0, -1.0)) < 1e-3,
        "an end at the bottom-right corner draws back along {retreat:?}, not up the right side"
    );
}

/// `n` words of four letters and a space each, `5 n` glyphs.
#[cfg(test)]
fn words(n: usize) -> String {
    "word ".repeat(n)
}

/// Run `f` with a halves test seam set to `value`, and the seam cleared
/// again after, whether or not `f` panics.
#[cfg(test)]
pub(crate) fn with_seam<R>(
    seam: &'static std::thread::LocalKey<std::cell::Cell<Option<usize>>>,
    value: usize,
    f: impl FnOnce() -> R,
) -> R {
    struct Clear(&'static std::thread::LocalKey<std::cell::Cell<Option<usize>>>);
    impl Drop for Clear {
        fn drop(&mut self) {
            self.0.with(|cell| cell.set(None));
        }
    }
    seam.with(|cell| cell.set(Some(value)));
    let _clear = Clear(seam);
    f()
}

/// R44.1: the cap a frame's text and sides are held to is 65,536 glyphs,
/// and a frame's budget is at most 3,000 glyph steps each of them.
/// (No `do_*` twin: a constant has nothing to benchmark.)
#[test]
fn test_flow_halves_a_frame_places_at_most_65536_glyphs() {
    use crate::flow::place::halves::frame_budget;
    assert_eq!(
        crate::flow::MAX_PLACED_GLYPHS,
        65_536,
        "the glyph cap (ruling R41)"
    );
    assert_eq!(
        frame_budget(usize::MAX),
        3_000 * 65_536,
        "a frame's budget is scaled by its glyphs to the cap, no further"
    );
}

/// R44.1: a frame's text is cut at a cluster to the glyphs it may still
/// place — a cluster's glyphs together or not at all — the blanks its
/// end is left with dropped, and what is cut said: three-glyph clusters
/// and a space, held to 5, 6 and 7 glyphs, and one held whole.
///
/// Controls: with a cluster kept whenever it starts within the cap, held
/// to 5 it keeps 7 glyphs, red; with the cut removed, held to 5 it keeps
/// all 11, red. (No `do_*` twin: it reads a test seam.)
#[test]
fn test_flow_halves_a_frames_text_is_cut_to_the_cap_at_a_cluster() {
    use crate::flow::place::halves::to_cap_seam;
    use crate::flow::tests::support::clustered;
    // "abc", " ", "def", " ", "ghi": clusters of 3, 1, 3, 1, 3 glyphs.
    let starts = [
        true, false, false, true, true, false, false, true, true, false, false,
    ];
    let mut run = clustered(&starts.map(|s| (6.0, s)), 20.0);
    for blank in [3, 7] {
        run.glyphs[blank].blank = true;
    }
    for (left, kept, cut) in [
        (5, 0..3, true),
        (6, 0..3, true),
        (7, 0..7, true),
        (11, 0..11, false),
    ] {
        let (got, was_cut, after) = to_cap_seam(&run, 0..11, left);
        assert_eq!(
            (got.clone(), was_cut),
            (kept.clone(), cut),
            "held to {left} glyphs: kept {got:?}, cut {was_cut}"
        );
        assert_eq!(after, left - kept.len(), "held to {left}: the glyphs left after");
    }
}

/// R44.1: a frame's text is held to the glyph cap end to end, cut at a
/// cluster and reported: 300 words, 1,500 glyphs, `Once` at 16 pt round
/// a 20,000 ellipse, which holds them whole, placed with the cap at
/// 1,000 (the test seam: the literal cap's 70,000-glyph frame lays for
/// a minute in a debug build). Nothing it keeps touches.
///
/// Control: with the cut removed, 1,498 glyphs are placed, untruncated.
/// (No `do_*` twin: it sets a test seam.)
#[test]
fn test_flow_halves_a_frames_text_is_held_to_the_glyph_cap() {
    use crate::flow::place::halves::CAP_OVERRIDE;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)), Shape::Ellipse);
    let lay = || {
        frame_with(
            &words(300),
            16.0,
            &shape,
            &FitMode::Once { filler: None },
            &RingLayout::default(),
            true,
        )
    };
    let (_, whole, _) = lay();
    assert!(
        whole.glyphs.len() > 1_400 && !whole.truncated,
        "precondition: the frame holds its text whole ({} glyphs, truncated {})",
        whole.glyphs.len(),
        whole.truncated
    );
    let (_, placed, trace) = with_seam(&CAP_OVERRIDE, 1_000, lay);
    assert!(
        placed.glyphs.len() <= 1_000 && placed.glyphs.len() > 900 && placed.truncated,
        "{} glyphs placed, truncated {}: at most the cap of 1,000, and reported",
        placed.glyphs.len(),
        placed.truncated
    );
    assert!(!trace.unheld.overlap, "no two inks touch ({:?})", trace.unheld);
}

/// R44.2: a long text's frame is laid within a budget that grows with
/// it, and nothing it leaves touches. The ruling's frames are 2,000
/// words, 10,000 glyphs, at 16 pt round 40,000 and 20,000 ellipses,
/// which lay for some 17 s in a debug build; these are a fifth of them —
/// 400 words round 8,000 and 4,000 ellipses, the same text to outline —
/// with the least budget a tenth of [`TRACK_BUDGET`] (the test seam):
/// the 8,000 frame takes some 64,000 glyph steps, as the ruling's 40,000
/// frame took some 555,000, so a budget fixed at that least would cut
/// it as the fixed 400,000 cut the ruling's. Round 8,000 both junctions are met, and
/// round 4,000 the frame holds or reports a cut — never two inks
/// touching.
///
/// Control: with the budget fixed at its least again, the 8,000 frame
/// leaves a junction unmet. (No `do_*` twin: it sets a test seam.)
#[test]
fn test_flow_halves_a_long_texts_frame_is_laid_within_its_budget() {
    use crate::flow::place::halves::{FLOOR_OVERRIDE, TRACK_BUDGET};
    for (side, meets) in [(8_000.0, true), (4_000.0, false)] {
        let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(side)), Shape::Ellipse);
        let (_, placed, trace) = with_seam(&FLOOR_OVERRIDE, TRACK_BUDGET / 10, || {
            frame_with(
                &words(400),
                16.0,
                &shape,
                &FitMode::Once { filler: None },
                &RingLayout::default(),
                true,
            )
        });
        let what = format!(
            "400 words round a {side} ellipse ({} of {} glyph steps)",
            trace.spent, trace.budget
        );
        if meets {
            assert!(
                trace.spent > TRACK_BUDGET / 10,
                "precondition: {what} takes more than the least budget"
            );
        }
        assert!(
            !trace.unheld.overlap,
            "{what}: two inks touch ({:?})",
            trace.unheld
        );
        if meets {
            assert!(
                trace.unheld.is_empty() && !placed.truncated,
                "{what}: {:?}",
                trace.unheld
            );
        } else {
            assert!(
                trace.unheld.is_empty() || placed.truncated,
                "{what}: {:?} uncut",
                trace.unheld
            );
        }
    }
}

/// R44.2, as R47.2 bounds it: the budget is read before every lay and
/// every build, so a frame that spends it overshoots it by no more than
/// a lay of each line — a few `track`s, a glyph step each glyph — and
/// one build's shaping and inking, and what runs before the budget is
/// first read (`overshoot_steps`, from the weight constants), and is
/// left with no two inks touching: the quatrain round the 600 × 12
/// rectangle, `Once` at span ½, with budgets of 1, 1,000 and 10,000
/// glyph steps. `untouched` runs within the budget, which holds its
/// share back: a budget smaller than that share is read as the share.
///
/// Controls: with the budget read only between clearance rounds, as at
/// 30411a6e, the overshoot passes the bound; with the shrink's builds
/// unread, likewise. (No `do_*` twin: it sets a test seam.)
#[test]
fn test_flow_halves_a_spent_budget_is_overshot_by_a_lay_at_most() {
    use crate::flow::place::halves::BUDGET_OVERRIDE;
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        ..RingLayout::whole()
    };
    use crate::flow::place::halves::{
        inking_steps, overshoot_steps, untouched_steps, FRAME_STEPS, OVERSHOOT_TRACKS, TRACK_CALL_STEPS,
    };
    for budget in [1, 1_000, 10_000] {
        let (run, _, trace) = with_seam(&BUDGET_OVERRIDE, budget, || {
            frame_with(
                QUATRAIN,
                22.0,
                &shape,
                &FitMode::Once { filler: None },
                &layout,
                true,
            )
        });
        // A `Once` frame with no sides, laid at its size: its builds
        // reshape nothing, so a build past the budget is its inking alone.
        let past =
            FRAME_STEPS + inking_steps(&run) + OVERSHOOT_TRACKS * (run.glyphs.len() + TRACK_CALL_STEPS);
        assert!(
            past <= overshoot_steps(&run, None, false),
            "precondition: the frame's overshoot is within the general bound"
        );
        let held = budget.max(untouched_steps(trace.glyphs));
        assert!(
            trace.spent >= budget,
            "precondition: the budget of {budget} is spent ({})",
            trace.spent
        );
        assert!(
            trace.spent <= held + past,
            "a budget of {budget}: {} glyph steps, at most {past} past it",
            trace.spent,
        );
        assert!(
            !trace.unheld.overlap,
            "a budget of {budget}: two inks touch ({:?})",
            trace.unheld
        );
    }
}

/// R46.2: a frame's `Trace::spent` counts all its work — every shaping
/// and inking it does and every `track` it lays, each at its weight —
/// held against the weights counted where the work is done (the test
/// counters `SHAPED_STEPS`, `INKED_STEPS`, `TRACKED_STEPS`), not where
/// the frame counts it: the quatrain inward round the 600 × 12
/// rectangle, `Fit`, which R27 sizes and the halves shrink (a reshape
/// and an inking each build), and outward, `Once` at span ½ with a side
/// ornament (shaped, sized and inked beside the text).
///
/// Controls: with a build's reshape uncounted, red; with its inking
/// uncounted, red; with `side_run`'s shaping uncounted, red. (No `do_*`
/// twin: it reads test counters.)
#[test]
fn test_flow_halves_a_frames_shaping_inking_and_tracking_are_all_counted() {
    use crate::flow::place::halves::{BUILDS, INKED_STEPS, SHAPED_STEPS, TRACKED_STEPS};
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let sided = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("=".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    for (what, fit, layout, outward) in [
        (
            "Fit inward",
            FitMode::Fit { min: 0.5, max: 2.0 },
            RingLayout::whole(),
            false,
        ),
        (
            "Once outward with sides",
            FitMode::Once { filler: None },
            sided,
            true,
        ),
    ] {
        let flow = style(22.0);
        // Shaped before the counters are reset: the frame's own work only.
        let run = with_fonts(|fs| real_run(QUATRAIN, &flow, fs));
        let mut trace = Trace::default();
        for counter in [&SHAPED_STEPS, &INKED_STEPS, &TRACKED_STEPS, &BUILDS] {
            counter.with(|count| count.set(0));
        }
        with_fonts(|fs| {
            place_on_closed_traced(
                &run,
                &shape,
                StartPoint {
                    edge: Edge::Top,
                    at: 0.0,
                },
                Direction::Clockwise,
                &fit,
                &layout,
                outward,
                &flow,
                fs,
                Some(&mut trace),
            )
        });
        let [shaped, inked, tracked, builds] = [&SHAPED_STEPS, &INKED_STEPS, &TRACKED_STEPS, &BUILDS]
            .map(|counter| counter.with(std::cell::Cell::get));
        assert!(
            shaped > 0 && inked > 0 && tracked > 0,
            "precondition, {what}: the frame shapes ({shaped}), inks ({inked}) and tracks ({tracked}), in {builds} builds"
        );
        assert!(
            trace.spent >= shaped + inked + tracked,
            "{what}: {} glyph steps counted, past the {shaped} shaping, {inked} inking and {tracked} tracking it did",
            trace.spent
        );
    }
}

/// R44.2: a frame's budget is 3,000 glyph steps a glyph of its text, at
/// least 400,000 — ten times the most a frame of the aspect sweep took
/// a glyph, rounded up, while a step counted tracking alone — and, as
/// R47.1 restates it now that a step counts all of a frame's work (R46),
/// at least five times the sweep's most a glyph: 3,000 over 597, ×5.0.
/// Control: with the sweep's most a glyph past 600, red. (No `do_*` twin:
/// it reads test counters.)
#[test]
fn test_flow_halves_a_frames_budget_is_3000_glyph_steps_a_glyph() {
    use crate::flow::place::halves::{frame_budget, STEPS_PER_GLYPH};
    assert_eq!(STEPS_PER_GLYPH, 3_000, "a frame's budget a glyph");
    assert_eq!(frame_budget(10), 400_000, "a short frame's budget");
    assert_eq!(frame_budget(1_000), 3_000_000, "a long frame's budget");
    let work = the_sweep();
    assert!(
        work.most_per_glyph > 0 && work.most_per_glyph * 5 <= STEPS_PER_GLYPH,
        "the aspect sweep: {} glyph steps a glyph at most: the budget's {STEPS_PER_GLYPH} a glyph is ×{:.1} that, at least ×5",
        work.most_per_glyph,
        STEPS_PER_GLYPH as f64 / work.most_per_glyph.max(1) as f64
    );
}

/// R44.3: filling a frame's sides is work in proportion to what it
/// places, counted against its budget: the reviewer's case, a 200-byte
/// ornament at span natural round a 200,000 ellipse, and the same round a
/// 200,000 sharp square (review 5739829769).
///
/// With a budget that holds it (a floor of 2,000,000, the test seam;
/// some 720,000 and 1,256,000 glyph steps), each fills both arcs, some
/// 65,000 glyphs, in at most [`SIDE_LOOKUPS_A_GLYPH`] path lookups a glyph
/// placed (128 round the ellipse, 146 round the square; 784 at dd04a8f5,
/// when each fill bisected its gap in 30 lays of every copy), and looks at most at
/// [`PAIRS_A_BOX`] pairs of boxes a glyph for touching (6.5 round the
/// ellipse and 6.8 round the square, each box placed or copied into a
/// class's strips counted a look; 7.3 and 8.1 at 289a684f, when every
/// box shared the tallest's strip height; every pair, some two billion,
/// at dd04a8f5; round the square 5,272 a glyph at 1f1b3f8e, when a
/// sweep along one axis weighed every copy down a straight side against
/// every other).
///
/// At its own budget, 400,000, neither can afford its arcs and the touch
/// check of them (some 34,000 copies each, the check held back at
/// `TOUCH_LOOKS_A_BOX` looks a box): each lays its text whole, leaves both
/// arcs empty and reported — a frame loses its sides before its text
/// (ruling R48.3) — and takes no more past its budget than
/// `overshoot_steps` (rulings R44.2, R47.2, R48.2).
///
/// Controls: with the gap bisected in 30 lays again, past the lookups'
/// bound; with the sweep along one axis again, past the pairs' round the
/// square; with a side's lay unread past its first copy, past the
/// overshoot. (No `do_*` twin: it reads test counters.)
#[cfg(test)]
fn a_long_side_ornament(shape: Shape, what: &str) {
    use crate::flow::place::halves::{overshoot_steps, BOXES_WEIGHED, FLOOR_OVERRIDE, PAST_BUDGET};
    use crate::flow::place::SideLoss;
    use crate::flow::tests::flow_fuzz::ornament_run;
    use crate::flow::tests::support::style;
    let text = "To see a World in a Grain of Sand";
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(200_000.0)), shape);
    let layout = RingLayout {
        span: LineSpan::NATURAL,
        sides: Some(("x".repeat(200), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let lay = || frame_with(text, 16.0, &shape, &FitMode::Once { filler: None }, &layout, true);

    let looked = lookups_so_far();
    let weighed = BOXES_WEIGHED.with(std::cell::Cell::get);
    let (_, placed, trace) = with_seam(&FLOOR_OVERRIDE, 2_000_000, lay);
    let lookups = lookups_so_far() - looked;
    let pairs = BOXES_WEIGHED.with(std::cell::Cell::get) - weighed;
    let glyphs = placed.glyphs.len();
    assert!(
        pairs <= PAIRS_A_BOX * glyphs,
        "{what}: {pairs} pairs of boxes looked at for {glyphs} glyphs, {:.1} a glyph, at most {PAIRS_A_BOX}",
        pairs as f64 / glyphs as f64
    );
    assert!(
        trace.lines.len() == 2 && glyphs > 60_000 && trace.unheld.sides.is_empty(),
        "precondition, {what}: both arcs are filled ({glyphs} glyphs, {:?})",
        trace.unheld
    );
    assert!(
        lookups <= SIDE_LOOKUPS_A_GLYPH * glyphs,
        "{what}: {lookups} path lookups for {glyphs} glyphs, {:.1} a glyph, at most {SIDE_LOOKUPS_A_GLYPH}",
        lookups as f64 / glyphs as f64
    );

    let flow = style(16.0);
    let allowed = with_fonts(|fs| {
        let ornament = ornament_run(&layout, &flow, 1.0, fs);
        overshoot_steps(&real_run(text, &flow, fs), ornament.as_ref(), false)
    });
    PAST_BUDGET.with(|past| past.set(0));
    let (_, placed, trace) = lay();
    let past = PAST_BUDGET.with(std::cell::Cell::get);
    assert!(
        placed.glyphs.len() >= text.chars().filter(|c| !c.is_whitespace()).count()
            && trace.unheld.sides == [(0, SideLoss::Empty), (1, SideLoss::Empty)]
            && !placed.truncated,
        "{what}: at its own budget, which cannot hold the check of either arc, the text is laid whole and both arcs are left empty and reported ({} glyphs, {:?})",
        placed.glyphs.len(),
        trace.unheld
    );
    assert!(
        past <= allowed && trace.spent <= trace.budget + allowed,
        "{what}: {past} glyph steps past a spent budget, {} in all against {}, at most {allowed} past it",
        trace.spent,
        trace.budget
    );
}

/// [`a_long_side_ornament`] round the ellipse.
#[test]
fn test_flow_halves_a_long_side_ornament_is_filled_in_few_lays() {
    a_long_side_ornament(Shape::Ellipse, "ellipse");
}

/// [`a_long_side_ornament`] round the sharp square.
#[test]
fn test_flow_halves_a_long_side_ornament_round_a_sharp_square_is_filled_in_few_lays() {
    a_long_side_ornament(
        Shape::Rectangle {
            radius: Percent::new(0),
        },
        "sharp square",
    );
}

/// The most path lookups [`a_long_side_ornament`]'s frames may take a
/// glyph they place, both arcs filled: a few lays of each copy at a dozen
/// lookups a glyph, with a margin past the 128 (ellipse) and 146 (sharp
/// square) they take. (At 1f1b3f8e the ellipse's frame took 78 at its own
/// budget, where its second arc's floor row was laid, unread, after the
/// budget was spent and its gap never solved; with each copy's gap solved
/// by bisection, 784.)
#[cfg(test)]
const SIDE_LOOKUPS_A_GLYPH: usize = 200;

/// The most pairs of boxes the same frames may look at for touching a
/// glyph they place: the few inks near each, in the strips of its class
/// and the coarser ones round it — 6.5 a glyph round the ellipse and 6.8
/// round the square, so a margin of ×1.46 (7.3 and 8.1 at 289a684f, a
/// margin of ×1.24).
#[cfg(test)]
const PAIRS_A_BOX: usize = 10;

/// R44.5 (C63, held on every host): the quatrain at 22 pt inward round
/// the 600 × 12 sharp rectangle, `Once` at span ½ — the sweep's frame the
/// bisection of whole clearance searches was caught on — in the pinned
/// face, so its path through the last resorts is every host's. Its two
/// lines face each other across the outline, and the later is cut apart
/// (`Trace::cut_apart`); the cut takes at most [`CUT_APART_TRACKS`]
/// `track`s.
///
/// Control: `cut_apart` bisecting its prefix on whole clearance searches
/// again (C63), past the bound. (No `do_*` twin: it reads test counters.)
#[test]
fn test_flow_halves_a_frame_cut_apart_lays_a_bounded_number_of_lines() {
    use crate::flow::place::halves::TRACKS;
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        ..RingLayout::whole()
    };
    let before = TRACKS.with(std::cell::Cell::get);
    let (_, _, trace) = frame_with(
        QUATRAIN,
        22.0,
        &shape,
        &FitMode::Once { filler: None },
        &layout,
        false,
    );
    let tracks = TRACKS.with(std::cell::Cell::get) - before;
    assert!(
        !trace.cut_apart.is_empty(),
        "precondition: a line is cut apart ({tracks} tracks, {:?})",
        trace.unheld
    );
    assert!(
        tracks <= CUT_APART_TRACKS,
        "the frame cut apart takes {tracks} tracks, at most {CUT_APART_TRACKS}"
    );
}

/// The most `track`s [`test_flow_halves_a_frame_cut_apart_lays_a_bounded_number_of_lines`]'s
/// frame may take.
#[cfg(test)]
const CUT_APART_TRACKS: usize = 700;

/// R44.7: a letter moved clear of an ink before its neighbor is moved
/// only when the pairwise rule would move it — nearer than the gap less
/// what the rule accepts — so no such round over the aspect sweep leaves
/// a letter where it was, and three lookups for nothing.
///
/// Control: with an ink nearer than the gap itself moved from, as before
/// R44.7, rounds that leave the letter where it was. (No `do_*` twin: it
/// reads test counters.)
#[test]
fn test_flow_halves_a_letter_never_spends_a_round_where_it_stands() {
    let (moved, idle) = the_sweep().earlier;
    assert!(
        moved > 0,
        "precondition: the sweep moves letters clear of earlier inks"
    );
    assert_eq!(
        idle,
        0,
        "{idle} of {} rounds left a letter where it was",
        moved + idle
    );
}

/// R44.3: a side's equal gap is solved to copies that leave the text's
/// ink clear: the room copies leave falls with their gap until the last
/// one's ink meets the text's, then drops to none — the two inks touching
/// — or past it, so a probe there is read as a miss however near, never
/// taken for the answer, where copies a hair short of their room close
/// with a join a hair narrower than their gaps, as a line's end a hair
/// past its part's is laid. A 20-byte ornament between a 20,000
/// ellipse's first half's junctions, every count of copies from 160 to
/// the most that fit at the floor, some 175.
///
/// Control: with a touching miss read at its own room, which a solve
/// within its tolerance takes, 170 copies touch. (No `do_*` twin: it
/// reads a test seam.)
#[test]
fn test_flow_halves_a_sides_gap_is_solved_to_copies_that_fit() {
    use crate::flow::place::halves::side_solve_seam;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)), Shape::Ellipse);
    let flow = style(16.0);
    let ornament = "x".repeat(20);
    let solved: Vec<(usize, f32, bool)> = with_fonts(|fs| {
        let run = real_run(QUATRAIN, &flow, fs);
        (160..=180)
            .filter_map(|n| {
                side_solve_seam(&run, &shape, &ornament, n, &flow, fs).map(|(gap, touches)| (n, gap, touches))
            })
            .collect()
    });
    assert!(
        solved.len() > 10,
        "precondition: the counts fit at the floor ({})",
        solved.len()
    );
    let touching: Vec<_> = solved.iter().filter(|(_, _, touches)| *touches).collect();
    assert!(
        touching.is_empty(),
        "solved to copies that touch the text's ink: {touching:?}"
    );
}

/// The reviewer's long mixed side ornament (review 5745584645): one wide
/// glyph, `—`, and a hundred thin ones, `|`, whose wide glyph down a
/// vertical side stands as tall as it is wide among the thin ones. The
/// review named `⸻`; `—` is as flat, and the compiled faces hold it, so
/// every host lays the same geometry (ruling R51.5).
#[cfg(test)]
pub(crate) fn mixed_sides() -> String {
    format!("—{}", "|".repeat(100))
}

/// R50.2 (review 5745584645): a touch check's looks are output-sensitive,
/// the inks near each box whatever the tallest, so a long ornament that
/// mixes one wide glyph with thin ones looks at no more pairs a box than
/// measured ([`looks_as_measured`]), and so within `touch_looks` of its
/// classes: [`mixed_sides`] round a 20,000 sharp
/// square, and the fuzzable `"𓀀|||"` round a 2,000 one — "To see a World
/// in a Grain of Sand" at 16 pt, `Once` at span natural, outward, both
/// arcs filled (a floor of 40,000,000, the test seam), each glyph one the
/// compiled faces hold (ruling R51.5). With one strip height, the tallest
/// box's, for every box, the first looked at 21.11 pairs a box (821,262
/// over 38,910 boxes) with host faces and without; with `⸻` and `﷽`,
/// which no compiled face holds, 28.9 and 9.2 (review 5745584645), and
/// 25.36 on CI's faces, where they shaped as `.notdef`s.
///
/// Control: with every box in strips of the tallest's class, red. (No
/// `do_*` twin: it sets a test seam.)
#[test]
fn test_flow_halves_a_long_mixed_ornament_s_touch_check_looks_at_few_pairs_a_box() {
    use crate::flow::place::halves::FLOOR_OVERRIDE;
    let text = "To see a World in a Grain of Sand";
    for (sides, size) in [(mixed_sides(), 20_000.0), ("𓀀|||".to_string(), 2_000.0)] {
        let shape = outline(
            Rect::new(Vec2::ZERO, Vec2::splat(size)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        );
        let layout = RingLayout {
            span: LineSpan::NATURAL,
            sides: Some((sides.clone(), Rgba::opaque(0, 0, 0))),
            ..RingLayout::whole()
        };
        let (_, placed, trace) = with_seam(&FLOOR_OVERRIDE, 40_000_000, || {
            frame_with(text, 16.0, &shape, &FitMode::Once { filler: None }, &layout, true)
        });
        assert!(
            trace.unheld.sides.is_empty() && placed.glyphs.len() > 1_000,
            "precondition, {sides:?} round {size}: both arcs are filled ({} glyphs, {:?})",
            placed.glyphs.len(),
            trace.unheld
        );
        if let Err(why) = looks_as_measured(&trace.looks) {
            panic!("{sides:?} round {size}: {why}");
        }
    }
}

/// R51.2 (review 5751828253): a touch check weighs a box against every
/// coarser class of height it holds, so its looks a box grow with its
/// classes, and a text that mixes sizes over the whole font-size window
/// looks at no more pairs a box than measured, a part for each class
/// ([`looks_as_measured`]), and so within `touch_looks` of its classes:
/// the reviewer's 14-size probe ([`mixed_sizes`]), round a 2,000 sharp
/// square with 60 dots, `Fit` from ¼ to 4 and `Once`, and five times over
/// round a 20,000 one with 3,000 dots a span, `Once`; at span natural,
/// `x` on its sides, inward, each at its own budget. At R50, whose bound
/// was 18 a box whatever the classes, they looked at 23.86, 21.28 and
/// 23.46 pairs a box.
///
/// Control: with `touch_looks` 18 a box whatever the classes, as at R50,
/// red. (No `do_*` twin: it reads a frame's trace.)
#[test]
fn test_flow_halves_a_mixed_size_frame_s_touch_check_looks_within_its_classes() {
    let square = |size: f32| {
        outline(
            Rect::new(Vec2::ZERO, Vec2::splat(size)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        )
    };
    let layout = RingLayout {
        span: LineSpan::NATURAL,
        sides: Some(("x".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    for (name, text, size, fit) in [
        (
            "Fit",
            mixed_sizes(1, 60),
            2_000.0,
            FitMode::Fit { min: 0.25, max: 4.0 },
        ),
        (
            "Once",
            mixed_sizes(1, 60),
            2_000.0,
            FitMode::Once { filler: None },
        ),
        (
            "Once, five times over",
            mixed_sizes(5, 3_000),
            20_000.0,
            FitMode::Once { filler: None },
        ),
    ] {
        let (_, placed, trace) = frame_of_spans(&text, 16.0, &square(size), &fit, &layout, false);
        assert!(
            trace.looks.across.2 >= 14
                && trace.looks.across.0 > 0
                && trace.looks.classes >= 10
                && trace.looks.reserved > 0
                && placed.glyphs.len() > 60,
            "precondition, {name} round {size}: a touch check of the frame holds the text's 14 sizes' classes or more and looks between them, and its side arcs' checks hold ten or more and are held back for ({}; {} glyphs)",
            looks_ratios(&trace.looks),
            placed.glyphs.len()
        );
        if let Err(why) = looks_as_measured(&trace.looks) {
            panic!("{name} round {size}: {why}");
        }
    }
}

/// One case of [`test_flow_halves_the_touch_check_finds_what_every_pair_does`]:
/// ink boxes, each keyed by its group and its place in it — columns of
/// boxes stacked across an axis, a column's gaps half, just under, at and
/// just over `TOUCH`, straddling strips of every class; one box in 15
/// tall; and a few scattered — laid along x or along y.
#[cfg(test)]
fn touch_case(choices: &mut crate::util::tests::choices::Choices<'_>) -> Vec<((u8, usize), (Vec2, Vec2))> {
    use crate::flow::place::halves::TOUCH;
    let gaps = [0.5, 0.999, 1.0, 1.01, 1.01, 1.01];
    let unit =
        |choices: &mut crate::util::tests::choices::Choices<'_>| 0.02 + choices.below(200) as f32 / 64.0;
    // One gap down every column, and across between some of them.
    let gap = gaps[choices.index(gaps.len())] * TOUCH;
    let mut boxes = Vec::new();
    let columns = 2 + choices.below(9) as usize;
    let mut x = (choices.below(64) as f32 - 32.0) / 8.0;
    for column in 0..columns {
        let width = unit(choices);
        let mut y =
            (choices.below(64) as f32 - 32.0) / 8.0 + [0.0, 0.5 * TOUCH, -0.5 * TOUCH][choices.index(3)];
        let height = unit(choices) / 4.0;
        for _ in 0..2 + choices.below(12) {
            let tall = if choices.one_in(15) {
                height * (2 + choices.below(400)) as f32
            } else {
                height
            };
            let group = (column % 3) as u8;
            let at = boxes.iter().filter(|((g, _), _)| *g == group).count();
            boxes.push(((group, at), (Vec2::new(x, y), Vec2::new(x + width, y + tall))));
            y += tall + gap;
        }
        x += width
            + if choices.one_in(4) {
                gap
            } else {
                1.0 + choices.below(16) as f32
            };
    }
    for _ in 0..choices.below(3) {
        let at = Vec2::new(
            x + choices.below(64) as f32 / 4.0,
            choices.below(64) as f32 / 4.0 - 8.0,
        );
        let size = Vec2::new(unit(choices), unit(choices));
        let n = boxes.iter().filter(|((g, _), _)| *g == 2).count();
        boxes.push(((2, n), (at, at + size)));
    }
    if choices.one_in(2) {
        for (_, (min, max)) in &mut boxes {
            *min = Vec2::new(min.y, min.x);
            *max = Vec2::new(max.y, max.x);
        }
    }
    boxes
}

/// R50.1 (review 5745584645), R51.1 (review 5751828253): the touch
/// check finds two inks touching exactly where a look at every pair of
/// them does, under each of the three kinds of pair it excuses — none, a
/// line's neighbors (`overlapping`), and a side arc's (`place_halves`) —
/// the whole check, and its strips alone across each axis. The reviewer's
/// brute-force comparison, committed: 40,000 fixed-seed cases of
/// [`touch_case`]; then 20,000 of them at extreme scales
/// ([`extreme_touch_case`]): half far, a tall box 10²⁰ to 10³⁰ out across
/// the axis beside finer boxes at a moderate scale, whose strips lie past
/// `i64`'s range, where `stripped` saturates them — a twentieth or more
/// of the cases are held to reach that — and half at one scale from
/// 10⁻³ to 10³⁰, offsets to ±10³⁰, heights over 24 powers of two, among
/// boxes with a corner not a number, infinite or of no area; and the
/// reviewer's repro, a box 10²⁰ out across the axis beside finer ones. At extreme scales
/// every pair is looked at among the boxes with ink — every corner finite,
/// some width and some height, decided here — the strips are given those
/// alone, as `touching` gives them, and `touching` itself all of them.
///
/// Controls, each red: finer boxes not weighed against the coarser strip
/// before their own; a class's strips not weighed against the one after;
/// a coarser box's copies without the strip either side; the strips'
/// two-list sweep in one direction only; `stripped`'s strips round a
/// box at a plain `- 1` and `+ 1` (a panic, `attempt to add with
/// overflow`, among the far cases before the repro). (No `do_*` twin: it
/// reads a test seam.)
#[test]
fn test_flow_halves_the_touch_check_finds_what_every_pair_does() {
    use crate::flow::place::halves::{touch_seam, touching_seam, TOUCH};
    type Keyed = ((u8, usize), (Vec2, Vec2));
    let excusing = |kind: u32| {
        move |(a, i): (u8, usize), (b, j): (u8, usize)| match kind {
            0 => false,
            1 => a == b && i.abs_diff(j) <= 1,
            _ => {
                if a == 2 && b == 2 {
                    i.abs_diff(j) <= 1
                } else {
                    a != 2 && b != 2
                }
            }
        }
    };
    let gap = |a: (Vec2, Vec2), b: (Vec2, Vec2)| (b.0 - a.1).max(a.0 - b.1).max(Vec2::ZERO).length();
    let every = |boxes: &[Keyed], exempt: &dyn Fn((u8, usize), (u8, usize)) -> bool| {
        (0..boxes.len()).any(|i| {
            (i + 1..boxes.len())
                .any(|j| !exempt(boxes[i].0, boxes[j].0) && gap(boxes[i].1, boxes[j].1) <= TOUCH)
        })
    };
    // Whether a box has ink, as this test reads it: every corner finite,
    // some width and some height.
    let inked = |(min, max): (Vec2, Vec2)| {
        [min.x, min.y, max.x, max.y].iter().all(|v| v.is_finite()) && max.x > min.x && max.y > min.y
    };
    // The check, and its strips across each axis, against every pair: the
    // count of the cases that reached the strips and that touch.
    let agree = |what: &str, case: usize, kind: u32, given: &[Keyed]| {
        let exempt = excusing(kind);
        let boxes: Vec<Keyed> = given.iter().copied().filter(|b| inked(b.1)).collect();
        let every = every(&boxes, &exempt);
        let (found, _, stripped) = touch_seam(&boxes, &exempt, None);
        assert_eq!(
            found, every,
            "{what} case {case}, excusing kind {kind}: the check, {boxes:?}"
        );
        for axis in [0, 1] {
            let (found, _, _) = touch_seam(&boxes, &exempt, Some(axis));
            assert_eq!(
                found, every,
                "{what} case {case}, excusing kind {kind}: the strips across axis {axis}, {boxes:?}"
            );
        }
        assert_eq!(
            touching_seam(given.to_vec(), exempt),
            every,
            "{what} case {case}, excusing kind {kind}: `touching`, given every box, {given:?}"
        );
        (stripped > 0, every)
    };
    let mut choices = crate::util::tests::choices::Choices::seeded(0x0057_4558_4645);
    let (mut strips, mut touch) = (0, 0);
    let cases = 40_000;
    for case in 0..cases {
        let boxes = touch_case(&mut choices);
        let kind = choices.below(3);
        let (reached, touches) = agree("a", case, kind, &boxes);
        strips += usize::from(reached);
        touch += usize::from(touches);
    }
    assert!(
        strips * 4 >= cases && touch * 5 >= cases && (cases - touch) * 5 >= cases,
        "precondition: a quarter of the cases or more reach the strips ({strips} of {cases}), and a fifth or more touch ({touch}) and do not"
    );

    let mut choices = crate::util::tests::choices::Choices::seeded(0x0057_5182_8253);
    // Whether one of `boxes`' inks lies further across the sweep's axis
    // than 2⁶³ strips of the finest class, which is no taller than twice
    // the thinnest ink and `TOUCH`: where a strip index saturates.
    let saturates = |boxes: &[Keyed]| {
        let inked: Vec<(Vec2, Vec2)> = boxes.iter().map(|b| b.1).filter(|b| inked(*b)).collect();
        let spread = |axis: usize| {
            let (lo, hi) = inked
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), b| {
                    (lo.min(f64::from(b.0[axis])), hi.max(f64::from(b.1[axis])))
                });
            hi - lo
        };
        let across = if spread(1) > spread(0) { 0 } else { 1 };
        let thinnest = inked
            .iter()
            .map(|b| f64::from(b.1[across] - b.0[across]))
            .fold(f64::INFINITY, f64::min);
        let strip = 2.0 * (thinnest + f64::from(TOUCH)) * (1.0 + 1e-6);
        inked
            .iter()
            .any(|b| f64::from(b.0[across]).abs().max(f64::from(b.1[across]).abs()) / strip > 2f64.powi(63))
    };
    let (mut strips, mut touch, mut far, mut uninked) = (0, 0, 0, 0);
    let cases = 20_000;
    for case in 0..cases {
        let boxes = extreme_touch_case(&mut choices);
        let kind = choices.below(3);
        let (reached, touches) = agree("an extreme", case, kind, &boxes);
        strips += usize::from(reached);
        touch += usize::from(touches);
        far += usize::from(reached && saturates(&boxes));
        uninked += usize::from(reached && boxes.iter().any(|b| !inked(b.1)));
    }
    assert!(
        strips * 4 >= cases && far * 20 >= cases && uninked * 20 >= cases && touch * 10 >= cases && (cases - touch) * 10 >= cases,
        "precondition: at extreme scales a quarter of the cases or more reach the strips ({strips} of {cases}), a twentieth or more with an ink past 2⁶³ strips of the finest class across the axis ({far}) and among boxes with no ink ({uninked}), and a tenth or more touch ({touch}) and do not"
    );

    // The reviewer's repro: ten inks in a column of strips 2⁻⁶ tall across
    // y, one 10²⁰ out across it, and one 10²¹ along x, so x is the sweep's
    // axis and the far one's strips of the finer class lie past `i64`'s.
    let mut boxes: Vec<Keyed> = (0..10)
        .map(|k| {
            let y = 0.02 * k as f32;
            ((0, k), (Vec2::new(0.0, y), Vec2::new(5.0, y + 0.01)))
        })
        .collect();
    boxes.push(((1, 0), (Vec2::new(0.0, 1e20), Vec2::new(5.0, 1e20 + 1e14))));
    boxes.push(((1, 1), (Vec2::new(1e21, 0.0), Vec2::new(2e21, 0.01))));
    for kind in 0..3 {
        let (reached, _) = agree("the repro", 0, kind, &boxes);
        assert!(reached, "precondition: the repro reaches the strips");
    }
}

/// R51.4 (review 5751828253, ruling R49.2): a frame's touch check
/// leaves out a box with no ink, so it touches nothing. Two coincident
/// boxes of no height, which would touch were they a point tall, do not;
/// and a box with a corner that is not a number, laid among three inked
/// boxes apart from one another where with that corner a number it
/// would touch two of them, touches none — `box_gap` reads the component
/// that is not a number as no distance.
///
/// Control: `has_ink` returning `true`, red on both. (No `do_*` twin: it
/// reads a test seam.)
#[test]
fn test_flow_halves_a_box_with_no_ink_touches_nothing() {
    use crate::flow::place::halves::touching_seam;
    let never = |_: usize, _: usize| false;
    let flat = (Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0));
    let tall = (Vec2::new(0.0, 0.0), Vec2::new(5.0, 1.0));
    assert!(
        touching_seam(vec![(0, tall), (1, tall)], never),
        "precondition: the boxes, a point tall, touch"
    );
    assert!(
        !touching_seam(vec![(0, flat), (1, flat)], never),
        "two coincident boxes of no height touched"
    );

    let inked: Vec<(usize, (Vec2, Vec2))> = (0..3)
        .map(|k| {
            let y = 3.0 * k as f32;
            (k, (Vec2::new(0.0, y), Vec2::new(5.0, y + 1.0)))
        })
        .collect();
    let across = |x: f32| (3, (Vec2::new(x, 0.5), Vec2::new(5.0, 3.5)));
    assert!(
        !touching_seam(inked.clone(), never),
        "precondition: the inked boxes are apart"
    );
    assert!(
        touching_seam([inked.clone(), vec![across(0.0)]].concat(), never),
        "precondition: the box across them, its corner a number, touches them"
    );
    assert!(
        !touching_seam([inked, vec![across(f32::NAN)]].concat(), never),
        "a box with a corner not a number touched the inked boxes"
    );
}

/// Ink boxes, each keyed by its group and its place in it, as
/// [`touch_case`] draws them.
#[cfg(test)]
type KeyedBoxes = Vec<((u8, usize), (Vec2, Vec2))>;

/// One case of [`test_flow_halves_the_touch_check_finds_what_every_pair_does`]
/// at an extreme scale (R51.1, reviews 5751828253 and 5757429244): a
/// [`touch_case`] and up to two boxes beside it as tall across y as one
/// of its by a power of two to 2²³. One case in two is far: the case at
/// a scale of 10⁻³, 1 or 10³, its tall boxes moved across y by ±10²⁰ to
/// ±10³⁰ of their own, and a box ten times as far along x — so a coarse
/// box lies past `i64`'s range of the finer classes' strips, where
/// `stripped` saturates them (a uniform scale never reaches it: the strips
/// scale with the boxes). The rest are scaled by one of 10⁻³, 1, 10³,
/// 10⁶, 10¹⁸ and 10³⁰ and moved along each axis, by up to 40,000 times the
/// scale or, one time in four, by up to ±10³⁰, among copies of their
/// boxes with a corner made not a number or infinite, or made of no width
/// or no height.
#[cfg(test)]
fn extreme_touch_case(choices: &mut crate::util::tests::choices::Choices<'_>) -> KeyedBoxes {
    const SCALES: [f32; 6] = [1e-3, 1.0, 1e3, 1e6, 1e18, 1e30];
    const OFFSETS: [f32; 7] = [0.0, 1e6, -1e6, 1e18, -1e18, 1e30, -1e30];
    let mut boxes = touch_case(choices);
    let cased = boxes.len();
    let right = boxes
        .iter()
        .map(|(_, (_, max))| max.x)
        .fold(f32::NEG_INFINITY, f32::max);
    for _ in 0..choices.below(3) {
        let at = Vec2::new(
            right + choices.below(64) as f32 / 4.0,
            choices.below(64) as f32 / 4.0 - 8.0,
        );
        let height = (0.02 + choices.below(200) as f32 / 64.0) / 4.0 * (1u32 << choices.below(24)) as f32;
        let n = boxes.iter().filter(|((g, _), _)| *g == 1).count();
        boxes.push(((1, n), (at, at + Vec2::new(0.5, height))));
    }
    // One case in two is far: its tall boxes moved across y by an offset of
    // their own, 10²⁰ to 10³⁰ out whatever the scale, beside the case's
    // boxes at a moderate scale, with one box ten times as far along x, so
    // that x is the sweep's axis — strips of the finer classes some 2⁶³ or
    // more out across it, past `i64`'s range, as the reviewer's repro lays
    // them (review 5757429244).
    if choices.one_in(2) {
        let far =
            [1e20_f32, 1e22, 1e25, 1e28, 1e30][choices.index(5)] * if choices.one_in(2) { -1.0 } else { 1.0 };
        let scale = [1e-3_f32, 1.0, 1e3][choices.index(3)];
        for (k, (_, (min, max))) in boxes.iter_mut().enumerate() {
            *min *= scale;
            *max *= scale;
            if k >= cased {
                // As tall as it was in proportion to how far out it is, so
                // that an `f32` there still gives it some height.
                let tall = (max.y - min.y) / scale * far.abs() * 1e-6;
                min.y = far;
                max.y = far + tall;
            }
        }
        let n = boxes.iter().filter(|((g, _), _)| *g == 1).count();
        let along = 10.0 * far.abs();
        boxes.push(((1, n), (Vec2::new(along, 0.0), Vec2::new(2.0 * along, 0.01))));
        return boxes;
    }
    let scale = SCALES[choices.index(SCALES.len())];
    // Mostly an offset a few thousand of the case's units out, which keeps
    // its boxes apart in an `f32`; else one of `OFFSETS`, which at a
    // smaller scale lays them all on one another, of no area.
    let mut offset = || {
        if choices.one_in(4) {
            OFFSETS[choices.index(OFFSETS.len())]
        } else {
            scale * [0.0, 1e3, -1e3, 4e4, -4e4][choices.index(5)]
        }
    };
    let offset = Vec2::new(offset(), offset());
    for (_, (min, max)) in &mut boxes {
        *min = *min * scale + offset;
        *max = *max * scale + offset;
    }
    for _ in 0..choices.below(4) {
        let (key, (mut min, mut max)) = boxes[choices.index(boxes.len())];
        let corner = choices.index(4);
        let value = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY][choices.index(3)];
        match choices.below(3) {
            0 => match corner {
                0 => min.x = value,
                1 => min.y = value,
                2 => max.x = value,
                _ => max.y = value,
            },
            1 => max.x = min.x,
            _ => max.y = min.y,
        }
        let n = boxes.iter().filter(|((g, _), _)| *g == key.0).count();
        boxes.push(((key.0, n), (min, max)));
    }
    boxes
}

/// R50.5 (review 5745584645): a side arc whose search for its count the
/// frame's budget stopped keeps the copies it had where they hold the
/// equal-gap rule, and is reported kept short, apart from an arc left
/// empty: "To see a World in a Grain of Sand" at 16 pt round a 2,000
/// sharp square, `Once` at span ½ with an `=` on its sides (ruling R51.5:
/// a glyph the compiled faces hold, wider than tall, where `❧` was one no
/// compiled face holds), outward, whose
/// count search moves from its guess, at budgets from 5,000 to 8,000 in
/// steps of 5. The first budget at which an arc is kept short places more
/// glyphs than the last before it at which that arc was left empty, the
/// other arc as it was.
///
/// Control: with a stopped search's kept copies not reported, red. (No
/// `do_*` twin: it sets a test seam.)
#[test]
fn test_flow_halves_an_arc_kept_short_is_reported() {
    use crate::flow::place::halves::BUDGET_OVERRIDE;
    use crate::flow::place::SideLoss;
    let text = "To see a World in a Grain of Sand";
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::splat(2_000.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("=".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let fit = FitMode::Once { filler: None };
    let mut frames = Vec::new();
    for budget in (5_000..8_000).step_by(5) {
        let (_, placed, trace) = with_seam(&BUDGET_OVERRIDE, budget, || {
            frame_with(text, 16.0, &shape, &fit, &layout, true)
        });
        let short = trace
            .unheld
            .sides
            .iter()
            .find(|s| s.1 == SideLoss::Short)
            .map(|s| s.0);
        frames.push((budget, trace.unheld.sides, placed.glyphs.len()));
        let Some(arc) = short else {
            continue;
        };
        let (budget, sides, glyphs) = frames.pop().unwrap_or_default();
        let empty: Vec<_> = sides
            .iter()
            .map(|&(j, loss)| if j == arc { (j, SideLoss::Empty) } else { (j, loss) })
            .collect();
        let before = frames.iter().rev().find(|f| f.1 == empty);
        let Some((at, _, fewer)) = before else {
            panic!("precondition: a budget below {budget} leaves arc {arc} empty, the other as at {budget} ({sides:?})");
        };
        assert!(
            glyphs > *fewer,
            "arc {arc} is reported kept short at a budget of {budget} ({sides:?}) and places {glyphs} glyphs, no more than the {fewer} placed at {at}, where it was left empty"
        );
        return;
    }
    panic!(
        "no arc is reported kept short at any budget from 5,000 to 8,000: {:?}",
        frames
            .iter()
            .map(|f| (f.0, f.1.clone()))
            .collect::<Vec<_>>()
            .last()
    );
}

/// R51.3 (review 5751828253): an ordinary line of one gap, two letters
/// at 16 pt laid outward round a circle 40 across, over parts of its
/// first half 15 to 35 long, is solved to its part's end, within the
/// line's tolerance. Round a bend, outward, a gap moves the line's end
/// by less than the gap, so one gap as long as all the room the line has
/// at no gap still leaves it short — where the tracking solve's stop for
/// a line no gap moves once fired and left it points short (precondition)
/// — but less short than at no gap, so the stop does not fire.
///
/// Control: the stop as 03ac0b3f wrote it, without `value >= low.1`, red:
/// the line is left points short of its end. (No `do_*` twin: it reads a
/// test seam.)
#[test]
fn test_flow_halves_a_one_gap_line_on_a_curve_is_solved_to_its_end() {
    use crate::flow::place::halves::solve_line_seam;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(40.0)), Shape::Ellipse);
    let mut stopped = 0;
    for room in [15.0, 20.0, 25.0, 30.0, 35.0] {
        let (gap, short, tolerance, at_zero, at_room) = with_fonts(|fs| {
            let run = real_run("ab", &style(16.0), fs);
            let fs: &mut FontSystem = fs;
            solve_line_seam(&run, &shape, room, fs)
        });
        assert!(
            at_zero > 0.0 && at_room > tolerance && at_room < at_zero,
            "precondition: over a part {room} long the line is short at no gap ({at_zero}), and at a gap of all that room still short ({at_room}), but less"
        );
        stopped += usize::from(at_room > 1.0);
        assert!(
            short.abs() <= tolerance,
            "over a part {room} long the line is solved to gap {gap}, {short} short of its end, past its tolerance of {tolerance}"
        );
    }
    assert!(
        stopped >= 3,
        "precondition: over three parts or more a gap of all the room leaves the line a point or more short ({stopped})"
    );
}

/// A frame's line tracked where no gap moves its end, or where a gap that
/// would places its glyphs past every `f32`, keeps a lay placed finitely
/// (seeds 2970 and 4284 of the structured flow generator, with a host's
/// faces: a part some 10³³ along its half, where an `f32` sets every
/// letter at one center, so that the room the line leaves never shrank;
/// the tracking solve widened its gap past every `f32`, and the line was
/// placed at `NaN`). The solve stops once one gap is as long as all the
/// room the line had at no gap and leaves it no less short than the probe
/// before, and reads a lay placed past every `f32` as past its part's end
/// ([`solve_tracking_seam`]).
///
/// Controls: without the stop, the first line's gap runs to 10³⁸; with
/// a lay placed past every `f32` read as any other, the second line is
/// placed at infinity. (No `do_*` twin: it reads a test seam.)
///
/// [`solve_tracking_seam`]: crate::flow::place::halves::solve_tracking_seam
#[test]
fn test_flow_halves_a_line_no_gap_can_move_keeps_a_finite_lay() {
    use crate::flow::place::halves::solve_tracking_seam;
    let room = 1.4e33_f32;
    let (gap, pos) = solve_tracking_seam(|g| (room, Vec2::new(g * 10.0, 0.0)), 7, 5e33);
    assert!(
        pos.is_finite() && gap <= 4.0 * room,
        "a line no gap moves: gap {gap}, its glyph at {pos}, against a room of {room}"
    );
    let (gap, pos) = solve_tracking_seam(|g| (1e38 - g * 1e-3, Vec2::new(g * 10.0, 0.0)), 7, 1e38);
    assert!(
        pos.is_finite(),
        "a line a gap moves by less than it places its glyph: gap {gap}, its glyph at {pos}"
    );
}
