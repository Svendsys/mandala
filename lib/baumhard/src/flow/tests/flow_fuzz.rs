// SPDX-License-Identifier: MPL-2.0

//! TextFlow's fuzz properties (design D29; TEST_CONVENTIONS §T14):
//! shaping and placing arbitrary strings, which a map hands the app in
//! every span. The flow_raw fuzz target shapes its bytes, read as
//! text, in every layout; flow_structured reads its bytes as choices
//! ([`Choices`](crate::util::tests::choices::Choices)) and builds spans
//! of hostile text in every face, size, weight and slant, then shapes
//! and places them as a box, around an outline, along a curve or at a
//! point, with every fit mode.
//!
//! What a flow must satisfy, whatever its text and its geometry:
//!
//! - **No panic**, under the font lock the caller holds.
//! - **Every glyph names where it came from:** its span is one of the
//!   flow's or [`FILLER_SPAN`](crate::flow::FILLER_SPAN), and its bytes are a range of that span's
//!   text that starts and ends on a character boundary — what a click on
//!   a linked glyph is resolved through.
//! - **The metrics are numbers:** every advance, size, pen position,
//!   ascent and descent is finite, and so is the line height.
//! - **Placement is finite and bounded** (#239, ruling R6): every placed
//!   position is finite whatever the path, every placed glyph names a
//!   glyph of the run it refers to, and no placement holds more than
//!   [`MAX_PLACED_GLYPHS`](crate::flow::MAX_PLACED_GLYPHS).
//! - **A frame's inks never touch** (rulings R38.1, R44.2), whatever
//!   its layout — span, sides and spacing drawn too (ruling R44.4) —
//!   and its text is cut to what it lays inside the timeout
//!   ([`fits_in_time`](crate::flow::tests::flow_fuzz::fits_in_time)).
//! - **An input's frames fit in the fuzz timeout** (rulings R45.1,
//!   R46, R47.2): each target lays one frame an input, whose budget is at
//!   most what the harness counts for it and whose work is at most that
//!   budget and the overshoot its weights allow (`frame_work_steps`), and
//!   the frames' work together is at most
//!   [`INPUT_STEPS`](crate::flow::tests::flow_fuzz::INPUT_STEPS), three
//!   quarters of the timeout at the ASan build's cost a counted step.
//! - **A box is a box:** its size is finite, non-negative, at least the
//!   usable part of the minimum asked for, and one glyph-span entry per
//!   laid-out glyph.
//!
//! The geometry is hostile on purpose (any `f32`, `NaN` and the
//! infinities included): placement's promise is "whatever the path".
//! What the document bounds is kept in its bounds — a `Fit` range
//! passes [`check_fit`](crate::document::bounds::check_fit), a start
//! point's `at` is in `0..=1`, sizes are in points — because placement
//! receives those from a loaded document, never raw.

use glam::Vec2;

use crate::document::bounds;
use crate::document::resolve::ResolvedText;
use crate::document::values::{
    Align, Direction, Edge, FitMode, FontSize, LineSpan, Percent, Rgba, Shape, StartPoint, Weight,
};
use crate::flow::place::halves::{frame_budget, frame_work_steps, touch_looks};
use crate::flow::place::{frame_halves, place_on_closed_traced, Trace, MIN_TABLE_SPACING};
use crate::flow::{
    offset, outline, place_at_point, place_on_open, shape_box, shape_run, Curve, FlowSpan, Outline,
    PlacedRun, Rect, RingLayout, ShapedBox, ShapedRun, FILLER_SPAN, MAX_PLACED_GLYPHS,
};
use crate::font::fonts::{
    self, acquire_font_system_write, AppFont, FontSystemWriteGuard, COMPILED_FONT_ID_MAP,
};
use crate::util::tests::choices::Choices;

/// The most characters a structured span holds: enough for a cut, a
/// repeat and a fit to have something to do, few enough that one input
/// shapes in milliseconds.
pub const MAX_SPAN_CHARS: u32 = 64;

/// The most glyphs a frame's text is shaped to here (rulings R44.2,
/// R45.1, R46.3): the most a frame of the lightest text — one face, no
/// glyph that no face holds, a short side ornament — may hold and still
/// be laid, to the whole of its budget and its overshoot past it
/// (`frame_work_steps`), within [`INPUT_STEPS`]: 150 glyphs, 3,000 glyph
/// steps each of budget and some 65 more of shaping, inking and lays,
/// 462,949 steps, 14.95 s at [`ASAN_STEP_NANOS`] a step.
/// `test_each_flow_harness_lays_an_input_in_three_quarters_of_the_timeout`
/// holds it to that: the most, and no more. A heavier text — glyphs in
/// many faces, or in none, which the shaper searches every face for —
/// is cut further ([`framed`]). A longer text's frame is laid from as
/// much of it as fits: the engine places the whole of a real text,
/// bounded by its glyphs; a fuzz input is held to what it can lay
/// inside the timeout.
pub const MAX_FRAME_GLYPHS: usize = 150;

/// The most glyphs a frame's text was shaped to when the committed flow
/// findings were found (ruling R44.2): what their replays lay, so that
/// each still exercises the frame it was found in
/// ([`check_flow_bytes_in_every_fit`], [`check_flow_structured_as_found`]).
pub const FOUND_FRAME_GLYPHS: usize = 320;

/// The frames flow_raw lays an input round its rectangle, each in a fit
/// mode that tracks its halves ([`raw_frame_fits`]; ruling R45.1). One:
/// a frame's budget is at least 400,000 glyph steps whatever its text
/// (`TRACK_BUDGET`), so two frames may take 800,000 steps, 25.8 s at
/// [`ASAN_STEP_NANOS`] a step, past [`INPUT_WORK_MICROS`] however few
/// glyphs they share — and three, as before R45, 38.8 s. Its `Repeat`
/// round the rectangle tracks no halves, and is laid besides.
pub const RAW_FRAMES: usize = 1;

/// The frames flow_structured lays an input: one, round its outline,
/// when it draws an outline at all and a fit mode other than `Repeat`.
pub const STRUCTURED_FRAMES: usize = 1;

/// `scripts/fuzz.sh`'s `-timeout`, in seconds: the longest one input
/// may run before libFuzzer reports it as a finding.
pub const FUZZ_TIMEOUT_SECONDS: u64 = 20;

/// The most one input's frames may take under the ASan fuzz build, in
/// microseconds: three quarters of [`FUZZ_TIMEOUT_SECONDS`], 15 s
/// (ruling R45.1). The quarter left is for the work no frame counts —
/// shaping the input's text, the box, cutting a frame's text
/// ([`framed`]), the `Repeat` and the placements along a curve and at a
/// point — and for a machine busy with something else.
///
/// That quarter is not derived from the frames' weights, which would not
/// fit it: a frame counts each glyph no face holds at
/// `SHAPE_NOTDEF_STEPS`, 2,200 steps, the cost of a `.notdef` shaped
/// alone, by which a 4,096-byte raw input of some 1,365 private-use
/// characters would shape for some 48 s. The shaper's fallback is paid
/// once a shaped segment rather than once a glyph — some 6 ms, and 0.15
/// to 0.3 ms a `.notdef` past the first, in a debug build (review
/// 5739829769) — so such an input shapes in well under a second, and the
/// weight overcounts a run of `.notdef`s by up to some 30×: safe for a
/// frame's bound, and #304's to rework.
pub const INPUT_WORK_MICROS: u64 = FUZZ_TIMEOUT_SECONDS * 1_000_000 / 4 * 3;

