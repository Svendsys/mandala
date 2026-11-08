// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33, R33.1, R34, R35 and R38).
//!
//! The owner, on the frame text: "It should follow the path of its
//! halve, this should be adjustable in the solution. But the point here
//! was to demo the text actually taking the shape of our nodes."
//!
//! **The halves** (ruling R33). The outline is split on its diagonal, at
//! its support points in the directions `(−1, −1)` and `(1, 1)`
//! ([`Outline::support`]): the point where the tangent is at 45° on an
//! ellipse, the corner arc's midpoint on a rounded rectangle — the
//! reading of R33's "in its own aspect" that keeps every horizontal run
//! reading rightward, where the bounding box's diagonal would start the
//! second half leftward on a wide ellipse. The first line takes the
//! upper-right half — from the top-left junction clockwise over the top
//! and down the right side to the bottom-right junction — and the second
//! the lower-left half — from the top-left junction down the left side
//! and along the bottom to the bottom-right one. So every vertical run
//! reads down and every horizontal one reads right, glyphs upright. Each
//! line carries its half with it: a text that opens with a line break
//! has one line, on the second half.
//!
//! **The band curve** (#308, rulings B15.1, B15.4). Every ink of a
//! frame — both lines' letters and the side ornament's copies — is
//! anchored on one offset curve of the outline, at one distance `d` from
//! it all round ([`Band`], [`ArcPath::offset_at`]), so the ring of text
//! is one even offset of the node's shape and nothing steps where the
//! lines meet. A letter's anchor is its ink's center along the line and,
//! across it, its span's band center: halfway between its baseline and
//! its x-height, a property of its font and size, so a span's letters
//! share a baseline and their ascenders and descenders hang free of it
//! ([`inked_with_bands`]). A side ornament's copy is anchored at its
//! own ink's center. `d` is solved once a frame, with its clearance
//! ([`Layout::lay_clear`]): outward, the least at which every ink
//! clears the outline; inward, the frame's ink band centered on it.
//! Inward, where the outline is thinner than `2d` across — the tips of
//! a long, thin ellipse — no curve lies `d` inside it all round, and the
//! curve folds over the far side there.
//!
//! **The lines** (rulings R27, R34). R27 parts the text and sizes a
//! `Fit` frame ([`place_lines`]); a `Once` or `Spread` frame's lines are
//! parted as R27 parts them but not cut to its level span
//! ([`line_partition`]).
//!
//! **The span** (ruling R32, [`LineSpan`]). At [`LineSpan::NATURAL`]
//! each line lies at its natural width: tracked at its font's own gap,
//! centered on its half's apex (where R27 centers it), and
//! slid along its half where that would reach into a junction's
//! clearance. At [`LineSpan::WHOLE`] each end is at its junction less
//! the clearance; in between, each end moves by arc length that part of
//! the way from natural to whole, so the ends move only outward.
//!
//! **Even tracking.** Over the part it covers, a line is laid with one
//! ink gap `g` between every two neighboring letters, its letters' side
//! bearings left out, and a space's advance plus `g` more between words:
//! a word gap is `2g` plus a space. Each letter after the first is set
//! by the pairwise rule (ruling R31), as far from the last letter's ink
//! as owed, a zero gap included, so the line's length grows with `g`,
//! nearly as a line does, and `g` is solved by regula falsi from the
//! straight path's answer — or from the line's last lay in the same
//! clearance search — to the precision its letters are set to: the
//! line's first and last inks centered on its part's ends ([`track`],
//! [`solve_tracking`]). The rule is solved for each letter from where
//! the two inks would be owed apart were the path straight on, the last
//! letter's share of that its warm start, within a bracket of centers
//! too near and far enough ([`pair`], ruling R43): a lookup or a few a
//! letter, 64 at most. Round a sharp corner, where no center sets a
//! letter exactly `g` from the one before, it is set at the nearest
//! center tried past that is at least `g` away, and clear by `g` of the
//! few letters before its neighbor ([`EARLIER`]). A side ornament's
//! copies are set by the same rule ([`InkLay`]). A glyph with no
//! outline steps as its box; a line of one letter sits centered on its
//! part.
//!
//! **Each junction is a visible break** (rulings R33.1, R35.1). At every
//! span the two lines' end inks at a junction are at least two word
//! gaps apart, box to box, of the mean of the lines that have one (a
//! line of one word has none: the span stretches its letter gaps
//! alone), and with `sides` one whole copy of the ornament fits between
//! them — the clearance grown for that copy's room alone only as far as
//! the lines' slack, never pressing a line: a frame loses its sides
//! before its text (rulings R48.3, B15.5); two end inks whose outward
//! normals turn more than 135° apart are lines stacked across a thin
//! outline, and need only not touch.
//! Each line keeps its own clearance from each junction, grown round by
//! round among the lines whose drawing back parts the ends there, at
//! whole and then on at the span laid ([`Layout::lay_clear`]): each
//! round's step damped by how much drawing back eases the break wanted,
//! and no line drawn back past its slack while the other line has room
//! to give (rulings R38.1, R38.3).
//!
//! **No line is laid tighter than its font, and no two inks touch**
//! (rulings R34, R35.2). Where the halves do not hold the frame so — a
//! line its half less the clearance cannot hold at its font's gap, a
//! junction left short of a break, two inks touching — the last resorts
//! follow: a `Fit` frame is shrunk, within its bounds; a line held even
//! so by nothing is cut at its end, the placement reporting it; two
//! inks that still touch — an outline too thin to hold the two lines
//! apart — have the later line cut, or left out, the trace saying so
//! ([`Trace::cut_apart`], ruling R38.1: never
//! overlap wins over never cut); and what is left unheld even so is
//! reported in the trace ([`Unheld`]), never silent.
//! Every last resort is
//! bounded in work (rulings R35.3, R44.2, R46): a shrink's probe looks
//! first whether its lines fit at all, and a frame's budget,
//! [`STEPS_PER_GLYPH`] glyph steps a glyph of its text and at least
//! [`TRACK_BUDGET`] ([`frame_budget`]), counts all of its work — its
//! tracking and side lays, its shaping and inking, its touch checks,
//! R27's walks and its paths' tables, each weighted to about one tracking
//! step's time — and is
//! read before every lay, build and reshape — in each tracking solve,
//! slide, shrink (its floor build too), cut and backtrack, R27's fit and
//! a `Once` filler's copies — and before each copy of a side ornament,
//! which is laid only while the frame can afford it and the touch check
//! of the arc after ([`check_steps`], at [`touch_looks`] looks a box of
//! the classes of height it holds): an arc it cannot afford is left empty, or keeps the copies it
//! has where they hold the equal-gap rule, and is reported as left empty
//! or kept short ([`Unheld::sides`]).
//! So a frame overshoots its budget
//! by at most [`overshoot_steps`] (ruling R47.2): what runs before the
//! budget is first read — its setup, R27's first reshape and one build's
//! shaping and inking — the lay of each line that finds it spent, and
//! the touch checks that end the frame. A frame whose budget
//! runs out while two of its inks still touch keeps the longest run of
//! its inks that touch nothing ([`untouched`], within a share of the
//! budget held back for it), and says it was cut ([`Trace::cut_apart`]). A frame's text is cut to
//! [`MAX_PLACED_GLYPHS`] at a cluster before its lines are prepared
//! (ruling R44.1), its sides placing no more than the cap leaves, so the
//! budget is at most `STEPS_PER_GLYPH × MAX_PLACED_GLYPHS`.
//!
//! **Sides.** With `sides`, an ornament closes what the lines leave
//! round each junction: whole copies, at least one and at most
//! [`MAX_SIDE_COPIES`], every gap and both joins to the text equal
//! (ruling R31's lay, seeded by the text's end inks), on the frame's band
//! curve at each copy's ink center, the ornament's ink no taller than the
//! text's x-height.
//! A frame loses its sides before its text (rulings R48.3, B15.5): a
//! junction grows for a copy's room only within its lines' slack, and a
//! frame that its sides' room leaves cut is laid again without them,
//! keeping that lay, its arcs empty and reported, where it keeps more of
//! its text.
//! The ornament is shaped into the run as a line of its own, the way a
//! `Once` filler is.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use glam::Vec2;

use super::lines::{frame_halves, frame_lines, line_partition, place_lines_in};
use super::*;
use crate::document::values::FontSize;
use crate::flow::path::{Bend, CornerForm, EllipseForm, FormPoint, OffsetIn};
use crate::flow::shape::shape_run;
use crate::flow::span::FlowSpan;
use crate::font::fonts::{face_char_ink, glyph_outline_ink};
use crate::font::SwashCache;

/// The most probes a line's tracking bracket is closed in ([`falsi`]):
/// as many as a bisection of a few points to some millionths of one
/// takes, though a line's length, near linear in its gap, closes in a
/// few.
const TRACK_ROUNDS: usize = 20;

/// The most times a line's tracking bracket widens looking for a gap
/// too wide to fit ([`solve_tracking`]): past about `2^24` points nothing
/// is laid.
const TRACK_DOUBLINGS: usize = 24;

/// The most rounds a natural line is slid back from its bottom-right
/// junction in: each a secant probe of the start whose line ends at the
/// clearance, its length changing only with the path's bend
/// ([`Layout::slid`]).
const SLIDE_ROUNDS: usize = 16;

/// How near the clearance a slid line's end must come, in points, where
/// the path's floats resolve that ([`resolved`]).
const SLIDE_TOLERANCE: f32 = 1e-4;

/// How many float steps of the positions compared a tolerance is held
/// to at the least ([`resolved`]).
const RESOLUTION_STEPS: f32 = 2.0;

/// `tolerance`, or [`RESOLUTION_STEPS`] float steps of a position `at`
/// along a path where those are wider — each [`ulp`]'s, one to two float
/// steps (a power of two's own `EPSILON` of it): a line's end and its part's end
/// some 2,400 points round an outline are a float step — 2.4e-4 points —
/// apart at the nearest, so a tolerance of 1e-4 there can never be met,
/// and a solve that waits for it runs out its rounds and keeps a worse
/// answer than the one it had (a slide left a line a float step too
/// long for its part, read as pressed; ruling R43).
fn resolved(tolerance: f32, at: f32) -> f32 {
    let step = at.abs() * f32::EPSILON;
    if step.is_finite() {
        tolerance.max(RESOLUTION_STEPS * step)
    } else {
        tolerance
    }
}

/// The most probes the secondary rule's change of a short line's word
/// gaps is solved in past its bracket, and the most times its bracket
/// widens ([`Layout::reach_its_end`]): the words it changes all lie past
/// the letter that steps, so the line's length is near linear in the
/// change, and a line it can bring to its end it brings in one or two; a
/// change searched for longer only spends the frame's budget.
const STRETCH_ROUNDS: usize = 3;
const STRETCH_DOUBLINGS: usize = 2;

/// How many [`ulp`]s apart two starts of a slide may be for a jump in
/// its line's end between them to be read as a step in the end, not a
/// slope the secant has yet to close on ([`Layout::slid`], #308): eight,
/// 8 to 16 float steps, past which the secant is still moving the start
/// by more than its floats resolve.
const SLIDE_ULPS: f32 = 8.0;

/// Whether a slide's line's end steps between two starts it tried
/// ([`Layout::slid`], ruling B15.3): `short`, a start and the miss, its
/// line's end past the clearance, short of it (negative), and `past`, one
/// whose line ends past it — within [`SLIDE_ULPS`] ulps of the start, and
/// the end moving across them by more than a line moving with its start
/// nearly one for one could, twice their distance and `tolerance`. Not a
/// number anywhere, or a bracket that is not one — `past` not after
/// `short`, or as wide as infinity — is no step.
///
/// Cost: O(1).
fn steps_between((a, short): (f32, f32), (b, past): (f32, f32), tolerance: f32) -> bool {
    let width = b - a;
    width > 0.0 && width <= SLIDE_ULPS * ulp(a.abs().max(b.abs())) && past - short > 2.0 * width + tolerance
}

/// Test seam (ruling B15.3): [`steps_between`].
#[cfg(test)]
pub(crate) fn steps_between_seam(short: (f32, f32), past: (f32, f32), tolerance: f32) -> bool {
    steps_between(short, past, tolerance)
}

/// About a float step of a position `at`: [`f32::EPSILON`] of it — for
/// `at` from `2^e` up to `2^(e+1)` one to two of its float steps, each
/// `2^(e−23)` — and the least positive `f32` at none.
fn ulp(at: f32) -> f32 {
    (at.abs() * f32::EPSILON).max(f32::MIN_POSITIVE)
}

/// The rounds a junction's clearance is grown in.
const CLEAR_ROUNDS: usize = 16;

/// The most copies of a side ornament one junction holds.
const MAX_SIDE_COPIES: usize = 256;

/// The most probes a side arc's gap is solved in past its bracket
/// ([`falsi`], ruling R44.3).
const SIDE_ROUNDS: usize = 30;

/// How narrow a side arc's gap is solved, in points, where the path's
/// floats resolve that ([`resolved`]): a thousandth of a point, the
/// pairwise rule's own tolerance (ruling R43: thirty rounds closed a
/// bracket of some points to some millionths of one).
const SIDE_PRECISION: f32 = INK_TOLERANCE;

/// How many word gaps apart two lines' end inks stay at a junction
/// (ruling R33.1).
pub(crate) const JUNCTION_WORD_GAPS: f32 = 2.0;

/// How far over the clearance the rounds aim, so a measure of it never
/// rounds below.
const CLEAR_MARGIN: f32 = 1.01;

/// The least a junction's clearance grows by in a round, in points.
const MIN_GROWTH: f32 = 0.25;

/// How many times a round of the clearance that would press a line is
/// halved before it is taken anyway.
const BACKTRACKS: usize = 6;

/// How many times a shrink's probe halves such a round (ruling R35.3):
/// a probe asks only whether its size can be held, and a round that
/// presses a line after one halving says, near enough, that it cannot.
const PROBE_BACKTRACKS: usize = 1;

/// The most probes a natural line is centered on its apex in
/// ([`falsi`]), and how near equal its two arcs either side must come,
/// in points.
const CENTER_ROUNDS: usize = 40;
const CENTER_TOLERANCE: f32 = 1e-3;

/// How far below its font's own gap a line's solved gap may fall and
/// still have it: the bisection's precision, in points.
const GAP_TOLERANCE: f32 = 1e-4;

/// The least gap a side ornament's copy is laid at, in points: past
/// [`INK_TOLERANCE`], so the pairwise rule always sets a copy clear of
/// the ink before it — an ornament with no side bearings (`─`, `█`)
/// included — rather than where its box would fall.
const JOIN_FLOOR: f32 = 4.0 * INK_TOLERANCE;

/// The rounds of bisection a `Fit` frame is shrunk in, when its halves
/// cannot hold its lines at the size R27 gave it.
const SHRINK_ROUNDS: usize = 12;

/// The shrink stops once its bracket on the scale is within this share
/// of the scale held (ruling R42.2).
const SHRINK_PRECISION: f32 = 0.01;

/// The shrink stops once its bracket spans less than this size step, in
/// points, at the frame's largest glyph (ruling R42.2).
const SHRINK_LEAST_STEP_PT: f32 = 0.25;

/// How much of the clearance of the largest size a shrink has found
/// held a probe at a larger size starts from (ruling R35.3): a clearance
/// grows with the word gap, so with the size — or stays, where a wide
/// space of no size of its own sets it — and the probe's rounds grow it
/// the rest of the way, from below.
const SEED_SHARE: f32 = 0.9;

/// The least glyph steps a frame's work may take in all, each glyph a
/// [`track`] or a side's lay sets counting one (rulings R35.3, R44.2):
/// the floor of its budget ([`frame_budget`]), which scales with its text
/// past it. A frame that reaches its budget keeps the best it has laid —
/// the largest size its shrink found clean, the cuts made so far, its
/// inks cut to those that touch nothing — and its trace reports what is
/// left unheld.
///
/// Ten times the most any real frame took while a step counted tracking
/// alone (rulings R42.3, R43.3): the aspect sweep's worst, 36,748 glyph
/// steps — the quatrain outward round the 600 × 12 rectangle, `Once` at
/// span ½, its sides' lays counted — rounded up to 40,000; the demo
/// maps' frames took under 2,400. At 30411a6e it was 2^21, and the
/// worst real frame took 306,098. Since a step counts all of a frame's
/// work (ruling R46), the sweep's worst, the quatrain inward round the
/// 50:1 ellipse, `Fit`, takes 59,468: the budget is unchanged, six times
/// that (ruling R47.1), so the fuzzers' per-input bound holds (R45).
pub(crate) const TRACK_BUDGET: usize = 400_000;

/// The glyph steps a frame's work may take a glyph of its text, past
/// [`TRACK_BUDGET`] (ruling R44.2): ten times the most any frame of the
/// aspect sweep took a glyph, rounded up, while a step counted tracking
/// alone; five times its 597 since a step counts all of a frame's work
/// (rulings R46, R47.1). A frame's work grows with its
/// text, so a fixed count would cut real text past some length — a
/// 2,000-word frame took more than 400,000 — where this bounds the work
/// a glyph, and the glyph cap ([`MAX_PLACED_GLYPHS`]) bounds the glyphs.
pub(crate) const STEPS_PER_GLYPH: usize = 3_000;

/// A frame's work budget, in glyph steps, for a text of `glyphs`
/// glyphs: [`TRACK_BUDGET`], or [`STEPS_PER_GLYPH`] a glyph where that
/// is more. At most `STEPS_PER_GLYPH × MAX_PLACED_GLYPHS`.
pub(crate) fn frame_budget(glyphs: usize) -> usize {
    #[cfg(test)]
    if let Some(budget) = BUDGET_OVERRIDE.with(Cell::get) {
        return budget;
    }
    #[cfg(test)]
    let floor = FLOOR_OVERRIDE.with(Cell::get).unwrap_or(TRACK_BUDGET);
    #[cfg(not(test))]
    let floor = TRACK_BUDGET;
    floor.max(STEPS_PER_GLYPH.saturating_mul(glyphs.min(MAX_PLACED_GLYPHS)))
}

// What a frame's work is counted as, in glyph steps (rulings R46.1,
// R47.3). Each weight is the most one unit of that work took under the
// ASan fuzz build, divided by 16 µs — about the most a glyph step of
// [`track`] took — and rounded up, so that one counted step takes about
// the same time whatever work it counts. Measured at e301e041, at a load
// average of 1 to 2: each call of the work timed by a temporary
// instrumentation of the engine, never committed, over every committed
// flow fuzz finding and seed and the heaviest frames a sweep of 2,600
// seeded structured inputs drew, and an upper envelope fitted to each
// call's time over its call, its glyphs and its faces (315 shapings,
// 381 inkings). The PR body of #294 has the table.

/// A [`track`], past its clusters (a step each): its setup, and the
/// lay's look at its ends round it — 11.5 µs a call ÷ 16 µs, rounded up.
pub(crate) const TRACK_CALL_STEPS: usize = 1;

/// Shaping a run, or reshaping one (`reshape`, `shape_run` in
/// `side_run`), a call: 435 µs ÷ 16 µs = 27.2, rounded up.
pub(crate) const SHAPE_CALL_STEPS: usize = 28;

/// A glyph shaped: 261 µs ÷ 16 µs = 16.3, rounded up.
pub(crate) const SHAPE_GLYPH_STEPS: usize = 17;

/// A face a shaped run's glyphs were found in: 5.2 ms ÷ 16 µs = 326,
/// rounded up to 330.
pub(crate) const SHAPE_FACE_STEPS: usize = 330;

/// A glyph shaped that no face holds (`.notdef`, glyph 0), which sends
/// the shaper through every face it has: 34.8 ms ÷ 16 µs = 2,173,
/// rounded up to 2,200. (That cost is a defect of its own, #304.)
pub(crate) const SHAPE_NOTDEF_STEPS: usize = 2_200;

/// Inking a run ([`inked`], each glyph's outline read uncached), a call:
/// 1.41 ms ÷ 16 µs = 88.2, rounded up.
pub(crate) const INK_CALL_STEPS: usize = 89;

/// A glyph inked: 83 µs ÷ 16 µs = 5.2, rounded up.
pub(crate) const INK_GLYPH_STEPS: usize = 6;

/// A face a run's inked glyphs are drawn in: 1.66 ms ÷ 16 µs = 103.8,
/// rounded up.
pub(crate) const INK_FACE_STEPS: usize = 104;

/// Ink boxes and pairs of them one glyph step weighs where two inks are
/// looked at for a touch in [`touch_scan`]'s sweep ([`touching`]): some
/// 0.4 µs a box, the sort and the sweep, so 16 µs ÷ 0.4 µs = 40 boxes,
/// rounded down to 32. Measured again at R48 (review 5739829769), each
/// box and each pair the sweep looks at a unit, on the flow_structured
/// target's ASan build over its seeds, the committed findings and the
/// heavy seeds, three runs, beside the sweep before it in the same
/// binary: at most 0.42 µs a unit over the calls of 256 units or more
/// (the sweep before, 0.50), at a load average of 2 to 6.
pub(crate) const BOXES_A_STEP: usize = 32;

/// Ink boxes and looks one glyph step weighs past [`touch_scan`]'s
/// sweep, in its strips ([`stripped`]; rulings R48.1, R50.1), a box
/// placed or copied into a class's strips a look: at most 1.97 µs a box
/// or a look over the calls of 256 units or more, measured as
/// [`BOXES_A_STEP`] is with the strips forced on, so 16 µs ÷ 1.97 µs =
/// 8.1, rounded down to 8. Measured again at R50 (review 5745584645) on
/// the strips by class of height: the flow_structured target's ASan
/// build over its seeds, the committed findings, the heavy seeds, 299
/// seeded structured inputs and twelve long-ornament frames round 2,000
/// and 20,000 outlines (calls of up to 155,732 units), three runs, at a
/// load average of about 1. The worst was a call of 258 boxes that found
/// a touch at its first look; calls of 100,000 units or more took 1.05
/// µs a unit at most. (At R48, with one strip height for every box,
/// 2.97 µs, and 4.)
pub(crate) const STRIPPED_A_STEP: usize = 8;

/// The most pairs a touch check looks at a box it is given past what
/// its classes of height add (rulings R48.2, R51.2): twice the most any
/// check made over its corpus ([`TOUCH_LOOKS_BASE_MEASURED`]) — its
/// sweep's looks and each class's own strips' ([`stripped`]).
pub(crate) const TOUCH_LOOKS_BASE: usize = TOUCH_LOOKS_BASE_MEASURED * 2;

/// The most pairs a touch check looked at a box in its sweep and within
/// each class's own strips, rounded up, over the frames
/// [`TOUCH_LOOKS_A_CLASS_MEASURED`] is measured over (ruling R51.2):
/// 4.96, 506 looks over 102 boxes, the quatrain round the budget grid's
/// 1,000 × 20 ellipse, a check of three classes; over a check of
/// thousands of boxes, 4.15 (a long ornament round a 20,000 sharp
/// square, 48,025 boxes). Pinned by
/// `test_flow_halves_a_touch_check_looks_at_few_pairs_a_box` over the
/// sweep and the demo maps, by the budget grids over theirs, and by the
/// long-ornament and mixed-size pins (`looks_as_measured`).
pub(crate) const TOUCH_LOOKS_BASE_MEASURED: usize = 5;

/// The most pairs a touch check looks at a box for each class of height
/// it holds (rulings R48.2, R51.2): twice the most any check made over
/// its corpus ([`TOUCH_LOOKS_A_CLASS_MEASURED`]) — the boxes a class pair
/// places and the looks between them ([`stripped`]).
pub(crate) const TOUCH_LOOKS_A_CLASS: usize = TOUCH_LOOKS_A_CLASS_MEASURED * 2;

/// The most pairs a touch check looked at a box for each class of height
/// it held, rounded up, between a class and the finer ones — each finer
/// box placed in its strips or each of its boxes copied into theirs, and
/// the looks between them ([`stripped`]) — over the aspect sweep, the
/// budget grids, the demo maps' frames, the long-ornament and mixed-size
/// pins and every committed flow fuzz finding (ruling R51.2):
/// 1.50, 1,654 looks over 79 boxes of 14 classes, the reviewer's text of
/// 14 sizes from 0.5 to 4,096 pt round a 2,000 sharp square, `Fit`
/// (review 5751828253: 1,885 looks over those 79 boxes in all, 23.86 a
/// box); over a check of thousands of boxes, 1.28 (a long text's frame,
/// 1,118 boxes of two classes). Pinned as [`TOUCH_LOOKS_BASE_MEASURED`]
/// is. (At R50, one bound of 18 a box for every check, `TOUCH_LOOKS_A_BOX`,
/// measured as 8.58 over a corpus with no text of mixed sizes.)
pub(crate) const TOUCH_LOOKS_A_CLASS_MEASURED: usize = 2;

/// The most pairs a touch check of `classes` classes of height looks at
/// a box (rulings R48.2, R51.2): [`TOUCH_LOOKS_BASE`], and
/// [`TOUCH_LOOKS_A_CLASS`] for each class. What a side arc holds back for
/// its check rests on it ([`check_steps`]), and with it
/// [`overshoot_steps`]; the flow fuzz property fails a frame whose check
/// looks at more.
///
/// A box is weighed against every coarser class the check holds, near
/// it or not: a lookup that charged it only for the coarser inks near
/// it, along the axis as well as across, would still look for them in
/// each class, so the classes are counted instead. A check holds no more
/// classes than its frame's inks span ([`height_classes`]), and the
/// font-size window bounds those: every size is from 0.5 to 4,096 pt,
/// so an ink no taller than twice its size is less than 8,192 tall, of
/// class 14 at most, and the least class, of an ink of no height and
/// [`TOUCH`], is −9: 24 classes at most, one more for each doubling of an
/// ink past twice its size.
pub(crate) fn touch_looks(classes: usize) -> usize {
    TOUCH_LOOKS_BASE.saturating_add(TOUCH_LOOKS_A_CLASS.saturating_mul(classes))
}

/// The most glyph steps a touch check of `boxes` boxes of `classes`
/// classes of height counts while it looks at no more than
/// [`touch_looks`] pairs a box (rulings R48.2, R51.2): its sweep,
/// [`SWEEP_PAIRS`] looks a box at most, and past it its strips, each at
/// its weight.
pub(crate) fn check_steps(boxes: usize, classes: usize) -> usize {
    let swept = boxes.saturating_add(boxes.saturating_add(1).saturating_mul(SWEEP_PAIRS));
    let stripped = boxes.saturating_add(boxes.saturating_mul(touch_looks(classes)));
    weighing_steps(swept).saturating_add(stripped.div_ceil(STRIPPED_A_STEP))
}

/// How many classes of height a touch check of boxes `sizes` wide and
/// tall holds at most, across either axis it may sweep along ([`stripped`],
/// ruling R51.2): the distinct classes ([`strip_class`]) of their heights
/// or of their widths, whichever are more. A size with no ink to touch
/// with ([`has_ink`]) is left out, as the check leaves it out.
pub(crate) fn height_classes(sizes: impl IntoIterator<Item = Vec2>) -> usize {
    let mut across: [Vec<i32>; 2] = [Vec::new(), Vec::new()];
    for size in sizes {
        if has_ink((Vec2::ZERO, size)) {
            across[0].push(strip_class(size.y));
            across[1].push(strip_class(size.x));
        }
    }
    across
        .iter_mut()
        .map(|classes| {
            classes.sort_unstable();
            classes.dedup();
            classes.len()
        })
        .max()
        .unwrap_or(0)
}

/// Clusters one glyph step walks in R27's arithmetic lays
/// (`lines::Lines`): some 1.3 µs a cluster along a path, so 16 µs ÷
/// 1.3 µs = 12 clusters, rounded down to 8.
pub(crate) const CLUSTERS_A_STEP: usize = 8;

/// Clusters one glyph step walks in a side arc's model lays
/// ([`InkLay::modeled`], #309, ruling B16.2a): at most 6.03 µs a cluster
/// over the calls of 32 clusters or more (4.28 over those of 256 or
/// more), so 16 µs ÷ 6.03 µs = 2.65, rounded down to 2. Measured as
/// [`CLUSTERS_A_STEP`]'s ruling R47.3 asks — each model lay timed by a
/// temporary instrumentation of the engine, never committed, on the flow
/// targets' ASan build over their seeds, every committed flow finding and
/// the heavy seeds, three runs each, at a load average of about 2.
pub(crate) const MODEL_CLUSTERS_A_STEP: usize = 2;

/// Samples of an arc-length table one glyph step weighs, where a frame's
/// setup builds its halves' paths or a `Repeat` ring its path
/// ([`ArcPath::closed`], [`ArcPath::table_samples`]; #309, ruling
/// B16.3a): at most 0.164 µs a sample over the builds of 1,024 samples or
/// more (0.134 over those of 16,384 or more, the largest of
/// [`MAX_TABLE_SAMPLES`](crate::flow::MAX_TABLE_SAMPLES) + 1), so 16 µs ÷
/// 0.164 µs = 97.6, rounded down to 64 — at which no build of any size
/// took more than 13 µs a step it is charged. Measured as
/// [`CLUSTERS_A_STEP`]'s ruling R47.3 asks — each table's build timed by a
/// temporary instrumentation of the engine, never committed, on the flow
/// targets' ASan build over their seeds, every committed flow finding and
/// the heavy seeds, three runs each, and a sweep of 2,600 seeded
/// structured inputs, at a load average of about 2.
pub(crate) const TABLE_SAMPLES_A_STEP: usize = 64;

/// The glyph steps building tables of `samples` samples in all counts
/// (ruling B16.3a): [`TABLE_SAMPLES_A_STEP`] samples a step, none for
/// none.
pub(crate) fn table_steps(samples: usize) -> usize {
    samples.div_ceil(TABLE_SAMPLES_A_STEP)
}

/// A frame's own setup, past what it counts — its partition, its runs
/// cloned — some 1 ms ÷ 16 µs, rounded up. (That 1 ms held its halves'
/// tables too, which are counted since #309 at their samples,
/// [`table_steps`].)
pub(crate) const FRAME_STEPS: usize = 64;

/// The distinct faces `run`'s glyphs are drawn in, and how many of its
/// glyphs no face holds.
fn faces_and_notdefs(run: &ShapedRun) -> (usize, usize) {
    let mut faces = Vec::new();
    let mut notdefs = 0;
    for glyph in run.layout_glyphs() {
        notdefs += usize::from(glyph.glyph_id == 0);
        if !faces.contains(&glyph.font_id) {
            faces.push(glyph.font_id);
        }
    }
    (faces.len(), notdefs)
}

/// The glyph steps shaping `run` counts (ruling R46.1).
pub(crate) fn shaping_steps(run: &ShapedRun) -> usize {
    let (faces, notdefs) = faces_and_notdefs(run);
    SHAPE_CALL_STEPS
        .saturating_add(SHAPE_GLYPH_STEPS.saturating_mul(run.glyphs.len()))
        .saturating_add(SHAPE_FACE_STEPS.saturating_mul(faces))
        .saturating_add(SHAPE_NOTDEF_STEPS.saturating_mul(notdefs))
}

/// The glyph steps inking `run` counts (ruling R46.1).
pub(crate) fn inking_steps(run: &ShapedRun) -> usize {
    let (faces, _) = faces_and_notdefs(run);
    INK_CALL_STEPS
        .saturating_add(INK_GLYPH_STEPS.saturating_mul(run.glyphs.len()))
        .saturating_add(INK_FACE_STEPS.saturating_mul(faces))
}

/// How many [`track`]s of every glyph a frame may take past its spent
/// budget: a lay of each line — the slide's and the solve's probe that
/// notices the budget is spent, and the check at the font's gap — the
/// look at whether its lines fit, and the touch checks that end the
/// frame, a step every [`BOXES_A_STEP`] boxes and pairs looked at
/// (ruling R44.2; pinned by the `PAST_BUDGET` grid,
/// `flow::tests::budget_tests`).
pub(crate) const OVERSHOOT_TRACKS: usize = 8;

/// The most glyph steps one build of a frame's text counts in shaping
/// and inking (ruling R47.2), for a text shaped as `text` and, with
/// sides, an ornament shaped alone as `ornament`: the text reshaped at a
/// size and inked, its spans' band centers read with it
/// ([`band_steps`], #308); with sides, the ornament and an `x` shaped and inked
/// to size it, and shaped and inked again beside the text, which is
/// shaped again with it (`side_run`). Each run's own faces and
/// `.notdef`s are counted, as the build counts them.
pub(crate) fn build_steps(text: &ShapedRun, ornament: Option<&ShapedRun>) -> usize {
    let alone = shaping_steps(text)
        .saturating_add(inking_steps(text))
        .saturating_add(band_steps(text));
    let Some(ornament) = ornament else {
        return alone;
    };
    let x = SHAPE_CALL_STEPS
        + SHAPE_GLYPH_STEPS
        + SHAPE_FACE_STEPS
        + INK_CALL_STEPS
        + INK_GLYPH_STEPS
        + INK_FACE_STEPS;
    let beside = shaping_steps(ornament).saturating_add(inking_steps(ornament));
    alone
        .saturating_add(shaping_steps(text))
        .saturating_add(x)
        .saturating_add(beside.saturating_mul(2))
}

/// The most glyph steps a frame of `text` may take past its budget
/// (ruling R47.2): what runs before its budget is first read — its own
/// setup ([`FRAME_STEPS`]), its paths' tables of `tables` samples
/// ([`table_steps`]; ruling B16.3a, `place::setup_table_samples`) and,
/// for a `Fit` frame (`sized`), R27's reshape at the size it fits — one
/// build's shaping and inking
/// ([`build_steps`]), and a lay of each line ([`OVERSHOOT_TRACKS`]
/// `track`s of every glyph), which the touch checks that end the frame
/// fall within. A budget is read before each build, each reshape of
/// R27's fit, each lay and each copy of a side ornament, so no more than
/// one of each runs past it; [`untouched`] runs within the budget, which
/// holds its share back, and so does a side arc's touch check, held back
/// before each copy ([`check_steps`], ruling R48.2). That rests on two
/// assumptions, which the flow fuzz property holds every frame to: that
/// no touch check looks at more than [`touch_looks`] pairs a box of the
/// classes of height it holds; and that no side arc's check holds more
/// classes than its arc held back for ([`height_classes`], over the
/// text's boxes and the ornament's inks before the fill; ruling R51.2). A check's looks are the inks near each
/// box, whatever the tallest box ([`stripped`], ruling R50.1), and a few
/// for each class of height it holds (ruling R51.2); the structured
/// fuzzer draws long mixed side ornaments and texts of up to 16 sizes, as
/// a map may hold (rulings R50.3, R51.2), so a geometry that breaks it is
/// a finding.
pub(crate) fn overshoot_steps(
    text: &ShapedRun,
    ornament: Option<&ShapedRun>,
    sized: bool,
    tables: usize,
) -> usize {
    let r27 = if sized { shaping_steps(text) } else { 0 };
    FRAME_STEPS
        .saturating_add(table_steps(tables))
        .saturating_add(r27)
        .saturating_add(build_steps(text, ornament))
        .saturating_add(OVERSHOOT_TRACKS.saturating_mul(text.glyphs.len() + TRACK_CALL_STEPS))
}

/// The most glyph steps a frame of `text` may take in all, its paths'
/// tables of `tables` samples: its budget ([`frame_budget`]) and the most
/// it may take past it ([`overshoot_steps`]). What the flow fuzzers hold
/// an input's frames to, at their cost a step under ASan (ruling R45.1).
pub(crate) fn frame_work_steps(
    text: &ShapedRun,
    ornament: Option<&ShapedRun>,
    sized: bool,
    tables: usize,
) -> usize {
    frame_budget(text.glyphs.len()).saturating_add(overshoot_steps(text, ornament, sized, tables))
}

/// The glyph steps a look at `boxes` ink boxes for a touch counts.
fn weighing_steps(boxes: usize) -> usize {
    boxes.div_ceil(BOXES_A_STEP).max(1)
}

/// The glyph steps [`untouched`] may take over `boxes` inks of `classes`
/// classes of height ([`height_classes`]): a bisection of looks at them,
/// each with its lines cut again — as much work as weighing them — and
/// a touch check of them, at most [`check_steps`] while it looks at no
/// more than [`touch_looks`] pairs a box (rulings R48.2, R51.2). What a
/// frame's budget holds back for it (ruling R46.1).
///
/// (Until #308 each look's check was weighed as one more weighing of its
/// boxes, a check that never reaches its strips: the quatrain outward
/// round the 600 × 12 sharp rectangle, its 102 inks two lines and sides
/// apart across the outline, held 64 back and took 75, its checks
/// looking at 2.04 pairs a box.)
pub(crate) fn untouched_steps(boxes: usize, classes: usize) -> usize {
    let probes = (usize::BITS - boxes.leading_zeros()) as usize + 1;
    probes.saturating_mul(weighing_steps(boxes).saturating_add(check_steps(boxes, classes)))
}

/// The most glyphs a frame places, its text and its sides together
/// ([`MAX_PLACED_GLYPHS`], ruling R44.1).
fn glyph_cap() -> usize {
    #[cfg(test)]
    if let Some(cap) = CAP_OVERRIDE.with(Cell::get) {
        return cap;
    }
    MAX_PLACED_GLYPHS
}

/// How far apart the outward normals of two lines' end inks at a
/// junction must turn, as a cosine, for the two to be lines stacked
/// across a thin outline rather than lines meeting there (ruling R35.1):
/// 135°.
const STACKED: f32 = -std::f32::consts::FRAC_1_SQRT_2;

/// How far apart two inks must be not to touch, in points: past
/// [`INK_TOLERANCE`], what the pairwise rule sets inks apart to.
pub(crate) const TOUCH: f32 = INK_TOLERANCE;

/// The most times a frame's lines are cut and the clearance found
/// again: a cut line, stretched over its half at whole, has wider word
/// gaps and so wants more clearance, which a round more settles.
const CUT_ROUNDS: usize = 8;

/// The outline's two halves (ruling R33): each its path, the part of it
/// between its junctions, and its apex.
struct Halves {
    /// The outline walked from the middle of its left side, clockwise
    /// (the first half's) and counterclockwise (the second's), each with
    /// its level span ([`frame_halves`]): what R27 sizes and parts the
    /// frame's text on too, so a frame builds its paths once (#309).
    level: [(ArcPath, Range<f32>); 2],
    /// Each half's junctions along its path, top-left then bottom-right,
    /// the second past the first.
    junctions: [(f32, f32); 2],
    /// Each half's apex along its path, between its junctions: the
    /// outline's support point straight up for the first, straight down
    /// for the second — where R27 centers a line.
    apexes: [f32; 2],
    /// The outline's length.
    length: f32,
}

