// SPDX-License-Identifier: MPL-2.0

//! A text frame in two lines (#291, ruling R27): a ring whose fit mode
//! is `Once`, `Spread` or `Fit` reads over the top and along the bottom,
//! each line on the level-dominant part of its half of the outline, the
//! steep sides clear. Runs are shaped for real. Whether a glyph keeps
//! off the steep sides is judged from the outline's own tangent where
//! the glyph's center sits, worked here from the ellipse's equation or
//! the rectangle's corners — nothing the placement computed.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::place::{frame_halves, place_on_closed_traced, table_spacing, Trace, TracedLine};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, PlacedRun, Rect, ShapedRun};

/// A sentence long enough to want two lines.
const VERSE: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower";

/// The frame around the README's Ecclesiastes center node.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven:";

/// Letters with no word break: one word.
const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";

const W: f32 = 420.0;
const H: f32 = 200.0;

fn ellipse() -> Outline {
    outline(Rect::new(Vec2::ZERO, Vec2::new(W, H)), Shape::Ellipse)
}

/// A rounded rectangle of corner radius 25% of half its shorter side.
fn rounded() -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(W, H)),
        Shape::Rectangle {
            radius: Percent::new(25),
        },
    )
}

/// A text frame placed, the run it refers to, and its trace.
struct Frame {
    run: ShapedRun,
    placed: PlacedRun,
    trace: Trace,
    /// The halves' spans, as the placement found them.
    spans: [std::ops::Range<f32>; 2],
}

/// `text` at 12 pt as a text frame on `outline`, glyph centers on it.
fn frame(text: &str, outline: &Outline, fit: &FitMode) -> Frame {
    let mut trace = Trace::default();
    let flow = style(12.0);
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    let (run, rescaled, placed, spacing) = with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        let (rescaled, placed) = place_on_closed_traced(
            &run,
            outline,
            start,
            Direction::Clockwise,
            fit,
            false,
            &flow,
            fs,
            Some(&mut trace),
        );
        let spacing = table_spacing(&run, fit, &flow);
        (run, rescaled, placed, spacing)
    });
    let spans = frame_halves(outline, spacing).map(|(_, span)| span);
    Frame {
        run: rescaled.unwrap_or(run),
        placed,
        trace,
        spans,
    }
}

/// `|cos θ|` of the outline's own tangent at `p`, a point on it: from
/// the ellipse's equation, or from where `p` lies on the rectangle — a
/// straight side, or a corner arc around its center.
fn level(outline: &Outline, p: Vec2) -> f32 {
    let (rect, radius) = match *outline {
        Outline::Ellipse { rect } => (rect, None),
        Outline::RoundedRect { rect, radius } => (rect, Some(radius)),
    };
    let center = rect.origin + rect.size * 0.5;
    let d = p - center;
    match radius {
        None => {
            let (a, b) = (rect.size.x * 0.5, rect.size.y * 0.5);
            let tangent = Vec2::new(-d.y / (b * b), d.x / (a * a));
            (tangent.x / tangent.length()).abs()
        }
        Some(radius) => {
            let half = rect.size * 0.5 - Vec2::splat(radius);
            if d.x.abs() <= half.x {
                1.0
            } else if d.y.abs() <= half.y {
                0.0
            } else {
                let corner = center + Vec2::new(half.x * d.x.signum(), half.y * d.y.signum());
                let r = p - corner;
                (r.y / r.length()).abs()
            }
        }
    }
}

/// Each line's cluster centers, in text order: a cluster begins at each
/// glyph that begins a grapheme; its box is its glyphs' advances wide.
fn line_centers(frame: &Frame, line: &TracedLine) -> Vec<Vec2> {
    let glyphs = &frame.placed.glyphs[line.glyphs.clone()];
    let mut centers = Vec::new();
    let mut i = 0;
    while i < glyphs.len() {
        let mut end = i + 1;
        while end < glyphs.len() && !frame.run.glyphs[glyphs[end].glyph as usize].grapheme_start {
            end += 1;
        }
        let metrics = |k: usize| &frame.run.glyphs[glyphs[k].glyph as usize];
        let left = (i..end).map(|k| metrics(k).x).fold(f32::INFINITY, f32::min);
        let width: f32 = (i..end).map(|k| metrics(k).advance).sum();
        let corner = glyphs[i].pos - Vec2::new(metrics(i).x - left, 0.0);
        centers.push(corner + Vec2::new(width, frame.run.line_height) * 0.5);
        i = end;
    }
    centers
}

