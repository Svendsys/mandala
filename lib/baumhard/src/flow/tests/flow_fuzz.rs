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
//!   R46, R47.2): each target lays one budgeted placement an input — a
//!   frame, a `Repeat` ring (#310), or a placement along an open path
//!   (#313) — whose budget is at most what the harness counts for it and
//!   whose work is at most that budget and the overshoot its weights allow
//!   (`frame_work_steps`, `ring_work_steps`), and the input's work
//!   together is at most
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
use crate::document::element::DirectionalGlyphs;
use crate::document::resolve::ResolvedText;
use crate::document::values::{
    Align, Direction, Edge, FitMode, FontSize, LineSpan, Percent, Rgba, Shape, StartPoint, Weight,
};
use crate::flow::place::halves::{frame_budget, frame_work_steps, table_steps, touch_looks, NEARER};
use crate::flow::place::pitch::place_pitched_capped;
use crate::flow::place::ring::{ring_work_steps, RING_STEPS};
use crate::flow::place::{
    frame_halves, place_on_closed_capped, place_on_open_capped, setup_table_samples, size_factor, Trace,
    INK_TOLERANCE, MIN_TABLE_SPACING,
};
use crate::flow::tests::support::raster_box;
use crate::flow::{
    offset, outline, place_at_point, shape_box, shape_run, Curve, FlowSpan, Outline, PathEnd, PlacedRun,
    Rect, RingLayout, ShapedBox, ShapedRun, FILLER_SPAN, MAX_PLACED_GLYPHS, MAX_TABLE_SAMPLES,
};
use crate::font::fonts::{self, AppFont, FontSystemLease, COMPILED_FONT_ID_MAP};
use crate::util::tests::choices::Choices;

/// The most characters a structured span holds: enough for a cut, a
/// repeat and a fit to have something to do, few enough that one input
/// shapes in milliseconds.
pub const MAX_SPAN_CHARS: u32 = 64;

/// The most glyphs a frame's text is shaped to here (rulings R44.2,
/// R45.1, R46.3): the most a frame of the lightest text — one face, no
/// glyph that no face holds, a short side ornament — may hold and still
/// be laid, to the whole of its budget and its overshoot past it
/// (`frame_work_steps`), within [`INPUT_STEPS`]: 152 glyphs, 3,000 glyph
/// steps each of budget and some 48 more of shaping, inking and lays,
/// 463,355 steps, 14.97 s at [`ASAN_STEP_NANOS`] a step (150 until #304
/// counted a reshape at a size as no shaping, some 65 a glyph more).
/// `test_each_flow_harness_lays_an_input_in_three_quarters_of_the_timeout`
/// holds it to that: the most, and no more. A heavier text — glyphs in
/// many faces — or a heavier ornament is cut further ([`framed`]). A longer text's frame is laid from as
/// much of it as fits: the engine places the whole of a real text,
/// bounded by its glyphs; a fuzz input is held to what it can lay
/// inside the timeout.
pub const MAX_FRAME_GLYPHS: usize = 152;

/// The most glyphs a `Repeat` ring the harnesses lay lays (rulings
/// B17.2a, B17.2b): the most whose budget, its overshoot past it for a
/// copy of [`MAX_FRAME_GLYPHS`] glyphs beside it (`ring_work_steps`), fits
/// [`INPUT_STEPS`] — the ring is an input's one budgeted placement — 154
/// glyphs, 462,479 steps. A ring's budget is scaled by the glyphs it will
/// lay, at most its cap: in a map 65,536, by which a ring far from the
/// origin, its glyphs reading its path a thousand times each, takes
/// 196,608,000 steps (seed 2771). The harnesses lay it with this cap
/// instead (`place_on_closed_capped`), its whole text and its whole path,
/// so its far-out floats are still walked, over fewer glyphs; cutting its
/// text cannot, since a path some 10²¹ long holds the cap's glyphs of any
/// copy. Its copy is a frame's text, cut to fit as a frame's
/// ([`framed`]), so one copy always fits the cap.
/// `test_each_flow_harness_lays_an_input_in_three_quarters_of_the_timeout`
/// holds it to that: the most, and no more. That is for a path of no
/// table, a rounded rectangle's or a circle's; a ring's table is counted
/// in its setup (ruling B16.3a), and `ring_cap` lowers the cap by it.
pub const MAX_RING_GLYPHS: usize = 154;

/// The cap a harness lays a `Repeat` ring of `copy` glyphs a copy with,
/// its path's table of `tables` samples (`setup_table_samples`): held to
/// the timeout (`bounded`), the most glyphs, [`MAX_RING_GLYPHS`] at most,
/// whose ring of a copy of [`MAX_FRAME_GLYPHS`] glyphs fits
/// [`INPUT_STEPS`] (`ring_work_steps`) — no fewer than the glyphs of the
/// longest copy a frame's cut leaves beside a table of as many samples
/// ([`fits_in_time`]), so one copy always fits the cap still
/// (`test_a_rings_cap_holds_the_copy_its_tables_leave`); replaying a finding, as many glyphs as one copy at least, as a
/// finding's frame text is laid as found ([`FOUND_FRAME_GLYPHS`]), so
/// that its ring is laid once whole.
pub(crate) fn ring_cap(copy: usize, bounded: bool, tables: usize) -> usize {
    if bounded {
        (0..=MAX_RING_GLYPHS)
            .rev()
            .find(|&cap| ring_work_steps(cap, MAX_FRAME_GLYPHS, tables) <= INPUT_STEPS)
            .unwrap_or(0)
    } else {
        MAX_RING_GLYPHS.max(copy)
    }
}

/// The samples of the arc-length tables a harness counts a path of
/// `curves` with before it is laid (#313): the most one curve's table
/// holds, [`MAX_TABLE_SAMPLES`] and its closing sample, for each curve but
/// a line, which builds none (`ArcPath::open`). What a placement along it
/// counts in its setup is held to no more ([`InputWork::open`],
/// [`InputWork::pitched`]).
pub(crate) fn open_tables(curves: &[Curve]) -> usize {
    curves
        .iter()
        .filter(|curve| !matches!(curve, Curve::Line(..)))
        .count()
        .saturating_mul(MAX_TABLE_SAMPLES + 1)
}

/// The cap a harness lays a placement along an open path of `copy` glyphs
/// with, its path's tables of `tables` samples at most (#313, as ruling
/// B17.2b has it for a ring): held to the timeout (`bounded`), the most
/// glyphs, [`MAX_RING_GLYPHS`] at most, whose placement — its budget
/// scaled by them and what it may take past it, a ring's
/// (`ring_work_steps`), the one cluster that may find it spent no wider
/// than its whole text — fits [`INPUT_STEPS`] as an input's one budgeted
/// placement; `None` where not even none does. Its whole text and path
/// are laid, so a path far from the origin is walked there, over fewer
/// glyphs. Replaying a finding, [`MAX_PLACED_GLYPHS`], as a map lays it.
pub(crate) fn open_cap(copy: usize, tables: usize, bounded: bool) -> Option<usize> {
    if !bounded {
        return Some(MAX_PLACED_GLYPHS);
    }
    (0..=MAX_RING_GLYPHS)
        .rev()
        .find(|&cap| ring_work_steps(cap, copy, tables) <= INPUT_STEPS)
}

/// The glyph steps a whole-glyph placement along an open path
/// (`place_pitched`) at most `cap` glyphs, its path's tables of `tables`
/// samples at most, may take (#313): its budget, and its setup and its
/// tables, counted before it first reads its budget.
pub(crate) fn pitched_work_steps(cap: usize, tables: usize) -> usize {
    frame_budget(cap)
        .saturating_add(RING_STEPS)
        .saturating_add(table_steps(tables))
}

/// The cap a harness lays a whole-glyph placement along an open path with,
/// its path's tables of `tables` samples at most (#313): held to the
/// timeout (`bounded`), the most glyphs, [`MAX_RING_GLYPHS`] at most, whose
/// placement may take no more than fits [`INPUT_STEPS`]
/// ([`pitched_work_steps`]) as an input's one budgeted placement; `None`
/// where not even none does. Replaying a finding, [`MAX_PLACED_GLYPHS`].
pub(crate) fn pitched_cap(tables: usize, bounded: bool) -> Option<usize> {
    if !bounded {
        return Some(MAX_PLACED_GLYPHS);
    }
    (0..=MAX_RING_GLYPHS)
        .rev()
        .find(|&cap| pitched_work_steps(cap, tables) <= INPUT_STEPS)
}

