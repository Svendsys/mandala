// SPDX-License-Identifier: MPL-2.0

//! Fit modes (#239 §6, ruling R6): each mode's end condition. `Once`
//! places the text once and fills the rest with whole filler copies;
//! `Repeat` reuses the run's glyph indices and cuts its last copy at a
//! grapheme; `Spread` and `Fit` end their last glyph's step at the
//! path's end; text too long is cut at a grapheme and never overlaps.
//! These hold along an open path. A frame ring in `Once`, `Spread` or
//! `Fit` is text in two lines (#291, ruling R27), whose end conditions
//! `lines_tests` holds.

use glam::Vec2;

use crate::document::values::FitMode;
use crate::flow::tests::support::{close, clustered, real_run, style, synthetic, with_fonts};
use crate::flow::{place_on_open, shape_run, Curve, FlowSpan, PlacedRun, ShapedRun, FILLER_SPAN};
use crate::font::fonts::MIN_FONT_SIZE_PT;

/// A horizontal line from the origin, `length` long.
fn line(length: f32) -> Curve {
    Curve::Line(Vec2::ZERO, Vec2::new(length, 0.0))
}

/// The right edge of every placed glyph's box, on a horizontal line.
fn right_edges(run: &ShapedRun, placed: &PlacedRun) -> Vec<f32> {
    placed
        .glyphs
        .iter()
        .map(|g| g.pos.x + run.glyphs[g.glyph as usize].advance)
        .collect()
}

/// Consecutive boxes on a horizontal line never overlap: each starts at
/// or after the previous one's right edge.
fn assert_disjoint(run: &ShapedRun, placed: &PlacedRun) {
    let edges = right_edges(run, placed);
    for (i, pair) in placed.glyphs.windows(2).enumerate() {
        assert!(
            pair[1].pos.x >= edges[i] - 1e-3,
            "glyphs {i} and {} overlap",
            i + 1
        );
    }
}

/// `text` shaped at 12 pt and placed along `curve` with `fit`.
fn shaped_on_line(text: &str, curve: Curve, fit: &FitMode) -> (ShapedRun, Option<ShapedRun>, PlacedRun) {
    with_fonts(|fs| {
        let run = real_run(text, &style(12.0), fs);
        let (rescaled, placed) = place_on_open(&run, &curve, fit, 0.0, &style(12.0), fs);
        (run, rescaled, placed)
    })
}

/// `run` placed along `curve` with `fit` and `spacing`.
fn on_line(run: &ShapedRun, curve: Curve, fit: &FitMode, spacing: f32) -> PlacedRun {
    let (rescaled, placed) = with_fonts(|fs| place_on_open(run, &curve, fit, spacing, &style(12.0), fs));
    assert!(rescaled.is_none(), "a synthetic run is placed as given");
    placed
}

#[test]
fn test_flow_fit_fills_a_line_exactly() {
    do_flow_fit_fills_a_line_exactly();
}

/// Unclamped, `Fit` rescales the text by `P / L` and its last glyph ends
/// at the line's end. On a straight line `L` is the sum of advances, so
/// the scale is known in closed form here.
pub fn do_flow_fit_fills_a_line_exactly() {
    let probe = with_fonts(|fs| real_run("flowing text", &style(12.0), fs));
    let natural: f32 = probe.glyphs.iter().map(|g| g.advance).sum();
    let length = natural * 1.37;
    let (run, rescaled, placed) =
        shaped_on_line("flowing text", line(length), &FitMode::Fit { min: 0.5, max: 2.0 });
    let rescaled = rescaled.expect("an unclamped fit rescales");
    close(rescaled.line_height / run.line_height, 1.37, 1e-3, "scale");
    assert!(!placed.truncated);
    assert_eq!(placed.glyphs.len(), run.glyphs.len());
    close(
        *right_edges(&rescaled, &placed).last().unwrap(),
        length,
        0.5,
        "end",
    );
    assert_disjoint(&rescaled, &placed);
}

#[test]
fn test_flow_fit_fills_a_steep_line_with_an_ornament() {
    do_flow_fit_fills_a_steep_line_with_an_ornament();
}