/// The text a line holds, read back from its glyphs' bytes.
fn line_text(frame: &Frame, text: &str, line: &TracedLine) -> String {
    let mut bytes: Vec<std::ops::Range<usize>> = frame.placed.glyphs[line.glyphs.clone()]
        .iter()
        .map(|g| frame.run.glyphs[g.glyph as usize].bytes.clone())
        .collect();
    bytes.dedup();
    bytes.into_iter().map(|range| &text[range]).collect()
}

/// Both lines read left to right — cluster centers move right in text
/// order — on the level-dominant part of the outline: at every center
/// the outline's tangent is at most 45° from level. The lay's steps keep
/// within their half's span and never overlap.
fn assert_lines(what: &str, outline: &Outline, frame: &Frame) {
    assert_eq!(frame.trace.lines.len(), 2, "{what}: two lines");
    for line in &frame.trace.lines {
        let centers = line_centers(frame, line);
        for pair in centers.windows(2) {
            assert!(
                pair[1].x > pair[0].x,
                "{what}: line {} reads right to left: {} then {}",
                line.half,
                pair[0],
                pair[1]
            );
        }
        for c in &centers {
            assert!(
                level(outline, *c) >= std::f32::consts::FRAC_1_SQRT_2 - 1e-3,
                "{what}: a glyph of line {} sits on a steep side, at {c}",
                line.half
            );
        }
        let span = &frame.spans[line.half];
        assert!(
            line.from >= span.start - 1e-3 && line.to <= span.end + 0.05,
            "{what}: line {} lies at {}..{}, outside its span {span:?}",
            line.half,
            line.from,
            line.to
        );
    }
    // Steps on one half never overlap; the halves share no part of the
    // outline, so a step on each may hold the same arc length.
    assert_eq!(
        frame.trace.laid_on.len(),
        frame.trace.laid.len(),
        "{what}: every step on a half"
    );
    for half in 0..2 {
        let mut laid: Vec<(f32, f32)> = frame
            .trace
            .laid
            .iter()
            .zip(&frame.trace.laid_on)
            .filter(|(_, on)| **on == half)
            .map(|(step, _)| *step)
            .collect();
        laid.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in laid.windows(2) {
            let ((s, step), (next, _)) = (pair[0], pair[1]);
            assert!(
                next >= s + step - 1e-3,
                "{what}: steps at {s} and {next} on half {half} overlap"
            );
        }
    }
}

#[test]
fn test_flow_lines_balance_a_sentence() {
    do_flow_lines_balance_a_sentence();
}

/// A sentence on an ellipse and on a rounded rectangle, in `Once` and
/// `Fit`, is set in two lines, over the top and along the bottom, each
/// reading left to right on the level-dominant part of the outline, the
/// steep sides clear (ruling R27). The lines are parted where they best
/// balance: of every word break, the one whose larger fill ratio — a
/// line's advances over its span — is least, worked here from the text's
/// words.
pub fn do_flow_lines_balance_a_sentence() {
    for (what, shape) in [("ellipse", ellipse()), ("rounded rect", rounded())] {
        for fit in [
            FitMode::Once { filler: None },
            FitMode::Fit { min: 0.5, max: 2.0 },
        ] {
            let what = format!("{what} {fit:?}");
            let placed = frame(VERSE, &shape, &fit);
            assert_lines(&what, &shape, &placed);
            assert!(!placed.placed.truncated, "{what}: truncated");
            let top = line_text(&placed, VERSE, &placed.trace.lines[0]);
            let bottom = line_text(&placed, VERSE, &placed.trace.lines[1]);
            assert_eq!(
                format!("{top} {bottom}"),
                VERSE,
                "{what}: the lines hold the text"
            );
            // Every split at a space, judged by the run's own advances.
            let width = |text: &str| {
                let run = with_fonts(|fs| real_run(text, &style(12.0), fs));
                run.glyphs.iter().map(|g| g.advance).sum::<f32>()
            };
            let spans = placed.spans.clone().map(|span| span.end - span.start);
            let worst = |top: &str, bottom: &str| (width(top) / spans[0]).max(width(bottom) / spans[1]);
            let chosen = worst(&top, &bottom);
            for (at, _) in VERSE.match_indices(' ') {
                let other = worst(&VERSE[..at], &VERSE[at + 1..]);
                assert!(
                    chosen <= other + 1e-4,
                    "{what}: parted at {top:?} / {bottom:?} ({chosen}), but {:?} balances better ({other})",
                    &VERSE[..at]
                );
            }
        }
    }
}