impl Halves {
    /// The halves of `outline`, walked along `level` — its two paths as
    /// [`frame_halves`] builds them, with their level spans.
    ///
    /// Cost: O(stretches), and a binary search of an ellipse's tables.
    fn new(outline: &Outline, level: [(ArcPath, Range<f32>); 2]) -> Self {
        let paths = [&level[0].0, &level[1].0];
        let length = paths[0].length();
        let top_left = outline.support(Vec2::new(-1.0, -1.0));
        let bottom_right = outline.support(Vec2::new(1.0, 1.0));
        let tops = [
            outline.support(Vec2::new(0.0, -1.0)),
            outline.support(Vec2::new(0.0, 1.0)),
        ];
        let mut junctions = [(0.0, 0.0); 2];
        let mut apexes = [0.0; 2];
        for k in 0..2 {
            // A support that is not a number — an outline with a side
            // that is not one — is nowhere along the path, though the
            // point nearest it would be somewhere: its place is not a
            // number either, so the halves read as unmeasured.
            // A support is a point of the outline: found from the path's
            // own geometry, at no lookup (ruling R43).
            let along = |p: Vec2| {
                if p.is_finite() {
                    paths[k].locate(p).unwrap_or_else(|| paths[k].nearest(p))
                } else {
                    f32::NAN
                }
            };
            let start = along(top_left);
            let end = unwrap(along(bottom_right), start, length);
            junctions[k] = (start, end);
            apexes[k] = unwrap(along(tops[k]), start, length);
        }
        Halves {
            level,
            junctions,
            apexes,
            length,
        }
    }

    /// The halves of `outline`, its paths' tables sampled no coarser than
    /// `spacing`, their level spans with them ([`frame_halves`]): what the
    /// test seams lay on, the benches' among them.
    fn of(outline: &Outline, spacing: f32) -> Self {
        Halves::new(outline, frame_halves(outline, spacing))
    }

    /// Half `k`'s path: the outline walked from the middle of its left
    /// side, clockwise for the first half and counterclockwise for the
    /// second.
    fn path(&self, k: usize) -> &ArcPath {
        &self.level[k].0
    }

    /// Where along half `k` the point nearest `p` lies, past its start.
    fn along(&self, k: usize, p: Vec2) -> f32 {
        unwrap(self.path(k).nearest(p), self.junctions[k].0, self.length)
    }
}

/// `s` moved by whole turns of `length` to lie at or past `from`.
fn unwrap(s: f32, from: f32, length: f32) -> f32 {
    if length > 0.0 {
        from + (s - from).rem_euclid(length)
    } else {
        s
    }
}

/// The share of a half that the first of two clearances, `a` and `b`
/// (neither negative), claims where together they pass it: `a / (a + b)`,
/// in `[0, 1]`. Taken over the larger of the two, so that neither their
/// sum nor its product with the half's length can overflow: a clearance
/// grown toward a break its line's gaps make wider than the outline —
/// a line tracked over an outline some 10²⁶ long — is many times the
/// half, and the product once took the meeting point to infinity, where
/// a line's ends are not a number (the 300 s flow_structured run's
/// finding on #294). An infinite clearance claims the whole of the half
/// against a finite one, and two claim half each.
fn share(a: f32, b: f32) -> f32 {
    match (a.is_infinite(), b.is_infinite()) {
        (true, true) => 0.5,
        (true, false) => 1.0,
        (false, true) => 0.0,
        (false, false) => {
            let larger = a.max(b);
            if larger > 0.0 {
                let (a, b) = (a / larger, b / larger);
                a / (a + b)
            } else {
                0.0
            }
        }
    }
}

/// The distance between two boxes, `(min, max)` each: 0 where they meet.
fn box_gap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    (b.0 - a.1).max(a.0 - b.1).max(Vec2::ZERO).length()
}

/// How far apart two boxes, `(min, max)` each, are: their distance
/// ([`box_gap`]) where they are apart, and where they overlap how deep,
/// as a negative — the overlap along the axis it is least on, how far
/// one must move to clear the other — so that two inks drawn apart out
/// of an overlap measure how far they have still to go (ruling R43).
fn separation(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    let apart = (b.0 - a.1).max(a.0 - b.1);
    if apart.x > 0.0 || apart.y > 0.0 {
        apart.max(Vec2::ZERO).length()
    } else {
        apart.x.max(apart.y)
    }
}

/// Which way the ends of a part `(sa, sb)` of `path` move as its
/// clearance grows ([`Line::retreats`]): its start on along the path
/// after it, its end back along the path before it — at a sharp corner
/// the tangent at the end itself is the next side's, and an end drawn
/// back from a corner moves along the side before it.
///
/// Cost: two path lookups.
fn retreats(path: &ArcPath, (sa, sb): (f32, f32)) -> [Vec2; 2] {
    [path.at(sa).1, -path.at(sb - resolved(INK_TOLERANCE, sb)).1]
}

/// The center of a box, `(min, max)`.
fn middle(b: (Vec2, Vec2)) -> Vec2 {
    Vec2::new(b.0.x.midpoint(b.1.x), b.0.y.midpoint(b.1.y))
}

/// `run` with every glyph's [`GlyphMetrics::ink`] measured but a space's:
/// what tracking a line, and its side ornament, step by. A glyph with
/// no outline to measure — a bitmap emoji, a face with no outlines — is
/// inked as its box, its advance by the line height, so a line keeps it
/// and steps it like any other letter.
///
/// Each distinct glyph's outline is read once (#309): a glyph's ink is a
/// function of its face, its id, its size, its weight, its flags and its
/// offsets alone ([`glyph_outline_ink`]), so a glyph the text repeats —
/// most of a line's letters are — reads the ink its first read did, to
/// the bit.
///
/// Cost: one outline scale per distinct glyph that is not a space, and a
/// search of the distinct glyphs read so far per glyph.
fn inked(run: &ShapedRun, fs: &mut FontSystem) -> ShapedRun {
    #[cfg(test)]
    INKED_STEPS.with(|count| count.set(count.get() + inking_steps(run)));
    #[cfg(test)]
    INKINGS.with(|count| count.set(count.get() + 1));
    let mut out = run.clone();
    let mut cache = SwashCache::new();
    let laid: Vec<_> = run.layout_glyphs().cloned().collect();
    let (baseline, height) = (run.baseline(), run.line_height);
    let mut read: BTreeMap<InkKey, Option<(Vec2, Vec2)>> = BTreeMap::new();
    for (glyph, laid) in out.glyphs.iter_mut().zip(&laid) {
        if !glyph.blank {
            let boxed = Ink {
                min: Vec2::new(0.0, -baseline),
                max: Vec2::new(glyph.advance.max(0.0), height - baseline),
            };
            let ink = *read.entry(ink_key(laid)).or_insert_with(|| {
                #[cfg(test)]
                OUTLINE_READS.with(|count| count.set(count.get() + 1));
                glyph_outline_ink(fs, &mut cache, laid)
            });
            glyph.ink = Some(ink.map_or(boxed, |(min, max)| Ink { min, max }));
        }
    }
    out
}

/// What a laid-out glyph's ink is a function of ([`glyph_outline_ink`]):
/// its face, its glyph id, the bits of its size, its weight, its cache
/// flags' bits and the bits of its offsets.
type InkKey = (crate::font::FontId, u16, u32, u16, u32, u32, u32);

/// The [`InkKey`] of `glyph`.
fn ink_key(glyph: &crate::font::LayoutGlyph) -> InkKey {
    (
        glyph.font_id,
        glyph.glyph_id,
        glyph.font_size.to_bits(),
        glyph.font_weight.0,
        glyph.cache_key_flags.bits(),
        glyph.x_offset.to_bits(),
        glyph.y_offset.to_bits(),
    )
}

/// [`inked`], and each of `run`'s spans' band center below its baseline
/// (negative above it), a point of its size, by span ([`Band::centers`];
/// #308, ruling B15.1): halfway between the baseline and its x-height,
/// the ink of an `x` above the baseline in the face its glyphs are drawn
/// in — the face most of them are, the first of those, so a span's
/// letters drawn in a fallback face for a script its own lacks read the
/// span's own — measured from the outline, unhinted, so linear in the
/// size ([`face_char_ink`]), over that glyph's size. A span whose face
/// maps no `x`, or draws none, takes the middle of its first glyph's em
/// box instead, from its face's ascent to its descent; one the run holds
/// no glyph of takes not a number.
///
/// Cost: [`inked`]'s, and per span one character-map lookup and one
/// outline scale, in a face the run is already inked in: an
/// [`INK_GLYPH_STEPS`] a span ([`band_steps`]).
fn inked_with_bands(run: &ShapedRun, fs: &mut FontSystem) -> (ShapedRun, Vec<f32>) {
    let out = inked(run, fs);
    let mut cache = SwashCache::new();
    let laid: Vec<_> = run.layout_glyphs().cloned().collect();
    let spans = text_spans(run);
    let centers = (0..spans)
        .map(|span| {
            let of_span = || run.glyphs.iter().zip(&laid).filter(move |(g, _)| g.span == span);
            // The face most of the span's glyphs are drawn in.
            let mut faces: Vec<(_, usize, &crate::font::LayoutGlyph)> = Vec::new();
            for (_, glyph) in of_span() {
                match faces.iter_mut().find(|(id, _, _)| *id == glyph.font_id) {
                    Some(face) => face.1 += 1,
                    None => faces.push((glyph.font_id, 1, glyph)),
                }
            }
            let most = faces.iter().map(|f| f.1).max().unwrap_or(0);
            let measured = faces.iter().find(|f| f.1 == most).and_then(|&(_, _, glyph)| {
                let (min, _) = face_char_ink(fs, &mut cache, glyph, 'x')?;
                let per = -min.y / glyph.font_size;
                (per.is_finite() && per > 0.0).then_some(-0.5 * per)
            });
            let em = || {
                let (glyph, _) = of_span().next()?;
                let per = 0.5 * (glyph.descent - glyph.ascent) / glyph.size;
                per.is_finite().then_some(per)
            };
            measured.or_else(em).unwrap_or(f32::NAN)
        })
        .collect();
    (out, centers)
}

/// How many spans of text `run` holds: one past the greatest span index
/// of its glyphs but a filler's.
fn text_spans(run: &ShapedRun) -> usize {
    run.glyphs
        .iter()
        .filter(|g| g.span != FILLER_SPAN)
        .map(|g| g.span.saturating_add(1))
        .max()
        .unwrap_or(0)
}

/// The glyph steps reading `run`'s spans' band centers counts past its
/// inking ([`inked_with_bands`], #308): an [`INK_GLYPH_STEPS`] a span, the
/// `x` each reads in a face the run is inked in.
pub(crate) fn band_steps(run: &ShapedRun) -> usize {
    INK_GLYPH_STEPS.saturating_mul(text_spans(run))
}

/// `run` with no glyph's ink: what placement returns, a run's ink being
/// an ornament's own ([`GlyphMetrics::ink`]).
fn uninked(mut run: ShapedRun) -> ShapedRun {
    for glyph in &mut run.glyphs {
        glyph.ink = None;
    }
    run
}

thread_local! {
    /// Test counter (#294): the lines [`track`] has laid on this thread,
    /// what a frame's work is counted in.
    #[cfg(test)]
    pub(crate) static TRACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#308, ruling B15.3): the lines a slide has tracked
    /// on this thread ([`Layout::slid`]).
    #[cfg(test)]
    pub(crate) static SLIDE_TRACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps the weights
    /// give every shaping on this thread (`shape_run`, `reshape`),
    /// counted where it is shaped, not where a frame counts it.
    #[cfg(test)]
    pub(crate) static SHAPED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps the weights
    /// give every inking on this thread ([`inked`]), counted there.
    #[cfg(test)]
    pub(crate) static INKED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.2): the glyph steps every [`track`]
    /// on this thread is worth — its clusters and its call — counted
    /// there.
    #[cfg(test)]
    pub(crate) static TRACKED_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294): the times a frame's text has been built at a
    /// size on this thread — shaped, inked and its lines prepared —
    /// whether or not its clearance was then grown.
    #[cfg(test)]
    pub(crate) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test seam (#294, ruling R44.2): a frame budget to lay with in
    /// place of [`frame_budget`]'s — R27's sizing pass included, which
    /// lays under the same budget — so a test can spend one.
    #[cfg(test)]
    pub(crate) static BUDGET_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#294, rulings R44.1, R44.2): the least budget a frame
    /// lays with in place of [`TRACK_BUDGET`], so a test can put a long
    /// text's pressure on a shorter one.
    #[cfg(test)]
    pub(crate) static FLOOR_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#294, ruling R44.1): the glyphs a frame may place in
    /// place of [`MAX_PLACED_GLYPHS`], so a test can reach the cap with a
    /// short text.
    #[cfg(test)]
    pub(crate) static CAP_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#308, ruling B15.1): the bits of an `f32`
    /// ([`f32::to_bits`]) that a frame's band curve is pinned at in place
    /// of the offset its inks want ([`Layout::clear_at`]), so a test can
    /// lay frames at different spans on the very same curve — each frame
    /// otherwise solves its own.
    #[cfg(test)]
    pub(crate) static OFFSET_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam (#308, review 5801763546): the most times the frame's
    /// lines are laid again at the offset its side copies want, in place
    /// of [`OFFSET_ROUNDS`] — none, so a test can leave copies wanting
    /// more of the curve than it is.
    #[cfg(test)]
    pub(crate) static RELAY_ROUNDS_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test counter (#308): the side arcs left empty because the frame's
    /// curve lies nearer the outline than their copies want.
    #[cfg(test)]
    pub(crate) static ARCS_OFF_THE_CURVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.1): the times R27's fit has
    /// reshaped a frame's text on this thread (`lines::fit_reshape`).
    #[cfg(test)]
    pub(crate) static FIT_RESHAPES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, rulings R47.2, R50.4): the glyph steps frames
    /// on this thread have taken past their budget, less the share held
    /// back for [`untouched`] — where [`Work::spent_all`] reads it as
    /// spent: each take made once it was spent, and the part past it of a
    /// take that crosses it — but for [`untouched`]'s, which run within
    /// that share: all a frame does past its budget.
    #[cfg(test)]
    pub(crate) static PAST_BUDGET: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R46.1): the most glyph steps one
    /// [`untouched`] has taken on this thread, and its share
    /// ([`untouched_steps`]) for the inks it was given, of the classes of
    /// height its frame held back for.
    #[cfg(test)]
    pub(crate) static UNTOUCHED_TAKEN: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#294, rulings R44.3, R46.1): the pairs of boxes
    /// [`touching`] has looked at on this thread, each a [`box_gap`]
    /// weighed or a pair it passed over.
    #[cfg(test)]
    pub(crate) static BOXES_WEIGHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One line laid with one tracking gap ([`track`]).
#[derive(Clone)]
struct Tracked {
    /// Its glyphs, in text order.
    glyphs: Vec<PlacedGlyph>,
    /// How far before its part's end its last ink's center falls, by arc
    /// length: positive with room left, negative past it.
    short: f32,
    /// Its first and last ink boxes.
    ends: Option<((Vec2, Vec2), (Vec2, Vec2))>,
    /// Where along its half its first and last inks are set: where their
    /// anchors lie on the band curve, over the point of the outline
    /// nearest each.
    centers: Option<(f32, f32)>,
    /// The outline's outward normal under its first and last inks'
    /// anchors ([`Set::normal`]).
    normals: Option<(Vec2, Vec2)>,
    /// Each inked cluster's ink box, in text order.
    boxes: Vec<InkBox>,
    /// How many of [`glyphs`](Self::glyphs) come before each box's: where
    /// the line is cut to keep only the inks before it.
    marks: Vec<usize>,
    /// How far each box's ink reaches from its anchor toward the outline
    /// and away from it, along the outline's normal there ([`Reach`]), in
    /// the order of [`boxes`](Self::boxes): what the frame's offset is
    /// solved from ([`Layout::wants`], #308).
    reaches: Vec<Reach>,
}

/// How far an ink set on the frame's band curve reaches along the
/// outward normal `n` under its anchor (#308, ruling B15.1): toward the
/// outline and away from it, as the ellipse its box bounds does — the
/// support of that ellipse, from the anchor its center lies straight
/// above or below. An ink whose `toward` is the frame's offset touches
/// the outline's tangent there, as every ink did when each was pushed
/// off the outline by its own reach.
#[derive(Copy, Clone, Debug, Default)]
struct Reach {
    toward: f32,
    away: f32,
}

impl Reach {
    /// The reach of an ink whose center is `middle` below its anchor
    /// (negative above it), its half size `half`, along `normal`.
    fn of(middle: f32, half: Vec2, normal: Vec2) -> Reach {
        let support = (normal * half).length();
        let along = middle * normal.y;
        Reach {
            toward: support - along,
            away: support + along,
        }
    }
}

/// An ink as a frame line's pairwise rule sets it ([`pair`]).
#[derive(Copy, Clone, Debug)]
struct Set {
    /// Its center on the canvas, and its half size.
    at: Vec2,
    half: Vec2,
    /// Where its anchor lies on the frame's band curve (#308): the curve's
    /// point at [`center`](Self::center), where the curve's form on from it
    /// is taken ([`Form`], #309).
    on: Vec2,
    /// How the curve bends on from there, where its stretch has one form
    /// ([`Bend`]): what [`Form::of`] reads, and only it ([`Bending`]).
    bend: Bending,
    /// Where along the path it is set.
    center: f32,
    /// The way it moves there as its center moves on: the way its
    /// anchor moves along the frame's band curve
    /// ([`ArcPath::offset_at`]).
    moves: Vec2,
    /// The outline's outward normal its anchor is offset along
    /// ([`OffsetPoint::normal`](crate::flow::path::OffsetPoint::normal)):
    /// at a sharp corner, the way from the corner to it.
    normal: Vec2,
    /// How far it reaches toward and away from the outline (#308).
    reach: Reach,
}

/// How the band curve bends on from a set ink (#309): as the form that
/// set it says ([`model_set`]), or, set from the path, where on the path
/// it lies, for the path to tell only where the model reads it
/// ([`ArcPath::bend_of`]) — a lay that takes no form does none of a
/// bend's work.
#[derive(Copy, Clone, Debug)]
enum Bending {
    Formed(Option<Bend>),
    Read(OffsetIn),
}

impl Bending {
    /// The bend ([`Bend`]) of `path`'s band curve at `offset` this tells.
    ///
    /// Cost: none, formed; [`ArcPath::bend_of`]'s, read.
    fn of(self, path: &ArcPath, offset: f32) -> Option<Bend> {
        match self {
            Bending::Formed(bend) => bend,
            Bending::Read(at) => path.bend_of(at, offset),
        }
    }
}

/// The most inks [`pair`] tries for one letter, each an
/// [`InkLay::set_at`] — a path lookup — so that no glyph step of a
/// frame's line takes more than 64 (ruling R43): shared between its
/// neighbor and the inks before it ([`EARLIER`]).
const PAIR_ROUNDS: usize = 21;

/// How much nearer than owed [`pair`] may set a letter, in points: a
/// quarter of [`INK_TOLERANCE`], which it may set it further — so that
/// a gap measured afresh from the inks as placed, with the float error
/// of their positions, is not read as nearer than the line's by a
/// tolerance.
pub(crate) const NEARER: f32 = 0.25 * INK_TOLERANCE;

/// How many inks before a letter's neighbor it is held clear of too
/// ([`track`]): round a sharp corner inward a letter set at the corner
/// lets the next wrap back toward the one before it.
const EARLIER: usize = 3;

/// The least and most a letter's step along the path is taken as the
/// last letter's share of the straight path's answer ([`track`]'s warm
/// start): the bend of the path and the ink's push off it change that
/// share from one letter to the next only a little.
const PAIR_RATIO: (f32, f32) = (0.5, 2.0);

/// The rounds of bisection [`along`] closes its model in: a bracket of
/// some points to some millionths of one, all arithmetic.
const ALONG_ROUNDS: usize = 24;

/// The most times [`along`] doubles its bracket: past `2^24` times a
/// glyph's reach no step is modeled.
const ALONG_DOUBLINGS: usize = 24;

/// `clusters` laid along `walk`'s path over the part `[sa, sb]` with
/// tracking `g`: the first ink centered at `sa`; every letter after it
/// set by the pairwise rule (ruling R31, [`pair`]) `g` from the letter
/// before, a zero gap included; a space owing its advance and `g` more.
/// A space draws nothing and is placed past the last ink's box, where
/// the box rule would start its step. [`Tracked::short`] is how far
/// before `sb` the last ink's center falls, by arc length: it shrinks as
/// `g` grows, a zero gap included, so the end condition can be solved.
///
/// Each letter's first center tried is warm-started from the one before
/// (ruling R43): the straight path's answer along the way the last ink
/// moves, scaled by the share of it the last letter's step took.
///
/// Every ink is set by its anchor on the frame's band curve, `band`
/// (#308, ruling B15.1, [`InkLay::set_at`]).
///
/// Cost: O(clusters): a letter one [`InkLay::set_at`] where the warm
/// start holds, a few where the path bends, [`PAIR_ROUNDS`] at most; a
/// space one path lookup.
fn track(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    part: (f32, f32),
    g: f32,
    band: Band,
) -> Tracked {
    track_words(walk, glyphs, clusters, part, g, Words::NONE, band)
}

/// How a line's secondary rule stretches its word gaps (#309,
/// [`Layout::reach_its_end`]): each space at cluster `from` or past it
/// owes `by` more than its advance and the line's gap — less, where `by`
/// is negative. A line's letter gaps are its tracking gap whatever its
/// words owe.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Words {
    by: f32,
    from: usize,
}

impl Words {
    /// No stretch: every word gap its space's advance and two gaps.
    const NONE: Words = Words { by: 0.0, from: 0 };
}

/// A line its secondary rule brought to its part's end
/// ([`Layout::reach_its_end`]): its tracking gap, its words' stretch and
/// its lay.
struct Reached {
    gap: f32,
    words: Words,
    tracked: Tracked,
}

/// The cluster whose ink steps where a line's length does (#309,
/// [`Layout::reach_its_end`]): of `short`, a line laid at one gap, and
/// `past`, the same line laid at a gap a little wider, the first of their
/// inks the two set further apart than `allowance` — what the wider gap
/// moves an ink by, and the line's tolerance — and so the cluster of
/// `clusters` it is the ink of. `0`, the line's first, where none is.
///
/// Cost: O(clusters).
fn stepped(clusters: &[Cluster], short: &Tracked, past: &Tracked, allowance: f32) -> usize {
    let center = |b: &InkBox| (b.0 + b.1) * 0.5;
    let Some(k) = short
        .boxes
        .iter()
        .zip(&past.boxes)
        .position(|(a, b)| !crate::flow::path::length_of(center(b) - center(a)).le(&allowance))
    else {
        return 0;
    };
    // The cluster that placed the glyphs from `marks[k]` on.
    let Some(&mark) = short.marks.get(k) else {
        return 0;
    };
    let mut placed = 0;
    for (i, cluster) in clusters.iter().enumerate() {
        if placed >= mark && !cluster.blank {
            return i;
        }
        placed += cluster.glyphs.len();
    }
    0
}

/// [`track`] at tracking `g` with its word gaps stretched by `words`
/// (#309, [`Layout::reach_its_end`]).
///
/// Cost: [`track`]'s.
fn track_words(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    (sa, sb): (f32, f32),
    g: f32,
    words: Words,
    band: Band,
) -> Tracked {
    #[cfg(test)]
    TRACKS.with(|count| count.set(count.get() + 1));
    #[cfg(test)]
    TRACKED_STEPS.with(|count| count.set(count.get() + clusters.len() + TRACK_CALL_STEPS));
    let mut lay = InkLay::new(walk, glyphs, sa, 0.5 * g, band);
    for (i, cluster) in clusters.iter().enumerate() {
        if i == words.from {
            lay.stretch = words.by;
        }
        lay.cluster(cluster, g);
    }
    tracked_of(lay, (sa, sb))
}

/// [`track`], its last letter's center set at its part's end, `sb`, in
/// place of where the pairwise rule would set it (#309, the secondary
/// rule's last letter, [`Layout::reach_its_end`]): `None` where so set it
/// stands nearer its neighbor than the line's gap or further from it than
/// the pairwise rule's tolerance — or, where that is the line's only gap,
/// nearer than `floor`, its font's own gap — or nearer than its gap to the
/// inks before ([`EARLIER`]), or the line has no letter after its first.
///
/// Cost: [`track`]'s.
fn track_ending_at(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    (sa, sb): (f32, f32),
    (g, floor): (f32, f32),
    band: Band,
) -> Option<Tracked> {
    #[cfg(test)]
    TRACKS.with(|count| count.set(count.get() + 1));
    #[cfg(test)]
    TRACKED_STEPS.with(|count| count.set(count.get() + clusters.len() + TRACK_CALL_STEPS));
    let last = clusters.iter().rposition(|c| !c.blank)?;
    if !clusters[..last].iter().any(|c| !c.blank) {
        return None;
    }
    let mut lay = InkLay::new(walk, glyphs, sa, 0.5 * g, band);
    lay.pin_floor = floor;
    for (i, cluster) in clusters.iter().enumerate() {
        if i == last {
            lay.pin = Some(sb);
        }
        lay.cluster(cluster, g);
    }
    (!lay.refused).then(|| tracked_of(lay, (sa, sb)))
}

/// The line `lay` laid over `(sa, sb)` ([`Tracked`]).
fn tracked_of(lay: InkLay<'_>, (sa, sb): (f32, f32)) -> Tracked {
    let (short, ends, centers, normals) = match (lay.first, lay.last) {
        (Some(a), Some(b)) => (
            sb - b.center,
            Some(((a.at - a.half, a.at + a.half), (b.at - b.half, b.at + b.half))),
            Some((a.center, b.center)),
            Some((a.normal, b.normal)),
        ),
        _ => (sb - sa, None, None, None),
    };
    Tracked {
        glyphs: lay.out,
        short,
        ends,
        centers,
        normals,
        boxes: lay.boxes,
        marks: lay.marks,
        reaches: lay.reaches,
    }
}

/// A run of clusters being laid along a path by the pairwise rule, a
/// frame's line ([`track`]) or a side ornament's copies
/// ([`SideArc::lay`]): what is placed, and what the next letter is set
/// from.
struct InkLay<'w> {
    walk: &'w PathWalk<'w>,
    glyphs: &'w [GlyphMetrics],
    out: Vec<PlacedGlyph>,
    /// Each ink's box, in order, and how many glyphs come before it.
    boxes: Vec<InkBox>,
    marks: Vec<usize>,
    first: Option<Set>,
    last: Option<Set>,
    /// The inks before the last, nearest first.
    earlier: [Option<Set>; EARLIER],
    /// How far the next ink is owed from the last.
    owed: f32,
    /// How much more than its advance and the gap a space owes: a line's
    /// word gaps stretched by its secondary rule ([`Words`]).
    stretch: f32,
    /// Where along the path the next letter's center is set, pinned there
    /// rather than set by the pairwise rule (the secondary rule's last
    /// letter, [`track_ending_at`]); and whether a letter so pinned stood
    /// nearer its neighbors than the rule allows.
    pin: Option<f32>,
    refused: bool,
    /// The least a pinned letter's gap may be where it is its line's only
    /// gap: its font's own gap.
    pin_floor: f32,
    /// Where a space is placed: past the last ink's box.
    s: f32,
    /// The share of the straight path's answer the last letter's step
    /// took ([`track`]'s warm start).
    ratio: f32,
    /// The frame's band curve, where every ink's anchor lies (#308).
    band: Band,
    /// Each ink's reach from its anchor, in the order of
    /// [`boxes`](Self::boxes).
    reaches: Vec<Reach>,
    /// Whether it is a model lay (#309, ruling B16.2d): each letter set
    /// where the band curve's form puts it ([`model_set`]), no path
    /// lookup, but where the form ends or the letter would stand too near
    /// an ink before its neighbor, where the rule sets it from the path;
    /// and a space placed nowhere. What a side arc's gap is solved on
    /// before a lay is read from the path at it ([`SideArc::solve`]).
    modeled: bool,
    /// The letters a model lay has set from the band curve's form, and
    /// those it set from the path; and whether it stopped, the forms not
    /// reading the copies ([`MODEL_MISSES`]).
    formed: (usize, usize),
    abandoned: bool,
    /// Whether it lays a side ornament's copies, not a line: which work
    /// counter its steps are counted in ([`note_step`]).
    #[cfg(test)]
    side: bool,
}

/// A frame's band curve (#308, ruling B15.1): the outline's offset curve
/// at `offset` along its outward normal — one for the whole frame, its
/// two lines and its side ornaments — and where on each glyph its anchor
/// lies, the point of it set on that curve.
///
/// **The anchor.** A glyph's anchor is its ink's center along the line
/// and, across it, its run's band center: halfway between the baseline
/// and the x-height of its span's font, at its size — a property of the
/// font and the size, not of the glyph, so every glyph of a span shares
/// it, and a line of one span sits on one baseline, its ascenders and
/// descenders hanging freely off it ([`inked_with_bands`]). The x-height
/// body that carries the reading lies on the curve in both halves, so
/// the ring of text is one uniform offset of the node's shape. A side
/// ornament's glyph, which has no x-height to read by, is anchored at its
/// own ink's center.
#[derive(Clone)]
struct Band {
    /// The curve's offset off the outline, along its outward normal.
    offset: f32,
    /// Each span's band center below its baseline (negative above it),
    /// a point of its size, by span ([`GlyphMetrics::span`]).
    centers: Rc<[f32]>,
}

impl Band {
    /// How far below its baseline (negative above it) the anchor of a
    /// cluster whose first glyph is `glyph` and whose ink is `ink` lies:
    /// its span's band center at its size; a side ornament's ink center;
    /// for a span with no band center measured, the middle of its face's
    /// em box, from its ascent to its descent.
    fn anchor(&self, glyph: &GlyphMetrics, ink: Ink) -> f32 {
        if glyph.span == FILLER_SPAN {
            return 0.5 * (ink.min.y + ink.max.y);
        }
        match self.centers.get(glyph.span) {
            Some(&center) if center.is_finite() => center * glyph.size,
            _ => 0.5 * (glyph.descent - glyph.ascent),
        }
    }
}

impl<'w> InkLay<'w> {
    /// A lay along `walk` whose first ink is centered at `s`, unless one
    /// is set before it ([`after`](Self::after)), and owed `owed` from
    /// whatever comes before it, its inks anchored on `band`.
    fn new(walk: &'w PathWalk<'w>, glyphs: &'w [GlyphMetrics], s: f32, owed: f32, band: Band) -> Self {
        InkLay {
            walk,
            glyphs,
            out: Vec::new(),
            boxes: Vec::new(),
            marks: Vec::new(),
            first: None,
            last: None,
            earlier: [None; EARLIER],
            owed,
            stretch: 0.0,
            pin: None,
            refused: false,
            pin_floor: 0.0,
            s,
            ratio: 1.0,
            band,
            reaches: Vec::new(),
            modeled: false,
            formed: (0, 0),
            abandoned: false,
            #[cfg(test)]
            side: false,
        }
    }

    /// Where an ink of half size `half`, its center `rise` below its
    /// anchor (negative above it), is set at `c` along the path (#308,
    /// ruling B15.1): its anchor — straight below or above its center —
    /// on the frame's band curve ([`Band`], [`ArcPath::offset_at`]). So
    /// every ink of a span sits on one baseline, whatever its ink; and
    /// the way it moves is the curve's.
    ///
    /// Cost: one path lookup.
    fn set_at(&self, c: f32, half: Vec2, rise: f32) -> Set {
        let (on, at) = self.walk.path.offset_in(c, self.band.offset);
        Set {
            at: on.point + Vec2::new(0.0, rise),
            half,
            on: on.point,
            bend: Bending::Read(at),
            center: c,
            moves: on.moves,
            normal: on.normal,
            reach: Reach::of(rise, half, on.normal),
        }
    }

    /// The lay set on from the ink `before`, as though it were the last
    /// laid — a text ink a side ornament's copies start past — which is
    /// neither placed nor among [`boxes`](Self::boxes).
    fn after(mut self, before: Set) -> Self {
        self.s = before.center + extent(before.moves, before.half.x * 2.0, before.half.y * 2.0) * 0.5;
        self.last = Some(before);
        self
    }

    /// Lay `cluster` at gap `g`: a space owing its advance and `g` more,
    /// placed where the box rule would start its step; a letter set by
    /// the pairwise rule ([`pair`]) as far from the last ink as owed —
    /// and at least `g` from the few before it ([`EARLIER`]) — or, the
    /// first, at the lay's start. Every letter after owes `g`.
    ///
    /// Cost: a space one path lookup; a letter one [`set_at`](Self::set_at)
    /// where the warm start holds, [`PAIR_ROUNDS`] at most.
    fn cluster(&mut self, cluster: &Cluster, g: f32) {
        #[cfg(test)]
        let before = crate::flow::path::PATH_LOOKUPS.with(Cell::get);
        let walk = self.walk;
        if self.abandoned {
            return;
        }
        if cluster.blank && self.modeled {
            // A model lay places nothing: a space only owes its room.
            self.owed += cluster.advance + g + self.stretch;
            self.s += cluster.advance;
            return;
        }
        if cluster.blank {
            // Anchored on the band curve too, though it draws nothing.
            let at = walk.path.offset_at(self.s, self.band.offset).point;
            let none = Ink {
                min: Vec2::ZERO,
                max: Vec2::ZERO,
            };
            for index in cluster.glyphs.clone() {
                let anchor = self.band.anchor(&self.glyphs[index], none);
                self.out.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: at - Vec2::new(0.5 * cluster.advance, walk.baseline + anchor),
                });
            }
            self.owed += cluster.advance + g + self.stretch;
            self.s += cluster.advance;
            #[cfg(test)]
            note_step(before, self.side);
            return;
        }
        let Body::Inked { advance, ink } = walk.body_of(cluster) else {
            return;
        };
        let size = ink.max - ink.min;
        let half = size * 0.5;
        // Its ink's center below its anchor (#308).
        let rise = 0.5 * (ink.min.y + ink.max.y) - self.band.anchor(&self.glyphs[cluster.glyphs.start], ink);
        // A model lay's letter, where its form sets it.
        let modeled = match (self.modeled, self.last, self.pin) {
            (true, Some(before), None) => {
                // The forms do not read these copies — their canvas too
                // coarse for any, or more of them read from the path than
                // a few ends of forms round corners take: the lay stops
                // rather than read them from the path.
                if resolution_at(&before, before.half + half, self.owed).is_none() {
                    self.abandoned = true;
                    return;
                }
                let set = self.model_letter(&before, (half, rise), g);
                if set.is_some() {
                    self.formed.0 += 1;
                } else {
                    self.formed.1 += 1;
                    if self.formed.1 > self.formed.0 + MODEL_MISSES {
                        self.abandoned = true;
                        return;
                    }
                }
                set
            }
            _ => None,
        };
        let set = match (self.last, self.pin.take(), modeled) {
            (_, _, Some(set)) => {
                #[cfg(test)]
                MODEL_LETTERS.with(|count| count.set(count.get() + 1));
                set
            }
            // Set where it is pinned, the gaps there held to the rule's:
            // no nearer its neighbor than owed, nor its gap to the inks
            // before that (the secondary rule, `Layout::reach_its_end`).
            (Some(before), Some(at), None) => {
                let set = self.set_at(at, half, rise);
                let apart = |e: &Set| {
                    ((set.at - e.at).abs() - (set.half + e.half))
                        .max(Vec2::ZERO)
                        .length()
                };
                let gap = apart(&before);
                let clear = g.max(2.0 * TOUCH);
                let owed = self.owed.max(0.0);
                // A line's only gap has no other to be even with: it takes
                // what the end leaves, no tighter than its font's own gap.
                // Any other is held to the pairwise rule's window, so the
                // line stays even.
                let held = if self.boxes.len() == 1 {
                    gap >= self.pin_floor - NEARER
                } else {
                    gap >= owed - NEARER && gap <= owed + INK_TOLERANCE
                };
                if !(gap > 0.0 && held) || self.earlier.iter().flatten().any(|e| apart(e) < clear - NEARER) {
                    self.refused = true;
                }
                set
            }
            (Some(before), None, None) => {
                let owed = self.owed.max(0.0);
                let straight = along(Vec2::ZERO, before.moves, before.half + half, owed);
                // The first center tried: its root on the band curve's
                // form at the last ink, aimed at what is owed ([`osculating`];
                // #309, rulings B16.1b, B16.2c); else the straight path's
                // answer, scaled by the share of it the last letter's step
                // took.
                let model = osculating(
                    walk.path,
                    &before,
                    self.earlier[0].as_ref(),
                    (half, rise),
                    owed,
                    self.band.offset,
                );
                let guess = match model {
                    Some(at) => at,
                    None if straight.is_finite() => before.center + straight * self.ratio,
                    None => f32::NAN,
                };
                let mut rounds = PAIR_ROUNDS;
                let mut set = pair(self, before, (half, rise), guess, owed, &mut rounds);
                #[cfg(test)]
                if model.is_some() {
                    let gap = ((set.at - before.at).abs() - (before.half + half))
                        .max(Vec2::ZERO)
                        .length();
                    let miss = gap - owed;
                    if (-NEARER..=INK_TOLERANCE).contains(&miss) {
                        MODEL_GAPS.with(|count| {
                            let (n, sum, squares) = count.get();
                            let miss = f64::from(miss);
                            count.set((n + 1, sum + miss, squares + miss * miss));
                        });
                    }
                }
                #[cfg(test)]
                OSCULATED.with(|count| {
                    let (offered, first, misses) = count.get();
                    count.set(match model {
                        Some(_) if rounds + 1 == PAIR_ROUNDS => (offered + 1, first + 1, misses),
                        Some(_) => (offered + 1, first, misses),
                        None => (offered, first, misses + 1),
                    });
                });
                // Clear of the inks before its neighbor too, by its gap —
                // and past touching, at no gap — moved on to the nearest
                // center the rule finds that far from the one it is too
                // near: a letter is never nearer than its gap to any
                // letter but its neighbors (rulings R34, R38.1). Near is
                // what `pair` would move: short of its acceptance, so a
                // round never solves for the center it already has.
                let clear = g.max(2.0 * TOUCH);
                for _ in 0..EARLIER {
                    let near = |e: &Set| {
                        ((set.at - e.at).abs() - (set.half + e.half))
                            .max(Vec2::ZERO)
                            .length()
                            < clear - NEARER
                    };
                    let Some(e) = self.earlier.iter().flatten().find(|e| near(e)).copied() else {
                        break;
                    };
                    // `pair`'s first try is the center the letter has,
                    // which is too near: a round with one try left would
                    // only set it there again (ruling R44.7) — as round a
                    // sharp corner inward on the band curve, where the
                    // neighbor's own pairing takes all but one (#308).
                    if rounds < 2 {
                        break;
                    }
                    #[cfg(test)]
                    let was = set.center;
                    set = pair(self, e, (half, rise), set.center, clear, &mut rounds);
                    #[cfg(test)]
                    EARLIER_ROUNDS.with(|count| {
                        let (moved, idle) = count.get();
                        count.set(if set.center == was {
                            (moved, idle + 1)
                        } else {
                            (moved + 1, idle)
                        });
                    });
                }
                let taken = (set.center - before.center) / straight;
                self.ratio = if taken.is_finite() {
                    taken.clamp(PAIR_RATIO.0, PAIR_RATIO.1)
                } else {
                    1.0
                };
                set
            }
            // The first ink centered at the lay's start.
            (None, _, _) => self.set_at(self.s, half, rise),
        };
        let corner = set.at - (ink.min + ink.max) * 0.5 - Vec2::new(0.0, walk.baseline);
        self.marks.push(self.out.len());
        for index in cluster.glyphs.clone() {
            self.out.push(PlacedGlyph {
                glyph: index as u32,
                pos: corner + Vec2::new(self.glyphs[index].x - cluster.left, 0.0),
            });
        }
        self.first.get_or_insert(set);
        self.boxes.push((set.at - half, set.at + half));
        self.reaches.push(set.reach);
        self.earlier.rotate_right(1);
        self.earlier[0] = self.last;
        self.s = set.center + 0.5 * extent(set.moves, size.x, size.y) + (advance - ink.max.x);
        self.last = Some(set);
        self.owed = g;
        // A letter a model lay set from its form took no step of the path:
        // one it read from the path did, and is counted as one.
        #[cfg(test)]
        if crate::flow::path::PATH_LOOKUPS.with(Cell::get) > before || !self.modeled {
            note_step(before, self.side);
        }
    }

    /// The letter of half size `half`, its center `rise` below its anchor,
    /// a model lay sets after `before` ([`modeled`](Self::modeled)): where
    /// the band curve's form puts it ([`model_set`]), if that is clear of
    /// the inks before its neighbor by `g` — and past touching — as the
    /// rule holds it; else `None`, and the rule sets it from the path.
    ///
    /// Cost: [`model_set`]'s, no path lookup.
    fn model_letter(&self, before: &Set, letter: (Vec2, f32), g: f32) -> Option<Set> {
        let owed = self.owed.max(0.0);
        let set = model_set(
            self.walk.path,
            before,
            self.earlier[0].as_ref(),
            letter,
            owed,
            self.band.offset,
        )?;
        let clear = g.max(2.0 * TOUCH);
        let near = |e: &Set| {
            ((set.at - e.at).abs() - (set.half + e.half))
                .max(Vec2::ZERO)
                .length()
                < clear - NEARER
        };
        (!self.earlier.iter().flatten().any(near)).then_some(set)
    }
}

