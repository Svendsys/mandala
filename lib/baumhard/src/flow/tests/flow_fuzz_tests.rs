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
    check_box, check_flow_bytes, check_flow_bytes_in_every_fit, check_flow_generated,
    check_flow_generated_in, check_flow_structured_as_found, check_placement, check_run, every_face,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{shape_box, shape_run, FlowSpan, PlacedGlyph, PlacedRun, RingLayout, FILLER_SPAN};
use crate::font::fonts::AppFont;
use crate::util::tests::choices::Choices;

/// The seeds a sweep of the structured generator's seeds 0 to 5,000 found
/// failing (review round R50), with the compiled faces alone and with this
/// machine's host faces:
///
/// - **1021**, on every host: a `Spread` of seven glyphs along a quadratic
///   from (1, 1) by (−2.4 × 10³⁸, −587) to (−10³⁰, 1), whose natural length
///   fits its 2.4 × 10³⁸, was cut to nothing. The curve's speed bound,
///   twice its first leg, was past the largest `f32`, so its path built no
///   table and had no length; the bound saturates now
///   (`Curve::speed_bound`).
/// - **2970** and **4284**, with host faces that shape their text: a
///   frame's line on a part some 10³³ along its half, where an `f32` sets
///   every letter at one center, so no gap moved its end; the tracking
///   solve widened its gap past every `f32` and placed the line at `NaN`.
///   It stops once one gap is as long as all the room the line had and
///   leaves it no less short than the probe before (R51.3), and reads a
///   lay past every `f32` as past its part's end
///   (`test_flow_halves_a_line_no_gap_can_move_keeps_a_finite_lay` holds
///   both on every host).
///
/// Seed 2771, with host faces, laid a `Repeat` ring whose seam no count of
/// its copies closed for past 18 minutes; its count is held by
/// `test_flow_a_ring_that_never_closes_counts_its_copies_in_few_asks`, and
/// the input it drew then, which R51.2's mixed sizes draw otherwise now,
/// is replayed at the harnesses' ring cap, short and within its budget
/// (#310, ruling B17.2c), by
/// `test_flow_structured_finding_a_ring_whose_inks_its_floats_cannot_part_replays_short_at_its_cap`.
///
/// Controls: with the speed bound not saturated, red at 1021 ("its natural
/// length fits, and it was cut"); without both the tracking solve's stop
/// and its reading of a lay past every `f32`, red at 2970 where host faces
/// shape its text, as here (either alone holds it finite).
#[test]
fn test_generated_flows_the_r50_seed_sweep_found_hold_every_flow_property() {
    for seed in [1021, 2970, 4284] {
        if let Err(why) = check_flow_generated(&mut Choices::seeded(seed)) {
            panic!("seed {seed}: {why}");
        }
    }
}

/// The frame's closed-form layer sets each letter at the first root of
/// what is owed (#309, ruling B16.1b; `check_pair_model`, which the
/// flow_structured target holds every input to): over 20,000 seeded
/// letters, nothing owed one time in eight, every root the model reads off
/// a side, an arc or a sharp corner's sweep sets the letter at its gap,
/// within bounds of the property's own scale, and is the first root along
/// the band curve, read from the path's own gap function. Its oracle is
/// that function, not the iterative rule's answer, which can be another
/// right center in its window (seed 19466) or a later root (seed 90637):
/// `check_pair_model` says how (review 5811563882).
///
/// Precondition: both holds are reached, many times. Controls (run, not
/// committed): an arc's curvature read with its sign turned
/// (`ArcPath::bend`), red on the model's gap round an ellipse of 40 × 40 —
/// a circle, built of arcs — at seed 2; `model_aim` aiming at a hundred
/// of its resolution where nothing is owed (review 5810838863), red at
/// seed 10 on a side ("its gap 0.012039185, outside [-0.00081357575,
/// 0.0024407273]") and, off one, at seed 47 round a 2,000 × 2,000 circle
/// and seed 50 round a sharp corner, where it was red on no form but a
/// side; a circle's root set a lap on (`model_root`: the same point, the
/// same gap), red at seed 2 on the first root ("past a root: at 75.577484
/// the gap is 5.5134983 already"); and the side check this replaced, the
/// two centers within twice the rule's window, red at seed 19466, where
/// the gap rises 0.19 a unit of center.
#[test]
fn test_the_pair_model_sets_the_first_root_at_what_is_owed() {
    use crate::flow::tests::flow_fuzz::pair_model_held;
    let mut held = [0usize; 2];
    for seed in 0..20_000 {
        match pair_model_held(&mut Choices::seeded(seed)) {
            Ok(h) => {
                for (n, h) in held.iter_mut().zip(h) {
                    *n += usize::from(h);
                }
            }
            Err(why) => panic!("seed {seed}: {why}"),
        }
    }
    assert!(
        held[0] > 5_000 && held[1] > 1_000,
        "precondition: the model is held to its window {} times, on a side with the rule's gap in its window {} times",
        held[0],
        held[1]
    );
}

/// Every property, over a hundred seeded inputs of the structured
/// generator; among them, far-out frames with sides and far-out `Repeat`s
/// along a path (#313), each laid within the per-input bound.
///
/// Precondition: the seeds draw far-out frames with sides and far-out
/// `Repeat`s. Control: with `far_out` drawing nothing, red.
#[test]
fn test_generated_flows_hold_every_flow_property() {
    use crate::flow::tests::flow_fuzz::{FAR_FRAMES, FAR_REPEATS};
    FAR_FRAMES.with(|n| n.set(0));
    FAR_REPEATS.with(|n| n.set(0));
    for seed in 0..100 {
        if let Err(why) = check_flow_generated(&mut Choices::seeded(seed)) {
            panic!("seed {seed}: {why}");
        }
    }
    let (frames, repeats) = (FAR_FRAMES.with(|n| n.get()), FAR_REPEATS.with(|n| n.get()));
    assert!(
        frames > 0 && repeats > 0,
        "precondition: the seeds draw far-out frames with sides ({frames}) and far-out repeats along a path ({repeats})"
    );
}

/// The sweep above over [`every_face`]: `faces()`, the ornament
/// fallback's two (#297) and the default face (#228), which a map's
/// `font` can pin as it pins any other. `faces()` stays as it is, since
/// every committed structured seed and finding chose its faces from that
/// list; this sweep and the flow_structured_every_face target are where a
/// span pins `Mandala Ornaments`, `Mandala Emoji` — the first compiled
/// face with no outline and no space — or `Inter Variable`, the first
/// variable one. The seeds are other inputs than the sweep above's, the
/// list being longer.
///
/// Precondition: each fallback face is pinned by some seed's spans or
/// flow, so the sweep cannot pass without meeting them. Control: with
/// `every_face` returning `faces()` alone, red on it.
#[test]
fn test_generated_flows_over_every_face_hold_every_flow_property() {
    let faces = every_face();
    let mut pinned: Vec<AppFont> = Vec::new();
    for seed in 0..100 {
        match check_flow_generated_in(&mut Choices::seeded(seed), &faces) {
            Ok(fonts) => pinned.extend(fonts),
            Err(why) => panic!("seed {seed}: {why}"),
        }
    }
    for (font, _) in crate::font::fallback::ORNAMENT_FALLBACK
        .iter()
        .chain(std::iter::once(&crate::font::fallback::DEFAULT_FACE))
    {
        let times = pinned.iter().filter(|pin| *pin == font).count();
        assert!(
            times > 0,
            "precondition: a span of the sweep pins {font:?} (pinned {} faces)",
            pinned.len()
        );
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
        check_flow_bytes_in_every_fit(text.as_bytes())
            .unwrap_or_else(|why| panic!("{text:?}, in every fit: {why}"));
    }
    check_flow_bytes(&[0xFF, 0xC0, b'a', 0x80]).unwrap();
    check_flow_bytes_in_every_fit(&[0xFF, 0xC0, b'a', 0x80]).unwrap();
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
        check_placement(&run, &placed(Vec2::ONE, 1), false, "p"),
        Ok(()),
        "precondition: the control passes"
    );
    assert!(
        check_placement(&run, &placed(Vec2::new(f32::NAN, 0.0), 0), false, "p")
            .unwrap_err()
            .contains("placed at")
    );
    assert!(check_placement(&run, &placed(Vec2::ONE, 2), false, "p")
        .unwrap_err()
        .contains("placed glyph 2 of 2"));
}

