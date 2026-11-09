// SPDX-License-Identifier: MPL-2.0

//! A size-only reshape shapes nothing, and an outline is read once per
//! glyph and size (V1-B14, #304). `Fit`'s rescale of
//! a path flow, and R27's sizing of a frame's two lines, lay the run's
//! own shaping out at the new size (`shape::resized_buffer`) instead of
//! shaping its text again, and lay out exactly what shaping it again laid
//! out.
//!
//! The reference is the same reshape with the test hook
//! `RESHAPE_BY_SHAPING` set, which shapes again as before. Work is read
//! off the shaper's own per-thread counters ([`cosmic_text::shape_work`]),
//! never a clock.

use cosmic_text::shape_work;
use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::{FitMode, Shape};
use crate::flow::place::place_lines;
use crate::flow::shape::{reshape, RESHAPE_BY_SHAPING};
use crate::flow::tests::support::{style, with_fonts};
use crate::flow::{outline, place_on_open, shape_run, Curve, FlowSpan, PlacedRun, Rect, ShapedRun};
use crate::font::fonts::{outline_reads, AppFont, FontSystemLease};

/// `f` with this thread's size-only reshapes shaping again when `by_shaping`.
fn reshaping<R>(by_shaping: bool, f: impl FnOnce() -> R) -> R {
    let was = RESHAPE_BY_SHAPING.with(|hook| hook.replace(by_shaping));
    let out = f();
    RESHAPE_BY_SHAPING.with(|hook| hook.set(was));
    out
}

/// Everything a run says about its glyphs: their metrics, its line
/// height and piece table, and every laid-out glyph and line width of its
/// buffer.
fn facts(run: &ShapedRun) -> (String, Vec<String>) {
    let summary = format!("{:?} {} {:?}", run.glyphs, run.line_height, run.pieces);
    let layout = run
        .buffer
        .layout_runs()
        .flat_map(|line| {
            std::iter::once(format!("line {} w {}", line.line_i, line.line_w))
                .chain(line.glyphs.iter().map(|glyph| format!("{glyph:?}")))
        })
        .collect();
    (summary, layout)
}

/// `style` in `font`.
fn in_face(points: f32, font: AppFont) -> ResolvedText {
    ResolvedText {
        font,
        ..style(points)
    }
}

#[test]
fn test_a_size_only_reshape_lays_out_the_runs_shaping_and_shapes_nothing() {
    // Runs of one span and of several, in several faces and sizes, right
    // to left, with marks, a line break, an ornament (whose ink is
    // measured), and a code point no face draws.
    let latin = style(12.0);
    let big = in_face(30.0, AppFont::GalatiaSIL);
    let hebrew = in_face(9.0, AppFont::NotoSansHebrewRegular);
    let devanagari = in_face(14.0, AppFont::NotoSansDevanagariRegular);
    let ornament = in_face(16.0, AppFont::MandalaOrnaments);
    let any = in_face(11.0, AppFont::Any);
    let runs: Vec<Vec<(&str, &ResolvedText)>> = vec![
        vec![("Sand, well-known \u{2726} star", &latin)],
        vec![
            ("Big ", &big),
            ("\u{05e9}\u{05dc}\u{05d5}\u{05dd} ", &hebrew),
            ("and small", &latin),
        ],
        vec![
            ("\u{0928}\u{092e}\u{0938}\u{094d}\u{0924}\u{0947}", &devanagari),
            (" x\u{0308}", &any),
        ],
        vec![("\u{2726} \u{00b7} \u{2766}", &ornament)],
        vec![("one\ntwo\r\nthree", &latin)],
        vec![("a\u{10fff4}b", &latin), ("\u{1f338}", &any)],
    ];
    // Shrinks and growths, and one so small every size is floored.
    let factors = [0.5, 0.3, 1.0e-3, 1.5, 4.0];
    for spans in &runs {
        let flow: Vec<FlowSpan<'_>> = spans
            .iter()
            .map(|&(text, style)| FlowSpan {
                text,
                style,
                link: None,
            })
            .collect();
        let run = with_fonts(|fs| shape_run(&flow, 1.0, fs));
        for factor in factors {
            let (resized, work) = with_fonts(|fs| {
                let before = shape_work();
                let resized = reshape(&run, factor, None, fs.fonts());
                (resized, shape_work().since(before))
            });
            let (shaped, reference) = reshaping(true, || {
                with_fonts(|fs| {
                    let before = shape_work();
                    let shaped = reshape(&run, factor, None, fs.fonts());
                    (shaped, shape_work().since(before))
                })
            });
            assert!(
                reference.shapes > 0 && shaped.1,
                "precondition: {spans:?} at {factor}: the reference shapes the text again, and says so"
            );
            // What a frame counts for a reshape is read from its report
            // (`halves::resizing_steps`): it must be the shaper's own word.
            assert!(
                work.shapes == 0 && !resized.1,
                "{spans:?} at {factor}: a size-only reshape shaped text ({} shapings), or said it did ({})",
                work.shapes,
                resized.1
            );
            let (resized, shaped) = (resized.0, shaped.0);
            let (resized, shaped) = (facts(&resized), facts(&shaped));
            assert_eq!(
                resized.0, shaped.0,
                "{spans:?} at {factor}: the glyphs differ from the run shaped again"
            );
            assert_eq!(
                resized.1, shaped.1,
                "{spans:?} at {factor}: the layout differs from the run shaped again"
            );
        }
    }
}