thread_local! {
    /// Test counter (#294, ruling R43): the most path lookups one glyph
    /// step of [`track`] has taken on this thread.
    #[cfg(test)]
    pub(crate) static MOST_STEP_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294, ruling R43): the glyph steps [`track`] has
    /// taken on this thread, and the path lookups they took, what
    /// [`MOST_STEP_LOOKUPS`] is the most of.
    #[cfg(test)]
    pub(crate) static STEP_LOOKUPS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#294, ruling R44.7): the rounds a letter has been
    /// moved clear of an ink before its neighbor ([`EARLIER`]) on this
    /// thread, and of them the rounds that left it where it was.
    #[cfg(test)]
    pub(crate) static EARLIER_ROUNDS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#309): of [`STEP_LOOKUPS`], the glyph steps a side
    /// ornament's lays took on this thread and the path lookups they took.
    #[cfg(test)]
    pub(crate) static SIDE_STEP_LOOKUPS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#309): the runs a frame's placement has shaped or
    /// reshaped on this thread — R27's fit, a build at a size, a side
    /// ornament's runs.
    #[cfg(test)]
    pub(crate) static RESHAPES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#309, ruling B16.1b): the letters whose first center
    /// tried was their root on the band curve's form ([`osculating`]), of
    /// them those that center held, and the letters that had no form.
    #[cfg(test)]
    pub(crate) static OSCULATED: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
    /// Test counter (#309, ruling B16.2c): over the letters whose first
    /// center tried was their root on the band curve's form and whose gap
    /// the rule took ([`pair`]'s window), how many, and the sum of how far
    /// each gap is from what is owed and of its square: what the layer's
    /// bias is measured by.
    #[cfg(test)]
    pub(crate) static MODEL_GAPS: std::cell::Cell<(usize, f64, f64)> = const { std::cell::Cell::new((0, 0.0, 0.0)) };
    /// Test counter (#309, ruling B16.2d): the letters model lays set from
    /// the band curve's form ([`InkLay::modeled`]), no step of the path.
    #[cfg(test)]
    pub(crate) static MODEL_LETTERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test control (#309): with it set, no line's secondary rule runs
    /// ([`Layout::reach_its_end`]) — what the rule's pins are shown red
    /// without.
    #[cfg(test)]
    pub(crate) static NO_SECONDARY_RULE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test control (#309, ruling B16.1b): with it set, the closed-form
    /// layer is off — no letter's first center is its root on the band
    /// curve's form ([`osculating`]), and no side arc's gap is solved on
    /// it first ([`SideArc::solve`]) — and the frame is laid as the
    /// iterative rule alone lays it: what the layer's pins are shown red
    /// without, and what its effect is measured against.
    #[cfg(test)]
    pub(crate) static NO_CLOSED_FORM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test counter (#309): the lines frames on this thread kept as their
    /// secondary rule brought them to their ends ([`Layout::reach_ends`]),
    /// by how: their last letter set at the end, their word gaps past the
    /// step stretched, or shrunk.
    #[cfg(test)]
    pub(crate) static RULE_KEPT: std::cell::Cell<[usize; 3]> = const { std::cell::Cell::new([0; 3]) };
    /// Test counter (#309, the B9 final review 5757429244): the one-gap
    /// lines frames laid on this thread at normal scale, their parts' ends
    /// within 10¹⁵ of the origin, and of them the ones ending past their
    /// tolerance of their part's end — what the secondary rule brings home
    /// ([`Layout::reach_its_end`]).
    #[cfg(test)]
    pub(crate) static ONE_GAP_LINES: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// Test counter (#309): the lines a frame's [`TrackMemo`] answered on
    /// this thread, each a [`track`] not laid again.
    #[cfg(test)]
    pub(crate) static TRACK_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test switch (#309, review 5810838863): a frame's [`TrackMemo`]
    /// answers no line, so each is laid again — what a test lays a frame
    /// with to hold the memo's lines, and what they count, to those it
    /// stands in for.
    #[cfg(test)]
    pub(crate) static NO_TRACK_MEMO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test counter (#309): the glyph outlines [`inked`] has read on this
    /// thread, each distinct glyph of a run once.
    #[cfg(test)]
    pub(crate) static OUTLINE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#309): the runs a frame's placement has inked on this
    /// thread ([`inked`]).
    #[cfg(test)]
    pub(crate) static INKINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test counter (#294, ruling R43): one glyph step of [`track`] or of a
/// side ornament's lay (`side`) taken, the path lookups since `before` its
/// cost.
#[cfg(test)]
fn note_step(before: usize, side: bool) {
    let spent = crate::flow::path::PATH_LOOKUPS.with(Cell::get) - before;
    MOST_STEP_LOOKUPS.with(|most| most.set(most.get().max(spent)));
    let add = |counts: &std::cell::Cell<(usize, usize)>| {
        let (steps, lookups) = counts.get();
        counts.set((steps + 1, lookups + spent));
    };
    STEP_LOOKUPS.with(add);
    if side {
        SIDE_STEP_LOOKUPS.with(add);
    }
}

/// Test counter (#309): one run shaped or reshaped by a frame's
/// placement ([`RESHAPES`]).
#[cfg(test)]
pub(crate) fn note_reshape() {
    RESHAPES.with(|count| count.set(count.get() + 1));
}

/// Where to set a letter whose ink is of half size `half`, its center
/// `rise` below its anchor, after the ink `last` so that the two are `owed` apart, box to box (ruling R31):
/// the first center tried whose gap is no more than [`NEARER`] short of
/// that and no more than [`INK_TOLERANCE`] past it,
/// more than none apart — boxes no gap apart may overlap, so a zero gap is
/// never near enough, though nothing is owed — or, where none is, the
/// nearest tried past it at which the ink is at least `owed` away: a
/// line's letters are never nearer than its gap (rulings R31, R34). With
/// nothing tried that far, the furthest tried.
///
/// `guess` is tried first ([`track`]'s warm start). Each next center is
/// where the two would be `owed` apart were the path straight on from the
/// last center tried, along the way its ink moves there ([`along`]): exact
/// where it is, and close where the path bends. It is held inside the
/// bracket the centers tried have closed — one too near, one far enough
/// — by regula falsi (Illinois) where it falls outside, and, while
/// nothing tried is far enough, to a stride on. Where that step is less
/// than the center's float can take, the next float toward the root is
/// tried (#309, ruling B16.2c). The gap is continuous in
/// the center, so the bracket holds a root, and the line's length moves
/// continuously with its gap wherever the root it closes on does: what
/// [`solve_tracking`] solves (ruling R43).
///
/// Each center tried is set by its anchor on `lay`'s band curve
/// ([`InkLay::set_at`]).
///
/// Cost: one [`InkLay::set_at`] where `guess` holds, a few where the
/// path bends, [`PAIR_ROUNDS`] at most.
fn pair(
    lay: &InkLay<'_>,
    last: Set,
    (half, rise): (Vec2, f32),
    guess: f32,
    owed: f32,
    rounds: &mut usize,
) -> Set {
    let reach = last.half + half;
    let set_at = |c: f32| lay.set_at(c, half, rise);
    let gap_of = |set: &Set| ((set.at - last.at).abs() - reach).max(Vec2::ZERO).length();
    let stride = owed + 2.0 * reach.max_element() + INK_TOLERANCE;
    // Too near: the last ink's own center, where the gap is none; then
    // the furthest tried that is too near, with its miss.
    let (mut low, mut low_miss) = (last.center, -owed);
    // Far enough: the nearest tried that is, with its miss.
    let mut high: Option<(Set, f32)> = None;
    // Which end the last center tried moved: an end kept twice running
    // has the other's miss halved (Illinois).
    let mut kept: Option<bool> = None;
    // Whether the model still steers: while each step it takes at least
    // halves the miss. Round a sharp corner outward an ink swings round
    // the corner as its center crosses it, which the straight path does
    // not model, and the model's steps fall either side of the root.
    let mut modeled = true;
    let mut last_miss = f32::INFINITY;
    // The bracket's width when it last halved.
    let mut width = f32::INFINITY;
    let mut tried: Option<Set> = None;
    let mut c = if guess > low && guess.is_finite() {
        guess
    } else {
        low + stride
    };
    while *rounds > 0 {
        *rounds -= 1;
        let set = set_at(c);
        let gap = gap_of(&set);
        if gap > 0.0 && gap >= owed - NEARER && gap <= owed + INK_TOLERANCE {
            return set;
        }
        tried = Some(set);
        // A gap that is not a number is not far enough.
        let miss = gap - owed;
        if miss > 0.0 {
            if high.is_none_or(|(h, _)| c < h.center) {
                if kept == Some(true) {
                    low_miss *= 0.5;
                }
                kept = Some(true);
                high = Some((set, miss));
            }
        } else if c > low && high.is_none_or(|(h, _)| c < h.center) {
            if kept == Some(false) {
                if let Some((_, far)) = high.as_mut() {
                    *far *= 0.5;
                }
            }
            kept = Some(false);
            (low, low_miss) = (c, if miss.is_nan() { -owed } else { miss });
        }
        if !miss.abs().lt(&(0.5 * last_miss)) {
            modeled = false;
        }
        last_miss = miss.abs();
        let step = along(set.at - last.at, set.moves, reach, owed);
        let model = c + step;
        // A step the center's float cannot take: the root lies within a
        // float of it, and the next float toward it is tried — the nudge
        // of the closed-form layer (#309, ruling B16.2c) — where the rule
        // alone strides on and bisects back to it.
        let model = if model.to_bits() == c.to_bits() && step != 0.0 && layer_on() {
            if step > 0.0 {
                c.next_up()
            } else {
                c.next_down()
            }
        } else {
            model
        };
        let next = match high {
            None if model > low => model.min(low + stride),
            None => low + stride,
            Some((h, h_miss)) => {
                let now = h.center - low;
                let halved = now <= 0.5 * width;
                if halved {
                    width = now;
                }
                // The line through the bracket's ends.
                let crossing = low + now * (-low_miss) / (h_miss - low_miss);
                let inside = |x: f32| x > low && x < h.center;
                if !halved && width.is_finite() {
                    low.midpoint(h.center)
                } else if modeled && inside(model) {
                    model
                } else if inside(crossing) {
                    crossing
                } else {
                    low.midpoint(h.center)
                }
            }
        };
        // A bracket closed to a float, or a step that leaves the center
        // where it was, finds nothing more.
        if !next.gt(&low) || next.to_bits() == c.to_bits() || high.is_some_and(|(h, _)| next >= h.center) {
            break;
        }
        c = next;
    }
    match (high, tried) {
        (Some((h, _)), _) => h,
        (None, Some(t)) => t,
        (None, None) => set_at(c),
    }
}

/// The step along unit `dir` from where two inks' centers are `delta`
/// apart, their half sizes summing to `reach`, after which their boxes
/// are `owed` apart: negative where they are further apart than that
/// already — the pairwise rule's model of a path straight on from a
/// center tried ([`pair`]), solved by bisection on its own arithmetic,
/// with no path lookup. `NaN` where no step on or back sets them so.
///
/// Cost: O(1): [`ALONG_DOUBLINGS`] and [`ALONG_ROUNDS`] rounds at most.
fn along(delta: Vec2, dir: Vec2, reach: Vec2, owed: f32) -> f32 {
    let miss = |d: f32| ((delta + dir * d).abs() - reach).max(Vec2::ZERO).length() - owed;
    let at = miss(0.0);
    if !at.is_finite() {
        return f32::NAN;
    }
    // Too near, the step is on; too far, back. Boxes no gap apart may
    // overlap: none apart is too near.
    let near = |m: f32| m < 0.0 || (m <= 0.0 && owed <= 0.0);
    let on = near(at);
    // A bracket from here, doubled until it holds the step.
    let unit = (owed + reach.max_element()).max(INK_TOLERANCE);
    let sign = if on { 1.0 } else { -1.0 };
    let (mut inside, mut outside) = (0.0_f32, sign * unit);
    let mut doublings = 0;
    while near(miss(outside)) == on {
        if doublings == ALONG_DOUBLINGS || !outside.is_finite() {
            return f32::NAN;
        }
        inside = outside;
        outside *= 2.0;
        doublings += 1;
    }
    for _ in 0..ALONG_ROUNDS {
        let middle = inside.midpoint(outside);
        if near(miss(middle)) == on {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    outside
}

/// How many more of its letters a model lay may read from the path than
/// it sets from the band curve's forms before it stops ([`InkLay::modeled`],
/// #309): a few, where forms end round corners; past them the forms do
/// not read the copies, and a lay read from the path costs no more.
const MODEL_MISSES: usize = 8;

/// Whether the closed-form layer is on (#309, ruling B16.1b): always, but
/// in a test that turns it off (`NO_CLOSED_FORM`).
fn layer_on() -> bool {
    #[cfg(test)]
    if NO_CLOSED_FORM.with(Cell::get) {
        return false;
    }
    true
}

/// The most Newton rounds [`model_root`] takes on a form of the band
/// curve that bends: from the tangent line's root a round closes the
/// miss quadratically, so a few reach the float resolution of the gap.
const MODEL_ROUNDS: usize = 8;

/// The band curve on from an ink, in the form its stretch has there
/// (#309, ruling B16.1b), so that a point on along it is arithmetic, no
/// path lookup: the osculating circle of a side or an arc — a side's of no
/// curvature — the stretch round a sharp corner outward, or the ellipse
/// an ellipse's table samples. Each answers the curve `u` on from where
/// it was taken, `u` its own measure of the way: arc length along the
/// path, or on an ellipse its parameter.
#[derive(Copy, Clone, Debug)]
enum Form<'p> {
    /// The circle tangent to the way the ink's anchor moves, of curvature
    /// `kappa` along the path, walked at `speed`: `base` the outline's
    /// point under the anchor, the anchor `offset` along its normal `n0`,
    /// the way it moves `t0`; held `most` on, a half turn at most.
    Circle {
        base: Vec2,
        t0: Vec2,
        n0: Vec2,
        kappa: f32,
        speed: f32,
        offset: f32,
        most: f32,
    },
    /// The stretch round a sharp corner outward ([`CornerForm`]), taken
    /// `from` along the path, held `most` on.
    Corner { form: CornerForm, from: f32, most: f32 },
    /// The ellipse ([`EllipseForm`]) and its stretch's table, read as the
    /// path reads it.
    Ellipse {
        form: EllipseForm,
        table: (&'p [f32], &'p [f32]),
    },
}

impl<'p> Form<'p> {
    /// The form of `path`'s band curve at `offset` on from the ink
    /// `last`, an ink `earlier` set before it: `None` where its stretch has
    /// none the model reads ([`Bend`]: inside a bend inward), or a table's
    /// curve has no ink before it to tell its turn, or its curvature is not
    /// finite.
    fn of(path: &'p ArcPath, last: &Set, earlier: Option<&Set>, offset: f32) -> Option<Form<'p>> {
        let bend = last.bend.of(path, offset)?;
        if let Some(form) = bend.ellipse {
            return Some(Form::Ellipse {
                form,
                table: path.table(form.stretch())?,
            });
        }
        if let Some(form) = bend.corner {
            return Some(Form::Corner {
                form,
                from: last.center,
                most: form.reach(last.center),
            });
        }
        let kappa = match bend.curvature {
            Some(kappa) => kappa,
            // A table's: the turn from the ink before over the path between.
            None => {
                let earlier = earlier?;
                let run = last.center - earlier.center;
                let (a, b) = (earlier.moves, last.moves);
                let turn = (a.x * b.y - a.y * b.x).atan2(a.dot(b));
                if !run.gt(&0.0) || earlier.bend.of(path, offset).is_none() {
                    return None;
                }
                turn / run
            }
        };
        (kappa.is_finite() && bend.speed.is_finite()).then_some(Form::Circle {
            base: last.on - last.normal * offset,
            t0: last.moves,
            n0: last.normal,
            kappa,
            speed: bend.speed,
            offset,
            most: (std::f32::consts::PI / kappa.abs()).min(bend.holds),
        })
    }

    /// Whether the curve is straight where the form was taken: a side, or
    /// a side's offset beside a corner's sweep.
    fn straight(&self) -> bool {
        match *self {
            Form::Circle { kappa, .. } => kappa == 0.0,
            Form::Corner { form, from, .. } => !form.swept(from),
            Form::Ellipse { .. } => false,
        }
    }

    /// The furthest on, in the form's own measure, that it holds.
    fn most(&self) -> f32 {
        match *self {
            Form::Circle { most, .. } | Form::Corner { most, .. } => most,
            // To the table's end, a half turn of the parameter at most.
            Form::Ellipse { form, table } => form.to_end(table).min(std::f32::consts::PI),
        }
    }

    /// The curve `u` on: its point, normal, way and velocity a unit of
    /// `u` ([`FormPoint`]); `None` where the form no longer holds.
    ///
    /// Cost: O(1), no path lookup.
    fn on(&self, u: f32) -> Option<FormPoint> {
        match *self {
            Form::Circle {
                base,
                t0,
                n0,
                kappa,
                speed,
                offset,
                ..
            } => {
                let phi = kappa * u;
                let (sin, cos) = phi.sin_cos();
                let perp = Vec2::new(-t0.y, t0.x);
                let chord = if phi.abs() < 1e-4 {
                    (t0 * u + perp * (0.5 * kappa * u * u)) * speed
                } else {
                    (t0 * sin + perp * (1.0 - cos)) * (speed / kappa)
                };
                let turn = |v: Vec2| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
                let (moves, normal) = (turn(t0), turn(n0));
                Some(FormPoint {
                    point: base + chord + normal * offset,
                    normal,
                    moves,
                    velocity: moves * speed + Vec2::new(-normal.y, normal.x) * (kappa * offset),
                })
            }
            Form::Corner { form, from, .. } => form.at(from + u),
            Form::Ellipse { form, .. } => Some(form.on(u)),
        }
    }

    /// The least step from `u` that moves the form's point: a unit of the
    /// last place of what it is measured from — the path's own measure
    /// round a corner, the ellipse's parameter, else `u` itself.
    fn quantum(&self, u: f32) -> f32 {
        match *self {
            Form::Circle { .. } => ulp(u),
            Form::Corner { from, .. } => ulp(from + u),
            Form::Ellipse { form, .. } => ulp(form.parameter() + u),
        }
    }

    /// How far along the path `u` on is: `None` past the ellipse's table.
    fn along(&self, u: f32) -> Option<f32> {
        match *self {
            Form::Circle { .. } | Form::Corner { .. } => Some(u),
            Form::Ellipse { form, table } => Some(form.walked(u, table)?.local() - form.local()),
        }
    }

    /// The form taken `u` on, `at` the curve's point there: what an ink
    /// set there reads on from it, no path lookup ([`model_set`]).
    fn moved(&self, u: f32, at: &FormPoint) -> Option<Form<'p>> {
        match *self {
            Form::Circle {
                kappa,
                speed,
                offset,
                most,
                ..
            } => (most > u).then_some(Form::Circle {
                base: at.point - at.normal * offset,
                t0: at.moves,
                n0: at.normal,
                kappa,
                speed,
                offset,
                most: most - u,
            }),
            Form::Corner { form, from, .. } => form.at(from + u).map(|_| Form::Corner {
                form,
                from: from + u,
                most: form.reach(from + u),
            }),
            Form::Ellipse { form, table } => Some(Form::Ellipse {
                form: form.walked(u, table)?,
                table,
            }),
        }
    }
}

/// The step along `dir` from where two inks' centers are `delta` apart,
/// their half sizes summing to `reach`, after which their boxes are
/// `owed` apart, in closed form (#309, ruling B16.1b): what [`along`]
/// brackets and bisects, solved exactly. Each axis's box gap is the
/// distance past `reach` along it, linear in the step but where it
/// reaches `reach` or crosses none, so between those steps — at most six
/// — the gap's square is a quadratic in the step, and the first of them
/// to reach `owed` holds its root: on, where the boxes are nearer than
/// `owed` now; back, a negative, where they are further. Boxes no gap
/// apart may overlap: where nothing is owed, the step to where they
/// begin to part. `NaN` where no step on or back sets them so, or a
/// number on the way is not finite.
///
/// Cost: O(1): six quadratics at most.
fn line_root(delta: Vec2, dir: Vec2, reach: Vec2, owed: f32) -> f32 {
    let gap = |d: f32| ((delta + dir * d).abs() - reach).max(Vec2::ZERO).length();
    let at = gap(0.0) - owed;
    if !at.is_finite() || !dir.is_finite() || !reach.is_finite() {
        return f32::NAN;
    }
    let on = at < 0.0 || (at <= 0.0 && owed <= 0.0);
    let sign = if on { 1.0 } else { -1.0 };
    let v = dir * sign;
    // Where an axis's gap begins or ends, or its center crosses the other's.
    let mut stops = [f32::INFINITY; 7];
    let mut n = 0;
    for (d, w, r) in [(delta.x, v.x, reach.x), (delta.y, v.y, reach.y)] {
        if w != 0.0 {
            for edge in [r, -r, 0.0] {
                let t = (edge - d) / w;
                if t > 0.0 && t.is_finite() {
                    stops[n] = t;
                    n += 1;
                }
            }
        }
    }
    stops[..n].sort_unstable_by(f32::total_cmp);
    let owed = owed.max(0.0);
    let mut lo = 0.0_f32;
    for &hi in &stops[..=n] {
        // The axes apart between the two stops, and the gap's square
        // there in the step past `lo`: `a τ² + 2 b τ + c`, `c` less what
        // is owed squared.
        let probe = delta + v * if hi.is_finite() { lo.midpoint(hi) } else { lo + 1.0 };
        let (mut a, mut b, mut c) = (0.0_f32, 0.0_f32, 0.0_f32);
        // Whether either axis is apart there, at whatever rate.
        let mut apart = false;
        for (p, d, w, r) in [(probe.x, delta.x, v.x, reach.x), (probe.y, delta.y, v.y, reach.y)] {
            if p.abs() > r {
                apart = true;
                let side = p.signum();
                let from = side * (d + w * lo) - r;
                let rate = side * w;
                a += rate * rate;
                b += from * rate;
                c += from * from;
            }
        }
        c -= owed * owed;
        let root = if on {
            if owed <= 0.0 {
                // Nothing owed: where the boxes begin to part.
                apart.then_some(0.0)
            } else if c >= 0.0 {
                Some(0.0)
            } else if a > 0.0 {
                let root = -c / (b + (b * b - a * c).sqrt());
                root.is_finite().then_some(root)
            } else {
                None
            }
        } else if !apart {
            // No gap left between the stops: near from where they begin.
            Some(0.0)
        } else if owed <= 0.0 || a == 0.0 {
            // A gap there that the step does not close, or one that must
            // close to none: on to the next stop.
            None
        } else if c <= 0.0 {
            Some(0.0)
        } else {
            let disc = b * b - a * c;
            (b < 0.0 && disc >= 0.0)
                .then(|| c / (-b + disc.sqrt()))
                .filter(|root| root.is_finite())
        };
        if let Some(root) = root.filter(|root| lo + root <= hi) {
            return sign * (lo + root);
        }
        lo = hi;
    }
    f32::NAN
}

/// The most forms of the band curve [`model_root`] reads a root across,
/// each past the first a path lookup where the last one ends.
const FORM_CHAIN: usize = 4;

/// Where [`model_root`] sets a letter: how far along the path past the
/// ink it is set after, the curve's point there, and the form it lies on
/// with how far on along it — what the next letter's form is taken from
/// ([`Form::moved`]).
struct Rooted<'p> {
    along: f32,
    on: FormPoint,
    form: Form<'p>,
    u: f32,
}

/// How [`solve_in`] ends on one form.
enum InForm {
    /// The root, `u` on, and the curve's point there.
    Found(f32, FormPoint),
    /// The root lies past where the form holds: the letter is still too
    /// near there.
    Beyond,
    /// No root the rounds close on, or a number on the way not finite.
    Failed,
}

/// The first point `u` on along `form` at which a letter is `owed` from
/// the ink before it, its gap at a point of the curve `gap_at` and the
/// way from that ink to it `delta_of`, their half sizes summing to
/// `reach` — as [`model_root`] reads it on one form: from the tangent
/// line's root at the form's start ([`line_root`]), exact on a side, each
/// round solves the tangent line at the point it reached — a Newton step —
/// held inside the bracket the points tried close, until the gap is what
/// is owed to `resolution`, or the step is one the form's measure cannot
/// take, or the bracket closes to neighboring floats of it: the root as
/// finely as the form holds it.
///
/// Cost: O(1): [`MODEL_ROUNDS`] rounds at most, no path lookup.
fn solve_in(
    form: &Form<'_>,
    gap_at: &dyn Fn(&FormPoint) -> f32,
    (delta_of, reach): (&dyn Fn(&FormPoint) -> Vec2, Vec2),
    owed: f32,
    resolution: f32,
) -> InForm {
    let near = |gap: f32| gap < owed;
    let done = |gap: f32| (gap - owed).abs() <= resolution && gap > 0.0;
    let most = form.most();
    let Some(mut on) = form.on(0.0) else {
        return InForm::Failed;
    };
    // The bracket the points tried close — too near, far enough — and its
    // ends' points and misses.
    let (mut inside, mut outside) = (0.0_f32, f32::INFINITY);
    let mut ends: [Option<(f32, FormPoint, f32)>; 2] = [None, None];
    let mut u = 0.0_f32;
    for _ in 0..=MODEL_ROUNDS {
        let step = line_root(delta_of(&on), on.velocity, reach, owed);
        if u > 0.0 && step.abs() <= form.quantum(u) {
            return InForm::Found(u, on);
        }
        let mut next = u + step;
        // A step out of the bracket, or none at all, halves it instead; and
        // with nothing yet far enough, takes the next point the float holds.
        if !(next > inside && next < outside) {
            next = if outside.is_finite() {
                inside.midpoint(outside)
            } else {
                inside.next_up()
            };
        }
        if next > most && outside > most {
            // Past where the form holds: still too near at its end, the root
            // is past it; else it lies before, which brackets it.
            let Some(end) = form.on(most) else {
                return InForm::Failed;
            };
            let gap = gap_at(&end);
            if near(gap) {
                return InForm::Beyond;
            }
            if done(gap) {
                return InForm::Found(most, end);
            }
            outside = most;
            ends[1] = Some((most, end, gap - owed));
            next = inside.midpoint(outside);
        }
        if !(next > inside && next < outside) {
            // Closed to neighboring floats of the form's measure: the end
            // nearer what is owed.
            return match ends {
                [Some(low), Some(high)] => {
                    let (u, on, _) = if low.2 <= high.2 { low } else { high };
                    InForm::Found(u, on)
                }
                _ => InForm::Failed,
            };
        }
        let Some(at) = form.on(next) else {
            return InForm::Failed;
        };
        (on, u) = (at, next);
        let gap = gap_at(&on);
        if done(gap) {
            return InForm::Found(u, on);
        }
        if near(gap) {
            inside = u;
            ends[0] = Some((u, on, owed - gap));
        } else if gap.is_finite() {
            outside = u;
            ends[1] = Some((u, on, gap - owed));
        } else {
            return InForm::Failed;
        }
    }
    InForm::Failed
}

/// How finely the band curve's forms measure a gap from the ink `last`
/// to a letter, their half sizes summing to `reach`, `owed` apart: their
/// arithmetic at points that far out carries the float error of a few
/// units of their last place. `None` where that is coarser than the inks
/// and their gap: the forms have nothing to tell them apart by.
fn resolution_at(last: &Set, reach: Vec2, owed: f32) -> Option<f32> {
    let scale = last.at.abs().max_element() + reach.max_element() + owed.max(0.0);
    let resolution = 4.0 * f32::EPSILON * scale;
    (resolution < reach.min_element() + owed.max(0.0)).then_some(resolution)
}

/// The gap [`model_root`] sets a letter at, `owed` from the ink before
/// it where its forms measure gaps to `resolution` ([`resolution_at`]):
/// what is owed, or where nothing is, the least gap a measurement there
/// tells from none.
fn model_aim(owed: f32, resolution: f32) -> f32 {
    if owed > 0.0 {
        owed
    } else {
        resolution
    }
}

/// Where along the band curve at `offset` to set a letter of half size
/// `half`, its center `rise` below its anchor, so that it is `owed` from
/// the ink `last` — the pairwise rule's root (ruling R31) on the curve's
/// forms, aimed at what is owed (ruling B16.2c) — read on `form`, the
/// curve's at `last`, and where the root lies past it, on the forms after
/// it, each read from `path` where the last ends, a lookup
/// ([`FORM_CHAIN`]). Boxes no gap apart may overlap: where nothing is
/// owed, the least gap a measurement there tells from none.
///
/// `None` where the letter is apart from `last` already at its center,
/// or no form read holds its root.
///
/// Cost: [`solve_in`]'s a form, and a path lookup each form past the
/// first.
fn model_root<'p>(
    path: &'p ArcPath,
    offset: f32,
    form: Form<'p>,
    last: &Set,
    (half, rise): (Vec2, f32),
    owed: f32,
) -> Option<Rooted<'p>> {
    let reach = last.half + half;
    let lift = Vec2::new(0.0, rise);
    let delta_of = |on: &FormPoint| on.point + lift - last.at;
    let gap_at = |on: &FormPoint| (delta_of(on).abs() - reach).max(Vec2::ZERO).length();
    let resolution = resolution_at(last, reach, owed)?;
    let owed = model_aim(owed, resolution);
    if !matches!(gap_at(&form.on(0.0)?).partial_cmp(&owed), Some(Ordering::Less)) {
        return None;
    }
    let (mut form, mut before) = (form, 0.0_f32);
    for _ in 0..FORM_CHAIN {
        match solve_in(&form, &gap_at, (&delta_of, reach), owed, resolution) {
            InForm::Found(u, on) => {
                let along = before + form.along(u)?;
                return along.is_finite().then_some(Rooted { along, on, form, u });
            }
            InForm::Beyond => {
                // The curve read where the form ends, its next form from there.
                let ends = form.along(form.most())?;
                let s = last.center + before + ends;
                let (at, place) = path.offset_in(s, offset);
                let next = Form::of(
                    path,
                    &Set {
                        at: at.point,
                        half: Vec2::ZERO,
                        on: at.point,
                        bend: Bending::Read(place),
                        center: s,
                        moves: at.moves,
                        normal: at.normal,
                        reach: Reach::default(),
                    },
                    None,
                    offset,
                )?;
                // A form that holds no further is no way on.
                if !next.along(next.most()).is_some_and(|on| on > 0.0) {
                    return None;
                }
                (form, before) = (next, before + ends);
            }
            InForm::Failed => return None,
        }
    }
    None
}

/// Where along the path to set a letter of half size `half`, its center
/// `rise` below its anchor, so that it is `owed` from the ink `last`, an
/// ink `earlier` set before it, every ink anchored on the band curve at
/// `offset`: its root on the curve's form at `last` ([`Form`],
/// [`model_root`]; #309, ruling B16.1b). On a side, an arc both inks are
/// set on, or a sharp corner's stretch outward, the form is the curve,
/// and the root is the rule's, exact to float error; round an ellipse,
/// to its table's chords. `None` where there is no form, or no root
/// within it: [`InkLay::cluster`]'s straight warm start then.
///
/// Cost: [`model_root`]'s, no path lookup.
fn osculating(
    path: &ArcPath,
    last: &Set,
    earlier: Option<&Set>,
    letter: (Vec2, f32),
    owed: f32,
    offset: f32,
) -> Option<f32> {
    if !layer_on() {
        return None;
    }
    let form = Form::of(path, last, earlier, offset)?;
    // On a straight stretch, where the center's floats are coarser than
    // the rule's window, the root is no better a start than the last
    // letter's step, whose share of the straight path follows the floats
    // the line is set on.
    if form.straight() && ulp(last.center) >= INK_TOLERANCE + NEARER {
        return None;
    }
    let rooted = model_root(path, offset, form, last, letter, owed)?;
    let at = last.center + rooted.along;
    at.is_finite().then_some(at)
}

/// The letter [`osculating`] places, set where its form puts it rather
/// than read from the path (#309, ruling B16.2d): what a side arc's
/// model lay sets ([`InkLay::modeled`]) to solve its gap before a lay is
/// read from the path at it. Its bend is the form's on from there, so
/// that the next letter's root is read from it in turn; none where the
/// form ends there, so that the next letter is read from the path.
///
/// Cost: [`model_root`]'s, no path lookup.
fn model_set(
    path: &ArcPath,
    last: &Set,
    earlier: Option<&Set>,
    (half, rise): (Vec2, f32),
    owed: f32,
    offset: f32,
) -> Option<Set> {
    let form = Form::of(path, last, earlier, offset)?;
    let Rooted { along, on, form, u } = model_root(path, offset, form, last, (half, rise), owed)?;
    let center = last.center + along;
    let bend = form.moved(u, &on).map(|moved| match moved {
        Form::Circle {
            kappa, speed, most, ..
        } => Bend {
            curvature: Some(kappa),
            speed,
            holds: most,
            corner: None,
            ellipse: None,
        },
        Form::Corner { form, most, .. } => Bend {
            curvature: None,
            speed: 1.0,
            holds: most,
            corner: Some(form),
            ellipse: None,
        },
        Form::Ellipse { form, table } => Bend {
            curvature: None,
            speed: 1.0,
            holds: table.1.last().map_or(0.0, |end| end - form.local()),
            corner: None,
            ellipse: Some(form),
        },
    });
    center.is_finite().then_some(Set {
        at: on.point + Vec2::new(0.0, rise),
        half,
        on: on.point,
        bend: Bending::Formed(bend),
        center,
        moves: on.moves,
        normal: on.normal,
        reach: Reach::of(rise, half, on.normal),
    })
}

/// One line of the frame before it is laid: the half it takes, its
/// glyphs and what tracking it needs of them.
#[derive(Clone)]
struct Prepared {
    /// The half it takes: 0 the upper-right, 1 the lower-left.
    half: usize,
    /// Its clusters, in text order.
    clusters: Vec<Cluster>,
    /// Its letters' mean ink half size.
    size: Vec2,
    /// Its font's own gap: its letters' mean side bearings.
    natural_gap: f32,
    /// Its widest space's advance.
    space: f32,
    /// Its natural ends along its half ([`LineSpan::NATURAL`]): its first
    /// and last ink centers at its font's own gap, centered on its half's
    /// apex — on the band curve at the offset they were centered at, the
    /// first of the three, and centered again on the curve at another
    /// ([`Layout::natural`], #308).
    natural: Cell<(f32, f32, f32)>,
    /// Whether R27 cut its end.
    cut: bool,
}

/// One line along its half ([`Layout::lay`]).
#[derive(Clone)]
struct Line {
    /// The half it takes.
    half: usize,
    /// Its glyphs, as [`track`] laid them at its gap.
    tracked: Tracked,
    /// Its tracking gap.
    gap: f32,
    /// Where its tracking solve stopped short of its end, the gap past
    /// `gap` at which it ends past it ([`solve_tracking`]): what its
    /// secondary rule finds the step between them by
    /// ([`Layout::reach_its_end`], #309).
    past: Option<f32>,
    /// Its word gap, a space's advance and two gaps, its words' stretch
    /// shared among its spaces ([`Line::finish`]): `None` for a line
    /// with no space, which has no word gap.
    word: Option<f32>,
    /// The part of its half it covers, and the room it was laid in
    /// ([`Layout::room`]).
    part: (f32, f32),
    room: (f32, f32),
    /// Its letters' mean ink half size.
    size: Vec2,
    /// Whether its half less the clearance was too short to hold it at
    /// its font's own gap, so it was laid tighter.
    pressed: bool,
    /// Whether it has two letters or more, and so a gap to track: a line
    /// of one sits centered on its part, which its clearance moves
    /// nowhere in particular.
    tracks: bool,
    /// How many gaps it has ([`solve_tracking`]'s `steps`).
    steps: usize,
    /// The outward normal of the outline at its first and last inks, as
    /// they lie ([`Layout::normals`]): what the 135° test weighs.
    normals: [Vec2; 2],
    /// Which way its ends move as its clearance grows at the top-left
    /// and bottom-right junctions: on along its half at its start, back
    /// at its end — along the path before its end, which at a sharp
    /// corner is not the way the path runs on from it.
    retreats: [Vec2; 2],
    /// How much more clearance its part can give before it is shorter
    /// than the line at its font's own gap, by arc length, never
    /// negative: the room its part leaves, and its gaps past its font's
    /// own. A line of one letter, which nothing presses, has no end of
    /// it.
    slack: f32,
}

impl Line {
    /// Whether it ends short of its part's end by more than its tolerance
    /// ([`line_tolerance`]), a line with a gap to track that its part does
    /// not press: what its secondary rule brings home, or the frame's
    /// trace reports ([`Trace::short`], #309).
    fn short_of_its_end(&self) -> bool {
        self.tracks && !self.pressed && self.tracked.short > line_tolerance(self.steps, self.part.1)
    }

    /// The line its secondary rule brought to its end, `reached`, in
    /// place of its lay ([`Layout::reach_its_end`], #309): its gap and lay, its
    /// word gap its spaces' mean — the stretch of those past its step
    /// shared among them all — and its slack with what its words hold.
    /// Its normals are its layout's to find.
    fn finish(&mut self, p: &Prepared, reached: Reached) {
        let spaces = p.clusters.iter().filter(|c| c.blank).count();
        let stretched = p.clusters[reached.words.from..]
            .iter()
            .filter(|c| c.blank)
            .count();
        let words = if spaces > 0 {
            reached.words.by * stretched as f32
        } else {
            0.0
        };
        self.gap = reached.gap;
        self.word = (p.space > 0.0).then(|| p.space + 2.0 * reached.gap + words / spaces.max(1) as f32);
        self.slack = ((self.room.1 - self.room.0) - (self.part.1 - self.part.0)
            + self.steps as f32 * (reached.gap - p.natural_gap)
            + words)
            .max(0.0);
        self.tracked = reached.tracked;
        self.past = None;
    }
}

/// A frame's lines being laid along its halves.
struct Layout<'a> {
    halves: &'a Halves,
    outward: bool,
    /// The work the frame has taken, and the most it may take.
    work: Work,
    /// The offset off the outline of the frame's one band curve, which
    /// every ink of it is anchored on (#308, ruling B15.1): as last
    /// solved ([`Layout::lay_clear`]), what every lay is laid at.
    offset: Cell<f32>,
    /// Each span's band center a point of its size ([`Band::centers`],
    /// [`inked_with_bands`]), as the text's last build read them.
    centers: std::cell::RefCell<Rc<[f32]>>,
    /// What the frame's side ornament wants of its offset, as its last
    /// fill laid it ([`SideWant`]): what each solve of the offset holds
    /// it to, along with the lines' own inks. `None` before a fill, or
    /// without sides.
    sides: std::cell::RefCell<Option<SideWant>>,
    /// The lines its last few [`track`]s laid ([`TrackMemo`]).
    tracked: std::cell::RefCell<TrackMemo>,
}

/// A frame's work (rulings R44.2, R46.1): the glyph steps all its work
/// has taken — its tracking, its side lays, its shaping and inking, its
/// looks for a touch, R27's lays — the glyphs of its text its budget is
/// scaled by, the most it may take ([`frame_budget`]), and what of that
/// is held back for [`untouched`].
pub(super) struct Work {
    spent: Cell<usize>,
    /// The samples its paths' tables were built with, counted in `spent`
    /// ([`tabled`](Self::tabled)).
    tables: Cell<usize>,
    glyphs: Cell<usize>,
    budget: Cell<usize>,
    reserve: Cell<usize>,
    /// The classes of height its text's inks hold ([`height_classes`]),
    /// what [`untouched`]'s share is held back for: one until the text is
    /// inked.
    classes: Cell<usize>,
    /// The most pairs its touch checks looked at a box (rulings R48.2,
    /// R51.2).
    looks: Cell<Looks>,
    /// The classes of height the last touch check held.
    last_classes: Cell<usize>,
    /// Whether [`untouched`] is running, within the share held back for
    /// it: its steps are not past the budget ([`PAST_BUDGET`]).
    #[cfg(test)]
    reserved: Cell<bool>,
}

impl Work {
    /// A frame's work, nothing taken yet, its budget scaled by `glyphs`.
    pub(super) fn new(glyphs: usize) -> Self {
        let work = Work {
            spent: Cell::new(FRAME_STEPS),
            tables: Cell::new(0),
            glyphs: Cell::new(0),
            budget: Cell::new(TRACK_BUDGET),
            reserve: Cell::new(0),
            classes: Cell::new(1),
            looks: Cell::new(Looks::default()),
            last_classes: Cell::new(0),
            #[cfg(test)]
            reserved: Cell::new(false),
        };
        work.scale(glyphs);
        work
    }

    /// Count `steps` glyph steps taken.
    pub(super) fn take(&self, steps: usize) {
        // The part of the take past the budget less what is held back for
        // `untouched`, where it reads as spent: all of it once spent, and
        // of a take that crosses it, what lies past (ruling R50.4).
        #[cfg(test)]
        if !self.reserved.get() {
            let room = self.budget.get().saturating_sub(self.reserve.get());
            let past = self
                .spent
                .get()
                .saturating_add(steps)
                .saturating_sub(room.max(self.spent.get()));
            PAST_BUDGET.with(|most| most.set(most.get().saturating_add(past)));
        }
        self.spent.set(self.spent.get().saturating_add(steps));
    }

    /// Count shaping `run` (ruling R46.1).
    pub(super) fn shaped(&self, run: &ShapedRun) {
        self.take(shaping_steps(run));
    }

    /// Count inking `run` (ruling R46.1).
    fn inked(&self, run: &ShapedRun) {
        self.take(inking_steps(run));
    }

    /// Count building the frame's `paths` (ruling B16.3a): their tables'
    /// samples ([`table_steps`]).
    pub(super) fn tabled<'p>(&self, paths: impl IntoIterator<Item = &'p ArcPath>) {
        let samples: usize = paths.into_iter().map(ArcPath::table_samples).sum();
        self.tables.set(self.tables.get().saturating_add(samples));
        self.take(table_steps(samples));
    }

    /// Note a touch check of `boxes` boxes that looked at the pairs
    /// `looked` holds.
    fn looked(&self, looked: &Looked, boxes: usize) {
        self.looks.set(self.looks.get().more(looked, boxes));
        self.last_classes.set(looked.classes);
    }

    /// Whether the frame can still take `steps` glyph steps within its
    /// budget, less what is held back for [`untouched`] (ruling R48.2).
    fn affords(&self, steps: usize) -> bool {
        self.spent
            .get()
            .saturating_add(self.reserve.get())
            .saturating_add(steps)
            <= self.budget.get()
    }

    /// Count walking `clusters` clusters in R27's arithmetic.
    pub(super) fn walked(&self, clusters: usize) {
        self.take(clusters.div_ceil(CLUSTERS_A_STEP).max(1));
    }

    /// Whether the frame has taken its budget, less what is held back
    /// for [`untouched`], which runs within it (ruling R46.1).
    pub(super) fn spent_all(&self) -> bool {
        self.spent.get().saturating_add(self.reserve.get()) >= self.budget.get()
    }

    /// Scale the budget by the frame's text, `glyphs` glyphs once cut to
    /// the glyph cap (ruling R44.2): its sides' lays count against it,
    /// and scale it not (ruling R44.3), so a frame's work is bounded by
    /// its text alone. As much of it as [`untouched`] may take over that
    /// many inks is held back for it.
    fn scale(&self, glyphs: usize) {
        self.glyphs.set(glyphs);
        self.budget.set(frame_budget(glyphs));
        self.reserve.set(untouched_steps(glyphs, self.classes.get()));
    }

    /// Hold back [`untouched`]'s share for a text whose inks hold
    /// `classes` classes of height ([`height_classes`]), one at the least:
    /// read from each build's inks, before any of its lines is laid, so
    /// that every check `untouched` makes — of a part of those inks —
    /// holds no more (ruling R46.1, #308).
    fn hold_for(&self, classes: usize) {
        self.classes.set(classes.max(1));
        self.reserve
            .set(untouched_steps(self.glyphs.get(), self.classes.get()));
    }

    /// The share of the budget held back for [`untouched`].
    pub(super) fn reserve(&self) -> usize {
        self.reserve.get()
    }

    /// The samples of the tables counted so far ([`tabled`](Self::tabled)).
    pub(super) fn tables(&self) -> usize {
        self.tables.get()
    }
}

/// The lines a frame's last few [`track`]s laid, by what each was laid
/// from (#309), so a line laid again as it was just laid is read, not
/// laid: a slide measures the line its centering last measured, and the
/// line from its room's start it measured to find it longer than its
/// room. A line is a function of what this keys and of the build's run and
/// band centers alone, and the frame forgets every line at each build.
/// The most recent [`TRACK_MEMO`] are kept, so a long text's lines are
/// not all held at once. A line read so counts the work it is — a search
/// of those kept and a copy, [`TRACK_CALL_STEPS`] — not a track's (ruling
/// B16.1c).
#[derive(Default)]
struct TrackMemo {
    kept: Vec<(TrackKey, Tracked)>,
    /// Where the next line is kept once [`TRACK_MEMO`] are.
    next: usize,
}

/// How many lines a frame's [`TrackMemo`] keeps.
const TRACK_MEMO: usize = 8;

/// What a line [`track`] lays is known by ([`TrackMemo`]): its half, the
/// first glyph of its first cluster and the end of its last, its clusters'
/// count — a run of a build's clusters, which are each its glyphs' — its
/// part's ends, its gap and the frame's offset, each bit for bit.
type TrackKey = (usize, usize, usize, usize, u32, u32, u32, u32);

impl TrackMemo {
    /// The line laid from `key`, if it is one of those kept.
    ///
    /// Cost: O([`TRACK_MEMO`]).
    fn get(&self, key: &TrackKey) -> Option<&Tracked> {
        #[cfg(test)]
        if NO_TRACK_MEMO.with(Cell::get) {
            return None;
        }
        self.kept.iter().find(|(k, _)| k == key).map(|(_, t)| t)
    }

    /// Keep `tracked`, laid from `key`, in place of the oldest kept.
    ///
    /// Cost: a clone of the line.
    fn put(&mut self, key: TrackKey, tracked: &Tracked) {
        if self.kept.len() < TRACK_MEMO {
            self.kept.push((key, tracked.clone()));
        } else {
            self.kept[self.next] = (key, tracked.clone());
            self.next = (self.next + 1) % TRACK_MEMO;
        }
    }

    /// Forget every line: the build's run and band centers have changed.
    fn clear(&mut self) {
        self.kept.clear();
        self.next = 0;
    }
}

/// The lines [`Layout::lay`] has laid in one clearance search.
///
/// Both are short lists searched in order, not hashed (#309): a search
/// lays each line a few dozen times at most — [`CLEAR_ROUNDS`] rounds,
/// each with its backtracks, at two spans — and keeps one warm start for
/// each line at each span.
#[derive(Default)]
struct LayMemo {
    /// By line, part of its half it may cover, span and the frame's
    /// offset (#308), all bit for bit: a round that grows one line's
    /// clearance lays the other where it lay.
    laid: Vec<(LayKey, Line)>,
    /// Each line's last lay at a span, by line and span's bits: its
    /// part's length and its gap, what its next lay's tracking is
    /// warm-started from ([`solve_tracking`], ruling R43) — a round moves
    /// a line's part a little, and its gap with it.
    warm: Vec<((usize, u32), (f32, f32))>,
}

/// What a line laid in a clearance search is known by ([`LayMemo`]): the
/// line, its room's ends, the span and the frame's offset, each bit for
/// bit.
type LayKey = (usize, u32, u32, u32, u32);

impl LayMemo {
    /// The line laid at `key`, if one has been.
    ///
    /// Cost: O(lines laid in the search).
    fn laid(&self, key: &LayKey) -> Option<&Line> {
        self.laid.iter().find(|(k, _)| k == key).map(|(_, line)| line)
    }

    /// Line `i`'s last lay at the span of bits `span`: its part's length
    /// and its gap.
    ///
    /// Cost: O(lines and spans warmed).
    fn warm(&self, i: usize, span: u32) -> Option<(f32, f32)> {
        self.warm.iter().find(|(k, _)| *k == (i, span)).map(|(_, w)| *w)
    }

    /// Remember line `i`'s lay at the span of bits `span` as its warm
    /// start.
    fn warm_with(&mut self, i: usize, span: u32, with: (f32, f32)) {
        match self.warm.iter_mut().find(|(k, _)| *k == (i, span)) {
            Some((_, was)) => *was = with,
            None => self.warm.push(((i, span), with)),
        }
    }
}

/// One clearance search's state ([`Layout::lay_clear`]): the lines it
/// has laid, and how many times a round that would press a line is
/// halved in it.
struct Search {
    memo: LayMemo,
    backtracks: usize,
}

/// What a frame's side ornament wants of the frame's offset (#308): the
/// most its copies, as its last fill laid them, reach toward the outline
/// and away from it ([`Reach`]). A copy's reach is its ink's support
/// along the outline's normal under it, which an offset moves only
/// round a sharp corner outward, where the curve sweeps.
#[derive(Copy, Clone, Debug)]
struct SideWant {
    toward: f32,
    away: f32,
}

impl SideWant {
    /// What it and `other` want together: the most of each.
    fn with(self, other: SideWant) -> SideWant {
        SideWant {
            toward: self.toward.max(other.toward),
            away: self.away.max(other.away),
        }
    }
}

impl<'a> Layout<'a> {
    /// A frame's layout round `halves`, its band curve not yet solved
    /// and anchored by each span's band center `centers`
    /// ([`inked_with_bands`]).
    fn new(halves: &'a Halves, outward: bool, work: Work, centers: Vec<f32>) -> Self {
        Layout {
            halves,
            outward,
            work,
            offset: Cell::new(0.0),
            centers: std::cell::RefCell::new(centers.into()),
            sides: std::cell::RefCell::new(None),
            tracked: std::cell::RefCell::new(TrackMemo::default()),
        }
    }
}

impl Layout<'_> {
    /// The frame's band curve as it stands ([`Layout::offset`]).
    fn band(&self) -> Band {
        Band {
            offset: self.offset.get(),
            centers: self.centers.borrow().clone(),
        }
    }

    /// [`track`] on the frame's band curve, counted against the frame's
    /// budget ([`Work`]), a line it laid just before read from its
    /// [`TrackMemo`].
    fn track(
        &self,
        walk: &PathWalk<'_>,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        part: (f32, f32),
        g: f32,
    ) -> Tracked {
        let key = (
            usize::from(std::ptr::eq(walk.path, self.halves.path(1))),
            clusters.first().map_or(0, |c| c.glyphs.start),
            clusters.last().map_or(0, |c| c.glyphs.end),
            clusters.len(),
            part.0.to_bits(),
            part.1.to_bits(),
            g.to_bits(),
            self.offset.get().to_bits(),
        );
        if let Some(tracked) = self.tracked.borrow().get(&key) {
            #[cfg(test)]
            TRACK_HITS.with(|count| count.set(count.get() + 1));
            // A line read, not laid: its search and its copy, the work of a
            // track's call past its clusters (#309, ruling B16.1c).
            self.work.take(TRACK_CALL_STEPS);
            return tracked.clone();
        }
        self.work.take(clusters.len() + TRACK_CALL_STEPS);
        let tracked = track(walk, glyphs, clusters, part, g, self.band());
        self.tracked.borrow_mut().put(key, &tracked);
        tracked
    }

    /// [`track_words`] on the frame's band curve, counted against the
    /// frame's budget ([`Work`]): a line's secondary rule's lay (#309,
    /// [`reach_its_end`](Self::reach_its_end)). Never read from or kept in
    /// the frame's [`TrackMemo`], which holds the lines its searches lay:
    /// the rule lays each stretch once, and a stretch kept there would put
    /// out a line a search lays again.
    fn track_words(
        &self,
        walk: &PathWalk<'_>,
        clusters: &[Cluster],
        run: &ShapedRun,
        part: (f32, f32),
        (g, words): (f32, Words),
    ) -> Tracked {
        self.work.take(clusters.len() + TRACK_CALL_STEPS);
        track_words(walk, &run.glyphs, clusters, part, g, words, self.band())
    }

    /// Whether the frame has taken its budget ([`Work`]).
    fn spent_all(&self) -> bool {
        self.work.spent_all()
    }

    /// The walk a line on half `k` is tracked along.
    fn walk<'s>(&'s self, k: usize, run: &ShapedRun) -> PathWalk<'s> {
        line_walk(self.halves.path(k), run, self.outward)
    }

    /// How long `clusters` are at gap `g` from their first ink center at
    /// `from` to their last, by arc length along half `k`, on the frame's
    /// band curve.
    fn reach(&self, k: usize, run: &ShapedRun, clusters: &[Cluster], from: f32, g: f32) -> f32 {
        -self
            .track(&self.walk(k, run), &run.glyphs, clusters, (from, from), g)
            .short
    }

    /// The offset the frame's band curve is first laid at (#308): the one
    /// the inks of `lines` (`run`'s clusters, each on its half) would want
    /// were each set at its half's apex, where R27 centers its line
    /// ([`offset_from`]), before any is laid — what the frame's offset is
    /// solved from ([`Layout::lay_clear`]). None with no ink.
    ///
    /// Cost: a path lookup a line, and O(clusters).
    fn estimate<'c>(&self, run: &ShapedRun, lines: impl IntoIterator<Item = (usize, &'c [Cluster])>) -> f32 {
        let band = self.band();
        let mut reaches = Vec::new();
        for (k, clusters) in lines {
            let walk = self.walk(k, run);
            let normal = walk.path.offset_at(self.halves.apexes[k], 0.0).normal;
            reaches.extend(
                clusters
                    .iter()
                    .filter(|c| !c.blank)
                    .filter_map(|c| match walk.body_of(c) {
                        Body::Inked { ink, .. } => {
                            let rise =
                                0.5 * (ink.min.y + ink.max.y) - band.anchor(&run.glyphs[c.glyphs.start], ink);
                            Some(Reach::of(rise, (ink.max - ink.min) * 0.5, normal))
                        }
                        Body::Boxed { .. } => None,
                    }),
            );
        }
        self.work.walked(reaches.len());
        let offset = offset_from(&reaches, self.outward, None);
        if offset.is_finite() {
            offset
        } else {
            0.0
        }
    }

    /// The offset the frame's inks want, laid as `lines` lie (#308,
    /// ruling B15.1): outward, the least at which every ink of its lines
    /// and its side ornament clears the outline, the frame's gap from what
    /// it frames; inward, its ink band centered on the outline
    /// ([`offset_from`]). The side ornament's share is its last fill's
    /// ([`Layout::sides`]). Not a number with no ink.
    ///
    /// Cost: O(inks), a fold over the reaches their lay found, with no
    /// path lookup: weighed as a touch check weighs its boxes
    /// ([`weighing_steps`]), whose sort and sweep take more a box.
    fn wants(&self, lines: &[Line]) -> f32 {
        let side = *self.sides.borrow();
        let reaches: Vec<Reach> = lines
            .iter()
            .flat_map(|l| l.tracked.reaches.iter().copied())
            .collect();
        self.work.take(weighing_steps(reaches.len()));
        offset_from(&reaches, self.outward, side)
    }

    /// The line of `run` (inked) of `clusters` on half `k`, ready to lay:
    /// `None` when it holds no glyph that is not a space. Its natural
    /// ends are centered on the apex on the frame's band curve as it
    /// stands ([`Layout::centered`]).
    ///
    /// Cost: one [`track`] at the apex and [`CENTER_ROUNDS`] at most, one
    /// on a straight path.
    fn prepare_clusters(
        &self,
        run: &ShapedRun,
        k: usize,
        clusters: Vec<Cluster>,
        cut: bool,
    ) -> Option<Prepared> {
        let (size, natural_gap) = letters_of(&clusters)?;
        let space = clusters
            .iter()
            .filter(|c| c.blank)
            .map(|c| c.advance)
            .fold(0.0_f32, f32::max);
        let (na, long) = self.centered(run, k, &clusters, natural_gap, None);
        Some(Prepared {
            half: k,
            natural: Cell::new((self.offset.get(), na, na + long)),
            clusters,
            size,
            natural_gap,
            space,
            cut,
        })
    }

    /// Line `p`'s natural ends on the frame's band curve as it stands
    /// ([`Prepared::natural`]): where it was centered, or centered again
    /// at the offset the frame's curve has moved to since (#308) — a
    /// curve further off the outline is longer, by how far it moves times
    /// how far the half turns under the line.
    ///
    /// Cost: none at the offset it was centered at; otherwise two
    /// [`track`]s where the line centered at its last offset is near
    /// centered still, [`CENTER_ROUNDS`] at most.
    fn natural(&self, run: &ShapedRun, p: &Prepared) -> (f32, f32) {
        let offset = self.offset.get();
        let (at, na, nb) = p.natural.get();
        if at.to_bits() == offset.to_bits() {
            return (na, nb);
        }
        let (na, long) = self.centered(run, p.half, &p.clusters, p.natural_gap, Some(na));
        p.natural.set((offset, na, na + long));
        (na, na + long)
    }

    /// Where `clusters` start on half `k` at gap `g`, centered on its
    /// apex on the frame's band curve, and how long they are
    /// ([`falsi`]): the arc from the first ink center to the apex equals
    /// the arc from the apex to the last, to [`CENTER_TOLERANCE`].
    /// `from`, where they started centered on the curve at another
    /// offset, warm-starts it.
    ///
    /// Cost: one [`track`] at the apex and [`CENTER_ROUNDS`] at most, one
    /// on a straight path; from `from`, one where it is centered still,
    /// two where the line is near straight, [`CENTER_ROUNDS`] at most.
    fn centered(
        &self,
        run: &ShapedRun,
        k: usize,
        clusters: &[Cluster],
        g: f32,
        from: Option<f32>,
    ) -> (f32, f32) {
        let apex = self.halves.apexes[k];
        let walk = self.walk(k, run);
        let reach = |from: f32| -self.track(&walk, &run.glyphs, clusters, (from, from), g).short;
        // Off-center by `before - after`: positive while the start is too
        // far back. It falls as the start moves on, by about twice as
        // much, the line's length changing only with the bend of the path
        // under it — so the line through the bracket's ends, at the apex
        // less the whole line and at the apex, crosses zero at the
        // straight path's answer, half the line back, and that is the
        // first probe. A miss that is not a number reads as too far back:
        // the start moves on, toward the apex, where the line is no
        // longer than it was.
        let miss = |na: f32| {
            let long = reach(na);
            let m = (apex - na) - (na + long - apex);
            (if m.is_nan() { f32::INFINITY } else { m }, long)
        };
        let tolerance = resolved(CENTER_TOLERANCE, apex);
        let stop = || self.spent_all();
        // From where it was centered: a start moved by half its miss,
        // the straight path's answer, and the two bracketing the root.
        if let Some(was) = from.filter(|was| was.is_finite()) {
            let (m0, l0) = miss(was);
            if m0.abs() <= tolerance {
                return (was, l0);
            }
            let next = was + 0.5 * m0;
            if next.is_finite() {
                let (m1, l1) = miss(next);
                if m1.abs() <= tolerance {
                    return (next, l1);
                }
                let (a, b) = if was < next {
                    ((was, m0, l0), (next, m1, l1))
                } else {
                    ((next, m1, l1), (was, m0, l0))
                };
                if a.1 > 0.0 && b.1 <= 0.0 {
                    return falsi(miss, a, b, tolerance, tolerance, CENTER_ROUNDS, &stop);
                }
            }
        }
        let whole = reach(apex).max(0.0);
        falsi(
            miss,
            (apex - whole, whole, whole),
            (apex, -whole, whole),
            tolerance,
            tolerance,
            CENTER_ROUNDS,
            &stop,
        )
    }

    /// The part of half `half` a line may cover with `clear` kept at
    /// each junction (top-left, bottom-right): `(lo, hi)`, never
    /// inverted and never off the half. Clearances that together pass
    /// the whole half meet where they divide it in their ratio
    /// ([`share`]): the line is then a point, pressed.
    fn room(&self, half: usize, clear: [f32; 2]) -> (f32, f32) {
        let (ja, jb) = self.halves.junctions[half];
        let (lo, hi) = (ja + clear[0], jb - clear[1]);
        if lo <= hi {
            (lo, hi)
        } else {
            let mid = ja + (jb - ja) * share(clear[0], clear[1]);
            (mid, mid)
        }
    }

    /// The natural line `p` slid clear of the clearance, at its font's
    /// own gap, within `(lo, hi)` when that holds it: slid on from the
    /// top-left clearance, slid back from the bottom-right one. A line
    /// longer than `hi − lo` at its gap starts at `lo` and ends at `hi`.
    /// It is measured on the frame's band curve as it stands.
    ///
    /// **A step in the line's end** (#308, ruling B15.3). Round a tight
    /// bend a letter crossing it moves the line's end by a jump as the
    /// start moves by an ulp, and no start ends the line within the
    /// tolerance of `hi`: the secant then circles the jump in starts a
    /// few ulps apart. Once the starts tried bracket `hi` — one whose line
    /// ends short of it, one whose ends past — within [`SLIDE_ULPS`] ulps
    /// of the start, and the end moves across them by more than a line
    /// that moves with its start nearly one for one could (twice the
    /// bracket and the tolerance), the slide stops at the start whose line
    /// ends short of `hi`: within its room.
    fn slid(&self, run: &ShapedRun, p: &Prepared, (lo, hi): (f32, f32)) -> (f32, f32) {
        let reach = |from: f32| {
            #[cfg(test)]
            SLIDE_TRACKS.with(|count| count.set(count.get() + 1));
            self.reach(p.half, run, &p.clusters, from, p.natural_gap)
        };
        let (mut na, mut nb) = self.natural(run, p);
        if na < lo {
            na = lo;
            nb = lo + reach(lo);
        }
        if nb > hi {
            // The start whose line ends at `hi`: where the miss, the
            // line's end past `hi`, is zero. It moves with the start
            // nearly one for one, bent only by the path under the line,
            // so each probe is where the line through the last two
            // crosses zero (ruling R38.3) — the first, the start the
            // line's length from `hi` gives.
            let mut long = reach(na);
            let mut last: Option<(f32, f32)> = None;
            let mut next = hi - long;
            let tolerance = resolved(SLIDE_TOLERANCE, hi);
            // The nearest starts tried whose lines end short of `hi` and
            // past it: each its start, its line's length and its miss.
            let mut short: Option<(f32, f32, f32)> = None;
            let mut past: Option<(f32, f32, f32)> = None;
            for _ in 0..SLIDE_ROUNDS {
                if self.spent_all() {
                    break;
                }
                na = next;
                long = reach(na);
                let miss = na + long - hi;
                if miss.abs() <= tolerance {
                    break;
                }
                if miss < 0.0 {
                    if short.is_none_or(|(at, _, _)| na > at) {
                        short = Some((na, long, miss));
                    }
                } else if miss > 0.0 && past.is_none_or(|(at, _, _)| na < at) {
                    past = Some((na, long, miss));
                }
                if let (Some((a, la, ma)), Some((b, _, mb))) = (short, past) {
                    if steps_between((a, ma), (b, mb), tolerance) {
                        (na, long) = (a, la);
                        break;
                    }
                }
                let secant = last.map(|(at, was)| na - miss * (na - at) / (miss - was));
                next = match secant {
                    Some(x) if x.is_finite() => x,
                    _ => hi - long,
                };
                if next == na {
                    break;
                }
                // A start before `lo`: the line from `lo` ends past `hi`
                // too, it is longer than the room and starts at `lo` —
                // where a slide that chased the start below it, round a
                // sharp corner whose letters move by a jump as the start
                // moves, once took all its rounds (#308).
                if next < lo {
                    let from_lo = reach(lo);
                    if lo + from_lo >= hi {
                        (na, long) = (lo, from_lo);
                        break;
                    }
                }
                last = Some((na, miss));
            }
            if na < lo {
                na = lo;
                long = reach(lo);
            }
            nb = (na + long).min(hi);
        }
        (na, nb)
    }

    /// The lines of `run` (inked), each moved by `span` from its natural
    /// place (slid clear of the clearance, [`slid`](Self::slid)) to its
    /// whole half less its `clear` (top-left, bottom-right), by arc length at
    /// each end (ruling R33), and laid at the gap that centers its end
    /// inks on its part's ends, every ink anchored on the frame's band
    /// curve as it stands (#308).
    ///
    /// A line laid before in this search with the same part, span and
    /// offset is read from `memo`: the line laid then. A line laid afresh has
    /// its tracking warm-started from its last lay at the span.
    fn lay(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        clear: Clear,
        span: f32,
        memo: &mut LayMemo,
    ) -> Vec<Line> {
        lines
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let room = self.room(p.half, clear[p.half]);
                let key = (
                    i,
                    room.0.to_bits(),
                    room.1.to_bits(),
                    span.to_bits(),
                    self.offset.get().to_bits(),
                );
                if let Some(line) = memo.laid(&key) {
                    return line.clone();
                }
                let warm = memo.warm(i, span.to_bits());
                let line = self.lay_one(run, p, room, span, warm);
                if line.tracks && !line.pressed {
                    memo.warm_with(i, span.to_bits(), (line.part.1 - line.part.0, line.gap));
                }
                memo.laid.push((key, line.clone()));
                line
            })
            .collect()
    }

    /// [`lay`](Self::lay)'s line `p` over `(lo, hi)`, its tracking
    /// warm-started from `warm` — the part's length and the gap of the
    /// line's last lay at this span — when there is one.
    fn lay_one(
        &self,
        run: &ShapedRun,
        p: &Prepared,
        (lo, hi): (f32, f32),
        span: f32,
        warm: Option<(f32, f32)>,
    ) -> Line {
        let walk = self.walk(p.half, run);
        let (na, nb) = self.slid(run, p, (lo, hi));
        // `lo ≤ na` and `nb ≤ hi` after the slide, so each end
        // moves only outward as the span grows.
        let sa = na + (lo - na) * span;
        let sb = nb + (hi - nb) * span;
        // A line of one letter has no gap to track: it sits at
        // its font's gap, centered on its part.
        let letters = p.clusters.iter().filter(|c| !c.blank).count();
        let steps = letters.saturating_sub(1) + p.clusters.iter().filter(|c| c.blank).count();
        let (gap, tracked, past) = if letters < 2 {
            let mid = sa.midpoint(sb);
            let gap = p.natural_gap;
            (
                gap,
                self.track(&walk, &run.glyphs, &p.clusters, (mid, mid), gap),
                None,
            )
        } else {
            let (gap, tracked, past) = solve_tracking(
                |g| self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), g),
                steps,
                sb,
                // The last gap moved by the part's change in length,
                // shared among the gaps, as on a straight path.
                warm.map(|(long, gap)| gap + ((sb - sa) - long) / steps.max(1) as f32),
                &|| self.spent_all(),
            );
            if gap < p.natural_gap - GAP_TOLERANCE {
                // A part its line fits at its font's gap is not one that
                // presses it, where the slide that set it fell short of
                // its end round a bend (ruling R35.2), or the solve came
                // to rest below the font's gap by no more than the line's
                // precision: it is laid at the font's gap from its start.
                let natural = self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), p.natural_gap);
                if natural.short >= -line_tolerance(steps, sb) {
                    (p.natural_gap, natural, None)
                } else {
                    (gap, tracked, past)
                }
            } else {
                (gap, tracked, past)
            }
        };
        // A line of one gap — two letters — whose gap could not bring its
        // last letter to its part's end is laid with that letter at the end
        // (#309, [`Layout::reach_its_end`]): its one gap has no other to be
        // even with. So it is laid in every clearance search, which grows
        // its junctions round it as it ends. A longer line is brought home
        // once the frame keeps it ([`Layout::reach_ends`]).
        let tracked = match (steps == 1 && gap >= p.natural_gap - GAP_TOLERANCE)
            .then(|| self.reach_its_end(&walk, run, p, (sa, sb), (gap, past), steps, &tracked))
            .flatten()
        {
            Some(reached) => reached.tracked,
            None => tracked,
        };
        Line {
            half: p.half,
            normals: self.normals(p.half, &tracked, (sa, sb)),
            retreats: retreats(self.halves.path(p.half), (sa, sb)),
            tracked,
            gap,
            past,
            word: (p.space > 0.0).then_some(p.space + 2.0 * gap),
            part: (sa, sb),
            room: (lo, hi),
            size: p.size,
            pressed: letters >= 2 && gap < p.natural_gap - GAP_TOLERANCE,
            tracks: letters >= 2,
            steps,
            // The room the part leaves, and what its gaps hold past the
            // font's own, measured where the line lies: on a bend an
            // upright letter takes more of the path than at the apex.
            slack: if letters >= 2 {
                ((hi - lo) - (sb - sa) + steps as f32 * (gap - p.natural_gap)).max(0.0)
            } else {
                f32::INFINITY
            },
        }
    }

    /// Line `p` over `part` brought to its part's end where its tracking
    /// gap could not bring it (#309, issue comment on the B9 final review
    /// 5757429244): `tracked`, laid at `gap`, the gap
    /// [`solve_tracking`] settled on, ends short of the end by more than
    /// the line's tolerance, though no budget stopped it. Its length steps
    /// past the end between `gap` and `past`: a letter crossing a bend sets
    /// itself on the near side of it at the one gap and the far side at
    /// the other — round a sharp corner, the first root of its pairwise
    /// rule jumps along the side — and every letter after it moves with
    /// it, so no gap lands the line's last ink within its tolerance of the
    /// end, nor does a stretch of every word gap, which moves that letter
    /// as a gap does. The secondary rule, in order:
    ///
    /// 1. **The last letter at the end.** The line's last letter's center
    ///    is set at the end of its part, its last gap taking what is left
    ///    ([`track_ending_at`]): kept only where that gap is still the
    ///    line's to within the pairwise rule's tolerance — or, where it is
    ///    the line's only gap, no tighter than its font's own — and the
    ///    letter is its gap clear of the inks before its neighbor. So a
    ///    line of two letters ends at its end; a longer line only where it
    ///    was short by a solve's precision.
    /// 2. **The word gaps past the step.** The line keeps its letter gaps
    ///    even and changes, by one amount, each word gap after the letter
    ///    that steps — the first ink the line laid at `past` sets further
    ///    than its gaps' difference from where the line at `gap` sets it,
    ///    or, with no step between them, the line's first — which moves
    ///    the letters after it but never the one that steps: either the
    ///    line at `gap`, its word gaps there stretched, or the line at
    ///    `past`, its word gaps there shrunk, whichever needs the smaller
    ///    change to each word gap, the other if that one does not land.
    ///    Each change is solved as the line's tracking is ([`falsi`] from
    ///    the straight path's answer, the shortfall shared among the word
    ///    gaps it changes). A word gap at most doubles, and keeps at least
    ///    half its space's advance: past either it does not read as the
    ///    line's word gap.
    ///
    /// `None` where neither brings the line within its tolerance of its
    /// end: it stays as it was laid, and is reported in the frame's trace
    /// ([`Trace::short`]), never left short silently. So is a line whose
    /// step lies in its last word, past every word gap: with its letter
    /// gaps even, its end is the step's on either side of it, wherever the
    /// letters before lie, and only an uneven letter gap would end it at
    /// its end.
    ///
    /// Cost: a [`track`] for its last letter, one for the line at `past`
    /// (read from the frame's [`TrackMemo`] where the solve's last lay
    /// is), and for each of the word gaps' two changes
    /// [`STRETCH_DOUBLINGS`] tracks and [`falsi`]'s [`STRETCH_ROUNDS`] at
    /// most.
    #[allow(clippy::too_many_arguments)] // The line, its part, its solve and its lay.
    fn reach_its_end(
        &self,
        walk: &PathWalk<'_>,
        run: &ShapedRun,
        p: &Prepared,
        part: (f32, f32),
        (gap, past): (f32, Option<f32>),
        steps: usize,
        tracked: &Tracked,
    ) -> Option<Reached> {
        let tolerance = line_tolerance(steps, part.1);
        // Past the end, or short by no more than its tolerance, a line is
        // where its tracking put it; a lay whose ink is placed past every
        // `f32`, or whose shortfall is not a number, is no shortfall to
        // share.
        let finite = |t: &Tracked| t.short.is_finite() && t.glyphs.iter().all(|g| g.pos.is_finite());
        if !(tracked.short > tolerance && finite(tracked)) || self.spent_all() {
            return None;
        }
        #[cfg(test)]
        if NO_SECONDARY_RULE.with(Cell::get) {
            return None;
        }
        self.work.take(p.clusters.len() + TRACK_CALL_STEPS);
        if let Some(t) = track_ending_at(
            walk,
            &run.glyphs,
            &p.clusters,
            part,
            (gap, p.natural_gap),
            self.band(),
        ) {
            return Some(Reached {
                gap,
                words: Words::NONE,
                tracked: t,
            });
        }
        if !p.space.gt(&0.0) {
            return None;
        }
        let beyond = past
            .filter(|g| g.is_finite() && *g > gap && !self.spent_all())
            .map(|g| (g, self.track(walk, &run.glyphs, &p.clusters, part, g)));
        let from = beyond.as_ref().map_or(0, |(g, t)| {
            stepped(&p.clusters, tracked, t, steps as f32 * (g - gap) + tolerance)
        });
        let spaces = p.clusters[from..].iter().filter(|c| c.blank).count();
        if spaces == 0 {
            return None;
        }
        let per = spaces as f32;
        // Each change: the line's gap, which way its word gaps move, how
        // far its end is from where it is owed, and its lay there.
        let mut changes: Vec<(f32, f32, f32, &Tracked)> = vec![(gap, 1.0, tracked.short, tracked)];
        if let Some((g, t)) = &beyond {
            if t.short < -tolerance && finite(t) {
                changes.push((*g, -1.0, -t.short, t));
            }
        }
        changes.sort_by(|a, b| a.2.total_cmp(&b.2));
        let width = GAP_PRECISION.max(tolerance / per);
        for (g, sign, need, laid) in changes {
            let most = if sign > 0.0 {
                p.space + 2.0 * g
            } else {
                0.5 * p.space
            };
            if need / per > most {
                continue;
            }
            let lay = |u: f32| {
                let t = self.track_words(walk, &p.clusters, run, part, (g, Words { by: sign * u, from }));
                let v = if finite(&t) {
                    sign * t.short
                } else {
                    f32::NEG_INFINITY
                };
                (v, t)
            };
            let reached = |u: f32, t: Tracked| Reached {
                gap: g,
                words: Words { by: sign * u, from },
                tracked: t,
            };
            let mut low = (0.0_f32, need, laid.clone());
            let mut u = (need / per * (1.0 + 1e-3)).min(most);
            for _ in 0..STRETCH_DOUBLINGS {
                if !u.gt(&low.0) || self.spent_all() {
                    break;
                }
                let (v, t) = lay(u);
                if v.abs() <= tolerance {
                    return Some(reached(u, t));
                }
                if v < 0.0 {
                    let (u, t) = falsi(lay, low, (u, v, t), tolerance, width, STRETCH_ROUNDS, &|| {
                        self.spent_all()
                    });
                    if t.short.abs() <= tolerance {
                        return Some(reached(u, t));
                    }
                    break;
                }
                if u >= most {
                    break;
                }
                // Still short: on along the line through the last two
                // lays, at least an eighth further, at most four times.
                let fall = (low.1 - v) / (u - low.0);
                let next = if fall > 0.0 {
                    u + v / fall * (1.0 + 1e-3)
                } else {
                    u * 2.0
                };
                low = (u, v, t);
                u = next.clamp(u * 1.125, u * 4.0).min(most);
            }
        }
        None
    }

    /// The lines a frame keeps and whether each junction is a break, each
    /// line of more than one gap short of its part's end — a line of one is
    /// brought home as it is laid ([`lay_one`](Self::lay_one)) — brought there
    /// by its secondary rule ([`reach_its_end`](Self::reach_its_end), #309,
    /// the issue comment on the B9 final review 5757429244), where it lies:
    /// each line the rule brings home is laid so in place of its lay, the
    /// rest as they are, its part and every clearance kept. Kept so where
    /// no junction that was a break is left short of one, no two inks touch
    /// that did not, and the frame's band curve is still where its inks
    /// want it ([`Layout::wants`]): every line so brought home, else each
    /// alone; otherwise the lines as kept. Those short of their ends are
    /// reported ([`Trace::short`]).
    ///
    /// A clearance is never grown for the rule: a line laid at a narrower
    /// span lies within where it lies at whole (ruling R35.1), and the two
    /// are laid apart, so a clearance grown round a line brought home at
    /// one would not be grown at the other. A junction is a break before a
    /// line ends at its end (ruling R33.1).
    ///
    /// Cost: the rule's, for each line short of its end; then, for each
    /// choice of lines brought home tried, a touch check, a measure of each
    /// junction and of the offset the inks want — three choices at most.
    fn reach_ends(
        &self,
        run: &ShapedRun,
        prepared: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        (lines, met): (Vec<Line>, [bool; 2]),
        overlap: bool,
    ) -> (Vec<Line>, [bool; 2]) {
        // Each line short of its end that the rule brings home, by index.
        let home: Vec<(usize, Line, Words)> = lines
            .iter()
            .zip(prepared)
            .enumerate()
            .filter(|(_, (line, _))| line.short_of_its_end() && line.steps > 1)
            .filter_map(|(i, (line, p))| {
                if self.spent_all() {
                    return None;
                }
                let walk = self.walk(line.half, run);
                let reached = self.reach_its_end(
                    &walk,
                    run,
                    p,
                    line.part,
                    (line.gap, line.past),
                    line.steps,
                    &line.tracked,
                )?;
                let words = reached.words;
                let mut line = line.clone();
                line.finish(p, reached);
                line.normals = self.normals(line.half, &line.tracked, line.part);
                Some((i, line, words))
            })
            .collect();
        let offset = self.offset.get();
        let tolerance = resolved(OFFSET_TOLERANCE, self.halves.length);
        // Every line brought home, else each alone: the first that holds
        // what the frame held — each break, inks apart, its band curve.
        let all = std::iter::once(home.iter().collect::<Vec<_>>());
        let each = home.iter().map(|one| vec![one]).filter(|_| home.len() > 1);
        for chosen in all.chain(each) {
            if chosen.is_empty() || self.spent_all() {
                break;
            }
            let mut laid = lines.clone();
            for (i, line, _) in &chosen {
                laid[*i] = line.clone();
            }
            let now = [0, 1].map(|j| self.met(&laid, side, j).0);
            if (0..2).all(|j| !met[j] || now[j])
                && (self.wants(&laid) - offset).abs() <= tolerance
                && if overlap {
                    // Lines that touched already: no two inks touch that
                    // did not (review 5810838863), not none at all.
                    !touching_anew(&boxes_of(&lines), &boxes_of(&laid), &self.work)
                } else {
                    !overlapping(&laid, &self.work)
                }
            {
                #[cfg(test)]
                RULE_KEPT.with(|kept| {
                    let mut by = kept.get();
                    for (_, _, words) in &chosen {
                        by[usize::from(words.by != 0.0) + usize::from(words.by < 0.0)] += 1;
                    }
                    kept.set(by);
                });
                return (laid, now);
            }
        }
        (lines, met)
    }

    /// The outward normal of the outline under each end ink of a line
    /// `tracked` on half `k` over `part`, first then last: the normal its
    /// anchor is offset along ([`Set::normal`], #308), which is the
    /// outline's at the point nearest the anchor — an offset curve's
    /// points lie along the normals of the points they are offset from.
    /// Past a sharp corner, where the outline has no one normal, the way
    /// from the corner to the anchor, as the band curve sweeps round it: an end set round a corner off the end of a side
    /// faces between the two sides, where the side after the corner's
    /// normal would face it the other side's way (a frame round a 10:1
    /// rectangle once took a junction so for lines stacked across it). A
    /// line with no ink takes the normals at its part's ends. Found once
    /// a line is laid ([`Line::normals`]).
    ///
    /// Cost: none, as the lay found them; two path lookups for a line
    /// with no ink.
    fn normals(&self, k: usize, tracked: &Tracked, part: (f32, f32)) -> [Vec2; 2] {
        let path = self.halves.path(k);
        let normal = |s: f32| path.outward(path.at(s).1);
        match tracked.normals {
            Some((first, last)) => [first, last],
            None => [normal(part.0), normal(part.1)],
        }
    }

    /// How far apart the lines' end inks are at each junction (top-left,
    /// bottom-right), box to box, how far apart they must be — two of the lines' mean
    /// word gap (ruling R33.1), over the lines that have one: a line of
    /// one word has no word gap to count, the span stretching its letter
    /// gaps alone; with no word gap on any line, two of a line's letter
    /// gaps stand for one; and for two end inks whose outward normals
    /// turn more than 135° apart, lines stacked across a thin outline
    /// rather than lines meeting, only not to touch (ruling R35.1) — and,
    /// for each line, how much drawing its
    /// end back at that junction parts the two: how much further apart
    /// their boxes are once its end box has moved the way its end moves
    /// as its clearance grows, by the break owed where lines meet, as a
    /// share of that — 1 drawn straight away, none drawn across, short of
    /// none drawn toward the other. Round a thin outline's tip, or a sharp
    /// corner its band curve turns at with no offset, two end boxes lie
    /// side by side where the line through their centers says they lie
    /// end to end, and the cosine between the way an end moves and the way
    /// away from the other's center reads drawing back as parting them
    /// where it parts nothing (#308). A single line is measured against the junction point
    /// on the frame's band curve, owing half that.
    fn apart(&self, lines: &[Line]) -> Junctions {
        let words: Vec<f32> = lines.iter().filter_map(|l| l.word).collect();
        let word = if words.is_empty() {
            lines.iter().map(|l| 2.0 * l.gap).sum::<f32>() / lines.len().max(1) as f32
        } else {
            words.iter().sum::<f32>() / words.len() as f32
        };
        // Which way line `l`'s end at junction `j` moves as its
        // clearance there grows: on along its half at the top-left, back
        // at the bottom-right.
        let retreat = |l: &Line, j: usize| l.retreats[j];
        // Ends that touch have no way apart to measure against: each
        // line's drawing back parts them, as at the junction itself.
        // How far drawing back is measured over: the break owed where
        // the lines meet.
        let probe = (JUNCTION_WORD_GAPS * word).max(TOUCH);
        let helps = |l: &Line, j: usize, own: (Vec2, Vec2), other: (Vec2, Vec2)| {
            let apart = box_gap(own, other);
            if !l.tracks {
                0.0
            } else if apart > 0.0 {
                let moved = retreat(l, j) * probe;
                let drawn = box_gap((own.0 + moved, own.1 + moved), other);
                if drawn.is_finite() {
                    (drawn - apart) / probe
                } else {
                    0.0
                }
            } else {
                1.0
            }
        };
        let mut out = Junctions {
            apart: [f32::INFINITY; 2],
            depth: [f32::INFINITY; 2],
            want: [0.0; 2],
            helps: [[0.0; 2]; 2],
        };
        match lines {
            [one, two, ..] => {
                out.want = [JUNCTION_WORD_GAPS * word; 2];
                if let (Some((a1, b1)), Some((a2, b2))) = (one.tracked.ends, two.tracked.ends) {
                    for (j, (e1, e2)) in [(a1, a2), (b1, b2)].into_iter().enumerate() {
                        if one.normals[j].dot(two.normals[j]) < STACKED {
                            out.want[j] = TOUCH;
                        }
                        out.apart[j] = box_gap(e1, e2);
                        out.depth[j] = separation(e1, e2);
                        out.helps[one.half][j] = helps(one, j, e1, e2);
                        out.helps[two.half][j] = helps(two, j, e2, e1);
                    }
                }
            }
            [one] => {
                out.want = [0.5 * JUNCTION_WORD_GAPS * word; 2];
                // The junction on the frame's band curve (#308).
                let path = self.halves.path(one.half);
                let (ja, jb) = self.halves.junctions[one.half];
                let offset = self.offset.get();
                let points = [path.offset_at(ja, offset).point, path.offset_at(jb, offset).point];
                if let Some((a, b)) = one.tracked.ends {
                    for (j, (e, p)) in [(a, points[0]), (b, points[1])].into_iter().enumerate() {
                        out.apart[j] = box_gap(e, (p, p));
                        out.depth[j] = separation(e, (p, p));
                        out.helps[one.half][j] = helps(one, j, e, (p, p));
                    }
                }
            }
            [] => {}
        }
        out
    }
}

