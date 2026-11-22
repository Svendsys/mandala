// SPDX-License-Identifier: MPL-2.0

//! A frame that keeps R27's placement — round an outline whose halves
//! cannot be measured — shapes its text with its `Once` filler appended
//! before it first reads its budget (#387): counted, exactly, in the most
//! it may take (`halves::filler_steps`, `Trace::filled`), and in what the
//! flow harnesses hold it to.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::place::halves::{filler_steps, frame_work_steps, MOST_FILLED};
use crate::flow::place::ring::RingLoss;
use crate::flow::place::{place_on_closed_capped, setup_table_samples, Trace};
use crate::flow::tests::flow_fuzz::{
    check_flow_structured_every_face, check_flow_structured_every_face_as_found, filler_in_time, InputWork,
    FILLER_LEFT_OUT, MAX_RING_GLYPHS, STRUCTURED_FRAMES,
};
use crate::flow::tests::support::{style, with_fonts};
use crate::flow::{outline, shape_run, FlowSpan, Outline, Rect, RingLayout};
use crate::util::tests::compiled_faces::{on_compiled_faces, report_hosts, this_test, undrawn, CHILD};

/// The size the rows are shaped at.
const POINTS: f32 = 16.0;

/// A rounded rectangle `width` × `height` at the origin: one whose width is
/// not a number has halves that cannot be measured, and a frame round it
/// keeps R27's placement (`halves::keeps_r27`, the flow_structured finding
/// `structured_frame_round_an_outline_whose_width_is_not_a_number`); one
/// 1e19 wide and of no finite height is too long to lay
/// (`halves::lays_nothing`), and its halves cannot be measured either.
fn rounded(width: f32, height: f32) -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(width, height)),
        Shape::Rectangle {
            radius: Percent::new(50),
        },
    )
}

/// How a frame of [`filler_counted`] lays against the most it may take
/// as the flow harnesses read it before #387, with no filler's shaping.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Past {
    /// Code points no face draws, its filler shaped: laid past it.
    Yes,
    /// Its halves measured, its filler never shaped: nothing charged.
    Unshaped,
    /// Its outline too long to lay, nothing laid: nothing charged, where a
    /// frame of no text round it, which is not too long, shapes its filler.
    TooLong,
    /// Text and a filler one face draws, shaped: within it.
    Drawn,
}

/// **A frame that keeps R27's placement counts its filler** (#387): round
/// an outline whose halves cannot be measured, a `Once` frame shapes its
/// text with its filler appended before it first reads its budget
/// (`lines::filling`), which its budget therefore does not bound — a
/// `.notdef` pass for each distinct code point no face draws. On the
/// compiled faces alone:
///
/// - 40 such code points round a rounded rectangle whose width is not a
///   number, with a filler of 200 more: the frame charges its filler
///   exactly as predicted (`halves::filler_steps`, `Trace::filled`), lays
///   past the most it may take as the flow harnesses read it before #387
///   and within it with the filler's shaping, which the harnesses' check
///   of a frame holds it to and refuses it without;
/// - the same frame round a rounded rectangle 400 wide, whose halves can
///   be measured: laid by halves, its filler never shaped, it charges
///   nothing for it and is predicted nothing, and the harnesses' check
///   refuses a prediction of the first frame's charge;
/// - the same frame round a rounded rectangle 1e19 wide and of no finite
///   height, too long to lay and whose halves cannot be measured: nothing
///   is laid, it charges nothing and is predicted nothing, where a frame
///   of no text round it — not too long to lay, since `lays_nothing` reads
///   only a frame with text — keeps R27's placement and is predicted its
///   filler's shaping, and the harnesses' check refuses that prediction
///   for it;
/// - letters and a filler one face draws round the first outline: counted
///   exactly, and within the most it may take as read before;
/// - the harnesses leave off a filler whose shaping leaves a frame of no
///   text outside its share of the timeout ([`filler_in_time`]), and count
///   it, and keep one one face draws, and one round an outline whose
///   halves can be measured, which is never shaped;
/// - the flow_structured_every_face input that found it (#387, 1,338
///   bytes; found by the exclusion-cost sweep of #387, not by a fuzz run),
///   whose `Once` frame of 102 glyphs round an outline whose halves cannot
///   be measured took 432,981 glyph steps past its budget of 400,000 and
///   its overshoot, 405,290 in all: its filler's shaping, 431,933 steps, is
///   counted, and every property holds, its frame charging its filler past
///   any frame's least budget.
///
/// In a child process (`util::tests::compiled_faces`): a host face here
/// maps every code point `compiled_faces::undrawn` reads, and lays the
/// finding's.
///
/// Controls, each run and seen red: `overshoot_steps` with no filler's
/// shaping, the first frame and the finding past the most they may take;
/// `filler_steps` saying every `Once` frame with a filler keeps R27's
/// placement, the second frame predicted a charge it never makes;
/// `filler_steps` without its too-long gate (`lays_nothing`), the third
/// frame predicted a charge it never makes; `filler_steps` saying none
/// does, the first frame and the finding
/// charging what was not predicted; the lay not noting its charge
/// (`Trace::filled`), the first frame's charge unseen; `filler_in_time`
/// keeping every filler, the first one kept and uncounted.
#[test]
fn test_a_frame_that_keeps_r27s_placement_counts_its_filler() {
    let name = this_test!();
    if std::env::var(CHILD).as_deref() == Ok("filler") {
        report_hosts();
        filler_counted();
        println!("REPORT FILLER counted");
        return;
    }
    let Some(reports) = on_compiled_faces(&name, "filler") else {
        return;
    };
    for report in &reports {
        eprintln!("{report}");
    }
    assert_eq!(
        reports.len(),
        9,
        "the child's four frames, its three fillers, the finding and its check: {reports:#?}"
    );
    assert_eq!(
        reports.last().map(String::as_str),
        Some("FILLER counted"),
        "the child's check"
    );
}