#[test]
fn test_flow_lines_part_at_a_line_break() {
    do_flow_lines_part_at_a_line_break();
}

/// A line break in the text parts the lines there, whatever balances
/// best (ruling R27): "and a time to every purpose" over "under the
/// heaven:". The balanced split of the same words, a space for the
/// break, is checked to fall elsewhere.
pub fn do_flow_lines_part_at_a_line_break() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let broken = "and a time to every purpose\nunder the heaven:";
    let placed = frame(broken, &ellipse(), &fit);
    assert_lines("a line break", &ellipse(), &placed);
    let balanced = frame(ECCLESIASTES, &ellipse(), &fit);
    let balanced_top = line_text(&balanced, ECCLESIASTES, &balanced.trace.lines[0]);
    assert_ne!(
        balanced_top, "and a time to every purpose",
        "the fixture needs a balance that parts elsewhere"
    );
    assert_eq!(
        line_text(&placed, broken, &placed.trace.lines[0]),
        "and a time to every purpose"
    );
    assert_eq!(
        line_text(&placed, broken, &placed.trace.lines[1]),
        "under the heaven:"
    );
}

#[test]
fn test_flow_lines_keep_one_word_on_top() {
    do_flow_lines_keep_one_word_on_top();
}

/// A text with no word break is one line, on top; the bottom line is
/// empty (ruling R27).
pub fn do_flow_lines_keep_one_word_on_top() {
    let placed = frame(LETTERS, &ellipse(), &FitMode::Fit { min: 0.1, max: 2.0 });
    assert_lines("one word", &ellipse(), &placed);
    assert!(
        placed.trace.lines[1].glyphs.is_empty(),
        "the bottom line is empty"
    );
    assert_eq!(line_text(&placed, LETTERS, &placed.trace.lines[0]), LETTERS);
}

#[test]
fn test_flow_lines_fit_to_the_fuller_line() {
    do_flow_lines_fit_to_the_fuller_line();
}

/// `Fit` sets both lines at one size — one reshape of the run — the
/// largest at which each fits its span with no gap (ruling R27): both
/// lines lie in their spans, and the fuller line fills its span — to
/// within half a point — so a larger size would overrun it.
pub fn do_flow_lines_fit_to_the_fuller_line() {
    for (what, shape) in [("ellipse", ellipse()), ("rounded rect", rounded())] {
        let placed = frame(VERSE, &shape, &FitMode::Fit { min: 0.1, max: 10.0 });
        assert_lines(what, &shape, &placed);
        assert!(!placed.placed.truncated, "{what}: truncated");
        let size = placed.run.glyphs[0].size;
        assert!((size - 12.0).abs() > 0.1, "{what}: the fixture needs a rescale");
        let slack = placed
            .trace
            .lines
            .iter()
            .map(|line| (placed.spans[line.half].end - placed.spans[line.half].start) - (line.to - line.from))
            .fold(f32::INFINITY, f32::min);
        assert!(slack <= 0.5, "{what}: the fuller line leaves {slack} of its span");
    }
}

#[test]
fn test_flow_lines_cut_the_bottom_line_at_min() {
    do_flow_lines_cut_the_bottom_line_at_min();
}

