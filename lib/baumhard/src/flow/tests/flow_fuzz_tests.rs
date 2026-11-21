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
    later_faces,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{
    shape_box, shape_run, FlowSpan, Outline, PlacedGlyph, PlacedRun, Rect, RingLayout, FILLER_SPAN,
};
use crate::font::fonts::AppFont;
use crate::util::tests::choices::Choices;

/// The seeds a sweep of the structured generator's seeds 0 to 5,000 found
/// failing (review round R50), with the compiled faces alone and with this
/// machine's host faces:
///
/// - **1021**, on every host: a `Spread` of seven glyphs along a quadratic
///   from (1, 1) by (−2.4 × 10³⁸, −587) to (−10³⁰, 1), whose natural length
///   fits its 2.4 × 10³⁸, was cut to nothing with nothing said. The
///   curve's speed bound, twice its first leg, was past the largest `f32`,
///   so its path built no table and had no length. Such a curve is longer
///   than placement lays, and its path says so now (`ArcPath::too_long`,
///   #363): nothing is laid along it, by name.
/// - **2970** and **4284**, with host faces that shape their text: a
///   frame's line on a part some 10³³ along its half, where an `f32` sets
///   every letter at one center, so no gap moved its end; the tracking
///   solve widened its gap past every `f32` and placed the line at `NaN`.
///   Such an outline is longer than placement lays since #363: the frame
///   lays nothing and says so, and the tracking solve's stop for it, and
///   its reading of a lay past every `f32`, which no input within the
///   limit reached, are gone.
///
/// Seed 2771, with host faces, laid a `Repeat` ring whose seam no count of
/// its copies closed for past 18 minutes; its count is held by
/// `test_flow_a_ring_that_never_closes_counts_its_copies_in_few_asks`, and
/// the input it drew then, which R51.2's mixed sizes draw otherwise now,
/// is replayed at the harnesses' ring cap, short and within its budget
/// (#310, ruling B17.2c), by
/// `test_flow_structured_finding_a_ring_whose_inks_its_floats_cannot_part_replays_short_at_its_cap`.
///
/// Controls: with a curve no count tables building no stretch, so that
/// its path is not too long, red at 1021 ("1 glyphs laid along a path
/// longer than placement lays"); the limit not held, red at 2970 where
/// host faces shape its text.
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
/// And each placement is held to the longest path placement lays (#363,
/// ruling B42.1 §6; `held_to_the_limit`): along a path clearly past it
/// nothing is laid and the placement says why, along one clearly within
/// it the placement does not say so.
///
/// Precondition: the seeds draw far-out frames with sides and far-out
/// `Repeat`s, and placements along paths clearly within the limit, ten or
/// more, and clearly past it, two or more (20 and 4 as the generator
/// stands). Controls: with `far_out` drawing nothing, red; with no path
/// too long, red at seed 79 ("116 glyphs laid along a path longer than
/// placement lays").
#[test]
fn test_generated_flows_hold_every_flow_property() {
    use crate::flow::tests::flow_fuzz::{FAR_FRAMES, FAR_REPEATS, HELD_TO_THE_LIMIT};
    FAR_FRAMES.with(|n| n.set(0));
    FAR_REPEATS.with(|n| n.set(0));
    HELD_TO_THE_LIMIT.with(|held| held.set((0, 0)));
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
    let (within, past) = HELD_TO_THE_LIMIT.with(std::cell::Cell::get);
    assert!(
        within >= 10 && past >= 2,
        "precondition: the seeds hold placements along paths clearly within the limit ({within}) and \
         clearly past it ({past})"
    );
}

/// The seeds the twin property draws its far-out frames from (#362), each
/// that draws an outline drawing it far out, at `10⁶` or `10⁷`
/// (`flow_fuzz::far_frames_laid`).
const TWIN_SEEDS: u64 = 1_000;

