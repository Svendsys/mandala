// SPDX-License-Identifier: MPL-2.0

//! TextFlow's fuzz properties held to account (TEST_CONVENTIONS §T14):
//! the structure-aware generator swept under a seeded source, the raw
//! property on hostile strings, and each property shown to go red on a
//! planted violation.

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::Align;
use crate::document::values::{FontSize, Rgba, Weight};
use crate::flow::tests::flow_fuzz::{check_box, check_flow_bytes, check_flow_generated, check_run};
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

/// R24.2: a tab the up-front decision lets through — the case of a tab
/// shaped in a fallback face, which the span's primary face cannot
/// predict — is caught after shaping and laid out again as a space: on
/// its own line in a box, so the line that came out finite keeps its tab
/// stop, and throughout a run. No `NaN` reaches placement.
///
/// No bundled face shapes a tab non-finite in fallback alone, so the
/// test drives the guard through a hook: `TRUST_EVERY_FACE` makes the
/// up-front check pass SilhousTwo, whose space has no width, and
/// cosmic-text lays its tab out as `0 / 0` for real. `TABS_RELAID`
/// shows the guard, not the up-front check, did the repair.
#[test]
fn test_a_tab_the_up_front_check_lets_through_is_laid_out_again_as_a_space() {
    use crate::flow::shape::{TABS_RELAID, TRUST_EVERY_FACE};
    let plain = style(16.0);
    let face = zero_width_space_style();
    let first_on = |shaped: &crate::flow::ShapedBox, line: usize| {
        shaped
            .buffer
            .layout_runs()
            .find(|run| run.line_i == line)
            .and_then(|run| run.glyphs.first().map(|glyph| glyph.w))
    };
    let all_finite = |shaped: &crate::flow::ShapedBox| {
        shaped.buffer.layout_runs().all(|run| {
            run.line_w.is_finite()
                && run
                    .glyphs
                    .iter()
                    .all(|glyph| glyph.w.is_finite() && glyph.x.is_finite())
        })
    };
    let (box_alone, box_mixed, run, relaid) = with_fonts(|fs| {
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
        TRUST_EVERY_FACE.with(|trust| trust.set(true));
        let before = TABS_RELAID.with(std::cell::Cell::get);
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
        let run = shape_run(
            &[
                FlowSpan {
                    text: "a\tb",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "\tc",
                    style: &face,
                    link: None,
                },
            ],
            1.0,
            fs,
        );
        TRUST_EVERY_FACE.with(|trust| trust.set(false));
        let relaid = TABS_RELAID.with(std::cell::Cell::get) - before;
        (box_alone, box_mixed, run, relaid)
    });
    assert_eq!(
        relaid, 2,
        "precondition: the up-front check let both tabs through and the guard laid both flows out again"
    );
    assert!(all_finite(&box_mixed), "the box is laid out finite");
    let line_0 = first_on(&box_alone, 0).expect("precondition: the box lays out its tab");
    assert_eq!(
        first_on(&box_mixed, 0),
        Some(line_0),
        "the line that came out finite keeps its tab stop"
    );
    let space = with_fonts(|fs| real_run(" ", &face, fs)).glyphs[0].advance;
    assert_eq!(
        first_on(&box_mixed, 1),
        Some(space),
        "the failing line's tab is drawn as the face's space"
    );
    assert!(
        run.glyphs
            .iter()
            .all(|glyph| glyph.advance.is_finite() && glyph.x.is_finite()),
        "the run is laid out finite: {:?}",
        run.glyphs
            .iter()
            .map(|glyph| (glyph.advance, glyph.x))
            .collect::<Vec<_>>()
    );
    if let Err(why) = check_run(
        &[
            FlowSpan {
                text: "a\tb",
                style: &plain,
                link: None,
            },
            FlowSpan {
                text: "\tc",
                style: &face,
                link: None,
            },
        ],
        &run,
        "the run",
    ) {
        panic!("the run holds every flow property: {why}");
    }
}