/// Each junction's measure ([`Layout::apart`]): its lines' end inks'
/// distance, the distance wanted, and how much each half's line parts
/// them by drawing back.
struct Junctions {
    apart: [f32; 2],
    /// [`apart`](Self::apart) where the inks are apart, and how deep
    /// they overlap, as a negative, where they do ([`separation`]): what
    /// a round's step is measured from, so that two inks drawn out of an
    /// overlap are short by its depth too, and show it (ruling R43).
    depth: [f32; 2],
    want: [f32; 2],
    /// `[half][junction]`.
    helps: [[f32; 2]; 2],
}

/// The clearance each half's line keeps from each junction, by arc
/// length: `[half][junction]`, top-left then bottom-right.
type Clear = [[f32; 2]; 2];

/// An ink box on the canvas, `(min, max)`.
type InkBox = (Vec2, Vec2);
/// A frame's side ornament as laid round a junction (ruling R33.1): the
/// walk it is laid along — the first half's path, each copy's inks
/// anchored at their own centers on the frame's band curve, where the
/// text's band lies (#308, ruling B15.1) — and one copy's clusters.
struct SideOrnament<'a> {
    walk: PathWalk<'a>,
    /// The frame's band curve as it stood when the ornament was taken.
    band: Band,
    glyphs: &'a [GlyphMetrics],
    clusters: Vec<Cluster>,
    /// The frame's work, which its lays count against (rulings R44.2,
    /// R44.3).
    work: &'a Work,
    /// The ink boxes the touch check of the arc being laid weighs besides
    /// its own copies: the text's and the copies already laid.
    base: Cell<usize>,
    /// The glyphs of one copy, each an ink box at most.
    per_copy: usize,
    /// The ink sizes of one copy's glyphs, and the classes of height the
    /// touch check of the arc being laid holds at most: the text's and
    /// these (ruling R51.2).
    sizes: Vec<Vec2>,
    classes: Cell<usize>,
}