/// What a counted glyph step of a frame's work costs under the ASan fuzz
/// build, in nanoseconds: the cost frame budgets are turned into time by
/// (rulings R45.2, R46.2) — the most measured, 21.53 µs, times 1.5.
///
/// Measured at e301e041, with the engine's accounting of rulings R46 and
/// R47, on the flow targets built by `scripts/fuzz.sh` (`cargo fuzz
/// build`: ASan, debug assertions, the pinned nightly): each frame's
/// `place_on_closed_traced` call timed, and its `Trace::spent` printed,
/// by a temporary instrumentation of the harness, never committed, over
/// every committed flow finding and seed and the heaviest frames a
/// sweep of 2,600 seeded structured inputs drew, each input run three
/// times in one process. The most of any frame's time over its counted
/// steps was 21.53 µs, a small frame on its first, cold run; frames of
/// 10,000 steps or more took 1.3 to 8.9 µs a step. The machine's load
/// average was 1 to 2. #294's PR body has the table.
pub const ASAN_STEP_NANOS: u64 = 32_300;

/// The glyph steps one input's frames may take, all together:
/// [`INPUT_WORK_MICROS`] at [`ASAN_STEP_NANOS`] a step.
pub const INPUT_STEPS: usize = (INPUT_WORK_MICROS * 1_000 / ASAN_STEP_NANOS) as usize;

/// The glyph steps one frame of an input may take, its budget and its
/// overshoot together: an even share of [`INPUT_STEPS`] among `frames`
/// frames.
pub const fn frame_steps(frames: usize) -> usize {
    INPUT_STEPS / frames
}

/// Whether a frame's text shaped as `run`, its sides as `ornament`, is
/// one the harnesses lay (rulings R45.1, R46.3): at most
/// [`MAX_FRAME_GLYPHS`] glyphs, and laid to the whole of its budget and
/// past it (`frame_work_steps`, a `Fit` frame's as the most) within its
/// share of [`INPUT_STEPS`] among `frames` frames.
pub fn fits_in_time(run: &ShapedRun, ornament: Option<&ShapedRun>, frames: usize) -> bool {
    run.glyphs.len() <= MAX_FRAME_GLYPHS
        && frame_work_steps(run, ornament, true) <= frame_steps(frames.max(1))
}

/// The work an input's frames may take, counted as they are laid
/// (rulings R45.1, R47.2): each frame's budget and its overshoot past it
/// (`frame_work_steps`), the most its work may take.
#[derive(Default)]
pub(crate) struct InputWork {
    frames: usize,
    steps: usize,
}

impl InputWork {
    /// Count a frame of `run` (sides shaped as `ornament`, a `Fit` frame
    /// `sized`), laid with `trace`: the most it may take,
    /// `frame_work_steps`. The engine scales a frame's budget by its
    /// text's glyphs once cut, never more than the run has, spends no
    /// more than its budget and its overshoot, and no touch check of it
    /// looks at more than `touch_looks` pairs a box of the classes of
    /// height it holds, which that overshoot rests on (rulings R48.2,
    /// R51.2): properties.
    pub(crate) fn frame(
        &mut self,
        run: &ShapedRun,
        ornament: Option<&ShapedRun>,
        sized: bool,
        trace: &Trace,
        what: &str,
    ) -> Result<(), String> {
        let budget = frame_budget(run.glyphs.len());
        let most = frame_work_steps(run, ornament, sized);
        if !trace.looks.within() {
            let (looks, boxes, classes) = trace.looks.worst;
            return Err(format!(
                "{what}: a touch check looked at {looks} pairs of {boxes} boxes of {classes} classes of height, past {} a box (rulings R48.2, R51.2)",
                touch_looks(classes)
            ));
        }
        if trace.budget > budget {
            return Err(format!(
                "{what}: a frame budgeted {} glyph steps, past the {budget} its {} glyphs give",
                trace.budget,
                run.glyphs.len()
            ));
        }
        if trace.spent > most {
            return Err(format!(
                "{what}: a frame took {} glyph steps, past its budget of {} and its overshoot, {most} in all",
                trace.spent, trace.budget
            ));
        }
        self.frames += 1;
        self.steps += most;
        Ok(())
    }

    /// Whether the input's frames, laid to their whole budgets and past
    /// them, fit in [`INPUT_STEPS`]: what holds the harness to the
    /// timeout on every input, as
    /// `test_each_flow_harness_lays_an_input_in_three_quarters_of_the_timeout`
    /// holds its constants.
    fn within_the_timeout(&self) -> Result<(), String> {
        if self.steps > INPUT_STEPS {
            return Err(format!(
                "an input's {} frames may take {} glyph steps, past the {INPUT_STEPS} one input may take",
                self.frames, self.steps
            ));
        }
        Ok(())
    }
}

/// The ornament of a ring's `sides`, shaped alone in `style` at `scale`:
/// what its frame's shaping and inking are counted from
/// (`frame_work_steps`). `None` without sides.
pub(crate) fn ornament_run(
    layout: &RingLayout,
    style: &ResolvedText,
    scale: f32,
    fs: &mut FontSystemWriteGuard,
) -> Option<ShapedRun> {
    let (ornament, _) = layout.sides.as_ref()?;
    Some(shape_run(
        &[FlowSpan {
            text: ornament,
            style,
            link: None,
        }],
        scale,
        fs,
    ))
}

/// The most characters a fuzzed side ornament [`ring_layout`] draws holds:
/// a short one, every copy of it laid many times round each junction.
/// A long one is drawn after it ([`long_ornament`], ruling R50.3).
const MAX_ORNAMENT_CHARS: u32 = 4;

/// Characters a structured span draws from besides any code point:
/// combining marks, ZWJ sequences, variation selectors, regional
/// indicators, right-to-left letters, bidi controls, every paragraph
/// separator, controls and box-drawing glyphs.
const INTERESTING: [char; 28] = [
    'a',
    ' ',
    '\u{301}',
    '\u{302}',
    '\u{200D}',
    '\u{FE0F}',
    '\u{1F469}',
    '\u{1F4BB}',
    '\u{1F1EF}',
    '\u{1F1F5}',
    '\u{5D0}',
    '\u{627}',
    '\u{644}',
    '\u{202E}',
    '\u{2066}',
    '\n',
    '\r',
    '\u{1C}',
    '\u{85}',
    '\u{2028}',
    '\u{2029}',
    '\t',
    '\u{0}',
    '\u{7F}',
    '─',
    '═',
    '\u{E000}',
    '\u{FFFD}',
];

