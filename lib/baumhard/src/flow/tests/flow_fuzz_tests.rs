// SPDX-License-Identifier: MPL-2.0

//! TextFlow's fuzz properties held to account (TEST_CONVENTIONS §T14):
//! the structure-aware generator swept under a seeded source, the raw
//! property on hostile strings, and each property shown to go red on a
//! planted violation.

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::Align;
use crate::document::values::{FontSize, Rgba, Weight};
use crate::flow::tests::flow_fuzz::{
    check_box, check_flow_bytes, check_flow_generated, check_flow_structured, check_placement, check_run,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{shape_box, shape_run, FlowSpan, PlacedGlyph, PlacedRun, FILLER_SPAN};
use crate::font::fonts::AppFont;
use crate::util::tests::choices::Choices;

#[test]
fn test_generated_flows_hold_every_flow_property() {
    for seed in 0..100 {
        if let Err(why) = check_flow_generated(&mut Choices::seeded(seed)) {
            panic!("seed {seed}: {why}");
        }
    }
}

#[test]
fn test_hostile_strings_hold_every_flow_property() {
    for text in [
        "",
        "plain",
        "e\u{301}\u{302}\u{303} \u{1F469}\u{200D}\u{1F4BB} \u{1F1EF}\u{1F1F5}",
        "\u{5D0}\u{5D1}\u{202E}abc\u{2066}def\u{2069}",
        "a\nb\r\nc\u{85}d\u{2028}e\u{2029}f\u{1C}g",
        "\u{0}\u{7F}\u{9B}[2J",
        "\u{301}",
        "\u{FE0F}\u{20E3}",
    ] {
        check_flow_bytes(text.as_bytes()).unwrap_or_else(|why| panic!("{text:?}: {why}"));
    }
    check_flow_bytes(&[0xFF, 0xC0, b'a', 0x80]).unwrap();
}

#[test]
fn test_the_flow_properties_find_a_glyph_that_names_no_source() {
    let text = "e\u{301}x";
    let plain = style(16.0);
    let spans = [FlowSpan {
        text,
        style: &plain,
        link: None,
    }];
    let (run, shaped) = with_fonts(|fs| {
        (
            real_run(text, &plain, fs),
            shape_box(&spans, None, Align::Left, fs),
        )
    });
    assert_eq!(
        check_run(&spans, &run, "run"),
        Ok(()),
        "precondition: the control passes"
    );
    assert_eq!(
        check_box(&spans, &shaped, None),
        Ok(()),
        "precondition: the control passes"
    );
    assert!(run.glyphs.len() >= 2, "precondition: the run has glyphs to break");

    let mut beyond = run.clone();
    beyond.glyphs[0].span = 1;
    assert!(check_run(&spans, &beyond, "run")
        .unwrap_err()
        .contains("names span 1 of 1"));
    let mut split = run.clone();
    // Inside the two bytes of U+0301.
    split.glyphs[0].bytes = 0..2;
    assert!(check_run(&spans, &split, "run")
        .unwrap_err()
        .contains("not whole characters"));
    let mut filler = run.clone();
    filler.glyphs[0].span = FILLER_SPAN;
    filler.glyphs[0].bytes = 0..99;
    assert_eq!(
        check_run(&spans, &filler, "run"),
        Ok(()),
        "a filler names no span"
    );
    let mut nan = run.clone();
    nan.glyphs[1].advance = f32::NAN;
    assert!(check_run(&spans, &nan, "run")
        .unwrap_err()
        .contains("advance is NaN"));
    let mut fewer = run.clone();
    fewer.glyphs.pop();
    assert!(check_run(&spans, &fewer, "run")
        .unwrap_err()
        .contains("laid-out glyphs"));

    let mut small = shaped.clone();
    small.size = Vec2::new(1.0, -1.0);
    assert!(check_box(&spans, &small, None)
        .unwrap_err()
        .contains("a box of size"));
    assert!(check_box(&spans, &shaped, Some(Vec2::new(1e6, 0.0)))
        .unwrap_err()
        .contains("below the minimum"));
    assert_eq!(
        check_box(&spans, &shaped, Some(Vec2::new(f32::NAN, -5.0))),
        Ok(()),
        "an unusable minimum asks for nothing"
    );
}

#[test]
fn test_the_placement_property_finds_a_position_that_is_not_finite() {
    use crate::flow::tests::flow_fuzz::check_placement;
    let plain = style(16.0);
    let run = with_fonts(|fs| real_run("ab", &plain, fs));
    let placed = |pos: Vec2, glyph: u32| PlacedRun {
        glyphs: vec![PlacedGlyph { glyph, pos }],
        truncated: false,
    };
    assert_eq!(
        check_placement(&run, &placed(Vec2::ONE, 1), "p"),
        Ok(()),
        "precondition: the control passes"
    );
    assert!(check_placement(&run, &placed(Vec2::new(f32::NAN, 0.0), 0), "p")
        .unwrap_err()
        .contains("placed at"));
    assert!(check_placement(&run, &placed(Vec2::ONE, 2), "p")
        .unwrap_err()
        .contains("placed glyph 2 of 2"));
}

/// The style of the flow finding: a face whose space has no width.
fn zero_width_space_style() -> ResolvedText {
    ResolvedText {
        font: AppFont::SilhousTwo,
        size: FontSize::Pt(16.0),
        color: Rgba::new(255, 255, 255, 255),
        weight: Weight::new(200).unwrap(),
        italic: true,
    }
}

/// The flow target's finding (ruling R24): a tab in a face whose space
/// has no width came back from cosmic-text with a `NaN` advance — its
/// tab stop is `0 / 0` — and every glyph after it on the line with a
/// `NaN` position. Shaping now draws the tabs of a span in such a face
/// as spaces (R24.1), so every value it returns is finite; a face that
/// can measure a stop keeps it.
#[test]
fn test_a_tab_in_a_face_whose_space_has_no_width_shapes_finite() {
    let face = zero_width_space_style();
    for text in ["\t", "\tab\tc"] {
        let spans = [FlowSpan {
            text,
            style: &face,
            link: None,
        }];
        let (run, shaped) = with_fonts(|fs| {
            (
                shape_run(&spans, 1.0, fs),
                shape_box(&spans, None, Align::Left, fs),
            )
        });
        assert_eq!(
            run.glyphs.len(),
            text.chars().count(),
            "{text:?}: one glyph per character"
        );
        assert_eq!(check_run(&spans, &run, "the run"), Ok(()), "{text:?}");
        let laid_out: Vec<(f32, f32)> = run.layout_glyphs().map(|glyph| (glyph.x, glyph.w)).collect();
        assert!(
            laid_out.iter().all(|(x, w)| x.is_finite() && w.is_finite()),
            "{text:?}: every laid-out glyph, those after the tab included: {laid_out:?}"
        );
        assert_eq!(check_box(&spans, &shaped, None), Ok(()), "{text:?}: the box");
        let box_glyphs: Vec<(f32, f32)> = shaped
            .buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|glyph| (glyph.x, glyph.w)))
            .collect();
        assert!(
            box_glyphs.iter().all(|(x, w)| x.is_finite() && w.is_finite()),
            "{text:?}: every glyph of the box: {box_glyphs:?}"
        );
    }
    // Control: a face that can measure a stop keeps it — the tab is the
    // width of eight of its spaces, not one.
    let plain = style(16.0);
    let (tab, space) = with_fonts(|fs| (real_run("\t", &plain, fs), real_run(" ", &plain, fs)));
    assert!(
        space.glyphs[0].advance > 0.0,
        "precondition: the face's space has a width"
    );
    assert!(
        (tab.glyphs[0].advance - 8.0 * space.glyphs[0].advance).abs() < 1e-3,
        "the tab stop is kept: {} against {}",
        tab.glyphs[0].advance,
        space.glyphs[0].advance
    );
}

