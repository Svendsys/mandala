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
/// `NaN` position. Shaping now reshapes such a flow with each tab as a
/// space, so every value it returns is finite; a face that can measure
/// a stop keeps it.
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