/// Every property of `bytes` read as text (lossily: a raw input need
/// not be UTF-8), shaped as one span in each layout: the flow_raw
/// target's whole body.
///
/// It lays [`RAW_FRAMES`] frames round its rectangle, in fit modes drawn
/// from the bytes ([`raw_frame_fits`]), so that the input's work is held
/// inside the fuzz timeout (ruling R45.1); a `Repeat` round the
/// rectangle, which tracks no halves, and all four modes along its line
/// are laid whatever it draws. [`check_flow_bytes_in_every_fit`] lays
/// all three frames, as a finding's replay does.
pub fn check_flow_bytes(bytes: &[u8]) -> Result<(), String> {
    let fits = raw_frame_fits(bytes);
    check_raw(bytes, &fits, true)
}

/// [`check_flow_bytes`] with a frame in every fit mode a frame tracks
/// its halves in — `Fit`, `Spread` and `Once` — as the flow_raw target
/// laid every input before ruling R45, its text cut as it was when the
/// committed findings were found ([`FOUND_FRAME_GLYPHS`]), and each
/// frame laid in both [`RingLayout::whole`] — the layout every input
/// was laid in before ruling R44.4, and every committed raw finding was
/// found in — and the layout its bytes draw ([`ring_layout`]): what a
/// committed raw finding replays through, so that it still exercises
/// every frame it was found in, and a finding found since R44.4 the
/// frame it was found in. It is not held to the per-input work bound,
/// which is the fuzz timeout's, and a test has none.
pub fn check_flow_bytes_in_every_fit(bytes: &[u8]) -> Result<(), String> {
    check_raw(bytes, &frame_fits(), false)
}

/// The fit modes a flow_raw frame is laid in, in the order
/// [`raw_frame_fits`] draws them: every one of them tracks its halves
/// (`place_halves`), so each is budgeted (`frame_budget`).
pub fn frame_fits() -> [FitMode; 3] {
    [
        FitMode::Fit { min: 0.5, max: 2.0 },
        FitMode::Spread,
        FitMode::Once {
            filler: Some("·".to_string()),
        },
    ]
}

/// The [`RAW_FRAMES`] fit modes flow_raw lays `bytes`' frames in
/// (ruling R45.1): read as choices after the ring's layout
/// ([`ring_layout`]), the first of them is any of [`frame_fits`], and
/// the rest follow it in turn. An input whose choices run out before it
/// — every committed raw finding but the ones that spell a layout —
/// draws `Fit`, the mode most of them were found in.
pub fn raw_frame_fits(bytes: &[u8]) -> Vec<FitMode> {
    let mut choices = Choices::new(bytes);
    ring_layout(&mut choices);
    let fits = frame_fits();
    let first = choices.below(fits.len() as u32) as usize;
    (0..RAW_FRAMES)
        .map(|k| fits[(first + k) % fits.len()].clone())
        .collect()
}

/// [`check_flow_bytes`]'s body, its frames laid in `fits`, held to the
/// per-input work bound with `bounded`; without, replaying a finding,
/// laid in [`RingLayout::whole`] as well as the layout its bytes draw.
pub(crate) fn check_raw(bytes: &[u8], fits: &[FitMode], bounded: bool) -> Result<(), String> {
    let text = String::from_utf8_lossy(bytes);
    let style = plain_style();
    let spans = [FlowSpan {
        text: &text,
        style: &style,
        link: None,
    }];
    fonts::init();
    let mut fs = acquire_font_system_write("flow fuzz");
    let shaped = shape_box(&spans, None, Align::Left, &mut fs);
    check_box(&spans, &shaped, None)?;
    let run = shape_run(&spans, 1.0, &mut fs);
    check_run(&spans, &run, "the run")?;
    // The frames' layout, drawn from the same bytes read as choices
    // (ruling R44.4). A replay lays them over their whole halves with no
    // sides too, as every input was laid before R44.4: a finding's bytes
    // are text, not zeros, and draw another layout from the one it was
    // found in (review 5739829769).
    let layouts = laid_layouts(ring_layout(&mut Choices::new(bytes)), bounded);
    for layout in &layouts {
        frames_round_a_rectangle(&text, &style, layout, fits, bounded, &mut fs)
            .map_err(|why| format!("{why}, laid {layout:?}"))?;
    }
    for fit in std::iter::once(FitMode::Repeat).chain(frame_fits()) {
        let curve = Curve::Line(Vec2::ZERO, Vec2::new(200.0, 0.0));
        let placed = place_on_open(&run, &curve, &fit, 1.0, &style, &mut fs);
        check_placed(
            &spans,
            &run,
            placed,
            owes_whole(&run, &fit, curve_room(&curve)),
            "along a line",
        )?;
    }
    check_placement(&run, &place_at_point(&run, Vec2::ZERO), false, "at a point")
}

/// The layouts a frame drawn in layout `drawn` is laid in: `drawn`, and
/// replaying a finding (not `bounded`) [`RingLayout::whole`] before it,
/// the layout every input was laid in before ruling R44.4 — so that a
/// committed finding replays in the frame it was found in, whenever it
/// was found (review 5739829769).
pub(crate) fn laid_layouts(drawn: RingLayout, bounded: bool) -> Vec<RingLayout> {
    if bounded || drawn == RingLayout::whole() {
        vec![drawn]
    } else {
        vec![RingLayout::whole(), drawn]
    }
}

/// [`check_raw`]'s frames round its rectangle for `text` in `style`,
/// laid with `layout` in `fits` and a `Repeat` — their text held to what
/// they lay inside the timeout with `bounded` (rulings R45.1, R46.3), or,
/// replaying a finding, to what a frame laid when it was found.
fn frames_round_a_rectangle(
    text: &str,
    style: &ResolvedText,
    layout: &RingLayout,
    fits: &[FitMode],
    bounded: bool,
    fs: &mut FontSystemWriteGuard,
) -> Result<(), String> {
    let rect = Rect::new(Vec2::ZERO, Vec2::new(120.0, 40.0));
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    // Its sides, then its text, held to what its frames lay inside the
    // timeout (rulings R45.1, R50.3).
    let (layout, ornament) = if bounded {
        sides_in_time(layout, style, 1.0, fits.len(), fs)
    } else {
        (layout.clone(), ornament_run(layout, style, 1.0, fs))
    };
    let layout = &layout;
    let fits_in_time = |run: &ShapedRun| fits_in_time(run, ornament.as_ref(), fits.len());
    let as_found = |run: &ShapedRun| run.glyphs.len() <= FOUND_FRAME_GLYPHS;
    let framed = framed(
        &[text.to_string()],
        std::slice::from_ref(style),
        1.0,
        fs,
        if bounded { &fits_in_time } else { &as_found },
    );
    let frame_spans = [FlowSpan {
        text: &framed[0],
        style,
        link: None,
    }];
    let frame_run = shape_run(&frame_spans, 1.0, fs);
    let mut work = InputWork::default();
    for fit in std::iter::once(&FitMode::Repeat).chain(fits) {
        let path = outline(
            rect,
            Shape::Rectangle {
                radius: Percent::new(50),
            },
        );
        let mut trace = Trace::default();
        let placed = place_on_closed_traced(
            &frame_run,
            &path,
            start,
            Direction::Clockwise,
            fit,
            layout,
            true,
            style,
            fs,
            Some(&mut trace),
        );
        if !matches!(fit, FitMode::Repeat) {
            let sized = matches!(fit, FitMode::Fit { .. });
            work.frame(&frame_run, ornament.as_ref(), sized, &trace, "around a rectangle")?;
        }
        // A line cut so that no two inks touch, and reported so, is owed
        // no longer (ruling R38.1); a frame its sides leave unheld is
        // reported (ruling R35.2).
        let whole = owes_whole(&frame_run, fit, frame_room(&path)) && trace.cut_apart.is_empty();
        apart(&trace, "around a rectangle")?;
        check_placed(&frame_spans, &frame_run, placed, whole, "around a rectangle")?;
    }
    if bounded {
        work.within_the_timeout()?;
    }
    Ok(())
}