/// The most glyphs a frame's text was shaped to when the committed flow
/// findings were found (ruling R44.2): what their replays lay, so that
/// each still exercises the frame it was found in
/// ([`check_flow_bytes_in_every_fit`], [`check_flow_structured_as_found`]).
pub const FOUND_FRAME_GLYPHS: usize = 320;

/// The frames flow_raw lays an input round its rectangle, each in a fit
/// mode that tracks its halves or as a `Repeat` ring ([`raw_frame_fits`];
/// ruling R45.1). One: a frame's budget is at least 400,000 glyph steps
/// whatever its text (`TRACK_BUDGET`), so two frames may take 800,000
/// steps, 25.8 s at [`ASAN_STEP_NANOS`] a step, past
/// [`INPUT_WORK_MICROS`] however few glyphs they share — and three, as
/// before R45, 38.8 s. A ring's budget is a frame's (#310), so the ring
/// the target laid besides its frame until then is one of the modes it
/// draws; and since #313 so is each placement along its line, which it
/// draws in place of its frame ([`raw_line`]).
pub const RAW_FRAMES: usize = 1;

/// The frames flow_structured lays an input: one, round its outline,
/// when it draws an outline at all — a `Repeat` ring among them, which is
/// budgeted as a frame is (#310). Where it draws a curve, it lays one
/// placement along it instead, budgeted so too (#313).
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
/// fit it: a frame counts a pass of the shaper through every face,
/// `SHAPE_NOTDEF_STEPS`, 2,200 steps, for each distinct one-code-point
/// glyph no face holds, and one a glyph of a line holding a `.notdef`
/// cluster of several code points (#304), by which a 4,096-byte raw input
/// of some 1,365 distinct private-use characters would shape for some
/// 48 s.
/// The weight bounds a pass; a pass for one code point shapes it alone in
/// each face, once a font system, and the published shaper's fallback,
/// which shaped the whole segment in each face, was measured at some
/// 6 ms a segment, and 0.15 to 0.3 ms a `.notdef` past the first, in a
/// debug build (review 5739829769). Until #304 a frame counted the pass
/// for every `.notdef`, a run of them some 30× over.
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