impl<'a> SideOrnament<'a> {
    /// The side ornament of `run` (inked) over `range`, laid round the
    /// halves of `layout` on its band curve as it stands.
    fn new(layout: &'a Layout<'_>, run: &'a ShapedRun, range: Range<usize>) -> SideOrnament<'a> {
        SideOrnament {
            walk: line_walk(layout.halves.path(0), run, layout.outward),
            band: layout.band(),
            glyphs: &run.glyphs,
            per_copy: range.len(),
            sizes: run.glyphs[range.clone()]
                .iter()
                .filter_map(|g| g.ink.map(|ink| ink.max - ink.min))
                .collect(),
            clusters: clusters_of(&run.glyphs, range),
            work: &layout.work,
            base: Cell::new(0),
            classes: Cell::new(0),
        }
    }

    /// Weigh the touch checks of its arcs as holding the classes of
    /// height of `text`, the text's ink boxes, and of its own copies'
    /// glyphs ([`height_classes`], ruling R51.2).
    fn beside(&self, text: &[InkBox]) {
        let sizes = text.iter().map(|(min, max)| *max - *min);
        self.classes
            .set(height_classes(sizes.chain(self.sizes.iter().copied())));
        let looks = self.work.looks.get();
        self.work.looks.set(Looks {
            reserved: looks.reserved.max(self.classes.get()),
            ..looks
        });
    }

    /// Whether the frame can afford `copies` copies of the arc being laid
    /// and the touch check of them after (rulings R48.2, R51.2): the
    /// check's glyph steps at [`touch_looks`] looks a box of the classes
    /// of height it holds ([`check_steps`]) held back from what the budget
    /// has left.
    fn affords(&self, copies: usize) -> bool {
        let boxes = self
            .base
            .get()
            .saturating_add(copies.saturating_mul(self.per_copy));
        self.work.affords(check_steps(boxes, self.classes.get()))
    }

    /// Count `copies` copies laid against the frame's glyph steps.
    fn count(&self, copies: usize) {
        self.work.take(copies.saturating_mul(self.clusters.len().max(1)));
    }

    /// Count `copies` copies of a model lay against the frame's glyph
    /// steps ([`InkLay::modeled`]): [`MODEL_CLUSTERS_A_STEP`] clusters a
    /// step.
    fn modeled(&self, copies: usize) {
        let clusters = copies.saturating_mul(self.clusters.len().max(1));
        self.work.take(clusters.div_ceil(MODEL_CLUSTERS_A_STEP).max(1));
    }

    /// The arc of this side between two text inks, `None` without an
    /// ink in the ornament.
    fn arc(&self, from: ArcEnd, to: ArcEnd) -> Option<SideArc<'_>> {
        SideArc::new(self, from, to)
    }

    /// About how long one copy and its two joins are along a path:
    /// what a junction's clearance grows by while it holds none.
    fn room(&self) -> f32 {
        let advance: f32 = self.clusters.iter().map(|c| c.advance).sum();
        let floor = self.arc_floor().unwrap_or(0.0);
        advance + 2.0 * floor
    }

    /// The least gap a copy is laid at: its first ink's side bearings,
    /// never less than [`JOIN_FLOOR`].
    fn arc_floor(&self) -> Option<f32> {
        let (advance, ink) = self
            .clusters
            .iter()
            .find(|c| !c.blank)
            .and_then(|c| c.ink.map(|i| (c.advance, i)))?;
        Some((advance - (ink.max.x - ink.min.x)).max(JOIN_FLOOR))
    }
}

/// The arc of a side between the text ink it starts past (`from`) and
/// the one it ends before (`to`), along the side's path.
struct SideArc<'s> {
    side: &'s SideOrnament<'s>,
    /// The text ink the arc starts past, as the pairwise rule sets the
    /// first copy from it.
    start: Set,
    to: (Vec2, Vec2),
    /// The least gap a copy is laid at ([`SideOrnament::arc_floor`]).
    floor: f32,
    /// Where `from`'s and `to`'s centers lie along the path, `cb` past
    /// `ca`.
    ca: f32,
    cb: f32,
    /// Whether a search of the arc stopped because the frame
    /// could not afford more (ruling R48.2).
    stopped: Cell<bool>,
    /// What the copies its fill kept want of the frame's offset (#308).
    reached: Cell<Option<SideWant>>,
}

impl<'s> SideArc<'s> {
    fn new(side: &'s SideOrnament<'s>, (from, ca): ArcEnd, (to, near_b): ArcEnd) -> Option<Self> {
        side.clusters.iter().find(|c| !c.blank && c.ink.is_some())?;
        let path = side.walk.path;
        // Each end ink's anchor lies on the band curve where it is set
        // along the path, the outline's point nearest it (#308).
        let cb = unwrap(near_b, ca, path.length());
        let (on, at) = path.offset_in(ca, side.band.offset);
        Some(SideArc {
            side,
            start: Set {
                at: middle(from),
                half: 0.5 * (from.1 - from.0),
                on: on.point,
                bend: Bending::Read(at),
                center: ca,
                moves: on.moves,
                normal: on.normal,
                reach: Reach::default(),
            },
            to,
            floor: side.arc_floor()?,
            ca,
            cb,
            stopped: Cell::new(false),
            reached: Cell::new(None),
        })
    }

    /// `n` copies laid at `gap` by the pairwise rule ([`InkLay`]): the
    /// first ink `gap` from `from`'s, each next ink `gap` from the last, a
    /// space owing its advance and `gap` more. Returns the lay, and how far
    /// the last ink is from `to` and where along the path its center lies.
    /// The frame's budget is the caller's to read: a search reads before
    /// each lay whether the frame can afford `n` copies and their check
    /// ([`SideOrnament::affords`]).
    ///
    /// Cost: an [`InkLay::set_at`] or a few a glyph.
    fn lay(&self, n: usize, gap: f32) -> (InkLay<'_>, (f32, f32)) {
        let mut lay = self.begun(gap);
        for _ in 0..n {
            for cluster in &self.side.clusters {
                lay.cluster(cluster, gap);
            }
        }
        self.side.count(n);
        let end = self.end_of(&lay);
        (lay, end)
    }

    /// A lay of copies at `gap` begun past the text ink the arc starts
    /// from, on the frame's band curve.
    fn begun(&self, gap: f32) -> InkLay<'_> {
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut lay = InkLay::new(
            &self.side.walk,
            self.side.glyphs,
            self.ca,
            gap,
            self.side.band.clone(),
        )
        .after(self.start);
        #[cfg(test)]
        {
            lay.side = true;
        }
        lay
    }

    /// How far the last ink `lay` set is from `to`, and where along the
    /// path its center lies.
    fn end_of(&self, lay: &InkLay<'_>) -> (f32, f32) {
        lay.last
            .map(|l| (box_gap((l.at - l.half, l.at + l.half), self.to), l.center))
            .unwrap_or((f32::INFINITY, self.ca))
    }

    /// Whether `n` copies at `gap` fit: the last ink's center short of
    /// `to`'s and its ink at least `gap` from `to`'s. A miss that is not
    /// a number does not fit.
    fn fits(&self, n: usize, gap: f32) -> bool {
        let (_, (end, at)) = self.lay(n, gap);
        at < self.cb && end >= gap
    }

    /// How much room `n` copies laid by `lay` leave: how far the last
    /// ink is from `to`'s past the gap — at least none where they fit
    /// ([`fits`](Self::fits)) — or, its center past `to`'s, as far past
    /// as that and the gap, a negative. A room that is not a number is
    /// none at all.
    fn room_of(&self, lay: &InkLay<'_>, gap: f32) -> f32 {
        let (end, at) = self.end_of(lay);
        let v = if at < self.cb {
            end - gap
        } else {
            -(gap + (at - self.cb))
        };
        if v.is_nan() {
            f32::NEG_INFINITY
        } else {
            v
        }
    }

    /// Copies laid at the floor's gap, one after another, up to `most` or
    /// the first that does not fit: what they place, and for each `k`
    /// copies that fit the room they leave and how many glyphs they
    /// place. Copies are laid in a row, each set on from the last, so the
    /// first `k` of any row are `k` copies laid on their own: this one lay
    /// tells every copy count's room at the floor (ruling R44.3). `None`
    /// when the frame cannot afford the row: before each copy it must
    /// afford the copies and their touch check ([`SideOrnament::affords`],
    /// rulings R47.2, R48.2).
    ///
    /// Cost: one lay of the copies that fit and the one that does not.
    fn floor_row(&self, most: usize) -> Option<FloorRow> {
        let gap = self.floor;
        let mut lay = self.begun(gap);
        let mut rooms = Vec::new();
        while rooms.len() < most.min(MAX_SIDE_COPIES) {
            if !self.side.affords(rooms.len() + 1) {
                return None;
            }
            for cluster in &self.side.clusters {
                lay.cluster(cluster, gap);
            }
            self.side.count(1);
            let room = self.room_of(&lay, gap);
            if room.is_nan() || room < 0.0 {
                break;
            }
            rooms.push((room, lay.out.len(), lay.reaches.len()));
        }
        Some((
            Copies {
                glyphs: lay.out,
                reaches: lay.reaches,
            },
            rooms,
        ))
    }

    /// How many gaps `n` copies have: one before each ink of every copy,
    /// and the join to `to`.
    fn gaps(&self, n: usize) -> f32 {
        let inked = self.side.clusters.iter().filter(|c| !c.blank).count().max(1);
        (n * inked + 1) as f32
    }

    /// The gap at which `n` copies would fill the arc were its path
    /// straight, from the room `room` they leave at the floor: the room
    /// shared among their gaps.
    fn straight(&self, n: usize, room: f32) -> f32 {
        self.floor + room / self.gaps(n)
    }

    /// The widest gap at which `n` copies fit with every gap and both
    /// joins equal, to [`SIDE_PRECISION`], and the copies laid at it,
    /// given the room `at_floor` they leave at the floor and what they
    /// place there ([`floor_row`](Self::floor_row)): where the last copy's
    /// ink is the gap from `to`'s. That room falls with the gap by about
    /// its gaps' count a point — one before each ink of every copy, and
    /// the join to `to` — bent only by the path, so it is solved as a
    /// line's tracking is (ruling R44.3, [`solve_tracking`]): from the gap
    /// solved on model lays ([`model_gap`](Self::model_gap); #309, ruling
    /// B16.2d), taken where the copies laid from the path at it land within
    /// the tolerance — where the band curve's forms are the curve, they
    /// do — or else from the straight path's root
    /// ([`straight`](Self::straight)), widened until it brackets the root,
    /// and closed by [`falsi`]. A copy past `to`'s center reads as past the
    /// end. The frame's budget spent, the widest gap found to fit so far.
    ///
    /// Cost: [`model_gap`](Self::model_gap)'s, a lay of `n` copies where
    /// it holds, else one a widening and [`falsi`]'s, at most
    /// [`SIDE_ROUNDS`]; none at all where the floor is the answer.
    fn solve(&self, n: usize, at_floor: f32, floor_out: Copies) -> (f32, Copies, bool) {
        let gaps = self.gaps(n);
        // As precise as the copies' inks are set: each to within the
        // pairwise rule's tolerance, as a line's end is ([`line_tolerance`]).
        let tolerance = resolved(SIDE_PRECISION, self.cb).max(gaps * INK_TOLERANCE);
        if at_floor <= tolerance {
            return (self.floor, floor_out, true);
        }
        // Each probe's gap and how far it misses, so that the answer is
        // known to hold the equal-gap rule or not.
        let probes: std::cell::RefCell<Vec<(f32, f32)>> = std::cell::RefCell::new(Vec::new());
        // The room each lay leaves, as `falsi` reads it
        // (`solved_room`).
        let room = |gap: f32| {
            let (lay, _) = self.lay(n, gap);
            let v = self.solved_room(&lay, gap, tolerance);
            probes.borrow_mut().push((gap, v));
            (
                v,
                Copies {
                    glyphs: lay.out,
                    reaches: lay.reaches,
                },
            )
        };
        // The frame cannot afford `n` copies and their touch check: the
        // search stops (ruling R48.2).
        let stop = || {
            let stopped = !self.side.affords(n);
            if stopped {
                self.stopped.set(true);
            }
            stopped
        };
        let mut low = (self.floor, at_floor, floor_out);
        // The gap solved on model lays first (#309, ruling B16.2d): each
        // copy set where the band curve's form puts it, so that where the
        // forms are the curve, the copies laid from the path at that gap
        // fill the arc, laid once. Else the straight path's root, a hair
        // past it — as the iterative rule alone starts — and past it a
        // hair further each widening, so that a probe brackets the root a
        // bend moved.
        let straight = self.straight(n, at_floor);
        let layer = layer_on();
        let mut modeled = layer
            .then(|| self.model_gap(n, (straight, at_floor), tolerance, &stop))
            .flatten();
        let mut g = modeled.unwrap_or(straight);
        let mut first = true;
        if !layer {
            g = straight + (at_floor / gaps) * 1e-3 + f32::MIN_POSITIVE;
            first = false;
        }
        for _ in 0..TRACK_DOUBLINGS {
            if !g.is_finite() || stop() {
                break;
            }
            let (v, out) = room(g);
            if std::mem::take(&mut first) && v.abs() <= tolerance {
                return (g, out, true);
            }
            if v < 0.0 {
                let (gap, out) = falsi(
                    room,
                    low,
                    (g, v, out),
                    tolerance,
                    tolerance / gaps,
                    SIDE_ROUNDS,
                    &stop,
                );
                let exact = probes
                    .borrow()
                    .iter()
                    .any(|&(at, v)| at.to_bits() == gap.to_bits() && v.abs() <= tolerance);
                return (gap, out, exact);
            }
            if v <= tolerance {
                return (g, out, true);
            }
            // On from the model's gap, its room shared among the gaps, a
            // hair past: it lies a hair short of the root.
            let next = if modeled.take().is_some() {
                g + v / gaps * (1.0 + 1e-3) + f32::MIN_POSITIVE
            } else {
                (g + v / gaps * (1.0 + 1e-3)).max(g * 1.125)
            };
            low = (g, v, out);
            g = next;
        }
        (low.0, low.2, false)
    }

    /// The room the copies `lay` set at `gap` leave, as [`solve`](Self::solve)
    /// reads it: a room of none fits, and reads as positive; a miss whose
    /// last ink touches `to`'s, or is set past its center, reads as at
    /// least twice `tolerance` short, so it is never taken for the answer:
    /// copies a hair short of `to` close with a join a hair narrower than
    /// their gaps, as a line's end a hair past its part's is laid, but
    /// copies that touch the text are no answer.
    fn solved_room(&self, lay: &InkLay<'_>, gap: f32, tolerance: f32) -> f32 {
        self.shifted_room(lay, gap, tolerance, 0.0)
    }

    /// [`solved_room`](Self::solved_room), the room `lay` leaves moved by
    /// `shift` first: a model lay's room as the copies read from the path
    /// would leave it ([`model_gap`](Self::model_gap)).
    fn shifted_room(&self, lay: &InkLay<'_>, gap: f32, tolerance: f32, shift: f32) -> f32 {
        let v = self.room_of(lay, gap) + shift;
        let (end, at) = self.end_of(lay);
        if v >= 0.0 {
            v.max(f32::MIN_POSITIVE)
        } else if at >= self.cb || end.is_nan() || end <= TOUCH {
            v.min(-2.0 * tolerance)
        } else {
            v
        }
    }

    /// The gap at which `n` copies fill the arc as model lays set them
    /// (#309, ruling B16.2d; [`InkLay::modeled`]): each copy's letters
    /// where the band curve's form puts them, no path lookup but where a
    /// form ends. A model lay at the floor first tells whether the forms
    /// read the arc — most of its letters set from them — and how far its
    /// room is from the room `at_floor` the copies read from the path leave
    /// there — none to float error at normal scale; far out, where each
    /// letter the rule sets lies a float past its root, their sum — and
    /// every model room is read moved by that much, so it is the room a
    /// lay read from the path would leave. Solved as [`solve`](Self::solve)
    /// solves the copies read from the path — from `straight`, the
    /// straight path's root, widened until it brackets the root, and
    /// closed by [`falsi`] — to a quarter of `tolerance`, the middle of
    /// what a lay read at it is judged to. `None` where it brackets no
    /// root, or the frame cannot afford a lay (`stop`).
    ///
    /// Cost: a model lay of `n` copies at the floor, one a widening and
    /// one each of [`falsi`]'s rounds, at most [`SIDE_ROUNDS`], each
    /// counted at its weight ([`SideOrnament::modeled`]).
    fn model_gap(
        &self,
        n: usize,
        (straight, at_floor): (f32, f32),
        tolerance: f32,
        stop: &dyn Fn() -> bool,
    ) -> Option<f32> {
        let gaps = self.gaps(n);
        let aim = 0.25 * tolerance;
        let lay_at = |gap: f32| {
            let mut lay = self.begun(gap);
            lay.modeled = true;
            for _ in 0..n {
                for cluster in &self.side.clusters {
                    lay.cluster(cluster, gap);
                }
            }
            self.side.modeled(n);
            lay
        };
        if stop() {
            return None;
        }
        // A model lay at the floor that read more of its letters from the
        // path than from the band curve's forms is a lay read from the
        // path: no model of the arc. Further than the tolerance off at the
        // floor, the model does not tell how the copies read from the path
        // lie: far out, where the float each letter lies past its root
        // moves with the gap.
        let at = lay_at(self.floor);
        if at.abandoned || at.formed.1 > at.formed.0 {
            return None;
        }
        let shift = at_floor - self.room_of(&at, self.floor);
        if matches!(
            shift.abs().partial_cmp(&tolerance),
            None | Some(Ordering::Greater)
        ) {
            return None;
        }
        let room = |gap: f32| (self.shifted_room(&lay_at(gap), gap, tolerance, shift), ());
        let mut low = (self.floor, at_floor, ());
        let mut g = straight;
        for _ in 0..TRACK_DOUBLINGS {
            if !g.is_finite() || stop() {
                return None;
            }
            let (v, ()) = room(g);
            if v.abs() <= aim {
                return Some(g);
            }
            if v < 0.0 {
                let (gap, ()) = falsi(room, low, (g, v, ()), aim, aim / gaps, SIDE_ROUNDS, stop);
                return Some(gap);
            }
            low = (g, v, ());
            g = (g + v / gaps * (1.0 + 1e-3)).max(g * 1.125);
        }
        None
    }

    /// Fill the arc (ruling R33.1): the whole number of copies whose
    /// equal gap is nearest `target`, at least one and at most
    /// [`MAX_SIDE_COPIES`] or as many as `budget` glyphs hold — weighing
    /// the most that fit at the target and one more, whose gap is
    /// narrower — every gap and both joins equal. Nothing when not even
    /// one copy fits, or the budget holds none. Where the frame cannot
    /// afford the copies and their touch check ([`SideOrnament::affords`],
    /// rulings R47.2, R48.2, R50.5) the arc is left as it stands, which it
    /// returns: before the floor's row is laid, or before its search ends
    /// where the copies it has do not hold the equal-gap rule, empty
    /// ([`SideLoss::Empty`]); before its search ends where they do, kept
    /// short with them ([`SideLoss::Short`]). `None` when the search ended.
    ///
    /// The two counts weighed are those whose gaps bracket the target:
    /// guessed from the floor's row by the straight path, then moved a
    /// count at a time until the gaps solved bracket it, so a guess the
    /// path's bend puts off by one costs a solve, never the answer.
    ///
    /// Cost (ruling R44.3): the floor's row, and two solves where the
    /// guess holds — a lay of every copy each widening and each of
    /// [`falsi`]'s rounds, the copies chosen kept as laid.
    fn fill(&self, target: f32, budget: usize, out: &mut Vec<PlacedGlyph>) -> Option<SideLoss> {
        let per_copy = self
            .side
            .clusters
            .iter()
            .map(|c| c.glyphs.len())
            .sum::<usize>()
            .max(1);
        let most = MAX_SIDE_COPIES.min(budget / per_copy);
        if most == 0 {
            return None;
        }
        let Some((row, rooms)) = self.floor_row(most) else {
            return Some(SideLoss::Empty);
        };
        let fit = rooms.len();
        if fit == 0 {
            return None;
        }
        let mut solved: Vec<Option<(f32, Copies, bool)>> = vec![None; fit + 1];
        let mut solve = |n: usize| -> f32 {
            if solved[n].is_none() {
                let (room, glyphs, reaches) = rooms[n - 1];
                let copies = Copies {
                    glyphs: row.glyphs[..glyphs].to_vec(),
                    reaches: row.reaches[..reaches].to_vec(),
                };
                solved[n] = Some(self.solve(n, room, copies));
            }
            solved[n].as_ref().map_or(self.floor, |s| s.0)
        };
        // The most copies that fit at the target, by the straight path: the
        // count whose gap is the last at or past it.
        let guess = (1..=fit)
            .take_while(|&k| self.straight(k, rooms[k - 1].0) >= target)
            .count()
            .clamp(1, fit);
        let mut n = guess;
        // Down while `n` copies do not reach the target, up while one more
        // still does: the pair whose gaps bracket it, or an end. A frame
        // that cannot afford `k` copies and their check stops the search
        // where it stands (ruling R48.2).
        let short = |k: usize| {
            let short = !self.side.affords(k);
            if short {
                self.stopped.set(true);
            }
            short
        };
        while n > 1 && solve(n) < target && !short(n) {
            n -= 1;
        }
        while n < fit && solve(n + 1) >= target && !short(n + 1) {
            n += 1;
        }
        let mut best = (n, solve(n));
        if n < fit {
            let wider = solve(n + 1);
            if (wider - target).abs() < (best.1 - target).abs() {
                best = (n + 1, wider);
            }
        }
        // A search the frame could not afford to finish keeps its copies
        // only where they hold the equal-gap rule, kept short; else the arc
        // is left empty. Either way it is reported (rulings R48.2, R50.5).
        match solved[best.0].take() {
            Some((_, _, false)) if self.stopped.get() => Some(SideLoss::Empty),
            Some((_, copies, _)) => {
                out.extend(copies.glyphs);
                self.reached.set(offset_want(&copies.reaches));
                self.stopped.get().then_some(SideLoss::Short)
            }
            None => None,
        }
    }
}

/// A side arc's copies laid at the floor's gap ([`SideArc::floor_row`]):
/// what they place, and for each count of them that fits the room it
/// leaves, the glyphs it places and the inks it sets.
type FloorRow = (Copies, Vec<(f32, usize, usize)>);

/// A side arc's copies as laid: their glyphs, and each ink's reach off
/// the outline ([`Reach`]), what the frame's offset is held to (#308).
#[derive(Clone)]
struct Copies {
    glyphs: Vec<PlacedGlyph>,
    reaches: Vec<Reach>,
}

/// The most `reaches` reach toward the outline and away from it
/// ([`SideWant`]); `None` for none.
fn offset_want(reaches: &[Reach]) -> Option<SideWant> {
    reaches.iter().fold(None, |want: Option<SideWant>, r| {
        let here = SideWant {
            toward: r.toward,
            away: r.away,
        };
        Some(want.map_or(here, |want| want.with(here)))
    })
}

/// A text ink a side arc runs from or to ([`side_arcs`]): its box, and
/// where along the first half's path it is set, its anchor on the band
/// curve there.
type ArcEnd = (InkBox, f32);

/// The text inks each arc of a side runs between, and which junctions
/// (top-left, bottom-right) the arc lies round: with two lines one arc
/// per junction; with one, one arc round both, over the half it leaves.
///
/// Each end ink comes with where it is set along the first half's path:
/// exactly, for the first half's line; mirrored, for the second's, whose
/// path runs round the other way from the same start ([`Halves`]).
/// `length` is the outline's.
fn side_arcs(lines: &[Line], length: f32) -> Vec<([bool; 2], ArcEnd, ArcEnd)> {
    let ends: Vec<(usize, (ArcEnd, ArcEnd))> = lines
        .iter()
        .filter_map(|l| {
            let (a, b) = l.tracked.ends?;
            let (first, last) = l.tracked.centers?;
            let on_first = |s: f32| if l.half == 0 { s } else { length - s };
            Some((l.half, ((a, on_first(first)), (b, on_first(last)))))
        })
        .collect();
    match ends.as_slice() {
        [(_, (a1, b1)), (_, (a2, b2)), ..] => vec![([true, false], *a2, *a1), ([false, true], *b1, *b2)],
        // The first half's line leaves the second: clockwise from its
        // last ink round to its first.
        [(0, (a, b))] => vec![([true, true], *b, *a)],
        // The second half's line leaves the first: clockwise from its
        // first ink over the top to its last.
        [(_, (a, b))] => vec![([true, true], *a, *b)],
        [] => Vec::new(),
    }
}