/// Every property of the flow `bytes` choose: the flow_structured
/// target's whole body.
pub fn check_flow_structured(bytes: &[u8]) -> Result<(), String> {
    check_flow_generated(&mut Choices::new(bytes))
}

/// [`check_flow_structured`] with a frame's text held to what a frame
/// laid when the committed flow findings were found
/// ([`FOUND_FRAME_GLYPHS`]) rather than to what it lays inside the fuzz
/// timeout (rulings R45.1, R46.3): what a committed structured finding
/// replays through, so that it still exercises the frame it was found
/// in. It is not held to the per-input work bound, which is the fuzz
/// timeout's, and a test has none.
pub fn check_flow_structured_as_found(bytes: &[u8]) -> Result<(), String> {
    check_generated(&mut Choices::new(bytes), false)
}

/// [`check_flow_structured`] for choices from any source.
pub fn check_flow_generated(choices: &mut Choices<'_>) -> Result<(), String> {
    check_generated(choices, true)
}

/// [`check_flow_generated`]'s body, a frame's text held to what it lays
/// inside the fuzz timeout with `bounded`, and to what it laid when the
/// findings were found without.
fn check_generated(choices: &mut Choices<'_>, bounded: bool) -> Result<(), String> {
    fonts::init();
    let faces = faces();
    let count = choices.below(4) as usize;
    let texts: Vec<String> = (0..count).map(|_| text(choices)).collect();
    let styles: Vec<ResolvedText> = (0..count).map(|_| style(choices, &faces)).collect();
    let spans: Vec<FlowSpan<'_>> = texts
        .iter()
        .zip(&styles)
        .map(|(text, style)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect();
    let flow = style(choices, &faces);
    let mut fs = acquire_font_system_write("flow fuzz");
    match choices.below(4) {
        0 => {
            let min = choices.one_in(2).then(|| any_vec2(choices));
            let align = *choices.pick(&[Align::Left, Align::Center, Align::Right]);
            let shaped = shape_box(&spans, min, align, &mut fs);
            check_box(&spans, &shaped, min)?;
            // `glyph_spans` lists the glyphs in the buffer's layout order.
            let glyphs = shaped
                .buffer
                .layout_runs()
                .flat_map(|run| {
                    run.glyphs
                        .iter()
                        .map(|glyph| (glyph.font_id, cluster(run.text, glyph)))
                        .collect::<Vec<_>>()
                })
                .zip(shaped.glyph_spans.iter())
                .map(|((face, text), (span, _))| (face, *span, text))
                .collect::<Vec<_>>();
            shapes_in_pinned_faces(&spans, glyphs, &mut fs).map_err(|why| format!("in a box: {why}"))
        }
        layout => {
            let scale = any_f32(choices);
            let run = shape_run(&spans, scale, &mut fs);
            check_run(&spans, &run, "the run")
                .map_err(|why| format!("{why}, at scale {scale}, of {spans:?}"))?;
            // `glyphs` lists them in visual order: a right-to-left line's
            // backward (`ShapedRun::layout_glyphs`).
            let glyphs = run
                .buffer
                .layout_runs()
                .flat_map(|line| {
                    let mut laid: Vec<_> = line
                        .glyphs
                        .iter()
                        .map(|glyph| (glyph.font_id, cluster(line.text, glyph)))
                        .collect();
                    if line.rtl {
                        laid.reverse();
                    }
                    laid
                })
                .zip(&run.glyphs)
                .map(|((face, text), metrics)| (face, metrics.span, text))
                .collect::<Vec<_>>();
            shapes_in_pinned_faces(&spans, glyphs, &mut fs).map_err(|why| format!("in the run: {why}"))?;
            match layout {
                1 => {
                    let rect = Rect::new(any_vec2(choices), any_vec2(choices));
                    let shape = if choices.one_in(2) {
                        Shape::Ellipse
                    } else {
                        Shape::Rectangle {
                            radius: Percent::new(choices.below(101) as u8),
                        }
                    };
                    let mut path = outline(rect, shape);
                    if choices.one_in(2) {
                        path = offset(&path, any_f32(choices));
                    }
                    let start = StartPoint {
                        edge: *choices.pick(&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left]),
                        at: choices.below(1001) as f32 / 1000.0,
                    };
                    let direction = *choices.pick(&[Direction::Clockwise, Direction::CounterClockwise]);
                    let fit = fit(choices, &texts);
                    let outward = choices.one_in(2);
                    // Drawn last, so an input that ends before them lays
                    // over whole halves with no sides, as every input did
                    // before R44.4. A replay lays a finding in that layout
                    // too, whatever it draws: a finding that runs on past
                    // its outline draws another layout from the one it was
                    // found in (review 5739829769).
                    let drawn = ring_layout(choices);
                    let layouts = laid_layouts(long_ornament(choices, drawn), bounded);
                    // Its text's sizes, spread over the font-size window one time in
                    // four (ruling R51.2), drawn after its layout too. A replay
                    // lays a finding's text as found as well, as it lays its
                    // layout (`laid_layouts`): two committed findings run on
                    // past the draw.
                    let drawn_texts = mixed_sizes(choices, &texts, &styles, scale, &faces);
                    let mut laid_texts = vec![drawn_texts];
                    if !bounded && laid_texts[0].0 != texts {
                        laid_texts.push((texts.clone(), styles.clone()));
                    }
                    for ((frame_texts, frame_styles), layout) in laid_texts
                        .iter()
                        .flat_map(|t| layouts.iter().map(move |l| (t, l)))
                    {
                        if laid_texts.len() > 1 && *frame_texts == texts {
                            FOUND_TEXT_LAID.with(|n| n.set(n.get() + 1));
                        }
                        // The frame's sides and then its text, held to what
                        // its frame lays inside the timeout (rulings R45.1,
                        // R46.3, R50.3) — or, replaying a finding, its text
                        // to what a frame laid when it was found.
                        let (layout, ornament) = if bounded {
                            sides_in_time(layout, &flow, scale, STRUCTURED_FRAMES, &mut fs)
                        } else {
                            (layout.clone(), ornament_run(layout, &flow, scale, &mut fs))
                        };
                        let layout = &layout;
                        let fits_in_time =
                            |run: &ShapedRun| fits_in_time(run, ornament.as_ref(), STRUCTURED_FRAMES);
                        let as_found = |run: &ShapedRun| run.glyphs.len() <= FOUND_FRAME_GLYPHS;
                        let framed = framed(
                            frame_texts,
                            frame_styles,
                            scale,
                            &mut fs,
                            if bounded { &fits_in_time } else { &as_found },
                        );
                        let frame_spans: Vec<FlowSpan<'_>> = framed
                            .iter()
                            .zip(frame_styles)
                            .map(|(text, style)| FlowSpan {
                                text,
                                style,
                                link: None,
                            })
                            .collect();
                        let run = if framed == texts {
                            run.clone()
                        } else {
                            shape_run(&frame_spans, scale, &mut fs)
                        };
                        let mut trace = Trace::default();
                        let placed = place_on_closed_traced(
                            &run,
                            &path,
                            start,
                            direction,
                            &fit,
                            layout,
                            outward,
                            &flow,
                            &mut fs,
                            Some(&mut trace),
                        );
                        // A frame that cannot hold its lines without two
                        // inks touching — inward round an outline too thin
                        // for both, or a line of tall inks wrapped round a
                        // tight bend — cuts the line that cannot be held
                        // and says so (ruling R38.1): what it cut is no
                        // longer owed.
                        let whole = owes_whole(&run, &fit, frame_room(&path)) && trace.cut_apart.is_empty();
                        let mut work = InputWork::default();
                        let sized = matches!(fit, FitMode::Fit { .. });
                        let checked = work
                            .frame(&run, ornament.as_ref(), sized, &trace, "around an outline")
                            .and_then(|()| {
                                if bounded {
                                    work.within_the_timeout()
                                } else {
                                    Ok(())
                                }
                            })
                            .and_then(|()| apart(&trace, "around an outline"))
                            .and_then(|()| {
                                check_placed(&frame_spans, &run, placed, whole, "around an outline")
                            });
                        checked
                            .map_err(|why| format!("{why}, laid {layout:?}, {} spans", frame_texts.len()))?;
                    }
                    Ok(())
                }
                2 => {
                    let points: Vec<Vec2> = (0..choices.below(3)).map(|_| any_vec2(choices)).collect();
                    let Some(curve) = Curve::from_points(any_vec2(choices), &points, any_vec2(choices))
                    else {
                        return Err("two control points or fewer always make a curve".to_string());
                    };
                    let fit = fit(choices, &texts);
                    let spacing = any_f32(choices);
                    let placed = place_on_open(&run, &curve, &fit, spacing, &flow, &mut fs);
                    // `Spread` and `Fit` solve the gap; the spacing does not
                    // enter (ruling R6, reading 7).
                    let whole = owes_whole(&run, &fit, curve_room(&curve));
                    check_placed(&spans, &run, placed, whole, "along a curve")
                }
                _ => check_placement(
                    &run,
                    &place_at_point(&run, any_vec2(choices)),
                    false,
                    "at a point",
                ),
            }
        }
    }
}