/// Whether a frame's text shaped as `run`, its sides as `ornament`, its
/// paths' tables of `tables` samples (`setup_table_samples`, ruling
/// B16.3a), is one the harnesses lay (rulings R45.1, R46.3): at most
/// [`MAX_FRAME_GLYPHS`] glyphs, and laid to the whole of its budget and
/// past it (`frame_work_steps`, a `Fit` frame's as the most) within its
/// share of [`INPUT_STEPS`] among `frames` frames.
pub fn fits_in_time(run: &ShapedRun, ornament: Option<&ShapedRun>, frames: usize, tables: usize) -> bool {
    run.glyphs.len() <= MAX_FRAME_GLYPHS
        && frame_work_steps(run, ornament, true, tables) <= frame_steps(frames.max(1))
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
    /// `sized`, its paths' tables of `tables` samples), laid with `trace`:
    /// the most it may take, `frame_work_steps`. The engine counts the
    /// tables its setup builds, as many samples as `setup_table_samples`
    /// said before it was laid (ruling B16.3a), scales a frame's budget
    /// by its text's glyphs once cut, never more than the run has, spends
    /// no more than its budget and its overshoot, and no touch check of it
    /// looks at more than `touch_looks` pairs a box of the classes of
    /// height it holds, nor a side arc's check holds more classes than its
    /// arc held back for — the two that overshoot rests on (rulings R48.2,
    /// R51.2): properties.
    pub(crate) fn frame(
        &mut self,
        run: &ShapedRun,
        ornament: Option<&ShapedRun>,
        sized: bool,
        tables: usize,
        trace: &Trace,
        what: &str,
    ) -> Result<(), String> {
        let budget = frame_budget(run.glyphs.len());
        let most = frame_work_steps(run, ornament, sized, tables);
        if trace.tables != tables {
            return Err(format!(
                "{what}: a frame's setup counted tables of {} samples, not the {tables} its outline gives (ruling B16.3a)",
                trace.tables
            ));
        }
        if !trace.looks.within() {
            let (looks, boxes, classes) = trace.looks.worst;
            return Err(format!(
                "{what}: a touch check looked at {looks} pairs of {boxes} boxes of {classes} classes of height, past {} a box (rulings R48.2, R51.2)",
                touch_looks(classes)
            ));
        }
        if trace.looks.classes > trace.looks.reserved {
            return Err(format!(
                "{what}: a side arc's touch check held {} classes of height, past the {} its arc held back for (ruling R51.2)",
                trace.looks.classes, trace.looks.reserved
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

    /// Count a `Repeat` ring of copies of `run`, laid at most `cap`
    /// glyphs as `placed`, with `trace`: the most it may take,
    /// `ring_work_steps` of its cap (#310, rulings B17.1a, B17.2b). The
    /// engine scales a ring's budget by the glyphs it will lay, never more
    /// than its cap, places no more than its cap, spends no more than its
    /// budget and the one cluster past it that found it spent, and reports
    /// a ring its cap cut short: properties. A ring cut
    /// (`PlacedRun::truncated`) that kept a whole copy was stopped by its
    /// cap, and says so: the one cut that reports nothing is the path's,
    /// of a ring too short for one copy, which keeps less than a copy. A
    /// ring its budget stopped after whole copies is not cut — it keeps
    /// them, unclosed — so this does not see it; the ring grid's silence
    /// check does (`ring_budget_tests`, ruling B17.1c).
    pub(crate) fn ring(
        &mut self,
        run: &ShapedRun,
        cap: usize,
        tables: usize,
        placed: &PlacedRun,
        trace: &Trace,
        what: &str,
    ) -> Result<(), String> {
        let copy = run.glyphs.len();
        let budget = frame_budget(cap);
        let most = ring_work_steps(cap, copy, tables);
        if trace.tables != tables {
            return Err(format!(
                "{what}: a ring's setup counted a table of {} samples, not the {tables} its outline gives (ruling B16.3a)",
                trace.tables
            ));
        }
        if trace.glyphs > cap || trace.budget > budget {
            return Err(format!(
                "{what}: a ring budgeted {} glyph steps for {} glyphs, past the {budget} its cap of {cap} gives",
                trace.budget, trace.glyphs
            ));
        }
        if trace.spent > most {
            return Err(format!(
                "{what}: a ring took {} glyph steps, past its budget of {} and its overshoot, {most} in all ({:?})",
                trace.spent, trace.budget, trace.ring
            ));
        }
        if placed.glyphs.len() > cap {
            return Err(format!(
                "{what}: a ring placed {} glyphs, past its cap of {cap} (ruling B17.2a)",
                placed.glyphs.len()
            ));
        }
        if placed.truncated && copy > 0 && placed.glyphs.len() >= copy && trace.ring.is_none() {
            return Err(format!(
                "{what}: a ring cut at {} glyphs of {copy} a copy, its cap {cap}, reported nothing: capped silently",
                placed.glyphs.len()
            ));
        }
        self.frames += 1;
        self.steps += most;
        Ok(())
    }

    /// Count a placement of `run` along an open path, laid at most `cap`
    /// glyphs as `placed`, with `trace`, its path's tables of `tables`
    /// samples at most ([`open_tables`]): the most it may take,
    /// `ring_work_steps` of its cap (#313, as a ring's). The engine
    /// counts tables of no more samples, scales its budget by no more
    /// than its cap, places no more than its cap, and spends no more than
    /// its budget and the one cluster past it that found it spent:
    /// properties. Its silence is the engine's grid's to hold
    /// (`open_budget_tests`): a placement along an open path is cut by its
    /// path too, which says nothing.
    pub(crate) fn open(
        &mut self,
        run: &ShapedRun,
        cap: usize,
        tables: usize,
        placed: &PlacedRun,
        trace: &Trace,
        what: &str,
    ) -> Result<(), String> {
        let copy = run.glyphs.len();
        let budget = frame_budget(cap);
        if trace.tables > tables {
            return Err(format!(
                "{what}: a placement's setup counted tables of {} samples, past the {tables} its path's curves hold",
                trace.tables
            ));
        }
        if trace.glyphs > cap || trace.budget > budget {
            return Err(format!(
                "{what}: a placement budgeted {} glyph steps for {} glyphs, past the {budget} its cap of {cap} gives",
                trace.budget, trace.glyphs
            ));
        }
        let most = ring_work_steps(cap, copy, trace.tables);
        if trace.spent > most {
            return Err(format!(
                "{what}: a placement took {} glyph steps, past its budget of {} and its overshoot, {most} in all ({:?})",
                trace.spent, trace.budget, trace.ring
            ));
        }
        if placed.glyphs.len() > cap {
            return Err(format!(
                "{what}: a placement placed {} glyphs, past its cap of {cap}",
                placed.glyphs.len()
            ));
        }
        self.frames += 1;
        self.steps += ring_work_steps(cap, copy, tables);
        Ok(())
    }

    /// Count a whole-glyph placement along an open path (`place_pitched`),
    /// laid at most `cap` glyphs as `placed`, with `trace`, its path's
    /// tables of `tables` samples at most ([`open_tables`]): the most it
    /// may take, [`pitched_work_steps`] of its cap (#313). The engine counts
    /// tables of no more samples, scales its budget by no more than its
    /// cap, places no more than its cap, spends no more than its budget but
    /// for its setup and its tables, and reports a placement its cap held:
    /// properties.
    pub(crate) fn pitched(
        &mut self,
        cap: usize,
        tables: usize,
        placed: &PlacedRun,
        trace: &Trace,
        what: &str,
    ) -> Result<(), String> {
        let budget = frame_budget(cap);
        if trace.tables > tables {
            return Err(format!(
                "{what}: a placement's setup counted tables of {} samples, past the {tables} its path's curves hold",
                trace.tables
            ));
        }
        if trace.glyphs > cap || trace.budget > budget {
            return Err(format!(
                "{what}: a placement budgeted {} glyph steps for {} glyphs, past the {budget} its cap of {cap} gives",
                trace.budget, trace.glyphs
            ));
        }
        let setup = RING_STEPS.saturating_add(table_steps(trace.tables));
        if trace.spent > trace.budget.max(setup) {
            return Err(format!(
                "{what}: a placement took {} glyph steps, past its budget of {} and its setup's {setup} ({:?})",
                trace.spent, trace.budget, trace.ring
            ));
        }
        if placed.glyphs.len() > cap {
            return Err(format!(
                "{what}: a placement placed {} glyphs, past its cap of {cap}",
                placed.glyphs.len()
            ));
        }
        if placed.truncated && trace.ring.is_none() {
            return Err(format!(
                "{what}: a placement held to its cap of {cap} at {} glyphs reported nothing: capped silently",
                placed.glyphs.len()
            ));
        }
        self.frames += 1;
        self.steps += pitched_work_steps(cap, tables);
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
    fs: &mut FontSystemLease,
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
/// It lays one budgeted placement an input, so that the input's work is
/// held inside the fuzz timeout (ruling R45.1): [`RAW_FRAMES`] frames
/// round its rectangle, in fit modes drawn from the bytes
/// ([`raw_frame_fits`]) — a `Repeat` ring among them, which is budgeted as
/// a frame is (#310) — or, drawn in their place, one placement along its
/// line ([`raw_line`]), each budgeted as a ring is since #313.
/// [`check_flow_bytes_in_every_fit`] lays all three frames, the ring and
/// every placement along the line, as a finding's replay does.
pub fn check_flow_bytes(bytes: &[u8]) -> Result<(), String> {
    let line = raw_line(bytes);
    let fits = if line.is_some() {
        Vec::new()
    } else {
        raw_frame_fits(bytes)
    };
    check_raw(bytes, &fits, line.as_ref(), true)
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
    check_raw(bytes, &frame_fits(), None, false)
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
/// the rest follow it in turn — or, one time in four, a `Repeat` ring,
/// budgeted as a frame is (#310), is laid in their place, so that an
/// input lays one budgeted placement round its rectangle: two would pass
/// the timeout. An input whose choices run out before it — every
/// committed raw finding but the ones that spell a layout — draws `Fit`,
/// the mode most of them were found in.
pub fn raw_frame_fits(bytes: &[u8]) -> Vec<FitMode> {
    let mut choices = Choices::new(bytes);
    ring_layout(&mut choices);
    let fits = frame_fits();
    let first = choices.below(fits.len() as u32) as usize;
    // The ring in place of the frame one time in four, drawn after it so
    // that every input draws the frame it drew before (#310).
    if choices.one_in(4) {
        return vec![FitMode::Repeat; RAW_FRAMES];
    }
    (0..RAW_FRAMES)
        .map(|k| fits[(first + k) % fits.len()].clone())
        .collect()
}

/// A placement flow_raw lays along its line (#313): a fit mode's, or the
/// whole-glyph pattern's (`place_pitched`).
#[derive(Clone, Debug, PartialEq)]
pub enum RawLine {
    /// The run along the line in this fit mode.
    Open(FitMode),
    /// The run as whole glyphs along the line, as a connector's ornament.
    Pitched,
}

/// The placements flow_raw lays along its line: `Repeat`, each of
/// [`frame_fits`], and the whole-glyph pattern.
pub fn raw_lines() -> Vec<RawLine> {
    std::iter::once(RawLine::Open(FitMode::Repeat))
        .chain(frame_fits().into_iter().map(RawLine::Open))
        .chain(std::iter::once(RawLine::Pitched))
        .collect()
}

/// The placement flow_raw lays along its line in place of its frame, if
/// any (#313): each placement along it is budgeted as a ring is, and an
/// input lays one budgeted placement (ruling R45.1) — so one time in four,
/// where it draws no ring, it lays one of [`raw_lines`], drawn after the
/// frame and the ring ([`raw_frame_fits`]), and no frame. An input whose
/// choices run out before it — every committed raw finding — lays its
/// frame, as it did.
pub fn raw_line(bytes: &[u8]) -> Option<RawLine> {
    let mut choices = Choices::new(bytes);
    ring_layout(&mut choices);
    choices.below(frame_fits().len() as u32);
    if choices.one_in(4) || !choices.one_in(4) {
        return None;
    }
    let lines = raw_lines();
    lines.get(choices.below(lines.len() as u32) as usize).cloned()
}

/// [`check_flow_bytes`]'s body, its frames laid in `fits` and, along its
/// line, `line`, held to the per-input work bound with `bounded`;
/// without, replaying a finding, laid in [`RingLayout::whole`] as well as
/// the layout its bytes draw, and every placement along its line.
pub(crate) fn check_raw(
    bytes: &[u8],
    fits: &[FitMode],
    line: Option<&RawLine>,
    bounded: bool,
) -> Result<(), String> {
    let text = String::from_utf8_lossy(bytes);
    let style = plain_style();
    let spans = [FlowSpan {
        text: &text,
        style: &style,
        link: None,
    }];
    fonts::init();
    let mut fs = FontSystemLease::new("flow fuzz");
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
    // The input's work, its frames' and what it lays along its line
    // together (ruling R45.1, #313).
    let mut work = InputWork::default();
    if !fits.is_empty() || !bounded {
        for layout in &layouts {
            frames_round_a_rectangle(&text, &style, layout, fits, bounded, &mut work, &mut fs)
                .map_err(|why| format!("{why}, laid {layout:?}"))?;
        }
    }
    // Along its line: held to the timeout, the one placement drawn in
    // place of its frame (#313); replaying a finding, every one.
    let curve = Curve::Line(Vec2::ZERO, Vec2::new(200.0, 0.0));
    let lines = if bounded {
        line.into_iter().cloned().collect()
    } else {
        raw_lines()
    };
    for laid in lines {
        match laid {
            RawLine::Open(fit) => check_open(
                &spans, &run, &curve, &fit, 1.0, bounded, &style, &mut work, &mut fs,
            )
            .map_err(|why| format!("along a line: {why}"))?,
            RawLine::Pitched => check_pitched(&run, &[curve], 1.0, bounded, &mut work)?,
        }
    }
    if bounded {
        work.within_the_timeout()?;
    }
    check_placement(&run, &place_at_point(&run, Vec2::ZERO), false, "at a point")
}

/// `run` of `spans` laid along `curve` in `fit` with `spacing`, at the cap
/// that holds it to the timeout with `bounded` ([`open_cap`]), counted in
/// `work` ([`InputWork::open`], #313): the placement's properties
/// ([`check_placed`]), a text a placement its budget or its cap stopped
/// owed no longer whole — it says so (`Trace::ring`). Laid at none where
/// no cap holds it.
#[allow(clippy::too_many_arguments)] // A placement's inputs, and the work it is counted in.
fn check_open(
    spans: &[FlowSpan<'_>],
    run: &ShapedRun,
    curve: &Curve,
    fit: &FitMode,
    spacing: f32,
    bounded: bool,
    flow: &ResolvedText,
    work: &mut InputWork,
    fs: &mut FontSystemLease,
) -> Result<(), String> {
    let tables = open_tables(std::slice::from_ref(curve));
    let Some(cap) = open_cap(run.glyphs.len(), tables, bounded) else {
        return Ok(());
    };
    let mut trace = Trace::default();
    let placed = place_on_open_capped(run, curve, fit, spacing, flow, fs, cap, Some(&mut trace));
    work.open(run, cap, tables, &placed.1, &trace, "the placement")?;
    // `Spread` and `Fit` solve the gap; the spacing does not enter
    // (ruling R6, reading 7).
    let whole = owes_whole(run, fit, curve_room(curve)) && trace.ring.is_none();
    check_placed(spans, run, placed, whole, "the placement")
}

/// `run` laid as whole glyphs along `path` and on along a line past its
/// end (#296), a box at each end of the chain to keep its gap from, the
/// start's half a line height back from the path's start: the
/// placement's properties ([`check_placement`]). It draws no choice of
/// its own, so a committed seed draws what it drew before. It is laid at
/// the cap that holds it to the timeout with `bounded` ([`pitched_cap`]),
/// and counted in `work` ([`InputWork::pitched`], #313); at none where no
/// cap holds it.
fn check_pitched(
    run: &ShapedRun,
    path: &[Curve],
    spacing: f32,
    bounded: bool,
    work: &mut InputWork,
) -> Result<(), String> {
    let (Some(first), Some(last)) = (path.first(), path.last()) else {
        return Ok(());
    };
    let (start, end) = (first.point(0.0), last.point(1.0));
    let mut chain = path.to_vec();
    chain.push(Curve::Line(end, end + Vec2::new(50.0, 25.0)));
    let side = Vec2::splat(run.line_height.max(1.0));
    let boxes = [
        [Rect::new(start - side * 1.5, side)],
        [Rect::new(end + Vec2::new(50.0, 25.0), side)],
    ];
    let ends = [
        PathEnd {
            inks: &boxes[0],
            back: 0.5 * run.line_height,
            hold: 0.0,
        },
        PathEnd {
            inks: &boxes[1],
            back: spacing,
            hold: run.line_height,
        },
    ];
    let tables = open_tables(&chain);
    let Some(cap) = pitched_cap(tables, bounded) else {
        return Ok(());
    };
    let mut trace = Trace::default();
    #[cfg(test)]
    let before = crate::flow::path::PATH_LOOKUPS.with(std::cell::Cell::get);
    let placed = place_pitched_capped(run, &chain, spacing, ends, cap, Some(&mut trace));
    work.pitched(cap, tables, &placed, &trace, "as whole glyphs")?;
    #[cfg(test)]
    PITCHED_LOOKUPS.with(|count| {
        count.set(count.get() + crate::flow::path::PATH_LOOKUPS.with(std::cell::Cell::get) - before)
    });
    #[cfg(test)]
    PITCHED_GLYPHS.with(|count| count.set(count.get() + placed.glyphs.len()));
    check_placement(run, &placed, false, "as whole glyphs")
}

#[cfg(test)]
thread_local! {
    /// Test counter (#333): the path lookups the whole-glyph placements
    /// [`check_pitched`] lays on this thread have made, so that a test of
    /// another placement's work can hold the two apart.
    pub(crate) static PITCHED_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#333): the glyphs those placements placed.
    pub(crate) static PITCHED_GLYPHS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
/// replaying a finding, to what a frame laid when it was found — and
/// counted in `work`, the input's, which its caller holds to the timeout.
#[allow(clippy::too_many_arguments)] // Its frames' inputs, and the work they are counted in.
fn frames_round_a_rectangle(
    text: &str,
    style: &ResolvedText,
    layout: &RingLayout,
    fits: &[FitMode],
    bounded: bool,
    work: &mut InputWork,
    fs: &mut FontSystemLease,
) -> Result<(), String> {
    let rect = Rect::new(Vec2::ZERO, Vec2::new(120.0, 40.0));
    let path = outline(
        rect,
        Shape::Rectangle {
            radius: Percent::new(50),
        },
    );
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    // Held to the timeout, the frames drawn, a `Repeat` ring among them
    // (#310); replaying a finding, a ring and every frame, as found.
    let laid: Vec<&FitMode> = if bounded {
        fits.iter().collect()
    } else {
        std::iter::once(&FitMode::Repeat).chain(fits).collect()
    };
    // The tables its frames' setups build, the most of any it lays
    // (ruling B16.3a): none, round a rounded rectangle, so they are not
    // held to them here — that is the structured target's, whose
    // outlines are ellipses too.
    let tables = |run: &ShapedRun| {
        laid.iter()
            .map(|fit| setup_table_samples(run, &path, fit, style))
            .max()
            .unwrap_or(0)
    };
    // Its sides, then its text, held to what its frames lay inside the
    // timeout (rulings R45.1, R50.3).
    let (layout, ornament) = if bounded {
        sides_in_time(layout, style, 1.0, fits.len(), &tables, fs)
    } else {
        (layout.clone(), ornament_run(layout, style, 1.0, fs))
    };
    let layout = &layout;
    let fits_in_time = |run: &ShapedRun| fits_in_time(run, ornament.as_ref(), fits.len(), tables(run));
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
    for fit in laid {
        let tables = setup_table_samples(&frame_run, &path, fit, style);
        // A ring's cap (rulings B17.2a, B17.2b): what fits the timeout, or
        // replaying a finding, a copy at least.
        let cap = ring_cap(frame_run.glyphs.len(), bounded, tables);
        let mut trace = Trace::default();
        let placed = place_on_closed_capped(
            &frame_run,
            &path,
            start,
            Direction::Clockwise,
            fit,
            layout,
            true,
            style,
            fs,
            cap,
            Some(&mut trace),
        );
        if matches!(fit, FitMode::Repeat) {
            work.ring(&frame_run, cap, tables, &placed.1, &trace, "around a rectangle")?;
        } else {
            let sized = matches!(fit, FitMode::Fit { .. });
            work.frame(
                &frame_run,
                ornament.as_ref(),
                sized,
                tables,
                &trace,
                "around a rectangle",
            )?;
        }
        // A line cut so that no two inks touch, and reported so, is owed
        // no longer (ruling R38.1); a frame its sides leave unheld is
        // reported (ruling R35.2).
        let whole = owes_whole(&frame_run, fit, frame_room(&path)) && trace.cut_apart.is_empty();
        apart(&trace, "around a rectangle")?;
        check_frame_placed(
            &frame_spans,
            &frame_run,
            placed,
            (whole, &trace),
            "around a rectangle",
            fs,
        )?;
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
    check_generated(&mut Choices::new(bytes), false, &faces())
}

/// [`check_flow_structured`] for choices from any source.
pub fn check_flow_generated(choices: &mut Choices<'_>) -> Result<(), String> {
    check_generated(choices, true, &faces())?;
    check_pair_model(choices)
}

/// The frame's closed-form layer held to its oracles (#309, ruling
/// B16.1b): the tangent line's root to the iterative rule's bisection of
/// it ([`check_line_root`]); and a letter after two inks on an outline the
/// choices draw — a sharp or rounded rectangle, an ellipse, at normal
/// scale, the band curve on it, in it or off it — at a gap they draw,
/// nothing one time in eight, to the path's own gap function (the seam's
/// `PairSeam::curve`: the letter set on the path at a center, its gap to
/// the ink read back) and to bounds about what is owed taken from this
/// property's own scale (review 5810838863). Where the ink before it lies
/// on one form the model reads, a side, an arc or a sharp corner's sweep,
/// the model's root sets the letter at what is owed (ruling B16.2c), read
/// back from the path, to within the float error of positions some
/// thousands of points out; where nothing is owed, touching to that error
/// or a few of it past. And it is the first root: set past the ink, with
/// no sample of the band curve before it already past that window.
///
/// The letter is not held to the iterative rule's answer (review
/// 5811563882): the center `pair` sets it at from its straight path's warm
/// start, the first probe inside its window. That answer cannot be the
/// oracle:
/// - Its window, from `NEARER` short of what is owed to `INK_TOLERANCE`
///   past, holds a stretch of centers, not one. Where the gap rises slowly
///   with the center, two right answers lie further apart than the
///   window: on a 40 × 40 rounded rectangle's side at seed 19466 the gap
///   rises 0.19 a unit of center, and the model, at 60.17368 (gap
///   7.9679947, owed 7.968), and the rule, at 60.178894 (gap 7.968998),
///   are 5.2e-3 apart.
/// - It need not take the first root. Round a 40 × 40 circle at seed
///   90637 the model sets the letter at 28.46 (gap 2.2079992, owed
///   2.208), the first crossing, and the rule at 36.72 (gap 2.2077866),
///   the third: between them the gap falls to 1.9965, 0.21 below what is
///   owed, where a letter would stand nearer its ink than it is owed.
///   Held to the rule there, the model would be held to the wrong root.
///
/// The rule's answer is read only to count the holds on a side, where its
/// gap lies in its own window too: the precondition of the sweep over this
/// (`test_the_pair_model_sets_the_first_root_at_what_is_owed`).
///
/// The first root is held at the curve's samples, 63 evenly between the
/// last ink and the further of the two centers: a crossing before the
/// model's narrower than their spacing, a 64th of that span — some 0.3 pt
/// on a 20 pt span — can lie between two samples unseen.
pub fn check_pair_model(choices: &mut Choices<'_>) -> Result<(), String> {
    check_line_root(choices)?;
    pair_model_held(choices).map(|_| ())
}

/// The tangent line's root in closed form held to the iterative rule's
/// bisection of it as its oracle (#309, ruling B16.1b; `line_root`,
/// `along`): two inks some way apart, their half sizes summing to a reach
/// the choices draw, one stepped along a way they draw until the two are
/// owed apart — on where they are nearer, back where they are further.
/// The gap along a line is convex in the step, so each way has one root:
/// where the bisection brackets one, the closed form finds it, to the
/// bracket's width; and where only the closed form does, the boxes are
/// owed apart there.
pub fn check_line_root(choices: &mut Choices<'_>) -> Result<(), String> {
    use crate::flow::place::halves::line_root_seam;
    let unit = |choices: &mut Choices<'_>| choices.below(1001) as f32 / 1000.0;
    let delta = Vec2::new(unit(choices) - 0.5, unit(choices) - 0.5) * 40.0;
    let angle = unit(choices) * std::f32::consts::TAU;
    let dir = Vec2::new(angle.cos(), angle.sin()) * (0.25 + 2.0 * unit(choices));
    let reach = Vec2::new(unit(choices), unit(choices)) * 12.0;
    let owed = if choices.below(8) == 0 {
        0.0
    } else {
        unit(choices) * 12.0
    };
    let (closed, bisected) = line_root_seam(delta, dir, reach, owed);
    let what = format!("inks {delta} apart, reach {reach}, stepped along {dir}, owed {owed}");
    let gap = |d: f32| ((delta + dir * d).abs() - reach).max(Vec2::ZERO).length();
    // The bisection's last bracket, and the floats of the step and the gap.
    let width = (bisected.abs() + owed + reach.max_element()).max(INK_TOLERANCE) * 1e-5;
    match (closed.is_finite(), bisected.is_finite()) {
        (true, true) if (closed - bisected).abs() <= width => Ok(()),
        (false, true) => Err(format!(
            "{what}: the bisection steps {bisected}, the closed form finds none"
        )),
        (true, false) if owed > 0.0 && (gap(closed) - owed).abs() > 1e-4 * (owed + 1.0) => Err(format!(
            "{what}: the closed form steps {closed}, where the gap is {}",
            gap(closed)
        )),
        (true, true) => Err(format!(
            "{what}: the closed form steps {closed}, the bisection {bisected}"
        )),
        _ => Ok(()),
    }
}

/// [`check_pair_model`], and which of its holds it reached: the model's
/// window and first root, and those on a side, where the rule's gap is in
/// its own window too (read for that count alone).
pub(crate) fn pair_model_held(choices: &mut Choices<'_>) -> Result<[bool; 2], String> {
    use crate::flow::place::halves::pair_seam;
    let sizes = [40.0_f32, 120.0, 600.0, 2_000.0];
    let (w, h) = (sizes[choices.below(4) as usize], sizes[choices.below(4) as usize]);
    let rect = crate::flow::Rect::new(Vec2::ZERO, Vec2::new(w, h));
    let shape = match choices.below(3) {
        0 => Shape::Ellipse,
        k => Shape::Rectangle {
            radius: crate::document::values::Percent::new(if k == 1 { 0 } else { 50 }),
        },
    };
    let outline = crate::flow::outline(rect, shape);
    let unit = |choices: &mut Choices<'_>| choices.below(1001) as f32 / 1000.0;
    let offset = (unit(choices) - 0.3) * 0.2 * w.min(h);
    let k = choices.below(2) as usize;
    let length = 2.0 * (w + h);
    let earlier = unit(choices) * 0.4 * length;
    let last = earlier + 0.5 + unit(choices) * 20.0;
    let size = |choices: &mut Choices<'_>| Vec2::new(0.5 + unit(choices) * 8.0, 0.5 + unit(choices) * 10.0);
    let (before, half) = (size(choices), size(choices));
    let rise = (unit(choices) - 0.5) * 8.0;
    // Nothing owed one time in eight, as `check_line_root` draws it: a
    // letter owed nothing is its own case (review 5810838863), which
    // `unit` alone draws one time in 1,001.
    let owed = if choices.below(8) == 0 {
        0.0
    } else {
        unit(choices) * 12.0
    };
    let found = pair_seam(&outline, k, offset, (earlier, last), (before, half), rise, owed);
    let what = format!(
        "{w} × {h} {shape:?} half {k} offset {offset}: inks at {earlier} and {last} ({before}), a letter {half} rising {rise}, owed {owed}"
    );
    let window = |gap: f32| gap > 0.0 && gap >= owed - NEARER && gap <= owed + INK_TOLERANCE;
    // A gap measured from inks set some thousands of points out carries the
    // float error of their positions: a few units of the outline's last
    // place, past the rule's own window.
    let resolution = 4.0 * f32::EPSILON * (w.max(h) + last);
    // The model's window, from this property's own scale, not from what
    // the model aims at (review 5810838863; TEST_CONVENTIONS §T13.6). The
    // model measures gaps to 4ε of its own scale — the last ink's
    // coordinates, the reach and what is owed: at most 1.14 × max(w, h) + 27
    // over these ranges, against this one's max(w, h) + 0.5 at least, so at
    // most 1.8 of this resolution, worst at 40 — and accepts a gap within
    // that of its aim; the gap read back from the path adds as much again.
    // So where something is owed, within four of what is owed; where
    // nothing is, it aims at its own resolution's worth past touching, and
    // the gap lies from two short of touching (the seam's gaps are signed:
    // a letter set into the ink reads how far) to six past it. Over 300,000
    // seeds: from 1.01 short of what is owed to 1.02 past it, and from 0.13
    // short of touching to 1.55 past.
    let (low, top) = if owed > 0.0 {
        (owed - 4.0 * resolution, owed + 4.0 * resolution)
    } else {
        (-2.0 * resolution, 6.0 * resolution)
    };
    let aimed = |gap: f32| gap >= low && gap <= top;
    let mut held = [false; 2];
    if let (true, Some((at, gap))) = (found.exact, found.model) {
        held[0] = true;
        if !found.told {
            return Err(format!(
                "{what}: the model sets it at {at}, its gap {gap}, where its forms tell no gap from none"
            ));
        }
        if !aimed(gap) {
            return Err(format!(
                "{what}: the model sets it at {at}, its gap {gap}, outside [{low}, {top}]"
            ));
        }
        // The first root (review 5810838863): set past the ink, and before
        // its center the curve holds no letter already past the model's
        // window, where an earlier root would lie — a root past one with the
        // same gap, a step or a form's hop over the first crossing, does.
        if at.partial_cmp(&last) != Some(std::cmp::Ordering::Greater) {
            return Err(format!(
                "{what}: the model sets it at {at}, not past the ink at {last}"
            ));
        }
        if let Some((c, g)) = found.curve.iter().find(|&&(c, g)| c < at && g > top) {
            return Err(format!(
                "{what}: the model sets it at {at} (gap {gap}), past a root: at {c} the gap is {g} already"
            ));
        }
        // A side, away from any corner, is where the gap along the band
        // curve is convex in the center — the boxes' gap along a line — so
        // it has one root on from the ink, which the first root's check
        // holds: no comparison of the two centers adds to it (on a 40 × 40
        // side the gap can rise at 0.19 a unit of center, so two gaps each
        // in its window lie further apart than the windows; seed 19466 with
        // nothing owed one time in eight). Counted where the rule's gap is
        // in its own window there too.
        let (_, its) = found.iterative;
        held[1] = found.side && window(its);
    }
    Ok(held)
}

/// Every property of the flow `bytes` choose, its spans' faces drawn
/// from [`every_face`]: the flow_structured_every_face target's whole
/// body (#297). The same generator as [`check_flow_structured`], over a
/// list that also holds the ornament fallback's faces, so an input pins
/// `Mandala Ornaments` or `Mandala Emoji` as a map may; a finding here
/// replays in the face it was found in, as a flow_structured one does in
/// `faces()`.
pub fn check_flow_structured_every_face(bytes: &[u8]) -> Result<(), String> {
    check_generated(&mut Choices::new(bytes), true, &every_face())
}

/// [`check_flow_generated`] over `faces`, and the faces its spans and
/// its flow's own style were pinned to, in the order drawn: what a sweep
/// over [`every_face`] reads to know it met each fallback face.
pub fn check_flow_generated_in(choices: &mut Choices<'_>, faces: &[AppFont]) -> Result<Vec<AppFont>, String> {
    let mut pinned = Vec::new();
    check_generated_pinning(choices, true, faces, &mut pinned)?;
    Ok(pinned)
}

/// [`check_flow_generated`]'s body over `faces`, a frame's text held to
/// what it lays inside the fuzz timeout with `bounded`, and to what it
/// laid when the findings were found without.
fn check_generated(choices: &mut Choices<'_>, bounded: bool, faces: &[AppFont]) -> Result<(), String> {
    check_generated_pinning(choices, bounded, faces, &mut Vec::new())
}

/// [`check_generated`], recording in `pinned` the face of each span's
/// style and of the flow's own, once they are drawn.
fn check_generated_pinning(
    choices: &mut Choices<'_>,
    bounded: bool,
    faces: &[AppFont],
    pinned: &mut Vec<AppFont>,
) -> Result<(), String> {
    fonts::init();
    let count = choices.below(4) as usize;
    let texts: Vec<String> = (0..count).map(|_| text(choices)).collect();
    let styles: Vec<ResolvedText> = (0..count).map(|_| style(choices, faces)).collect();
    let spans: Vec<FlowSpan<'_>> = texts
        .iter()
        .zip(&styles)
        .map(|(text, style)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect();
    let flow = style(choices, faces);
    pinned.extend(styles.iter().map(|style| style.font));
    pinned.push(flow.font);
    let mut fs = FontSystemLease::new("flow fuzz");
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
            shapes_in_pinned_faces(&spans, glyphs, fs.fonts()).map_err(|why| format!("in a box: {why}"))
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
            shapes_in_pinned_faces(&spans, glyphs, fs.fonts()).map_err(|why| format!("in the run: {why}"))?;
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
                    let drawn_texts = mixed_sizes(choices, &texts, &styles, scale, faces);
                    // Its glyphs by direction (#324), drawn last of all, so
                    // an input that ends before them lays none, as every
                    // input did before them; the layout as found is laid
                    // as it was (`laid_layouts`).
                    let mut layouts = layouts;
                    if let Some(drawn) = layouts.last_mut() {
                        drawn.directional = directional(choices);
                    }
                    // Far from the origin, its sides drawn (#313), drawn
                    // after every choice before, so that an input that ends
                    // before it lays the outline and the layout it drew. A
                    // replay lays a finding's outline and layouts as found
                    // as well, as it lays its layout (`laid_layouts`).
                    let mut sets = vec![(path, layouts)];
                    if let Some((at, size)) = far_out(choices) {
                        let far = outline(Rect::new(at, Vec2::splat(size)), shape);
                        let sides = choices.pick(&FAR_SIDES).to_string();
                        let mut drawn = sets[0].1.last().cloned().unwrap_or_else(RingLayout::whole);
                        if drawn.sides.is_none() {
                            drawn.sides = Some((sides, Rgba::new(255, 255, 255, 255)));
                        }
                        if bounded {
                            sets.clear();
                        }
                        FAR_FRAMES.with(|n| n.set(n.get() + 1));
                        sets.push((far, vec![drawn]));
                    }
                    let mut laid_texts = vec![drawn_texts];
                    if !bounded && laid_texts[0].0 != texts {
                        laid_texts.push((texts.clone(), styles.clone()));
                    }
                    for (path, layouts) in &sets {
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
                            // Its paths' tables, counted in its setup (ruling
                            // B16.3a), for the text it is laid with.
                            let tables = |run: &ShapedRun| setup_table_samples(run, path, &fit, &flow);
                            let (layout, ornament) = if bounded {
                                sides_in_time(layout, &flow, scale, STRUCTURED_FRAMES, &tables, &mut fs)
                            } else {
                                (layout.clone(), ornament_run(layout, &flow, scale, &mut fs))
                            };
                            let layout = &layout;
                            let fits_in_time = |run: &ShapedRun| {
                                fits_in_time(run, ornament.as_ref(), STRUCTURED_FRAMES, tables(run))
                            };
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
                            // A ring's cap (rulings B17.2a, B17.2b).
                            let laid_tables = tables(&run);
                            let cap = ring_cap(run.glyphs.len(), bounded, laid_tables);
                            let placed = place_on_closed_capped(
                                &run,
                                path,
                                start,
                                direction,
                                &fit,
                                layout,
                                outward,
                                &flow,
                                &mut fs,
                                cap,
                                Some(&mut trace),
                            );
                            // A frame that cannot hold its lines without two
                            // inks touching — inward round an outline too thin
                            // for both, or a line of tall inks wrapped round a
                            // tight bend — cuts the line that cannot be held
                            // and says so (ruling R38.1): what it cut is no
                            // longer owed.
                            let whole =
                                owes_whole(&run, &fit, frame_room(path)) && trace.cut_apart.is_empty();
                            let mut work = InputWork::default();
                            let sized = matches!(fit, FitMode::Fit { .. });
                            let counted = if matches!(fit, FitMode::Repeat) {
                                work.ring(&run, cap, laid_tables, &placed.1, &trace, "around an outline")
                            } else {
                                work.frame(
                                    &run,
                                    ornament.as_ref(),
                                    sized,
                                    laid_tables,
                                    &trace,
                                    "around an outline",
                                )
                            };
                            let checked = counted
                                .and_then(|()| {
                                    if bounded {
                                        work.within_the_timeout()
                                    } else {
                                        Ok(())
                                    }
                                })
                                .and_then(|()| apart(&trace, "around an outline"))
                                .and_then(|()| {
                                    check_frame_placed(
                                        &frame_spans,
                                        &run,
                                        placed,
                                        (whole, &trace),
                                        "around an outline",
                                        &mut fs,
                                    )
                                });
                            checked.map_err(|why| {
                                format!("{why}, laid {layout:?}, {} spans", frame_texts.len())
                            })?;
                        }
                    }
                    Ok(())
                }
                2 => {
                    let points: Vec<Vec2> = (0..choices.below(3)).map(|_| any_vec2(choices)).collect();
                    let (start, end) = (any_vec2(choices), any_vec2(choices));
                    let curve = match points[..] {
                        [c] => Curve::Quadratic(start, c, end),
                        [c1, c2] => Curve::Cubic(start, c1, c2, end),
                        _ => Curve::Line(start, end),
                    };
                    let fit = fit(choices, &texts);
                    let spacing = any_f32(choices);
                    // One budgeted placement an input (ruling R45.1, #313):
                    // held to the timeout, the curve's in its fit mode, or,
                    // one time in two, its whole-glyph pattern — drawn
                    // after every choice before, so an input that ends
                    // before it lays the fit mode's. A replay lays both.
                    let pitched = choices.one_in(2);
                    // Far from the origin, a `Repeat` one time in two
                    // (#313), drawn last of all.
                    // A replay lays a finding's curve as found as well.
                    let mut curves = vec![(curve, fit)];
                    if let Some((at, length)) = far_out(choices) {
                        let line = matches!(curves[0].0, Curve::Line(..));
                        let fit = if choices.one_in(2) {
                            FitMode::Repeat
                        } else {
                            curves[0].1.clone()
                        };
                        if bounded {
                            curves.clear();
                        }
                        if fit == FitMode::Repeat {
                            FAR_REPEATS.with(|n| n.set(n.get() + 1));
                        }
                        curves.push((far_curve(at, length, line), fit));
                    }
                    for (curve, fit) in &curves {
                        let mut work = InputWork::default();
                        if !bounded || !pitched {
                            check_open(
                                &spans, &run, curve, fit, spacing, bounded, &flow, &mut work, &mut fs,
                            )
                            .map_err(|why| format!("along a curve: {why}"))?;
                        }
                        if !bounded || pitched {
                            check_pitched(&run, std::slice::from_ref(curve), spacing, bounded, &mut work)?;
                        }
                        if bounded {
                            work.within_the_timeout()?;
                        }
                    }
                    Ok(())
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
/// compiled-in `AppFont`, in name order — not `Any`, which until #228
/// drew in the host's fallback face, so a finding in it need not have
/// replayed elsewhere (ruling R30's intent); it draws in the compiled
/// default face now, which [`every_face`] lists by name. Read from the
/// compiled set itself rather than from the family index, which
/// attributes a host's families to `Any`: a sweep that drew its faces
/// from the index would pin families this machine has and another
/// lacks.
///
/// Not the ornament fallback's faces (#297) either, and this list must
/// not change length. Every committed structured seed and finding chose
/// its faces from it: a choice picks the face at its value modulo the
/// list's length, so a face added anywhere in it, at the end included,
/// moves every one of them to another face. And it moves them silently:
/// with the two fallback faces put in, every `flow::`, seed and fuzz test
/// stays green (review 5804264394), each finding replaying in a face it
/// was not found in. The fallback faces are drawn from [`every_face`]
/// instead, by the flow_structured_every_face target and its own seeds,
/// and by `test_generated_flows_over_every_face_hold_every_flow_property`.
/// A face a map can pin that this list lacks belongs there, not here.
pub(crate) fn faces() -> Vec<AppFont> {
    let fallback: Vec<AppFont> = later_faces().collect();
    let mut faces: Vec<AppFont> = COMPILED_FONT_ID_MAP
        .iter()
        .filter(|(font, ids)| !ids.is_empty() && !fallback.contains(font))
        .map(|(font, _)| *font)
        .collect();
    faces.sort_by_key(|font| format!("{font:?}"));
    faces
}

/// The faces compiled in after the seeds [`faces`] picks from were
/// committed, which it leaves out for that reason: the ornament
/// fallback's (#297), in
/// [`ORNAMENT_FALLBACK`](crate::font::fallback::ORNAMENT_FALLBACK)'s
/// order, then the default face (#228).
fn later_faces() -> impl Iterator<Item = AppFont> {
    crate::font::fallback::ORNAMENT_FALLBACK
        .iter()
        .chain(std::iter::once(&crate::font::fallback::DEFAULT_FACE))
        .map(|(font, _)| *font)
}

/// `faces()`, then `later_faces()`: every face a map can pin. `Mandala
/// Emoji` is the first compiled face with no outline at all and no space,
/// the kind of face unlike the others that gave the R24 findings; the
/// default face, Inter, the first variable one; and all three are on the
/// untrusted surface, since a map's `font` can name any of them. Adding a
/// face here moves the flow_structured_every_face seeds' faces, so its
/// `pins_*` seeds are drawn again with it (#228).
pub fn every_face() -> Vec<AppFont> {
    let mut faces = faces();
    faces.extend(later_faces());
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
    RingLayout {
        span,
        sides,
        spacing,
        ..RingLayout::whole()
    }
}

/// A ring's glyphs by direction from choices (#324): a vertical text
/// and four corners, each one time in two, each a short ornament from
/// the structured alphabet that
/// [`check_ornament`](bounds::check_ornament) passes. Zero choices draw
/// none.
pub fn directional(choices: &mut Choices<'_>) -> DirectionalGlyphs {
    let ornament = |choices: &mut Choices<'_>| -> Option<String> {
        let text: String = (0..1 + choices.below(MAX_ORNAMENT_CHARS))
            .map(|_| {
                if choices.one_in(8) {
                    char::from_u32(choices.below(0x11_0000)).unwrap_or('\u{FFFD}')
                } else {
                    *choices.pick(&INTERESTING)
                }
            })
            .collect();
        bounds::check_ornament(&text).ok().map(str::to_owned)
    };
    let vertical = if choices.one_in(2) {
        ornament(choices)
    } else {
        None
    };
    let corners = if choices.one_in(2) {
        let drawn: Vec<Option<String>> = (0..4).map(|_| ornament(choices)).collect();
        drawn
            .into_iter()
            .collect::<Option<Vec<String>>>()
            .and_then(|corners| <[String; 4]>::try_from(corners).ok())
    } else {
        None
    };
    DirectionalGlyphs { vertical, corners }
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
    tables: &dyn Fn(&ShapedRun) -> usize,
    fs: &mut FontSystemLease,
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
        if fits_in_time(&empty, Some(shaped), frames, tables(&empty)) {
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
/// from a hairline to an em dash, an underscore with no side bearings
/// and an Egyptian hieroglyph, each held and inked by a compiled face, so that it shapes alike on
/// every host — and one, U+FFFE, a noncharacter no face holds on any
/// host, as a map's ornament may hold glyphs no face does: each distinct
/// `.notdef` weighs 2,200 glyph steps a shaping (`SHAPE_NOTDEF_STEPS`),
/// which [`sides_in_time`] cuts the ornament for.
/// (At e402949d it drew `❧`, `⸻` and `﷽`, which no compiled face holds:
/// held by a host face here and not on CI, so the long ornaments CI drew
/// were `.notdef`s, which the harness did not cut for — the flow_structured
/// finding `structured_frame_long_ornament_past_the_input_bound`. Until
/// R51 it drew `─`, which the compiled faces hold but give no ink.)
pub const ORNAMENT_GLYPHS: [char; 10] = ['|', '·', '-', '=', 'x', 'W', '—', '_', '𓀀', '\u{FFFE}'];

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
/// face beside a 4-glyph ornament take 463,355 steps, within the 464,396
/// of [`INPUT_STEPS`]; beside 200 `|`s, which add 6,700 more to shape
/// and ink, 470,055, past it, so the text is halved to 76 glyphs, whose
/// budget is the 400,000-step floor: 411,669. A text of one glyph beside
/// the heaviest long ornament the compiled faces hold, 200 `|`s, takes
/// 409,308, with host faces or without; a frame of no text beside fifty
/// U+FFFE, one code point no face holds, 408,098, and beside fifty
/// distinct noncharacters 484,150, so that ornament is cut first.
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
    fs: &mut FontSystemLease,
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

thread_local! {
    /// Test counters (#313): the far-out frames with sides, and the
    /// far-out `Repeat`s along a path, the structured generator has drawn
    /// on this thread ([`far_out`]).
    pub(crate) static FAR_FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static FAR_REPEATS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The side ornaments a far-out frame is given where its layout draws none
/// (#313): a box-drawing pair and a dot, whose copies far from the origin
/// read their path many times a cluster (review 5761697746), and two of
/// [`ORNAMENT_GLYPHS`].
pub const FAR_SIDES: [&str; 4] = ["─═", "· ", "-=", "x"];

/// One time in four, a place far from the origin and a size there (#313):
/// a point `10^k` from it along both axes, `k` from 6 to 22 — where an
/// `f32` sets a point to the nearest unit, and on to where it sets an
/// ornament's glyphs apart by bisection that barely moves them (seed
/// 2771) — and a size of 100 to 204,800, an outline's or a path's,
/// ordinary there. `None` on a zero choice, so an input that ends before
/// it draws what it drew.
pub fn far_out(choices: &mut Choices<'_>) -> Option<(Vec2, f32)> {
    if !choices.one_in(4) {
        return None;
    }
    let at = 10.0_f32.powi(6 + choices.below(17) as i32);
    let size = 100.0 * (1u32 << choices.below(12)) as f32;
    Some((Vec2::splat(at), size))
}

/// A path `length` long from `at` (#313): a line, or a cubic that bends
/// one way and back.
fn far_curve(at: Vec2, length: f32, line: bool) -> Curve {
    if line {
        Curve::Line(at, at + Vec2::new(0.8 * length, 0.6 * length))
    } else {
        Curve::Cubic(
            at,
            at + Vec2::new(0.35 * length, -0.3 * length),
            at + Vec2::new(0.65 * length, 0.3 * length),
            at + Vec2::new(length, 0.0),
        )
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

/// [`check_placed`] for a frame laid as `trace` traced it: where its
/// natural length fits (`whole`), a glyph the rasterizer draws nothing
/// for may be left unplaced where its line lays no ink round it — before
/// its line's first ink or past its last, as a line's outer spaces are,
/// or in a line of nothing that draws, which is not laid (#333) — and
/// nowhere else: never between two inks of one line, which places it.
/// `fs` is read only for a glyph that is not placed, and the glyphs
/// beside it.
pub(crate) fn check_frame_placed(
    spans: &[FlowSpan<'_>],
    run: &ShapedRun,
    (replaced, placed): (Option<ShapedRun>, PlacedRun),
    (whole, trace): (bool, &Trace),
    what: &str,
    fs: &mut FontSystemLease,
) -> Result<(), String> {
    let refers_to = match &replaced {
        Some(new) => {
            check_run(spans, new, &format!("{what}: the run placement made"))?;
            new
        }
        None => run,
    };
    check_frame_placement(refers_to, &placed, (whole, trace), what, fs)
}

/// [`check_placement`] of a frame's placement, with
/// [`check_frame_placed`]'s excuse.
pub(crate) fn check_frame_placement(
    run: &ShapedRun,
    placed: &PlacedRun,
    (whole, trace): (bool, &Trace),
    what: &str,
    fs: &mut FontSystemLease,
) -> Result<(), String> {
    // Each glyph's traced line, by glyph index.
    let mut line_of: Vec<Option<usize>> = vec![None; run.glyphs.len()];
    for (k, line) in trace.lines.iter().enumerate() {
        let ends = line.glyphs.start.min(placed.glyphs.len())..line.glyphs.end.min(placed.glyphs.len());
        for g in &placed.glyphs[ends] {
            if let Some(slot) = line_of.get_mut(g.glyph as usize) {
                *slot = Some(k);
            }
        }
    }
    let laid: Vec<crate::font::LayoutGlyph> = run.layout_glyphs().cloned().collect();
    let mut known: Vec<Option<bool>> = vec![None; run.glyphs.len()];
    // Whether glyph `k` draws: a space never; else the rasterizer's
    // answer, asked at the sizes the engine asks it at (`fonts::draws_at`:
    // 16 pixels per em, then 256 where nothing is drawn at 16), so that
    // "draws" means here what it means to the engine. Asked directly, not
    // through `fonts::glyph_draws`' cache; at those sizes, not the
    // glyph's own, because an image at a frame's largest size would be
    // thousands of pixels square.
    let mut draws = |k: usize, fs: &mut FontSystemLease| -> bool {
        if run.glyphs[k].blank {
            return false;
        }
        *known[k].get_or_insert_with(|| {
            laid.get(k)
                .is_none_or(|laid| fonts::draws_at(|size| raster_box(fs.fonts(), laid, size).is_some()))
        })
    };
    let n = run.glyphs.len();
    let mut excused = |k: usize| {
        if draws(k, fs) {
            return false;
        }
        let before = (0..k).rev().find(|&i| draws(i, fs));
        let after = (k + 1..n).find(|&i| draws(i, fs));
        match (before, after) {
            (Some(a), Some(b)) => line_of[a].is_none() || line_of[a] != line_of[b],
            _ => true,
        }
    };
    check_placement_excusing(run, placed, whole, what, &mut excused)
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

/// The run's natural length reshaped at `scale`, every cluster as long
/// as the longer of its advance and the line height — what the longest
/// step the walk takes for it is, a steep one — and so no shorter than
/// any lay of it at no gap. Each glyph's advance, and the line height
/// with the run's largest glyph, are scaled by the engine's own
/// `size_factor` (ruling R52.1), so below `MIN_FONT_SIZE_PT` a glyph
/// stops shrinking here as it does there.
pub(crate) fn natural_length_at(run: &ShapedRun, scale: f32) -> f32 {
    let glyphs = || run.glyphs.iter().filter(|g| g.span != FILLER_SPAN);
    let largest = glyphs().map(|g| g.size).fold(0.0_f32, f32::max);
    let height = run.line_height.max(0.0) * size_factor(largest, scale);
    glyphs()
        .map(|g| (g.advance.max(0.0) * size_factor(g.size, scale)).max(height))
        .sum()
}

/// Whether `fit` on a path of room `room` owes the whole run: a
/// `Spread` or `Fit` whose natural length fits — for `Fit` at its least
/// scale, as the engine shrinks it, no glyph below `MIN_FONT_SIZE_PT`
/// (ruling R52.1) — places every cluster, and cuts nothing. Owed of every path
/// whose room is finite, as far from the origin as an `f32` reaches: a
/// glyph's step lost below an `f32`'s precision there moves it nowhere,
/// but it is placed.
pub(crate) fn owes_whole(run: &ShapedRun, fit: &FitMode, room: f32) -> bool {
    let least = match fit {
        FitMode::Spread => 1.0,
        FitMode::Fit { min, .. } => *min,
        _ => return false,
    };
    let length = natural_length_at(run, least);
    room.is_finite() && length.is_finite() && length <= room
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
    check_placement_excusing(run, placed, whole, what, &mut |_| false)
}

/// [`check_placement`], a glyph that is not a space nor a filler's left
/// unplaced where `excused` says it may be.
fn check_placement_excusing(
    run: &ShapedRun,
    placed: &PlacedRun,
    whole: bool,
    what: &str,
    excused: &mut dyn FnMut(usize) -> bool,
) -> Result<(), String> {
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
        if let Some(missing) = (0..run.glyphs.len()).find(|&k| {
            let g = &run.glyphs[k];
            !drawn[k] && !g.blank && g.span != FILLER_SPAN && !excused(k)
        }) {
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
/// a span at a weight its family lacks is drawn in another face that has
/// that weight: the default face, whose axis covers every weight (#228),
/// so this property catches it on every machine.
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