impl Layout<'_> {
    /// Whether junction `j` (top-left 0, bottom-right 1) of `laid` is a
    /// break (ruling R33.1): the lines' end inks there at least as far
    /// apart as [`apart`](Self::apart) wants — a distance that is not a
    /// number is not — and, with `side` (the ornament's run and glyphs),
    /// one whole copy fitting between them at its own floor gap: the
    /// test [`SideArc::solve`] makes before it solves anything, so the
    /// two cannot disagree, and all this asks — whether, not how wide.
    /// With it, the junction's measure.
    fn met(&self, laid: &[Line], side: Option<(&ShapedRun, &Range<usize>)>, j: usize) -> (bool, Junctions) {
        let measured = self.apart(laid);
        let words = measured.apart[j] >= measured.want[j] * CLEAR_MARGIN;
        let copy = side.is_none_or(|(drawn, range)| {
            let side = SideOrnament::new(self, drawn, range.clone());
            side_arcs(laid, self.halves.length)
                .into_iter()
                .filter(|(at, _, _)| at[j])
                .all(|(_, from, to)| side.arc(from, to).is_none_or(|arc| arc.fits(1, arc.floor)))
        });
        (words && copy, measured)
    }

    /// The lines laid at `span`, each line's clearance from each
    /// junction grown from `seed` at the whole span (ruling R33.1) and
    /// then, below whole, grown on at `span` until each junction is a
    /// break there too (ruling R35.1): so a line at a narrower span lies
    /// within where it lies at whole. Returns the lines, their clearance,
    /// and whether each junction (top-left, bottom-right) is a break.
    ///
    /// **The frame's band curve** (#308, ruling B15.1). Every ink of the
    /// frame is anchored on one offset curve of the outline ([`Band`]),
    /// at the offset its inks want ([`Layout::wants`]): outward, the least
    /// at which every ink of both lines and of the side ornament clears
    /// the outline; inward, the frame's ink band centered on it. Which ink
    /// reaches furthest depends on where each lies, and where each lies
    /// on the offset a little, so the offset is solved with the lines, one
    /// unknown for the frame, at each span its clearance is grown at
    /// ([`Layout::clear_at`]), from the offset the lines' inks would want
    /// at their halves' apexes ([`Layout::estimate`]): at whole as the
    /// frame at whole solves it, so that a narrower span's clearance is grown on from the very
    /// clearance the frame at whole holds, and a line at the narrower span
    /// lies within where it lies at whole, whatever the offset each span's
    /// lines want.
    ///
    /// Cost: [`clear_at`](Self::clear_at)'s, once at whole and, below it,
    /// once more at `span`.
    fn lay_clear(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        seed: Clear,
        backtracks: usize,
    ) -> Cleared {
        let mut search = Search {
            memo: LayMemo::default(),
            backtracks,
        };
        // From the offset the lines' inks would want at their halves'
        // apexes, whatever the frame stood at before: what the frame at
        // whole starts from too.
        self.offset
            .set(self.estimate(run, lines.iter().map(|p| (p.half, p.clusters.as_slice()))));
        let whole = self.clear_at(run, lines, side, 1.0, seed, &mut search);
        if span >= 1.0 {
            return whole;
        }
        self.clear_at(run, lines, side, span, whole.clear, &mut search)
    }

    /// The lines laid at `span`, each line's clearance grown from `seed`
    /// ([`grow`](Self::grow)), on the frame's band curve at the offset
    /// they want ([`Layout::wants`], #308): solved first against the lines
    /// laid at `seed` ([`solve_offset`]), from the offset the frame
    /// stands at, so that the clearance is grown at nearly the offset it
    /// ends at; then, the clearance grown, checked against the lines as
    /// they lie, and the clearance grown again from `seed` at the offset
    /// they want where they miss it by more than [`OFFSET_TOLERANCE`] —
    /// the line through the last two misses aimed at none, as
    /// [`solve_offset`] aims it — [`OFFSET_ROUNDS`] times at most.
    ///
    /// Cost: a lay of each line at `seed` for each offset tried, one where
    /// the frame's last offset holds; and a clearance search for each
    /// offset the lines are checked at, one where it holds.
    fn clear_at(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        seed: Clear,
        search: &mut Search,
    ) -> Cleared {
        #[cfg(test)]
        if let Some(bits) = OFFSET_OVERRIDE.with(Cell::get) {
            self.offset.set(f32::from_bits(bits as u32));
            return self.grow(run, lines, side, span, seed, search);
        }
        let tolerance = resolved(OFFSET_TOLERANCE, self.halves.length);
        let (settled, _) = solve_offset(
            self.offset.get(),
            tolerance,
            OFFSET_ROUNDS,
            |offset, _: Option<&Vec<Line>>| {
                self.offset.set(offset);
                self.lay(run, lines, seed, span, &mut search.memo)
            },
            |laid| self.wants(laid),
            &|| self.spent_all(),
        );
        self.offset.set(settled);
        let mut last: Option<(f32, f32)> = None;
        let mut round = 0;
        loop {
            let cleared = self.grow(run, lines, side, span, seed, search);
            round += 1;
            let offset = self.offset.get();
            let miss = self.wants(&cleared.lines) - offset;
            if miss.abs().partial_cmp(&tolerance) != Some(Ordering::Greater)
                || round >= OFFSET_ROUNDS
                || self.spent_all()
            {
                return cleared;
            }
            let next = match last {
                Some((was, missed)) if missed != miss => offset - miss * (offset - was) / (miss - missed),
                _ => offset + miss,
            };
            last = Some((offset, miss));
            self.offset
                .set(if next.is_finite() { next } else { offset + miss });
        }
    }

    /// The lines laid at `span` with each line's clearance grown from
    /// `clear` until each junction is a break ([`met`](Self::met)).
    ///
    /// A junction's clearance grows, round by round, while it is not a
    /// break ([`met`](Self::met)). Each round shares what is missing —
    /// the words' shortfall, ends that overlap short by how deep they do
    /// too, or for a copy of the side ornament the room along the path it
    /// lacks — among the lines whose drawing back parts their ends there,
    /// by how much it does, measured afresh each round: where both halves
    /// run on side by side, as on a long flat outline, only the line whose
    /// end moves away is drawn back. Drawing a line back narrows its
    /// tracking, and so the word gap the break is measured in: the step is
    /// damped by how much that eases the break wanted, or, where the last
    /// round grew the junction alone and by all it asked, aimed by what
    /// that round gave — a secant on the junction's own measure (ruling
    /// R43). A round that would press a line it left unpressed is halved
    /// until it does not ([`BACKTRACKS`] times at most), so the rounds
    /// close on the least clearance from below rather than overshoot. A
    /// junction stops growing when no line's drawing back parts its ends,
    /// when a line is pressed, or after [`CLEAR_ROUNDS`] rounds: the bound
    /// on its work.
    fn grow(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        mut clear: Clear,
        search: &mut Search,
    ) -> Cleared {
        let backtracks = search.backtracks;
        let memo = &mut search.memo;
        let mut laid = self.lay(run, lines, clear, span, memo);
        // Each junction's depth before the round that grew it, if the last
        // round did; how much that round grew it; whether it grew it
        // alone — growth at the other junction narrows both lines' gaps,
        // and moves this one's ends with them, either way — and by all its
        // step asked, none held to a line's slack nor halved for a press:
        // what its next step is aimed by.
        let mut depth_before = [f32::NEG_INFINITY; 2];
        let mut grown_by = [0.0_f32; 2];
        let mut alone = [false; 2];
        let mut free = [false; 2];
        let mut stuck = [false; 2];
        let mut met_now: Option<[bool; 2]> = None;
        // Whether the last round's whole step, laid, pressed a line it
        // left unpressed (#308).
        let mut full_pressed = false;
        for _ in 0..CLEAR_ROUNDS {
            // A pressed line ends the search: the frame goes to its last
            // resorts from here — a shrink, a cut — and they grow the
            // clearance again for what they leave, so rounds that grow it
            // round a line already too long for its room are work spent
            // on nothing (ruling R38.3).
            if self.spent_all() || laid.iter().any(|l| l.pressed) {
                break;
            }
            let mut grow: Clear = [[0.0; 2]; 2];
            let mut grown = false;
            // Whether this round grows each junction for a copy of the
            // side ornament alone, its words' break met (#308).
            let mut for_copy = [false; 2];
            // Whether a line's slack held back this round's growth at
            // each junction.
            let mut held = [false; 2];
            let mut met_here = [false; 2];
            let mut measured_helps = [[0.0_f32; 2]; 2];
            let measures = [0, 1].map(|j| self.met(&laid, side, j));
            for j in 0..2 {
                let (met, measured) = &measures[j];
                let met = *met;
                met_here[j] = met;
                for (row, helps) in measured_helps.iter_mut().zip(measured.helps) {
                    row[j] = helps[j].max(0.0);
                }
                if met || stuck[j] {
                    continue;
                }
                let apart = measured.apart[j];
                let depth = measured.depth[j];
                let aim = measured.want[j] * CLEAR_MARGIN;
                let helps = [0, 1].map(|k| measured.helps[k][j].max(0.0));
                let total = helps[0] + helps[1];
                if total.partial_cmp(&0.0) != Some(Ordering::Greater) {
                    stuck[j] = true;
                    continue;
                }
                // Ends whose distance is not a number are short of it.
                let by_words = matches!(apart.partial_cmp(&aim), None | Some(Ordering::Less));
                for_copy[j] = !by_words;
                let shortfall = if by_words {
                    // Ends that overlap are short by how deep they do too.
                    aim - if depth.is_finite() {
                        depth.min(apart)
                    } else {
                        apart
                    }
                } else {
                    // Room for a copy of the side ornament is room along
                    // the path between the end inks: the arcs from the
                    // junction to each line's end ink, less the inks'
                    // own halves.
                    let room = side.map_or(0.0, |(drawn, range)| {
                        SideOrnament::new(self, drawn, range.clone()).room()
                    });
                    let arc: f32 = laid
                        .iter()
                        .map(|l| {
                            let (ja, jb) = self.halves.junctions[l.half];
                            let from = if j == 0 { l.part.0 - ja } else { jb - l.part.1 };
                            from - l.size.x
                        })
                        .sum();
                    // The arc measured on the lines' paths is not quite
                    // the room the ornament's own walk finds: while the
                    // copy still does not fit, it grows by a quarter of a
                    // copy at the least.
                    (room - if laid.len() > 1 { arc } else { apart }).max(0.25 * room)
                };
                // Drawing a line back narrows its tracking, and so the word
                // gap the break is measured in: by twice its gap's fall,
                // which is the span's share of the clearance spread over
                // its gaps. A step that counts only the inks' parting
                // overshoots by that much, so it is cut to the share of
                // the closing the parting makes (ruling R38.3); with no
                // word gap in the measure — a break of a copy's room, or
                // of not touching — nothing eases.
                let eases = [0, 1].map(|k| {
                    let words = laid.iter().filter(|l| l.word.is_some()).count();
                    laid.iter()
                        .find(|l| l.half == k)
                        .filter(|l| {
                            by_words
                                && l.tracks
                                && l.gap > 0.0
                                && measured.want[j] > TOUCH
                                && (words == 0 || l.word.is_some())
                        })
                        .map_or(0.0, |l| {
                            let among = if words == 0 { laid.len() } else { words };
                            // A single line owes half the break.
                            let owed = if laid.len() > 1 { 1.0 } else { 0.5 };
                            owed * JUNCTION_WORD_GAPS * 2.0 * span / (l.steps.max(1) * among.max(1)) as f32
                        })
                });
                let shares = helps.map(|h| h / total.max(1.0));
                let parting: f32 = (0..2).map(|k| helps[k] * shares[k]).sum();
                let closing: f32 = (0..2).map(|k| (helps[k] + eases[k]) * shares[k]).sum();
                let damped = if closing > 0.0 {
                    shortfall * parting / closing
                } else {
                    shortfall
                };
                // A break wider than any `f32` — two word gaps of a line
                // tracked over an outline as long as an `f32` — is short
                // by an infinite amount, and an infinite step shared by
                // a share of zero, or held to a line's slack, is not a
                // number: the step is held to the largest `f32`, past
                // which a clearance presses its line to a point however
                // much more it is (the probe in
                // `test_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely`).
                // A step that is not a number — a shortfall that is not
                // one — grows by the least.
                let mut step = if damped.is_nan() {
                    MIN_GROWTH
                } else {
                    damped.clamp(MIN_GROWTH, f32::MAX)
                };
                // Where the last round grew this junction alone, by all its
                // step asked, the step is aimed by what that round gave: the
                // shortfall over the depth it gained a point of growth, at
                // most four times the last step — a secant on the junction's
                // own measure (ruling R43), where the damping guesses how
                // much drawing back eases the break, and closes on it a
                // share a round.
                let gained = (depth - depth_before[j]) / grown_by[j];
                if by_words && alone[j] && free[j] && gained.is_finite() && gained > 0.0 {
                    let secant = (aim - depth) / gained;
                    if secant.is_finite() {
                        step = secant.clamp(MIN_GROWTH, (4.0 * grown_by[j]).max(MIN_GROWTH));
                    }
                }
                for k in 0..2 {
                    // Two lines that both draw back part by about the
                    // sum: each gives its share.
                    grow[k][j] = step * shares[k];
                }
                grown = true;
            }
            // A line drawn back past its slack is pressed, where the
            // other line at the junction may have room to give: each
            // line's growth is held to its slack, what is cut handed to
            // the other line where it parts the ends there too and has
            // room (rulings R34, R38.1). Where no line that parts a
            // junction has room the round grows as it would, and the line
            // it presses takes the last resorts — for its words' break; a
            // junction grown for a copy of the side ornament alone is held
            // to the slack all the same, for a frame loses its sides before
            // its text (ruling R48.3): an arc no copy fits is left empty
            // (#308: one copy of a 92-byte ornament, longer than a half
            // 3.24 long, once grew a single line's clearance to 28 and cut
            // its text).
            for k in 0..2 {
                let Some(line) = laid.iter().find(|l| l.half == k) else {
                    continue;
                };
                let wanted = grow[k][0] + grow[k][1];
                if wanted <= line.slack || wanted <= 0.0 {
                    continue;
                }
                let other = 1 - k;
                let mut room = laid
                    .iter()
                    .find(|l| l.half == other)
                    .map_or(0.0, |l| (l.slack - grow[other][0] - grow[other][1]).max(0.0));
                let keep = line.slack / wanted;
                for j in 0..2 {
                    let cut = grow[k][j] * (1.0 - keep);
                    let theirs = measured_helps[other][j];
                    let mine = measured_helps[k][j];
                    let handed = if theirs > 0.0 {
                        (cut * mine / theirs).min(room)
                    } else {
                        0.0
                    };
                    if line.slack >= MIN_GROWTH || handed > 0.0 || for_copy[j] {
                        held[j] |= cut > 0.0;
                        grow[k][j] -= cut;
                        grow[other][j] += handed;
                        room -= handed;
                    }
                }
            }
            let grew = [0, 1].map(|j| grow[0][j] > 0.0 || grow[1][j] > 0.0);
            for j in 0..2 {
                depth_before[j] = if grew[j] {
                    measures[j].1.depth[j]
                } else {
                    f32::NEG_INFINITY
                };
                alone[j] = grew[j] && !grew[1 - j];
            }
            met_now = Some(met_here);
            // Nothing left to grow — every step held to a slack of none —
            // and so nothing to lay again.
            if !grown || !grew.iter().any(|g| *g) {
                break;
            }
            let pressed: Vec<bool> = laid.iter().map(|l| l.pressed).collect();
            // A round that draws a line back past what slack it has,
            // there being no other line with room to take it, presses it
            // knowingly: halving it would only close on the same press in
            // smaller steps (ruling R38.3).
            let foreseen = laid
                .iter()
                .any(|l| !l.pressed && l.slack < MIN_GROWTH && grow[l.half][0] + grow[l.half][1] > l.slack);
            let backtracks = if foreseen { 0 } else { backtracks };
            // A round that grows junctions for a copy of the side ornament
            // alone never presses a line (ruling R48.3, B15.5): a step held
            // to a slack lays the line at the very edge of its room, and a
            // try that presses it even so is not taken however far it is
            // halved — the search ends with the lines as they lie, the arc
            // its copy does not fit left empty.
            let copy_only = (0..2).all(|j| !grew[j] || for_copy[j]);
            let mut taken = false;
            let mut scale = 1.0;
            for _ in 0..=backtracks {
                // The frame's budget spent: the round not taken, the lines
                // as the last round laid them (ruling R44.2).
                if self.spent_all() {
                    break;
                }
                let mut next = clear;
                for k in 0..2 {
                    for j in 0..2 {
                        next[k][j] += grow[k][j] * scale;
                    }
                }
                let last = scale < 0.5_f32.powi(backtracks as i32) * 1.5;
                // Once a try has pressed a line, each halved try that would
                // press a line it left unpressed, looked at first at the
                // cost of a `track` a line, is halved again without being
                // laid (#308): round a sharp corner a line's length jumps by
                // a letter as its start moves, and tries that each press it
                // by the jump, laid in full, cost a clearance search most of
                // its work. So is a whole step held to a line's slack after a
                // round whose whole step, laid, pressed a line: there the
                // slack, measured where the line lay, is no measure of where
                // it would lie, and the step lands on the press round after
                // round (the quatrain inward round the 600 × 12 sharp
                // rectangle, cut apart: 750 tracks laid, 657 looked at first).
                if (scale < 1.0 || (full_pressed && held.iter().any(|h| *h)))
                    && !last
                    && self.would_press(run, lines, &laid, next)
                {
                    scale *= 0.5;
                    continue;
                }
                let tried = self.lay(run, lines, next, span, memo);
                let presses = tried.iter().zip(&pressed).any(|(l, was)| l.pressed && !was);
                if scale == 1.0 {
                    full_pressed = presses;
                }
                if !presses || (last && !copy_only) {
                    taken = true;
                    clear = next;
                    laid = tried;
                    met_now = None;
                    free = held.map(|h| !h && scale == 1.0);
                    grown_by = [0, 1].map(|j| (grow[0][j] + grow[1][j]) * scale);
                    break;
                }
                scale *= 0.5;
            }
            if copy_only && !taken {
                met_now = Some(met_here);
                break;
            }
        }
        let met = met_now.unwrap_or_else(|| [0, 1].map(|j| self.met(&laid, side, j).0));
        Cleared {
            lines: laid,
            clear,
            met,
        }
    }
}

impl Layout<'_> {
    /// Whether laying `lines` with the clearance `next` would press a
    /// line that `laid`, their last lay, left unpressed, a line whose room
    /// `next` moves: whether, at its font's gap from the start of its new
    /// room, it ends past the room's end by more than its tolerance — what
    /// [`lay_one`](Self::lay_one) lays a line pressed by, its slide
    /// starting it there whenever it is longer than its room. The cheap
    /// look a clearance round's try takes before it is laid
    /// ([`grow`](Self::grow), #308).
    ///
    /// Cost: a [`track`] for each line whose room moves.
    fn would_press(&self, run: &ShapedRun, lines: &[Prepared], laid: &[Line], next: Clear) -> bool {
        lines.iter().zip(laid).any(|(p, line)| {
            let (lo, hi) = self.room(p.half, next[p.half]);
            let letters = p.clusters.iter().filter(|c| !c.blank).count();
            if line.pressed || letters < 2 || (lo, hi) == line.room {
                return false;
            }
            let long = self.reach(p.half, run, &p.clusters, lo, p.natural_gap);
            long - (hi - lo) > line_tolerance(line.steps, hi)
        })
    }
}

/// A frame's lines laid with their clearance ([`Layout::lay_clear`]).
struct Cleared {
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction, top-left and bottom-right, is a break.
    met: [bool; 2],
}

/// Each line's ink boxes, as [`touching_anew`] reads them.
fn boxes_of(lines: &[Line]) -> Vec<&[InkBox]> {
    lines.iter().map(|l| l.tracked.boxes.as_slice()).collect()
}

/// Whether two of the inks `after` holds, line by line, touch that did not
/// in `before` — the same lines, each with as many inks, some laid again
/// ([`Layout::reach_ends`]) — but for a line's neighboring inks, which
/// [`overlapping`] excuses too: the touch check of a frame whose lines
/// touched already (review 5810838863). The pairs that touch after are a
/// subset of those that touched before exactly where none touches but
/// for a pair that did.
///
/// Cost: [`overlapping`]'s, and a box gap a pair it looks at.
fn touching_anew(before: &[&[InkBox]], after: &[&[InkBox]], work: &Work) -> bool {
    let boxes = after
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.iter().enumerate().map(move |(i, b)| ((k, i), *b)))
        .collect();
    let was = |(k, i): (usize, usize)| {
        before
            .get(k)
            .and_then(|l| l.get(i))
            .copied()
            .filter(|b| has_ink(*b))
    };
    let touched = |a: (usize, usize), b: (usize, usize)| match (was(a), was(b)) {
        (Some(p), Some(q)) => box_gap(p, q) <= TOUCH,
        _ => false,
    };
    touching(
        boxes,
        |(k, i), (l, j)| (k == l && i.abs_diff(j) <= 1) || touched((k, i), (l, j)),
        work,
    )
}

/// Whether two of `lines`' inks touch (ruling R35.2): any two of
/// different lines, or of one line but not neighbors — two neighbors
/// are as far apart as the line's tracking sets them, a font with no
/// side bearings' zero gap included. Sorted by their left edges, each
/// box is held only against those that start before it ends.
///
/// Cost: O(n log n) in the lines' inks, and a pair per two that share
/// some of their width.
fn overlapping(lines: &[Line], work: &Work) -> bool {
    let boxes = lines
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.tracked.boxes.iter().enumerate().map(move |(i, b)| ((k, i), *b)))
        .collect();
    touching(boxes, |(k, i), (l, j)| k == l && i.abs_diff(j) <= 1, work)
}

/// Whether two of `boxes` touch — no more than [`TOUCH`] apart
/// ([`box_gap`]) — but for the pairs `exempt` excuses, each box known by
/// its key ([`touch_scan`]). A box with no ink to touch with is left out
/// ([`has_ink`], ruling R49.2). Each box given and each pair looked at is
/// counted against `work` (rulings R46.1, R48.1), [`BOXES_A_STEP`] a
/// step in its sweep and [`STRIPPED_A_STEP`] past it.
fn touching<K: Copy>(boxes: Vec<(K, InkBox)>, exempt: impl Fn(K, K) -> bool, work: &Work) -> bool {
    let given = boxes.len();
    let boxes: Vec<(K, InkBox)> = boxes.into_iter().filter(|(_, b)| has_ink(*b)).collect();
    let mut looked = Looked::default();
    let touches = touch_scan(&boxes, &exempt, &mut looked);
    #[cfg(test)]
    BOXES_WEIGHED.with(|count| count.set(count.get() + looked.swept + looked.stripped));
    work.take(weighing_steps(given.saturating_add(looked.swept)));
    if looked.stripped > 0 {
        work.take(
            boxes
                .len()
                .saturating_add(looked.stripped)
                .div_ceil(STRIPPED_A_STEP),
        );
    }
    work.looked(&looked, boxes.len());
    touches
}

/// Whether an ink box has ink to touch with (ruling R49.2): every
/// corner a number, and some width and some height. A box zero wide or
/// zero tall — the ink a control character's glyph is given, a line of
/// no area — draws nothing another ink could meet, and one with a corner
/// that is not a number has no place on the canvas; [`box_gap`] reads a
/// component that is not a number as no distance, so either would once
/// have been weighed against every box near it, and a run of them laid
/// side by side, each with every other: the flow_raw finding
/// `raw_frame_touch_check_past_its_looks_a_box`, 4,315 looks over 95
/// boxes.
fn has_ink((min, max): InkBox) -> bool {
    min.is_finite() && max.is_finite() && max.x > min.x && max.y > min.y
}

/// How many pairs a box [`touch_scan`]'s sweep looks at, on the mean,
/// before it lays its boxes in strips instead: round a ring it looks at
/// some 0.25.
const SWEEP_PAIRS: usize = 2;

/// How much taller than a box and [`TOUCH`] a strip of its class is at
/// least ([`strip_class`]): enough that rounding a box's corner to its
/// strip never puts two boxes that touch two strips apart.
const CELL_MARGIN: f64 = 1.0 + 1.0 / (1u64 << 20) as f64;

/// The pairs of boxes [`touch_scan`] has looked at: in its sweep, and
/// past it, in its strips ([`stripped`]) — of those, the ones between a
/// class of height and the finer ones — and the classes of height its
/// strips held, none where it did not reach them.
#[derive(Default)]
struct Looked {
    swept: usize,
    stripped: usize,
    across: usize,
    classes: usize,
}

/// The most pairs a frame's touch checks looked at a box (rulings R48.2,
/// R51.2): of the check that came nearest its bound ([`touch_looks`]),
/// its looks, boxes and classes of height; and, each over all its
/// checks, the most a box in its sweep and within each class's own
/// strips, as those looks and the boxes; the most a box for each class,
/// between a class and the finer ones, as those looks, the boxes and the
/// classes; and the most classes a side arc's check held, and one was
/// held back for.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Looks {
    pub(crate) worst: (usize, usize, usize),
    pub(crate) base: (usize, usize),
    pub(crate) across: (usize, usize, usize),
    /// The most classes of height a side arc's touch check held, and the
    /// most one was held back for ([`height_classes`]).
    pub(crate) classes: usize,
    pub(crate) reserved: usize,
}

impl Looks {
    /// These and one more check, of `boxes` boxes, that looked at the
    /// pairs `looked` holds.
    fn more(self, looked: &Looked, boxes: usize) -> Looks {
        let looks = looked.swept.saturating_add(looked.stripped);
        self.with(Looks {
            worst: (looks, boxes, looked.classes),
            base: (looks.saturating_sub(looked.across), boxes),
            across: (looked.across, boxes, looked.classes),
            classes: 0,
            reserved: 0,
        })
    }

    /// The most of these and `other`, each part apart.
    pub(crate) fn with(self, other: Looks) -> Looks {
        // `a` over `of` against `b` over `by`, as a fraction, without one.
        let more = |(a, of): (usize, usize), (b, by): (usize, usize)| {
            (a as u128) * (by.max(1) as u128) > (b as u128) * (of.max(1) as u128)
        };
        let bound = |(looks, boxes, classes): (usize, usize, usize)| {
            (looks, boxes.saturating_mul(touch_looks(classes)))
        };
        let per_class =
            |(looks, boxes, classes): (usize, usize, usize)| (looks, boxes.saturating_mul(classes));
        Looks {
            worst: if more(bound(other.worst), bound(self.worst)) {
                other.worst
            } else {
                self.worst
            },
            base: if more(other.base, self.base) {
                other.base
            } else {
                self.base
            },
            across: if more(per_class(other.across), per_class(self.across)) {
                other.across
            } else {
                self.across
            },
            classes: self.classes.max(other.classes),
            reserved: self.reserved.max(other.reserved),
        }
    }

    /// Whether the check nearest its bound looked at no more than
    /// [`touch_looks`] pairs a box of its classes.
    pub(crate) fn within(&self) -> bool {
        let (looks, boxes, classes) = self.worst;
        looks <= boxes.saturating_mul(touch_looks(classes))
    }
}

/// Whether two of `boxes`, each with ink ([`has_ink`]), touch, as
/// [`touching`] asks, the pairs of them looked at added to `looked`.
///
/// The pairs weighed are a sweep's: sorted by where each box starts along
/// the axis the boxes spread further on, ties in their order, a box is
/// weighed against each after it that starts no further than [`TOUCH`]
/// past where it ends. Round a ring those are the few inks either side
/// of each, and the sweep is the answer. Where it looks at more than
/// [`SWEEP_PAIRS`] pairs a box — boxes stacked across the axis, a side
/// ornament's copies down a straight side — it stops, and the boxes are
/// looked at in strips across the axis instead ([`stripped`]), where a
/// box looks only at the inks near it. Two inks laid on one another
/// touch, and the check returns at the first two that do.
///
/// Cost: a sort, and [`SWEEP_PAIRS`] looks a box at most before
/// [`stripped`]'s: the inks near each box, whatever the tallest ink and
/// however many inks the check is given. The sweep alone looked at every
/// pair that shared some of its width, which down a straight side across
/// the axis is every pair: quadratic in a side ornament's copies (R44.3's
/// 200-byte ornament round a sharp square, some 5,000 pairs a glyph). One
/// strip height for every box, the tallest's, was quadratic too, in the
/// thin boxes a tall one's strip holds (review 5745584645: `"⸻"` and a
/// hundred `"|"` round a 20,000 sharp square, 28.9 pairs a box).
fn touch_scan<K: Copy>(boxes: &[(K, InkBox)], exempt: &impl Fn(K, K) -> bool, looked: &mut Looked) -> bool {
    let spread = |axis: usize| {
        let (lo, hi) = boxes
            .iter()
            .map(|(_, b)| b.0[axis])
            .filter(|v| v.is_finite())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            });
        hi - lo
    };
    let axis = if spread(1) > spread(0) { 1 } else { 0 };
    let start = |i: usize| boxes[i].1 .0[axis];
    // Whether box `i` touches `j`, after it in the sweep's order, but for
    // a pair `exempt` excuses.
    let weigh = |i: usize, j: usize| {
        let ((k, a), (l, b)) = (boxes[i], boxes[j]);
        !exempt(k, l) && box_gap(a, b) <= TOUCH
    };
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&i, &j| start(i).total_cmp(&start(j)));

    // The sweep itself, while it looks at a few pairs a box: round a ring,
    // the few inks either side of each along the axis.
    let most = boxes.len().saturating_add(1).saturating_mul(SWEEP_PAIRS);
    let mut swept = 0;
    // The boxes the sweep has weighed against every box after them in its
    // order that starts within their reach: a prefix of its order.
    let mut done = vec![false; boxes.len()];
    let whole = 'sweep: {
        for (at, &i) in order.iter().enumerate() {
            let end = boxes[i].1 .1[axis];
            for &j in &order[at + 1..] {
                swept += 1;
                if swept > most {
                    break 'sweep false;
                }
                if !within_reach(end, start(j)) {
                    break;
                }
                if weigh(i, j) {
                    looked.swept += swept;
                    return true;
                }
            }
            done[i] = true;
        }
        true
    };
    looked.swept += swept;
    if whole {
        return false;
    }
    stripped(boxes, axis, exempt, &done, looked)
}

/// Whether two of `boxes` touch, as [`touch_scan`] asks, looked at in
/// strips across `axis` by class of height, as a hierarchical grid lays
/// them (rulings R48, R50.1), the pairs looked at added to `looked`.
///
/// Each box has a class, the least power of two as tall across the axis
/// as it and [`TOUCH`] are ([`strip_class`]), and lies in the strip of
/// its class its least corner across the axis falls in. Two boxes of a
/// class or finer that touch lie less than one strip of that class
/// apart, corner to corner, so in one of its strips or in two beside each
/// other. So each class's boxes are weighed against one another in its
/// strips, each strip with itself and the one after it; and against each
/// finer class's boxes, in whichever of two ways places fewer boxes —
/// each finer box placed in this class's strips, and weighed against the
/// strip before its own, its own and the one after; or each of this
/// class's boxes copied into every strip of the finer class it lies
/// across, with one more either side, and weighed against that strip's
/// finer boxes. Every pair is weighed at the class of the taller of the
/// two, and once. Within the strips weighed together the boxes are swept
/// as [`touch_scan`] sweeps them, each against those after it that start
/// within its reach.
///
/// So the looks are output-sensitive: a box looks only at inks near it,
/// of its own class or a coarser one, in the strips of that class round
/// it, which start within its reach along the axis or whose own reach
/// holds where it starts. No tall box makes the strips a thin one is
/// weighed in tall: a thin box is weighed against thin boxes in strips of
/// its own height, and against a tall one only in the strips the tall
/// one lies across. Where no two touch, which is when every look is made,
/// boxes of a class set more than [`TOUCH`] apart fit no more than a few
/// across the strips weighed together, each at least half a strip tall;
/// along the axis, within one box's reach, as many as its width holds of
/// theirs: font geometry, never the count of boxes nor the tallest's
/// height. Each box placed or copied counts as a look, and a class pair
/// places no more boxes than the finer class has, nor than its own
/// boxes' copies.
///
/// **What the sweep weighed is not weighed again** (#308, ruling
/// B15.2). The strips sweep in [`touch_scan`]'s order, by the same
/// reach, and weigh each pair once, from the box before the other in
/// that order: so a box `done` holds — one the sweep before them weighed
/// against every box after it within its reach before it stopped, a
/// prefix of its order — has no pair left to weigh, and never scans. A
/// box after it still weighs it, from its own side. Where lines are
/// stacked across the axis the sweep stops at [`SWEEP_PAIRS`] looks a
/// box with most of its boxes done, and the strips once weighed all of
/// them again: the quatrain outward round the 1,000 × 20 ellipse, `Fit`
/// at span ½ with a 40-byte ornament, its budget of 20,000 spent with
/// its 102 inks laid, looked at 537 pairs, 5.26 a box — the sweep's 207
/// and the strips' 330, 120 weighed and 210 at the end of a box's reach,
/// nearly all from boxes the sweep had done; with those left out, the
/// strips' 330 are 40, and the check's 247, 2.42 a box.
///
/// Cost: a sort of each class's boxes, and of the boxes each pair of
/// classes places, and the looks.
fn stripped<K: Copy>(
    boxes: &[(K, InkBox)],
    axis: usize,
    exempt: &impl Fn(K, K) -> bool,
    done: &[bool],
    looked: &mut Looked,
) -> bool {
    let across = 1 - axis;
    let class = |i: usize| strip_class(boxes[i].1 .1[across] - boxes[i].1 .0[across]);
    // Box `i` in the strip of class `class` its least corner across the
    // axis falls in.
    let placed = |i: usize, class: i32| {
        let (min, max) = boxes[i].1;
        Swept {
            // A strip past `i64`'s range is its end's, shared by every box
            // there.
            strip: (f64::from(min[across]) / strip_height(class)).floor() as i64,
            start: min[axis],
            end: max[axis],
            i,
        }
    };
    let by_strip = |a: &Swept, b: &Swept| a.strip.cmp(&b.strip).then_with(|| a.before(b));
    // `a` weighed against `after`, the boxes after it in the sweep's order,
    // while they start within its reach.
    let looks = Cell::new(0_usize);
    let scan = &mut |a: &Swept, after: &[Swept]| {
        if done[a.i] {
            return false;
        }
        for b in after {
            looks.set(looks.get() + 1);
            if !within_reach(a.end, b.start) {
                return false;
            }
            let ((k, p), (l, q)) = (boxes[a.i], boxes[b.i]);
            if !exempt(k, l) && box_gap(p, q) <= TOUCH {
                return true;
            }
        }
        false
    };

    // Each class's boxes in its own strips, finest first.
    let mut classes: Vec<(i32, usize)> = (0..boxes.len()).map(|i| (class(i), i)).collect();
    classes.sort_unstable();
    let mut levels: Vec<(i32, Vec<Swept>)> = Vec::new();
    let mut at = 0;
    while at < classes.len() {
        let class = classes[at].0;
        let end = at + classes[at..].iter().take_while(|c| c.0 == class).count();
        let mut own: Vec<Swept> = classes[at..end].iter().map(|&(_, i)| placed(i, class)).collect();
        own.sort_unstable_by(by_strip);
        levels.push((class, own));
        at = end;
    }
    looked.classes = levels.len();
    let mut touches = false;
    for (c, (class, own)) in levels.iter().enumerate() {
        touches = touches || within_strips(own, scan);
        for (fine, finer) in &levels[..c] {
            if touches {
                break;
            }
            let before = looks.get();
            // The cheaper of the two ways to weigh a finer class's boxes
            // against this one's: each of this class's boxes copied into
            // each of the finer class's strips it lies across, or each finer
            // box placed in this class's strips.
            // The strips round a box saturate at `i64`'s ends, as
            // `placed`'s do: a box further out than `i64` counts strips is
            // in its end's, and so is the one beside it.
            let spans = |i: usize| {
                let (min, max) = boxes[i].1;
                let strip = |at: f32| (f64::from(at) / strip_height(*fine)).floor() as i64;
                (
                    strip(min[across]).saturating_sub(1),
                    strip(max[across] + TOUCH).saturating_add(1),
                )
            };
            let copies = own.iter().try_fold(0_usize, |n, b| {
                let (low, high) = spans(b.i);
                let n = n
                    .saturating_add(usize::try_from(high.saturating_sub(low)).unwrap_or(usize::MAX))
                    .saturating_add(1);
                (n <= finer.len()).then_some(n)
            });
            touches = if copies.is_some() {
                let mut copied: Vec<Swept> = own
                    .iter()
                    .flat_map(|b| {
                        let (low, high) = spans(b.i);
                        (low..=high).map(move |strip| Swept { strip, ..*b })
                    })
                    .collect();
                copied.sort_unstable_by(by_strip);
                looked.stripped += copied.len();
                looked.across += copied.len();
                in_shared_strips(finer, &copied, scan)
            } else {
                let mut visiting: Vec<Swept> = finer.iter().map(|b| placed(b.i, *class)).collect();
                visiting.sort_unstable_by(by_strip);
                looked.stripped += visiting.len();
                looked.across += visiting.len();
                among_strips(&visiting, own, scan)
            };
            looked.across += looks.get() - before;
        }
        if touches {
            break;
        }
    }
    looked.stripped += looks.get();
    touches
}

/// The class of [`stripped`]'s strips a box `height` tall across the
/// axis is weighed in: the least power of two, as an exponent, no
/// shorter than its height and [`TOUCH`] with [`CELL_MARGIN`] to spare.
fn strip_class(height: f32) -> i32 {
    let tall = (f64::from(height) + f64::from(TOUCH)) * CELL_MARGIN;
    let class = tall.log2().ceil() as i32;
    if strip_height(class) < tall {
        class + 1
    } else {
        class
    }
}

/// How tall a strip of class `class` is ([`strip_class`]).
fn strip_height(class: i32) -> f64 {
    2.0_f64.powi(class)
}

/// The strips `order` runs over, sorted by strip and then in the sweep's
/// order: each strip's range, and the strip.
fn strip_runs(order: &[Swept]) -> impl Iterator<Item = (i64, Range<usize>)> + '_ {
    let mut at = 0;
    std::iter::from_fn(move || {
        let strip = order.get(at)?.strip;
        let end = at + order[at..].iter().take_while(|e| e.strip == strip).count();
        let run = (strip, at..end);
        at = end;
        Some(run)
    })
}

/// Whether two of `order`'s boxes touch, one class's in its strips
/// ([`stripped`]): each strip's boxes against one another and against
/// the strip after it's, by `scan`.
fn within_strips(order: &[Swept], scan: &mut impl FnMut(&Swept, &[Swept]) -> bool) -> bool {
    let runs: Vec<(i64, Range<usize>)> = strip_runs(order).collect();
    for (n, (strip, run)) in runs.iter().enumerate() {
        let here = &order[run.clone()];
        let beside = match runs.get(n + 1) {
            Some((next, next_run)) if strip.checked_add(1) == Some(*next) => &order[next_run.clone()],
            _ => &[][..],
        };
        if here.iter().enumerate().any(|(k, a)| scan(a, &here[k + 1..])) || between(here, beside, scan) {
            return true;
        }
    }
    false
}

/// Whether one of `finer`'s boxes touches one of `own`'s, both placed in
/// the strips of `own`'s class ([`stripped`]): each strip of `finer`'s
/// against the strips of `own`'s before it, at it and after it.
fn among_strips(finer: &[Swept], own: &[Swept], scan: &mut impl FnMut(&Swept, &[Swept]) -> bool) -> bool {
    let runs: Vec<(i64, Range<usize>)> = strip_runs(own).collect();
    let mut first = 0;
    for (strip, run) in strip_runs(finer) {
        let here = &finer[run];
        let lowest = strip.saturating_sub(1);
        while first < runs.len() && runs[first].0 < lowest {
            first += 1;
        }
        for (near, near_run) in &runs[first..] {
            if *near > strip.saturating_add(1) {
                break;
            }
            if between(here, &own[near_run.clone()], scan) {
                return true;
            }
        }
    }
    false
}

