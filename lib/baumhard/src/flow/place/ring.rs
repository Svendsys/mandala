// SPDX-License-Identifier: MPL-2.0

//! A `Repeat` ring's work budget (#310): the budget a text frame's work
//! runs under (ruling R44.2, `halves::frame_budget`), for the ring of
//! whole copies `Repeat` lays round a closed outline (ruling R29).
//!
//! **What it counts.** A ring's work is walking its path: the walk that
//! lays its copies, the measures of its seam that count the copies and
//! solve their spacing ([`close_ring`](super::close_ring)), the lay of
//! them, and phase 2's walks that lay them again in reading order
//! ([`PathWalk::readable`](super::PathWalk::readable)). Every one of them
//! reads the path — a point looked up, or a step its memo answered
//! ([`ArcPath::reads`]) — a few times a cluster where it runs straight,
//! and up to [`CLUSTER_READS`] for an ornament's glyph whose ink is set
//! by bisection where the path's floats barely move it. So a ring counts
//! its path's reads, [`READS_A_STEP`] to a glyph step, and not its
//! clusters: a frame's weights count a glyph a step (ruling R46.1), and
//! an ornament's glyph some 10²² from the origin read its path 1,262
//! times (seed 2771 of the structured flow generator, as found).
//!
//! **What it may take.** The frame's budget, scaled by the glyphs the
//! ring will lay, as a frame's is by its text (rulings R44.2, B17.1a):
//! `max(TRACK_BUDGET, STEPS_PER_GLYPH × glyphs)` ([`frame_budget`],
//! [`STEPS_PER_GLYPH`]), its glyphs [`ring_glyphs`] — estimated before it
//! is laid, the path's length over one copy's advance, rounded up, times
//! a copy's glyphs — and at most its cap. A ring's work grows with its
//! copies: its walk lays each, and its seam's measures walk them all. So
//! a budget scaled by one copy's glyphs ran out on ordinary rings —
//! `─═` round an ellipse 10,000 across, `· ` round one 20,000 across
//! (review 5761697746) — where a budget scaled by the ring's glyphs closes
//! every ring of a sweep of ordinary ones, and holds six and a half times
//! the most steps a glyph any of them takes: an ornament of sparse ink,
//! whose glyphs step shorter than the advance they are estimated by
//! (`test_flow_ring_budget_ordinary_rings_close_within_a_fifth_of_their_budget_a_glyph`).
//!
//! **Its cap.** The most glyphs the ring lays is an argument of its
//! placement (ruling B17.2a): [`MAX_PLACED_GLYPHS`] in a map, as for any
//! placement, and the estimate is capped by it, so the budget is too. A
//! ring far from the origin, whose glyphs read its path a thousand times
//! each, runs to that budget — 196,608,000 steps at the cap's 65,536
//! glyphs — and stops, as a frame of that many glyphs would. The flow
//! fuzzers pass the cap that fits what an input may take, so the same
//! ring, its whole text and path, lays a few glyphs there and is short.
//! Its first walk stops at the cap: a ring the cap stops is short
//! ([`RingLoss::Short`]), as one its budget stops is.
//!
//! **What is held back.** Phase 2 lays the ring again whatever is spent:
//! its walks are what the placement is, not a search that may stop. So
//! as the ring's copies are laid, the budget holds back what phase 2 may
//! take over the glyphs laid so far ([`phase_two_reads`]), as a frame's
//! side holds back its touch check (ruling R48.2): the walk lays a
//! cluster only while what is left after the hold covers it. Phase 2's
//! run on past the walk's end ([`run_on_reads`]) is its work only for a
//! ring left unclosed, and only over the path such a ring leaves: so the
//! hold keeps the run on a walk stopped where it is would take — the whole
//! of it at the walk's start, next to nothing once its copies fill the
//! path — and a ring that closes lets it go ([`RingWork::closed`], ruling
//! B17.1d). A ring that closes carries no run on into phase 2, whose
//! solves stop at its budget and not at a slack of the hold.
//!
//! **Where it is read.** Before every cluster the ring's walks lay or
//! measure (`PathWalk::lay`, `PathWalk::walk_bodies`), and before every
//! probe of phase 2's spacing solves and every round of its regrouping.
//! A walk that finds it spent stops:
//!
//! - the first walk, before its copies fill the path: the ring keeps the
//!   copies it has, laid as walked, and is **short** ([`RingLoss::Short`]),
//!   as it is when its cut of a last copy is cut short by its budget;
//! - a measure of the seam, or the lay of the closed ring: the ring keeps
//!   its first walk, unclosed ([`RingLoss::Unclosed`]);
//! - a solve of phase 2: the spacing keeps the fitting side of its
//!   bracket, and the ring is **uneven** ([`RingLoss::Uneven`]).
//!
//! The trace reports each (`Trace::ring`): a ring is never capped
//! silently. So a ring takes at most its budget and [`ring_overshoot`]
//! past it: its setup, its path's table, and the one cluster that finds
//! it spent, at most [`CLUSTER_READS`] reads.
//!
//! The ring counts, too, the glyphs its walks set and phase 2 handles,
//! [`GLYPHS_A_STEP`] to a step, a setup of its own ([`RING_STEPS`]), and
//! its path's table, at its samples (`halves::TABLE_SAMPLES_A_STEP`,
//! ruling B16.3a): the work a ring does past its reads.