/// #362, the issue's item 5 held by the suite rather than a fuzz target:
/// every far-out frame with sides the structured generator lays over
/// [`TWIN_SEEDS`] seeded inputs, drawn far out each time it draws an
/// outline, at `10⁶` or `10⁷` (`flow_fuzz::far_frames_laid`, which lays
/// those frames from the generator's choices and checks none of the
/// properties the structured sweep above holds), where the coordinates
/// hold its text's and its ornament's glyphs apart (ruling B18.1 §9,
/// `coordinates_hold_apart` at the least scale its fit lays them at, a
/// `Once` at a `Spread`'s), and whose twin — the same frame, text, layout
/// and cap round the same outline at the origin — lays its sides with no
/// junction empty or kept short, both laying their text whole, lays them
/// so too: some copies, no junction empty or kept short, and no fewer
/// copies than `least_far_copies` bounds from the twin's count and arcs,
/// the coordinates' spacing and the runs as shaped.
///
/// Most far frames are owed nothing here. Of the 264 the seeds lay as the
/// generator stands, 238 are not held: 74 are a `Repeat`, which owes no
/// text whole; 105 lie where the coordinates cannot hold their glyphs
/// apart; 46 have a twin that lays no sides; 11 a twin that cuts its text,
/// and 1 a far frame that does; 1 a twin that reports a loss of its sides.
///
/// It is not a property of the fuzz targets: a twin is a second budgeted
/// frame an input, past what one input may take there (ruling R45.1,
/// `STRUCTURED_FRAMES`). Laying it within that bound, by the twin's
/// realized work rather than its worst case, is the owner's to rule on.
///
/// Precondition: the seeds hold 15 far frames or more to their twins (26
/// of the 264). Control: with every arc of a frame further than `10⁵` from
/// the origin cut to its first two inks, no loss reported, red: 13 of the
/// 26, each short of the copies' bound. It is a net for a far frame laying
/// fewer copies than its twin, whatever the cause, across the generator's
/// frames; it does not reach the mechanisms of #362 itself — with the
/// count search walking, the floor eight times as wide or none at all, it
/// is green: the walk costs a frame its sides only where some two hundred
/// copies of a short ornament lie beside a long text, which is what the
/// spreads of `far_sides_tests` are, and those hold them.
#[test]
fn test_generated_far_frames_lay_their_sides_as_their_twins_at_the_origin() {
    use crate::document::values::FitMode;
    use crate::flow::place::{place_on_closed_capped, Trace};
    use crate::flow::tests::flow_fuzz::{
        coordinates_hold_apart, far_frames_laid, least_far_copies, ornament_run, outline_reach,
    };
    use crate::flow::PlacedRun;
    let frames = far_frames_laid(0..TWIN_SEEDS).unwrap_or_else(|why| panic!("{why}"));
    let sides_of = |placed: &PlacedRun, trace: &Trace| {
        let lines: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
        placed.glyphs.len().saturating_sub(lines)
    };
    // Why each far frame not held is owed nothing here.
    let mut dropped = [0usize; 6];
    let mut held = 0;
    let mut broken = Vec::new();
    for frame in &frames {
        if frame.fit == FitMode::Repeat {
            dropped[0] += 1;
            continue;
        }
        let held_as = match &frame.fit {
            FitMode::Once { .. } => FitMode::Spread,
            other => other.clone(),
        };
        let Some(ornament) = with_fonts(|fs| ornament_run(&frame.layout, &frame.flow, frame.scale, fs))
        else {
            continue;
        };
        let reach = outline_reach(&frame.far);
        if !coordinates_hold_apart(reach, &frame.run, &held_as)
            || !coordinates_hold_apart(reach, &ornament, &held_as)
        {
            dropped[1] += 1;
            continue;
        }
        let (placed, far_trace) = frame
            .laid
            .as_ref()
            .expect("a far frame the coordinates hold apart is laid (`far_frame_owed`)");
        let mut trace = Trace::default();
        let (_, twin) = with_fonts(|fs| {
            place_on_closed_capped(
                &frame.run,
                &frame.origin,
                frame.start,
                frame.direction,
                &frame.fit,
                &frame.layout,
                frame.outward,
                &frame.flow,
                fs,
                frame.cap,
                Some(&mut trace),
            )
        });
        let twin_sides = sides_of(&twin, &trace);
        // Owed where the twin lays its sides and both lay their text whole:
        // a frame loses its sides before its text (ruling R48.3), so one
        // that keeps more of a cut text than its twin may keep fewer sides.
        let whole = |placed: &PlacedRun, trace: &Trace| !placed.truncated && trace.cut_apart.is_empty();
        let why_not = if twin_sides == 0 {
            Some(2)
        } else if !whole(&twin, &trace) {
            Some(3)
        } else if !whole(placed, far_trace) {
            Some(4)
        } else if !trace.unheld.sides.is_empty() {
            Some(5)
        } else {
            None
        };
        if let Some(why) = why_not {
            dropped[why] += 1;
            continue;
        }
        held += 1;
        let glyphs = ornament.glyphs.len().max(1);
        let least = least_far_copies(
            twin_sides / glyphs,
            trace.side_arcs,
            &frame.run,
            &ornament,
            &held_as,
            reach,
        ) * glyphs;
        let far_sides = sides_of(placed, far_trace);
        if far_sides == 0 || !far_trace.unheld.sides.is_empty() || far_sides < least {
            broken.push(format!(
                "{:?} {:?} round {:?}: {far_sides} glyphs of its sides ({twin_sides} at the origin over {} arcs, {least} owed), {:?}, its budget {} of {} spent",
                frame.fit,
                frame.layout.sides,
                frame.far,
                trace.side_arcs,
                far_trace.unheld.sides,
                far_trace.spent,
                far_trace.budget
            ));
        }
    }
    assert!(
        held >= 15,
        "precondition: the seeds hold 15 far frames or more to their twins ({held} of {}; not held: \
         {} a `Repeat`, {} where the coordinates cannot hold them, {} whose twin lays no sides, {} \
         whose twin cuts its text, {} that cut their own, {} whose twin reports a loss)",
        frames.len(),
        dropped[0],
        dropped[1],
        dropped[2],
        dropped[3],
        dropped[4],
        dropped[5]
    );
    assert!(
        broken.is_empty(),
        "{} of {held} far frames do not lay their sides as their twins at the origin:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// The sweep above over [`every_face`]: `faces()`, the ornament
/// fallback's two (#297), the default face (#228) and Scrapbook Chinese
/// (#355), which a map's `font` can pin as it pins any other. `faces()`
/// stays as it is, since every committed structured seed and finding
/// chose its faces from that list; this sweep and the
/// flow_structured_every_face target are where a span pins `Mandala
/// Ornaments`, `Mandala Emoji` — the first compiled face with no outline
/// and no space — `Inter Variable`, the first variable one, or `Scrapbook
/// Chinese`, the first symbol font that maps the whole printable Latin-1
/// range and so draws a text-shaped string entirely in its own glyphs
/// (`Symbols1`, a symbol font holding only capital letters, is in
/// `faces()`).
/// The seeds are other inputs than the sweep above's, the list being
/// longer.
///
/// Precondition: each of [`later_faces`] is pinned by some seed's spans
/// or flow, so the sweep cannot pass without meeting them. Controls: with
/// `every_face` returning `faces()` alone, red on it; with
/// `ScrapbookChinese` left out of `every_face` but kept in `later_faces`,
/// red on `ScrapbookChinese`.
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
    for font in &later_faces().collect::<Vec<_>>() {
        let times = pinned.iter().filter(|pin| *pin == font).count();
        assert!(
            times > 0,
            "precondition: a span of the sweep pins {font:?} (pinned {} faces)",
            pinned.len()
        );
    }
}

/// `faces()` never changes, as its doc says: a choice picks the face at
/// its value modulo the list's length, so a face added anywhere, one
/// taken out, or one renamed (the list sorts by name, and the build names
/// each `AppFont` after its face's full name) moves committed
/// flow_structured seeds and findings to other faces, and silently, since
/// each still passes. So the list is held, face by face and in order, to
/// what it was when they were drawn. #355 is how it nearly moved:
/// Scrapbook Chinese, left out only because it had no ids until then,
/// gained them and would have joined it. A face a map can newly pin goes
/// in `later_faces` instead. Controls: `ScrapbookChinese` taken out of
/// `later_faces`, red on the face added; and `Regal` in the literal
/// spelled `Regal2`, red on the face that differs.
#[test]
fn test_the_structured_flow_face_list_keeps_the_faces_its_seeds_chose_from() {
    let faces: Vec<String> = crate::flow::tests::flow_fuzz::faces()
        .iter()
        .map(|font| format!("{font:?}"))
        .collect();
    assert_eq!(faces, FACES_THE_SEEDS_CHOSE_FROM, "faces() changed");
}

/// `faces()` when the flow_structured seeds and findings were drawn from
/// it, each face by its `AppFont` name.
const FACES_THE_SEEDS_CHOSE_FROM: [&str; 51] = [
    "African",
    "AliceInWonderland",
    "AlphaMusicMan",
    "AlphaSports",
    "AppleTea",
    "ArigatouGozaimasu",
    "CarnivaleeFreakshow",
    "CarnivaleeFreakshowDemo",
    "Casanova",
    "ChubbyDotty",
    "Critters1DCSemiBold",
    "CulinaryArt",
    "DenseLetters",
    "DistorRegular",
    "EgyptienneZierinitialien",
    "Evilz",
    "FabulousSteampunk",
    "FasterOne",
    "Flowers3",
    "GalatiaSIL",
    "GalatiaSILBold",
    "Keyboard212",
    "LittleCity2000",
    "MexicanOrnaments",
    "NIGHTCROW",
    "Norse",
    "NorseBold",
    "NotoSansDevanagariRegular",
    "NotoSansEgyptianHieroglyphsRegular",
    "NotoSansHebrewRegular",
    "NotoSerifTibetanRegular",
    "Novem",
    "PeasHeartsDelight2",
    "PerpetratorItalic",
    "PerpetratorRegular",
    "PictoPeopleRegular",
    "Pinewood",
    "PublicPixel",
    "RCRocketRegular",
    "RazzleDazzle",
    "Regal",
    "Ships",
    "SilhousTwo",
    "Skullphabet",
    "SuperPlumberBrothers",
    "Symbols1",
    "ToyTrain",
    "TreeRoot",
    "XTypewriter",
    "XTypewriterBold",
    "ZOEGERMANYFingeralphabetGraffiti",
];

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
/// A miss that is not a number now overshoots, as one a spent budget
/// leaves still does.
///
/// Since #363 an outline that long is not laid at all: it is longer than
/// placement lays (`MAX_PATH_LENGTH`), and each placement round it lays
/// nothing and says why. The input as the fuzzer found it is replayed
/// beside the case it comes down to. Control: the limit not held, red
/// (the replay: "76 glyphs laid along a path longer than placement
/// lays").
#[test]
fn test_a_spread_around_an_outline_near_the_largest_f32_is_too_long_to_lay() {
    use crate::document::values::{Direction, Edge, FitMode, StartPoint};
    use crate::flow::place::ring::RingLoss;
    use crate::flow::place::{place_on_closed_traced, Trace};
    use crate::flow::{Outline, Rect, RingLayout};
    let found = include_bytes!("fuzz_findings/spread_gap_past_the_largest_f32");
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("the fuzzer's input: {why}");
    }
    let plain = style(16.0);
    // The outline the input drew: a rounded rectangle 1.78e38 on a side,
    // its perimeter 2.8e38.
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
            let mut trace = Trace::default();
            let (run, placed) = with_fonts(|fs| {
                let run = real_run("abc", &plain, fs);
                let (replaced, placed) = place_on_closed_traced(
                    &run,
                    &path,
                    start,
                    direction,
                    &fit,
                    &RingLayout::whole(),
                    false,
                    &plain,
                    fs,
                    Some(&mut trace),
                );
                (replaced.unwrap_or(run), placed)
            });
            assert!(
                placed.glyphs.is_empty() && placed.truncated && trace.ring == Some(RingLoss::TooLong),
                "{fit:?} {direction:?}: {} glyphs, truncated {}, {:?}",
                placed.glyphs.len(),
                placed.truncated,
                trace.ring
            );
            if let Err(why) = check_placement(&run, &placed, false, "around the outline") {
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
///
/// Since #363 a path 10³⁰ long is longer than placement lays, and each
/// input as found lays nothing along it and says why: both are replayed
/// for that. The bounds are held on the same inputs with the floats that
/// set their curves that far out shortened to 10¹⁷
/// (`…_within_the_limit`), which lay the cap as the inputs did.
#[test]
fn test_a_repeat_along_a_curve_as_long_as_placement_lays_walks_each_glyph_thrice_at_most() {
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::{GLYPHS_PLACED, MAX_RUN_ON_STEPS, STEPS_TAKEN};
    use crate::flow::MAX_PLACED_GLYPHS;
    for (what, found) in [
        (
            "the CI input as found",
            &include_bytes!("fuzz_findings/repeat_along_a_curve_past_the_root_of_the_largest_f32")[..],
        ),
        (
            "the review's cubic as found",
            &include_bytes!("fuzz_findings/repeat_along_a_cubic_past_the_root_of_the_largest_f32")[..],
        ),
    ] {
        GLYPHS_PLACED.with(|count| count.set(0));
        if let Err(why) = check_flow_structured_as_found(found) {
            panic!("{what}: {why}");
        }
        assert_eq!(
            GLYPHS_PLACED.with(std::cell::Cell::get),
            0,
            "{what}: its path is longer than placement lays, and nothing is laid along it"
        );
    }
    let findings: [(&str, &[u8]); 2] = [
        (
            "the CI input",
            include_bytes!(
                "fuzz_findings/repeat_along_a_curve_past_the_root_of_the_largest_f32_within_the_limit"
            ),
        ),
        (
            "the review's cubic",
            include_bytes!(
                "fuzz_findings/repeat_along_a_cubic_past_the_root_of_the_largest_f32_within_the_limit"
            ),
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
/// now, #291): along a line as long as placement lays, a `Spread` — and a
/// `Fit` held at scale 1 — of three letters places every glyph, finite,
/// and cuts nothing (`check_placement`'s whole-placement property): lines
/// 2 × 10¹⁵, 2 × 10¹⁸ and 9.2 × 10¹⁸ long, the last the longest there is.
/// A line longer than that lays nothing and says why (#363), from one a
/// float longer to the longest an `f32` holds: among them the lines past
/// `√f32::MAX` (about `1.8e19`), which once measured their length
/// infinite, kept no stretch, and placed nothing with nothing said.
/// Controls: the limit not held, red (the lines past it are laid); the
/// limit held at half what it is, red (the longest line within it is
/// cut).
#[test]
fn test_a_spread_along_a_line_as_long_as_placement_lays_places_every_glyph() {
    use crate::document::values::FitMode;
    use crate::flow::place::ring::RingLoss;
    use crate::flow::place::{place_on_open_traced, Trace};
    use crate::flow::{Curve, MAX_PATH_LENGTH};
    let plain = style(16.0);
    let half = 0.5 * MAX_PATH_LENGTH;
    for (reach, laid) in [
        (1.0e15_f32, true),
        (1.0e18, true),
        (half, true),
        (half.next_up(), false),
        (1.0e19, false),
        (1.0e20, false),
        (1.0e30, false),
        (1.7e38, false),
    ] {
        let curve = Curve::Line(Vec2::new(-reach, 0.0), Vec2::new(reach, 0.0));
        for fit in [FitMode::Spread, FitMode::Fit { min: 1.0, max: 1.0 }] {
            let mut trace = Trace::default();
            let (run, placed) = with_fonts(|fs| {
                let run = real_run("abc", &plain, fs);
                let (replaced, placed) =
                    place_on_open_traced(&run, &curve, &fit, 0.0, &plain, fs, Some(&mut trace));
                (replaced.unwrap_or(run), placed)
            });
            if let Err(why) = check_placement(&run, &placed, laid, "along the line") {
                panic!("{reach:e} {fit:?}: {why}");
            }
            assert_eq!(
                (
                    placed.glyphs.is_empty(),
                    placed.truncated,
                    trace.ring == Some(RingLoss::TooLong)
                ),
                (!laid, !laid, !laid),
                "{reach:e} {fit:?}: {} glyphs, truncated {}, {:?}",
                placed.glyphs.len(),
                placed.truncated,
                trace.ring
            );
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
/// `-1e30`. The walk laid the `MAX_PLACED_GLYPHS` cap (a million glyphs
/// then, 65,536 since ruling R41), every one stepped by its ink (ruling
/// R31), and on a path that far out no glyph's ink gap could come within
/// `INK_TOLERANCE` of what it owes: each took every one of `INK_TURNS`
/// turns, and each turn every one of `INK_ROUNDS` rounds of bisection —
/// 319 path lookups a glyph. A round that leaves its bracket as it was,
/// and a turn that leaves its center where it was, leave them so every
/// round and turn after, so each stops there, which changes no position;
/// and since #333 the whole-glyph placement laid beside the walk
/// (`place_pitched`) stops solving its slack where its floats cannot see
/// it.
///
/// Since #363 a cubic 10³⁰ long is longer than placement lays: the input
/// lays nothing along it, as text or as whole glyphs, and says why,
/// which is what is held here. Its stops are held within the limit by
/// the two tests below: the whole-glyph solve's on this input with its
/// floats rescaled into the limit, and the walk's ink solves' on a cubic
/// 10⁷ from the origin, where both of theirs are what bound it.
///
/// Control: the limit not held, red ("65520 glyphs laid along a path
/// longer than placement lays").
#[test]
fn test_a_repeat_of_an_ornament_along_a_cubic_too_long_to_lay_lays_nothing_and_says_so() {
    use crate::flow::tests::flow_fuzz::HELD_TO_THE_LIMIT;
    let found = include_bytes!("fuzz_findings/repeat_of_an_ornament_along_a_cubic_its_floats_cannot_resolve");
    HELD_TO_THE_LIMIT.with(|held| held.set((0, 0)));
    // The replay holds each placement to the limit: past it, nothing laid
    // and the reason named (`held_to_the_limit`).
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("the fuzzer's input: {why}");
    }
    let (_, past) = HELD_TO_THE_LIMIT.with(std::cell::Cell::get);
    assert!(
        past >= 2,
        "precondition: its cubic's placements, as text and as whole glyphs, are held as clearly past the \
         limit ({past})"
    );
}

/// The finding above with the floats that set its cubic rescaled into
/// the limit (#363): each of them past 9 × 10¹⁸, its control point at
/// `-1e30` among them, set to its leading digits times 10¹⁴ — a cubic
/// within the limit whose far points lie some 10¹⁴ from the origin, where
/// the floats, 10⁷ apart, still cannot resolve the ornament's gaps. The
/// walk lays the cap of it as the input did, and the replay lays two
/// whole-glyph placements beside the walk (`place_pitched`, where a face
/// draws one of the input's glyphs with no outline and inks it as its
/// box, #333): one of the cap, 65,536 glyphs, cut there, and one of 8.
/// Their floats cannot see the slack either, so each solve of a count's
/// slack stops at the first round whose lay does not answer its move
/// (`MOST_UNRESOLVED_UNANSWERED`).
///
/// The walk's own cost there is held by the test below, on a path where
/// both of its ink solves' stops are what bound it.
///
/// Precondition: the input lays the cap of an ornament (no `STEPS_TAKEN`
/// for most glyphs, which an ornament takes by its ink: its box steps are
/// the run-on probe's, `MAX_RUN_ON_STEPS` at most), its placements are
/// held as clearly within the limit, and a slack solve where the floats
/// cannot resolve the pattern's gaps meets a round its lay does not
/// answer. Control: the solve without its stop where the floats cannot
/// resolve the gaps (`unresolved && unanswered` in `pitch::pitch`), red:
/// a solve goes on past two such rounds.
#[test]
fn test_a_repeat_of_an_ornament_where_floats_cannot_resolve_its_gaps_stops_solving_them() {
    use crate::flow::place::pitch::MOST_UNRESOLVED_UNANSWERED;
    use crate::flow::place::{GLYPHS_PLACED, MAX_RUN_ON_STEPS, STEPS_TAKEN};
    use crate::flow::tests::flow_fuzz::HELD_TO_THE_LIMIT;
    use crate::flow::MAX_PLACED_GLYPHS;
    let found = include_bytes!(
        "fuzz_findings/repeat_of_an_ornament_along_a_cubic_its_floats_cannot_resolve_within_the_limit"
    );
    for counter in [&STEPS_TAKEN, &GLYPHS_PLACED, &MOST_UNRESOLVED_UNANSWERED] {
        counter.with(|count| count.set(0));
    }
    HELD_TO_THE_LIMIT.with(|held| held.set((0, 0)));
    if let Err(why) = check_flow_structured_as_found(found) {
        panic!("the input within the limit: {why}");
    }
    let [steps, placed, unanswered] = [&STEPS_TAKEN, &GLYPHS_PLACED, &MOST_UNRESOLVED_UNANSWERED]
        .map(|counter| counter.with(std::cell::Cell::get));
    let (within, past) = HELD_TO_THE_LIMIT.with(std::cell::Cell::get);
    assert!(
        placed + 64 > MAX_PLACED_GLYPHS && steps < placed / 4 + MAX_RUN_ON_STEPS,
        "precondition: the input lays the cap of an ornament ({placed} glyphs, {steps} box steps)"
    );
    assert!(
        within >= 2 && past == 0,
        "precondition: its placements are held as clearly within the limit ({within} within, {past} past)"
    );
    assert!(
        unanswered > 0,
        "precondition: a slack solve where the floats cannot resolve the pattern's gaps meets a round its \
         lay does not answer"
    );
    assert!(
        unanswered <= 1,
        "a slack solve where the floats cannot see the slack went on past a round its lay did not answer \
         ({unanswered} such rounds)"
    );
}

/// What the #294 finding above was found for, where the path's floats
/// are coarse and its length is ordinary (#363, ruling B42.1 §4): `· `
/// repeated along a cubic 2 × 10⁵ long, 10⁷ from the origin, where the
/// floats are a unit apart, a thousand times `INK_TOLERANCE`. No ink gap
/// can come within the tolerance of what it owes there, so without
/// their stops every glyph would take every one of `INK_TURNS` turns of
/// `ink_gap_at` and, in each, every one of `INK_ROUNDS` rounds of
/// `ink_bisect`. A turn that leaves its center where it
/// was, and a round that leaves its bracket as it was, leave them so
/// every turn and round after, so each stops there, which changes no
/// position. So the placement, its readable lay and its run-on probe
/// included, takes at most 26 path lookups a glyph: measured, 20.07
/// (756,090 for 37,682 glyphs; 756,156 before #376 had a turn after a
/// still one stride on without reading its gap and guess again, which
/// three solves past the cubic's end, where the path holds its end point,
/// take). The finding rescaled into the limit above is no pin for them:
/// there the round's stop moves its walk's lookups by a tenth (696,222 to
/// 759,786), well inside what its run-on probe alone may take
/// (`8 × MAX_RUN_ON_STEPS`).
///
/// Precondition: the walk lays many glyphs, each by its ink (no
/// `STEPS_TAKEN` for most of them). Controls: the turn's stop taken out,
/// red at 88.2 lookups a glyph; the round's, red at 32.7; both, 162.
#[test]
fn test_a_repeat_of_an_ornament_far_from_the_origin_stops_its_ink_solves() {
    use crate::document::values::FitMode;
    use crate::flow::path::PATH_LOOKUPS;
    use crate::flow::place::{place_on_open_capped, GLYPHS_PLACED, STEPS_TAKEN};
    use crate::flow::tests::support::{real_run, style, with_fonts};
    use crate::flow::{Curve, MAX_PLACED_GLYPHS};
    use glam::Vec2;
    let at = Vec2::splat(1.0e7);
    let length = 2.0e5;
    let curve = Curve::Cubic(
        at,
        at + Vec2::new(0.35 * length, -0.3 * length),
        at + Vec2::new(0.65 * length, 0.3 * length),
        at + Vec2::new(length, 0.0),
    );
    let plain = style(16.0);
    let [steps, placed, lookups] = with_fonts(|fs| {
        let run = real_run("· ", &plain, fs);
        for counter in [&STEPS_TAKEN, &GLYPHS_PLACED, &PATH_LOOKUPS] {
            counter.with(|count| count.set(0));
        }
        let (_, laid) = place_on_open_capped(
            &run,
            std::slice::from_ref(&curve),
            &FitMode::Repeat,
            0.0,
            &plain,
            fs,
            MAX_PLACED_GLYPHS,
            None,
        );
        assert!(!laid.truncated, "the cubic is laid whole");
        [&STEPS_TAKEN, &GLYPHS_PLACED, &PATH_LOOKUPS].map(|counter| counter.with(std::cell::Cell::get))
    });
    assert!(
        placed > 30_000 && steps < placed / 4,
        "precondition: the walk lays many glyphs, each by its ink ({placed} glyphs, {steps} box steps)"
    );
    assert!(
        lookups <= 26 * placed,
        "{lookups} path lookups for {placed} glyphs placed, past 26 a glyph"
    );
}

/// A ring of `text` at `points`, laid `Repeat` round `shape`, outward, at
/// most `cap` glyphs — with the full stride search (`NO_STILL_CHAIN`)
/// where `full` — the solves of its inks it made (`INK_SOLVES`), and how
/// its budget left it.
fn ring_and_its_ink_solves(
    text: &str,
    points: f32,
    shape: &Outline,
    cap: usize,
    full: bool,
) -> (
    PlacedRun,
    crate::flow::place::InkSolves,
    Option<crate::flow::place::ring::RingLoss>,
) {
    use crate::document::values::{Direction, Edge, FitMode, StartPoint};
    use crate::flow::place::{place_on_closed_capped, InkSolves, Trace, INK_SOLVES, NO_STILL_CHAIN};
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            NO_STILL_CHAIN.with(|full| full.set(false));
        }
    }
    let flow = style(points);
    with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        INK_SOLVES.with(|solves| solves.set(InkSolves::default()));
        NO_STILL_CHAIN.with(|switch| switch.set(full));
        let _restore = Restore;
        let mut trace = Trace::default();
        let (_, placed) = place_on_closed_capped(
            &run,
            shape,
            StartPoint {
                edge: Edge::Top,
                at: 0.0,
            },
            Direction::Clockwise,
            &FitMode::Repeat,
            &RingLayout::whole(),
            true,
            &flow,
            fs,
            cap,
            Some(&mut trace),
        );
        (placed, INK_SOLVES.with(std::cell::Cell::get), trace.ring)
    })
}

/// The spacing of the `f32`s at `at`, and above it: the last bit of a
/// coordinate there.
fn grain_at(at: f32) -> f32 {
    f32::from_bits(at.to_bits() + 1) - at
}

/// The reads `ink_gap_at` takes for a glyph no stride of whose turns moves
/// its ink (#376, rulings B48.1 to B48.3): the gap at its center and
/// `ink_distance_at`'s guess, once, and each stride of its `INK_TURNS`
/// turns of `INK_STRIDES` strides — 2 + 12 × 64 = 770 points, three reads
/// each on a closed path, which pushes an ink outward along the chord
/// across its reach (`ink_at`: the point and the chord's two ends). The
/// full search reads each turn's gap and guess again, 12 × 66 points:
/// 2,376 reads.
const ONE_STILL_CHAIN: usize = (2 + 12 * 64) * 3;

/// #376: a `Repeat` ring of dots round an ellipse 10¹⁵ across about the
/// origin, further out than #374's ring, where the canvas's floats are
/// 2²⁵ apart. Every turn of every solve of the ring's first 1,024 dots is
/// still, so each turn after the first strides on from where the last
/// one's strides ended, without reading its center's gap and its guess
/// again, and the solve returns what the full search returns: exactly
/// [`ONE_STILL_CHAIN`] reads a solve, where the full search takes 2,376.
///
/// The precondition — floats further apart than 768 of the longest strides
/// the ring's searches take — is the regime's boundary, below which a
/// chain crosses a float often; it is not enough for no chain to cross
/// one, since a coordinate moved far less than a float still changes
/// where it crosses half of one. Round #374's ring, which meets it too, 28
/// of the first 1,024 solves cross a float and read fewer. What pins this
/// fixture as still throughout is the exact count of still solves.
///
/// Preconditions: that boundary, and the ring lays its cap, a solve for
/// each glyph. Controls: the full search (`NO_STILL_CHAIN`), red
/// ("2433024 path reads for 1024 solves of a glyph's ink, not 2365440,
/// 2310 each"); a solve whose turns end at its first still one, red at
/// 198 each; a search that ends at its first stride that leaves the ink
/// where it was, red at 9 each.
#[test]
fn test_a_repeat_ring_whose_strides_cannot_move_its_ink_reads_no_gap_twice() {
    use crate::document::values::Shape;
    use crate::flow::outline;
    let across = 1.0e15_f32;
    let far = 0.5 * across;
    let shape = outline(Rect::new(Vec2::splat(-far), Vec2::splat(across)), Shape::Ellipse);
    let cap = 1_024;
    let (placed, solved, _) = ring_and_its_ink_solves("·", 16.0, &shape, cap, false);
    let (_, longest) = solved.strides.unwrap_or((f32::NAN, f32::NAN));
    assert!(
        grain_at(far) > 768.0 * longest,
        "precondition: the floats 5 × 10¹⁴ from the origin ({} apart) are further apart than 768 of the \
         ring's longest stride ({longest})",
        grain_at(far)
    );
    assert!(
        placed.glyphs.len() == cap && solved.solves == cap,
        "precondition: the ring lays its cap of {cap} glyphs, a solve of its ink for each ({} placed, {} solves)",
        placed.glyphs.len(),
        solved.solves
    );
    assert_eq!(
        (solved.reads, solved.most, solved.still),
        (ONE_STILL_CHAIN * solved.solves, ONE_STILL_CHAIN, solved.solves),
        "{} path reads for {} solves of a glyph's ink, not {}, {ONE_STILL_CHAIN} each ({} at the most, {} \
         solves whose last turn was still)",
        solved.reads,
        solved.solves,
        ONE_STILL_CHAIN * solved.solves,
        solved.most,
        solved.still
    );
}

/// `·` at `points` laid round an ellipse 2,000 across 10⁹ from the origin,
/// where the floats are 64 apart: as the full search lays it
/// (`NO_STILL_CHAIN`), with its solves and how its budget left it, and as
/// it is laid.
fn ring_at_a_billion(
    points: f32,
) -> (
    PlacedRun,
    crate::flow::place::InkSolves,
    Option<crate::flow::place::ring::RingLoss>,
    PlacedRun,
) {
    use crate::document::values::Shape;
    use crate::flow::outline;
    let shape = outline(
        Rect::new(Vec2::splat(1.0e9), Vec2::splat(2_000.0)),
        Shape::Ellipse,
    );
    let (full, solved, loss) = ring_and_its_ink_solves("·", points, &shape, 1_024, true);
    let (laid, _, _) = ring_and_its_ink_solves("·", points, &shape, 1_024, false);
    (full, solved, loss, laid)
}

/// Where one search's strides may not stop early (#376, ruling B48.1): a
/// `Repeat` ring of 16-point dots round an ellipse 2,000 across 10⁹ from
/// the origin, where the floats are 64 apart: further apart than one
/// stride of a dot moves its ink, and nearer than its 64 strides do. A
/// stride that leaves the ink where it was is followed there by one that
/// moves it, and the search sets the ink on the next float, as R31 keeps
/// it where the floats allow. The full search (`NO_STILL_CHAIN`) closes
/// the ring with 128 dots, and the ring is laid as it lays it, bit for
/// bit. A search stopped at its first still stride stacked its dots,
/// about 8 to a float, and ran on to the cap.
///
/// Preconditions: every stride the ring's searches take, in its first
/// walk, its close's measures and its lay alike, is shorter than a float
/// and longer than a 64th of one (the regime's two boundaries) — the
/// longest by a margin, 60.3 of 64, in the close's measures, whose gap is
/// owed besides the bearings — and the full search closes the ring with
/// 128 dots, so that the reference cannot stack too. Controls: a search
/// stopped at its first stride that leaves the ink where it was, the
/// reference spared, red ("1024 glyphs, the full search's 128 ..."); the
/// same stop in the reference too, red ("precondition: the full search
/// closes the ring with 128 dots (1024 dots ...").
#[test]
fn test_a_repeat_ring_whose_floats_one_stride_cannot_cross_is_laid_as_the_full_search_lays_it() {
    let grain = grain_at(1.0e9);
    let (full, solved, loss, laid) = ring_at_a_billion(16.0);
    assert!(
        full.glyphs.len() == 128 && loss.is_none(),
        "precondition: the full search closes the ring with 128 dots ({} dots, {loss:?})",
        full.glyphs.len()
    );
    let (shortest, longest) = solved.strides.unwrap_or((f32::NAN, f32::NAN));
    assert!(
        longest < grain && grain < 64.0 * shortest,
        "precondition: the floats 10⁹ from the origin ({grain} apart) are further apart than the ring's \
         longest stride ({longest}) and nearer than 64 of its shortest ({shortest})"
    );
    assert!(
        laid == full,
        "{} glyphs, the full search's {}: the ring is not laid as the full search lays it",
        laid.glyphs.len(),
        full.glyphs.len()
    );
}

/// Where a still turn may not end the turns (#376, ruling B48.2): a
/// `Repeat` ring of 1-point dots round the same ellipse 10⁹ from the
/// origin, where the floats, 64 apart, are further apart than a dot's 64
/// strides move its ink and nearer than the 768 of its chain of turns do.
/// In the ring's first walk, which steps with no gap, a turn none of whose
/// strides moves the ink is followed by one whose stride does, and the
/// solve sets the ink on the next float. The full search
/// (`NO_STILL_CHAIN`) closes the ring with 128 dots, and the ring is laid
/// as it lays it, bit for bit. A solve whose turns ended at its first
/// still one stacked the first walk's dots and ran it to the cap.
///
/// The regime is the first walk's: its close's measures and its lay step
/// with the gap they solve for, which each ink is owed besides its
/// bearings, and stride further, up to a float — outside it, where one
/// stride can cross a float, for some of their searches. The property
/// bites in the first walk, whose stacking runs the ring to its cap.
///
/// Preconditions: every stride the searches of the walks with no gap take
/// is longer than a 768th of a float and shorter than a 64th (the regime's
/// two boundaries), and the full search closes the ring with 128 dots, so
/// that the reference cannot stack too. Controls: the solve's turns ended
/// at its first still one, the reference spared, which leaves the ring of
/// 16-point dots above as it is, red ("1024 glyphs, the full search's 128
/// ..."); a search stopped at its first stride that leaves the ink where
/// it was in the reference too, red ("precondition: the full search
/// closes the ring with 128 dots (1024 dots ...").
#[test]
fn test_a_repeat_ring_whose_floats_one_turn_cannot_cross_is_laid_as_the_full_search_lays_it() {
    let grain = grain_at(1.0e9);
    let (full, solved, loss, laid) = ring_at_a_billion(1.0);
    assert!(
        full.glyphs.len() == 128 && loss.is_none(),
        "precondition: the full search closes the ring with 128 dots ({} dots, {loss:?})",
        full.glyphs.len()
    );
    let (shortest, longest) = solved.gapless_strides.unwrap_or((f32::NAN, f32::NAN));
    assert!(
        64.0 * longest < grain && grain < 768.0 * shortest,
        "precondition: the floats 10⁹ from the origin ({grain} apart) are further apart than 64 of the longest \
         stride of a walk with no gap ({longest}) and nearer than 768 of its shortest ({shortest})"
    );
    assert!(
        laid == full,
        "{} glyphs, the full search's {}: the ring is not laid as the full search lays it",
        laid.glyphs.len(),
        full.glyphs.len()
    );
}

/// What #376 leaves (`ink_gap_at`, `ink_strides`): a stride that moves the
/// ink is taken, however many there are, and a turn after one is read
/// whole, so the most one glyph may read its path (`CLUSTER_READS`) does
/// not tighten. `·:·` at 16 points round a sharp rectangle 1 × 3, outward:
/// the ring places one glyph, and the walks that measure and probe it
/// solve the ink of a dot three times. In each, the dot's ink circles the
/// rectangle within its owed distance of the dot's before, every stride
/// moving it and none far enough, so each of `INK_TURNS` turns takes the
/// gap at its center, its guess and every one of `INK_STRIDES` strides,
/// three reads a point — 12 × (2 + 64) × 3 = 2,376 reads a solve, every
/// turn and stride `CLUSTER_READS` counts.
///
/// Precondition: the ring's walks solve an ink. Control: a search stopped
/// at its first stride that leaves the ink's `x` where it was, which a
/// stride up the rectangle's side does, red ("27 path reads for 3 solves
/// of a glyph's ink, not 7128 ..."); `CLUSTER_READS` without its strides,
/// red ("2376 path reads for one solve of a glyph's ink, past the 946 a
/// cluster may take").
#[test]
fn test_a_repeat_ring_whose_strides_move_its_ink_takes_every_stride() {
    use crate::document::values::{Percent, Shape};
    use crate::flow::outline;
    use crate::flow::place::ring::CLUSTER_READS;
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(1.0, 3.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let (placed, solved, _) = ring_and_its_ink_solves("·:·", 16.0, &shape, 1_024, false);
    assert!(
        solved.solves >= 1,
        "precondition: the ring's walks solve an ink ({} placed, {} solves)",
        placed.glyphs.len(),
        solved.solves
    );
    // `INK_TURNS` turns, each a point for its gap, one for its guess and
    // one for each of `INK_STRIDES` strides, three reads a point.
    let every_stride = 12 * (2 + 64) * 3;
    assert_eq!(
        (solved.reads, solved.most),
        (every_stride * solved.solves, every_stride),
        "{} path reads for {} solves of a glyph's ink, not {}, every stride of every turn, {every_stride} each",
        solved.reads,
        solved.solves,
        every_stride * solved.solves
    );
    assert!(
        solved.most <= CLUSTER_READS,
        "{} path reads for one solve of a glyph's ink, past the {CLUSTER_READS} a cluster may take",
        solved.most
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
/// `test_a_spread_along_a_line_as_long_as_placement_lays_places_every_glyph`
/// with the limit held at half what it is.
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
/// timeout, at most 150 glyphs (152 since #304, 151 since #313); the
/// replay lays its text as it was found,
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

/// The flow_structured_every_face finding of CI's run on #354 at
/// c0268bfa, after 2,171 runs (#313, ruling B18.1 §9): a far-out frame, a
/// square 800 across with 24 rounded corners at (10¹⁰, 10¹⁰), where `f32`s
/// are 1,024 apart, round which a `Spread` of 8 glyphs 4.8 long placed
/// one, its line cut. The frame's room, its own halves' level spans, is
/// 1,024, so the text's natural length fits it; but no two positions
/// nearer than 1,024 exist there, so its inks cannot be parted, and the
/// frame cuts its line and says so (truncated). The property owed it
/// whole. The oracle now owes a placement whole only where the
/// coordinates hold its glyphs apart (`coordinates_hold_apart`), and holds
/// a frame's cut reported wherever it cuts (`frame_reports_its_cut`).
///
/// Controls: the grain condition taken out, red with CI's message
/// ("around an outline: its natural length fits, and it was cut"); the
/// frame's placement left untruncated where its line is cut, red ("…, and
/// the frame does not say it was cut").
#[test]
fn test_flow_structured_every_face_finding_a_far_out_frame_its_floats_cannot_part() {
    use crate::flow::tests::flow_fuzz::{LAST_FAR_FRAME, LAST_FRAME_LAID};
    use crate::flow::{Outline, Rect};
    let found = include_bytes!("fuzz_findings/structured_frame_far_out_its_floats_cannot_part");
    LAST_FAR_FRAME.with(|last| last.set(None));
    LAST_FRAME_LAID.with(|last| last.set(None));
    if let Err(why) = crate::flow::tests::flow_fuzz::check_flow_structured_every_face(found) {
        panic!("{why}");
    }
    let far = Outline::RoundedRect {
        rect: Rect::new(Vec2::splat(1e10), Vec2::splat(800.0)),
        radius: 24.0,
    };
    assert_eq!(
        LAST_FAR_FRAME.with(std::cell::Cell::get),
        Some(far),
        "the finding draws a far-out frame smaller than its floats' spacing"
    );
    let Some((outline, placed, glyphs, truncated)) = LAST_FRAME_LAID.with(std::cell::Cell::get) else {
        panic!("the finding lays no frame");
    };
    assert_eq!(outline, far, "precondition: the frame laid last is the far one");
    assert!(
        placed < glyphs && truncated,
        "the far frame places {placed} of {glyphs}, cut and saying so: truncated {truncated}"
    );
}

/// The ornament runs the review found cut round a bend (review
/// 5904164019) and the oracle's sweep finds besides: owed whole by a bound
/// that sums each cluster's step and no more — its advance and ink height,
/// a space its advance — and cut, reporting nothing, because round a bend
/// the engine moves a glyph on until its ink is as far from the last as a
/// line sets it, and the arc that takes outruns the straight gap. The
/// oracle measures an ornament with its path's curvature
/// (`ornament_end_along`), and owes none of them.
///
/// Control: the oracle's whole account of the bend taken out — each glyph
/// set at its natural center, with no window and no bound from the table's
/// spacing — red on each. Each part alone taken out stays green: the
/// table's spacing at their curvature, a segment beyond `π/κ`, already
/// refuses them (ruling B18.1 §7, as amended).
#[test]
fn test_an_ornament_round_a_bend_is_owed_whole_only_where_its_moves_fit() {
    use crate::document::values::FitMode;
    use crate::flow::place::{place_on_open_capped, Trace};
    use crate::flow::tests::flow_fuzz::{curve_room, owes_whole_along};
    use crate::flow::Curve;
    let cases = [
        (
            "· · · ·",
            16.0,
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(6.0, 40.0),
                Vec2::new(14.0, -40.0),
                Vec2::new(20.0, 0.0),
            ),
            FitMode::Spread,
        ),
        (
            "· · · ·",
            8.0,
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(3.0, 20.0),
                Vec2::new(7.0, -20.0),
                Vec2::new(10.0, 0.0),
            ),
            FitMode::Spread,
        ),
        // The review's loop 16 long (review 5905087617).
        (
            "· · · ·",
            8.0,
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(19.2, 12.8),
                Vec2::new(-3.2, 12.8),
                Vec2::new(16.0, 0.0),
            ),
            FitMode::Spread,
        ),
    ];
    with_fonts(|fs| {
        for (text, points, curve, fit) in &cases {
            let flow = style(*points);
            let run = real_run(text, &flow, fs);
            let what = format!("{text:?} at {points} pt, {fit:?}, along {curve:?}");
            // Each cluster's step summed, the bound a straight path keeps.
            let straight: f32 = run
                .glyphs
                .iter()
                .map(|g| {
                    g.advance
                        + g.ink
                            .filter(|_| !g.blank)
                            .map_or(0.0, |ink| ink.max.y - ink.min.y)
                })
                .sum();
            let mut trace = Trace::default();
            let (_, placed) = place_on_open_capped(&run, curve, fit, 0.0, &flow, fs, 154, Some(&mut trace));
            assert!(
                straight <= curve_room(curve) && placed.glyphs.len() < run.glyphs.len() && trace.ring.is_none(),
                "precondition: {what} is owed whole by its steps summed, {straight} in {}, and is cut ({} of {}), reporting nothing",
                curve_room(curve),
                placed.glyphs.len(),
                run.glyphs.len()
            );
            assert!(
                !owes_whole_along(&run, fit, curve),
                "{what}: owed whole, and cut ({} of {} placed)",
                placed.glyphs.len(),
                run.glyphs.len()
            );
        }
    });
}

/// A `Fit` reaches a scale its text fits at whenever one in its window
/// does (#313, ruling B18.1 §10): texts and ornaments along a line and
/// curves that bend, at lengths from a few glyphs to many, `Fit` from 0.5
/// to 2 — 300 placements, 199 owed whole by the oracle (`owes_whole_along`:
/// its text at the window's least fits, a measure with no engine code),
/// each laid whole, untruncated and with no loss. The review's "━━━━"
/// at 16 pt round a 40-long arch is among them: its arithmetic measure
/// set it at 1.656, a hair too large for its readable lay, and the solve,
/// reading a cut walk as only just overrunning, shrank a thousandth a
/// round and ran out of rounds cut at 1.61, silent.
///
/// Control: the bracket taken out — the shrink alone, a cut walk read as
/// just overrunning — red on the review's case (cut, its rounds run out).
#[test]
fn test_a_fit_reaches_a_scale_that_fits_whenever_one_does() {
    use crate::document::values::FitMode;
    use crate::flow::Curve;
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let (mut owed, mut laid, mut cut, mut review) = (0usize, 0usize, Vec::new(), false);
    for text in ["MANDALA", "To see a World", "━━━━", "· · · ·", "─═─═", "x"] {
        for points in [8.0, 16.0] {
            for length in [10.0_f32, 20.0, 40.0, 100.0, 400.0] {
                let to = |x: f32, y: f32| Vec2::new(x * length, y * length);
                let curves = [
                    Curve::Line(Vec2::ZERO, to(0.8, 0.6)),
                    Curve::Cubic(Vec2::ZERO, to(0.0, 0.5), to(0.5, 0.5), to(0.5, 0.0)),
                    Curve::Cubic(Vec2::ZERO, to(1.2, 0.8), to(-0.2, 0.8), to(1.0, 0.0)),
                    Curve::Cubic(Vec2::ZERO, to(0.3, 2.0), to(0.7, -2.0), to(1.0, 0.0)),
                    Curve::Quadratic(Vec2::ZERO, to(0.5, 1.0), to(1.0, 0.0)),
                ];
                for curve in &curves {
                    laid += 1;
                    let o = owed_along(text, points, curve, &fit);
                    if !o.grained {
                        continue;
                    }
                    owed += 1;
                    review |= text == "━━━━"
                        && points == 16.0
                        && length == 40.0
                        && matches!(curve, Curve::Cubic(_, c, _, _) if c.x == 0.0);
                    if o.placed < o.glyphs || o.lost {
                        cut.push(format!(
                            "{text:?} {points} pt {length} {curve:?}: {} of {}, lost {}",
                            o.placed, o.glyphs, o.lost
                        ));
                    }
                }
            }
        }
    }
    assert_eq!(laid, 300, "every placement of the sweep is laid");
    assert!(review, "precondition: the review's case is owed whole");
    assert!(owed >= 100, "precondition: {owed} of {laid} owed whole");
    assert!(
        cut.is_empty(),
        "{} owed whole and not laid whole: {:?}",
        cut.len(),
        &cut[..cut.len().min(5)]
    );
}

/// A `Fit` no scale of whose window fits is laid at the window's least
/// and cut there (#313, ruling B18.1 §10): the cut is its window's, not
/// its solve's, so it reports the text truncated and no loss. "━━━━" at
/// 16 pt round a 40-long arch, from 2.5 to 3 and from 1.7 to 2, where its
/// measure finds even the least too long and the solve begins there. (It
/// lays whole at 1.648 where its window reaches that low.)
///
/// Control: a cut at the least reported as a scale not solved, red.
#[test]
fn test_a_fit_no_scale_of_whose_window_fits_is_laid_at_its_least() {
    use crate::document::values::FitMode;
    use crate::flow::place::{place_on_open_capped, Trace};
    use crate::flow::tests::flow_fuzz::owes_whole_along;
    use crate::flow::Curve;
    let curve = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(0.0, 20.0),
        Vec2::new(20.0, 20.0),
        Vec2::new(20.0, 0.0),
    );
    with_fonts(|fs| {
        let flow = style(16.0);
        let run = real_run("━━━━", &flow, fs);
        for (min, max) in [(2.5_f32, 3.0_f32), (1.7, 2.0)] {
            let fit = FitMode::Fit { min, max };
            assert!(
                !owes_whole_along(&run, &fit, &curve),
                "precondition: {fit:?}: no scale of the window is owed"
            );
            let mut trace = Trace::default();
            let (rescaled, placed) =
                place_on_open_capped(&run, &curve, &fit, 0.0, &flow, fs, 154, Some(&mut trace));
            let sizes: Vec<f32> = rescaled
                .as_ref()
                .map(|r| r.glyphs.iter().map(|g| g.size).collect())
                .unwrap_or_default();
            assert!(
                placed.truncated && placed.glyphs.len() < run.glyphs.len(),
                "{fit:?}: cut and truncated: {} of {}",
                placed.glyphs.len(),
                run.glyphs.len()
            );
            assert_eq!(
                trace.ring, None,
                "{fit:?}: a cut at the window's least is no loss"
            );
            assert!(
                !sizes.is_empty() && sizes.iter().all(|&size| (size - 16.0 * min).abs() < 1e-3),
                "{fit:?}: laid at the window's least, {} pt: {sizes:?}",
                16.0 * min
            );
        }
    });
}

/// `text` at `points` laid in `fit` along `curve`, at its own budget: the
/// glyphs placed and the run's, whether it is truncated, its loss, and the
/// size its first glyph was laid at.
fn fit_laid(
    text: &str,
    points: f32,
    curve: &crate::flow::Curve,
    fit: &crate::document::values::FitMode,
) -> (
    usize,
    usize,
    bool,
    Option<crate::flow::place::ring::RingLoss>,
    f32,
) {
    use crate::flow::place::{place_on_open_capped, Trace};
    with_fonts(|fs| {
        let flow = style(points);
        let run = real_run(text, &flow, fs);
        let mut trace = Trace::default();
        let (rescaled, placed) =
            place_on_open_capped(&run, curve, fit, 0.0, &flow, fs, 154, Some(&mut trace));
        let size = rescaled
            .as_ref()
            .unwrap_or(&run)
            .glyphs
            .first()
            .map_or(0.0, |g| g.size);
        (
            placed.glyphs.len(),
            run.glyphs.len(),
            placed.truncated,
            trace.ring,
            size,
        )
    })
}

/// [`fit_laid`], and the glyph steps the placement took in all.
fn fit_laid_spent(
    text: &str,
    points: f32,
    curve: &crate::flow::Curve,
    fit: &crate::document::values::FitMode,
) -> (
    usize,
    bool,
    Option<crate::flow::place::ring::RingLoss>,
    f32,
    usize,
) {
    use crate::flow::place::{place_on_open_capped, Trace};
    with_fonts(|fs| {
        let flow = style(points);
        let run = real_run(text, &flow, fs);
        let mut trace = Trace::default();
        let (rescaled, placed) =
            place_on_open_capped(&run, curve, fit, 0.0, &flow, fs, 154, Some(&mut trace));
        let size = rescaled
            .as_ref()
            .unwrap_or(&run)
            .glyphs
            .first()
            .map_or(0.0, |g| g.size);
        (
            placed.glyphs.len(),
            placed.truncated,
            trace.ring,
            size,
            trace.spent,
        )
    })
}

/// A `Fit` whose bracket closes on its window's least with no round found
/// to fit lays one round there (#313, ruling B18.1 §10.2 as amended,
/// review 5905087617): "──────" at 8 pt along a steep line 24 long, `Fit
/// {0.5, 2}`, its shrink given one round (`FIT_ROUNDS_OVERRIDE`), so that
/// its bracket begins from the least: every scale it tries above the least
/// is cut, and the least, laid, is whole — 4 pt, no loss. No case reaches
/// that round without the seam: the shrink reaches the least itself before
/// the bracket can begin there, on every `Fit` of the review's grids.
///
/// Control: the round at the least taken out, red (laid a thousandth above
/// it, reported not solved).
#[test]
fn test_a_fit_whose_bracket_closes_on_its_least_lays_a_round_there() {
    use crate::document::values::FitMode;
    use crate::flow::place::FIT_ROUNDS_OVERRIDE;
    use crate::flow::tests::halves_tests::with_seam;
    use crate::flow::Curve;
    let curve = Curve::Line(Vec2::ZERO, Vec2::new(14.4, -19.2));
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let (placed, glyphs, truncated, ring, size) =
        with_seam(&FIT_ROUNDS_OVERRIDE, 1, || fit_laid("──────", 8.0, &curve, &fit));
    assert!(
        placed == glyphs && !truncated && ring.is_none(),
        "laid whole, no loss: {placed} of {glyphs}, {ring:?}"
    );
    assert_eq!(size, 4.0, "laid at its least");
}

/// The review's `Fit`s at or near their window's least (#313, ruling
/// B18.1 §10.2 as amended, reviews 5905087617 and 5905947478), each laid
/// whole with no loss:
///
/// - "━━━━" at 16 pt round an arch 6.2 across, `Fit {0.5, 2}`: cut at its
///   least alone, and laid whole a little above it, at 8.31 pt;
/// - "──────" at 8 pt along a steep line 24 long, `Fit {0.5, 2}`: laid at
///   exactly its least, 4 pt, as its least alone lays it;
/// - "━━━━" at 8 pt along a steep line 2.25 long, `Fit {0.001, 8}`, and
///   "─═─═" at 8 pt along one 2.5 long, `Fit {0.01, 4}`: windows whose `min`
///   lies below the scale at which every glyph reaches the size floor,
///   where a smaller scale shrinks nothing and a stop is the window's.
///
/// Control: the window's least back at its `min` and the bracket taken
/// out, red (the wide windows laid whole and reported not solved, as the
/// review found them). Either alone stays green: the least stops the
/// shrink at the floor, and the bracket takes over a shrink that runs out.
#[test]
fn test_a_fit_whose_bracket_closes_on_its_least_is_laid_there() {
    use crate::document::values::FitMode;
    use crate::flow::Curve;
    let cases = [
        (
            "━━━━",
            16.0,
            Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(0.0, 6.2),
                Vec2::new(6.2, 6.2),
                Vec2::new(6.2, 0.0),
            ),
            FitMode::Fit { min: 0.5, max: 2.0 },
        ),
        (
            "──────",
            8.0,
            Curve::Line(Vec2::ZERO, Vec2::new(14.4, -19.2)),
            FitMode::Fit { min: 0.5, max: 2.0 },
        ),
        (
            "━━━━",
            8.0,
            Curve::Line(Vec2::ZERO, Vec2::new(1.35, -1.8)),
            FitMode::Fit { min: 0.001, max: 8.0 },
        ),
        (
            "─═─═",
            8.0,
            Curve::Line(Vec2::ZERO, Vec2::new(1.5, -2.0)),
            FitMode::Fit { min: 0.01, max: 4.0 },
        ),
    ];
    for (text, points, curve, fit) in &cases {
        let what = format!("{text:?} at {points} pt, {fit:?}, along {curve:?}");
        let (placed, glyphs, truncated, ring, _) = fit_laid(text, *points, curve, fit);
        assert!(
            placed == glyphs && !truncated && ring.is_none(),
            "{what}: {placed} of {glyphs}, truncated {truncated}, {ring:?}"
        );
    }
    let least = FitMode::Fit { min: 0.5, max: 0.5 };
    let (placed, glyphs, _, _, _) = fit_laid("━━━━", 16.0, &cases[0].2, &least);
    assert!(placed < glyphs, "precondition: the arch's least alone cuts it");
    let (_, _, _, _, size) = fit_laid("──────", 8.0, &cases[1].2, &cases[1].3);
    assert_eq!(size, 4.0, "the steep line laid at exactly its least");
}

/// A `Fit` in a window far wider than its text can use reports no loss
/// where none is taken (#313, ruling B18.1 §10.2 as amended, review
/// 5905947478): texts and ornaments at 8 and 16 pt along lines, arches,
/// loops and a quadratic, from a glyph or two long to many, in `Fit
/// {0.001, 8}` and `Fit {0.01, 4}`, whose `min` lies below the scale at
/// which every glyph reaches the size floor. A smaller scale shrinks
/// nothing there, so a stop at that scale is the window's: none reports a
/// loss, whole or cut, and each is solved exactly as the window whose
/// `min` is that scale solves it — the same glyphs, at the same size,
/// for the same work. 1,920 placements, 361 of them at the floor.
///
/// Control: the window's least back at its `min`, red (109 solved unlike
/// the floor's window: the same placement, for more work, its shrink's
/// rounds spent below the floor).
#[test]
fn test_a_fit_in_a_window_wider_than_its_text_reports_no_loss() {
    use crate::document::values::FitMode;
    use crate::flow::Curve;
    let (mut laid, mut floored, mut lost, mut unlike) = (0usize, 0usize, Vec::new(), Vec::new());
    for text in ["━━━━", "─═─═", "· · · ·", "MANDALA", "ab"] {
        for points in [8.0_f32, 16.0] {
            for i in 0..24 {
                let length = 1.5 * 100f32.powf(i as f32 / 23.0);
                let to = |x: f32, y: f32| Vec2::new(x * length, y * length);
                let curves = [
                    Curve::Line(Vec2::ZERO, to(0.6, -0.8)),
                    Curve::Cubic(Vec2::ZERO, to(0.0, 0.5), to(0.5, 0.5), to(0.5, 0.0)),
                    Curve::Cubic(Vec2::ZERO, to(1.2, 0.8), to(-0.2, 0.8), to(1.0, 0.0)),
                    Curve::Quadratic(Vec2::ZERO, to(0.5, 1.0), to(1.0, 0.0)),
                ];
                for curve in &curves {
                    for (min, max) in [(0.001_f32, 8.0_f32), (0.01, 4.0)] {
                        laid += 1;
                        let fit = FitMode::Fit { min, max };
                        let laid_at = fit_laid_spent(text, points, curve, &fit);
                        floored += usize::from(laid_at.3 <= crate::font::fonts::MIN_FONT_SIZE_PT);
                        if laid_at.2.is_some() {
                            lost.push(format!(
                                "{text:?} {points} pt {fit:?} along {curve:?}: {laid_at:?}"
                            ));
                        }
                        // As the window whose least is the floor's scale.
                        let at_floor = FitMode::Fit {
                            min: crate::font::fonts::MIN_FONT_SIZE_PT / points,
                            max,
                        };
                        let floor = fit_laid_spent(text, points, curve, &at_floor);
                        if laid_at != floor {
                            unlike.push(format!("{text:?} {points} pt {fit:?} along {curve:?}: {laid_at:?}, at the floor {floor:?}"));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(laid, 1_920, "every placement is laid");
    assert!(floored >= 100, "precondition: {floored} laid at the size floor");
    assert!(
        lost.is_empty(),
        "{} reported a loss: {:?}",
        lost.len(),
        &lost[..lost.len().min(5)]
    );
    assert!(
        unlike.is_empty(),
        "{} solved unlike the floor's window: {:?}",
        unlike.len(),
        &unlike[..unlike.len().min(3)]
    );
}

/// The solve lays no smaller than its shrink laid the run whole (#313,
/// ruling B18.1 §10.5, reviews 5905947478 and 5906753939): where the
/// shrink lays the run whole after a round that overran, the bracket
/// begins from that scale and moves only up. "━━━━" and "To see a World"
/// at 8, 16 and 32 pt round arches and loops 2 to 400 across, in
/// `Fit {0.5, 2}`, `{0.9, 1.1}` and `{0.25, 4}` — the review's grid where
/// a bracket bisecting up from the least settled on a lower island of
/// fitting scales than the shrink had reached. Each placement's shrink
/// records the scale it laid the run whole at as its bracket began
/// (`place::SHRINK_FIT`); the run is laid whole at a scale no smaller.
///
/// Control: the bracket bisecting down from a round that fits rather than
/// up, red.
#[test]
fn test_a_fit_lays_no_smaller_than_the_shrink_did() {
    use crate::document::values::FitMode;
    use crate::flow::place::SHRINK_FIT;
    use crate::flow::Curve;
    let (mut bracketed, mut larger, mut smaller) = (0usize, 0usize, Vec::new());
    for text in ["━━━━", "To see a World"] {
        for points in [8.0_f32, 16.0, 32.0] {
            for i in 0..150 {
                let length = 2.0 * 200f32.powf(i as f32 / 149.0);
                let to = |x: f32, y: f32| Vec2::new(x * length, y * length);
                let curves = [
                    Curve::Cubic(Vec2::ZERO, to(0.0, 0.5), to(0.5, 0.5), to(0.5, 0.0)),
                    Curve::Cubic(Vec2::ZERO, to(1.2, 0.8), to(-0.2, 0.8), to(1.0, 0.0)),
                ];
                for curve in &curves {
                    for (min, max) in [(0.5_f32, 2.0_f32), (0.9, 1.1), (0.25, 4.0)] {
                        let fit = FitMode::Fit { min, max };
                        let (placed, glyphs, truncated, ring, size) = fit_laid(text, points, curve, &fit);
                        let Some(shrink) = SHRINK_FIT.with(std::cell::Cell::get) else {
                            continue;
                        };
                        bracketed += 1;
                        let at = shrink * points;
                        if placed < glyphs || truncated || ring.is_some() || size < at * (1.0 - 1e-6) {
                            smaller.push(format!(
                                "{text:?} {points} pt {fit:?} along {curve:?}: the shrink whole at {at} pt, laid {placed} of {glyphs} at {size} pt, {ring:?}"
                            ));
                        }
                        larger += usize::from(size > at * (1.0 + 1e-6));
                    }
                }
            }
        }
    }
    assert!(
        bracketed >= 500,
        "precondition: {bracketed} brackets began from a whole round"
    );
    assert!(
        smaller.is_empty(),
        "{} laid smaller or cut: {:?}",
        smaller.len(),
        &smaller[..smaller.len().min(5)]
    );
    assert!(larger >= 500, "{larger} laid larger than their shrink");
}

/// The invariant the bracket must keep (#313, ruling B18.1 §10.2, review
/// 5905087617): wherever a `Fit` from 0.5 to 2 is laid within a thousandth
/// of its window's least — no scale above found to fit — it is laid at
/// exactly that least, as `Fit {0.5, 0.5}` lays it, with no loss. Texts
/// and ornaments along lines at eight angles and the review's, lengths
/// from a glyph or two to many, and along arches: 240 placements, 155 laid
/// at their least, every one as its least alone lays it.
///
/// Control: the bracket begun at the shrink's first cut round, from the
/// least, with no round laid at the least when it closes — db952d6c's
/// shape — red (3: the review's line and arch, laid a thousandth above the
/// least and reported not solved). The round at the least alone taken out
/// stays green here, since the shrink reaches the least itself before a
/// bracket can begin there; it is held by
/// `test_a_fit_whose_bracket_closes_on_its_least_lays_a_round_there`.
#[test]
fn test_a_fit_laid_at_its_least_is_laid_as_its_least_alone() {
    use crate::document::values::FitMode;
    use crate::flow::Curve;
    let window = FitMode::Fit { min: 0.5, max: 2.0 };
    let least = FitMode::Fit { min: 0.5, max: 0.5 };
    let (mut laid, mut at_least, mut wrong) = (0usize, 0usize, Vec::new());
    for (text, points) in [
        ("──────", 8.0),
        ("━━━━", 16.0),
        ("MANDALA", 8.0),
        ("To see a World", 16.0),
    ] {
        for length in [6.0_f32, 6.2, 12.0, 24.0, 48.0, 96.0] {
            let mut curves: Vec<Curve> = (0..8)
                .map(|k| {
                    let angle = k as f32 * std::f32::consts::TAU / 8.0 + 0.3;
                    Curve::Line(Vec2::ZERO, Vec2::new(angle.cos(), angle.sin()) * length)
                })
                .collect();
            // The review's line (review 5905087617).
            curves.push(Curve::Line(Vec2::ZERO, Vec2::new(0.6, -0.8) * length));
            curves.push(Curve::Cubic(
                Vec2::ZERO,
                Vec2::new(0.0, length),
                Vec2::new(length, length),
                Vec2::new(length, 0.0),
            ));
            for curve in &curves {
                laid += 1;
                let (placed, _, truncated, ring, size) = fit_laid(text, points, curve, &window);
                if size > 0.5 * points * (1.0 + 2e-3) {
                    continue;
                }
                at_least += 1;
                let alone = fit_laid(text, points, curve, &least);
                if (placed, truncated, ring, size) != (alone.0, alone.2, alone.3, alone.4) {
                    wrong.push(format!(
                        "{text:?} {points} pt along {curve:?}: {placed} glyphs, truncated {truncated}, {ring:?}, {size} pt; alone {:?}",
                        (alone.0, alone.2, alone.3, alone.4)
                    ));
                }
            }
        }
    }
    assert_eq!(laid, 240, "every placement is laid");
    assert!(at_least >= 20, "precondition: {at_least} laid at their least");
    assert!(
        wrong.is_empty(),
        "{} not laid as their least alone: {:?}",
        wrong.len(),
        &wrong[..wrong.len().min(5)]
    );
}

/// A `Fit` whose rounds run out with its text still cut above its window's
/// least reports its scale not solved, `Uneven` (#313, ruling B18.1 §10.2
/// as amended, review 5906753939): "━━━━" at 16 pt round the 40-long arch
/// is cut at every scale from 1.612 to 1.642, so `Fit {1.615, 1.64}` lies
/// wholly in that island. Given its rounds, its shrink reaches the least
/// and lays it cut there, the window's stop, no loss. Given one round of
/// its shrink (`FIT_ROUNDS_OVERRIDE`) it runs out cut above the least, and
/// the bracket bisects up from the least; given two rounds of that
/// (`BRACKET_ROUNDS_OVERRIDE`), both are cut — 1.6275, then 1.62125 — and
/// they run out with no round found to fit: laid cut there, and said so.
/// No case reaches the report without the seams — none of 40,000 `Fit`s
/// along random cubics, of 126,000 in the review's grids, of the sweeps
/// here, nor of 8,640 at 16, 256 and 4,096 pt in windows as wide as
/// `{0.0001, 50}` — so the report is defensive: a readable lay that kept
/// overrunning could run its rounds out, and would say so.
///
/// Control: the report taken out, red.
#[test]
fn test_a_fit_whose_rounds_run_out_cut_reports_its_scale_not_solved() {
    use crate::document::values::FitMode;
    use crate::flow::place::ring::RingLoss;
    use crate::flow::place::{BRACKET_ROUNDS_OVERRIDE, FIT_ROUNDS_OVERRIDE};
    use crate::flow::tests::halves_tests::with_seam;
    use crate::flow::Curve;
    let curve = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(0.0, 20.0),
        Vec2::new(20.0, 20.0),
        Vec2::new(20.0, 0.0),
    );
    let (least, points) = (1.615_f32, 16.0_f32);
    let fit = FitMode::Fit {
        min: least,
        max: 1.64,
    };
    let (placed, glyphs, _, ring, size) = fit_laid("━━━━", points, &curve, &fit);
    assert!(
        placed < glyphs && ring.is_none() && (size - least * points).abs() < 1e-3,
        "precondition: given its rounds, cut at its least with no loss: {placed} of {glyphs} at {size} pt, {ring:?}"
    );
    let (placed, glyphs, truncated, ring, size) = with_seam(&FIT_ROUNDS_OVERRIDE, 1, || {
        with_seam(&BRACKET_ROUNDS_OVERRIDE, 2, || {
            fit_laid("━━━━", points, &curve, &fit)
        })
    });
    assert!(
        placed < glyphs && truncated && size > least * points * (1.0 + 1e-3),
        "precondition: its bracket's last round is cut above its least: {placed} of {glyphs} at {size} pt"
    );
    assert_eq!(
        ring,
        Some(RingLoss::Uneven),
        "its rounds run out cut: its scale not solved"
    );
}

/// Every `Spread` and `Fit` along a far-out path (#313) that its natural
/// length fits is laid whole (review 5902086644, ruling B18.1 §1 as
/// amended, and §9): three texts and two ornaments in both modes along
/// `far_curve`'s line and cubic at every `far_out` draw, 10⁶ to 10²² from
/// the origin and 100 to 204,800 long, 4,080 placements. The floats far
/// out round a path's control points, so a path is often not as long as
/// it was drawn, and is no length at all where its length is below their
/// spacing; each is owed whole on the room of the path it is
/// ([`curve_room`], an oracle independent of the engine's arithmetic),
/// against its natural length along it (ruling B18.1 §7), where the
/// coordinates hold its glyphs apart (§9): 1,260 fit, and 394 of them are
/// so held — 226 texts, 168 ornaments — 197 on paths the floats changed,
/// the case swept for. The grain gives up the other 866: of the 760 texts
/// that fit, it keeps 226, and of the 500 ornaments, 168.
///
/// Control: with `curve_room` finding its points by `Curve::point` in
/// `f32` again, red (1 placement owed whole and cut, rounding read as
/// room: "MANDALA" in `Spread` along a cubic 100 long at 10⁷, given 205.8;
/// 1,236 before the grain held out what its coordinates cannot part, and
/// 695 when the engine's own tables overmeasured too).
#[test]
fn test_far_out_open_placements_their_natural_length_fits_are_laid_whole() {
    use crate::document::values::FitMode;
    use crate::flow::place::{place_on_open_capped, Trace};
    use crate::flow::tests::flow_fuzz::{
        coordinates_hold_apart, curve_reach, curve_room, far_curve, owes_whole_along,
    };
    let (mut owed, mut moved, mut cut) = (0usize, 0usize, Vec::new());
    let (mut along, mut ornaments) = (0usize, 0usize);
    with_fonts(|fs| {
        let flow = style(16.0);
        for text in ["MANDALA", "To see a World", "ab", "━━━━", "· · · ·"] {
            let run = real_run(text, &flow, fs);
            for fit in [FitMode::Spread, FitMode::Fit { min: 0.5, max: 2.0 }] {
                for k in 6..=22 {
                    for doubled in 0..12 {
                        for line in [true, false] {
                            let length = 100.0 * (1u32 << doubled) as f32;
                            let curve = far_curve(Vec2::splat(10f32.powi(k)), length, line);
                            let room = curve_room(&curve);
                            if !owes_whole_along(&run, &fit, &curve) {
                                continue;
                            }
                            along += 1;
                            if !coordinates_hold_apart(curve_reach(&curve), &run, &fit) {
                                continue;
                            }
                            owed += 1;
                            ornaments += usize::from(!run.glyphs.iter().any(|g| g.letter));
                            if (room - length).abs() > length * 1e-3 {
                                moved += 1;
                            }
                            let mut trace = Trace::default();
                            let (_, placed) = place_on_open_capped(
                                &run,
                                &curve,
                                &fit,
                                0.0,
                                &flow,
                                fs,
                                154,
                                Some(&mut trace),
                            );
                            if placed.glyphs.len() < run.glyphs.len() {
                                cut.push(format!(
                                    "{text:?} {fit:?} 10^{k} {length} line {line}: room {room}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    });
    assert!(
        cut.is_empty(),
        "{} owed whole and cut: {:?}",
        cut.len(),
        &cut[..cut.len().min(5)]
    );
    assert!(owed >= 350, "the sweep owes {owed} placements whole");
    assert!(
        moved >= 150,
        "the sweep owes {moved} placements whole on paths the floats changed"
    );
    assert!(
        ornaments >= 100,
        "the sweep owes {ornaments} ornament placements whole"
    );
}

/// One placement along an open path as the oracle reads it
/// ([`owed_along`]).
struct Owed {
    /// The glyphs placed, and the run's.
    placed: usize,
    glyphs: usize,
    /// Whether it reported a loss.
    lost: bool,
    /// Whether it is owed whole along a level line's natural length
    /// (`owes_whole`), along the path whatever way it runs
    /// (`owes_whole_along`), and that where the coordinates also hold its
    /// glyphs apart (`coordinates_hold_apart`, ruling B18.1 §9).
    level: bool,
    along: bool,
    grained: bool,
}

/// Its text at `points` along `curve` in `fit`, laid, and read by the
/// oracle ([`Owed`]).
fn owed_along(
    text: &str,
    points: f32,
    curve: &crate::flow::Curve,
    fit: &crate::document::values::FitMode,
) -> Owed {
    use crate::flow::place::{place_on_open_capped, Trace};
    use crate::flow::tests::flow_fuzz::{
        coordinates_hold_apart, curve_reach, curve_room, owes_whole, owes_whole_along,
    };
    with_fonts(|fs| {
        let flow = style(points);
        let run = real_run(text, &flow, fs);
        let mut trace = Trace::default();
        let (_, placed) = place_on_open_capped(&run, curve, fit, 0.0, &flow, fs, 154, Some(&mut trace));
        let along = owes_whole_along(&run, fit, curve);
        Owed {
            placed: placed.glyphs.len(),
            glyphs: run.glyphs.len(),
            lost: trace.ring.is_some(),
            level: owes_whole(&run, fit, curve_room(curve)),
            along,
            grained: along && coordinates_hold_apart(curve_reach(curve), &run, fit),
        }
    })
}

/// A run along an open path is owed whole only where it fits every way
/// its path may run (#313, ruling B18.1 §7, review 5903692895): the
/// review's five placements, four of `far_out`'s own lines at 10⁹ — whose
/// rounding of `0.8·L, 0.6·L` turns the line toward 45° — and one at the
/// origin along (0.8, 0.6), each owed whole by a level line's natural
/// length, the longer of each glyph's advance and the line height, and
/// each cut by a glyph: a text's box steps its diagonal there, more than
/// either. Along any way, the box's diagonal a cluster, none is owed.
///
/// Control: the open path's oracle measuring a level line's natural
/// length again, red on each.
#[test]
fn test_an_open_path_owes_a_run_whole_only_where_it_fits_every_way() {
    use crate::document::values::FitMode;
    use crate::flow::tests::flow_fuzz::far_curve;
    use crate::flow::Curve;
    let far = Vec2::splat(1e9);
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let cases = [
        ("MANDALA", 48.0, far_curve(far, 400.0, true), FitMode::Spread),
        ("MANDALA", 96.0, far_curve(far, 400.0, true), fit),
        ("MANDALA", 96.0, far_curve(far, 800.0, true), FitMode::Spread),
        ("mmmmmmmm", 8.0, far_curve(far, 100.0, true), FitMode::Spread),
        (
            "MANDALA",
            4.0,
            Curve::Line(Vec2::ZERO, Vec2::new(0.8 * 37.0, 0.6 * 37.0)),
            FitMode::Spread,
        ),
    ];
    for (text, points, curve, fit) in &cases {
        let o = owed_along(text, *points, curve, fit);
        let what = format!("{text:?} at {points} pt, {fit:?}, along {curve:?}");
        assert!(
            o.level && o.placed < o.glyphs && !o.lost,
            "precondition: {what} is owed whole along a level line, and is cut ({} of {}), reporting nothing",
            o.placed,
            o.glyphs
        );
        assert!(
            !o.along,
            "{what}: owed whole, and cut ({} of {} placed)",
            o.placed, o.glyphs
        );
    }
}

/// No run owed whole along an open path is cut (#313, rulings B18.1 §7,
/// §9, §10), over the review's sweep (review 5903692895) and ornaments
/// round bends (review 5904164019):
///
/// - four texts at three sizes, 10⁰ to 10²² from the origin, along a
///   line, a quadratic, a cubic, a loop and a cubic of no length but its
///   last control point, five lengths each, in `Spread`, `Fit` and `Once`:
///   20,700 placements, 4,628 owed whole along their paths, 3,225 of them
///   where the coordinates hold their glyphs apart, none cut. A level
///   line's natural length owed 4,736, 20 of them cut: 108 placements are
///   what owing only what fits every way gives up, and 1,403 more what the
///   grain does;
/// - three ornaments at two sizes, at the same distances, along a line
///   and four curves that bend — the review's arch, its loop and its S,
///   and a hump — five lengths each, in `Spread` and `Fit`: 6,900
///   placements, 1,207 owed whole, none cut. Each cluster's step summed,
///   the bound that holds on a straight path, owed 1,693, 22 of them cut:
///   the moves round a bend the oracle now measures. (Its window read with
///   a thousandth to spare, not the margin the table's spacing sets, the
///   oracle owed 1,373.)
///
/// Controls: the open path's oracle measuring a level line's natural length
/// again, red (15 owed whole and cut, "MANDALA" at 4 pt along (0.8, 0.6) 37
/// long, at every distance its glyphs are held apart); an ornament's glyphs
/// read as set at their natural centers, with no window and no bound from
/// the table's spacing, red (1,583 owed, 22 cut, "· · · ·" at 8 pt round
/// the S 10 long among them). What each part decides here: the window
/// refuses 131 placements, the table's spacing 245; the move's own bound,
/// with the window kept, changes no verdict the grain leaves — it is a step
/// of the window's derivation, kept as such. The `Fit` solve's bracket is
/// held by `test_a_fit_reaches_a_scale_that_fits_whenever_one_does`: taken
/// out, a cut `Fit` here reports its scale not solved, which this sweep
/// reads as reported.
#[test]
fn test_an_open_path_cuts_no_run_it_owes_whole_at_any_distance() {
    use crate::document::values::FitMode;
    use crate::flow::Curve;
    // Laid, owed along the path, owed where the grain holds too, owed
    // along a level line, for texts [0] and ornaments [1]; and the cut.
    let mut counts = [[0usize; 4]; 2];
    let mut cut = Vec::new();
    let mut lay =
        |text: &str, points: f32, k: i32, length: f32, curve: &Curve, fit: &FitMode, kind: usize| {
            let o = owed_along(text, points, curve, fit);
            let c = &mut counts[kind];
            c[0] += 1;
            c[1] += usize::from(o.along);
            c[2] += usize::from(o.grained);
            c[3] += usize::from(o.level);
            if o.grained && o.placed < o.glyphs && !o.lost {
                cut.push(format!("{text:?} {points} pt 10^{k} {length} {fit:?} {curve:?}"));
            }
        };
    // Texts, along a line, a quadratic, a cubic, a loop and a cubic of no
    // length but its last control point.
    for text in ["MANDALA", "To see a World", "ab", "mmmmmmmm"] {
        for points in [4.0, 16.0, 96.0] {
            for k in 0..=22 {
                let at = Vec2::splat(10f32.powi(k));
                for length in [37.0_f32, 100.0, 400.0, 1_600.0, 6_400.0] {
                    let to = |x: f32, y: f32| at + Vec2::new(x * length, y * length);
                    let curves = [
                        Curve::Line(at, to(0.8, 0.6)),
                        Curve::Quadratic(at, to(0.5, 0.4), to(1.0, 0.0)),
                        Curve::Cubic(at, to(0.35, -0.3), to(0.65, 0.3), to(1.0, 0.0)),
                        Curve::Cubic(at, to(1.2, 0.8), to(-0.2, 0.8), to(1.0, 0.0)),
                        Curve::Cubic(at, at, at, to(1.0, 0.0)),
                    ];
                    for curve in &curves {
                        for fit in [
                            FitMode::Spread,
                            FitMode::Fit { min: 0.5, max: 2.0 },
                            FitMode::Once { filler: None },
                        ] {
                            lay(text, points, k, length, curve, &fit, 0);
                        }
                    }
                }
            }
        }
    }
    // Ornaments, along a line and curves that bend: the review's arch, its
    // loop and its S, and a hump.
    for text in ["━━━━", "· · · ·", "─═─═"] {
        for points in [8.0, 16.0] {
            for k in 0..=22 {
                let at = Vec2::splat(10f32.powi(k));
                for length in [10.0_f32, 20.0, 40.0, 100.0, 400.0] {
                    let to = |x: f32, y: f32| at + Vec2::new(x * length, y * length);
                    let curves = [
                        Curve::Line(at, to(0.8, 0.6)),
                        Curve::Cubic(at, to(0.0, 0.5), to(0.5, 0.5), to(0.5, 0.0)),
                        Curve::Cubic(at, to(1.2, 0.8), to(-0.2, 0.8), to(1.0, 0.0)),
                        Curve::Cubic(at, to(0.3, 2.0), to(0.7, -2.0), to(1.0, 0.0)),
                        Curve::Quadratic(at, to(0.5, 1.0), to(1.0, 0.0)),
                    ];
                    for curve in &curves {
                        for fit in [FitMode::Spread, FitMode::Fit { min: 0.5, max: 2.0 }] {
                            lay(text, points, k, length, curve, &fit, 1);
                        }
                    }
                }
            }
        }
    }
    let [texts, ornaments] = counts;
    assert_eq!(
        (texts[0], ornaments[0]),
        (20_700, 6_900),
        "every placement of the sweep is laid"
    );
    assert!(
        texts[2] >= 3_000 && texts[1] <= texts[3],
        "the sweep owes {} text placements whole, {} along their paths, a level line's length {}",
        texts[2],
        texts[1],
        texts[3]
    );
    assert!(
        ornaments[2] >= 1_100,
        "the sweep owes {} ornament placements whole",
        ornaments[2]
    );
    assert!(
        cut.is_empty(),
        "{} owed whole and cut: {:?}",
        cut.len(),
        &cut[..cut.len().min(5)]
    );
}

/// An open path's length is the length of the curve its control points
/// make, at every magnitude `far_out` draws and at the origin, to the
/// bound `ArcPath::open` states (#313, ruling B18.1 §6): `far_curve`'s
/// line and cubic 100 to 204,800 long from the origin and 10⁶ to 10²²
/// from it, and at each a line and a cubic whose control points coincide,
/// which measure exactly 0. Held to [`polyline_length`] through the
/// curve at the parameters the table was built at, an oracle that shares
/// no arithmetic with the engine: de Casteljau's construction where the
/// engine takes the Bernstein form, and no engine code. A Bézier's table
/// is within `2⁻²⁴·L` of that length `L`, and `2⁻⁴⁷·(R + L)` more a
/// sample, `R` its control points' reach; a line within `2⁻²²·L` of its
/// length `L`.
///
/// Controls: a Bézier's table found from its points rounded to `f32` where
/// they lie (`Param::point`'s), red already at the origin (a cubic 25,600
/// across measured 27,509.2422 against its 27,509.2401, past 0.0017), and
/// far out by orders more (a cubic 128 long 10⁹ from the origin measured
/// 20,260); its distances summed in `f32`, red at the origin (a cubic 100
/// across measured 107.457306 against its 107.457241, past 6.4·10⁻⁶).
#[test]
fn test_an_open_paths_length_is_its_curves_at_every_magnitude() {
    use crate::flow::tests::flow_fuzz::{far_curve, polyline_length};
    use crate::flow::{ArcPath, Curve};
    let mut curves = Vec::new();
    for at in std::iter::once(Vec2::ZERO).chain((6..=22).map(|k| Vec2::splat(10f32.powi(k)))) {
        for doubled in 0..12 {
            for line in [true, false] {
                curves.push(far_curve(at, 100.0 * (1u32 << doubled) as f32, line));
            }
        }
        curves.push(Curve::Line(at, at));
        curves.push(Curve::Cubic(at, at, at, at));
    }
    let (mut checked, mut far_beziers) = (0usize, 0usize);
    for curve in &curves {
        let path = ArcPath::open(curve, 1.0);
        let measured = f64::from(path.length());
        let (exact, bound) = match *curve {
            Curve::Line(a, b) => {
                let exact = (b.as_dvec2() - a.as_dvec2()).length();
                (exact, exact * 2f64.powi(-22))
            }
            Curve::Quadratic(a, ..) | Curve::Cubic(a, ..) => {
                let samples = path.table_samples();
                assert!(samples > 1, "{curve:?} is tabled");
                let exact = polyline_length(curve, samples - 1);
                let reach = match *curve {
                    Curve::Quadratic(_, c, b) => [c, b, b],
                    Curve::Cubic(_, c1, c2, b) => [c1, c2, b],
                    Curve::Line(..) => unreachable!(),
                }
                .iter()
                .map(|p| (p.as_dvec2() - a.as_dvec2()).length())
                .fold(0.0, f64::max);
                if a.x > 0.0 && exact > 0.0 {
                    far_beziers += 1;
                }
                (
                    exact,
                    exact * 2f64.powi(-24) + samples as f64 * 2f64.powi(-47) * (reach + exact),
                )
            }
        };
        if exact == 0.0 {
            assert_eq!(measured, 0.0, "{curve:?} has no length");
        }
        assert!(
            (measured - exact).abs() <= bound,
            "{curve:?} measures {measured}, its curve {exact}: past {bound}"
        );
        checked += 1;
    }
    assert_eq!(checked, 18 * 26, "every curve is checked");
    // Far out, where the absolute arithmetic overmeasured: the cubics
    // whose control points the floats there keep apart.
    assert_eq!(far_beziers, 66, "the far-out cubics that have a length to hold");
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
    use crate::flow::tests::flow_fuzz::{natural_length_any_way, owes_whole_any_way};
    use crate::font::fonts::MIN_FONT_SIZE_PT;
    let text = "To see a World";
    let (run, floored) = with_fonts(|fs| {
        (
            real_run(text, &style(16.0), fs),
            real_run(text, &style(MIN_FONT_SIZE_PT), fs),
        )
    });
    // The text shaped at the floor, measured as the oracle measures a text.
    let at_floor = natural_length_any_way(&floored, 1.0);
    let fit = FitMode::Fit { min: 1e-6, max: 1.0 };
    assert!(
        natural_length_any_way(&run, 1.0) * 1e-6 < at_floor * 0.99,
        "precondition: a millionth of the text's natural length fits a path shorter than it at the floor"
    );
    assert!(
        owes_whole_any_way(&run, &fit, at_floor * 1.01),
        "a path 1% longer than the text at the floor, {at_floor}, does not owe it whole"
    );
    assert!(
        !owes_whole_any_way(&run, &fit, at_floor * 0.99),
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
/// face, with a short side ornament, that fit: 151, 464,172 glyph steps,
/// 14.99 s; 152 take 467,221, 15.09 s (150 and 151 until #304 counted a
/// reshape at a size as no shaping; 152 and 153 until a line's lay
/// counted its path's reads past one a cluster, #313 ruling B18.1 §2).
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
/// by its first walk, red (its budget stops it instead, 7,560 glyphs
/// placed, past the cap of 149); with the cap's stop not reported, red
/// (the fuzz property: "a ring cut at 147 glyphs of 3 a copy, its cap
/// 149, reported nothing: capped silently"); with its budget scaled past
/// its cap, red (the fuzz property: 196,608,000 glyph steps budgeted for
/// 65,536 glyphs, past the 447,000 its cap of 149 gives).
///
/// Since #363 the ellipse the input drew, 3.4 × 10²¹ round, is longer
/// than placement lays, and its ring lays nothing and says why: the input
/// as found is replayed for that. What is held here is held on the same
/// input with its outline's floats shortened a thousandfold
/// (`…_within_the_limit`): an ellipse 2 by 1.7 × 10¹⁸, 10¹⁸ from the
/// origin, whose table is as large and whose floats part its inks no
/// better.
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
    let as_found = include_bytes!("fuzz_findings/structured_ring_whose_inks_its_floats_cannot_part");
    LAST_RING.with(|last| last.set(None));
    if let Err(why) = check_flow_structured_as_found(as_found) {
        panic!("the fuzzer's input as found: {why}");
    }
    assert!(
        LAST_RING.with(std::cell::Cell::get).is_none(),
        "the input as found: its outline is longer than placement lays, and no ring is laid round it"
    );
    let found =
        include_bytes!("fuzz_findings/structured_ring_whose_inks_its_floats_cannot_part_within_the_limit");
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
/// The input reached this line from #336, whose directional glyphs are
/// drawn from the choices before it, and no longer does: with the
/// discriminant `b² − a c` planted it replays green as the generator
/// stands, the planted root never compared (#384's CI finding found this).
/// Its line is held where no generator can move it, by
/// `test_flow_halves_the_tangent_line_root_holds_where_little_is_owed_of_a_wide_gap`,
/// which that plant turns red ("the finding: … the closed form steps
/// -15.051204, the line's root is -15.051883…"). 202 bytes, as found.
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

/// The flow_structured finding of CI's run on #384 at 53639670 (job
/// 136791952303): the tangent line's root in closed form held to its
/// bisection (`check_line_root`) for two inks `(-20, -12)` apart, reach
/// `(6, 6)`, stepped along `(-1.25, -1.09e-7)` and owed 6. The closed
/// form steps -11.196939, 4.5e-6 from the line's root in `f64`
/// (-11.1969345); the bisection -11.19753, 5.9e-4 past it, beyond the
/// property's bracket width of 2.3e-4. The bisection was the one off,
/// and rightly: one axis's gap is all but what is owed and the other's
/// 4·10⁻³ and closing, so the gap moves 8·10⁻⁴ a unit of step there, and
/// `along`, reading the gap in `f32`, cannot tell steps within a rounding
/// of 6 (5.99999957 at its step) from the root. The property now holds
/// each answer by what it owes, read in `f64` from the line alone — the
/// closed form's gap within its rounding of what is owed, each step among
/// those whose gap may read so (`root_window`) — and the closed form to
/// the line's root only where the line places its step (`check_line_root`).
/// The input is the base's: it fails alike on `epic/unified-mutations` at
/// ec9d5f41, whose `line_root`, `along` and `check_line_root` #384 does not
/// touch. 142 bytes, as found (sha1 20b3f06f…).
///
/// Controls, each planted in `line_root_seam`: with the bisection 10⁻²
/// short of the root, where the gap is clearly past what is owed, red
/// ("the bisection steps -11.18753, outside -20.805981…..-11.194736…").
/// With the closed form 10⁻³ or 3 further, green, and rightly: this line
/// does not place its step, and at both the gap is within the closed
/// form's rounding of 6 (`structured_line_root_owed_little_of_a_wide_gap`'s
/// replay, whose line does, goes red on both).
#[test]
fn test_flow_structured_finding_a_line_root_owed_a_gap_its_step_barely_moves_replays() {
    let found = include_bytes!("fuzz_findings/structured_line_root_owed_a_gap_its_step_barely_moves");
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

/// The flow_structured finding of CI's run on #384 at 22023fc9 (job
/// 136836330346), the first finding above with one byte more: two inks
/// `(-20, -12)` apart, reach `(6, 6)`, stepped along `(-0.872,
/// -7.62e-8)` and owed 6. The property as it stood held the closed form to
/// the line's root in `f64` to `CLOSED_FORM_REACH` of its scale, 1.4e-4
/// here, and it was 7.6e-4 off (-16.051414, the root -16.0506519). Its gap
/// there is 3.9e-7 short of 6, one rounding of 6: the line does not place
/// its step, the gap moving 5.6e-4 a unit of it. The engine owes the gap,
/// and keeps it far inside the pairwise rule's window. The property now
/// holds the gap, and the step to the root only where the line places it.
/// #384's own: on the epic at ec9d5f41 the property compared the closed
/// form to the bisection, 9.2e-5 apart, within that check's width of
/// 2.8e-4. 143 bytes, as found (sha1 6b3193d8…).
///
/// Control: with the closed form held to the root wherever the line runs,
/// red, CI's message ("the closed form steps -16.051414, the line's root
/// is -16.05065194473038").
#[test]
fn test_flow_structured_finding_a_line_root_owed_a_gap_along_all_but_one_axis_replays() {
    let found = include_bytes!("fuzz_findings/structured_line_root_owed_a_gap_along_all_but_one_axis");
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

/// The flow_structured finding of a local 600 s run on #384 at a48abf25,
/// in the property's own window rather than the engine: two inks `(-12,
/// 0)` apart, reach `(6, 6)`, stepped along `(-0.467, 1.159)` and owed 6,
/// where the start's gap is 6 itself, so the root is the start. The
/// window's scan bisected from the start to the first sample past it and
/// never read the start, so its end lay at -3e-63, and the closed form's
/// step, -0, fell outside it. The scan now reads its start first. 143
/// bytes, as found (sha1 a0724487…).
///
/// Control: with the scan not reading its start, red ("the closed form
/// steps -0, outside -10.244521…..-3.08e-63, the steps whose gap is
/// within its rounding 6.29e-5 of what is owed").
#[test]
fn test_flow_structured_finding_a_line_root_owed_its_gap_at_the_start_replays() {
    let found = include_bytes!("fuzz_findings/structured_line_root_owed_its_gap_at_the_start");
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

/// The flow_structured_every_face finding of CI's run on #360 at
/// 59826cfd (job 132771815404), after 5,055 runs: a directional ring
/// (#324) of its vertical text alone, `spacing` 10³⁰, round a rounded
/// rectangle some 4.5 × 10³⁴ tall whose width is not a number, placed a
/// glyph at `NaN`. The engine was wrong, and the path had no part in it:
/// the same ring round that rectangle with a width is the case below. 224
/// bytes as found (sha1 4503e5eb…); 160 here, cut down and its other
/// choices zeroed, which fails both structured properties the same way.
///
/// The pitched lay (`pitch`) sets two inks a gap apart, box to box, and
/// where the path runs on a slant that is the distance between their
/// corners, found from the gap's square. A gap past the root of the
/// largest `f32`, 1.8 × 10¹⁹, has no `f32` square: the second ink was set
/// infinitely far on, the slack's every round missed by an infinite
/// length, and the lay was laid at the slack it started from, unsolved,
/// its second ink past every float of the path. Not this PR's: the square
/// has been there since the pitched lay gapped its inks box to box, and
/// the input as found fails the same way on the engines of 1659d339,
/// 47d609c4 and 3fcf59f2. The square is taken in `f64` where `f32` has
/// none.
///
/// Until #363 the ring was then laid as any other its path holds more of
/// than its cap, 154 glyphs, cut short and saying so; at 59826cfd it laid
/// 8 glyphs, 4 of them at `NaN`, and reported nothing. Its outline is
/// 1.6 × 10³⁵ round, longer than placement lays (`MAX_PATH_LENGTH`), so
/// since #363 nothing is laid round it and the ring says why. The same
/// ring round the outline a part in 10¹⁷ the size, within the limit, lays
/// an ink a run: its gap of 10³⁰ is wider than any run, and no two of its
/// inks are set a gap apart.
///
/// Controls: the limit not held, red on the outline as drawn ("nothing
/// laid, and the ring says its path is too long": 154 laid); a ring's
/// report left out (`place`, which a directional ring round an outline
/// with no run to lay falls back to), red there too (nothing said).
#[test]
fn test_flow_structured_every_face_finding_a_directional_ring_whose_gap_is_past_the_root_of_the_largest_f32()
{
    use crate::document::element::DirectionalGlyphs;
    use crate::document::values::{Direction, Edge, FitMode, StartPoint};
    use crate::flow::place::ring::RingLoss;
    use crate::flow::place::{place_on_closed_capped, Trace};
    use crate::flow::{Outline, Rect};
    let found = include_bytes!(
        "fuzz_findings/structured_directional_ring_whose_gap_is_past_the_root_of_the_largest_f32"
    );
    for (what, checked) in [
        (
            "every face",
            crate::flow::tests::flow_fuzz::check_flow_structured_every_face(found),
        ),
        (
            "structured",
            crate::flow::tests::flow_fuzz::check_flow_structured(found),
        ),
    ] {
        if let Err(why) = checked {
            panic!("the fuzzer's input, {what}: {why}");
        }
    }
    // The outline the input drew, its width its height's rather than no
    // number, and the layout: no spans, the vertical text all round. It
    // is 1.6 × 10³⁵ round, longer than placement lays (#363): nothing is
    // laid round it, and the ring says why. A part in 10¹⁷ the size it is
    // within the limit, its gap still past the root of the largest `f32`
    // and wider than any of its runs, and it lays an ink a run.
    let (side, radius, spacing, cap) = (4.510_807_4e34_f32, 1.240_472_1e34_f32, 1e30_f32, 154);
    let layout = RingLayout {
        spacing,
        directional: DirectionalGlyphs {
            vertical: Some("a".to_owned()),
            corners: None,
        },
        ..RingLayout::whole()
    };
    assert!(
        (spacing * spacing).is_infinite(),
        "precondition: the gap has no `f32` square"
    );
    let plain = style(16.0);
    for (scale, laid) in [(1.0_f32, false), (1.0e-17, true)] {
        let path = Outline::RoundedRect {
            rect: Rect::new(Vec2::ZERO, Vec2::splat(side * scale)),
            radius: radius * scale,
        };
        let round = f64::from(side * scale) * 4.0 - f64::from(radius * scale) * (8.0 - std::f64::consts::TAU);
        assert_eq!(
            round <= f64::from(crate::flow::MAX_PATH_LENGTH),
            laid,
            "precondition: the outline, {round:e} round, is within what placement lays or past it"
        );
        for outward in [false, true] {
            let mut trace = Trace::default();
            let (run, placed) = with_fonts(|fs| {
                let run = real_run("", &plain, fs);
                let (replaced, placed) = place_on_closed_capped(
                    &run,
                    &path,
                    StartPoint {
                        edge: Edge::Top,
                        at: 0.0,
                    },
                    Direction::Clockwise,
                    &FitMode::Repeat,
                    &layout,
                    outward,
                    &plain,
                    fs,
                    cap,
                    Some(&mut trace),
                );
                (replaced.unwrap_or(run), placed)
            });
            if let Err(why) = check_placement(&run, &placed, false, "around an outline") {
                panic!("{round:e} round, outward {outward}: {why}");
            }
            if laid {
                assert_eq!(
                    (placed.glyphs.len(), placed.truncated, trace.ring),
                    (4, false, None),
                    "{round:e} round, outward {outward}: an ink a run, whole"
                );
            } else {
                assert_eq!(
                    (placed.glyphs.len(), placed.truncated, trace.ring),
                    (0, true, Some(RingLoss::TooLong)),
                    "{round:e} round, outward {outward}: nothing laid, and the ring says its path is too long"
                );
            }
        }
    }
}

/// What a structured input laid round its outline, read off the
/// harness's own record: the outline it drew, how many glyphs were laid
/// of how many, and whether the placement said it was cut; and how many
/// placements the input's replay held to the longest path placement lays,
/// clearly within it and clearly past it.
type LaidRound = (Outline, usize, usize, bool, (usize, usize));

/// `bytes` replayed through the flow_structured target's properties
/// (`check_flow_structured`), and what it laid ([`LaidRound`]).
fn replayed_round(name: &str, bytes: &[u8]) -> LaidRound {
    use crate::flow::tests::flow_fuzz::{check_flow_structured, HELD_TO_THE_LIMIT, LAST_FRAME_LAID};
    HELD_TO_THE_LIMIT.with(|held| held.set((0, 0)));
    LAST_FRAME_LAID.with(|last| last.set(None));
    if let Err(why) = check_flow_structured(bytes) {
        panic!("{name}: {why}");
    }
    let (path, placed, run, truncated) = LAST_FRAME_LAID
        .with(|last| last.take())
        .unwrap_or_else(|| panic!("precondition: {name} lays a placement round an outline"));
    (
        path,
        placed,
        run,
        truncated,
        HELD_TO_THE_LIMIT.with(std::cell::Cell::get),
    )
}

/// The four defects of #363, each as a structured input: a placement
/// round an outline too large for the engine's `f32` arithmetic. At
/// 0debe7d9 each was laid, and wrong, with nothing said:
///
/// - **a ring round an outline longer than the largest `f32`**: `aaa`
///   spaced 10³⁷, `Repeat`, round a box 1.5 × 10³⁸ by 10³⁸, 5 × 10³⁸
///   round. Its path's length was infinite; a ring of text like it laid
///   38 glyphs, three of them at positions that are not finite.
/// - **a directional ring round an outline past half the largest
///   `f32`**: the vertical text `a` round a box 6 × 10³⁷ a side, its
///   corners a quarter of that. A place on it and its length summed past
///   the largest `f32`: 40 of its 160 glyphs were set at positions that
///   are not finite.
/// - **an ornament ring round an outline past the root of the largest
///   `f32`**: `─` spaced 10²⁴, `Repeat`, round a box 10²⁵ across. The
///   distance between two of its inks had no `f32` square, and inks were
///   set nearer than their spacing: 32 glyphs, two of them 7.6 × 10²³
///   apart.
/// - **a frame round an ellipse past the root of the largest `f32`**:
///   `aaa aa a`, `Spread`, round an ellipse 10²⁵ by 6 × 10²⁴. A frame
///   like it sat 0.28 of its outline's size from where it sits round a
///   smaller ellipse.
///
/// Each outline is longer than placement lays (`MAX_PATH_LENGTH`,
/// ruling B42.1), so each lays nothing now and says so by name: the
/// replay holds it to that (`held_to_the_limit`), and here the outline
/// each input draws, that it is clearly past the limit by its own
/// numbers, and that none of its glyphs is laid and it is cut.
///
/// Controls: with no path too long (`ArcPath::from_stretches` keeping
/// its stretches whatever their length), red ("39 glyphs laid along a
/// path longer than placement lays", on the first); with a path too long
/// building no stretch and not saying so, red ("nothing laid along a path
/// longer than placement lays, and the placement does not say why").
#[test]
fn test_flow_structured_findings_round_outlines_too_large_for_f32_lay_nothing_and_say_so() {
    use crate::flow::tests::flow_fuzz::{limit_of, outline_length, Limit};
    let boxed = |w: f32, h: f32, radius: f32| Outline::RoundedRect {
        rect: Rect::new(Vec2::ZERO, Vec2::new(w, h)),
        radius,
    };
    let findings: [(&str, &[u8], Outline, usize); 4] = [
        (
            "structured_ring_round_an_outline_longer_than_the_largest_f32",
            include_bytes!("fuzz_findings/structured_ring_round_an_outline_longer_than_the_largest_f32"),
            boxed(1.5e38, 1.0e38, 0.0),
            3,
        ),
        (
            "structured_directional_ring_round_an_outline_past_half_the_largest_f32",
            include_bytes!(
                "fuzz_findings/structured_directional_ring_round_an_outline_past_half_the_largest_f32"
            ),
            boxed(6.0e37, 6.0e37, 1.5e37),
            3,
        ),
        (
            "structured_ornament_ring_round_an_outline_past_the_root_of_the_largest_f32",
            include_bytes!(
                "fuzz_findings/structured_ornament_ring_round_an_outline_past_the_root_of_the_largest_f32"
            ),
            boxed(1.0e25, 0.6e25, 0.0),
            1,
        ),
        (
            "structured_frame_round_an_ellipse_past_the_root_of_the_largest_f32",
            include_bytes!(
                "fuzz_findings/structured_frame_round_an_ellipse_past_the_root_of_the_largest_f32"
            ),
            Outline::Ellipse {
                rect: Rect::new(Vec2::ZERO, Vec2::new(1.0e25, 0.6e25)),
            },
            8,
        ),
    ];
    for (name, bytes, outline, glyphs) in findings {
        assert_eq!(
            limit_of(outline_length(&outline)),
            Limit::Past,
            "precondition: {name}: its outline is clearly longer than placement lays"
        );
        assert_eq!(
            replayed_round(name, bytes),
            (outline, 0, glyphs, true, (0, 1)),
            "{name}: its outline, none of its {glyphs} glyphs laid, cut, and held as past the limit"
        );
    }
}

/// The two flow_structured seeds either side of the longest path
/// placement lays (`MAX_PATH_LENGTH`, 2⁶³; ruling B42.1 §6): `aaa aa a`,
/// `Spread`, round a square whose four sides sum to 0.995 of the limit,
/// and round one whose sides sum to 1.005 of it. The first is laid as any
/// frame is — every glyph with ink, whole — and owes what any frame owes;
/// the second lays nothing and says why. Each is clearly on its side of
/// the limit by its own numbers, and the replay holds it there.
///
/// Controls: the limit held at half what it is, red on the first ("a
/// placement along a path within what placement lays says it is too
/// long"); at twice, red on the second ("7 glyphs laid along a path
/// longer than placement lays").
#[test]
fn test_the_flow_seeds_either_side_of_the_longest_path_laid_are_laid_and_not() {
    use crate::flow::tests::flow_fuzz::{limit_of, outline_length, Limit};
    use crate::flow::MAX_PATH_LENGTH;
    let seeds = crate::util::doc_fixtures::repo_path("fuzz/seeds/flow_structured");
    for (name, share, standing) in [
        (
            "frame_round_an_outline_just_within_the_longest_path_laid",
            0.995_f32,
            Limit::Within,
        ),
        (
            "frame_round_an_outline_just_past_the_longest_path_laid",
            1.005,
            Limit::Past,
        ),
    ] {
        let bytes =
            std::fs::read(seeds.join(name)).unwrap_or_else(|e| panic!("{name} must be readable: {e}"));
        let side = share * 0.25 * MAX_PATH_LENGTH;
        let outline = Outline::RoundedRect {
            rect: Rect::new(Vec2::ZERO, Vec2::splat(side)),
            radius: 0.0,
        };
        assert_eq!(
            limit_of(outline_length(&outline)),
            standing,
            "precondition: {name}: a square {side} a side stands {standing:?} the limit"
        );
        let (path, placed, run, truncated, held) = replayed_round(name, &bytes);
        assert_eq!((path, run), (outline, 8), "{name}: its outline and its 8 glyphs");
        if standing == Limit::Within {
            // Its two spaces part its lines or end them: six inks, and
            // the one space between two words of a line.
            assert_eq!(
                (placed, truncated, held),
                (7, false, (1, 0)),
                "{name}: laid whole, and held as within the limit"
            );
        } else {
            assert_eq!(
                (placed, truncated, held),
                (0, true, (0, 1)),
                "{name}: nothing laid, cut, and held as past the limit"
            );
        }
    }
}

/// The flow_structured finding of CI's run on #370 (job 133116364538,
/// #371): a frame some 10⁶ from the origin, `"\u{302}👩"` on its sides,
/// whose side arc's touch check held 4 classes of height, past the 3 its
/// arc held back for. The combining circumflex's ink is 0.984375 tall as
/// shaped, of class 0; placed 10⁶ from the origin, where an `f32` is a
/// sixteenth apart, its box is 0.9375 tall at some places and 1.0 at
/// others, of class 1. The arc held back for the classes of its glyphs as
/// shaped, the check counted them as placed. It now holds back for every
/// class its glyphs can take placed where the frame lies (rulings B46.1,
/// `reachable_classes`). 761 bytes as found (sha1 ed073022…); 701 here,
/// minimized on stable, which draws `"\u{302}🇯"` and fails the same way.
///
/// Control: with the arc held back for its glyphs' classes as shaped
/// (`SideOrnament::beside` counting them as placed), red ("held 4
/// classes of height, past the 3 its arc held back for").
#[test]
fn test_flow_structured_finding_a_side_arc_ink_placed_past_its_shaped_class_replays() {
    let found = include_bytes!("fuzz_findings/structured_side_arc_ink_placed_past_its_shaped_class");
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