/// [`test_a_frame_that_keeps_r27s_placement_counts_its_filler`]'s check,
/// in its child.
fn filler_counted() {
    let flow = style(POINTS);
    let unmeasured = rounded(f32::NAN, 120.0);
    let measured = rounded(400.0, 120.0);
    let too_long = rounded(1.0e19, f32::INFINITY);
    let (text, filler) = with_fonts(|fs| {
        let undrawn = undrawn(fs, 240);
        assert_eq!(undrawn.len(), 240, "precondition: 240 code points no face draws");
        (
            undrawn[..40].iter().collect::<String>(),
            undrawn[40..].iter().collect::<String>(),
        )
    });
    let rows = [
        (
            "code points no face draws, round an outline whose halves cannot be measured",
            text.as_str(),
            filler.as_str(),
            &unmeasured,
            Past::Yes,
        ),
        (
            "code points no face draws, round an outline whose halves can be measured",
            text.as_str(),
            filler.as_str(),
            &measured,
            Past::Unshaped,
        ),
        (
            "code points no face draws, round an outline too long to lay",
            text.as_str(),
            filler.as_str(),
            &too_long,
            Past::TooLong,
        ),
        (
            "letters one face draws, round an outline whose halves cannot be measured",
            "MANDALA mindmap",
            "·",
            &unmeasured,
            Past::Drawn,
        ),
    ];
    // What the first frame charged: a prediction the second must refuse.
    let mut charged = 0;
    for (what, text, filler, path, past) in rows {
        let fit = FitMode::Once {
            filler: Some(filler.to_string()),
        };
        let (run, predicted, empty, tables, trace) = with_fonts(|fs| {
            let shaped = |text: &str, fs: &mut crate::font::fonts::FontSystemLease| {
                shape_run(
                    &[FlowSpan {
                        text,
                        style: &flow,
                        link: None,
                    }],
                    1.0,
                    fs,
                )
            };
            let run = shaped(text, fs);
            let predicted = filler_steps(&run, path, &fit, &flow, fs);
            // What a frame of no text round it is predicted, as
            // `filler_in_time` reads it.
            let none = shaped("", fs);
            let empty = filler_steps(&none, path, &fit, &flow, fs);
            let tables = setup_table_samples(&run, path, &fit, &flow);
            let mut trace = Trace::default();
            place_on_closed_capped(
                &run,
                path,
                StartPoint {
                    edge: Edge::Top,
                    at: 0.0,
                },
                Direction::Clockwise,
                &fit,
                &RingLayout::whole(),
                true,
                &flow,
                fs,
                MAX_RING_GLYPHS,
                Some(&mut trace),
            );
            (run, predicted, empty, tables, trace)
        });
        let before = frame_work_steps(&run, None, false, tables, 0);
        let most = frame_work_steps(&run, None, false, tables, predicted);
        println!(
            "REPORT FRAME {what}: filled {} predicted {predicted}, spent {}, most {most}, as read before {before}",
            trace.filled, trace.spent
        );
        assert_eq!(trace.filled, predicted, "{what}: its filler charged as predicted");
        let held = InputWork::default().frame(&run, None, false, tables, predicted, &trace, what);
        assert!(held.is_ok(), "{what}: held to its prediction: {held:?}");
        assert!(trace.spent <= most, "{what}: within the most it may take");
        match past {
            Past::Yes => {
                assert!(predicted > 0, "{what}: its filler shaped");
                assert!(
                    trace.spent > before,
                    "{what}: past the most it may take as read before #387"
                );
                assert!(
                    InputWork::default()
                        .frame(&run, None, false, tables, 0, &trace, what)
                        .is_err(),
                    "{what}: refused with no filler predicted"
                );
                charged = predicted;
            }
            Past::Unshaped => {
                assert_eq!(predicted, 0, "{what}: its filler never shaped");
                assert!(charged > 0, "precondition: the first frame's charge");
                assert!(
                    InputWork::default()
                        .frame(&run, None, false, tables, charged, &trace, what)
                        .is_err(),
                    "{what}: refused with the first frame's charge predicted"
                );
            }
            Past::TooLong => {
                assert_eq!(predicted, 0, "{what}: nothing laid, its filler never shaped");
                assert_eq!(
                    trace.ring,
                    Some(RingLoss::TooLong),
                    "{what}: precondition: too long to lay, nothing laid"
                );
                assert!(
                    empty > 0,
                    "{what}: precondition: a frame of no text round it keeps R27's placement"
                );
                assert!(
                    InputWork::default()
                        .frame(&run, None, false, tables, empty, &trace, what)
                        .is_err(),
                    "{what}: refused with a frame of no text's charge predicted"
                );
            }
            Past::Drawn => {
                assert!(predicted > 0, "{what}: its filler shaped");
                assert!(
                    trace.spent <= before,
                    "{what}: within the most it may take as read before #387"
                );
            }
        }
    }
    // The harnesses' cut: a filler whose shaping leaves a frame of no text
    // outside the timeout is left off, and counted.
    let fillers = [
        (
            "code points no face draws, round an outline whose halves cannot be measured",
            filler.as_str(),
            &unmeasured,
            true,
        ),
        (
            "a filler one face draws, round an outline whose halves cannot be measured",
            "·",
            &unmeasured,
            false,
        ),
        (
            "code points no face draws, round an outline whose halves can be measured",
            filler.as_str(),
            &measured,
            false,
        ),
    ];
    for (what, filler, path, left) in fillers {
        let fit = FitMode::Once {
            filler: Some(filler.to_string()),
        };
        let was = FILLER_LEFT_OUT.with(std::cell::Cell::get);
        let kept = with_fonts(|fs| filler_in_time(&fit, path, &flow, 1.0, STRUCTURED_FRAMES, fs));
        let counted = FILLER_LEFT_OUT.with(std::cell::Cell::get) - was;
        println!(
            "REPORT FILLER {what}: its filler kept {}, counted {counted}",
            kept == fit
        );
        if left {
            assert_eq!(
                kept,
                FitMode::Once { filler: None },
                "{what}: its filler left off"
            );
            assert_eq!(counted, 1, "{what}: and counted");
        } else {
            assert_eq!(kept, fit, "{what}: its filler kept");
            assert_eq!(counted, 0, "{what}: and nothing counted");
        }
    }
    // The input that found it: held to the timeout, as the fuzzer lays
    // it, and as found, its frame's text whole.
    let found = include_bytes!(
        "fuzz_findings/structured_once_filler_round_an_outline_whose_halves_cannot_be_measured"
    );
    let held = check_flow_structured_every_face(found);
    MOST_FILLED.with(|most| most.set(0));
    let as_found = check_flow_structured_every_face_as_found(found);
    let most = MOST_FILLED.with(std::cell::Cell::get);
    println!("REPORT FINDING held {held:?}, as found {as_found:?}, most filled {most}");
    assert!(
        held.is_ok(),
        "the input that found it, held to the timeout: {held:?}"
    );
    assert!(
        as_found.is_ok(),
        "the input that found it, as found: {as_found:?}"
    );
    assert!(
        most > crate::flow::place::halves::frame_budget(0),
        "the input that found it charges its filler past any frame's least budget, as found: {most}"
    );
}