use std::cell::Cell;

use super::halves::{frame_budget, table_steps};
use super::{ArcPath, GlyphMetrics, CHORD_ROUNDS, INK_ROUNDS, INK_STRIDES, INK_TURNS, MAX_RUN_ON_STEPS};

#[cfg(doc)]
use super::halves::{STEPS_PER_GLYPH, TRACK_BUDGET};
#[cfg(doc)]
use super::MAX_PLACED_GLYPHS;

// What a ring's work is counted as, in glyph steps (#310, rulings R46.1,
// R47.3): a frame's unit, about 16 µs of the ASan fuzz build, the most a
// glyph step of a frame's tracking took. A ring's work is its path's reads
// and the glyphs it sets, and a ring's own setup. Measured by a temporary
// instrumentation of the engine, never committed, timing each ring's
// placement on the flow_structured target's ASan build, at the weights
// below: 504 rings of a grid — six texts, a word, a dot, two box-drawing
// glyphs, a letter at 4 pt, a bar and a 176-letter sentence, round an
// ellipse, a sharp and a rounded rectangle 600, 20,000, 10⁵, 10¹² and 10²²
// across, 600 across 10²² from the origin and 2,000 across 10⁸ from it,
// outward and inward, each laid twice — and the 154 rings seeds 0 to 2,500
// of the structured generator drew, at a load average of 3 to 4. The most
// any ring took was 13.11 µs a step (seed 569, one glyph over 262,169
// reads, the run on of a ring left short); at a read and a glyph a step
// alone, on #310's first draft, 22.07 µs (seed 1093). #310's PR body has
// the table.
//
// `READS_A_STEP` is R47.3's method, a weight's own envelope. The other two
// are not: a ring's reads, the glyphs it sets and its setup are not
// separate passes to be timed apart, so `GLYPHS_A_STEP` and `RING_STEPS`
// are one fit with it, made on #310's first draft and held on the final
// corpus above: every ring within 16 µs a step, the worst 13.11, a
// residual of 2.89 µs a step at the worst ring.

/// Path reads ([`ArcPath::reads`]) one glyph step weighs: over the rings
/// of 100,000 reads or more, at most 0.907 µs a read, setting glyphs
/// included (seed 494, 32,364 glyphs over 403,209 reads), so 16 µs ÷
/// 0.907 µs = 17.6 reads, rounded down — a weight rounded up would count
/// a read for less than its time — to 16, the weight [`GLYPHS_A_STEP`] and
/// [`RING_STEPS`] were fitted beside. (On #310's first draft the envelope
/// was 1.11 µs, 14.4 reads: its rings set glyphs counted with their reads,
/// where [`GLYPHS_A_STEP`] counts them apart.)
pub(crate) const READS_A_STEP: usize = 16;