/// The style of the flow finding: a face whose space has no width, at
/// the weight and slant SilhousTwo has (R30). The finding was shaped at
/// 200 italic; SilhousTwo has only a 400 upright face, and before R30 a
/// weight it lacks was drawn in whatever host face had that weight — the
/// CI runner's `DejaVuSans-ExtraLight`, whose space measures — so these
/// tests never met a zero-width space there.
fn zero_width_space_style() -> ResolvedText {
    ResolvedText {
        font: AppFont::SilhousTwo,
        size: FontSize::Pt(16.0),
        color: Rgba::new(255, 255, 255, 255),
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// `Err` naming the first laid-out glyph of `buffer` not drawn in a
/// compiled face of `font`'s family (ruling R30): the precondition that a
/// test of a face shapes in that face, on every machine.
fn drawn_in(buffer: &crate::font::Buffer, font: AppFont) -> Result<(), String> {
    drawn_in_lines(buffer, font, |_| true)
}

/// [`drawn_in`] for the lines `which` keeps.
fn drawn_in_lines(
    buffer: &crate::font::Buffer,
    font: AppFont,
    which: impl Fn(usize) -> bool,
) -> Result<(), String> {
    let fs = crate::font::fonts::acquire_font_system_write("drawn_in");
    let pin = crate::font::fonts::pinned_family(&fs, font).ok_or("the face has no family")?;
    for run in buffer.layout_runs().filter(|run| which(run.line_i)) {
        for glyph in run.glyphs {
            if !pin.faces.contains(&glyph.font_id) {
                let face = fs
                    .db()
                    .face(glyph.font_id)
                    .map(|info| info.post_script_name.clone());
                return Err(format!(
                    "{:?} of line {} is drawn in {face:?}, not in {:?}",
                    run.text.get(glyph.start..glyph.end),
                    run.line_i,
                    pin.name
                ));
            }
        }
    }
    Ok(())
}

/// `Err` naming the first glyph of span `span` of `run` not drawn in a
/// compiled face of `font`'s family.
fn span_drawn_in(run: &crate::flow::ShapedRun, span: usize, font: AppFont) -> Result<(), String> {
    let fs = crate::font::fonts::acquire_font_system_write("span_drawn_in");
    let pin = crate::font::fonts::pinned_family(&fs, font).ok_or("the face has no family")?;
    for (glyph, metrics) in run.layout_glyphs().zip(&run.glyphs) {
        if metrics.span == span && !pin.faces.contains(&glyph.font_id) {
            let face = fs
                .db()
                .face(glyph.font_id)
                .map(|info| info.post_script_name.clone());
            return Err(format!(
                "a glyph of span {span} is drawn in {face:?}, not in {:?}",
                pin.name
            ));
        }
    }
    Ok(())
}

/// The precondition of the zero-width tests: SilhousTwo's space has no
/// width as drawn here, in SilhousTwo.
fn assert_zero_width_space(face: &ResolvedText) {
    let space = with_fonts(|fs| real_run(" ", face, fs));
    if let Err(why) = drawn_in(&space.buffer, AppFont::SilhousTwo) {
        panic!("precondition: {why}");
    }
    assert_eq!(
        space.glyphs[0].advance, 0.0,
        "precondition: the space has no width"
    );
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
    assert_zero_width_space(&face);
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
        for buffer in [&run.buffer, &shaped.buffer] {
            if let Err(why) = drawn_in(buffer, AppFont::SilhousTwo) {
                panic!("precondition, {text:?}: {why}");
            }
        }
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
    // Preconditions (R30): the face that cannot measure is SilhousTwo's
    // zero-width space, on the run and on line 1 of the box.
    assert_zero_width_space(&face);
    if let Err(why) = span_drawn_in(&mixed, 1, AppFont::SilhousTwo) {
        panic!("precondition: {why}");
    }
    if let Err(why) = drawn_in_lines(&box_mixed.buffer, AppFont::SilhousTwo, |line| line == 1) {
        panic!("precondition: {why}");
    }
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
    let mixed_box = |fs: &mut crate::font::fonts::FontSystemLease| {
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
    let mixed_run = |fs: &mut crate::font::fonts::FontSystemLease| {
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

/// Ruling R30: a span pinned to a family, at a weight the family's
/// compiled faces lack, is drawn in that family at the nearest weight it
/// has — on every machine. Before R30 cosmic-text skipped the pin and
/// drew it in any face of the exact weight: the CI runner's
/// `DejaVuSans-ExtraLight` for SilhousTwo at 200. Since #228 the face
/// that would take it is the default face, the first face of the
/// fallback, whose `wght` axis covers 200 and which holds every letter
/// of the text. This is the style the R24 finding was shaped in, and the
/// control that the zero-width tests' precondition bites.
#[test]
fn test_a_span_at_a_weight_its_family_lacks_is_drawn_in_its_family() {
    use crate::font::fallback::DEFAULT_FACE;
    use crate::font::fonts::pinned_family;
    let face = ResolvedText {
        weight: Weight::new(200).unwrap(),
        italic: true,
        ..zero_width_space_style()
    };
    let (default, pin) = with_fonts(|fs| {
        let fs: &crate::font::FontSystem = fs.fonts();
        (
            pinned_family(fs, DEFAULT_FACE.0),
            pinned_family(fs, AppFont::SilhousTwo),
        )
    });
    assert!(
        default.is_some_and(|default| default
            .axes
            .iter()
            .any(|(lightest, heaviest)| (*lightest..=*heaviest).contains(&200))),
        "precondition: a face at 200 the matcher falls back to"
    );
    let pin = pin.expect("precondition: SilhousTwo has a family");
    assert_eq!(
        pin.weights,
        vec![400],
        "precondition: SilhousTwo has only a 400 face"
    );
    assert_eq!(pin.weight_for(200), 400);
    for text in ["a b\tc", "\t"] {
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
        for (what, buffer) in [("run", &run.buffer), ("box", &shaped.buffer)] {
            if let Err(why) = drawn_in(buffer, AppFont::SilhousTwo) {
                panic!("{text:?}, the {what}: {why}");
            }
        }
    }
}

/// R24.2 on a real `0 / 0`: the up-front check, told by
/// `TRUST_EVERY_FACE` to pass every face, lets SilhousTwo's tab through
/// — as it would one shaped in a fallback face it cannot see — and
/// cosmic-text lays it out with a zero-width space as its stop. The
/// last-resort check finds it and lays out again with the tab as a
/// space: on that line alone in a box, and throughout a run. Since R30
/// SilhousTwo is drawn in SilhousTwo on every machine, so this is the
/// same `0 / 0` everywhere; `test_a_tab_the_up_front_check_lets_through_is_laid_out_again_as_a_space`
/// drives the same guard through a face whose stops are finite.
#[test]
fn test_a_real_zero_width_tab_stop_the_up_front_check_lets_through_is_laid_out_again() {
    use crate::flow::shape::{failing_lines, TABS_RELAID, TRUST_EVERY_FACE};
    let plain = style(16.0);
    let face = zero_width_space_style();
    assert_zero_width_space(&face);
    let first_on = |shaped: &crate::flow::ShapedBox, line: usize| {
        shaped
            .buffer
            .layout_runs()
            .find(|run| run.line_i == line)
            .and_then(|run| run.glyphs.first().map(|glyph| glyph.w))
    };
    let box_spans = [
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
    ];
    let run_spans = [
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
    ];
    let (box_alone, hooked_box, hooked_run, relaid) = with_fonts(|fs| {
        let box_alone = shape_box(&box_spans[..1], None, Align::Left, fs);
        TRUST_EVERY_FACE.with(|trust| trust.set(true));
        let before = TABS_RELAID.with(std::cell::Cell::get);
        let hooked_box = shape_box(&box_spans, None, Align::Left, fs);
        let hooked_run = shape_run(&run_spans, 1.0, fs);
        TRUST_EVERY_FACE.with(|trust| trust.set(false));
        let relaid = TABS_RELAID.with(std::cell::Cell::get) - before;
        (box_alone, hooked_box, hooked_run, relaid)
    });
    // Preconditions: the tab that came through is SilhousTwo's, and the
    // guard, not the up-front check, laid both flows out again.
    if let Err(why) = drawn_in_lines(&hooked_box.buffer, AppFont::SilhousTwo, |line| line == 1) {
        panic!("precondition: {why}");
    }
    if let Err(why) = span_drawn_in(&hooked_run, 1, AppFont::SilhousTwo) {
        panic!("precondition: {why}");
    }
    assert_eq!(relaid, 2, "precondition: the guard laid both flows out again");
    assert_eq!(
        (
            failing_lines(&hooked_box.buffer),
            failing_lines(&hooked_run.buffer)
        ),
        (Vec::new(), Vec::new()),
        "after the guard, nothing laid out is not finite"
    );
    assert_eq!(
        first_on(&hooked_box, 0),
        first_on(&box_alone, 0),
        "the line that came out finite keeps its tab stop"
    );
    assert_eq!(
        first_on(&hooked_box, 1),
        Some(0.0),
        "the failing line's tab is SilhousTwo's space"
    );
    assert_eq!(check_box(&box_spans, &hooked_box, None), Ok(()), "the box");
    assert_eq!(check_run(&run_spans, &hooked_run, "the run"), Ok(()), "the run");
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
    use crate::flow::{place_on_closed, Outline, Rect, RingLayout};
    let found = include_bytes!("fuzz_findings/spread_gap_past_the_largest_f32");
    if let Err(why) = check_flow_structured_as_found(found) {
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
                let (replaced, placed) = place_on_closed(
                    &run,
                    &path,
                    start,
                    direction,
                    &fit,
                    &RingLayout::whole(),
                    false,
                    &plain,
                    fs,
                );
                (replaced.unwrap_or(run), placed)
            });
            let (run, placed) = placed;
            assert!(
                placed.glyphs.len() == 3,
                "precondition: {fit:?} {direction:?} places all three glyphs ({})",
                placed.glyphs.len()
            );
            if let Err(why) = check_placement(&run, &placed, true, "around the outline") {
                panic!("{fit:?} {direction:?}: {why}");
            }
        }
    }
}

/// The flow_structured smoke's finding on #294 (CI run 41234757416, a
/// libFuzzer timeout): a `Repeat` of twenty glyphs along a quadratic
/// whose control point is `1e30` off its ends — and the review's of the
/// same kind, thirty glyphs along a cubic with a control point at
/// `-4.94e33`. The path is about `1e30`
/// long, so the walk lays copies to the `MAX_PLACED_GLYPHS` cap — a
/// million glyphs then, 65,536 since ruling R41 — which any `Repeat` as long as the canvas reaches
/// too. The cost was in how often each was walked: the readable pass laid
/// the placement again to measure each stretch and the reading order
/// before laying them, five walks more in all, and every step looked up
/// the point it starts from three times over. Now a placement's walks
/// share the steps they take (`StepMemo`), and a step looks its start
/// up once.
///
/// So the work is bounded by what is placed: at most three steps taken
/// afresh per placed glyph — the walk, a stretch laid from where it
/// borrowed room, and the reading-order lay of what the walk reverses —
/// and fourteen path lookups, four a step and the glyphs' corners.
/// Precondition: each input lays the cap. Control: with every walk
/// stepping afresh, or a step's start looked up per chord, the counts
/// pass their bounds.
#[test]
fn test_a_repeat_along_a_curve_as_long_as_an_f32_walks_each_glyph_thrice_at_most() {
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::{GLYPHS_PLACED, MAX_RUN_ON_STEPS, STEPS_TAKEN};
    use crate::flow::MAX_PLACED_GLYPHS;
    let findings: [(&str, &[u8]); 2] = [
        (
            "the CI input",
            include_bytes!("fuzz_findings/repeat_along_a_curve_past_the_root_of_the_largest_f32"),
        ),
        (
            "the review's cubic",
            include_bytes!("fuzz_findings/repeat_along_a_cubic_past_the_root_of_the_largest_f32"),
        ),
    ];
    for (what, found) in findings {
        for counter in [&STEPS_TAKEN, &GLYPHS_PLACED, &PATH_LOOKUPS] {
            counter.with(|count| count.set(0));
        }
        if let Err(why) = check_flow_structured_as_found(found) {
            panic!("{what}: {why}");
        }
        let [steps, placed, lookups] =
            [&STEPS_TAKEN, &GLYPHS_PLACED, &PATH_LOOKUPS].map(|counter| counter.with(std::cell::Cell::get));
        assert!(
            placed + 64 > MAX_PLACED_GLYPHS,
            "precondition: {what} lays the cap ({placed} glyphs)"
        );
        // Past the text's end the readable walk probes at most
        // `MAX_RUN_ON_STEPS` steps more for where the path stops running
        // its last stretch's way: a bound of its own, not the glyphs'.
        assert!(
            steps <= 3 * placed + MAX_RUN_ON_STEPS,
            "{what}: {steps} steps taken for {placed} glyphs placed"
        );
        // And each step of that probe looks the path up a few times.
        assert!(
            lookups <= 14 * placed + 8 * MAX_RUN_ON_STEPS,
            "{what}: {lookups} path lookups for {placed} glyphs placed"
        );
    }
}

/// The finding above along an open line, where `solve_gap` still lays
/// a `Spread` (a frame round a closed outline is laid along its halves
/// now, #291): along a line as long as an `f32` goes, a `Spread` — and a
/// `Fit` held at scale 1 — of three letters places every glyph, finite,
/// and cuts nothing (`check_placement`'s whole-placement property). A
/// line longer than `√f32::MAX` (about `1.8e19`) once measured its
/// length infinite, kept no stretch, and placed nothing; the lines here
/// run from there to the largest. Control: with path lengths measured
/// by `Vec2::length`, which overflows there, the property sees the cut.
#[test]
fn test_a_spread_along_a_line_as_long_as_an_f32_places_every_glyph() {
    use crate::document::values::FitMode;
    use crate::flow::{place_on_open, Curve};
    let plain = style(16.0);
    for reach in [1.0e19_f32, 1.0e20, 1.0e30, 1.7e38] {
        let curve = Curve::Line(Vec2::new(-reach, 0.0), Vec2::new(reach, 0.0));
        for fit in [FitMode::Spread, FitMode::Fit { min: 1.0, max: 1.0 }] {
            let (run, placed) = with_fonts(|fs| {
                let run = real_run("abc", &plain, fs);
                let (replaced, placed) = place_on_open(&run, &curve, &fit, 0.0, &plain, fs);
                (replaced.unwrap_or(run), placed)
            });
            if let Err(why) = check_placement(&run, &placed, true, "along the line") {
                panic!("{reach:e} {fit:?}: {why}");
            }
        }
    }
}

/// `check_placement`'s whole-placement property: a `Spread` — and a
/// `Fit` held at scale 1 — of three letters along a line from one side
/// of the canvas to the other, the longest a map can draw, places every
/// glyph and cuts nothing. Control: the property refuses a placement
/// that left a glyph out.
#[test]
fn test_a_spread_across_the_canvas_places_every_glyph() {
    use crate::document::bounds::MAX_CANVAS_COORD;
    use crate::document::values::FitMode;
    use crate::flow::{place_on_open, Curve};
    let plain = style(16.0);
    let curve = Curve::Line(
        Vec2::new(-MAX_CANVAS_COORD, 0.0),
        Vec2::new(MAX_CANVAS_COORD, 0.0),
    );
    for fit in [FitMode::Spread, FitMode::Fit { min: 1.0, max: 1.0 }] {
        let (run, placed) = with_fonts(|fs| {
            let run = real_run("abc", &plain, fs);
            let (replaced, placed) = place_on_open(&run, &curve, &fit, 0.0, &plain, fs);
            (replaced.unwrap_or(run), placed)
        });
        if let Err(why) = check_placement(&run, &placed, true, "across the canvas") {
            panic!("{fit:?}: {why}");
        }
        let mut short = placed.clone();
        short.glyphs.pop();
        assert!(
            check_placement(&run, &short, true, "short")
                .unwrap_err()
                .contains("was not placed"),
            "{fit:?}: the property sees a glyph left out"
        );
    }
}

/// The coordinator's flow_structured finding on #294 (CI run
/// 41245750394, a libFuzzer timeout at aef0eba4): a `Repeat` of an
/// ornament, eighteen glyphs, along a cubic with a control point at
/// `-1e30`. The walk lays the `MAX_PLACED_GLYPHS` cap (a million glyphs
/// then, 65,536 since ruling R41),
/// every one stepped by its ink (ruling R31), and on a path that far
/// out its points are an `f32` step or two apart where the walk is, so
/// no glyph's ink gap could come within `INK_TOLERANCE` of what it
/// owes: each took every one of `INK_TURNS` turns, and each turn every
/// one of `INK_ROUNDS` rounds of bisection — 319 path lookups a glyph.
/// A round that leaves its bracket as it was, and a turn that leaves
/// its center where it was, leave them so every round and turn after,
/// so each stops there, which changes no position.
///
/// So an ink-stepped glyph costs at most 24 path lookups here. Precondition:
/// the input lays the cap, and is an ornament (no `STEPS_TAKEN` for most
/// glyphs, which an ornament takes by its ink). Control: with either
/// stop removed, the lookups pass their bound.
///
/// Since #333 a face draws one of its glyphs with no outline, a host's
/// color emoji, which is inked as its box, and the whole-glyph placement
/// the replay lays beside the walk (`place_pitched`) lays it 65,536 times
/// along the same path, whose floats cannot see its slack either: its
/// solve stops by its third round there, and it is held to a bound of
/// its own, the walk's count leaving it out. Control: the slack's rounds
/// without their stops, the loop before #333, red: "the whole-glyph
/// placement: 8181179 path lookups for 65536 glyphs placed, at most
/// 3145788".
#[test]
fn test_a_repeat_of_an_ornament_where_floats_cannot_resolve_its_gaps_stops_solving_them() {
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::{GLYPHS_PLACED, MAX_RUN_ON_STEPS, STEPS_TAKEN};
    use crate::flow::MAX_PLACED_GLYPHS;
    let found = include_bytes!("fuzz_findings/repeat_of_an_ornament_along_a_cubic_its_floats_cannot_resolve");
    use crate::flow::place::pitch::{REFINE, SLACK_ROUNDS};
    use crate::flow::tests::flow_fuzz::{PITCHED_GLYPHS, PITCHED_LOOKUPS};
    let counters = [
        &STEPS_TAKEN,
        &GLYPHS_PLACED,
        &PATH_LOOKUPS,
        &PITCHED_LOOKUPS,
        &PITCHED_GLYPHS,
    ];
    for counter in counters {
        counter.with(|count| count.set(0));
    }
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("the fuzzer's input: {why}");
    }
    let [steps, placed, lookups, pitched, pitched_glyphs] =
        counters.map(|counter| counter.with(std::cell::Cell::get));
    // The walk's: the whole-glyph placement the replay lays beside it is
    // a placement of its own, held to its own bound below (#296), and
    // lays this input's glyphs too where a face draws one of them with no
    // outline, a bitmap, and inks it as its box (#333).
    let lookups = lookups - pitched;
    assert!(
        placed + 64 > MAX_PLACED_GLYPHS && steps < placed / 4 + MAX_RUN_ON_STEPS,
        "precondition: the input lays the cap of an ornament ({placed} glyphs, {steps} box steps)"
    );
    // The readable walk's run-on probe past the text's end is a bound
    // of its own (`MAX_RUN_ON_STEPS` steps), a few lookups a step.
    assert!(
        lookups <= 24 * placed + 8 * MAX_RUN_ON_STEPS,
        "{lookups} path lookups for {placed} glyphs placed"
    );
    // The whole-glyph placement stops solving its slack too (#333): an
    // ink is placed in `4 + REFINE` lookups a lay at most — its point, its
    // direction, its guess, and its gap's first measure and refinements —
    // and it is laid once to count the inks, once at the least slack, in
    // the slack's rounds, which stop by the third where the floats cannot
    // see the slack (the first move answered, the second not), and once
    // as kept; each lay's two ends a solve of `1 + REFINE` measures each.
    let rounds = SLACK_ROUNDS.min(3);
    let lays = 1 + 1 + rounds + 1;
    let bound = (4 + REFINE) * lays * pitched_glyphs + 2 * (1 + REFINE) * lays;
    assert!(
        pitched_glyphs > 0 && pitched <= bound,
        "the whole-glyph placement: {pitched} path lookups for {pitched_glyphs} glyphs placed, at most {bound}"
    );
}

/// The phase-2 review's flow_raw finding on #294 (a slow unit, 20.75 s
/// against `-timeout=20` under the fuzz build): 26 bytes, mostly
/// U+FFFD read lossily, which `check_flow_bytes` lays as a `Fit` frame
/// round its 120 × 40 rounded rectangle (the mode it draws, ruling R45.1;
/// it replays through `check_flow_bytes_in_every_fit`, as it was found,
/// with a `Spread` and a `Once` frame besides). Its first build left a line
/// pressed, so the shrink ran: `SHRINK_ROUNDS` probes, each a whole
/// clearance search from nothing with its backtracks — 15 builds and
/// 17,720 `track`s for that frame, 22,016 for the input. A probe now
/// looks first whether its lines fit at all and grows its clearance
/// from the last size held; a clearance search lays a line once for
/// each part it tries (`LayMemo`); and (ruling R38.3) a line's tracking
/// is solved by regula falsi from the straight path's answer rather than
/// by bisection, its slide and its centering likewise, and a round of
/// the clearance draws back only the line with the slack to give, its
/// step damped by what drawing back eases the break wanted — so the
/// review's input is now held at R27's size, 1,083 `track`s in all.
///
/// The shrink itself is held by a frame that reaches it on every
/// machine: the README quatrain in the pinned face (`FACE`, so no host
/// face draws a glyph of it, ruling R30) round the aspect sweep's
/// 600 × 12 rectangle, inward, where its two lines' inks are together
/// taller than the outline. (Outward it once shrank too; since a letter
/// round a sharp corner is held clear of the letters before its
/// neighbor, and a junction's end draws back along the path before it
/// (ruling R43), it is held at R27's size there.) The raw target's own style is
/// `AppFont::Any`, drawn in whatever face the host falls back to, so a
/// raw input's path through the last resorts is the host's: the seed
/// `flow_raw/controls_and_tabs`, which this test once held here, shrank
/// on one machine and not on CI's.
///
/// The shrink stops once its bracket is within 1% of the scale held, or
/// spans under a quarter point (ruling R42.2): the quatrain takes 8
/// builds, at most 10.
///
/// Control: with the tracking bisected, the review's input passes its
/// bound (1,943 `track`s). With the shrink bisected to its full
/// precision (C64), the quatrain takes 14 builds. The line memo, the cheap look, the seeded
/// clearance and the probe's single halving no longer move either count
/// past its bound on their own: each is a refinement no claim here rests
/// on.
#[test]
fn test_a_shrunk_frame_lays_a_bounded_number_of_lines() {
    use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
    use crate::flow::place::halves::{BUILDS, TRACKS};
    use crate::flow::{outline, place_on_closed, RingLayout};
    let counted = |run: &mut dyn FnMut()| {
        TRACKS.with(|count| count.set(0));
        BUILDS.with(|count| count.set(0));
        run();
        (
            TRACKS.with(std::cell::Cell::get),
            BUILDS.with(std::cell::Cell::get),
        )
    };
    let review = include_bytes!("fuzz_findings/fit_frame_shrunk_by_whole_clearance_searches");
    let (tracks, _) = counted(&mut || {
        if let Err(why) = check_flow_bytes_in_every_fit(review) {
            panic!("the review's input: {why}");
        }
    });
    assert!(
        tracks <= 1_500,
        "the review's input: {tracks} lines tracked, at most 1,500"
    );

    let quatrain = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower, \
         Hold Infinity in the palm of your hand, And Eternity in an hour. ";
    let flow = style(18.0);
    let thin = outline(
        crate::flow::Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    // Asserted outside the font lock, so a failure cannot poison it for
    // the tests after.
    let (mut pinned, mut truncated) = (false, true);
    let (tracks, builds) = counted(&mut || {
        (pinned, truncated) = with_fonts(|fs| {
            let run = real_run(quatrain, &flow, fs);
            let pinned = crate::font::fonts::pinned_family(fs.fonts(), flow.font)
                .is_some_and(|pins| run.layout_glyphs().all(|g| pins.faces.contains(&g.font_id)));
            let start = StartPoint {
                edge: Edge::Top,
                at: 0.0,
            };
            let fit = FitMode::Fit { min: 0.5, max: 2.0 };
            let (_, placed) = place_on_closed(
                &run,
                &thin,
                start,
                Direction::Clockwise,
                &fit,
                &RingLayout::whole(),
                false,
                &flow,
                fs,
            );
            (pinned, placed.truncated)
        });
    });
    assert!(pinned, "precondition: every glyph is drawn in the pinned face");
    assert!(!truncated, "the quatrain round the flat rectangle: nothing cut");
    assert!(
        builds > 3,
        "precondition: the quatrain round the flat rectangle shrinks ({builds} builds)"
    );
    assert!(
        tracks <= 2_000,
        "the shrunk quatrain: {tracks} lines tracked, at most 2,000"
    );
    // The shrink stops at a useful precision (ruling R42.2): 8 builds;
    // bisected to its full precision, 14.
    assert!(builds <= 10, "the shrunk quatrain: {builds} builds, at most 10");
}

/// The 300 s flow_raw run's finding on #294 (base64 `bnAgZXRhaWx4dA==`,
/// "np etailxt"): "around a rectangle: its natural length fits, and it
/// was cut". `check_flow_bytes` lays it as a `Fit` frame round its
/// 120 × 40 rounded rectangle (the mode it draws, ruling R45.1; it
/// replays through `check_flow_bytes_in_every_fit`, as it was found), "np" on the first half and "etailxt" on
/// the second. At span whole "np" is stretched over its whole half, so
/// its letter gap, and the break of two of them owed at the top-left
/// junction, was some 200 points; the round that grew that junction's
/// clearance by the whole shortfall, shared between the two lines, drew
/// "etailxt" back by 107 points of its 151, and the bottom-right
/// junction then pressed it. Nothing was left but the shrink, which
/// held nothing either, and R34's cut. Yet the line can be held: "np"
/// had 140 points to give. Under ruling R38.1 a cut is the last resort
/// only where the line cannot be held without overlap, so the cut was
/// the fault, not the property.
///
/// Now a round's step is damped by how much drawing the lines back eases
/// the break wanted (a line's gap narrows with its part), and each line
/// is drawn back no further than its slack — the room its part leaves
/// and its gaps past its font's own — what it cannot give handed to the
/// other line where that parts the ends too (ruling R38.3). The frame
/// is held, uncut, at R27's size.
///
/// The raw target shapes in `AppFont::Any`, in whatever face the host
/// falls back to, so this replay is this machine's path through it; the
/// two mechanisms are each held on every machine, in the pinned face, by
/// `test_flow_halves_a_frame_its_lines_can_hold_is_not_shrunk`. On this
/// input no one of R38.3's changes reverted alone, nor the slack cap and
/// the damping together, cuts the frame again: the replay holds the input
/// to the property, whose "it was cut" branch is shown red by
/// `test_a_spread_along_a_line_as_long_as_an_f32_places_every_glyph` under
/// the PR's control C30.
#[test]
fn test_a_frame_whose_other_line_can_give_the_clearance_is_not_cut() {
    let found = include_bytes!("fuzz_findings/fit_frame_pressed_by_a_clearance_its_other_line_could_give");
    if let Err(why) = check_flow_bytes_in_every_fit(found) {
        panic!("the fuzzer's input: {why}");
    }
}

/// The 300 s runs' findings at 9eeb66c0, both "its natural length fits,
/// and it was cut": a frame that cut a line so that no two inks touch
/// (ruling R38.1) and reported it in `Trace::cut_apart`, outward — the
/// raw input's second line is seven NULs, ink boxes a line high and no
/// wide, wrapped round the 120 × 40 rectangle's tight end, where two of
/// them that are not neighbors meet. The properties excused a reported
/// cut only inward; a line of tall inks round a tight bend cannot be
/// held apart outward either, so a reported cut is excused on both
/// sides. Each input replays under the pinned face; the raw one through
/// `check_flow_bytes_in_every_fit`, in every frame it was found among
/// (ruling R45.1: the harness now draws one, `Fit`, for it).
///
/// Control: with the excuse narrowed to inward frames again, the raw
/// input, laid outward, fails.
#[test]
fn test_a_frame_cut_apart_outward_is_owed_no_longer() {
    let raw = include_bytes!("fuzz_findings/raw_frame_cut_apart_round_a_tight_bend");
    if let Err(why) = check_flow_bytes_in_every_fit(raw) {
        panic!("the raw input: {why}");
    }
    let structured = include_bytes!("fuzz_findings/structured_frame_cut_apart_outward");
    if let Err(why) = check_flow_structured_as_found(structured) {
        panic!("the structured input: {why}");
    }
}

/// The 300 s flow_structured runs' findings at a7896c70, here and on
/// CI, both "around an outline: a glyph placed at [NaN, NaN]": one line
/// round an outline some 10²⁶ (here) or 10³⁰ (CI) long, its letters a
/// fraction of a point apart, owing a break of two word gaps that its
/// gaps, stretched over its half, made some 10³³ wide. The clearance
/// search grew both of its clearances toward that break until together
/// they passed its half many times over, and `Layout::room` took the
/// point where they meet as `ja + (jb - ja) * clear[0] / (clear[0] +
/// clear[1])`: the product overflowed to infinity, the line's part was
/// `(inf, inf)`, and `track` laid it at `inf - inf`. The share is now
/// taken over the larger clearance, so neither the product nor the sum
/// can overflow (`share`; the cases are held by
/// `test_flow_halves_a_part_never_inverts`). Each input is minimized
/// on stable.
///
/// Control: with the old quotient restored in `room`, each input fails
/// with "a glyph placed at [NaN, NaN]".
#[test]
fn test_a_frame_whose_clearances_pass_its_half_places_finitely() {
    for (name, found) in [
        (
            "here",
            &include_bytes!("fuzz_findings/structured_frame_clearances_past_its_half_overflow_their_meeting")
                [..],
        ),
        (
            "on CI",
            &include_bytes!(
                "fuzz_findings/structured_frame_clearances_past_its_half_overflow_their_meeting_on_ci"
            )[..],
        ),
    ] {
        if let Err(why) = check_flow_structured_as_found(found) {
            panic!("the input found {name}: {why}");
        }
    }
}

/// CI's flow_structured crash at 8baf8e6e (sha1 6f1864cf…), "around an
/// outline: its natural length fits, and it was cut": a rounded
/// rectangle whose width is not a number. Its support points' `x` is
/// not a number, yet the point of its path nearest each is somewhere,
/// so both halves' junctions fell at one place: halves of no length,
/// whose `Spread` line was cut to one cluster where R27 had laid it
/// whole along the side that is a number. A support that is not a
/// number now places its junction nowhere, and the frame keeps R27's
/// placement, as an outline whose halves cannot be measured does.
/// Minimized on stable (692 → 517 bytes).
///
/// Control: with the junctions taken from the nearest point again, it
/// fails with "its natural length fits, and it was cut".
#[test]
fn test_a_frame_round_an_outline_whose_width_is_not_a_number_is_not_cut() {
    let found = include_bytes!("fuzz_findings/structured_frame_round_an_outline_whose_width_is_not_a_number");
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("the fuzzer's input: {why}");
    }
}

/// The 300 s flow_raw run's timeout at 8baf8e6e (ruling R42; input sha1
/// 08d8dfda…, 420 bytes, about 400 NULs read as text), which the harness
/// laid as three frames round its 120 × 40 rounded rectangle — `Spread`,
/// `Fit` and `Once` — of lines of 164 and 255 clusters that touch at
/// R27's size. Since ruling R45.1 the harness lays one frame an input,
/// `Fit` for this one, and the replay lays all three
/// (`check_flow_bytes_in_every_fit`), as it was found. Its work was 11,488 `track`s, 16
/// builds and 378,758,104 path lookups: `cut_apart` ran a whole clearance
/// search per bisection probe, the `Fit` shrink bisected to a scale of
/// 0.99995, and a glyph step took some 220 lookups. With R42 it took
/// 2,893 `track`s and 69 s in the ASan `flow_raw` binary, too slow to be
/// committed where `scripts/fuzz.sh` feeds it to that binary. With
/// R43's cheaper steps and fewer of them it is committed: every
/// property holds, in at most 1,500 `track`s and 4,000,000 path lookups
/// (528 and 696,428 on one machine, 856 and 935,189 on another, while
/// its frames laid all 420 glyphs; the bounds are kept well clear of
/// every host's count, ruling R44.5), and no glyph step of a line above
/// 64 lookups. Since R44.2 the harness lays a frame from the first
/// [`MAX_FRAME_GLYPHS`](crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS)
/// glyphs of its text, what one frame lays inside the fuzz timeout: 247
/// `track`s and 340,712 lookups on the second machine. Since ruling
/// R46.3 the harness lays a frame from what its work fits inside the
/// timeout, at most 150 glyphs (152 since #304); the replay lays its text as it was found,
/// cut to 210 characters, 209 glyphs, at R44's 320
/// ([`FOUND_FRAME_GLYPHS`](crate::flow::tests::flow_fuzz::FOUND_FRAME_GLYPHS)),
/// so its frames are as they were.
///
/// Its NULs are drawn in whatever face the host falls back to, so its
/// path through the last resorts is the host's: on one machine its
/// `Spread` and `Once` frames cut a line apart and its `Fit` frame is
/// held at R27's size (3 builds), on another nothing is cut apart, on
/// CI's runner the `Fit` frame is shrunk (8 builds), and no bound here
/// counts builds or cuts. R42.1's bisection of `cut_apart` (C63) is held
/// on every host, in the pinned face, by
/// `test_flow_halves_a_frame_cut_apart_lays_a_bounded_number_of_lines`;
/// R42.2's stop by `test_a_shrunk_frame_lays_a_bounded_number_of_lines`.
///
/// Control: with a letter allowed 64 inks tried, a glyph step takes 192
/// lookups.
#[test]
fn test_a_raw_frame_cut_apart_lays_a_bounded_number_of_lines() {
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::halves::{BUILDS, MOST_STEP_LOOKUPS, TRACKS};
    let found = include_bytes!("fuzz_findings/raw_frame_cut_apart_by_whole_clearance_searches");
    for counter in [&TRACKS, &BUILDS, &PATH_LOOKUPS, &MOST_STEP_LOOKUPS] {
        counter.with(|count| count.set(0));
    }
    let checked = check_flow_bytes_in_every_fit(found);
    let [tracks, builds, lookups, most_step] = [&TRACKS, &BUILDS, &PATH_LOOKUPS, &MOST_STEP_LOOKUPS]
        .map(|counter| counter.with(std::cell::Cell::get));
    if let Err(why) = checked {
        panic!("the fuzzer's input: {why}");
    }
    let counts =
        format!("{tracks} tracks, {builds} builds, {lookups} path lookups, a step at most {most_step}");
    assert!(
        builds >= 3,
        "precondition: its three frames are laid along their halves ({counts})"
    );
    assert!(
        tracks <= 1_500,
        "the fuzzer's input: {counts}; at most 1,500 tracks"
    );
    assert!(
        lookups <= 4_000_000,
        "the fuzzer's input: {counts}; at most 4,000,000 lookups"
    );
    assert!(
        most_step <= 64,
        "the fuzzer's input: {counts}; at most 64 lookups a step"
    );
}

/// R44.4: the flow fuzzers place frames in every layout a document can
/// give a ring, not only the built-in: drawn from choices, a ring's
/// lines span natural, whole and unit fractions of their halves, its
/// sides are none or a short ornament, and its spacing none or some —
/// and choices of zero, an input that ends before them, draw the
/// built-in, so every finding before them replays as it did.
///
/// Control: with every ring drawn the built-in, as before R44.4, red.
#[test]
fn test_the_flow_fuzzers_draw_every_ring_layout() {
    use crate::document::values::LineSpan;
    use crate::flow::tests::flow_fuzz::ring_layout;
    use crate::flow::RingLayout;
    assert_eq!(
        ring_layout(&mut Choices::new(&[])),
        RingLayout::whole(),
        "zero choices draw the built-in"
    );
    let (mut natural, mut whole, mut fraction, mut sides, mut spacing) = (0, 0, 0, 0, 0);
    for seed in 0..200 {
        let layout = ring_layout(&mut Choices::seeded(seed));
        if layout.span == LineSpan::NATURAL {
            natural += 1;
        } else if layout.span == LineSpan::WHOLE {
            whole += 1;
        } else {
            fraction += 1;
        }
        sides += usize::from(layout.sides.is_some());
        spacing += usize::from(layout.spacing > 0.0);
    }
    let drawn = format!(
        "{natural} natural, {whole} whole, {fraction} fractions, {sides} with sides, {spacing} with spacing"
    );
    assert!(
        natural > 0 && whole > 0 && fraction > 0 && sides > 0 && spacing > 0,
        "200 layouts drawn: {drawn}"
    );
}

/// R50.3: the structured flow fuzzer draws long side ornaments, so that
/// a geometry that breaks `touch_looks` is a finding: drawn after
/// the ring's layout, one time in four, up to 200 bytes of a sequence of
/// [`ORNAMENT_GLYPHS`] repeated — a wide glyph among thin ones — and on
/// zero choices, an input that ends before it, not at all, so every
/// committed finding replays in the layout it was found in.
///
/// Control: with no long ornament ever drawn, red.
///
/// [`ORNAMENT_GLYPHS`]: crate::flow::tests::flow_fuzz::ORNAMENT_GLYPHS
#[test]
fn test_the_structured_flow_fuzzer_draws_long_mixed_ornaments() {
    use crate::flow::tests::flow_fuzz::{long_ornament, LONG_ORNAMENT_BYTES};
    assert_eq!(
        long_ornament(&mut Choices::new(&[]), RingLayout::whole()),
        RingLayout::whole(),
        "zero choices draw no long ornament"
    );
    let (mut drawn, mut long, mut mixed, mut most) = (0, 0, 0, 0);
    for seed in 0..400 {
        let layout = long_ornament(&mut Choices::seeded(seed), RingLayout::whole());
        let Some((ornament, _)) = layout.sides else {
            continue;
        };
        drawn += 1;
        most = most.max(ornament.len());
        long += usize::from(ornament.len() >= 100);
        let mut glyphs: Vec<char> = ornament.chars().collect();
        glyphs.sort_unstable();
        glyphs.dedup();
        mixed += usize::from(ornament.len() >= 100 && glyphs.len() >= 2);
    }
    assert!(
        drawn >= 50 && long >= 20 && mixed >= 10 && most <= LONG_ORNAMENT_BYTES,
        "400 seeds: {drawn} long ornaments drawn, {long} of 100 bytes or more, {mixed} of them of two glyphs or more, the longest {most} bytes"
    );
}

/// R51.2: the structured flow fuzzer draws texts of many sizes, so that
/// a touch check of many classes of height is reached and a geometry
/// that breaks `touch_looks` is a finding: drawn after the ring's layout
/// and its long ornament, one time in four, spans added to
/// [`MIXED_SIZE_SPANS`] at most, each at 0.5 pt times a power of two
/// over the whole font-size window, 0.5 to 4,096 pt, as the frame is
/// scaled; and on zero choices not at all, so every committed finding
/// replays the text it was found with.
///
/// Control: with no span ever added, red.
///
/// [`MIXED_SIZE_SPANS`]: crate::flow::tests::flow_fuzz::MIXED_SIZE_SPANS
#[test]
fn test_the_structured_flow_fuzzer_draws_texts_of_many_sizes() {
    use crate::document::resolve::ResolvedText;
    use crate::document::values::FontSize;
    use crate::flow::tests::flow_fuzz::{faces, mixed_sizes, MIXED_SIZE_SPANS};
    let faces = faces();
    let texts = vec!["A frame".to_string()];
    let styles = vec![crate::flow::tests::support::style(16.0)];
    let (none, unstyled) = mixed_sizes(&mut Choices::new(&[]), &texts, &styles, 1.0, &faces);
    assert!(
        none == texts && unstyled.len() == 1,
        "zero choices add no span: {none:?}"
    );
    let points = |style: &ResolvedText, scale: f32| match style.size {
        FontSize::Pt(points) => points * scale,
        _ => f32::NAN,
    };
    let (mut drawn, mut sixteen, mut most_sizes) = (0, 0, 0);
    let (mut least, mut largest) = (f32::INFINITY, 0.0_f32);
    for seed in 0..400 {
        let scale = [1.0, 0.25, 3.0][seed as usize % 3];
        let (texts, styles) = mixed_sizes(&mut Choices::seeded(seed), &texts, &styles, scale, &faces);
        assert!(
            texts.len() == styles.len() && texts.len() <= MIXED_SIZE_SPANS,
            "seed {seed}: {} texts, {} styles",
            texts.len(),
            styles.len()
        );
        if texts.len() == 1 {
            continue;
        }
        drawn += 1;
        sixteen += usize::from(texts.len() == MIXED_SIZE_SPANS);
        let mut sizes: Vec<u32> = styles[1..]
            .iter()
            .map(|s| (points(s, scale) * 2.0).round() as u32)
            .collect();
        sizes.sort_unstable();
        sizes.dedup();
        most_sizes = most_sizes.max(sizes.len());
        for style in &styles[1..] {
            least = least.min(points(style, scale));
            largest = largest.max(points(style, scale));
        }
    }
    assert!(
        drawn >= 50 && sixteen >= 3 && most_sizes >= 10 && (least - 0.5).abs() < 1e-3 && (largest - 4_096.0).abs() < 1.0,
        "400 seeds: {drawn} texts of more sizes drawn, {sixteen} of {MIXED_SIZE_SPANS} spans, at most {most_sizes} sizes in one, from {least} to {largest} pt as scaled"
    );
}

/// R51.2: the two committed structured findings whose bytes run on past
/// the mixed-size draw — `structured_frame_cut_apart_outward` and
/// `structured_frame_long_ornament_past_the_input_bound`, both found
/// before it — draw a text of more sizes now, and a replay lays each
/// with its text as found as well, as it lays a finding's layout as
/// found (`laid_layouts`).
///
/// Control: with a replay laying the drawn text alone, red.
#[test]
fn test_a_committed_structured_finding_replays_the_text_it_was_found_with() {
    use crate::flow::tests::flow_fuzz::{FOUND_TEXT_LAID, MIXED_DRAWN};
    for (name, found) in [
        (
            "structured_frame_cut_apart_outward",
            &include_bytes!("fuzz_findings/structured_frame_cut_apart_outward")[..],
        ),
        (
            "structured_frame_long_ornament_past_the_input_bound",
            &include_bytes!("fuzz_findings/structured_frame_long_ornament_past_the_input_bound")[..],
        ),
    ] {
        MIXED_DRAWN.with(|n| n.set(0));
        FOUND_TEXT_LAID.with(|n| n.set(0));
        if let Err(why) = check_flow_structured_as_found(found) {
            panic!("{name}: {why}");
        }
        let (drawn, laid) = (MIXED_DRAWN.with(|n| n.get()), FOUND_TEXT_LAID.with(|n| n.get()));
        assert!(drawn > 0, "precondition: {name} draws a text of more sizes");
        assert!(
            laid > 0,
            "{name} is not replayed with the text it was found with ({drawn} drawn)"
        );
    }
}

/// The flow_structured finding of CI's run at 3e767241 (ruling R52): a
/// `Fit { min: 3.56e-22, max: 141 }` line of 18 glyphs at 16 pt along a
/// straight path 1.40 long, its natural length 346. The engine shrinks
/// it to the size floor, `MIN_FONT_SIZE_PT`, where it is some 10.8 long,
/// and cuts it, as specified (R9.3); the property owed it whole, reading
/// `natural × min` as what it shrinks to. It owes a text whole now only
/// where it fits as the engine's own `size_factor` shrinks it. Found on
/// the compiled faces alone; replays alike on every host, and failed alike
/// at 289a684f.
///
/// Control: with the oracle's natural length scaled by `scale` without
/// the floor, red.
#[test]
fn test_flow_structured_finding_a_curve_fit_shrunk_to_the_size_floor() {
    let found = include_bytes!("fuzz_findings/structured_curve_fit_shrunk_to_the_size_floor");
    for (what, checked) in [
        (
            "fuzzed",
            crate::flow::tests::flow_fuzz::check_flow_structured(found),
        ),
        ("as found", check_flow_structured_as_found(found)),
    ] {
        if let Err(why) = checked {
            panic!("{what}: {why}");
        }
    }
}

/// The flow_structured_every_face finding of CI's run on #354 at
/// 03fd7d9a, after 1,068 runs: a `Spread` of 68 glyphs at 4,096 pt,
/// some 336,000 long, along a far-out line (#313) from (10¹⁵, 10¹⁵),
/// `0.8 × length` across and `0.6 × length` down. Floats there are some
/// 6.7 × 10⁷ apart, so its end rounded onto its start and the line has no
/// length; the engine laid nothing and reported it cut, as it should. The
/// property owed the run whole: its oracle, `curve_room`, found the
/// line's 256 sample points in `f32`, where `a·(1−t) + b·t` rounds to a
/// neighboring float of `a` at each, and summed 1.29 × 10¹⁰ of chords
/// that were rounding alone. The oracle finds them in `f64` now, and the
/// line measures what it is, nothing. The engine was not wrong, and no
/// budget is involved: it spent 273 steps on the line.
///
/// Control: with `curve_room` finding its points by `Curve::point` again,
/// red ("along a curve: the placement: its natural length fits, and it
/// was cut").
#[test]
fn test_flow_structured_every_face_finding_a_far_out_line_its_floats_cannot_part() {
    use crate::flow::tests::flow_fuzz::LAST_FAR_CURVE;
    let found = include_bytes!("fuzz_findings/structured_curve_far_out_line_its_floats_cannot_part");
    LAST_FAR_CURVE.with(|last| last.set(None));
    if let Err(why) = crate::flow::tests::flow_fuzz::check_flow_structured_every_face(found) {
        panic!("{why}");
    }
    let Some((curve, room)) = LAST_FAR_CURVE.with(|last| last.get()) else {
        panic!("the finding draws no far-out path");
    };
    assert_eq!(
        curve,
        crate::flow::Curve::Line(Vec2::splat(1e15), Vec2::splat(1e15)),
        "the finding draws a far-out line whose ends its floats cannot part"
    );
    assert_eq!(room, 0.0, "a line of no length gives a run no room");
}

/// The flow_structured finding of #308's fuzz run, failing alike at
/// d7242d38: a `Spread` frame laid inward round an ellipse 3.24 wide and
/// 1.18e-38 tall, its text of two glyphs one line, 0.45 long at its
/// font's gap along a half 3.24 long, with a 92-byte mixed ornament on
/// its sides. Its words' break was met at both junctions; one whole copy
/// of the ornament, some 60 glyphs, longer than the half, was not, and
/// the clearance was grown for the copy's room till it reached 28.3 at
/// each junction, the line pressed, and the pressed line cut to one
/// glyph: a frame lost its text to its sides, where it loses its sides
/// before its text (ruling R48.3, B15.5). Growth for a side copy's room
/// alone is held to the lines' slack now, and never takes a try that
/// presses a line; the arc no copy fits is left empty. Found on the
/// compiled faces alone (`FONTCONFIG_FILE` at an empty directory);
/// replays alike on every host.
///
/// Control: with a side copy's growth pressing a line as the words' does,
/// and no frame laid again without its sides where they cut it (either
/// holds the finding), red ("its natural length fits, and it was cut").
#[test]
fn test_flow_structured_finding_a_frame_of_one_line_round_an_outline_of_no_height() {
    let found = include_bytes!("fuzz_findings/structured_frame_one_line_round_an_outline_of_no_height");
    for (what, checked) in [
        (
            "fuzzed",
            crate::flow::tests::flow_fuzz::check_flow_structured(found),
        ),
        ("as found", check_flow_structured_as_found(found)),
    ] {
        if let Err(why) = checked {
            panic!("{what}: {why}");
        }
    }
}

/// R52.2: the oracle owes a `Fit` text whole exactly where it fits as the
/// engine shrinks it, no glyph below the size floor: "To see a World" at
/// 16 pt, `Fit` from a millionth, on a path just longer than the text
/// shaped at the floor, 0.5 pt, is owed whole, and on one just shorter is
/// not — though a millionth of its natural length fits both.
///
/// Control: with the oracle's natural length scaled by `scale` without
/// the floor, red.
#[test]
fn test_flow_the_whole_text_owed_a_fit_is_measured_at_the_size_floor() {
    use crate::document::values::FitMode;
    use crate::flow::tests::flow_fuzz::{natural_length_at, owes_whole};
    use crate::font::fonts::MIN_FONT_SIZE_PT;
    let text = "To see a World";
    let (run, floored) = with_fonts(|fs| {
        (
            real_run(text, &style(16.0), fs),
            real_run(text, &style(MIN_FONT_SIZE_PT), fs),
        )
    });
    // The text shaped at the floor, measured as the oracle measures a text.
    let at_floor = natural_length_at(&floored, 1.0);
    let fit = FitMode::Fit { min: 1e-6, max: 1.0 };
    assert!(
        natural_length_at(&run, 1.0) * 1e-6 < at_floor * 0.99,
        "precondition: a millionth of the text's natural length fits a path shorter than it at the floor"
    );
    assert!(
        owes_whole(&run, &fit, at_floor * 1.01),
        "a path 1% longer than the text at the floor, {at_floor}, does not owe it whole"
    );
    assert!(
        !owes_whole(&run, &fit, at_floor * 0.99),
        "a path 1% shorter than the text at the floor, {at_floor}, owes it whole"
    );
}

/// R50.3, R45.1: a long fuzzed side ornament of the glyphs the compiled
/// faces hold fits the per-input work bound beside a text cut to
/// nothing, on every host: in every face a span can name, upright and
/// bold italic, each glyph of [`ORNAMENT_GLYPHS`] but the last (U+FFFE,
/// which no face holds) and all of them in turn, repeated to
/// [`LONG_ORNAMENT_BYTES`], shape with no `.notdef`, and a frame of one
/// glyph beside such an ornament, laid to the whole of its budget and its
/// overshoot, takes no more than [`INPUT_STEPS`] — so the harness need
/// not cut it ([`sides_in_time`]).
///
/// Control: with `𓀀` replaced by `⸻`, which no compiled face holds, red
/// where no host face holds it either (on CI, and here with
/// `FONTCONFIG_FILE` naming a config of one empty font directory, which
/// leaves the compiled faces alone).
///
/// [`ORNAMENT_GLYPHS`]: crate::flow::tests::flow_fuzz::ORNAMENT_GLYPHS
/// [`LONG_ORNAMENT_BYTES`]: crate::flow::tests::flow_fuzz::LONG_ORNAMENT_BYTES
/// [`INPUT_STEPS`]: crate::flow::tests::flow_fuzz::INPUT_STEPS
/// [`sides_in_time`]: crate::flow::tests::flow_fuzz::sides_in_time
#[test]
fn test_a_long_fuzzed_ornament_fits_the_per_input_bound() {
    use crate::flow::place::halves::{build_steps, frame_work_steps};
    use crate::flow::tests::flow_fuzz::{faces, INPUT_STEPS, LONG_ORNAMENT_BYTES, ORNAMENT_GLYPHS};
    let repeated = |glyphs: &[char]| {
        let mut ornament = String::new();
        for glyph in glyphs.iter().cycle() {
            if ornament.len() + glyph.len_utf8() > LONG_ORNAMENT_BYTES {
                break;
            }
            ornament.push(*glyph);
        }
        ornament
    };
    let held = &ORNAMENT_GLYPHS[..ORNAMENT_GLYPHS.len() - 1];
    let mut ornaments: Vec<String> = held.iter().map(|g| repeated(&[*g])).collect();
    ornaments.push(repeated(held));
    let mut heaviest = (0, String::new(), 0);
    for face in faces() {
        for (weight, italic) in [
            (Weight::NORMAL, false),
            (Weight::new(700).unwrap_or(Weight::NORMAL), true),
        ] {
            let style = ResolvedText {
                font: face,
                size: FontSize::Pt(16.0),
                color: Rgba::new(255, 255, 255, 255),
                weight,
                italic,
            };
            for ornament in &ornaments {
                let (notdefs, work, alone) = with_fonts(|fs| {
                    let shaped = shape_run(
                        &[FlowSpan {
                            text: ornament,
                            style: &style,
                            link: None,
                        }],
                        1.0,
                        fs,
                    );
                    let notdefs = shaped.layout_glyphs().filter(|g| g.glyph_id == 0).count();
                    let text = shape_run(
                        &[FlowSpan {
                            text: "a",
                            style: &style,
                            link: None,
                        }],
                        1.0,
                        fs,
                    );
                    (
                        notdefs,
                        frame_work_steps(&text, Some(&shaped), true, 0),
                        build_steps(&text, Some(&shaped)) - build_steps(&text, None),
                    )
                });
                assert_eq!(
                    notdefs, 0,
                    "{face:?}: {ornament:?} shapes {notdefs} glyphs no face holds"
                );
                assert!(
                    work <= INPUT_STEPS,
                    "{face:?}: a one-glyph frame beside {ornament:?} takes {work} glyph steps, past the {INPUT_STEPS} an input may take"
                );
                if work > heaviest.0 {
                    heaviest = (work, format!("{face:?} {ornament:?}"), alone);
                }
            }
        }
    }
    assert!(
        heaviest.0 > 400_000,
        "precondition: the frames are laid at the floor of their budget ({heaviest:?})"
    );
}

/// `n` distinct noncharacters, code points no face holds on any host: the
/// 32 of U+FDD0 to U+FDEF, then the last two of each plane, U+FFFE and
/// U+FFFF first. Each is a pass of the shaper through every face, its
/// first time (`halves::SHAPE_NOTDEF_STEPS`, #304), where one code point
/// repeated is one pass however often.
fn noncharacters(n: usize) -> String {
    (0xFDD0..=0xFDEF_u32)
        .chain((0..17).flat_map(|plane: u32| [plane * 0x1_0000 + 0xFFFE, plane * 0x1_0000 + 0xFFFF]))
        .filter_map(char::from_u32)
        .take(n)
        .collect()
}

/// R50.3, R45.1: a long side ornament no frame could lay inside the
/// timeout is cut until a frame of no text beside it fits, on every host:
/// fifty distinct noncharacters, code points no face holds, each a pass of
/// the shaper through every face (`SHAPE_NOTDEF_STEPS`) each time the
/// ornament is shaped, take a frame of no text to 484,150 glyph steps,
/// past the 464,396 one input may take; [`sides_in_time`] cuts them to
/// what fits, 25 of them, 403,130. (Fifty U+FFFE, one code point, is one
/// pass however long, and fits uncut since #304, 408,098; until then each
/// was a pass, 624,221.)
///
/// Control: with the ornament left uncut, red.
///
/// [`sides_in_time`]: crate::flow::tests::flow_fuzz::sides_in_time
#[test]
fn test_a_long_ornament_no_face_holds_is_cut_to_fit() {
    use crate::flow::place::halves::frame_work_steps;
    use crate::flow::tests::flow_fuzz::{frame_steps, sides_in_time};
    let flow = style(16.0);
    let given = noncharacters(50);
    assert_eq!(
        given.chars().count(),
        50,
        "precondition: fifty distinct noncharacters"
    );
    let layout = RingLayout {
        sides: Some((given.clone(), Rgba::new(255, 255, 255, 255))),
        ..RingLayout::whole()
    };
    let (work, cut) = with_fonts(|fs| {
        let empty = shape_run(
            &[FlowSpan {
                text: "",
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        );
        let (cut, ornament) = sides_in_time(&layout, &flow, 1.0, 1, &|_| 0, fs);
        (frame_work_steps(&empty, ornament.as_ref(), true, 0), cut)
    });
    let kept = cut.sides.as_ref().map_or(0, |(s, _)| s.chars().count());
    assert!(
        work <= frame_steps(1) && kept > 0 && kept < given.chars().count(),
        "fifty glyphs no face holds cut to {kept}, a frame of no text beside them taking {work} glyph steps against the {} one frame may take",
        frame_steps(1)
    );
}

/// CI's flow_structured finding at d5ae4735 (run 41583576270): a frame
/// whose long side ornament — `"﷽=·W"` repeated, when the long-ornament
/// mode drew `﷽`, which no compiled face holds — was `.notdef`s on the
/// runner, so that even its text cut to nothing left its frame 465,977
/// glyph steps, past the 464,396 one input may take. Here the same bytes
/// draw U+FFFE in its place, which no face holds on any host, and the
/// harness cuts the ornament until a frame fits ([`sides_in_time`],
/// ruling R50.3).
///
/// Control: with the ornament left uncut, red.
///
/// [`sides_in_time`]: crate::flow::tests::flow_fuzz::sides_in_time
#[test]
fn test_flow_structured_finding_a_long_ornament_past_the_input_bound() {
    let found = include_bytes!("fuzz_findings/structured_frame_long_ornament_past_the_input_bound");
    if let Err(why) = crate::flow::tests::flow_fuzz::check_flow_structured(found) {
        panic!("{why}");
    }
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("as found: {why}");
    }
}

/// R44.2's property: a frame two of whose inks touch fails, whatever its
/// budget did — never overlap wins over never cut, and a frame that
/// cannot hold its inks apart cuts them and says so.
///
/// Control: with the property never failing, red.
#[test]
fn test_the_flow_properties_find_a_frame_whose_inks_touch() {
    use crate::flow::place::Trace;
    use crate::flow::tests::flow_fuzz::apart;
    let mut trace = Trace::default();
    assert!(
        apart(&trace, "a frame apart").is_ok(),
        "precondition: a frame apart holds"
    );
    trace.unheld.overlap = true;
    assert!(
        apart(&trace, "a frame touching").is_err(),
        "a frame two of whose inks touch fails"
    );
}

/// R44.2, R46.3: a fuzzed frame's text is held to what its frame lays
/// inside the timeout ([`fits_in_time`]): a long text of one face is laid
/// from its first [`MAX_FRAME_GLYPHS`] glyphs or fewer, a short one
/// whole, and so is a text of sixty distinct glyphs no face holds. Its
/// first shaping searches every face for each, but that is before its
/// frame: the frame only reshapes it at a size, which shapes nothing
/// (#304, `halves::resizing_steps`), so it is no heavier to lay than
/// sixty letters. (Until #304 a frame counted each a pass again at every
/// reshape, and cut it to what its work fit.)
///
/// Controls: with no text cut, the long text's 2,000 glyphs, red; with a
/// size-only reshape counted as a shaping, the heavy text cut.
///
/// [`MAX_FRAME_GLYPHS`]: crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS
/// [`fits_in_time`]: crate::flow::tests::flow_fuzz::fits_in_time
#[test]
fn test_a_fuzzed_frame_is_held_to_what_one_frame_lays_in_the_timeout() {
    use crate::flow::place::halves::frame_work_steps;
    use crate::flow::tests::flow_fuzz::{fits_in_time, frame_steps, framed, MAX_FRAME_GLYPHS};
    let flow = style(16.0);
    let fits = |run: &crate::flow::ShapedRun| fits_in_time(run, None, 1, 0);
    // Sixty noncharacters, distinct code points no face holds (`.notdef`).
    let heavy_text = noncharacters(60);
    assert_eq!(
        heavy_text.chars().count(),
        60,
        "precondition: sixty distinct noncharacters"
    );
    let (long, short, heavy) = with_fonts(|fs| {
        let cut = |text: String, fs: &mut crate::font::fonts::FontSystemLease| {
            let kept = framed(&[text], std::slice::from_ref(&flow), 1.0, fs, &fits);
            let run = real_run(&kept[0], &flow, fs);
            (run.glyphs.len(), frame_work_steps(&run, None, true, 0), kept)
        };
        (
            cut("word ".repeat(400), fs),
            cut("word ".repeat(20), fs),
            cut(heavy_text.clone(), fs),
        )
    });
    assert!(
        long.0 <= MAX_FRAME_GLYPHS && long.0 > MAX_FRAME_GLYPHS / 4,
        "the long text is laid from its first glyphs: {}",
        long.0
    );
    assert_eq!(short.2, ["word ".repeat(20)], "a short text is laid whole");
    assert!(
        heavy.2[0] == heavy_text && heavy.1 <= frame_steps(1),
        "a text of sixty glyphs no face holds is laid whole: {} of 60 characters, {} glyph steps of {}",
        heavy.2[0].chars().count(),
        heavy.1,
        frame_steps(1)
    );
}

/// Ruling B16.3a: the harnesses' bounds hold the tables a frame's or a
/// ring's setup builds, which they count before it is laid
/// (`setup_table_samples`), up to the largest, [`MAX_TABLE_SAMPLES`] + 1
/// samples a path.
///
/// A ring's copy is a frame's text cut to fit beside its path's table as
/// a frame's ([`framed`], [`fits_in_time`]), so at every size of table
/// its cap ([`ring_cap`]) holds the longest copy that cut leaves — a text
/// of the cheapest letters and no sides — and the ring of that cap stays
/// within [`INPUT_STEPS`], the most glyphs that do; with no table the cap
/// is [`MAX_RING_GLYPHS`]. And a frame's two halves of the largest table
/// each, beside a text of no glyphs and its sides cut to fit
/// (`sides_in_time`), fit one frame's share of [`INPUT_STEPS`], so the
/// text cut after always finds a frame that fits.
///
/// Controls (run, not committed): the cap not lowered by the table (as
/// before #309), red on the ring's steps beside the largest table; the
/// cap found as if the table were twice its samples, red on the copy the
/// cut leaves; `TABLE_SAMPLES_A_STEP` at 16, red on both: beside the
/// largest table no copy fits a frame, nor do the halves.
///
/// [`MAX_TABLE_SAMPLES`]: crate::flow::MAX_TABLE_SAMPLES
/// [`INPUT_STEPS`]: crate::flow::tests::flow_fuzz::INPUT_STEPS
/// [`framed`]: crate::flow::tests::flow_fuzz::framed
/// [`fits_in_time`]: crate::flow::tests::flow_fuzz::fits_in_time
/// [`ring_cap`]: crate::flow::tests::flow_fuzz::ring_cap
/// [`MAX_RING_GLYPHS`]: crate::flow::tests::flow_fuzz::MAX_RING_GLYPHS
#[test]
fn test_a_rings_cap_holds_the_copy_its_tables_leave() {
    use crate::flow::place::halves::frame_work_steps;
    use crate::flow::place::ring::ring_work_steps;
    use crate::flow::tests::flow_fuzz::{
        fits_in_time, frame_steps, ring_cap, sides_in_time, INPUT_STEPS, MAX_FRAME_GLYPHS, MAX_RING_GLYPHS,
    };
    use crate::flow::MAX_TABLE_SAMPLES;
    let table = MAX_TABLE_SAMPLES + 1;
    assert_eq!(
        ring_cap(MAX_FRAME_GLYPHS, true, 0),
        MAX_RING_GLYPHS,
        "with no table, the cap is the most that fits"
    );
    let flow = style(16.0);
    // The cheapest copy of each length the cut may leave: one face, no
    // `.notdef`, no sides.
    let runs: Vec<_> = with_fonts(|fs| {
        (0..=MAX_FRAME_GLYPHS)
            .map(|n| real_run(&"a".repeat(n), &flow, fs))
            .collect()
    });
    let mut broken = Vec::new();
    for samples in [0, 1 << 10, 1 << 14, 1 << 17, 1 << 19, table] {
        let Some(copy) = runs.iter().rposition(|run| fits_in_time(run, None, 1, samples)) else {
            broken.push(format!(
                "beside a table of {samples} samples, no copy fits a frame"
            ));
            continue;
        };
        let cap = ring_cap(copy, true, samples);
        let ring = ring_work_steps(cap, MAX_FRAME_GLYPHS, samples);
        eprintln!("table {samples}: the cut leaves {copy} glyphs, the cap {cap}, {ring} glyph steps of {INPUT_STEPS}");
        if cap < copy {
            broken.push(format!(
                "beside a table of {samples} samples, the cut leaves a copy of {copy} glyphs, past the cap of {cap}"
            ));
        }
        if ring > INPUT_STEPS {
            broken.push(format!(
                "beside a table of {samples} samples, a ring of the cap of {cap} takes {ring} glyph steps, past {INPUT_STEPS}"
            ));
        }
        if cap < MAX_RING_GLYPHS && ring_work_steps(cap + 1, MAX_FRAME_GLYPHS, samples) <= INPUT_STEPS {
            broken.push(format!(
                "beside a table of {samples} samples, a cap of {} glyphs fits too: {cap} is not the most",
                cap + 1
            ));
        }
    }
    let layout = RingLayout {
        sides: Some(("─═".repeat(8), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let (fits, steps, kept) = with_fonts(|fs| {
        let empty = real_run("", &flow, fs);
        let (cut, ornament) = sides_in_time(&layout, &flow, 1.0, 1, &|_| 2 * table, fs);
        (
            fits_in_time(&empty, ornament.as_ref(), 1, 2 * table),
            frame_work_steps(&empty, ornament.as_ref(), true, 2 * table),
            cut.sides.is_some(),
        )
    });
    if !(fits && kept) {
        broken.push(format!(
            "a frame of no text beside halves of {table} samples each takes {steps} glyph steps of the {} one frame may take, its sides kept: {kept}",
            frame_steps(1)
        ));
    }
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

/// R45.2, R46.3: each flow harness's worst input fits in
/// [`INPUT_WORK_MICROS`], three quarters of the `-timeout` that
/// `scripts/fuzz.sh` gives every input: its frames — [`RAW_FRAMES`] or
/// [`STRUCTURED_FRAMES`] — each laid to its whole budget and past it by
/// the most the weights allow (`frame_work_steps`), at
/// [`ASAN_STEP_NANOS`] a counted step. The cut ([`fits_in_time`]) holds
/// each frame to its share of [`INPUT_STEPS`]; what is computed here is
/// that a frame at the floor of its budget fits that share at all — no
/// cut can make a frame lighter than its budget's floor — and that
/// [`MAX_FRAME_GLYPHS`] is the most glyphs of the lightest text, one
/// face, with a short side ornament, that fit: 152, 463,355 glyph steps,
/// 14.97 s; 153 take 466,434, 15.07 s (150 and 151 until #304 counted a
/// reshape at a size as no shaping).
///
/// A `Repeat` ring's work (#310) fits an input's as its one budgeted
/// placement, and not beside a frame; and [`MAX_RING_GLYPHS`] is the most
/// glyphs whose ring, its budget scaled by them and its overshoot past it
/// for a copy of [`MAX_FRAME_GLYPHS`] glyphs, fits (rulings B17.2a,
/// B17.2b): 154, 462,479 glyph steps, 14.94 s; 155 take 465,479, 15.03 s
/// — and no fewer than a copy's, so one copy always fits.
///
/// Control: flow_raw's three frames an input, as before R45, red — three
/// floor frames take 1,206,402 steps, 38.97 s.
///
/// [`RAW_FRAMES`]: crate::flow::tests::flow_fuzz::RAW_FRAMES
/// [`STRUCTURED_FRAMES`]: crate::flow::tests::flow_fuzz::STRUCTURED_FRAMES
/// [`MAX_FRAME_GLYPHS`]: crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS
/// [`MAX_RING_GLYPHS`]: crate::flow::tests::flow_fuzz::MAX_RING_GLYPHS
/// [`ASAN_STEP_NANOS`]: crate::flow::tests::flow_fuzz::ASAN_STEP_NANOS
/// [`INPUT_WORK_MICROS`]: crate::flow::tests::flow_fuzz::INPUT_WORK_MICROS
/// [`INPUT_STEPS`]: crate::flow::tests::flow_fuzz::INPUT_STEPS
/// [`fits_in_time`]: crate::flow::tests::flow_fuzz::fits_in_time
#[test]
fn test_each_flow_harness_lays_an_input_in_three_quarters_of_the_timeout() {
    use crate::flow::place::halves::frame_work_steps;
    use crate::flow::tests::flow_fuzz::{
        frame_steps, ASAN_STEP_NANOS, FUZZ_TIMEOUT_SECONDS, INPUT_STEPS, INPUT_WORK_MICROS, MAX_FRAME_GLYPHS,
        RAW_FRAMES, STRUCTURED_FRAMES,
    };
    use crate::util::doc_fixtures::repo_path;
    let script = std::fs::read_to_string(repo_path("scripts/fuzz.sh")).unwrap();
    assert!(
        script.contains(&format!("  -timeout={FUZZ_TIMEOUT_SECONDS} \\\n")),
        "scripts/fuzz.sh runs every target with -timeout={FUZZ_TIMEOUT_SECONDS}"
    );
    assert_eq!(
        INPUT_WORK_MICROS, 15_000_000,
        "an input's frames may take 15 s, three quarters of the timeout"
    );
    let seconds = |steps: usize| steps as f64 * ASAN_STEP_NANOS as f64 / 1e9;
    assert!(
        INPUT_STEPS as u64 * ASAN_STEP_NANOS <= INPUT_WORK_MICROS * 1_000,
        "{INPUT_STEPS} glyph steps take {:.2} s",
        seconds(INPUT_STEPS)
    );
    let flow = style(16.0);
    let (floor, light, past) = with_fonts(|fs| {
        let ornament = real_run("─═─═", &flow, fs);
        let empty = real_run("", &flow, fs);
        let at = |glyphs: usize, fs: &mut crate::font::fonts::FontSystemLease| {
            frame_work_steps(
                &real_run(&"a".repeat(glyphs), &flow, fs),
                Some(&ornament),
                true,
                0,
            )
        };
        (
            frame_work_steps(&empty, Some(&ornament), true, 0),
            at(MAX_FRAME_GLYPHS, fs),
            at(MAX_FRAME_GLYPHS + 1, fs),
        )
    });
    for (target, frames) in [("flow_raw", RAW_FRAMES), ("flow_structured", STRUCTURED_FRAMES)] {
        assert!(
            frames * floor <= INPUT_STEPS,
            "{target}: {frames} frames at the floor of their budget take {} glyph steps, {:.2} s, past the {:.2} s an input may take",
            frames * floor,
            seconds(frames * floor),
            seconds(INPUT_STEPS)
        );
    }
    assert!(
        light <= frame_steps(1) && past > frame_steps(1),
        "{MAX_FRAME_GLYPHS} glyphs of the lightest text take {light} glyph steps, {:.2} s, and one more {past}, {:.2} s: the most that fit {} is the cap",
        seconds(light),
        seconds(past),
        frame_steps(1)
    );
    // A `Repeat` ring is budgeted as a frame is (#310), by the glyphs it
    // will lay at most its cap (rulings B17.1a, B17.2a): laid as an
    // input's one budgeted placement at the harnesses' cap, with any copy a
    // frame's cut leaves it, it fits, and at one glyph more it does not;
    // and laid beside a frame, as flow_raw laid it until then, the two do
    // not.
    use crate::flow::place::ring::ring_work_steps;
    use crate::flow::tests::flow_fuzz::MAX_RING_GLYPHS;
    let (ring, more) = (
        ring_work_steps(MAX_RING_GLYPHS, MAX_FRAME_GLYPHS, 0),
        ring_work_steps(MAX_RING_GLYPHS + 1, MAX_FRAME_GLYPHS, 0),
    );
    assert!(
        ring <= frame_steps(1) && more > frame_steps(1) && MAX_RING_GLYPHS >= MAX_FRAME_GLYPHS,
        "a ring of {MAX_RING_GLYPHS} glyphs, {MAX_FRAME_GLYPHS} a copy, takes {ring} glyph steps, {:.2} s, and one more {more}, {:.2} s, against the {} an input may take: the most that fit, and a copy's glyphs at least, is the cap",
        seconds(ring),
        seconds(more),
        frame_steps(1)
    );
    let beside = floor + ring_work_steps(0, 0, 0);
    assert!(
        beside > INPUT_STEPS,
        "a ring beside a frame takes {beside} glyph steps, {:.2} s, within an input's {:.2} s: lay it besides",
        seconds(beside),
        seconds(INPUT_STEPS)
    );
}

/// R45.1: flow_raw draws its frame's fit mode from its input, so every
/// mode a frame tracks its halves in is laid by some input, and so is a
/// `Repeat` ring, budgeted as a frame is (#310) and drawn in place of
/// one; and an input whose choices run out before it — every committed
/// raw finding — draws `Fit`, the mode they were found in.
///
/// Controls: with the mode fixed at `Fit`, red; with the ring left out of
/// the draw, red.
#[test]
fn test_the_raw_harness_draws_every_frame_fit() {
    use crate::document::values::FitMode;
    use crate::flow::tests::flow_fuzz::{raw_frame_fits, RAW_FRAMES};
    assert!(
        matches!(raw_frame_fits(&[])[..], [FitMode::Fit { .. }, ..]),
        "an input with no choices left draws Fit first"
    );
    let mut drawn = [false; 4];
    for byte in 0..=u8::MAX {
        let fits = raw_frame_fits(&[byte; 64]);
        assert_eq!(fits.len(), RAW_FRAMES, "{RAW_FRAMES} frames an input");
        for fit in fits {
            drawn[match fit {
                FitMode::Fit { .. } => 0,
                FitMode::Spread => 1,
                FitMode::Once { .. } => 2,
                FitMode::Repeat => 3,
            }] = true;
        }
    }
    assert_eq!(
        drawn, [true; 4],
        "Fit, Spread, Once and a Repeat ring are each drawn"
    );
    for name in [
        "fit_frame_pressed_by_a_clearance_its_other_line_could_give",
        "fit_frame_shrunk_by_whole_clearance_searches",
        "raw_frame_cut_apart_by_whole_clearance_searches",
        "raw_frame_cut_apart_round_a_tight_bend",
    ] {
        let found = std::fs::read(
            crate::util::doc_fixtures::repo_path("lib/baumhard/src/flow/tests/fuzz_findings").join(name),
        )
        .unwrap();
        assert!(
            matches!(raw_frame_fits(&found)[..], [FitMode::Fit { .. }, ..]),
            "the finding {name} draws a Fit frame first"
        );
    }
}

/// The flow_raw finding of the R48 fuzz run: NULs, a DEL and a line
/// break, whose control characters' glyphs are given inks zero wide and
/// line tall, laid a third of a point apart round the rectangle. Each was
/// once weighed against every other in the touch check's strips: 4,315
/// looks over 95 boxes, past `TOUCH_LOOKS_A_BOX`, then 12 a box (now
/// `touch_looks`, ruling R51.2). A box with
/// no ink is left out of the check now (ruling R49.2), and the replay —
/// held to the bound by the fuzz property — passes. The strips by class
/// of height hold it even with every box taken as inked (ruling R50.1):
/// a line-tall box no wider than nothing is weighed against its own
/// class's few neighbors, 383 looks over the 95 boxes, 4.03 a box.
///
/// Control: with every box taken as inked and in strips of the tallest's
/// class, as when it was found, red.
#[test]
fn test_flow_raw_finding_a_touch_check_past_its_looks_a_box() {
    let found = include_bytes!("fuzz_findings/raw_frame_touch_check_past_its_looks_a_box");
    if let Err(why) = check_flow_bytes_in_every_fit(found) {
        panic!("{why}");
    }
}

/// The R45 check, as review 5739829769 found it broken: every committed
/// raw finding was found before ruling R44.4, laid over whole halves with
/// no sides, but its bytes are text, not zeros, and draw another layout
/// now. A replay lays it in [`RingLayout::whole`] as well as the layout
/// it draws; the fuzzers lay the layout drawn alone.
///
/// Control: with a replay laid in the drawn layout alone, red.
#[test]
fn test_a_committed_raw_finding_replays_in_the_layout_it_was_found_in() {
    use crate::flow::tests::flow_fuzz::{laid_layouts, ring_layout};
    use crate::util::tests::choices::Choices;
    for name in [
        "fit_frame_pressed_by_a_clearance_its_other_line_could_give",
        "fit_frame_shrunk_by_whole_clearance_searches",
        "raw_frame_cut_apart_by_whole_clearance_searches",
        "raw_frame_cut_apart_round_a_tight_bend",
    ] {
        let found = std::fs::read(
            crate::util::doc_fixtures::repo_path("lib/baumhard/src/flow/tests/fuzz_findings").join(name),
        )
        .unwrap();
        let drawn = ring_layout(&mut Choices::new(&found));
        assert!(
            drawn != RingLayout::whole(),
            "precondition: the finding {name} draws a layout of its own ({drawn:?})"
        );
        let replayed = laid_layouts(drawn.clone(), false);
        assert!(
            replayed.contains(&RingLayout::whole()) && replayed.contains(&drawn),
            "the finding {name} replays in {replayed:?}: in whole halves, as found, and as drawn"
        );
        assert_eq!(
            laid_layouts(drawn.clone(), true),
            [drawn],
            "the fuzzer lays {name}'s drawn layout alone"
        );
    }
}

/// R47.2's property: a frame that takes more than its budget and the
/// overshoot its weights allow (`frame_work_steps`) fails, as does one
/// the engine budgets more than its glyphs give; and a frame whose setup
/// counted tables of other samples than `setup_table_samples` said it
/// builds (ruling B16.3a), more or fewer. A frame's tables are within
/// what it may take: at its bound with them it holds, at one step past
/// it it fails.
///
/// Control: with the property never failing, red; with the tables'
/// property never failing, red.
#[test]
fn test_the_flow_properties_find_a_frame_past_its_work() {
    use crate::flow::place::halves::{frame_work_steps, TABLE_SAMPLES_A_STEP};
    use crate::flow::place::Trace;
    use crate::flow::tests::flow_fuzz::InputWork;
    let flow = style(16.0);
    let run = with_fonts(|fs| real_run("plain text", &flow, fs));
    let tables = 1_000 * TABLE_SAMPLES_A_STEP;
    for samples in [0, tables] {
        let most = frame_work_steps(&run, None, false, samples);
        let mut trace = Trace {
            spent: most,
            tables: samples,
            ..Trace::default()
        };
        assert!(
            InputWork::default()
                .frame(&run, None, false, samples, &trace, "a frame")
                .is_ok(),
            "precondition: a frame that takes its budget and its overshoot holds, its tables of {samples} samples among them"
        );
        trace.spent = most + 1;
        assert!(
            InputWork::default()
                .frame(&run, None, false, samples, &trace, "a frame")
                .is_err(),
            "a frame past its budget and its overshoot fails, its tables of {samples} samples among them"
        );
    }
    assert!(
        frame_work_steps(&run, None, false, tables) == frame_work_steps(&run, None, false, 0) + 1_000,
        "a frame's tables are what it may take past its budget, at their samples a step"
    );
    for counted in [tables - 1, tables + 1] {
        let trace = Trace {
            spent: 0,
            tables: counted,
            ..Trace::default()
        };
        assert!(
            InputWork::default()
                .frame(&run, None, false, tables, &trace, "a frame")
                .is_err(),
            "a frame whose setup counted {counted} table samples, not the {tables} its outline gives, fails"
        );
    }
}

/// #310's property (rulings B17.1a, B17.2b): a `Repeat` ring that takes
/// more than its cap's budget and the overshoot past it
/// (`ring_work_steps`) fails, as does one the engine budgets past its cap,
/// one that places more glyphs than its cap, one cut past its first copy
/// that reports nothing — capped silently — and one whose setup counted
/// a table of other samples than `setup_table_samples` said it builds
/// (ruling B16.3a). A ring cut before one copy is the path's cut, owed no
/// report.
///
/// Control: with each of the five never failing, red (that one).
#[test]
fn test_the_flow_properties_find_a_ring_past_its_work() {
    use crate::flow::place::halves::frame_budget;
    use crate::flow::place::ring::{ring_work_steps, RingLoss};
    use crate::flow::place::Trace;
    use crate::flow::tests::flow_fuzz::{InputWork, MAX_RING_GLYPHS};
    let flow = style(16.0);
    let run = with_fonts(|fs| real_run("· ", &flow, fs));
    let cap = MAX_RING_GLYPHS;
    let (budget, most) = (frame_budget(cap), ring_work_steps(cap, run.glyphs.len(), 0));
    let glyph = PlacedGlyph {
        glyph: 0,
        pos: Vec2::ZERO,
    };
    let laid = |glyphs: usize, truncated: bool| PlacedRun {
        glyphs: vec![glyph; glyphs],
        truncated,
    };
    let whole = laid(cap, false);
    let mut trace = Trace {
        spent: most,
        budget,
        glyphs: cap,
        ..Trace::default()
    };
    let ring =
        |placed: &PlacedRun, trace: &Trace| InputWork::default().ring(&run, cap, 0, placed, trace, "a ring");
    assert!(
        ring(&whole, &trace).is_ok(),
        "precondition: a ring that takes its budget and its overshoot holds"
    );
    trace.spent = most + 1;
    assert!(
        ring(&whole, &trace).is_err(),
        "a ring past its budget and its overshoot fails"
    );
    trace.spent = most;
    trace.budget = budget + 1;
    assert!(
        ring(&whole, &trace).is_err(),
        "a ring budgeted past its cap fails"
    );
    trace.budget = budget;
    trace.glyphs = cap + 1;
    assert!(
        ring(&whole, &trace).is_err(),
        "a ring scaled by more glyphs than its cap fails"
    );
    trace.glyphs = cap;
    assert!(
        ring(&laid(cap + 1, false), &trace).is_err(),
        "a ring placing more glyphs than its cap fails"
    );
    let capped = laid(cap - cap % run.glyphs.len(), true);
    assert!(
        ring(&capped, &trace).is_err(),
        "a ring cut past its first copy, reporting nothing, fails"
    );
    trace.ring = Some(RingLoss::Short);
    assert!(
        ring(&capped, &trace).is_ok(),
        "precondition: a ring cut and reported holds"
    );
    trace.ring = None;
    assert!(
        ring(&laid(run.glyphs.len() - 1, true), &trace).is_ok(),
        "precondition: a ring the path cut before one copy, reporting nothing, holds"
    );
    // Its path's table (ruling B16.3a): within what it may take, and
    // counted as `setup_table_samples` says it is built.
    use crate::flow::place::halves::TABLE_SAMPLES_A_STEP;
    let tables = 1_000 * TABLE_SAMPLES_A_STEP;
    let tabled = ring_work_steps(cap, run.glyphs.len(), tables);
    assert!(
        tabled == most + 1_000,
        "a ring's table is what it may take past its budget, at its samples a step"
    );
    let mut trace = Trace {
        spent: tabled,
        budget,
        glyphs: cap,
        tables,
        ..Trace::default()
    };
    let tabled_ring = |trace: &Trace| InputWork::default().ring(&run, cap, tables, &whole, trace, "a ring");
    assert!(
        tabled_ring(&trace).is_ok(),
        "precondition: a ring that takes its budget and its overshoot, its table among them, holds"
    );
    for counted in [tables - 1, tables + 1] {
        trace.tables = counted;
        assert!(
            tabled_ring(&trace).is_err(),
            "a ring whose setup counted {counted} table samples, not the {tables} its outline gives, fails"
        );
    }
}

/// #313: flow_raw lays one budgeted placement an input (ruling R45.1):
/// one time in four where it draws no ring, one placement along its line
/// in place of its frame — each of [`raw_lines`] somewhere — and an input
/// whose choices run out before the draw, every committed raw finding,
/// none, laying its frame as it did.
///
/// Controls: with the line's placements left out of the draw, red; with
/// one drawn on no choice, red.
///
/// [`raw_lines`]: crate::flow::tests::flow_fuzz::raw_lines
#[test]
fn test_the_raw_harness_draws_every_line_placement_in_place_of_its_frame() {
    use crate::flow::tests::flow_fuzz::{raw_line, raw_lines};
    assert!(
        raw_line(&[]).is_none(),
        "an input with no choices left draws no placement along its line"
    );
    let lines = raw_lines();
    let mut drawn = vec![0usize; lines.len()];
    let mut frames = 0;
    // Inputs of 64 bytes from a xorshift, each seed its own.
    for seed in 1..=512_u64 {
        let mut state = seed;
        let bytes: Vec<u8> = (0..64)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        match raw_line(&bytes) {
            Some(line) => {
                let k = lines
                    .iter()
                    .position(|l| *l == line)
                    .expect("a line placement of the list");
                drawn[k] += 1;
            }
            None => frames += 1,
        }
    }
    assert!(
        drawn.iter().all(|&n| n > 0) && frames > 0,
        "each placement along the line is drawn ({drawn:?}), and frames besides ({frames})"
    );
    for name in [
        "fit_frame_pressed_by_a_clearance_its_other_line_could_give",
        "fit_frame_shrunk_by_whole_clearance_searches",
        "raw_frame_cut_apart_by_whole_clearance_searches",
        "raw_frame_cut_apart_round_a_tight_bend",
    ] {
        let found = std::fs::read(
            crate::util::doc_fixtures::repo_path("lib/baumhard/src/flow/tests/fuzz_findings").join(name),
        )
        .unwrap();
        assert!(
            raw_line(&found).is_none(),
            "the finding {name} draws its frame, not a placement along its line"
        );
    }
}

/// #313: a placement along an open path, and a whole-glyph one, are each
/// an input's one budgeted placement, as a `Repeat` ring is (ruling
/// B17.2b): laid at the harnesses' cap they fit [`INPUT_STEPS`], the
/// cap the most glyphs that do — whatever the run's length, a curve's
/// table at its most among them — and laid beside a frame at the floor of
/// its budget, as flow_raw laid its line until then, neither does. So an
/// input whose frame and line placement may take more than one input may
/// inside the timeout fails.
///
/// Control: with an input's frame and its line's placement counted apart,
/// as two inputs, red.
///
/// [`INPUT_STEPS`]: crate::flow::tests::flow_fuzz::INPUT_STEPS
#[test]
fn test_an_open_placement_is_an_input_s_one_budgeted_placement() {
    use crate::document::values::FitMode;
    use crate::flow::place::halves::frame_work_steps;
    use crate::flow::place::ring::ring_work_steps;
    use crate::flow::tests::flow_fuzz::{
        check_raw, open_cap, open_tables, pitched_cap, pitched_work_steps, RawLine, INPUT_STEPS,
        MAX_RING_GLYPHS,
    };
    use crate::flow::Curve;
    let cubic = [Curve::Cubic(Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE)];
    let tables = [0, open_tables(&cubic)];
    for copy in [1, 256, 4_096] {
        for tables in tables {
            let cap = open_cap(copy, tables, true).expect("a cap holds a placement along an open path");
            let (at, more) = (
                ring_work_steps(cap, copy, tables),
                ring_work_steps(cap + 1, copy, tables),
            );
            assert!(
                at <= INPUT_STEPS && (cap == MAX_RING_GLYPHS || more > INPUT_STEPS) && cap >= 100,
                "a placement of {copy} glyphs, tables of {tables} samples, at its cap of {cap} takes {at} glyph steps and one more {more}, against the {INPUT_STEPS} an input may take"
            );
        }
    }
    for tables in tables {
        let cap = pitched_cap(tables, true).expect("a cap holds a whole-glyph placement");
        let (at, more) = (
            pitched_work_steps(cap, tables),
            pitched_work_steps(cap + 1, tables),
        );
        assert!(
            at <= INPUT_STEPS && (cap == MAX_RING_GLYPHS || more > INPUT_STEPS) && cap >= 100,
            "a whole-glyph placement, tables of {tables} samples, at its cap of {cap} takes {at} glyph steps and one more {more}"
        );
    }
    let flow = style(16.0);
    let floor = with_fonts(|fs| frame_work_steps(&real_run("", &flow, fs), None, true, 0));
    let (open, pitched) = (floor + ring_work_steps(0, 1, 0), floor + pitched_work_steps(0, 0));
    assert!(
        open > INPUT_STEPS && pitched > INPUT_STEPS,
        "beside a frame at its floor, a placement along a line takes {open} glyph steps and a whole-glyph one {pitched}, within the {INPUT_STEPS} an input may take: lay them besides"
    );
    let both = check_raw(
        b"plain text",
        &[FitMode::Fit { min: 0.5, max: 2.0 }],
        Some(&RawLine::Open(FitMode::Repeat)),
        true,
    );
    assert!(
        both.as_ref().is_err_and(|why| why.contains("past the")),
        "a frame and a placement along its line in one input fail: {both:?}"
    );
}

/// #313's properties: a placement along an open path that takes more than
/// its cap's budget and the overshoot past it (`ring_work_steps`) fails, as
/// does one the engine budgets past its cap, one that places more glyphs
/// than its cap, and one whose setup counted tables past the samples its
/// path's curves hold; and a whole-glyph placement that takes more than its
/// budget past its setup and its tables fails, as does one budgeted past
/// its cap, one placing more than its cap, one whose tables pass its
/// curves', and one held to its cap that reports nothing.
///
/// Control: with each never failing, red (that one).
#[test]
fn test_the_flow_properties_find_an_open_placement_past_its_work() {
    use crate::flow::place::halves::{frame_budget, table_steps};
    use crate::flow::place::ring::{ring_work_steps, RingLoss, RING_STEPS};
    use crate::flow::place::Trace;
    use crate::flow::tests::flow_fuzz::{InputWork, MAX_RING_GLYPHS};
    let flow = style(16.0);
    let run = with_fonts(|fs| real_run("MANDALA", &flow, fs));
    let (cap, tables) = (MAX_RING_GLYPHS, 1_025);
    let budget = frame_budget(cap);
    let glyph = PlacedGlyph {
        glyph: 0,
        pos: Vec2::ZERO,
    };
    let laid = |glyphs: usize, truncated: bool| PlacedRun {
        glyphs: vec![glyph; glyphs],
        truncated,
    };
    let whole = laid(cap, false);
    let base = Trace {
        spent: ring_work_steps(cap, run.glyphs.len(), tables),
        budget,
        glyphs: cap,
        tables,
        ..Trace::default()
    };
    let open = |placed: &PlacedRun, trace: &Trace| {
        InputWork::default().open(&run, cap, tables, placed, trace, "a placement")
    };
    assert!(
        open(&whole, &base).is_ok(),
        "precondition: a placement that takes its budget and its overshoot holds"
    );
    let broken: [(&str, Trace, PlacedRun); 5] = [
        (
            "past its budget and its overshoot",
            Trace {
                spent: base.spent + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        (
            "budgeted past its cap",
            Trace {
                budget: budget + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        (
            "scaled by more glyphs than its cap",
            Trace {
                glyphs: cap + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        (
            "placing more glyphs than its cap",
            base.clone(),
            laid(cap + 1, false),
        ),
        (
            "whose tables pass its curves'",
            Trace {
                tables: tables + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
    ];
    for (what, trace, placed) in &broken {
        assert!(open(placed, trace).is_err(), "a placement {what} fails");
    }
    let setup = RING_STEPS + table_steps(tables);
    let base = Trace {
        spent: budget,
        ..base
    };
    let pitched = |placed: &PlacedRun, trace: &Trace| {
        InputWork::default().pitched(cap, tables, placed, trace, "a whole-glyph placement")
    };
    assert!(
        pitched(&whole, &base).is_ok() && setup < budget,
        "precondition: a whole-glyph placement that takes its budget holds"
    );
    let capped = laid(cap, true);
    let broken: [(&str, Trace, PlacedRun); 5] = [
        (
            "past its budget",
            Trace {
                spent: budget + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        (
            "budgeted past its cap",
            Trace {
                budget: budget + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        (
            "placing more glyphs than its cap",
            base.clone(),
            laid(cap + 1, false),
        ),
        (
            "whose tables pass its curves'",
            Trace {
                tables: tables + 1,
                ..base.clone()
            },
            whole.clone(),
        ),
        ("held to its cap, reporting nothing", base.clone(), capped.clone()),
    ];
    for (what, trace, placed) in &broken {
        assert!(
            pitched(placed, trace).is_err(),
            "a whole-glyph placement {what} fails"
        );
    }
    let reported = Trace {
        ring: Some(RingLoss::Short),
        ..base
    };
    assert!(
        pitched(&capped, &reported).is_ok(),
        "precondition: a whole-glyph placement held to its cap and reported holds"
    );
}

/// R45.1's property: an input whose frames may take more than one input
/// may inside the timeout fails — flow_raw's three frames at their
/// least budget, 1,200,000 glyph steps, as the harness laid an input
/// before R45 — and the harness itself lays none such.
///
/// Control: with the property never failing, red.
#[test]
fn test_the_flow_properties_find_an_input_past_the_timeout() {
    use crate::flow::tests::flow_fuzz::{check_raw, frame_fits};
    assert!(
        check_flow_bytes(b"plain text").is_ok(),
        "precondition: one frame an input fits"
    );
    let three = check_raw(b"plain text", &frame_fits(), None, true);
    assert!(
        three.as_ref().is_err_and(|why| why.contains("past the")),
        "three frames an input fail: {three:?}"
    );
}

/// Seed 2771 of the structured generator as it drew at 03ac0b3f, before
/// R51.2 drew its frames' texts at mixed sizes (which draws it another
/// text since): a `Repeat` of an ornament of three glyphs round an
/// ellipse 3.4 × 10²¹ round, where no `f32` sets one glyph's ink apart
/// from the last but by a bisection that barely moves it — 1,262 path
/// reads a glyph, its first walk capped at 65,535 glyphs and 82.7
/// million reads, 26.7 s in a debug build with the compiled faces alone
/// (review 5751828253 found 25.6 s, and 141 s with host faces, and read
/// it as the ring's close; the close never ran, the walk being capped).
///
/// Its ring is budgeted as a frame is (#310), by the glyphs it will lay,
/// at most its cap (rulings B17.1a, B17.2a), and both sides of that are
/// pinned here (ruling B17.2c). **At the harness's cap** (`ring_cap`:
/// 149 glyphs, [`MAX_RING_GLYPHS`] less the steps its path's table of
/// 1,048,577 samples counts, ruling B16.3a), its whole text round its
/// whole path, its first
/// walk stops at the cap, short, and says so; its work is within that
/// cap's budget and its overshoot, its path lookups within what they read
/// as, and the replay is quick — a few hundredths of the 15 s an input may
/// take. **In a map**, at [`MAX_PLACED_GLYPHS`], the path holds the
/// cap's glyphs of its copy, so its budget is 196,608,000 steps: by the
/// estimate the engine scales it by, computed from what the replay's ring
/// estimated from, not by laying it there, which would take some 20 s.
///
/// Precondition: the ring is laid, its estimate past the harness's cap,
/// and its cap, not its budget, stops it. Controls: with the cap not read
/// by its first walk, red (its budget stops it instead, 3,972 glyphs
/// placed, past the cap of 149); with the cap's stop not reported, red
/// (the fuzz property: "a ring cut at 147 glyphs of 3 a copy, its cap
/// 149, reported nothing: capped silently"); with its budget scaled past
/// its cap, red (the fuzz property: 196,608,000 glyph steps budgeted for
/// 65,536 glyphs, past the 447,000 its cap of 149 gives).
///
/// [`MAX_RING_GLYPHS`]: crate::flow::tests::flow_fuzz::MAX_RING_GLYPHS
/// [`MAX_PLACED_GLYPHS`]: crate::flow::MAX_PLACED_GLYPHS
#[test]
fn test_flow_structured_finding_a_ring_whose_inks_its_floats_cannot_part_replays_short_at_its_cap() {
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::halves::frame_budget;
    use crate::flow::place::ring::{ring_glyphs, ring_work_steps, RingLoss, READS_A_STEP};
    use crate::flow::place::{LAST_RING, LAST_RING_ESTIMATE, RINGS_STOPPED, RING_CROSSED};
    use crate::flow::tests::flow_fuzz::{ring_cap, MAX_RING_GLYPHS};
    use crate::flow::MAX_PLACED_GLYPHS;
    let found = include_bytes!("fuzz_findings/structured_ring_whose_inks_its_floats_cannot_part");
    for counter in [&PATH_LOOKUPS, &RINGS_STOPPED] {
        counter.with(|count| count.set(0));
    }
    LAST_RING.with(|last| last.set(None));
    if let Err(why) = crate::flow::tests::flow_fuzz::check_flow_structured(found) {
        panic!("the fuzzer's input: {why}");
    }
    let lookups = PATH_LOOKUPS.with(std::cell::Cell::get);
    let Some(ring) = LAST_RING.with(std::cell::Cell::get) else {
        panic!("precondition: the input lays a ring");
    };
    // The harness's cap beside its path's table (ruling B16.3a): the
    // table counted in its setup, the most of [`MAX_RING_GLYPHS`] that
    // fits the timeout beside it.
    let harness_cap = ring_cap(ring.copy, true, ring.tables);
    assert!(
        ring.cap == harness_cap && ring.cap < MAX_RING_GLYPHS && ring.glyphs == ring.cap,
        "precondition: the ring is laid at the harness's cap beside its table, {harness_cap} glyphs, its estimate past it ({ring:?})"
    );
    assert!(
        ring.loss == Some(RingLoss::Short)
            && RINGS_STOPPED.with(std::cell::Cell::get) == 1
            && RING_CROSSED.with(std::cell::Cell::get).is_none(),
        "precondition: the ring's cap stops it, not its budget, and it is short ({ring:?})"
    );
    assert!(
        ring.placed <= ring.cap,
        "the ring placed {} glyphs, past its cap of {}",
        ring.placed,
        ring.cap
    );
    let most = ring_work_steps(ring.cap, ring.copy, ring.tables);
    assert!(
        ring.spent <= most && lookups <= most * READS_A_STEP,
        "the ring of {} glyphs a copy took {} glyph steps and looked up {lookups} points, past the {most} steps its cap's budget and overshoot give, {} lookups",
        ring.copy,
        ring.spent,
        most * READS_A_STEP
    );
    // In a map: the budget its estimate gives at the engine's cap,
    // arithmetic from what the replay's ring estimated from.
    let Some((copy, advance, length)) = LAST_RING_ESTIMATE.with(std::cell::Cell::get) else {
        panic!("precondition: the ring was estimated");
    };
    let glyphs = ring_glyphs(copy, advance, length, MAX_PLACED_GLYPHS);
    assert_eq!(
        (glyphs, frame_budget(glyphs)),
        (MAX_PLACED_GLYPHS, 196_608_000),
        "in a map the ring's estimate is the cap, its budget 196,608,000 glyph steps ({copy} glyphs a copy, {advance} along a path {length} long)"
    );
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("as found: {why}");
    }
}

/// The flow_structured finding of #318's CI fuzz run (job 128331247806):
/// a letter owed no gap after an ink on the arc of a 120 × 120 rounded
/// rectangle of corners half its side, the band curve 10.68 out. Where
/// nothing is owed, the closed-form layer aims at the least gap its forms
/// tell from none, 6.19e-5 there (`resolution_at`), and set the letter
/// 8.68e-5 apart, 2.5e-5 from its aim; `check_pair_model` held that gap to
/// none instead, to within 8.06e-5, and failed. The property now holds a
/// gap owed nothing to touching, within two of its own resolution, to six
/// of it past (review 5810838863): bounds from its own scale, which the
/// model's aim lies inside, not the aim read from the layer. No placement
/// changes. Found on the compiled faces alone (`FONTCONFIG_FILE` at an
/// empty directory).
///
/// Control: with the property holding a gap owed nothing to none, as it
/// first did, red ("the model sets it at 64.12113, its gap
/// 0.00008678436").
#[test]
fn test_flow_structured_finding_a_letter_owed_nothing_is_held_touching_or_just_past() {
    let found = include_bytes!("fuzz_findings/structured_letter_owed_nothing_past_its_aim");
    for (what, checked) in [
        (
            "fuzzed",
            crate::flow::tests::flow_fuzz::check_flow_structured(found),
        ),
        ("as found", check_flow_structured_as_found(found)),
    ] {
        if let Err(why) = checked {
            panic!("{what}: {why}");
        }
    }
}

/// The frame placement property's excuse (#333, review 5834654837): a
/// glyph the rasterizer draws nothing for may be left unplaced only where
/// its line lays no ink round it. In a frame of `aaaa`, U+10FFFD, `aaaa`,
/// laid `Spread`, it is placed between two inks of its line, and the
/// placement holds; with its `PlacedGlyph` dropped, the property finds it
/// ("glyph 4 was not placed"). In a frame whose second line is U+10FFFD
/// three times and nothing that draws, the line is not laid, and the
/// placement holds with its glyphs unplaced. Precondition: U+10FFFD draws
/// nothing here, and its line's glyphs are not placed.
///
/// Control: the excuse as it was, every glyph that draws nothing
/// excused wherever it is, red: the dropped glyph passes ("the dropped
/// glyph between two inks is found").
#[test]
fn test_the_frame_placement_property_excuses_only_a_glyph_its_line_lays_no_ink_round() {
    use crate::document::values::{FitMode, Percent, Shape};
    use crate::flow::tests::flow_fuzz::check_frame_placement;
    use crate::flow::tests::halves_tests::frame_with;
    use crate::flow::{outline, Rect, RingLayout};
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(400.0, 200.0)),
        Shape::Rectangle {
            radius: Percent::new(30),
        },
    );
    let lay = |text: &str| frame_with(text, 24.0, &shape, &FitMode::Spread, &RingLayout::whole(), true);
    // Laid first: a frame's lay takes the font lock, which the checks
    // below hold.
    let (run, placed, trace) = lay("aaaa\u{10FFFD}aaaa");
    let (run2, placed2, trace2) = lay("aaaa aaaa\n\u{10FFFD}\u{10FFFD}\u{10FFFD}");
    let undrawn = run
        .glyphs
        .iter()
        .position(|g| g.bytes.start == 4)
        .expect("U+10FFFD is shaped");
    let laid = run.layout_glyphs().nth(undrawn).cloned().expect("it is laid out");
    let mut lease = crate::font::fonts::FontSystemLease::new("flow fuzz tests");
    let draws = crate::flow::tests::support::raster_box(lease.fonts(), &laid, 16.0).is_some();
    let held = check_frame_placement(&run, &placed, (true, &trace), "between inks", &mut lease);
    let mut dropped = placed.clone();
    dropped.glyphs.retain(|g| g.glyph as usize != undrawn);
    let found = check_frame_placement(&run, &dropped, (true, &trace), "between inks", &mut lease);
    let lone: Vec<usize> = (0..run2.glyphs.len())
        .filter(|&k| !run2.glyphs[k].blank && run2.glyphs[k].bytes.start >= 10)
        .collect();
    let unplaced = lone
        .iter()
        .all(|&k| placed2.glyphs.iter().all(|g| g.glyph as usize != k));
    let lone_held = check_frame_placement(&run2, &placed2, (true, &trace2), "a line of nothing", &mut lease);
    drop(lease);
    assert!(!draws, "precondition: U+10FFFD draws nothing here");
    assert_eq!(held, Ok(()), "the placement holds");
    assert!(
        found
            .as_ref()
            .is_err_and(|why| why.contains(&format!("glyph {undrawn} was not placed"))),
        "the dropped glyph between two inks is found: {found:?}"
    );
    assert!(
        lone.len() == 3 && unplaced,
        "precondition: the second line's three glyphs are not placed ({lone:?})"
    );
    assert_eq!(lone_held, Ok(()), "a line of nothing that draws is not owed");
}

/// The flow_structured finding of #324's smoke run: a directional ring
/// round an outline whose four turns, located on its path, do not fall
/// in order along it — the path's floats do not tell them apart — so a
/// short run's middle was held between a turn and one before it, and
/// `f32::clamp` panicked on bounds out of order. Such an outline has no
/// runs to lay and is laid as a ring of its spans alone. 296 bytes, as
/// found.
///
/// Control: with the turns' order not checked, red ("min > max, or
/// either was NaN").
#[test]
fn test_flow_structured_finding_a_directional_ring_whose_turns_fall_out_of_order_replays() {
    let found = include_bytes!("fuzz_findings/structured_directional_ring_whose_turns_fall_out_of_order");
    if let Err(why) = crate::flow::tests::flow_fuzz::check_flow_structured(found) {
        panic!("the fuzzer's input: {why}");
    }
}

/// The flow_structured finding of #347's CI fuzz run (job 132083191219,
/// #348): the tangent line's root in closed form held to its bisection
/// (`check_line_root`) for two inks one axis 11.04 apart and 0.012 owed,
/// where the closed form's discriminant was a difference of two near
/// equals in `f32`, and its root was 6.8e-4 from the bisection's, past
/// the property's bound of 2.5e-4. The discriminant is taken by
/// Lagrange's identity now. The line itself is held to an oracle in `f64`
/// by `test_flow_halves_the_tangent_line_root_holds_where_little_is_owed_of_a_wide_gap`.
/// The input has reached this line only since #336, whose directional
/// glyphs are drawn from the choices before it; `line_root` is older. 202
/// bytes, as found.
///
/// Control: with the discriminant `b² − a c`, red ("owed 0.012: the
/// closed form steps -15.051204, the bisection -15.051884").
#[test]
fn test_flow_structured_finding_a_line_root_owed_little_of_a_wide_gap_replays() {
    let found = include_bytes!("fuzz_findings/structured_line_root_owed_little_of_a_wide_gap");
    for (what, checked) in [
        (
            "fuzzed",
            crate::flow::tests::flow_fuzz::check_flow_structured(found),
        ),
        ("as found", check_flow_structured_as_found(found)),
    ] {
        if let Err(why) = checked {
            panic!("{what}: {why}");
        }
    }
}