/// Whether one of `finer`'s boxes touches one of `copied`'s, both in the
/// strips of `finer`'s class, each copied box in each strip it lies
/// across ([`stripped`]): each strip's boxes of the two against each
/// other.
fn in_shared_strips(
    finer: &[Swept],
    copied: &[Swept],
    scan: &mut impl FnMut(&Swept, &[Swept]) -> bool,
) -> bool {
    let runs: Vec<(i64, Range<usize>)> = strip_runs(copied).collect();
    let mut first = 0;
    for (strip, run) in strip_runs(finer) {
        while first < runs.len() && runs[first].0 < strip {
            first += 1;
        }
        match runs.get(first) {
            Some((there, copies))
                if *there == strip && between(&finer[run], &copied[copies.clone()], scan) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Whether a box of `here` touches one of `there`, each in the sweep's
/// order: each box of either against those of the other after it in
/// that order, by `scan`.
fn between(here: &[Swept], there: &[Swept], scan: &mut impl FnMut(&Swept, &[Swept]) -> bool) -> bool {
    for (one, other) in [(here, there), (there, here)] {
        let mut first = 0;
        for a in one {
            while first < other.len() && other[first].before(a) == Ordering::Less {
                first += 1;
            }
            if scan(a, &other[first..]) {
                return true;
            }
        }
    }
    false
}

/// A box as [`stripped`] sweeps it: its strip, where it starts and ends
/// along the sweep's axis, and which box it is.
#[derive(Clone, Copy)]
struct Swept {
    strip: i64,
    start: f32,
    end: f32,
    i: usize,
}

/// Whether a box starting at `start` along the sweep's axis, after one
/// that ends at `end`, may touch it: no more than [`TOUCH`] past it, by
/// the subtraction [`box_gap`] makes — so a pair it reads as touching is
/// never passed over where `end + TOUCH` rounds below `start` (review
/// 5757429244: two inks 0.001 apart once `f32` subtracted them, at a
/// thousandth of a point's scale). Monotone in `start`, so a sweep in its
/// order stops at the first box past it.
fn within_reach(end: f32, start: f32) -> bool {
    (start - end).le(&TOUCH)
}

impl Swept {
    /// The sweep's order: by where each box starts, ties in their order.
    fn before(&self, other: &Swept) -> Ordering {
        self.start.total_cmp(&other.start).then(self.i.cmp(&other.i))
    }
}

/// `lines` cut to the longest run of their inks, the first line's then
/// the second's, of which no two touch ([`overlapping`]), and the halves
/// of the lines cut: the cheapest cut that leaves no ink touching, for a
/// frame whose work budget is spent before its last resorts held it
/// apart (rulings R38.1, R44.2). A line cut to no ink is left out.
///
/// Cost: a bisection of [`overlapping`]s over the frame's inks.
fn untouched(lines: &[Line], work: &Work) -> (Vec<Line>, Vec<usize>) {
    let total: usize = lines.iter().map(|l| l.tracked.boxes.len()).sum();
    #[cfg(test)]
    let (spent, reserved) = (work.spent.get(), work.reserved.replace(true));
    let cut_to = |m: usize| {
        let mut left = m;
        let mut out = Vec::new();
        for line in lines {
            if left == 0 {
                break;
            }
            let keep = left.min(line.tracked.boxes.len());
            left -= keep;
            let mut line = line.clone();
            if keep < line.tracked.boxes.len() {
                line.tracked.glyphs.truncate(line.tracked.marks[keep]);
                line.tracked.boxes.truncate(keep);
                line.tracked.marks.truncate(keep);
            }
            out.push(line);
        }
        out
    };
    // Apart with no ink, touching with all; the longest run apart lies
    // between.
    let (mut apart, mut over) = (0, total);
    while over - apart > 1 {
        let mid = apart + (over - apart) / 2;
        // Each look cuts the lines again: as much work as weighing them.
        work.take(weighing_steps(mid));
        if overlapping(&cut_to(mid), work) {
            over = mid;
        } else {
            apart = mid;
        }
    }
    let kept = cut_to(apart);
    #[cfg(test)]
    {
        work.reserved.set(reserved);
        let taken = work.spent.get() - spent;
        UNTOUCHED_TAKEN.with(|most| {
            if taken > most.get().0 {
                most.set((taken, untouched_steps(total, work.classes.get())));
            }
        });
    }
    let cut = lines
        .iter()
        .enumerate()
        .filter(|(k, l)| {
            kept.get(*k)
                .is_none_or(|c| c.tracked.boxes.len() < l.tracked.boxes.len())
        })
        .map(|(_, l)| l.half)
        .collect();
    (kept, cut)
}

/// One end of a bracket [`falsi`] closes: where, the function's value
/// there, and what the function made there.
type End<T> = (f32, f32, T);

/// The root of a function falling through a bracket — positive at
/// `low`, not positive at `high`, each an [`End`] — by the Illinois
/// regula falsi: each probe where the line through the bracket's ends
/// crosses zero, the value at an end kept twice running halved, so a
/// bend in the function cannot hold one end still. A probe that would
/// not fall strictly inside the bracket — an end that is not finite, a
/// line that crosses outside — is taken at the middle instead. It stops
/// at a probe within `tolerance` of zero, at a bracket no probe can
/// narrow (the middle is an end: no change) or no wider than `width` —
/// a function with a step in it, as a line's length is where a letter
/// crosses a corner, need never come within tolerance — or after `rounds` probes,
/// and returns the probe within tolerance, or else the low end: the
/// root approached from where the function is still positive.
///
/// `f` returns a value that is a number: the caller says which side a
/// value that is not one reads as.
///
/// Cost: `rounds` calls of `f` at most, and as few as one where the
/// function is near a line — what a line's length is in its gap, and
/// its centering in its start, on any path but a sharp bend.
fn falsi<T>(
    f: impl FnMut(f32) -> (f32, T),
    low: End<T>,
    high: End<T>,
    tolerance: f32,
    width: f32,
    rounds: usize,
    stop: &dyn Fn() -> bool,
) -> (f32, T) {
    let (x, made, _) = falsi_bracket(f, low, high, tolerance, width, rounds, stop);
    (x, made)
}

/// [`falsi`], and where its bracket's other end stood when it stopped
/// short of the root: `Some` of where the function was not positive,
/// past the low end it returns, when no probe came within `tolerance`;
/// `None` when one did (#309: a line short of its end, and the gap at
/// which it steps past it, [`Layout::reach_its_end`]).
///
/// Cost: [`falsi`]'s.
fn falsi_bracket<T>(
    mut f: impl FnMut(f32) -> (f32, T),
    mut low: End<T>,
    mut high: End<T>,
    tolerance: f32,
    width: f32,
    rounds: usize,
    stop: &dyn Fn() -> bool,
) -> (f32, T, Option<f32>) {
    // The values the line is drawn through: an end's own, halved while
    // the other end moves and it does not.
    let (mut at_low, mut at_high) = (low.1, high.1);
    let mut kept: Option<bool> = None;
    for _ in 0..rounds {
        // The frame's work budget spent: the bracket as it stands
        // (ruling R44.2).
        if stop() {
            break;
        }
        let mid = low.0.midpoint(high.0);
        if !(mid > low.0 && mid < high.0) || high.0 - low.0 <= width {
            break;
        }
        let crossing = low.0 + (high.0 - low.0) * at_low / (at_low - at_high);
        let x = if at_low.is_finite() && at_high.is_finite() && crossing > low.0 && crossing < high.0 {
            crossing
        } else {
            mid
        };
        let (value, made) = f(x);
        if value > 0.0 {
            let near = value <= tolerance;
            low = (x, value, made);
            at_low = value;
            if kept == Some(true) {
                at_high *= 0.5;
            }
            kept = Some(true);
            if near {
                return (low.0, low.2, None);
            }
        } else {
            let near = -value <= tolerance;
            high = (x, value, made);
            at_high = value;
            if kept == Some(false) {
                at_low *= 0.5;
            }
            kept = Some(false);
            if near {
                return (high.0, high.2, None);
            }
        }
    }
    (low.0, low.2, Some(high.0))
}

/// How near its part's end a line's last ink is laid, by arc length, in
/// points: [`solve_tracking`]'s tolerance, at most a millionth of a
/// point of tracking per gap on a line of a hundred.
const SHORT_TOLERANCE: f32 = 1e-4;

/// How near its part's end, in points, a line of `steps` gaps whose
/// part ends at `end` is laid: [`SHORT_TOLERANCE`] where the path's
/// floats resolve it ([`resolved`]), but no finer than the line's letters
/// are set — each to within the pairwise rule's tolerance, so the line's
/// length moves by up to that many times its gaps as its gap moves, and a
/// solve held finer chases that round after round (ruling R43).
fn line_tolerance(steps: usize, end: f32) -> f32 {
    resolved(SHORT_TOLERANCE, end).max(steps.max(1) as f32 * INK_TOLERANCE)
}

/// How narrow a line's tracking bracket closes to, in points of gap: a
/// tenth of [`GAP_TOLERANCE`], what the gap a line has is judged to.
const GAP_PRECISION: f32 = 0.1 * GAP_TOLERANCE;

/// The least gap at which `track` lays a line's last ink at its part's
/// end — [`Tracked::short`] no longer positive, the room it leaves
/// shrinking as the gap grows — and the line laid at it: 0 where the
/// line fills its part at no gap, and at most about `2^TRACK_DOUBLINGS`.
/// `steps` is how many gaps the line has, each letter's but the first and
/// each space's — what its length grows by per point of gap on a
/// straight path.
///
/// The line's length is its inks and spaces and `steps` times its gap,
/// bent only by the path under it: so the first probe is that line's
/// root, its room at no gap shared among its gaps, aimed a hair past it
/// so it brackets the root, and [`falsi`] closes the bracket from there
/// (ruling R38.3). `end` is where the part ends, what the tolerance on
/// the line's end is resolved against ([`resolved`]).
///
/// With `warm`, a gap near the root — the line's last lay's, moved by
/// its part's change in length (ruling R43) — that is probed first, and
/// then the gap the straight path's slope from it gives, aimed a hair
/// past the root, toward it: where the two bracket the root, [`falsi`]
/// closes it from there, and the line is laid in two or three tracks;
/// where they do not, the solve starts afresh.
///
/// With the gap and its line, the gap past it at which the line ends past
/// its end, where the solve closed its bracket short of the root
/// ([`falsi_bracket`]): the line's length steps there (#309,
/// [`Layout::reach_its_end`]).
///
/// Cost: one [`track`] at no gap, one per bracket widening (none on a
/// straight path), and [`falsi`]'s, at most [`TRACK_ROUNDS`]; two more
/// with `warm`, where they do not bracket the root.
fn solve_tracking(
    track: impl Fn(f32) -> Tracked,
    steps: usize,
    end: f32,
    warm: Option<f32>,
    stop: &dyn Fn() -> bool,
) -> (f32, Tracked, Option<f32>) {
    let per_gap = steps.max(1) as f32;
    // The bracket closes no finer than the line's precision spread over
    // its gaps: past it, a narrower bracket moves the line's end by less
    // than its letters are set to.
    let tolerance = line_tolerance(steps, end);
    let width = GAP_PRECISION.max(tolerance / per_gap);
    // A room that is not a number reads as past the end, and so does a
    // lay with a glyph that is not placed finitely: a gap past what an
    // `f32` can lay along the path.
    let short = |t: &Tracked| {
        if t.short.is_nan() || !t.glyphs.iter().all(|g| g.pos.is_finite()) {
            f32::NEG_INFINITY
        } else {
            t.short
        }
    };
    if let Some(g0) = warm.filter(|g| g.is_finite() && *g > 0.0) {
        let first = track(g0);
        let v0 = short(&first);
        if v0.abs() <= tolerance {
            return (g0, first, None);
        }
        // On along the straight path's slope, a hair past the root.
        let toward = g0 + v0 / per_gap;
        let g1 = if v0 > 0.0 {
            toward * (1.0 + 1e-3) + f32::MIN_POSITIVE
        } else {
            toward * (1.0 - 1e-3)
        };
        if g1.is_finite() && g1 > 0.0 {
            let second = track(g1);
            let v1 = short(&second);
            if v1.abs() <= tolerance {
                return (g1, second, None);
            }
            let bracket = match (v0 > 0.0, v1 > 0.0) {
                (true, false) => Some(((g0, v0, first), (g1, v1, second))),
                (false, true) => Some(((g1, v1, second), (g0, v0, first))),
                _ => None,
            };
            if let Some((low, high)) = bracket {
                return falsi_bracket(
                    |g| {
                        let t = track(g);
                        (short(&t), t)
                    },
                    low,
                    high,
                    tolerance,
                    width,
                    TRACK_ROUNDS,
                    stop,
                );
            }
        }
    }
    let at_zero = track(0.0);
    // No room even at no gap, or a room that is not a number: no gap.
    if at_zero.short.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return (0.0, at_zero, None);
    }
    let aim = |g: f32| g * (1.0 + 1e-3) + f32::MIN_POSITIVE;
    let at_zero_short = at_zero.short;
    let mut low = (0.0_f32, at_zero.short, at_zero);
    let mut g = aim(low.1 / per_gap);
    for round in 0..TRACK_DOUBLINGS {
        if !g.is_finite() || stop() {
            break;
        }
        let tracked = track(g);
        let value = short(&tracked);
        if value <= 0.0 {
            if -value <= tolerance {
                return (g, tracked, None);
            }
            return falsi_bracket(
                |g| {
                    let t = track(g);
                    (short(&t), t)
                },
                low,
                (g, value, tracked),
                tolerance,
                width,
                TRACK_ROUNDS,
                stop,
            );
        }
        if value <= tolerance {
            return (g, tracked, None);
        }
        // Still short with one gap as long as all the room the line had at
        // no gap, and no less short than at the probe before: no gap moves
        // its end there, where the path is too far out for an `f32` to set
        // its letters apart (seeds 2970 and 4284 of the structured flow
        // generator: a part 10³³ along its half, every letter at one
        // center). Widened on, the gap ran past every `f32`; it keeps its
        // last lay instead. A line its gap still moves goes on, however
        // little a gap moves its end: round a bend, outward, one gap moves
        // a line's end by less than the gap (ruling R51.3).
        if g > at_zero_short && value >= low.1 {
            return (g, tracked, None);
        }
        // Still short: on along the line through the last two probes,
        // at least an eighth further, at least twice as far once a few
        // probes have not reached it, at most four times.
        let fall = (low.1 - value) / (g - low.0);
        let line = if fall > 0.0 {
            g + value / fall
        } else {
            f32::INFINITY
        };
        let least = if round < 4 { 1.125 } else { 2.0 };
        let next = aim(line).clamp(g * least, g * 4.0);
        low = (g, value, tracked);
        g = next;
    }
    (low.0, low.2, None)
}

/// A line's letters' mean ink half size and their font's own gap — their
/// mean side bearings; `None` without a letter.
fn letters_of(clusters: &[Cluster]) -> Option<(Vec2, f32)> {
    let letters: Vec<(f32, Ink)> = clusters
        .iter()
        .filter(|c| !c.blank)
        .filter_map(|c| c.ink.map(|i| (c.advance, i)))
        .collect();
    if letters.is_empty() {
        return None;
    }
    let count = letters.len() as f32;
    let half = letters.iter().map(|(_, i)| (i.max - i.min) * 0.5).sum::<Vec2>() / count;
    let gap = letters.iter().map(|(a, i)| a - (i.max.x - i.min.x)).sum::<f32>() / count;
    Some((half, gap))
}

/// The most times a frame's lines are laid again at the offset their
/// last lay wanted, in one solve of the frame's band curve
/// ([`Layout::lay_clear`], #308). Each lay moves the letters along their
/// halves by a little of what the offset moved — the curve's length
/// changes by the offset's change times how far a half turns under its
/// line, and that is spread over the line's gaps — so the offset the
/// frame's inks want settles in a few.
const OFFSET_ROUNDS: usize = 4;

/// How near the offset a frame is laid at must be to the one its inks
/// want for the frame to keep it, in points ([`Layout::lay_clear`],
/// #308): a hundredth of a point, which its nearest ink may stand off the
/// outline, or into its gap, by no more.
const OFFSET_TOLERANCE: f32 = 1e-2;

/// The offset off the outline a frame's band curve wants, from how far
/// its inks reach toward the outline and away from it as they lie
/// (`reaches`, and `side`, the side ornament's as its last fill laid it;
/// #308, ruling B15.1): outward, the most any reaches toward it, so that
/// its nearest ink touches the outline, the frame's gap from what it
/// frames (the outline is the frame's offset by its inset and gap);
/// inward, the frame's ink band centered on the outline, as far past it
/// as short of it. Not a number with no ink.
fn offset_from(reaches: &[Reach], outward: bool, side: Option<SideWant>) -> f32 {
    let mut toward = reaches.iter().map(|r| r.toward).fold(f32::NEG_INFINITY, f32::max);
    let mut away = reaches.iter().map(|r| r.away).fold(f32::NEG_INFINITY, f32::max);
    if let Some(side) = side {
        toward = toward.max(side.toward);
        away = away.max(side.away);
    }
    if toward == f32::NEG_INFINITY {
        return f32::NAN;
    }
    if outward {
        toward
    } else {
        0.5 * (toward - away)
    }
}

/// The offset a frame's band curve settles at, and the frame laid there
/// (#308): where the offset its lay wants ([`offset_from`], by `wants`)
/// is the offset it was laid at, to `tolerance`. `lay` lays the frame at
/// an offset, given the lay before when there is one. The first lay is
/// at `start`, the second at the offset the first wants, and each after
/// where the line through the last two lays' misses — the offset wanted
/// less the offset laid at — crosses none (the secant): round a tight
/// bend the ink nearest the outline turns with its line's letters, and
/// the offset it wants swings past where it was laid by more than the
/// change, so laying the frame again at what the last wanted would
/// circle the answer and never reach it. [`OFFSET_ROUNDS`] lays at most,
/// fewer with `stop`; the lay whose miss is the least is kept. A miss
/// that is not a number — no ink — keeps the lay.
///
/// Cost: `lay`'s, once for each offset tried.
fn solve_offset<T: Clone>(
    start: f32,
    tolerance: f32,
    rounds: usize,
    mut lay: impl FnMut(f32, Option<&T>) -> T,
    wants: impl Fn(&T) -> f32,
    stop: &dyn Fn() -> bool,
) -> (f32, T) {
    let mut offset = start;
    let mut laid = lay(offset, None);
    let mut best: Option<(f32, f32, T)> = None;
    let mut before: Option<(f32, f32)> = None;
    for round in 1..=rounds {
        let wanted = wants(&laid);
        if !wanted.is_finite() {
            return (offset, laid);
        }
        let miss = wanted - offset;
        if best.as_ref().is_none_or(|(_, least, _)| miss.abs() < *least) {
            best = Some((offset, miss.abs(), laid.clone()));
        }
        if miss.abs() <= tolerance || round == rounds || stop() {
            break;
        }
        let next = match before {
            Some((was, missed)) if missed != miss => offset - miss * (offset - was) / (miss - missed),
            _ => wanted,
        };
        before = Some((offset, miss));
        offset = if next.is_finite() { next } else { wanted };
        laid = lay(offset, Some(&laid));
    }
    match best {
        Some((offset, _, laid)) => (offset, laid),
        None => (offset, laid),
    }
}

/// The walk a line is tracked along on `path`.
fn line_walk<'p>(path: &'p ArcPath, run: &ShapedRun, outward: bool) -> PathWalk<'p> {
    PathWalk {
        path,
        memo: None,
        work: None,
        line_height: run.line_height,
        outward,
        limit: f32::INFINITY,
        level: false,
        ornament: true,
        baseline: run.baseline(),
    }
}

/// The frame's side ornament (ruling R33.1): its run (`run` and one
/// shaped copy of the ornament at the size the x-height cap sets, a line
/// of its own) and the ornament's glyphs in it. `None` without sides,
/// or when the ornament shapes to nothing. Its shaping and inking are
/// counted in `work` (ruling R46.1).
fn side_run(
    run: &ShapedRun,
    sides: &Option<(String, Rgba)>,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    work: &Work,
) -> Option<(ShapedRun, Range<usize>)> {
    let (content, color) = sides.as_ref()?;
    let size = run
        .glyphs
        .iter()
        .filter(|g| g.span != FILLER_SPAN)
        .map(|g| g.size)
        .fold(0.0_f32, f32::max);
    if size.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return None;
    }
    let at = |text: &str, size: f32, fs: &mut FontSystemLease| {
        let style = ResolvedText {
            size: FontSize::Pt(size),
            ..*flow
        };
        let shaped = shape_run(
            &[FlowSpan {
                text,
                style: &style,
                link: None,
            }],
            1.0,
            fs,
        );
        #[cfg(test)]
        note_reshape();
        work.shaped(&shaped);
        work.inked(&shaped);
        let fsys = fs.fonts();
        inked(&shaped, fsys)
            .glyphs
            .iter()
            .filter_map(|g| g.ink.map(|i| i.max.y - i.min.y))
            .fold(0.0_f32, f32::max)
    };
    let x_height = at("x", size, fs);
    let tall = at(content, size, fs);
    let ornament = if tall > x_height && tall > 0.0 && x_height > 0.0 {
        size * x_height / tall
    } else {
        size
    };
    let style = ResolvedText {
        size: FontSize::Pt(ornament),
        color: *color,
        ..*flow
    };
    let fsys = fs.fonts();
    let with = reshape(run, 1.0, Some((content.as_str(), &style)), fsys);
    #[cfg(test)]
    note_reshape();
    work.shaped(&with);
    let start = with.glyphs.iter().position(|g| g.span == FILLER_SPAN)?;
    let range = start..with.glyphs.len();
    Some((with, range))
}

impl Layout<'_> {
    /// Line `p` cut at its end to the longest prefix of whole clusters
    /// that its half less `clear` holds at its font's own gap — its
    /// trailing spaces left out, at least its first cluster kept — or
    /// `None` when it holds the whole line (ruling R34: a `Once` frame
    /// never cuts text its half holds).
    ///
    /// Cost: a bisection of [`track`]s over the line's clusters.
    fn cut(&self, run: &ShapedRun, p: &Prepared, clear: [f32; 2]) -> Option<Prepared> {
        let kept = self.held(run, p.half, &p.clusters, p.natural_gap, clear)?;
        self.prepare_clusters(run, p.half, kept, true)
    }

    /// The longest prefix of whole `clusters` that half `half` less
    /// `clear` holds at gap `gap`, its trailing spaces left out and at
    /// least its first cluster kept: [`cut`](Self::cut)'s, before the
    /// line is prepared. `None` when it holds them all, or there is
    /// nothing to cut.
    fn held(
        &self,
        run: &ShapedRun,
        half: usize,
        clusters: &[Cluster],
        gap: f32,
        clear: [f32; 2],
    ) -> Option<Vec<Cluster>> {
        let (lo, hi) = self.room(half, clear);
        // Each prefix measured on the frame's band curve as it stands
        // (#308).
        let fits = |k: usize| {
            let kept = trimmed(&clusters[..k]);
            kept.is_empty() || self.reach(half, run, kept, lo, gap) <= hi - lo + resolved(SLIDE_TOLERANCE, hi)
        };
        let n = clusters.len();
        if fits(n) {
            return None;
        }
        // `fits` holds for no clusters and fails for all of them; the
        // longest prefix it holds for lies between.
        let (mut held, mut over) = (0, n);
        while over - held > 1 && !self.spent_all() {
            let mid = held + (over - held) / 2;
            if fits(mid) {
                held = mid;
            } else {
                over = mid;
            }
        }
        let kept = trimmed(&clusters[..held.max(1)]).to_vec();
        let kept = if kept.is_empty() {
            clusters[..1].to_vec()
        } else {
            kept
        };
        // Nothing left to cut: a line of one cluster its half cannot
        // hold is laid pressed.
        (kept.len() != n).then_some(kept)
    }
}

impl Layout<'_> {
    /// Line `i` of `lines` cut at its end to the longest prefix of whole
    /// clusters with which no two of the frame's inks touch, its
    /// clearance found again ([`lay_clear`](Self::lay_clear)) — `None`
    /// for the line when only leaving it out does so — or `None` when
    /// even leaving it out leaves two inks touching (ruling R38.1).
    ///
    /// The prefix is bisected on a cheap look (ruling R42.1): the frame
    /// with the line cut to it, laid once against the clearance `held`,
    /// and whether two of its inks touch. The clearance search runs once,
    /// on the prefix chosen; only where its inks touch even so is the
    /// prefix bisected again below it on whole searches.
    ///
    /// Cost: a bisection of [`lay`](Self::lay)s over the line's clusters
    /// and one clearance search; where that search's inks touch, a
    /// bisection of clearance searches below the prefix.
    fn cut_apart(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        i: usize,
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        held: Clear,
    ) -> Option<(Option<Prepared>, Cleared)> {
        let p = &lines[i];
        // The frame with line `i` cut to its first `k` clusters: the
        // line, `None` when it is left out, and the frame's lines.
        let frame_of = |k: usize| {
            let kept = trimmed(&p.clusters[..k]).to_vec();
            let line = if kept.is_empty() {
                None
            } else {
                self.prepare_clusters(run, p.half, kept, true)
            };
            let mut frame = lines.to_vec();
            match &line {
                Some(shorter) => frame[i] = shorter.clone(),
                None => {
                    frame.remove(i);
                }
            }
            (line, frame)
        };
        // That frame, its clearance found again, when no two of its inks
        // touch.
        let apart = |k: usize| {
            let (line, frame) = frame_of(k);
            let cleared = self.lay_clear(run, &frame, side, span, [[0.0; 2]; 2], BACKTRACKS);
            (!overlapping(&cleared.lines, &self.work)).then_some((line, cleared))
        };
        // That frame laid once against the clearance held: whether no two
        // of its inks touch.
        let looks_apart = |k: usize| {
            let (_, frame) = frame_of(k);
            !overlapping(
                &self.lay(run, &frame, held, span, &mut LayMemo::default()),
                &self.work,
            )
        };
        // Apart with none of the line's clusters, touching with all of
        // them; the longest prefix apart lies between.
        let (mut chosen, mut over) = (0, p.clusters.len());
        while over - chosen > 1 && !self.spent_all() {
            let mid = chosen + (over - chosen) / 2;
            if looks_apart(mid) {
                chosen = mid;
            } else {
                over = mid;
            }
        }
        if let Some(found) = apart(chosen) {
            return Some(found);
        }
        if chosen == 0 {
            return None;
        }
        // The look misled: the prefix is bisected below it on whole
        // clearance searches.
        let mut best = apart(0)?;
        let (mut held_k, mut over) = (0, chosen);
        while over - held_k > 1 && !self.spent_all() {
            let mid = held_k + (over - held_k) / 2;
            match apart(mid) {
                Some(found) => (held_k, best) = (mid, found),
                None => over = mid,
            }
        }
        Some(best)
    }
}

impl Layout<'_> {
    /// The frame's side ornament, `drawn`'s glyphs over `range`, laid round
    /// the junctions `lines` leave on the frame's band curve as it stands
    /// (ruling R33.1): each arc filled ([`SideArc::fill`]), its glyphs
    /// placed after the text's and within the glyph cap — an arc the frame
    /// cannot afford to lay and check left empty, or kept short, and an
    /// arc whose copies would touch the text, the copies already laid, or
    /// one another but for neighbors left empty, each reported. With what
    /// the copies kept want of the frame's offset (#308).
    ///
    /// Cost: each arc's fill and one touch check of it.
    fn lay_sides(&self, drawn: &ShapedRun, range: &Range<usize>, lines: &[Line]) -> LaidSides {
        let mut out = LaidSides {
            glyphs: Vec::new(),
            losses: Vec::new(),
            want: None,
            arcs: Vec::new(),
        };
        let sides = &mut out.glyphs;
        let text_len: usize = lines.iter().map(|l| l.tracked.glyphs.len()).sum();
        let side = SideOrnament::new(self, drawn, range.clone());
        let target = lines.iter().map(|l| l.gap).sum::<f32>() / lines.len().max(1) as f32;
        let text_boxes: Vec<InkBox> = lines
            .iter()
            .flat_map(|l| l.tracked.boxes.iter().copied())
            .collect();
        side.beside(&text_boxes);
        for (at, from, to) in side_arcs(lines, self.halves.length) {
            let budget = glyph_cap().saturating_sub(text_len + sides.len());
            let Some(arc) = side.arc(from, to) else {
                continue;
            };
            let before = sides.len();
            side.base.set(text_boxes.len().saturating_add(before));
            // An arc the frame cannot afford to lay and check is left
            // empty, or kept short, and reported (rulings R47.2, R48.2,
            // R50.5).
            let loss = arc.fill(target, budget, sides);
            if loss == Some(SideLoss::Empty) {
                out.losses
                    .extend((0..2).filter(|&j| at[j]).map(|j| (j, SideLoss::Empty)));
                continue;
            }
            // An arc whose copies would touch the text, the copies
            // already laid, or one another but for neighbors — round
            // an outline thinner than its lines, inward, where an arc
            // folds back across itself — is left empty, and reported
            // (ruling R35.2).
            let boxes: Vec<InkBox> = sides[before..]
                .iter()
                .filter_map(|g| glyph_box(drawn, g))
                .collect();
            let laid: Vec<InkBox> = sides[..before]
                .iter()
                .filter_map(|g| glyph_box(drawn, g))
                .collect();
            // Keyed by which they are — the text's, the copies already
            // laid, this arc's — and where among them.
            let keyed = text_boxes
                .iter()
                .map(|b| (0, *b))
                .chain(laid.iter().map(|b| (1, *b)))
                .enumerate()
                .map(|(i, (from, b))| ((from, i), b))
                .chain(boxes.iter().enumerate().map(|(i, b)| ((2, i), *b)))
                .collect();
            let touches = touching(
                keyed,
                |(a, i), (b, j)| {
                    if a == 2 && b == 2 {
                        i.abs_diff(j) <= 1
                    } else {
                        a != 2 && b != 2
                    }
                },
                &self.work,
            );
            let looks = self.work.looks.get();
            self.work.looks.set(Looks {
                classes: looks.classes.max(self.work.last_classes.get()),
                ..looks
            });
            if touches {
                sides.truncate(before);
                out.losses
                    .extend((0..2).filter(|&j| at[j]).map(|j| (j, SideLoss::Empty)));
                continue;
            }
            if let Some(loss) = loss {
                out.losses.extend((0..2).filter(|&j| at[j]).map(|j| (j, loss)));
            }
            if let Some(reached) = arc.reached.get().filter(|_| sides.len() > before) {
                out.want = Some(out.want.map_or(reached, |want| want.with(reached)));
                out.arcs.push((at, before..sides.len(), reached));
            }
        }
        out
    }
}

/// A frame's side ornament as laid ([`Layout::lay_sides`]): its glyphs,
/// the arcs it lost (by junction), and what its copies want of the
/// frame's offset (#308).
struct LaidSides {
    glyphs: Vec<PlacedGlyph>,
    losses: Vec<(usize, SideLoss)>,
    want: Option<SideWant>,
    /// Each arc that kept copies: the junctions it lies round, its
    /// glyphs among [`glyphs`](Self::glyphs), and what its copies want of
    /// the frame's offset.
    arcs: Vec<([bool; 2], Range<usize>, SideWant)>,
}

/// A frame's text laid at one size ([`place_halves`]).
struct Built {
    /// The offset of the frame's band curve its lines are laid at (#308).
    offset: f32,
    /// The run as drawn, inked.
    drawn: ShapedRun,
    /// The side ornament's glyphs in it.
    side: Option<Range<usize>>,
    prepared: Vec<Prepared>,
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction is a break ([`Layout::met`]).
    met: [bool; 2],
    /// Whether two of its inks touch ([`overlapping`]).
    overlap: bool,
}

/// What a frame's `lines` leave unheld (ruling R35.2), with whether
/// each junction is a break and whether two inks touch: a line its half
/// less the clearance cannot hold at its font's gap, a junction left
/// short of a break, two inks touching.
fn unheld_of(lines: &[Line], met: [bool; 2], overlap: bool) -> Unheld {
    Unheld {
        pressed: lines.iter().filter(|l| l.pressed).map(|l| l.half).collect(),
        junctions: [0, 1].into_iter().filter(|&j| !met[j]).collect(),
        overlap,
        sides: Vec::new(),
    }
}

impl Built {
    /// Whether its halves hold it: nothing left unheld (ruling R35.2).
    fn holds(&self) -> bool {
        unheld_of(&self.lines, self.met, self.overlap).is_empty()
    }

    /// Its clearance as a seed for the text at a larger size
    /// ([`SEED_SHARE`] of it).
    fn seed(&self) -> Clear {
        self.clear.map(|half| half.map(|c| c * SEED_SHARE))
    }
}

/// The ink box on the canvas of `glyph`, placed from `run` (inked):
/// `None` for one with no ink, a space.
fn glyph_box(run: &ShapedRun, glyph: &PlacedGlyph) -> Option<InkBox> {
    let ink = run.glyphs.get(glyph.glyph as usize)?.ink?;
    let pen = glyph.pos + Vec2::new(0.0, run.baseline());
    Some((pen + ink.min, pen + ink.max))
}

/// `clusters` without their trailing spaces.
fn trimmed(clusters: &[Cluster]) -> &[Cluster] {
    let end = clusters.iter().rposition(|c| !c.blank).map_or(0, |i| i + 1);
    &clusters[..end]
}

/// `clusters` cut to the glyphs `left` still holds, at a cluster — a
/// cluster's glyphs are placed together or not at all — its end's blanks
/// left out with what follows them, and `left` less what is kept: a
/// frame's text held to [`MAX_PLACED_GLYPHS`] (ruling R44.1). Returns
/// what is kept, and whether anything was cut.
fn to_cap(mut clusters: Vec<Cluster>, left: &mut usize) -> (Vec<Cluster>, bool) {
    let mut placed = 0;
    let fits = clusters
        .iter()
        .take_while(|c| {
            placed += c.glyphs.len();
            placed <= *left
        })
        .count();
    let cut = fits < clusters.len();
    if cut {
        clusters.truncate(fits);
        let end = trimmed(&clusters).len();
        clusters.truncate(end);
    }
    *left -= clusters.iter().map(|c| c.glyphs.len()).sum::<usize>();
    (clusters, cut)
}

/// Test seam (#294, ruling R44.1): `run`'s glyphs `range` cut to `left`
/// glyphs at a cluster ([`to_cap`]): the glyphs kept, as a range of the
/// run's, whether it was cut, and the glyphs left after.
#[cfg(test)]
pub(crate) fn to_cap_seam(run: &ShapedRun, range: Range<usize>, left: usize) -> (Range<usize>, bool, usize) {
    let mut left = left;
    let (kept, cut) = to_cap(clusters_of(&run.glyphs, range.clone()), &mut left);
    let end = kept.last().map_or(range.start, |c| c.glyphs.end);
    (range.start..end, cut, left)
}