/// Glyphs set one glyph step weighs: each glyph a ring's walks place
/// (`PathWalk::lay`), and each glyph phase 2 moves and copies
/// ([`PHASE_TWO_GLYPH_PASSES`]) — what a ring of many glyphs a cluster,
/// or of few reads a glyph, spends past its reads. A joint fit with
/// [`READS_A_STEP`] and [`RING_STEPS`], not a weight's own envelope (see
/// above): the three hold every ring measured within 16 µs a step.
pub(crate) const GLYPHS_A_STEP: usize = 8;

/// A ring's own setup, past what it counts — its clusters, its memo and
/// laying, phase 2's stretches — in glyph steps: a joint fit with the two
/// weights above, the most a small ring took past them on #310's first
/// draft (seed 1891, 150 glyphs over 12,055 reads, and a word round a 600
/// ellipse, 96 glyphs over 6,823 reads). On the final corpus no ring took
/// more than 13 steps past them.
pub(crate) const RING_STEPS: usize = 256;

/// The times phase 2 handles each glyph a ring places, past its walks'
/// reads: moving it with its cluster (`PathWalk::lay_readable`),
/// copying it into the trace, and gathering those kept.
const PHASE_TWO_GLYPH_PASSES: usize = 3;

/// The most reads one point an ornament's ink is set at takes
/// ([`PathWalk::ink_at`](super::PathWalk::ink_at)): the point, and
/// pushed outward, the two ends of the chord it is pushed off along.
const INK_READS: usize = 3;

/// The most reads one step of a walk takes
/// ([`PathWalk::step`](super::PathWalk::step),
/// [`PathWalk::take`](super::PathWalk::take)):
/// the point where it starts, and the far end of a chord for each of
/// [`CHORD_ROUNDS`] and the last — or one, read from the walk's memo.
const STEP_READS: usize = CHORD_ROUNDS + 2;

/// The most reads one cluster of a walk takes (`PathWalk::advance`, and
/// its corner in `PathWalk::lay`): its step ([`STEP_READS`]) and, for an
/// ornament's glyph set by its ink, [`INK_TURNS`] turns of
/// `PathWalk::ink_gap_at` — each the gap at its center and a solve of
/// `PathWalk::ink_distance_at`, its guess, [`INK_STRIDES`] strides and
/// [`INK_ROUNDS`] rounds of bisection, a point each — the point its ink is
/// remembered at, and the point its corner is set from. 3,250.
pub(crate) const CLUSTER_READS: usize =
    STEP_READS + INK_TURNS * (2 + INK_STRIDES + INK_ROUNDS) * INK_READS + 2 * INK_READS;

/// The walks of a ring's clusters phase 2 may make in moving its stretch
/// boundaries to word breaks (`PathWalk::snap_to_words`): each boundary
/// tries at most two moves, and each move's look for room
/// (`PathWalk::finds_room`) walks at most the four stretches about it,
/// the two it parts at most twice; a cluster lies in the four of at most
/// four boundaries, and a boundary moves at most one word into a
/// neighbor. `2 × 2 × 4`, and a walk more for the words moved: 17.
const SNAP_WALKS: usize = 17;

/// The walks of a reading ring's clusters phase 2 makes past its
/// stretches, whatever its budget, but for its cut: borrowing room
/// (`PathWalk::borrow_room`, two), each stretch's spacing measured, its
/// solve's bracket checked and the stretch laid (three), the pooled lay's
/// checks (three), the trace's lay and the readable lay (two), and one
/// probe of a spacing solve begun before the budget was spent (one).
const SPACED_WALKS: usize = 2 + 3 + 3 + 2 + 1;