/// Text too long for its spans even at `Fit`'s `min` keeps the top line
/// whole and cuts the bottom line's end, at a grapheme boundary, and says
/// so (ruling R27; §6.4 step 4): the bottom line is a prefix of the text
/// after the top line's.
pub fn do_flow_lines_cut_the_bottom_line_at_min() {
    let text = "the quick brown fox jumps over the lazy dog while the poem goes around and around and around";
    let small = outline(Rect::new(Vec2::ZERO, Vec2::new(120.0, 60.0)), Shape::Ellipse);
    let placed = frame(text, &small, &FitMode::Fit { min: 0.9, max: 2.0 });
    // At `min`, the text's advances alone outrun both spans together.
    let natural: f32 = with_fonts(|fs| real_run(text, &style(12.0), fs))
        .glyphs
        .iter()
        .map(|g| g.advance)
        .sum();
    let room: f32 = placed.spans.iter().map(|span| span.end - span.start).sum();
    assert!(natural * 0.9 > room, "the fixture needs text too long at min");
    let top = line_text(&placed, text, &placed.trace.lines[0]);
    let bottom = line_text(&placed, text, &placed.trace.lines[1]);
    assert!(!placed.trace.lines[0].cut, "the top line is whole");
    assert!(placed.trace.lines[1].cut, "the bottom line is cut");
    assert!(placed.placed.truncated, "the cut is not reported");
    let rest = text[top.len()..].trim_start();
    assert!(
        rest.starts_with(&bottom) && bottom.len() < rest.len(),
        "{bottom:?} is not a cut of {rest:?}"
    );
}

#[test]
fn test_flow_lines_spread_over_their_spans() {
    do_flow_lines_spread_over_their_spans();
}

/// `Spread` spreads each line over its whole span (ruling R27): its
/// first step starts at the span's start and its last ends at the
/// span's end, within half a point.
pub fn do_flow_lines_spread_over_their_spans() {
    for (what, shape) in [("ellipse", ellipse()), ("rounded rect", rounded())] {
        let placed = frame(VERSE, &shape, &FitMode::Spread);
        assert_lines(what, &shape, &placed);
        for line in &placed.trace.lines {
            let span = &placed.spans[line.half];
            assert!(
                (line.from - span.start).abs() <= 0.5,
                "{what}: line {} starts at {}",
                line.half,
                line.from
            );
            assert!(
                (line.to - span.end).abs() <= 0.5,
                "{what}: line {} ends at {}",
                line.half,
                line.to
            );
        }
    }
}

#[test]
fn test_flow_lines_center_and_fill() {
    do_flow_lines_center_and_fill();
}

/// `Once` centers each line on its span, at the declared size, and a
/// filler fills the rest of each span on both sides of each line, in
/// whole copies, never touching it (ruling R27).
pub fn do_flow_lines_center_and_fill() {
    let shape = ellipse();
    let placed = frame(VERSE, &shape, &FitMode::Once { filler: None });
    assert!(
        (placed.run.glyphs[0].size - 12.0).abs() < 1e-4,
        "the declared size"
    );
    for line in &placed.trace.lines {
        let span = &placed.spans[line.half];
        let (before, after) = (line.from - span.start, span.end - line.to);
        assert!(
            (before - after).abs() <= 0.5,
            "line {} is not centered: {before} / {after}",
            line.half
        );
    }
    let filled = frame(
        VERSE,
        &shape,
        &FitMode::Once {
            filler: Some("·".to_owned()),
        },
    );
    assert_lines("filled", &shape, &filled);
    // The lines' steps come first, one per cluster; the filler's after.
    let text: usize = filled
        .trace
        .lines
        .iter()
        .map(|line| {
            let glyphs = &filled.placed.glyphs[line.glyphs.clone()];
            glyphs
                .iter()
                .filter(|g| filled.run.glyphs[g.glyph as usize].grapheme_start)
                .count()
        })
        .sum();
    let filler: Vec<((f32, f32), usize)> = filled.trace.laid[text..]
        .iter()
        .copied()
        .zip(filled.trace.laid_on[text..].iter().copied())
        .collect();
    assert!(!filler.is_empty(), "the fixture needs a filler placed");
    for line in &filled.trace.lines {
        let (span, words) = (&filled.spans[line.half], (line.from, line.to));
        let on_half = |s: f32, on: usize| on == line.half && s >= span.start - 1e-3 && s <= span.end + 1e-3;
        let before = filler
            .iter()
            .filter(|((s, step), on)| on_half(*s, *on) && s + step <= words.0 + 1e-3)
            .count();
        let after = filler
            .iter()
            .filter(|((s, _), on)| on_half(*s, *on) && *s >= words.1 - 1e-3)
            .count();
        assert!(
            before > 0 && after > 0,
            "line {}: filler before {before}, after {after}",
            line.half
        );
    }
}