/// Place `run` as a text frame around `outline` (rulings R27, R32, R33,
/// R33.1, R34): its two lines laid at `layout.span` along the diagonal
/// halves, each junction a break, `layout.sides` closing what the lines
/// leave. The placement lists the first half's line's glyphs, then the
/// second's, in text order, then the side ornament's copies. Returns the
/// run the placement refers to when it is not `run` — the rescaled run
/// for `Fit`, with the side ornament as a line of its own — and the
/// placement.
///
/// R27 parts the text and sizes a `Fit` frame ([`place_lines`]); its
/// lines, cut where R27 cut them at `min`, are laid here. A `Once` or
/// `Spread` frame is laid at its own size, its lines parted as R27 parts
/// them ([`line_partition`]) but never cut to R27's level span: a line
/// is cut at its end only where its half less the clearance cannot hold
/// it at its font's own gap (ruling R34), and the placement then
/// reports [`PlacedRun::truncated`]. A `Once` filler is not drawn: the
/// resolver gives it to the frame as its `sides` when `sides` is unset
/// ([`resolve_ring`](crate::document::resolve::resolve_ring)).
///
/// Cost: [`place_lines`]; per line a few [`track`]s to center it
/// ([`falsi`]) and a few per lay ([`solve_tracking`]), each O(its
/// glyphs); the junction clearance [`CLEAR_ROUNDS`] rounds at most, at
/// whole and again below it, each laying again only the lines whose
/// part it moved, with a one-copy fit per side arc; a shrink
/// [`SHRINK_ROUNDS`] probes, each one [`track`] a line and, where they
/// fit, a clearance search; a cut a bisection of [`track`]s more and the
/// clearance again, [`CUT_ROUNDS`] times at most; a cut apart a
/// bisection of clearance searches per line; a side arc a lay of its
/// copies at the floor, and a few lays per copy count weighed
/// ([`falsi`]); every touch check a sort and its looks, a sweep's few a
/// box and past it the inks near each box ([`touch_scan`]); a build's
/// shaping and inking. In all, at most the frame's budget of glyph steps
/// ([`frame_budget`]: [`STEPS_PER_GLYPH`] a glyph of its text cut to
/// [`MAX_PLACED_GLYPHS`], at least [`TRACK_BUDGET`]), every kind of work
/// counted at its weight (ruling R46.1) and the budget read before every
/// lay, build, reshape and side copy, and [`overshoot_steps`] past it at
/// most (ruling R47.2);
/// where two inks still touch, a bisection of overlap checks over its
/// inks ([`untouched`]) runs within the share of the budget held back
/// for it.
#[allow(clippy::too_many_arguments)] // `place_on_closed`'s, and the trace.
pub(super) fn place_halves(
    run: &ShapedRun,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let span = layout.span.get();
    let sized = matches!(fit, FitMode::Fit { .. });
    // The frame's work, all of it counted (rulings R44.2, R46.1): its
    // budget scaled by its text, cut to the glyph cap, until R27 has
    // parted it, then by the lines it parted.
    let work = Work::new(run.glyphs.len().min(glyph_cap()));
    // The outline's two paths and their level spans, built once for the
    // frame: R27 sizes and parts its text on them, and its lines are laid
    // along them (#309).
    let level = frame_halves(outline, table_spacing(run, fit, flow));
    // Their tables, counted as they are built (ruling B16.3a): before the
    // budget is first read, so within what the frame may take past it
    // ([`overshoot_steps`]).
    work.tabled(level.iter().map(|(path, _)| path));
    // R27's placement, laid in full only for a frame that keeps it.
    let keep_r27 = |trace: Option<&mut Trace>,
                    fs: &mut FontSystemLease,
                    work: &Work,
                    level: &[(ArcPath, Range<f32>); 2]| {
        let mut natural_trace = Trace::default();
        let placed = place_lines_in(run, level, fit, outward, flow, fs, Some(&mut natural_trace), work);
        if let Some(trace) = trace {
            *trace = natural_trace;
            trace.spent = work.spent.get();
            trace.tables = work.tables();
            trace.budget = work.budget.get();
            trace.reserve = work.reserve();
            trace.glyphs = work.glyphs.get();
            trace.looks = work.looks.get();
        }
        placed
    };
    // What a `Fit` frame reads of R27: its size, its parting and its cuts
    // (ruling R43: not where it centers each line, which the halves lay
    // again). A `Once` or `Spread` frame parts its lines itself, below.
    let mut natural_trace = Trace::default();
    let (fitted, natural) = if sized {
        frame_lines(
            run,
            &level,
            fit,
            outward,
            flow,
            fs,
            Some(&mut natural_trace),
            &work,
        )
    } else {
        (
            None,
            PlacedRun {
                glyphs: Vec::new(),
                truncated: false,
            },
        )
    };
    if sized && natural_trace.lines.is_empty() {
        return keep_r27(trace, fs, &work, &level);
    }
    let halves = Halves::new(outline, level);
    // An outline whose halves cannot be measured — one with a side that
    // is not a number — keeps R27's placement, which holds every
    // position finite there. (`place_on_closed` shrinks an outline too
    // large for its length to be an `f32` before it reaches here.)
    let ends = [
        halves.junctions[0],
        halves.junctions[1],
        (halves.apexes[0], halves.apexes[1]),
    ];
    if !(halves.length.is_finite() && ends.iter().all(|(a, b)| a.is_finite() && b.is_finite())) {
        return keep_r27(trace, fs, &work, &halves.level);
    }
    let lay = Layout::new(&halves, outward, work, Vec::new());

    // The run the lines are laid from, and each line: its half, its
    // glyphs, and whether R27 cut it.
    let (base, specs): (ShapedRun, Vec<(usize, Range<usize>, bool)>) = if sized {
        let base = fitted.clone().unwrap_or_else(|| run.clone());
        let specs = natural_trace
            .lines
            .iter()
            .filter_map(|line| {
                let ids = natural.glyphs[line.glyphs.clone()]
                    .iter()
                    .map(|g| g.glyph as usize);
                Some((line.half, ids.clone().min()?..ids.max()? + 1, line.cut))
            })
            .collect();
        (base, specs)
    } else {
        lay.work.walked(run.glyphs.len());
        let specs = line_partition(run, &halves.level)
            .into_iter()
            .enumerate()
            .filter(|(_, range)| !range.is_empty())
            .map(|(k, range)| (k, range, false))
            .collect();
        (run.clone(), specs)
    };

    // The text at `scale` times its size: the run as drawn — the text,
    // and the side ornament on a line of its own — its lines, laid with
    // their clearance grown from `seed`. With `cheap`, `None` when a line
    // does not fit its half less the seed at its font's gap (one `track`
    // a line), before any clearance is grown: what a shrink's probe
    // looks at first (ruling R35.3).
    let build = |scale: f32, seed: Clear, cheap: bool, fs: &mut FontSystemLease| {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        let scaled = if scale == 1.0 {
            base.clone()
        } else {
            let fsys = fs.fonts();
            let scaled = reshape(&base, scale, None, fsys);
            #[cfg(test)]
            note_reshape();
            lay.work.shaped(&scaled);
            scaled
        };
        let (drawn, side) = match side_run(&scaled, &layout.sides, flow, fs, &lay.work) {
            Some((with, range)) => (with, Some(range)),
            None => (scaled, None),
        };
        lay.work.inked(&drawn);
        lay.work.take(band_steps(&drawn));
        let fsys = fs.fonts();
        // Each span's band center, what every glyph's anchor on the
        // frame's band curve is read from (#308, ruling B15.1).
        let (drawn, centers) = inked_with_bands(&drawn, fsys);
        lay.centers.replace(centers.into());
        lay.tracked.borrow_mut().clear();
        // `untouched`'s share, for the classes of height the text's inks
        // hold (ruling R46.1).
        lay.work.hold_for(height_classes(
            drawn.glyphs.iter().filter_map(|g| g.ink.map(|i| i.max - i.min)),
        ));
        // The glyphs the text may still place (ruling R44.1): a frame's
        // text and its sides together place at most `MAX_PLACED_GLYPHS`,
        // so a text longer is cut at a cluster, its end left out.
        let mut left = glyph_cap();
        let parted: Vec<(usize, Vec<Cluster>, bool)> = specs
            .iter()
            .map(|(k, range, cut)| {
                let (clusters, capped) = to_cap(clusters_of(&drawn.glyphs, range.clone()), &mut left);
                (*k, clusters, *cut || capped)
            })
            .collect();
        // The frame's band curve, first at the offset its inks would want
        // each set at its half's apex (#308): what its lines are prepared
        // on, and its offset solved from.
        lay.offset
            .set(lay.estimate(&drawn, parted.iter().map(|(k, c, _)| (*k, c.as_slice()))));
        let prepared: Vec<Prepared> = parted
            .into_iter()
            .filter_map(|(k, clusters, cut)| {
                // A `Once` or `Spread` line its whole half cannot hold at
                // its font's gap is cut to what that holds before it is
                // prepared or any clearance is grown: a clearance only
                // narrows its room, so the cut would take that much and
                // more in the end (ruling R34), and a line centered and
                // laid whole round after round is work spent on text that
                // is never drawn (ruling R38.3).
                let kept = (!sized)
                    .then(|| letters_of(&clusters))
                    .flatten()
                    .and_then(|(_, gap)| lay.held(&drawn, k, &clusters, gap, [0.0; 2]));
                match kept {
                    Some(kept) => lay.prepare_clusters(&drawn, k, kept, true),
                    None => lay.prepare_clusters(&drawn, k, clusters, cut),
                }
            })
            .collect();
        let too_long = |p: &Prepared| {
            let (lo, hi) = lay.room(p.half, seed[p.half]);
            p.clusters.iter().filter(|c| !c.blank).count() >= 2
                && !lay
                    .reach(p.half, &drawn, &p.clusters, lo, p.natural_gap)
                    .le(&(hi - lo + resolved(SLIDE_TOLERANCE, hi)))
        };
        if cheap && prepared.iter().any(too_long) {
            return None;
        }
        let backtracks = if cheap { PROBE_BACKTRACKS } else { BACKTRACKS };
        // Its lines laid and their clearance grown read the inked run and
        // no font: with the font system's lock released (#308).
        let cleared = fs.unlocked(|| {
            lay.lay_clear(
                &drawn,
                &prepared,
                side.as_ref().map(|r| (&drawn, r)),
                span,
                seed,
                backtracks,
            )
        });
        let overlap = overlapping(&cleared.lines, &lay.work);
        Some(Built {
            offset: lay.offset.get(),
            drawn,
            side,
            prepared,
            lines: cleared.lines,
            clear: cleared.clear,
            met: cleared.met,
            overlap,
        })
    };
    // The frame's budget scales with its text, cut to the glyph cap
    // (rulings R44.1, R44.2).
    let glyphs = specs
        .iter()
        .map(|(_, range, _)| range.len())
        .sum::<usize>()
        .min(glyph_cap());
    lay.work.scale(glyphs);
    let zero: Clear = [[0.0; 2]; 2];
    let Some(mut built) = build(1.0, zero, false, fs) else {
        unreachable!("only a cheap build is refused")
    };
    let mut scale = 1.0_f32;
    // A `Fit` frame that its halves do not hold at R27's size — a line
    // its half less the clearance cannot hold at its font's gap, a
    // junction left short of a break, two inks touching: a flat outline,
    // its half barely longer than the level span R27 sized the text for —
    // is shrunk, as the last resort, to the largest size within its
    // bounds at which they hold (§6.2 step 6, rulings R34 and R35.2).
    // Each probe looks first at whether its lines fit their halves less
    // a clearance seeded from the last size held, and grows that
    // clearance only if they do (ruling R35.3).
    if let (false, false, FitMode::Fit { min, .. }) = (built.holds(), lay.spent_all(), fit) {
        let fitted_size = base.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let declared = run.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let floor = if fitted_size > 0.0 && declared > 0.0 {
            (min * declared / fitted_size).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let Some(at_floor) = build(floor, zero, false, fs) else {
            unreachable!("only a cheap build is refused")
        };
        if at_floor.holds() {
            let (mut held, mut over) = (floor, 1.0_f32);
            let mut best = at_floor;
            for _ in 0..SHRINK_ROUNDS {
                // Sub-percent precision in a last resort buys nothing a
                // reader can see (ruling R42.2): the bracket within 1% of
                // the scale held, or a size step under a quarter point,
                // ends the shrink.
                let step = over - held;
                let precise = step <= SHRINK_PRECISION * held || step * fitted_size < SHRINK_LEAST_STEP_PT;
                if lay.spent_all() || precise {
                    break;
                }
                let mid = 0.5 * (held + over);
                match build(mid, best.seed(), true, fs) {
                    Some(probe) if probe.holds() => {
                        (held, best) = (mid, probe);
                    }
                    _ => over = mid,
                }
            }
            (built, scale) = (best, held);
        } else {
            (built, scale) = (at_floor, floor);
        }
    }
    let Built {
        offset,
        drawn,
        side,
        prepared,
        lines,
        clear,
        met,
        overlap,
    } = built;
    // The frame's band curve where the size kept laid it, a later probe's
    // laid elsewhere (#308); and none of a later probe's lines, which
    // were laid from another run (#309).
    lay.offset.set(offset);
    lay.tracked.borrow_mut().clear();
    let side_of = side.as_ref().map(|range| (&drawn, range));

    // The frame's last resorts, from its lines as laid with `clear` kept
    // at each junction and `side_of` the ornament its junctions make room
    // for: what it keeps, and whether it cut its text.
    let resorts = |mut prepared: Vec<Prepared>,
                   mut lines: Vec<Line>,
                   mut clear: Clear,
                   mut met: [bool; 2],
                   mut overlap: bool,
                   side_of: Option<(&ShapedRun, &Range<usize>)>| {
        // A frame its halves hold even so by nothing is cut (rulings R34,
        // R35.2): a line its half less the clearance cannot hold at its
        // font's gap — a `Once` or `Spread` line at its size, a `Fit` line at
        // its least — to what it can. A line its half holds is never cut
        // (ruling R34): a junction short of a break, or two inks touching,
        // where no line is pressed, is reported in the trace instead.
        let mut truncated = (natural.truncated && sized) || prepared.iter().any(|p| p.cut);
        for _ in 0..CUT_ROUNDS {
            let holds = !lines.iter().any(|l| l.pressed) && met.iter().all(|&m| m) && !overlap;
            if holds || lay.spent_all() {
                break;
            }
            let mut changed = false;
            for (p, line) in prepared.iter_mut().zip(&lines) {
                if !line.pressed {
                    continue;
                }
                if let Some(shorter) = lay.cut(&drawn, p, clear[p.half]) {
                    *p = shorter;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
            truncated = true;
            // The clearance grown again from a share of the last: a cut
            // line, stretched wider, wants as much or more (ruling R38.3).
            let seed = clear.map(|half| half.map(|c| c * SEED_SHARE));
            let cleared = lay.lay_clear(&drawn, &prepared, side_of, span, seed, BACKTRACKS);
            (lines, clear, met) = (cleared.lines, cleared.clear, cleared.met);
            overlap = overlapping(&lines, &lay.work);
        }
        // Two inks that still touch, every line its half holds: an outline
        // too thin to hold the lines apart — inward round a flat outline,
        // the two lines facing across it. The line that cannot be held
        // beside the others is cut, the last first, to the longest prefix
        // that touches nothing, or left out (ruling R38.1); a `Once` frame
        // is not shrunk.
        let mut cut_apart = Vec::new();
        for i in (0..prepared.len()).rev() {
            if !overlap || lay.spent_all() {
                break;
            }
            if let Some((kept, cleared)) = lay.cut_apart(&drawn, &prepared, i, side_of, span, clear) {
                cut_apart.push(prepared[i].half);
                match kept {
                    Some(p) => prepared[i] = p,
                    None => {
                        prepared.remove(i);
                    }
                }
                (lines, met) = (cleared.lines, cleared.met);
                overlap = false;
                truncated = true;
            }
        }
        // A frame whose budget was spent while two of its inks still touch is
        // cut to the inks that touch nothing (rulings R38.1, R44.2): never
        // overlap wins over never cut, and a spent budget is no exception.
        if overlap {
            let (kept, halves_cut) = untouched(&lines, &lay.work);
            lines = kept;
            prepared.truncate(lines.len());
            cut_apart.extend(halves_cut);
            met = [0, 1].map(|j| lay.met(&lines, side_of, j).0);
            overlap = overlapping(&lines, &lay.work);
            truncated = true;
        }
        (prepared, lines, clear, met, overlap, truncated, cut_apart)
    };
    let bare_prepared = prepared.clone();
    // The last resorts, the frame laid again without its sides, and its
    // sides' fill read the inked runs and no font: each runs with the
    // font system's lock released (#308).
    let (mut prepared, mut lines, mut clear, mut met, overlap, truncated, cut_apart) =
        fs.unlocked(|| resorts(prepared, lines, clear, met, overlap, side_of));
    // A frame loses its sides before its text (rulings R48.3, B15.5):
    // where the frame its sides make room for is cut, it is laid again
    // with no room made for them, and where that keeps more of its text,
    // the sides are left out — both junctions empty, and reported — and the
    // frame keeps that text. A side copy's room grown within the lines'
    // slack presses no line, but draws the lines in, and inward round an
    // outline too thin for both, lines drawn in touch the more, and the
    // one cut apart keeps the less (#308, review 5801763546: the
    // Ecclesiastes line inward round the 600 × 12 rectangle, 29 glyphs of
    // its text beside 40-dot sides, 30 without).
    let kept_text = |lines: &[Line]| lines.iter().map(|l| l.tracked.glyphs.len()).sum::<usize>();
    let mut sides_left_out = false;
    let (overlap, truncated, cut_apart) = if side_of.is_some() && truncated && !lay.spent_all() {
        let held = lay.sides.replace(None);
        let sided_offset = lay.offset.get();
        let zero: Clear = [[0.0; 2]; 2];
        let bare = fs.unlocked(|| {
            let cleared = lay.lay_clear(&drawn, &bare_prepared, None, span, zero, BACKTRACKS);
            let bare_overlap = overlapping(&cleared.lines, &lay.work);
            resorts(
                bare_prepared,
                cleared.lines,
                cleared.clear,
                cleared.met,
                bare_overlap,
                None,
            )
        });
        if kept_text(&bare.1) > kept_text(&lines) {
            sides_left_out = true;
            (prepared, lines, clear, met) = (bare.0, bare.1, bare.2, bare.3);
            (bare.4, bare.5, bare.6)
        } else {
            lay.offset.set(sided_offset);
            lay.sides.replace(held);
            (overlap, truncated, cut_apart)
        }
    } else {
        (overlap, truncated, cut_apart)
    };
    // A line the frame keeps short of its part's end is brought there by
    // its secondary rule (#309), unless that loses the frame what it held:
    // a frame whose lines all end at their ends is laid as it was.
    if lines.iter().any(Line::short_of_its_end) && !lay.spent_all() {
        (lines, met) = fs.unlocked(|| lay.reach_ends(&drawn, &prepared, side_of, (lines, met), overlap));
    }
    let mut unheld = unheld_of(&lines, met, overlap);

    // The side ornament round the junctions, and its share of the frame's
    // offset (#308, ruling B15.1): every ink of the frame, its sides'
    // too, clears the outline at the offset its band curve lies at, or,
    // inward, is centered on it. The sides are laid on the curve the
    // lines settled, and where their copies want it elsewhere the lines
    // are laid again at the offset the frame's inks want with the
    // copies' share as laid ([`Layout::sides`]), the clearance grown on
    // from where it stood, and the sides laid again, [`OFFSET_ROUNDS`]
    // times at most — unless that would press a line the curve before did
    // not, or leave two of the lines' inks touching, where the frame
    // keeps the curve before. Outward, an arc whose copies the curve kept
    // leaves wanting more of it than it is, their inks across the
    // outline, is left empty and reported: a frame loses its sides before
    // the outline its text is set off (ruling R48.3, review 5801763546).
    let tolerance = resolved(OFFSET_TOLERANCE, halves.length);
    #[cfg(test)]
    let relay_rounds = RELAY_ROUNDS_OVERRIDE.with(Cell::get).unwrap_or(OFFSET_ROUNDS);
    #[cfg(not(test))]
    let relay_rounds = OFFSET_ROUNDS;
    let mut laid_sides = side
        .as_ref()
        .filter(|_| !sides_left_out)
        .map(|range| fs.unlocked(|| lay.lay_sides(&drawn, range, &lines)));
    // Left out, the sides lose both junctions' copies, whatever arcs the
    // lines leave: a loss is keyed by junction, as `lay_sides` keys it,
    // and one line's one arc runs round both (review 5805412034).
    if sides_left_out {
        unheld.sides.extend((0..2).map(|j| (j, SideLoss::Empty)));
    }
    for _ in 0..relay_rounds {
        let Some(want) = laid_sides.as_ref().map(|laid| laid.want) else {
            break;
        };
        let offset = lay.offset.get();
        let reaches: Vec<Reach> = lines
            .iter()
            .flat_map(|l| l.tracked.reaches.iter().copied())
            .collect();
        lay.work.walked(reaches.len());
        let wanted = offset_from(&reaches, outward, want);
        if (wanted - offset).abs().partial_cmp(&tolerance) != Some(Ordering::Greater) || lay.spent_all() {
            break;
        }
        let kept = lay.sides.replace(want);
        let cleared = fs.unlocked(|| lay.lay_clear(&drawn, &prepared, side_of, span, clear, BACKTRACKS));
        let pressed = lines.iter().any(|l| l.pressed);
        if (!pressed && cleared.lines.iter().any(|l| l.pressed)) || overlapping(&cleared.lines, &lay.work) {
            lay.offset.set(offset);
            lay.sides.replace(kept);
            break;
        }
        (lines, clear, met) = (cleared.lines, cleared.clear, cleared.met);
        // The relay lays the lines again, so a line it leaves short of its
        // end is brought there as the lines before it were (review
        // 5810838863), before the sides are laid round them.
        if lines.iter().any(Line::short_of_its_end) && !lay.spent_all() {
            (lines, met) = fs.unlocked(|| lay.reach_ends(&drawn, &prepared, side_of, (lines, met), overlap));
        }
        unheld = unheld_of(&lines, met, overlap);
        laid_sides = side
            .as_ref()
            .map(|range| fs.unlocked(|| lay.lay_sides(&drawn, range, &lines)));
    }

    let mut text = Vec::new();
    let mut line_ranges = Vec::new();
    for line in &lines {
        line_ranges.push(text.len()..text.len() + line.tracked.glyphs.len());
        text.extend(line.tracked.glyphs.iter().copied());
    }
    let mut sides = Vec::new();
    if let Some(laid) = laid_sides {
        let offset = lay.offset.get();
        let mut kept = vec![true; laid.glyphs.len()];
        let mut emptied = [false; 2];
        for (at, glyphs, want) in &laid.arcs {
            if outward && want.toward > offset + tolerance {
                for k in glyphs.clone() {
                    kept[k] = false;
                }
                for j in 0..2 {
                    emptied[j] |= at[j];
                }
                #[cfg(test)]
                ARCS_OFF_THE_CURVE.with(|n| n.set(n.get() + 1));
            }
        }
        sides = laid
            .glyphs
            .into_iter()
            .zip(kept)
            .filter_map(|(g, keep)| keep.then_some(g))
            .collect();
        unheld
            .sides
            .extend(laid.losses.into_iter().filter(|(j, _)| !emptied[*j]));
        unheld
            .sides
            .extend((0..2).filter(|&j| emptied[j]).map(|j| (j, SideLoss::Empty)));
    }

    if let Some(trace) = trace {
        *trace = Trace::default();
        trace.lines = lines
            .iter()
            .zip(&prepared)
            .zip(&line_ranges)
            .map(|((line, p), glyphs)| TracedLine {
                half: line.half,
                glyphs: glyphs.clone(),
                from: line.part.0,
                to: line.part.1,
                cut: p.cut,
            })
            .collect();
        trace.offset = Some(lay.offset.get());
        trace.short = lines
            .iter()
            .filter(|l| l.short_of_its_end())
            .map(|l| (l.half, l.tracked.short))
            .collect();
        #[cfg(test)]
        ONE_GAP_LINES.with(|tally| {
            let (mut laid, mut short) = tally.get();
            for l in lines
                .iter()
                .filter(|l| l.steps == 1 && l.part.1.abs() < 1e15 && !l.pressed)
            {
                laid += 1;
                short += usize::from(l.tracked.short.abs() > line_tolerance(l.steps, l.part.1));
            }
            tally.set((laid, short));
        });
        trace.unheld = unheld;
        trace.cut_apart = cut_apart;
        trace.clear = clear;
        trace.spent = lay.work.spent.get();
        trace.tables = lay.work.tables();
        trace.budget = lay.work.budget.get();
        trace.reserve = lay.work.reserve();
        trace.glyphs = lay.work.glyphs.get();
        trace.looks = lay.work.looks.get();
    }
    let changed = (sized && fitted.is_some()) || side.is_some() || scale != 1.0;
    let mut glyphs = text;
    glyphs.extend(sides);
    let placed = PlacedRun { glyphs, truncated };
    (changed.then(|| uninked(drawn)), placed)
}

/// What [`pair_seam`] finds for one letter (#309, ruling B16.1b).
pub(crate) struct PairSeam {
    /// Whether the ink it is set after lies where the band curve has one
    /// form the model reads — a side, an arc, a sharp corner's sweep — so
    /// that the model's root is exact, to float error: the letter set at
    /// what is owed, read back from the path.
    pub(crate) exact: bool,
    /// Whether that form is a side's, away from any corner: where the gap
    /// along the band curve is convex in the center, so it has one root on
    /// from the ink.
    pub(crate) side: bool,
    /// The model's center, and the gap the letter set there is at
    /// ([`osculating`]): `None` where it has none. Every gap here is
    /// signed: where the boxes reach into one another, how far, as a
    /// negative.
    pub(crate) model: Option<(f32, f32)>,
    /// The center [`pair`] sets it at from the straight path's warm start,
    /// the iterative rule's answer, and its gap: not the model's oracle,
    /// since it can be another right center in its window or a later root
    /// (review 5811563882; `check_pair_model` says how), and read only to
    /// count a property's holds on a side.
    pub(crate) iterative: (f32, f32),
    /// Whether the model's forms tell a gap from none there
    /// ([`resolution_at`]): where they do not, it sets nothing. What it
    /// aims at is not told, so that a property holds the gap to bounds of
    /// its own (review 5810838863; TEST_CONVENTIONS §T13.6).
    pub(crate) told: bool,
    /// The band curve between the last ink and the further of the model's
    /// center and the rule's: [`CURVE_SAMPLES`] `- 1` centers evenly
    /// between, each with the gap a letter set there is at: the path's own
    /// gap function, which a property holds the model to, that no root
    /// lies before the model's (review 5810838863), to the samples'
    /// spacing. Empty where neither center lies past the last ink's.
    pub(crate) curve: Vec<(f32, f32)>,
}

/// How finely [`PairSeam::curve`] samples the band curve: a letter's
/// gap at [`CURVE_SAMPLES`] `- 1` centers between the last ink's and the
/// further root. A crossing narrower than their spacing, a 64th of that
/// span — some 0.3 pt on a 20 pt span — can lie between two samples
/// unseen (review 5811563882).
const CURVE_SAMPLES: usize = 64;

/// Test seam (#309, ruling B16.1b): the closed-form layer's root, beside
/// the path's own gap function round it, its oracle ([`PairSeam::curve`]),
/// and the iterative rule's answer ([`PairSeam::iterative`]), for a letter
/// of half size `half`, its center `rise` below its anchor, owed `owed`
/// from an ink of half size `before` set at `last` along half `k` of
/// `outline`, an ink of that size set at `earlier` before it, every ink on
/// the band curve at `offset`.
#[allow(clippy::too_many_arguments)] // The outline, the curve, two inks and the letter.
pub(crate) fn pair_seam(
    outline: &Outline,
    k: usize,
    offset: f32,
    (earlier, last): (f32, f32),
    (before, half): (Vec2, Vec2),
    rise: f32,
    owed: f32,
) -> PairSeam {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let walk = PathWalk {
        path: halves.path(k),
        memo: None,
        work: None,
        line_height: 0.0,
        outward: true,
        limit: f32::INFINITY,
        level: false,
        ornament: true,
        baseline: 0.0,
    };
    let band = Band {
        offset,
        centers: Rc::from(Vec::new()),
    };
    let lay = InkLay::new(&walk, &[], earlier, owed, band);
    let (e, l) = (lay.set_at(earlier, before, 0.0), lay.set_at(last, before, 0.0));
    // The gap a letter set at `c` is at, signed: past touching, the boxes'
    // gap; else how far into one another they reach, its least axis's, as
    // a negative — so a letter set touching to the float reads apart from
    // one set into the ink.
    let gap_of = |c: f32| {
        let set = lay.set_at(c, half, rise);
        let apart = (set.at - l.at).abs() - (l.half + half);
        if apart.max_element() > 0.0 {
            apart.max(Vec2::ZERO).length()
        } else {
            apart.max_element()
        }
    };
    let bend = l.bend.of(walk.path, offset);
    let exact = bend.is_some_and(|b| b.ellipse.is_none() && (b.corner.is_some() || b.curvature.is_some()));
    let side = bend.is_some_and(|b| b.corner.is_none() && b.curvature == Some(0.0) && b.speed == 1.0);
    let model = osculating(walk.path, &l, Some(&e), (half, rise), owed, offset).map(|c| (c, gap_of(c)));
    let straight = along(Vec2::ZERO, l.moves, l.half + half, owed);
    let guess = if straight.is_finite() {
        last + straight
    } else {
        f32::NAN
    };
    let mut rounds = PAIR_ROUNDS;
    let set = pair(&lay, l, (half, rise), guess, owed, &mut rounds);
    let far = model.map_or(set.center, |(c, _)| c.max(set.center));
    let curve = if far.is_finite() && far > last {
        (1..CURVE_SAMPLES)
            .map(|i| {
                let c = last + (far - last) * (i as f32 / CURVE_SAMPLES as f32);
                (c, gap_of(c))
            })
            .collect()
    } else {
        Vec::new()
    };
    PairSeam {
        exact,
        side,
        model,
        iterative: (set.center, gap_of(set.center)),
        told: resolution_at(&l, l.half + half, owed).is_some(),
        curve,
    }
}

/// Test seam (#309, ruling B16.1b): the tangent line's root in closed form
/// ([`line_root`]) and as the iterative rule brackets and bisects it
/// ([`along`]), for two inks `delta` apart, their half sizes summing to
/// `reach`, one stepped along `dir` until they are `owed` apart.
pub(crate) fn line_root_seam(delta: Vec2, dir: Vec2, reach: Vec2, owed: f32) -> (f32, f32) {
    (line_root(delta, dir, reach, owed), along(delta, dir, reach, owed))
}

/// Test seam (#294): [`solve_tracking`] of a line whose lay at gap `g`
/// leaves `lay(g).0` of its part short and places its one glyph at
/// `lay(g).1`, over `steps` gaps to `end`: the gap solved and where the
/// glyph lies.
#[cfg(test)]
pub(crate) fn solve_tracking_seam(lay: impl Fn(f32) -> (f32, Vec2), steps: usize, end: f32) -> (f32, Vec2) {
    let track = |g: f32| {
        let (short, pos) = lay(g);
        Tracked {
            glyphs: vec![PlacedGlyph { glyph: 0, pos }],
            short,
            ends: None,
            centers: None,
            normals: None,
            boxes: Vec::new(),
            marks: Vec::new(),
            reaches: Vec::new(),
        }
    };
    let (gap, tracked, _) = solve_tracking(track, steps, end, None, &|| false);
    (gap, tracked.glyphs[0].pos)
}

/// Test seam (#294, ruling R50.1): whether two of `boxes`, each with
/// ink, touch, but for the pairs `exempt` excuses — by [`touch_scan`],
/// or with `strips`, by [`stripped`] alone across that axis — and the
/// pairs it looked at, in its sweep and in its strips.
#[cfg(test)]
pub(crate) fn touch_seam<K: Copy>(
    boxes: &[(K, InkBox)],
    exempt: &impl Fn(K, K) -> bool,
    strips: Option<usize>,
) -> (bool, usize, usize) {
    let mut looked = Looked::default();
    let touches = match strips {
        Some(axis) => stripped(boxes, axis, exempt, &vec![false; boxes.len()], &mut looked),
        None => touch_scan(boxes, exempt, &mut looked),
    };
    (touches, looked.swept, looked.stripped)
}

/// Test seam (#309, review 5810838863): whether two of the inks `after`
/// holds, line by line, touch that did not in `before` ([`touching_anew`]):
/// the touch check of a frame whose lines touched already, as its
/// secondary rule asks it.
#[cfg(test)]
pub(crate) fn touching_anew_seam(before: &[Vec<(Vec2, Vec2)>], after: &[Vec<(Vec2, Vec2)>]) -> bool {
    fn slices(lines: &[Vec<(Vec2, Vec2)>]) -> Vec<&[(Vec2, Vec2)]> {
        lines.iter().map(Vec::as_slice).collect()
    }
    touching_anew(&slices(before), &slices(after), &Work::new(0))
}

/// Test seam (#294, ruling R51.4): whether two of `boxes` touch, but for
/// the pairs `exempt` excuses, as a frame's checks ask it — by
/// [`touching`], which leaves out a box with no ink ([`has_ink`]) before
/// it looks.
#[cfg(test)]
pub(crate) fn touching_seam<K: Copy>(boxes: Vec<(K, InkBox)>, exempt: impl Fn(K, K) -> bool) -> bool {
    touching(boxes, exempt, &Work::new(0))
}

/// Test seam (#294, ruling R46.1): whether a frame's work, its budget
/// scaled by `glyphs` glyphs and then set to `budget`, reads as spent once
/// it has taken `steps` glyph steps.
#[cfg(test)]
pub(crate) fn spent_seam(glyphs: usize, classes: usize, budget: usize, steps: usize) -> bool {
    let work = Work::new(glyphs);
    work.hold_for(classes);
    work.budget.set(budget);
    work.spent.set(0);
    work.take(steps);
    work.spent_all()
}

/// Test seam (#291, ruling R32): where along `outline`'s first half the
/// last ink of `run` (one line, every glyph) ends, laid over the whole
/// half with each of `gaps` — how [`track`]'s length moves with its
/// gap, a zero gap included.
pub(crate) fn track_seam(run: &ShapedRun, outline: &Outline, gaps: &[f32], fs: &mut FontSystem) -> Vec<f32> {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let run = inked(run, fs);
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let walk = line_walk(halves.path(0), &run, true);
    let lay = seam_layout(&halves, &run, &clusters, fs);
    let (sa, sb) = halves.junctions[0];
    gaps.iter()
        .map(|&g| {
            let tracked = track(&walk, &run.glyphs, &clusters, (sa, sb), g, lay.band());
            tracked.ends.map_or(sa, |(_, last)| halves.along(0, middle(last)))
        })
        .collect()
}

/// A layout round `halves`, outward, for a test seam laying `clusters`
/// of `run` (inked) on the first half: its band centers measured, its
/// band curve at the offset the clusters want set at the half's apex
/// ([`Layout::estimate`], #308).
fn seam_layout<'h>(
    halves: &'h Halves,
    run: &ShapedRun,
    clusters: &[Cluster],
    fs: &mut FontSystem,
) -> Layout<'h> {
    let work = Work::new(0);
    let (_, centers) = inked_with_bands(run, fs);
    let lay = Layout::new(halves, true, work, centers);
    lay.offset.set(lay.estimate(run, [(0, clusters)]));
    lay
}

/// Test seam (#294, ruling R51.3): `run`, a line of letters, solved as
/// a frame lays it ([`solve_tracking`]) along `outline`'s first half,
/// outward, over a part from its top-left junction `room` points long:
/// the gap solved, how short of the part's end it leaves the line, the
/// tolerance on that, and how short the line is at no gap and at one gap
/// as long as all that room — where [`solve_tracking`]'s stop for a line
/// no gap moves once looked.
#[cfg(test)]
pub(crate) fn solve_line_seam(
    run: &ShapedRun,
    outline: &Outline,
    room: f32,
    fs: &mut FontSystem,
) -> (f32, f32, f32, f32, f32) {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let run = inked(run, fs);
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let walk = line_walk(halves.path(0), &run, true);
    let sa = halves.junctions[0].0;
    let sb = sa + room;
    let letters = clusters.iter().filter(|c| !c.blank).count();
    let steps = letters.saturating_sub(1) + clusters.iter().filter(|c| c.blank).count();
    let layout = seam_layout(&halves, &run, &clusters, fs);
    let lay = |g: f32| track(&walk, &run.glyphs, &clusters, (sa, sb), g, layout.band());
    let (gap, tracked, _) = solve_tracking(lay, steps, sb, None, &|| false);
    let at_zero = lay(0.0).short;
    (
        gap,
        tracked.short,
        line_tolerance(steps, sb),
        at_zero,
        lay(at_zero).short,
    )
}

/// Test seam (#294, ruling R43; #308): the geometry of a line's ends on
/// `outline`'s half `half` — the outward normal the 135° test weighs for
/// an end ink whose anchor is set at `set` along the half, on the band
/// curve `offset` off the outline ([`Layout::normals`]), and the way the
/// end of a part ending at `end` draws back ([`retreats`]).
pub(crate) fn end_seam(outline: &Outline, half: usize, offset: f32, set: f32, end: f32) -> (Vec2, Vec2) {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0), Vec::new());
    let on = halves.path(half).offset_at(set, offset);
    let ink = (on.point, on.point);
    let tracked = Tracked {
        glyphs: Vec::new(),
        short: 0.0,
        ends: Some((ink, ink)),
        centers: Some((set, set)),
        normals: Some((on.normal, on.normal)),
        boxes: vec![ink],
        marks: vec![0],
        reaches: vec![Reach::default()],
    };
    let normal = lay.normals(half, &tracked, (set, set))[0];
    (normal, retreats(halves.path(half), (set, end))[1])
}

/// Test seam (#291, ruling R33.1): `n` copies of `ornament` laid as a
/// junction lays them at their floor gap ([`SideOrnament::arc_floor`]),
/// along `outline`'s first half from a point at its top-left junction,
/// beside `run`'s text: the run they are shaped into, the copies, and the
/// floor. `None` when the ornament shapes to nothing or has no ink.
pub(crate) fn side_floor_seam(
    run: &ShapedRun,
    outline: &Outline,
    ornament: &str,
    n: usize,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
) -> Option<(ShapedRun, Vec<PlacedGlyph>, f32)> {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0), Vec::new());
    let sides = Some((ornament.to_owned(), Rgba::opaque(0, 0, 0)));
    let (with, range) = side_run(run, &sides, flow, fs, &lay.work)?;
    let fsys = fs.fonts();
    let drawn = inked(&with, fsys);
    letters_of(&clusters_of(&drawn.glyphs, 0..range.start))?;
    let side = SideOrnament::new(&lay, &drawn, range);
    let (ja, jb) = halves.junctions[0];
    let point = |s: f32| {
        let p = halves.path(0).at(s).0;
        ((p, p), s)
    };
    let arc = side.arc(point(ja), point(jb))?;
    let glyphs = arc.lay(n, arc.floor).0.out;
    let floor = arc.floor;
    Some((uninked(drawn), glyphs, floor))
}

/// Test seam (#294, ruling R44.3): `n` copies of `ornament` solved for
/// their equal gap ([`SideArc::solve`]) along the arc
/// [`side_floor_seam`] lays them on, between `outline`'s first half's
/// junctions, from the floor's row: the gap, and whether the copies laid
/// at it touch the junction's ink or pass its center. `None` when `n`
/// copies do not fit at the floor.
#[cfg(test)]
pub(crate) fn side_solve_seam(
    run: &ShapedRun,
    outline: &Outline,
    ornament: &str,
    n: usize,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
) -> Option<(f32, bool)> {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0), Vec::new());
    let sides = Some((ornament.to_owned(), Rgba::opaque(0, 0, 0)));
    let (with, range) = side_run(run, &sides, flow, fs, &lay.work)?;
    let fsys = fs.fonts();
    let drawn = inked(&with, fsys);
    letters_of(&clusters_of(&drawn.glyphs, 0..range.start))?;
    let side = SideOrnament::new(&lay, &drawn, range);
    let (ja, jb) = halves.junctions[0];
    let point = |s: f32| {
        let p = halves.path(0).at(s).0;
        ((p, p), s)
    };
    let arc = side.arc(point(ja), point(jb))?;
    let (row, rooms) = arc.floor_row(n)?;
    let &(room, glyphs, reaches) = rooms.get(n.checked_sub(1)?)?;
    let copies = Copies {
        glyphs: row.glyphs[..glyphs].to_vec(),
        reaches: row.reaches[..reaches].to_vec(),
    };
    let (gap, _, _) = arc.solve(n, room, copies);
    let (_, (end, at)) = arc.lay(n, gap);
    Some((gap, at >= arc.cb || end.is_nan() || end <= TOUCH))
}

/// Test seam (#291, ruling R34): the part of `outline`'s half `half` a
/// line may cover with `clear` kept at its junctions, and the half's
/// junctions — [`Layout::room`], which never inverts.
pub(crate) fn room_seam(outline: &Outline, half: usize, clear: [f32; 2]) -> ((f32, f32), (f32, f32)) {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true, Work::new(0), Vec::new());
    (lay.room(half, clear), halves.junctions[half])
}

/// Test seam (#291, ruling R33): `outline`'s two halves, each as its
/// junctions and apex along its path — from the middle of the left
/// side, clockwise for the first, counterclockwise for the second — and
/// the outline's length.
pub(crate) fn halves_seam(outline: &Outline) -> ([(f32, f32); 2], [f32; 2], f32) {
    let halves = Halves::of(outline, MIN_TABLE_SPACING);
    (halves.junctions, halves.apexes, halves.length)
}