/// The most reads phase 2 of a ring of `glyphs` glyphs takes whatever its
/// budget (#310), but for its run on past its walk's end
/// ([`run_on_reads`]): what the ring holds back for it. Its stretches'
/// word breaks ([`SNAP_WALKS`] walks); for a run with a letter in it
/// (`letters`), which is laid again in reading order, [`SPACED_WALKS`]
/// walks more, a cut's binary search of a walk for each halving of the
/// glyphs and one more (`PathWalk::cut`), and the point each laid glyph's
/// corner is set from. A walk of a glyph is at most [`STEP_READS`] reads:
/// a cluster's step, whatever its glyphs. Phase 2's spacing solves are
/// read against the budget (`PathWalk::spent`) and are not held back for.
pub(crate) fn phase_two_reads(glyphs: usize, letters: bool) -> usize {
    let walk = glyphs.saturating_mul(STEP_READS);
    let snapped = walk.saturating_mul(SNAP_WALKS);
    if !letters {
        return snapped;
    }
    let halvings = (usize::BITS - glyphs.leading_zeros()) as usize + 1;
    snapped
        .saturating_add(walk.saturating_mul(SPACED_WALKS + halvings))
        .saturating_add(glyphs)
}

/// The glyph steps phase 2 of a ring of `glyphs` glyphs takes whatever
/// its budget, but for its run on: its reads ([`phase_two_reads`]) and its
/// handling of each glyph ([`PHASE_TWO_GLYPH_PASSES`]). What the ring
/// holds back for it.
pub(crate) fn phase_two_steps(glyphs: usize, letters: bool) -> usize {
    steps_of(phase_two_reads(glyphs, letters)).saturating_add(
        glyphs
            .saturating_mul(PHASE_TWO_GLYPH_PASSES)
            .div_ceil(GLYPHS_A_STEP),
    )
}

/// The most reads phase 2's run on past a walk's end takes
/// (`PathWalk::runs_on`) for a ring left unclosed, its walk ending at
/// `from` or further along a path `length` long, at line height `h`: a
/// step, [`STEP_READS`] reads at most, a line height of path at a time to
/// its end, and [`MAX_RUN_ON_STEPS`] at most. A step is `h` along its
/// chord's direction, a unit vector, at least `h` of path (`PathWalk::step`),
/// and the `f32` of where it ends rounds it by half the path's grain
/// at most: so `h` less that grain a step. A path whose grain is half a
/// line height or more may not move on at all, and takes them all; as does
/// a line height no step is taken at, which takes none.
pub(crate) fn run_on_reads(from: f32, length: f32, h: f32) -> usize {
    let most = MAX_RUN_ON_STEPS * STEP_READS;
    if h.is_nan() || h <= 0.0 || from >= length {
        return 0;
    }
    // The spacing of the `f32`s about the path's length, the largest any
    // step's end is set to: its last bit.
    let grain = f64::from(f32::from_bits(length.abs().to_bits() & 0x7F80_0000)) * f64::from(f32::EPSILON);
    // A unit vector's two parts sum to its length, one less a rounding.
    let moved = f64::from(h) * (1.0 - 1e-4) - grain * 0.5;
    let steps = (f64::from(length) - f64::from(from)) / moved;
    // A grain that leaves a step no length, or a length that is not
    // finite, may take every step.
    if moved.is_nan() || moved <= 0.0 || steps.is_nan() || steps >= MAX_RUN_ON_STEPS as f64 {
        return most;
    }
    (steps.ceil() as usize + 1).saturating_mul(STEP_READS).min(most)
}

/// Glyph steps for `reads` reads of a path.
pub(super) fn steps_of(reads: usize) -> usize {
    reads.div_ceil(READS_A_STEP)
}