/// An ornament steps by its ink (ruling R31), and `Fit` predicts its
/// length so: down a vertical line each glyph steps its ink's height
/// plus its side bearings, each space its advance, so `L` is their sum
/// and `Fit` rescales by `P / L` — not by the line-high boxes' sum, which
/// would leave it smaller with the rest spread as gaps — the last ink
/// ending its right bearing short of the line's end.
pub fn do_flow_fit_fills_a_steep_line_with_an_ornament() {
    let text = "✦ · ✦ · ✦";
    let probe = with_fonts(|fs| real_run(text, &style(12.0), fs));
    let step = |glyph: &crate::flow::GlyphMetrics| match glyph.ink {
        Some(ink) => (ink.max.y - ink.min.y) + glyph.advance - (ink.max.x - ink.min.x),
        None => glyph.advance,
    };
    let natural: f32 = probe.glyphs.iter().map(step).sum();
    let length = natural * 1.37;
    let down = Curve::Line(Vec2::ZERO, Vec2::new(0.0, length));
    let (run, rescaled, placed) = shaped_on_line(text, down, &FitMode::Fit { min: 0.5, max: 2.0 });
    let rescaled = rescaled.expect("an unclamped fit rescales");
    close(rescaled.line_height / run.line_height, 1.37, 2e-3, "scale");
    assert!(!placed.truncated);
    assert_eq!(placed.glyphs.len(), run.glyphs.len());
    let last = placed.glyphs[placed.glyphs.len() - 1];
    let glyph = &rescaled.glyphs[last.glyph as usize];
    let ink = glyph.ink.expect("the last star has ink");
    let bottom = last.pos.y + rescaled.baseline() + ink.max.y;
    close(bottom + glyph.advance - ink.max.x, length, 0.5, "end");
}

#[test]
fn test_flow_fit_clamped_at_max_spreads_the_rest() {
    do_flow_fit_clamped_at_max_spreads_the_rest();
}

/// Text too short even at `max` is drawn at `max` with positive, equal
/// spacing that still ends it at the line's end.
pub fn do_flow_fit_clamped_at_max_spreads_the_rest() {
    let (run, rescaled, placed) = shaped_on_line("abc", line(400.0), &FitMode::Fit { min: 0.5, max: 2.0 });
    let rescaled = rescaled.expect("clamped at 2, the run is rescaled");
    close(
        rescaled.line_height / run.line_height,
        2.0,
        1e-4,
        "scale held at max",
    );
    let edges = right_edges(&rescaled, &placed);
    let gaps: Vec<f32> = placed
        .glyphs
        .windows(2)
        .enumerate()
        .map(|(i, p)| p[1].pos.x - edges[i])
        .collect();
    assert!(gaps.iter().all(|&g| g > 1.0), "{gaps:?}");
    close(gaps[0], gaps[1], 1e-2, "equal spacing");
    close(*edges.last().unwrap(), 400.0, 0.5, "end");
}

#[test]
fn test_flow_fit_clamped_at_min_cuts_without_overlap() {
    do_flow_fit_clamped_at_min_cuts_without_overlap();
}

/// Text too long even at `min` is drawn at `min`, cut at the last
/// grapheme that fits — `truncated` — and its glyphs never overlap.
pub fn do_flow_fit_clamped_at_min_cuts_without_overlap() {
    let (run, rescaled, placed) = shaped_on_line(
        "far too long for so short a line",
        line(60.0),
        &FitMode::Fit { min: 0.5, max: 2.0 },
    );
    let rescaled = rescaled.expect("clamped at 0.5, the run is rescaled");
    close(
        rescaled.line_height / run.line_height,
        0.5,
        1e-4,
        "scale held at min",
    );
    assert!(placed.truncated);
    assert!(!placed.glyphs.is_empty() && placed.glyphs.len() < run.glyphs.len());
    assert_disjoint(&rescaled, &placed);
    assert!(*right_edges(&rescaled, &placed).last().unwrap() <= 60.0 + 1e-3);
}

#[test]
fn test_flow_repeat_reuses_glyph_indices() {
    do_flow_repeat_reuses_glyph_indices();
}

/// `Repeat` places copies of the same glyphs: advances 5, 7, 9 with
/// spacing 1 take 24 per copy, so a 100-long line holds four copies and
/// the fifth's first glyph (ending at 101) does not fit.
pub fn do_flow_repeat_reuses_glyph_indices() {
    let run = synthetic(&[5.0, 7.0, 9.0], 12.0);
    let placed = on_line(&run, line(100.0), &FitMode::Repeat, 1.0);
    let indices: Vec<u32> = placed.glyphs.iter().map(|g| g.glyph).collect();
    assert_eq!(indices, [0, 1, 2].repeat(4));
    assert!(!placed.truncated, "a full path is the mode working");
}

#[test]
fn test_flow_repeat_cuts_at_a_grapheme() {
    do_flow_repeat_cuts_at_a_grapheme();
}