/// R24.1: tabs are decided per span, from that span's face. A tab in a
/// face that can measure its stop keeps it beside a tab in a face that
/// cannot — on the same run, and on another line of the same box —
/// exactly as wide as it is with no such tab in the flow. The review of
/// #293 found R24's first fix replacing every tab of the flow.
#[test]
fn test_a_measurable_tab_keeps_its_stop_beside_one_that_cannot_be_measured() {
    let plain = style(16.0);
    let face = zero_width_space_style();
    // The tab's advance in the plain face, the first glyph after `a`.
    let tab_in = |run: &crate::flow::ShapedRun| run.glyphs[1].advance;
    // The width of the box's first laid-out glyph on `line`.
    let first_on = |shaped: &crate::flow::ShapedBox, line: usize| {
        shaped
            .buffer
            .layout_runs()
            .find(|run| run.line_i == line)
            .and_then(|run| run.glyphs.first().map(|glyph| glyph.w))
    };
    let (alone, mixed, box_alone, box_mixed) = with_fonts(|fs| {
        let alone = shape_run(
            &[FlowSpan {
                text: "a\tb",
                style: &plain,
                link: None,
            }],
            1.0,
            fs,
        );
        let mixed = shape_run(
            &[
                FlowSpan {
                    text: "a\tb",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "\t",
                    style: &face,
                    link: None,
                },
            ],
            1.0,
            fs,
        );
        let box_alone = shape_box(
            &[FlowSpan {
                text: "\tx",
                style: &plain,
                link: None,
            }],
            None,
            Align::Left,
            fs,
        );
        let box_mixed = shape_box(
            &[
                FlowSpan {
                    text: "\tx\n",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "\ty",
                    style: &face,
                    link: None,
                },
            ],
            None,
            Align::Left,
            fs,
        );
        (alone, mixed, box_alone, box_mixed)
    });
    let space = with_fonts(|fs| real_run(" ", &plain, fs)).glyphs[0].advance;
    assert!(
        tab_in(&alone) > 1.5 * space,
        "precondition: alone, the tab reaches a stop wider than a space ({} against {space})",
        tab_in(&alone)
    );
    assert_eq!(
        tab_in(&mixed),
        tab_in(&alone),
        "on one run, the measurable tab keeps its stop"
    );
    assert!(mixed.glyphs[2..].iter().all(|glyph| glyph.advance.is_finite()));
    let line_0 = first_on(&box_alone, 0).expect("precondition: the box lays out its tab");
    assert!(
        line_0 > 1.5 * space,
        "precondition: alone, the box's tab reaches a stop"
    );
    assert_eq!(
        first_on(&box_mixed, 0),
        Some(line_0),
        "on line 0 of a box, the measurable tab keeps its stop"
    );
    let line_1 = first_on(&box_mixed, 1).expect("the box lays out line 1");
    assert!(
        line_1.is_finite(),
        "the tab that cannot be measured is drawn finite: {line_1}"
    );
}