/// How far along its path one copy of `glyphs`, in `clusters` clusters,
/// steps with `gap` between their steps: its glyphs' advances and a gap a
/// cluster. What [`ring_glyphs`] estimates a ring's copies by.
pub(crate) fn copy_advance(glyphs: &[GlyphMetrics], clusters: usize, gap: f32) -> f64 {
    let advances: f64 = glyphs.iter().map(|glyph| f64::from(glyph.advance)).sum();
    advances + f64::from(gap) * clusters as f64
}

/// The glyphs a ring of copies of `copy` glyphs, each `advance` along
/// its path ([`copy_advance`]), lays round a path `length` long,
/// estimated before it is laid (ruling B17.1a): the copies the path
/// holds, its length over a copy's advance rounded up, at least one,
/// times a copy's glyphs, and at most `cap`. What the ring's budget is
/// scaled by. A path that holds copies past any count — an advance of
/// nothing, or a path whose length is not finite — holds `cap`; one whose
/// copies are not a number, one copy.
pub(crate) fn ring_glyphs(copy: usize, advance: f64, length: f32, cap: usize) -> usize {
    if copy == 0 {
        return 0;
    }
    // `max` takes the number of the two: copies that are not a number
    // are one copy.
    let glyphs = (f64::from(length) / advance).ceil().max(1.0) * copy as f64;
    if glyphs >= cap as f64 {
        cap
    } else {
        glyphs as usize
    }
}

/// The most glyph steps one cluster of a ring of `glyphs` glyphs a copy
/// takes: [`CLUSTER_READS`] reads, and its glyphs, no more than a copy's.
pub(crate) fn cluster_steps(glyphs: usize) -> usize {
    steps_of(CLUSTER_READS).saturating_add(glyphs.div_ceil(GLYPHS_A_STEP))
}

/// The most glyph steps a ring of `glyphs` glyphs a copy may take past
/// its budget (#310): its setup ([`RING_STEPS`]) and its path's table of
/// `tables` samples ([`table_steps`], ruling B16.3a), counted before its
/// budget is first read, and the cluster whose walk found it spent
/// ([`cluster_steps`]). Every other cluster laid past it is held back
/// for ([`phase_two_steps`]).
pub(crate) fn ring_overshoot(glyphs: usize, tables: usize) -> usize {
    RING_STEPS
        .saturating_add(table_steps(tables))
        .saturating_add(cluster_steps(glyphs))
}

/// The most glyph steps a ring of `copy` glyphs a copy, its budget scaled
/// by `glyphs` ([`ring_glyphs`]), its path's table of `tables` samples,
/// may take in all: its budget ([`frame_budget`]) and the most past it
/// ([`ring_overshoot`]). What the flow fuzzers hold an input's ring to
/// (ruling R45.1), its glyphs at most the cap they lay it with (ruling
/// B17.2b).
pub(crate) fn ring_work_steps(glyphs: usize, copy: usize, tables: usize) -> usize {
    frame_budget(glyphs).saturating_add(ring_overshoot(copy, tables))
}

/// How a ring's budget, or its cap, left it (#310), when it stopped its
/// work (`Trace::ring`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RingLoss {
    /// Phase 2's spacing was not solved to the end: the ring is laid at
    /// the fitting side of its bracket, its seam wider than its other
    /// gaps, or a stretch tighter than its walk's.
    Uneven,
    /// Its copies fill the path, but their spacing was not solved to
    /// close its seam: laid as walked, the seam a part-copy wide.
    Unclosed,
    /// Its first walk stopped before its copies filled the path — its
    /// budget spent, or its cap reached (ruling B17.2a): the copies laid
    /// so far, unclosed.
    Short,
}