/// A sharp rectangle, the default frame's outline.
fn sharp(w: f32, h: f32) -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(w, h)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    )
}

#[test]
fn test_flow_lines_fit_sweeps_a_sharp_rect() {
    do_flow_lines_fit_sweeps_a_sharp_rect();
}

/// `Fit` on the default frame's outline, a sharp rectangle, and on an
/// ellipse, over every prefix of a sentence from 5 to 68 characters,
/// shaped for real: no prefix is cut, both lines lie in their spans
/// without overlap, and either the scale is held at `max` or the fuller
/// line fills its span to within half a point (rulings R9.3, R27).
pub fn do_flow_lines_fit_sweeps_a_sharp_rect() {
    let sentence = "the quick brown fox jumps over the lazy dog while the poem goes around";
    let max = 4.0;
    let fit = FitMode::Fit { min: 0.1, max };
    for (what, shape) in [("sharp rect", sharp(300.0, 120.0)), ("ellipse", ellipse())] {
        for n in 5..=68 {
            let text = &sentence[..n];
            let what = format!("{what} {text:?}");
            let placed = frame(text, &shape, &fit);
            assert!(!placed.placed.truncated, "{what}: truncated");
            assert_lines(&what, &shape, &placed);
            let scale = placed.run.glyphs[0].size / 12.0;
            let slack = placed
                .trace
                .lines
                .iter()
                .filter(|line| !line.glyphs.is_empty())
                .map(|line| {
                    (placed.spans[line.half].end - placed.spans[line.half].start) - (line.to - line.from)
                })
                .fold(f32::INFINITY, f32::min);
            assert!(
                (scale - max).abs() < 1e-3 || slack <= 0.5,
                "{what}: scale {scale} below max, yet the fuller line leaves {slack}"
            );
        }
    }
}

#[test]
fn test_flow_lines_spread_sweeps_a_sharp_rect() {
    do_flow_lines_spread_sweeps_a_sharp_rect();
}

/// `Spread` on a sharp rectangle over a sweep of runs — three advances,
/// 26 lengths each, all short enough to fit — rather than one fixture:
/// every run is placed whole, and each line's last step ends at its
/// span's end within half a point (rulings R9.2, R27).
pub fn do_flow_lines_spread_sweeps_a_sharp_rect() {
    let shape = sharp(300.0, 120.0);
    for w in [5.0_f32, 6.0, 7.5] {
        for n in (10..=60).step_by(2) {
            let run = crate::flow::tests::support::synthetic(&vec![w; n], 14.0);
            let mut trace = Trace::default();
            let start = StartPoint {
                edge: Edge::Top,
                at: 0.5,
            };
            let (_, placed) = with_fonts(|fs| {
                place_on_closed_traced(
                    &run,
                    &shape,
                    start,
                    Direction::Clockwise,
                    &FitMode::Spread,
                    false,
                    &style(12.0),
                    fs,
                    Some(&mut trace),
                )
            });
            let spans = frame_halves(&shape, table_spacing(&run, &FitMode::Spread, &style(12.0)))
                .map(|(_, span)| span);
            let what = format!("w {w}, n {n}");
            assert!(!placed.truncated, "{what}: truncated");
            assert_eq!(placed.glyphs.len(), n, "{what}");
            for line in &trace.lines {
                let span = &spans[line.half];
                assert!(
                    (line.from - span.start).abs() <= 0.5,
                    "{what}: line {} starts at {}",
                    line.half,
                    line.from
                );
                assert!(
                    (line.to - span.end).abs() <= 0.5,
                    "{what}: line {} ends at {}",
                    line.half,
                    line.to
                );
            }
        }
    }
}