/// The last copy is cut at the last whole grapheme: with a cluster of
/// two 5-wide glyphs then a 5-wide one, a line of 22 holds one copy and
/// not the second cluster's first glyph alone (it ends at 20, its
/// partner at 25); a line of 26 holds the whole cluster.
pub fn do_flow_repeat_cuts_at_a_grapheme() {
    let run = clustered(&[(5.0, true), (5.0, false), (5.0, true)], 10.0);
    let indices = |length: f32| {
        on_line(&run, line(length), &FitMode::Repeat, 0.0)
            .glyphs
            .iter()
            .map(|g| g.glyph)
            .collect::<Vec<_>>()
    };
    assert_eq!(indices(22.0), vec![0, 1, 2]);
    assert_eq!(indices(26.0), vec![0, 1, 2, 0, 1]);
}

#[test]
fn test_flow_once_with_a_filler_fills_the_rest() {
    do_flow_once_with_a_filler_fills_the_rest();
}

/// `Once` with a filler places the text, then whole copies of the
/// filler — shaped in the flow's style, appended to the run it returns —
/// until the next copy would pass the end.
pub fn do_flow_once_with_a_filler_fills_the_rest() {
    let fit = FitMode::Once {
        filler: Some("-".to_owned()),
    };
    let (_, filled, placed) = shaped_on_line("ab", line(200.0), &fit);
    let filled = filled.expect("a placed filler is in the run returned");
    assert_eq!(filled.glyphs.len(), 3);
    assert_eq!(filled.glyphs[2].span, FILLER_SPAN);
    let indices: Vec<u32> = placed.glyphs.iter().map(|g| g.glyph).collect();
    assert_eq!(&indices[..2], &[0, 1]);
    assert!(
        indices.len() > 10 && indices[2..].iter().all(|&i| i == 2),
        "{indices:?}"
    );
    let end = *right_edges(&filled, &placed).last().unwrap();
    let dash = filled.glyphs[2].advance;
    assert!(
        end <= 200.0 + 1e-3 && 200.0 - end < dash,
        "end {end}, a copy is {dash}"
    );
    assert!(!placed.truncated);
}

#[test]
fn test_flow_once_places_the_text_once() {
    do_flow_once_places_the_text_once();
}

/// `Once` without a filler places the text once and nothing after it;
/// on a path too short it is cut at a grapheme and `truncated`.
pub fn do_flow_once_places_the_text_once() {
    let run = synthetic(&[10.0; 3], 10.0);
    let once = FitMode::Once { filler: None };
    let placed = on_line(&run, line(100.0), &once, 0.0);
    assert_eq!(placed.glyphs.len(), 3);
    assert!(!placed.truncated);
    let placed = on_line(&run, line(25.0), &once, 0.0);
    assert_eq!(placed.glyphs.len(), 2);
    assert!(placed.truncated);
}

#[test]
fn test_flow_spread_ends_on_a_line() {
    do_flow_spread_ends_on_a_line();
}

/// `Spread` on a line: advances 10, 20, 30 over 200 leave 140 to share
/// between two gaps, so the boxes start at 0, 80 and 170 and the last
/// ends at 200. The open path's own spacing is replaced, not added.
pub fn do_flow_spread_ends_on_a_line() {
    let run = synthetic(&[10.0, 20.0, 30.0], 10.0);
    let placed = on_line(&run, line(200.0), &FitMode::Spread, 5.0);
    let x: Vec<f32> = placed.glyphs.iter().map(|g| g.pos.x).collect();
    assert_eq!(x.len(), 3);
    for (got, want) in x.iter().zip([0.0, 80.0, 170.0]) {
        close(*got, want, 0.05, "box start");
    }
    let single = synthetic(&[10.0], 10.0);
    let placed = on_line(&single, line(200.0), &FitMode::Spread, 0.0);
    assert_eq!(placed.glyphs[0].pos.x, 0.0, "one glyph takes no spacing");
}

#[test]
fn test_flow_spread_too_long_is_cut() {
    do_flow_spread_too_long_is_cut();
}

/// Text longer than the path takes no spacing and is cut at the last
/// grapheme that fits.
pub fn do_flow_spread_too_long_is_cut() {
    let run = synthetic(&[30.0; 3], 10.0);
    let placed = on_line(&run, line(70.0), &FitMode::Spread, 0.0);
    let x: Vec<f32> = placed.glyphs.iter().map(|g| g.pos.x).collect();
    assert_eq!(x, vec![0.0, 30.0]);
    assert!(placed.truncated);
}

#[test]
fn test_flow_fit_models_the_size_floor() {
    do_flow_fit_models_the_size_floor();
}