/// A `Repeat` ring's work (#310): the path reads it has taken, its
/// budget and what of it is held back for phase 2.
pub(crate) struct RingWork {
    /// The path's reads when the ring began ([`ArcPath::reads`]).
    from: usize,
    /// The samples its path's table was built with, counted in its setup
    /// (ruling B16.3a).
    tables: usize,
    /// The ring's glyphs a copy, and whether any of them is a letter,
    /// which phase 2 lays in reading order.
    copy: usize,
    letters: bool,
    /// The most glyphs it lays (ruling B17.2a), the glyphs its budget is
    /// scaled by ([`ring_glyphs`], at most the cap), and its budget.
    cap: usize,
    glyphs: Cell<usize>,
    budget: Cell<usize>,
    /// Its path's length and its line height, what its run on is held
    /// back for by ([`run_on_reads`]).
    length: f32,
    line_height: Cell<f32>,
    /// Where its walk would end were it stopped now, as last noted
    /// ([`ends_by`](Self::ends_by)).
    from_end: Cell<f32>,
    /// The glyph steps of work it counts past its reads and its glyphs:
    /// a directional ring's shaping of its glyphs by direction (#324).
    took: Cell<usize>,
    /// The glyphs its walks have set, and the glyph steps held back for
    /// phase 2 over those its first walk set: its work on them, and its
    /// run on past where its walk would end were it stopped now.
    set: Cell<usize>,
    reserve: Cell<usize>,
    run_on: Cell<usize>,
    /// How the budget left the ring, the worst so far.
    loss: Cell<Option<RingLoss>>,
    /// Whether phase 2 has begun ([`lay_again`](Self::lay_again)).
    again: Cell<bool>,
    /// How far past what its budget left after the hold the ring was
    /// when its walk, its close or its lay first found it spent
    /// ([`crossed`](Self::crossed)).
    #[cfg(test)]
    crossed: Cell<Option<usize>>,
    /// The probes of phase 2's spacing solves, and the rounds of its
    /// regrouping, begun with the budget spent ([`late`](Self::late)).
    #[cfg(test)]
    late: Cell<usize>,
    /// What was held back for phase 2 when it began, and whether the ring
    /// had closed then ([`held_again`](Self::held_again)).
    #[cfg(test)]
    held_again: Cell<(usize, bool)>,
    #[cfg(test)]
    was_closed: Cell<bool>,
}

impl RingWork {
    /// A ring of copies of `copy` glyphs, letters among them or not, a
    /// line `line_height` high along `path`, at most `cap` glyphs, its
    /// budget scaled by `glyphs` ([`ring_glyphs`]): nothing taken yet, its
    /// run on held back for from the path's start.
    pub(crate) fn new(
        path: &ArcPath,
        copy: usize,
        letters: bool,
        line_height: f32,
        glyphs: usize,
        cap: usize,
    ) -> Self {
        let glyphs = glyphs.min(cap);
        let length = path.length();
        RingWork {
            from: path.reads(),
            tables: path.table_samples(),
            copy,
            letters,
            cap,
            glyphs: Cell::new(glyphs),
            budget: Cell::new(frame_budget(glyphs)),
            length,
            line_height: Cell::new(line_height),
            from_end: Cell::new(0.0),
            took: Cell::new(0),
            set: Cell::new(0),
            reserve: Cell::new(0),
            run_on: Cell::new(steps_of(run_on_reads(0.0, length, line_height))),
            loss: Cell::new(None),
            again: Cell::new(false),
            #[cfg(test)]
            crossed: Cell::new(None),
            #[cfg(test)]
            late: Cell::new(0),
            #[cfg(test)]
            held_again: Cell::new((0, false)),
            #[cfg(test)]
            was_closed: Cell::new(false),
        }
    }

    /// The most glyphs the ring lays (ruling B17.2a): its walks stop
    /// before a cluster that would pass it (`PathWalk::lay`).
    pub(crate) fn cap(&self) -> usize {
        self.cap
    }

    /// The samples its path's table was built with (ruling B16.3a).
    pub(crate) fn tables(&self) -> usize {
        self.tables
    }