/// The plain style the raw target shapes in: a compiled face, pinned,
/// so a finding replays alike on every machine (ruling R30's intent) —
/// `AppFont::Any` draws in whatever face the host falls back to, and a
/// raw input's path through the placement was the host's. A character
/// the face lacks still falls back, as it would in a map.
fn plain_style() -> ResolvedText {
    ResolvedText {
        font: AppFont::GalatiaSIL,
        size: FontSize::Pt(16.0),
        color: Rgba::new(255, 255, 255, 255),
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// Every face a span can name and replay alike on every machine: each
/// compiled-in `AppFont`, in name order — not `Any`, which draws in the
/// host's fallback face, so a finding in it need not replay elsewhere
/// (ruling R30's intent). Read from the compiled set itself rather than from
/// the family index, which attributes a family to `Any` when a host face
/// bearing its name comes first (the shadowing R30.1 removes): a sweep
/// that drew its faces from the index would stop pinning that family at
/// all, and so could not see a host face win its pin.
pub(crate) fn faces() -> Vec<AppFont> {
    let mut faces: Vec<AppFont> = COMPILED_FONT_ID_MAP
        .iter()
        .filter(|(_, ids)| !ids.is_empty())
        .map(|(font, _)| *font)
        .collect();
    faces.sort_by_key(|font| format!("{font:?}"));
    faces
}

/// A resolved style: any face, any size in points (the shaper clamps
/// what is out of its domain, `NaN` included), any weight and slant.
fn style(choices: &mut Choices<'_>, faces: &[AppFont]) -> ResolvedText {
    let size = match choices.below(3) {
        0 => 16.0,
        1 => choices.below(200) as f32,
        _ => any_f32(choices),
    };
    ResolvedText {
        font: *choices.pick(faces),
        size: FontSize::Pt(size),
        color: Rgba::new(255, 255, 255, choices.below(256) as u8),
        weight: Weight::new(100 * (1 + choices.below(9) as u16)).unwrap_or(Weight::NORMAL),
        italic: choices.one_in(2),
    }
}

/// A text of up to [`MAX_SPAN_CHARS`] characters, each an interesting
/// one or any code point.
fn text(choices: &mut Choices<'_>) -> String {
    (0..choices.below(MAX_SPAN_CHARS))
        .map(|_| {
            // Any code point, rarely: one no face holds sends shaping
            // through every face in search of a fallback, which is slow.
            if choices.one_in(8) {
                char::from_u32(choices.below(0x11_0000)).unwrap_or('\u{FFFD}')
            } else {
                *choices.pick(&INTERESTING)
            }
        })
        .collect()
}

/// A ring's layout a document can hold, from choices (ruling R44.4):
/// its lines' span natural, whole or a unit fraction from ½ to ⅛; its
/// sides none, or a short ornament from the structured alphabet that
/// [`check_ornament`](bounds::check_ornament) passes; and a repeat
/// ring's spacing, none or any value
/// [`check_non_negative`](bounds::check_non_negative) passes. Zero
/// choices draw [`RingLayout::whole`].
pub fn ring_layout(choices: &mut Choices<'_>) -> RingLayout {
    let span = match choices.below(3) {
        0 => LineSpan::WHOLE,
        1 => LineSpan::NATURAL,
        _ => LineSpan::new(1.0 / (2 + choices.below(7)) as f32).unwrap_or(LineSpan::WHOLE),
    };
    let sides = if choices.one_in(2) {
        let ornament: String = (0..1 + choices.below(MAX_ORNAMENT_CHARS))
            .map(|_| {
                if choices.one_in(8) {
                    char::from_u32(choices.below(0x11_0000)).unwrap_or('\u{FFFD}')
                } else {
                    *choices.pick(&INTERESTING)
                }
            })
            .collect();
        bounds::check_ornament(&ornament)
            .ok()
            .map(|ornament| (ornament.to_owned(), Rgba::new(255, 255, 255, 255)))
    } else {
        None
    };
    let spacing = if choices.one_in(2) {
        bounds::check_non_negative(any_f32(choices)).unwrap_or(0.0)
    } else {
        0.0
    };
    RingLayout { span, sides, spacing }
}

/// `layout` with its sides cut, and their ornament shaped alone in
/// `style` at `scale`, so that a frame of no text beside them, laid to
/// the whole of its budget and its overshoot, fits its share of
/// [`INPUT_STEPS`] among `frames` frames (rulings R45.1, R50.3): halved
/// until it fits, and left off once nothing is left. So the text cut after ([`framed`]) always finds a frame
/// that fits, whatever the ornament's glyphs cost to shape on this host —
/// a glyph no face holds included, which no text cut makes room for.
///
/// Cost: a shaping of the ornament, and one more each halving.
pub fn sides_in_time(
    layout: &RingLayout,
    style: &ResolvedText,
    scale: f32,
    frames: usize,
    fs: &mut FontSystemWriteGuard,
) -> (RingLayout, Option<ShapedRun>) {
    let empty = shape_run(
        &[FlowSpan {
            text: "",
            style,
            link: None,
        }],
        scale,
        fs,
    );
    let mut layout = layout.clone();
    loop {
        let ornament = ornament_run(&layout, style, scale, fs);
        let Some(shaped) = &ornament else {
            return (layout, None);
        };
        if fits_in_time(&empty, Some(shaped), frames) {
            return (layout, ornament);
        }
        let cut = layout.sides.as_ref().and_then(|(text, color)| {
            let chars = text.chars().count();
            let cut: String = text.chars().take(chars / 2).collect();
            bounds::check_ornament(&cut)
                .ok()
                .map(|cut| (cut.to_owned(), *color))
        });
        layout.sides = cut;
    }
}

/// The glyphs a long side ornament is drawn from (ruling R50.3): widths
/// from a hairline to an em dash, a full block and an Egyptian
/// hieroglyph, each held by a compiled face, so that it shapes alike on
/// every host — and one, U+FFFE, a noncharacter no face holds on any
/// host, as a map's ornament may hold glyphs no face does: a `.notdef` weighs 2,200 glyph steps to shape
/// (`SHAPE_NOTDEF_STEPS`), which [`sides_in_time`] cuts the ornament for.
/// (At e402949d it drew `❧`, `⸻` and `﷽`, which no compiled face holds:
/// held by a host face here and not on CI, so the long ornaments CI drew
/// were `.notdef`s, which the harness did not cut for — the flow_structured
/// finding `structured_frame_long_ornament_past_the_input_bound`.)
pub const ORNAMENT_GLYPHS: [char; 10] = ['|', '·', '-', '=', 'x', 'W', '—', '─', '𓀀', '\u{FFFE}'];

/// The longest side ornament the long-ornament mode draws, in bytes
/// (ruling R50.3): the reviewer's 200 (review 5739829769).
pub const LONG_ORNAMENT_BYTES: usize = 200;

/// `layout`, or, one time in four, `layout` with a long side ornament
/// (ruling R50.3): up to two glyphs of [`ORNAMENT_GLYPHS`], then a
/// sequence of one to four of them repeated, up to
/// [`LONG_ORNAMENT_BYTES`] bytes in all — a wide glyph among thin ones,
/// as a map's ornament may be, where [`ring_layout`]'s are at most
/// `MAX_ORNAMENT_CHARS` characters.
///
/// Drawn after [`ring_layout`], from choices nothing before it reads, and
/// not at all on a zero choice: an input that ends before it — every
/// committed finding but the one it was found with — lays the layout it
/// was found in.
///
/// It stays within the per-input work bound (ruling R45.1) by trading
/// text, and then ornament, for the ornament's work: the ornament is cut
/// until a frame of no text beside it fits ([`sides_in_time`]), and then
/// the frame's text until its work, laid to the whole of its budget and
/// its overshoot, fits [`INPUT_STEPS`] ([`framed`], [`fits_in_time`]);
/// the overshoot counts the ornament's shaping and inking twice beside
/// the text's, its faces and `.notdef`s included (`build_steps`). At
/// 16 pt in the compiled faces alone, [`MAX_FRAME_GLYPHS`] glyphs of one
/// face beside a 4-glyph ornament take 462,949 steps, within the 464,396
/// of [`INPUT_STEPS`]; beside 200 `|`s, which add 9,016 more to shape
/// and ink, 471,965, past it, so the text is halved to 75 glyphs, whose
/// budget is the 400,000-step floor: 417,090. A text of one glyph beside
/// the heaviest long ornament the compiled faces hold, 200 `|`s, takes
/// 412,280, with host faces or without; a frame of no text beside fifty
/// `.notdef`s, 624,221, so the ornament is cut first.
pub fn long_ornament(choices: &mut Choices<'_>, layout: RingLayout) -> RingLayout {
    if !choices.one_in(4) {
        return layout;
    }
    let mut ornament: String = (0..choices.below(3))
        .map(|_| *choices.pick(&ORNAMENT_GLYPHS))
        .collect();
    let unit: Vec<char> = (0..1 + choices.below(4))
        .map(|_| *choices.pick(&ORNAMENT_GLYPHS))
        .collect();
    let bytes = 1 + choices.index(LONG_ORNAMENT_BYTES);
    for glyph in unit.iter().cycle() {
        if ornament.len() + glyph.len_utf8() > bytes {
            break;
        }
        ornament.push(*glyph);
    }
    match bounds::check_ornament(&ornament) {
        Ok(_) => RingLayout {
            sides: Some((ornament, Rgba::new(255, 255, 255, 255))),
            ..layout
        },
        Err(_) => layout,
    }
}

thread_local! {
    /// Test counters (ruling R51.2): the texts of more sizes
    /// [`mixed_sizes`] has drawn on this thread, and the frames replayed
    /// with a finding's text as found beside one it drew.
    pub(crate) static MIXED_DRAWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static FOUND_TEXT_LAID: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most spans a frame's text has in the mixed-size mode (ruling
/// R51.2): the three it may draw, and more of a size each, to sixteen.
pub const MIXED_SIZE_SPANS: usize = 16;

/// `texts` in `styles`, or, one time in four, with spans added to
/// [`MIXED_SIZE_SPANS`] in all (ruling R51.2): each one to four glyphs of
/// [`ORNAMENT_GLYPHS`] at a size of 0.5 pt times a power of two to 2¹³,
/// over the whole font-size window, 0.5 to 4,096 pt, at `scale` — a text
/// of up to sixteen sizes, as a map's may be, whose touch checks hold a
/// class of height for each, where the three spans `check_generated`
/// draws first hold three at most.
///
/// Drawn after [`long_ornament`], from choices nothing before it reads,
/// and not at all on a zero choice: an input that ends before it — every
/// committed finding — lays the text it was found with. Its spans are
/// short, and the frame's text is cut as any other to fit the per-input
/// work bound ([`framed`]).
pub fn mixed_sizes(
    choices: &mut Choices<'_>,
    texts: &[String],
    styles: &[ResolvedText],
    scale: f32,
    faces: &[AppFont],
) -> (Vec<String>, Vec<ResolvedText>) {
    let (mut texts, mut styles) = (texts.to_vec(), styles.to_vec());
    if !choices.one_in(4) {
        return (texts, styles);
    }
    MIXED_DRAWN.with(|n| n.set(n.get() + 1));
    let more = 1 + choices.index(MIXED_SIZE_SPANS.saturating_sub(texts.len()).max(1));
    for _ in 0..more {
        texts.push(
            (0..1 + choices.below(4))
                .map(|_| *choices.pick(&ORNAMENT_GLYPHS))
                .collect(),
        );
        // The size the shaper sets at `scale`, where that is a number.
        let size = 0.5 * (1u32 << choices.below(14)) as f32;
        let unscaled = size / scale;
        styles.push(ResolvedText {
            size: FontSize::Pt(if unscaled.is_finite() && unscaled > 0.0 {
                unscaled
            } else {
                size
            }),
            ..style(choices, faces)
        });
    }
    (texts, styles)
}

/// `texts`, in `styles`, at `scale`, each cut to the same share of its
/// characters — halved until their run `fits`: what a frame lays here
/// (rulings R44.2, R45.1, R46.3). The harnesses' rule: at most
/// [`MAX_FRAME_GLYPHS`] glyphs, and its frames' work, laid to their
/// budgets and past them, within [`INPUT_STEPS`]. Texts whose run fits
/// already are returned whole; with no characters left, the empty texts.
///
/// Cost: a shaping of the texts, and one more each halving.
pub fn framed(
    texts: &[String],
    styles: &[ResolvedText],
    scale: f32,
    fs: &mut FontSystemWriteGuard,
    fits: &dyn Fn(&ShapedRun) -> bool,
) -> Vec<String> {
    let mut share = usize::MAX;
    loop {
        let cut: Vec<String> = texts
            .iter()
            .map(|text| {
                let chars = text.chars().count();
                text.chars()
                    .take(if share == usize::MAX {
                        chars
                    } else {
                        chars * share / 1024
                    })
                    .collect()
            })
            .collect();
        let spans: Vec<FlowSpan<'_>> = cut
            .iter()
            .zip(styles)
            .map(|(text, style)| FlowSpan {
                text,
                style,
                link: None,
            })
            .collect();
        if share == 0 || fits(&shape_run(&spans, scale, fs)) {
            return cut;
        }
        share = if share == usize::MAX { 512 } else { share / 2 };
    }
}

/// A number: plain, or any `f32` at all.
fn any_f32(choices: &mut Choices<'_>) -> f32 {
    match choices.below(4) {
        0 => 1.0,
        1 => choices.below(2000) as f32 - 1000.0,
        2 => *choices.pick(&[
            0.0,
            -0.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e30,
            -1e30,
            f32::MIN_POSITIVE,
        ]),
        _ => choices.any_f32(),
    }
}

fn any_vec2(choices: &mut Choices<'_>) -> Vec2 {
    Vec2::new(any_f32(choices), any_f32(choices))
}

/// A fit mode a document can hold: a `Fit` range that passes
/// `check_fit` (the built-in one otherwise), and a filler from the
/// flow's own texts.
fn fit(choices: &mut Choices<'_>, texts: &[String]) -> FitMode {
    match choices.below(4) {
        0 => FitMode::Repeat,
        1 => FitMode::Spread,
        2 => {
            let (min, max) = (any_f32(choices), any_f32(choices));
            match bounds::check_fit(min, max) {
                Ok((min, max)) => FitMode::Fit { min, max },
                Err(_) => FitMode::Fit { min: 0.5, max: 2.0 },
            }
        }
        _ => FitMode::Once {
            filler: (!texts.is_empty() && choices.one_in(2)).then(|| choices.pick(texts).clone()),
        },
    }
}

/// `bytes` names whole characters of `text`.
fn on_boundaries(text: &str, bytes: &std::ops::Range<usize>) -> bool {
    bytes.start <= bytes.end
        && bytes.end <= text.len()
        && text.is_char_boundary(bytes.start)
        && text.is_char_boundary(bytes.end)
}

/// A glyph's span and bytes name a span of `spans` and whole characters
/// of its text, or the filler.
fn names_its_source(
    spans: &[FlowSpan<'_>],
    span: usize,
    bytes: &std::ops::Range<usize>,
) -> Result<(), String> {
    if span == FILLER_SPAN {
        return Ok(());
    }
    let Some(source) = spans.get(span) else {
        return Err(format!("a glyph names span {span} of {}", spans.len()));
    };
    if !on_boundaries(source.text, bytes) {
        return Err(format!(
            "a glyph names bytes {bytes:?} of span {span}, {:?}, which are not whole characters of it",
            source.text
        ));
    }
    Ok(())
}

/// The properties of a shaped run: every glyph names its source, every
/// metric is a number, and the glyphs are the buffer's.
pub fn check_run(spans: &[FlowSpan<'_>], run: &ShapedRun, what: &str) -> Result<(), String> {
    if !run.line_height.is_finite() || run.line_height < 0.0 {
        return Err(format!("{what}: line height {}", run.line_height));
    }
    let laid_out = run.layout_glyphs().count();
    if laid_out != run.glyphs.len() {
        return Err(format!(
            "{what}: {} metrics for {laid_out} laid-out glyphs",
            run.glyphs.len()
        ));
    }
    for glyph in &run.glyphs {
        names_its_source(spans, glyph.span, &glyph.bytes).map_err(|why| format!("{what}: {why}"))?;
        for (name, value) in [
            ("advance", glyph.advance),
            ("size", glyph.size),
            ("x", glyph.x),
            ("ascent", glyph.ascent),
            ("descent", glyph.descent),
        ] {
            if !value.is_finite() {
                return Err(format!("{what}: a glyph's {name} is {value}"));
            }
        }
    }
    if !run.baseline().is_finite() {
        return Err(format!("{what}: the baseline is {}", run.baseline()));
    }
    Ok(())
}

/// The properties of a placement and the run it refers to.
fn check_placed(
    spans: &[FlowSpan<'_>],
    run: &ShapedRun,
    (replaced, placed): (Option<ShapedRun>, PlacedRun),
    whole: bool,
    what: &str,
) -> Result<(), String> {
    let refers_to = match &replaced {
        Some(new) => {
            check_run(spans, new, &format!("{what}: the run placement made"))?;
            new
        }
        None => run,
    };
    check_placement(refers_to, &placed, whole, what)
}

/// No two of a frame's inks touch (rulings R38.1, R44.2): never overlap
/// wins over never cut, a spent budget included. What its junctions and
/// sides leave unheld is reported in the trace, never silent.
pub(crate) fn apart(trace: &Trace, what: &str) -> Result<(), String> {
    if trace.unheld.overlap {
        return Err(format!("{what}: two inks touch ({:?})", trace.unheld));
    }
    Ok(())
}

/// The run's natural length, every cluster as long as the longer of its
/// advance and the line height — what the longest step the walk takes
/// for it is, a steep one — and so no shorter than any lay of it at no
/// gap.
fn natural_length(run: &ShapedRun) -> f32 {
    let height = run.line_height.max(0.0);
    run.glyphs
        .iter()
        .filter(|g| g.span != FILLER_SPAN)
        .map(|g| g.advance.max(0.0).max(height))
        .sum()
}

/// Whether `fit` on a path of room `room` owes the whole run: a
/// `Spread` or `Fit` whose natural length fits — for `Fit` at its least
/// scale — places every cluster, and cuts nothing. Owed of every path
/// whose room is finite, as far from the origin as an `f32` reaches: a
/// glyph's step lost below an `f32`'s precision there moves it nowhere,
/// but it is placed.
fn owes_whole(run: &ShapedRun, fit: &FitMode, room: f32) -> bool {
    let length = natural_length(run);
    let least = match fit {
        FitMode::Spread => 1.0,
        FitMode::Fit { min, .. } => *min,
        _ => return false,
    };
    room.is_finite() && length.is_finite() && length * least <= room
}

/// The room a curve gives a run: its length, measured on a polyline
/// through it, which is no longer than it.
/// Summed in `f64`, so a curve longer than `√f32::MAX` measures what it
/// is rather than infinite.
fn curve_room(curve: &Curve) -> f32 {
    const SAMPLES: usize = 256;
    (0..SAMPLES)
        .map(|k| {
            let (a, b) = (k as f32 / SAMPLES as f32, (k + 1) as f32 / SAMPLES as f32);
            (curve.point(b) - curve.point(a)).as_dvec2().length()
        })
        .sum::<f64>() as f32
}

/// The room a frame round `outline` gives a run: the narrower of the
/// level spans R27 sizes its two lines against, which each half of the
/// outline holds, so that the whole run fits one line there.
fn frame_room(outline: &Outline) -> f32 {
    frame_halves(outline, MIN_TABLE_SPACING)
        .iter()
        .map(|(path, span)| path.at(span.end).0.x - path.at(span.start).0.x)
        .fold(f32::INFINITY, f32::min)
}

/// Every placed glyph is finite, names a glyph of `run`, and there are
/// at most [`MAX_PLACED_GLYPHS`]. With `whole` — a `Spread` or `Fit`
/// whose natural length fits its path (`owes_whole`) — nothing is cut:
/// every glyph of the run that draws (not a space, not a filler's) is
/// placed, and the placement is not truncated. A gap solved past every
/// `f32` that the walk then cut silently is caught here.
pub fn check_placement(run: &ShapedRun, placed: &PlacedRun, whole: bool, what: &str) -> Result<(), String> {
    if whole {
        if placed.truncated {
            return Err(format!("{what}: its natural length fits, and it was cut"));
        }
        let mut drawn: Vec<bool> = vec![false; run.glyphs.len()];
        for glyph in &placed.glyphs {
            if let Some(seen) = drawn.get_mut(glyph.glyph as usize) {
                *seen = true;
            }
        }
        if let Some(missing) = run
            .glyphs
            .iter()
            .zip(&drawn)
            .position(|(g, seen)| !seen && !g.blank && g.span != FILLER_SPAN)
        {
            return Err(format!(
                "{what}: its natural length fits, and glyph {missing} was not placed"
            ));
        }
    }
    if placed.glyphs.len() > MAX_PLACED_GLYPHS {
        return Err(format!("{what}: {} glyphs placed", placed.glyphs.len()));
    }
    for glyph in &placed.glyphs {
        if !glyph.pos.is_finite() {
            return Err(format!("{what}: a glyph placed at {}", glyph.pos));
        }
        if glyph.glyph as usize >= run.glyphs.len() {
            return Err(format!(
                "{what}: placed glyph {} of {}",
                glyph.glyph,
                run.glyphs.len()
            ));
        }
    }
    Ok(())
}

/// The properties of a shaped box: a finite, non-negative size at least
/// the usable minimum, and one source per laid-out glyph.
pub fn check_box(spans: &[FlowSpan<'_>], shaped: &ShapedBox, min: Option<Vec2>) -> Result<(), String> {
    let size = shaped.size;
    if !size.is_finite() || size.x < 0.0 || size.y < 0.0 {
        return Err(format!("a box of size {size}"));
    }
    if let Some(min) = min {
        for (axis, asked, got) in [("x", min.x, size.x), ("y", min.y, size.y)] {
            if asked.is_finite() && asked > got {
                return Err(format!(
                    "a box {got} on {axis}, below the minimum {asked} asked for"
                ));
            }
        }
    }
    let laid_out: usize = shaped.buffer.layout_runs().map(|run| run.glyphs.len()).sum();
    if laid_out != shaped.glyph_spans.len() {
        return Err(format!(
            "{} glyph sources for {laid_out} laid-out glyphs",
            shaped.glyph_spans.len()
        ));
    }
    for (span, bytes) in &shaped.glyph_spans {
        names_its_source(spans, *span, bytes).map_err(|why| format!("a box: {why}"))?;
    }
    Ok(())
}

/// The cluster `glyph` was shaped from, as text of its line.
fn cluster(line: &str, glyph: &crate::font::LayoutGlyph) -> String {
    line.get(glyph.start..glyph.end).unwrap_or_default().to_string()
}

/// Ruling R30's property: every glyph of a span pinned to a compiled
/// family is drawn in one of that family's compiled faces, unless its
/// cluster holds a character none of them has (which cosmic-text draws
/// in a fallback face, as it must). `glyphs` are the laid-out glyphs,
/// each with its face, the span its cluster starts in, and the cluster
/// as cosmic-text shaped it (the bytes of its line, which may run on
/// into the next span); a filler glyph ([`FILLER_SPAN`]) is not a
/// span's and is not checked. `Err` names the first glyph drawn in
/// another face.
///
/// Without the weight resolved within the family
/// ([`PinnedFamily::weight_for`](crate::font::fonts::PinnedFamily::weight_for)),
/// a span at a weight its family lacks is drawn in any host face of that
/// exact weight; the test build registers such faces on every machine
/// (`fonts::register_host_decoys`), so this property catches it there.
pub fn shapes_in_pinned_faces(
    spans: &[FlowSpan<'_>],
    glyphs: impl IntoIterator<Item = (cosmic_text::fontdb::ID, usize, String)>,
    fs: &mut crate::font::FontSystem,
) -> Result<(), String> {
    let pins: Vec<Option<crate::font::fonts::PinnedFamily>> = spans
        .iter()
        .map(|span| match span.style.font {
            AppFont::Any => None,
            font => crate::font::fonts::pinned_family(fs, font),
        })
        .collect();
    for (face, span, text) in glyphs {
        let Some(Some(pin)) = pins.get(span) else {
            continue;
        };
        if pin.faces.contains(&face) {
            continue;
        }
        let held = |ch: char| {
            // cosmic-text shapes a tab as a space.
            let ch = if ch == '\t' { ' ' } else { ch };
            pin.faces.iter().any(|&id| {
                fs.get_font(id, cosmic_text::fontdb::Weight(400))
                    .is_some_and(|font| font.as_swash().charmap().map(ch) != 0)
            })
        };
        if !text.is_empty() && text.chars().all(held) {
            let family = fs.db().face(face).map(|info| info.post_script_name.clone());
            return Err(format!(
                "span {span} ({:?}) is pinned to {:?} (weights {:?}), and its cluster {text:?}, which the family holds, was drawn in {family:?}",
                spans[span],
                pin.name, pin.weights
            ));
        }
    }
    Ok(())
}