/// `Fit` predicts the reshape's size floor in a run of mixed sizes
/// (ruling R9.3): four letters at 24 pt and forty at 1 pt, fitted to
/// four and three tenths of their natural length on a line. Shrinking
/// that far takes the 1 pt span below the shaper's 0.5 pt floor, where
/// it stops shrinking while the 24 pt span still does — so the length is
/// not the scale times the natural length there. Predicted as if it
/// were, the reshaped run overruns and is cut at a scale well above
/// `min`; predicted with the floor, all 44 glyphs are placed and the
/// last one ends at the line's end.
pub fn do_flow_fit_models_the_size_floor() {
    let large = style(24.0);
    let small = style(1.0);
    let bs = "b".repeat(40);
    let shape = |fs: &mut crate::font::fonts::FontSystemWriteGuard| {
        shape_run(
            &[
                FlowSpan {
                    text: "AAAA",
                    style: &large,
                    link: None,
                },
                FlowSpan {
                    text: &bs,
                    style: &small,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    };
    let run = with_fonts(|fs| shape(fs));
    assert_eq!(run.glyphs.len(), 44);
    let natural: f32 = run.glyphs.iter().map(|g| g.advance).sum();
    let fit = FitMode::Fit { min: 0.05, max: 2.0 };
    for share in [0.4_f32, 0.3] {
        let length = natural * share;
        let (rescaled, placed) =
            with_fonts(|fs| place_on_open(&run, &line(length), &fit, 0.0, &style(12.0), fs));
        let rescaled = rescaled.expect("the run is rescaled");
        assert!(
            rescaled.glyphs[4].size <= MIN_FONT_SIZE_PT + 1e-4,
            "{share}: the fixture needs the 1 pt span at the floor, not {}",
            rescaled.glyphs[4].size
        );
        assert!(!placed.truncated, "{share}: cut at a scale above min");
        assert_eq!(placed.glyphs.len(), 44, "{share}");
        let end = right_edges(&rescaled, &placed).last().copied().unwrap();
        close(end, length, 0.5, &format!("{share}: ends at the line's end"));
    }
}

#[test]
fn test_flow_cut_keeps_the_reading_start() {
    do_flow_cut_keeps_the_reading_start();
}

/// A cut keeps the start of the text as it is read (ruling R9.4): five
/// Hebrew letters, listed and walked right to left in visual order, on a
/// line that holds three keep alef, bet and gimel — the first three
/// read — and lay them in visual order, gimel first. A visual-order cut
/// would keep the last three read instead.
pub fn do_flow_cut_keeps_the_reading_start() {
    let text = "\u{5D0}\u{5D1}\u{5D2}\u{5D3}\u{5D4}";
    let run = with_fonts(|fs| real_run(text, &style(12.0), fs));
    assert_eq!(run.glyphs.len(), 5);
    assert!(
        run.glyphs[0].bytes.start > run.glyphs[4].bytes.start,
        "the fixture needs right-to-left visual order"
    );
    let first_three: f32 = run
        .glyphs
        .iter()
        .filter(|g| g.bytes.start < 6)
        .map(|g| g.advance)
        .sum();
    let shortest = run.glyphs.iter().map(|g| g.advance).fold(f32::MAX, f32::min);
    let placed = on_line(
        &run,
        line(first_three + shortest * 0.5),
        &FitMode::Once { filler: None },
        0.0,
    );
    assert!(placed.truncated);
    let kept: Vec<usize> = placed
        .glyphs
        .iter()
        .map(|g| run.glyphs[g.glyph as usize].bytes.start)
        .collect();
    assert_eq!(kept, vec![4, 2, 0]);
}

/// Hitting the glyph ceiling stops a placement and says so, even when it
/// stops inside a copy: three-glyph clusters never land exactly on
/// `MAX_PLACED_GLYPHS` (2²⁰ is not a multiple of 3), so the copy that
/// would pass it is left out whole, and the run reports `truncated`
/// although fewer glyphs than the ceiling were placed. Plain `#[test]`:
/// the enumeration is the runtime (lib/baumhard/CONVENTIONS.md §B8,
/// class 3).
#[test]
fn test_flow_repeat_reports_the_glyph_ceiling() {
    let run = clustered(&[(0.5, true), (0.25, false), (0.25, false)], 2.0);
    let placed = on_line(&run, line(400_000.0), &FitMode::Repeat, 0.0);
    assert!(placed.glyphs.len() < crate::flow::MAX_PLACED_GLYPHS);
    assert_eq!(placed.glyphs.len(), crate::flow::MAX_PLACED_GLYPHS / 3 * 3);
    assert!(placed.truncated, "the ceiling stopped the placement");
}