    /// The glyph steps taken along `path`: its setup, its path's table,
    /// its reads, the glyphs it has set and the work it took apart.
    pub(crate) fn spent(&self, path: &ArcPath) -> usize {
        RING_STEPS
            .saturating_add(table_steps(self.tables))
            .saturating_add(steps_of(path.reads().saturating_sub(self.from)))
            .saturating_add(self.set.get().div_ceil(GLYPHS_A_STEP))
            .saturating_add(self.took.get())
    }

    /// Count `steps` glyph steps of work the ring did past its reads and
    /// its glyphs (#324).
    pub(crate) fn took(&self, steps: usize) {
        self.took.set(self.took.get().saturating_add(steps));
    }

    /// Count `glyphs` glyphs set: laid by a walk, or by phase 2 handled
    /// ([`PHASE_TWO_GLYPH_PASSES`] times each, [`placed`](Self::placed)).
    pub(crate) fn set(&self, glyphs: usize) {
        self.set.set(self.set.get().saturating_add(glyphs));
    }

    /// Count phase 2's handling of the `glyphs` glyphs the ring placed,
    /// held back for as they were laid.
    pub(crate) fn placed(&self, glyphs: usize) {
        self.set(glyphs.saturating_mul(PHASE_TWO_GLYPH_PASSES));
    }

    /// What is held back for phase 2 ([`admits`](Self::admits),
    /// [`ends_by`](Self::ends_by)).
    fn held(&self) -> usize {
        self.reserve.get().saturating_add(self.run_on.get())
    }

    /// Whether the ring may walk on along `path`: what it has taken is
    /// less than its budget less what is held back for phase 2.
    pub(crate) fn affords(&self, path: &ArcPath) -> bool {
        let room = self.budget.get().saturating_sub(self.held());
        let spent = self.spent(path);
        #[cfg(test)]
        if spent >= room && !self.again.get() && self.crossed.get().is_none() {
            self.crossed.set(Some(spent - room));
        }
        spent < room
    }

    /// Whether the placement may run a pass of `steps` glyph steps along
    /// `path` before its budget is read again (#313): what it has taken
    /// and that pass are less than its budget less what is held back for
    /// phase 2. Read before each pass of an open path's placement that
    /// does not read the budget as it goes — a reshape, a measure of its
    /// text as read — so that no such pass takes it past what its budget
    /// leaves.
    pub(crate) fn affords_more(&self, path: &ArcPath, steps: usize) -> bool {
        let room = self.budget.get().saturating_sub(self.held());
        self.spent(path).saturating_add(steps) < room
    }

    /// Scale the budget by `glyphs` glyphs, at most the cap, where that is
    /// more than it was scaled by (#313): an open path's `Once` with a
    /// filler, whose copies are estimated once the filler is shaped
    /// ([`ring_glyphs`]).
    pub(crate) fn scale(&self, glyphs: usize) {
        let glyphs = glyphs.min(self.cap);
        if glyphs > self.glyphs.get() {
            self.glyphs.set(glyphs);
            self.budget.set(frame_budget(glyphs));
        }
    }

    /// Hold back the run on phase 2 would take of a placement whose walks
    /// step `line_height` high (#313): an open path's `Fit`, reshaped to a
    /// smaller size than it began at, steps more often along the path
    /// past where its walk ends ([`run_on_reads`]). The least line height
    /// any of its walks steps at is what is held for.
    pub(crate) fn steps_at(&self, line_height: f32) {
        if line_height < self.line_height.get() {
            self.line_height.set(line_height);
            self.ends_by(self.from_end.get());
        }
    }

    /// Note that phase 2 begins: the ring is laid again in reading order
    /// with what its walk laid, its work held back for.
    pub(crate) fn lay_again(&self) {
        self.again.set(true);
        #[cfg(test)]
        self.held_again.set((self.held(), self.was_closed.get()));
    }