/// R24.2: a tab the up-front decision lets through — a tab shaped in a
/// fallback face, which the span's primary face cannot predict — whose
/// layout is not finite is caught after shaping and laid out again as a
/// space: on its own line in a box, so the line that came out finite
/// keeps its tab stop, and throughout a run. No `NaN` reaches placement.
///
/// The test drives the check through a hook, `TABS_NOT_FINITE_IN`, which
/// makes it count every tab laid out in one face (Novem, whose tab stops
/// are finite) as not finite, so the guard runs the same way on every
/// machine. (Its first version relied on cosmic-text laying SilhousTwo's
/// tab out as `0 / 0` once the up-front check was told to pass it; that
/// held on one machine and not on CI's runner, so the test went red there
/// with the guard never reached.) `TABS_RELAID` shows that the guard, not
/// the up-front check, did the work.
#[test]
fn test_a_tab_the_up_front_check_lets_through_is_laid_out_again_as_a_space() {
    use crate::flow::shape::{failing_lines, TABS_NOT_FINITE_IN, TABS_RELAID};
    let plain = style(16.0);
    let other = ResolvedText {
        font: AppFont::Novem,
        ..style(16.0)
    };
    let first_on = |shaped: &crate::flow::ShapedBox, line: usize| {
        shaped
            .buffer
            .layout_runs()
            .find(|run| run.line_i == line)
            .and_then(|run| run.glyphs.first().map(|glyph| glyph.w))
    };
    let mixed_box = |fs: &mut crate::font::fonts::FontSystemWriteGuard| {
        shape_box(
            &[
                FlowSpan {
                    text: "\tx\n",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "\ty",
                    style: &other,
                    link: None,
                },
            ],
            None,
            Align::Left,
            fs,
        )
    };
    let mixed_run = |fs: &mut crate::font::fonts::FontSystemWriteGuard| {
        shape_run(
            &[
                FlowSpan {
                    text: "a\tb",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "\tc",
                    style: &other,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    };
    let (unhooked_box, unhooked_run, hooked_box, hooked_run, relaid, still_failing) = with_fonts(|fs| {
        let unhooked_box = mixed_box(fs);
        let unhooked_run = mixed_run(fs);
        TABS_NOT_FINITE_IN.with(|face| face.set(Some(AppFont::Novem)));
        let before = TABS_RELAID.with(std::cell::Cell::get);
        let hooked_box = mixed_box(fs);
        let hooked_run = mixed_run(fs);
        // Read back through the hook while it is set: what the check sees.
        let still_failing = (
            failing_lines(&hooked_box.buffer),
            failing_lines(&hooked_run.buffer),
        );
        TABS_NOT_FINITE_IN.with(|face| face.set(None));
        let relaid = TABS_RELAID.with(std::cell::Cell::get) - before;
        (
            unhooked_box,
            unhooked_run,
            hooked_box,
            hooked_run,
            relaid,
            still_failing,
        )
    });
    let (plain_space, other_space) = with_fonts(|fs| {
        (
            real_run(" ", &plain, fs).glyphs[0].advance,
            real_run(" ", &other, fs).glyphs[0].advance,
        )
    });
    // Preconditions: both faces measure their stops, so the up-front check
    // passes both tabs, and without the hook each is a stop wider than a
    // space; with it, the guard laid both flows out again.
    let unhooked_1 = first_on(&unhooked_box, 1).expect("precondition: the box lays out line 1");
    assert!(
        unhooked_1 > 1.5 * other_space && other_space > 0.0,
        "precondition: Novem's tab reaches a stop ({unhooked_1} against a space of {other_space})"
    );
    assert!(
        unhooked_run.glyphs[1].advance > 1.5 * plain_space,
        "precondition: the run's first tab reaches a stop"
    );
    assert_eq!(relaid, 2, "precondition: the guard laid both flows out again");
    // The check finds nothing left to fail.
    assert_eq!(
        still_failing,
        (Vec::new(), Vec::new()),
        "after the guard, no line of the box or the run still fails the check"
    );
    // A box: the failing line's tab is a space; the other keeps its stop.
    assert_eq!(
        first_on(&hooked_box, 0),
        first_on(&unhooked_box, 0),
        "the line that came out finite keeps its tab stop"
    );
    assert_eq!(
        first_on(&hooked_box, 1),
        Some(other_space),
        "the failing line's tab is drawn as its face's space"
    );
    // A run: laid out again whole, every tab a space.
    assert_eq!(
        hooked_run.glyphs[1].advance, plain_space,
        "the run is laid out again whole: its first tab is a space too"
    );
    if let Err(why) = check_box(
        &[
            FlowSpan {
                text: "\tx\n",
                style: &plain,
                link: None,
            },
            FlowSpan {
                text: "\ty",
                style: &other,
                link: None,
            },
        ],
        &hooked_box,
        None,
    ) {
        panic!("the box holds every flow property: {why}");
    }
    if let Err(why) = check_run(
        &[
            FlowSpan {
                text: "a\tb",
                style: &plain,
                link: None,
            },
            FlowSpan {
                text: "\tc",
                style: &other,
                link: None,
            },
        ],
        &hooked_run,
        "the run",
    ) {
        panic!("the run holds every flow property: {why}");
    }
}

/// The flow_structured smoke's finding on #293 (CI run 41146219501): a
/// `Spread` — and a `Fit` whose window holds the scale at 1, which
/// spreads too — around an outline whose perimeter is near the largest
/// `f32` placed every cluster after the first at `NaN`. The gap that
/// fills such a path walks past every `f32` when the solver doubles its
/// guess; `solve_gap` took that `NaN` miss for a fitting low side, kept
/// doubling to an infinite gap, and the walk answered `NaN` from there.
/// A miss that is not a number now overshoots. The input as the fuzzer
/// found it is replayed beside the case it comes down to.
#[test]
fn test_a_spread_around_an_outline_near_the_largest_f32_places_every_glyph_finite() {
    use crate::document::values::{Direction, Edge, FitMode, StartPoint};
    use crate::flow::{place_on_closed, Outline, Rect};
    let found = include_bytes!("fuzz_findings/spread_gap_past_the_largest_f32");
    if let Err(why) = check_flow_structured(found) {
        panic!("the fuzzer's input: {why}");
    }
    let plain = style(16.0);
    // The outline the input drew: a rounded rectangle 1.78e38 on a side,
    // its perimeter 2.8e38, finite.
    let path = Outline::RoundedRect {
        rect: Rect::new(Vec2::splat(-8.908107e37), Vec2::splat(1.7816215e38)),
        radius: 8.908107e37,
    };
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.518,
    };
    for fit in [FitMode::Spread, FitMode::Fit { min: 1.0, max: 1.0 }] {
        for direction in [Direction::Clockwise, Direction::CounterClockwise] {
            let placed = with_fonts(|fs| {
                let run = real_run("abc", &plain, fs);
                let (replaced, placed) =
                    place_on_closed(&run, &path, start, direction, &fit, false, &plain, fs);
                (replaced.unwrap_or(run), placed)
            });
            let (run, placed) = placed;
            assert!(
                placed.glyphs.len() == 3,
                "precondition: {fit:?} {direction:?} places all three glyphs ({})",
                placed.glyphs.len()
            );
            if let Err(why) = check_placement(&run, &placed, "around the outline") {
                panic!("{fit:?} {direction:?}: {why}");
            }
        }
    }
}