#[test]
fn test_a_reshape_with_a_filler_still_shapes() {
    // A filler is new text: it is shaped, and so is the line beside it.
    let flow = style(12.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[FlowSpan {
                text: "text",
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        )
    });
    let work = with_fonts(|fs| {
        let before = shape_work();
        let (_, shaped) = reshape(&run, 1.0, Some(("\u{00b7} ", &flow)), fs.fonts());
        (shape_work().since(before), shaped)
    });
    assert!(
        work.0.shapes > 0 && work.1,
        "a filler is shaped, and the reshape says so"
    );
}

/// A placement's shapings, its rescaled run's facts, and what it placed.
type Placed = (u64, Option<(String, Vec<String>)>, PlacedRun);

/// What placing `run` costs in shapings, and what it places, once laying
/// its shaping out at each new size and once shaping it again.
fn placed_both_ways(place: &dyn Fn(&mut FontSystemLease) -> (Option<ShapedRun>, PlacedRun)) -> [Placed; 2] {
    [false, true].map(|by_shaping| {
        reshaping(by_shaping, || {
            with_fonts(|fs| {
                let before = shape_work();
                let (rescaled, placed) = place(fs);
                (
                    shape_work().since(before).shapes,
                    rescaled.as_ref().map(facts),
                    placed,
                )
            })
        })
    })
}

#[test]
fn test_fit_rescales_a_path_flow_without_shaping_it_again() {
    // Too long for its line at 12 pt: `Fit` shrinks it, probing sizes.
    let flow = style(12.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[FlowSpan {
                text: "far too long for so short a line, and longer still",
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        )
    });
    let curve = Curve::Line(Vec2::ZERO, Vec2::new(120.0, 0.0));
    let fit = FitMode::Fit { min: 0.1, max: 2.0 };
    let [resized, shaped] = placed_both_ways(&|fs| place_on_open(&run, &curve, &fit, 0.0, &flow, fs));
    assert!(
        shaped.1.is_some() && shaped.0 > 0,
        "precondition: `Fit` rescales the run, and shaping it again costs shapings ({})",
        shaped.0
    );
    assert_eq!(resized.0, 0, "`Fit` shaped text to rescale it");
    assert_eq!(
        resized.1, shaped.1,
        "the rescaled run differs from the one shaped again"
    );
    assert_eq!(
        resized.2, shaped.2,
        "the placement differs from the one shaped again"
    );
}

#[test]
fn test_r27_sizes_a_frames_lines_without_shaping_them_again() {
    // Two lines on an ellipse, `Fit` setting both at one size (R27).
    let flow = style(12.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[FlowSpan {
                text: "To see a World in a Grain of Sand, And a Heaven in a Wild Flower",
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        )
    });
    let ellipse = outline(Rect::new(Vec2::ZERO, Vec2::new(420.0, 200.0)), Shape::Ellipse);
    let fit = FitMode::Fit { min: 0.1, max: 10.0 };
    let [resized, shaped] = placed_both_ways(&|fs| place_lines(&run, &ellipse, &fit, false, &flow, fs, None));
    assert!(
        shaped.1.is_some() && shaped.0 > 0,
        "precondition: R27 resizes the lines, and shaping them again costs shapings ({})",
        shaped.0
    );
    assert_eq!(resized.0, 0, "R27 shaped text to size the lines");
    assert_eq!(
        resized.1, shaped.1,
        "the sized run differs from the one shaped again"
    );
    assert_eq!(resized.2, shaped.2, "the lines differ from the ones shaped again");
}

#[test]
fn test_an_ornaments_outlines_are_read_once_per_glyph_and_size() {
    // An ornament run's glyphs are inked (ruling R31): each outline is read
    // off its face the first time it is measured at a size, and kept.
    let ornament = in_face(16.0, AppFont::MandalaOrnaments);
    let flow = [FlowSpan {
        text: "\u{2726} \u{00b7} \u{2766} \u{2726}",
        style: &ornament,
        link: None,
    }];
    let reads = |f: &dyn Fn(&mut FontSystemLease) -> ShapedRun| {
        with_fonts(|fs| {
            let before = outline_reads();
            let run = f(fs);
            (run, outline_reads() - before)
        })
    };
    let (first, cold) = reads(&|fs| shape_run(&flow, 1.0, fs));
    let (second, warm) = reads(&|fs| shape_run(&flow, 1.0, fs));
    assert!(
        first.glyphs.iter().any(|glyph| glyph.ink.is_some()),
        "precondition: the run is an ornament, and inked"
    );
    // Three distinct ornaments, the fourth a repeat of the first: three
    // outlines read at most (a space is not inked).
    assert!(
        cold > 0 && cold <= 3,
        "the first measure reads each outline once ({cold})"
    );
    assert_eq!(warm, 0, "the second measure reads no outline");
    assert_eq!(first.glyphs, second.glyphs, "and measures the same inks");

    let (_, resized) = reads(&|fs| reshape(&first, 0.5, None, fs.fonts()).0);
    let (_, again) = reads(&|fs| reshape(&first, 0.5, None, fs.fonts()).0);
    assert!(resized > 0, "a new size is a new outline");
    assert_eq!(again, 0, "and is kept too");
}