    /// What was held back for phase 2 when it began, and whether the ring
    /// had closed then (ruling B17.1d): for a closed ring, phase 2's work
    /// on its glyphs ([`phase_two_steps`]) and no run on.
    #[cfg(test)]
    pub(crate) fn held_again(&self) -> (usize, bool) {
        self.held_again.get()
    }

    /// Whether the ring may lay a cluster that brings it to `glyphs`
    /// glyphs: [`affords`](Self::affords), with phase 2's work on them
    /// held back ([`phase_two_steps`]), and its run on past where its walk
    /// would end ([`ends_by`](Self::ends_by)).
    pub(crate) fn admits(&self, path: &ArcPath, glyphs: usize) -> bool {
        let held = phase_two_steps(glyphs, self.letters);
        if held > self.reserve.get() {
            self.reserve.set(held);
        }
        self.affords(path)
    }

    /// Note that the ring's walk, stopped now, would end at `from` along
    /// its path or further (ruling B17.1d): the run on phase 2 would take
    /// of the ring left so is what is held back for it ([`run_on_reads`]).
    /// The walk notes it before each copy it lays, from where the copy
    /// before it ends: what a stop of the copy keeps.
    pub(crate) fn ends_by(&self, from: f32) {
        self.run_on
            .set(steps_of(run_on_reads(from, self.length, self.line_height.get())));
        self.from_end.set(from);
    }

    /// Note that the ring closed, laid as `glyphs` glyphs (ruling B17.1d):
    /// phase 2 lays a closed ring to the path's end and takes no run on
    /// past it (`PathWalk::runs_on`), so the hold is phase 2's work on
    /// those glyphs without it. Every walk that may yet leave the ring
    /// unclosed — its first, the measures of its seam and the lay of it —
    /// is done.
    pub(crate) fn closed(&self, glyphs: usize) {
        self.reserve.set(phase_two_steps(glyphs, self.letters));
        self.run_on.set(0);
        #[cfg(test)]
        self.was_closed.set(true);
    }

    /// Note that a probe of phase 2's spacing solves, or a round of its
    /// regrouping, begins (ruling B17.1d): counted when the budget is
    /// spent (`RingWork::late`, a test's). Nothing outside a test.
    #[cfg_attr(not(test), allow(unused_variables))]
    pub(crate) fn begin(&self, path: &ArcPath) {
        #[cfg(test)]
        if self.again.get() && self.spent(path) >= self.budget.get().saturating_sub(self.held()) {
            self.late.set(self.late.get() + 1);
        }
    }

    /// The probes of phase 2's spacing solves, and the rounds of its
    /// regrouping, begun with the budget spent: none, since each reads
    /// the budget before it begins (ruling B17.1d). The one probe begun
    /// before the budget ran out and finished past it is held back for
    /// ([`SPACED_WALKS`]); one begun after is not.
    #[cfg(test)]
    pub(crate) fn late(&self) -> usize {
        self.late.get()
    }

    /// How far past what its budget left after the hold the ring was when
    /// its walk, its close or its lay first found it spent — before phase
    /// 2, whose required work runs within the hold: no further than its
    /// setup and the one cluster that crossed it ([`ring_overshoot`]).
    /// `None` when none did.
    #[cfg(test)]
    pub(crate) fn crossed(&self) -> Option<usize> {
        self.crossed.get()
    }

    /// Note that the budget left the ring so: the worst loss stands.
    pub(crate) fn lose(&self, loss: RingLoss) {
        if self.loss.get().is_none_or(|was| loss > was) {
            self.loss.set(Some(loss));
        }
    }

    /// How the budget left the ring, if it stopped any of its work.
    pub(crate) fn loss(&self) -> Option<RingLoss> {
        self.loss.get()
    }

    /// Its budget, the glyphs it was scaled by ([`ring_glyphs`], at most
    /// its cap), and its glyphs a copy.
    pub(crate) fn budget(&self) -> (usize, usize, usize) {
        (self.budget.get(), self.glyphs.get(), self.copy)
    }
}
