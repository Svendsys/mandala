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
//! **What it may take.** The frame's budget, scaled by the run's glyphs —
//! one copy's — at least [`TRACK_BUDGET`]: `max(TRACK_BUDGET,
//! STEPS_PER_GLYPH × glyphs)` ([`frame_budget`], [`STEPS_PER_GLYPH`]). The copies count
//! against it and scale it not, as a frame's sides do (ruling R44.3).
//!
//! **What is held back.** Phase 2 lays the ring again whatever is spent:
//! its walks are what the placement is, not a search that may stop. So
//! as the ring's copies are laid, the budget holds back what phase 2 may
//! take over the glyphs laid so far ([`phase_two_reads`]), as a frame's
//! side holds back its touch check (ruling R48.2): the walk lays a
//! cluster only while what is left after the hold covers it.
//!
//! **Where it is read.** Before every cluster the ring's walks lay or
//! measure (`PathWalk::lay`, `PathWalk::walk_bodies`), and before every
//! probe of phase 2's spacing solves. A walk that finds it spent stops:
//!
//! - the first walk, before its copies fill the path: the ring keeps the
//!   copies it has, laid as walked, and is **short** ([`RingLoss::Short`]);
//! - a measure of the seam, or the lay of the closed ring: the ring keeps
//!   its first walk, unclosed ([`RingLoss::Unclosed`]);
//! - a solve of phase 2: the spacing keeps the fitting side of its
//!   bracket, and the ring is **uneven** ([`RingLoss::Uneven`]).
//!
//! The trace reports each (`Trace::ring`): a ring is never capped
//! silently. So a ring takes at most its budget and [`ring_overshoot`]
//! past it: its setup, and the one cluster that finds it spent, at most
//! [`CLUSTER_READS`] reads.
//!
//! The ring counts, too, the glyphs its walks set and phase 2 handles,
//! [`GLYPHS_A_STEP`] to a step, and a setup of its own ([`RING_STEPS`]):
//! the work a ring does past its reads.

use std::cell::Cell;

use super::halves::frame_budget;
use super::{ArcPath, CHORD_ROUNDS, INK_ROUNDS, INK_STRIDES, INK_TURNS, MAX_RUN_ON_STEPS};

#[cfg(doc)]
use super::halves::{STEPS_PER_GLYPH, TRACK_BUDGET};

// What a ring's work is counted as, in glyph steps (#310, rulings R46.1,
// R47.3): a frame's unit, about 16 µs of the ASan fuzz build, the most a
// glyph step of a frame's tracking took. A ring's work is its path's reads
// and the glyphs it sets, and a ring's own setup; the three weights below
// are one fit of a ring's time against them. Measured on #310's first
// draft, whose weights, and so where its rings stopped, were others, by a
// temporary instrumentation of the engine, never committed, timing each
// ring's placement on the flow_structured target's ASan build: 504 rings
// of a grid — six texts, a word, a dot, two box-drawing glyphs, a letter
// at 4 pt, a bar and a 176-letter sentence, round an ellipse, a sharp and a
// rounded rectangle 600, 20,000, 10⁵, 10¹² and 10²² across, 600 across
// 10²² from the origin and 2,000 across 10⁸ from it, outward and inward,
// each laid twice — and the 154 rings seeds 0 to 2,500 of the structured
// generator drew, at a load average of 2 to 3.5. At these weights the most
// any ring took was 14.07 µs a step (seed 1093, 3,132 glyphs over 56,602
// reads); at a read and a glyph a step alone, 22.07 µs (the same ring).
// #310's PR body has the table.

/// Path reads ([`ArcPath::reads`]) one glyph step weighs: over the rings
/// of 100,000 reads or more, at most 1.11 µs a read, setting glyphs
/// included (seed 1440, 16,380 glyphs over 337,759 reads), so 16 µs ÷
/// 1.11 µs = 14.4, rounded up to 16 with [`GLYPHS_A_STEP`] beside it.
pub(crate) const READS_A_STEP: usize = 16;

/// Glyphs set one glyph step weighs: each glyph a ring's walks place
/// (`PathWalk::lay`), and each glyph phase 2 moves and copies
/// ([`PHASE_TWO_GLYPH_PASSES`]) — what a ring of many glyphs a cluster,
/// or of few reads a glyph, spends past its reads: fitted with
/// [`READS_A_STEP`] and [`RING_STEPS`] to hold every ring measured within
/// 16 µs a step.
pub(crate) const GLYPHS_A_STEP: usize = 8;

/// A ring's own setup, past what it counts — its clusters, its memo and
/// laying, phase 2's stretches — in glyph steps: fitted with the two
/// weights above, the most a small ring took past them (seed 1891, 150
/// glyphs over 12,055 reads, and a word round a 600 ellipse, 96 glyphs
/// over 6,823 reads).
pub(crate) const RING_STEPS: usize = 256;

/// The times phase 2 handles each glyph a ring places, past its walks'
/// reads: moving it with its cluster (`PathWalk::lay_readable`),
/// copying it into the trace, and gathering those kept.
const PHASE_TWO_GLYPH_PASSES: usize = 3;

/// The most reads one point an ornament's ink is set at takes
/// ([`PathWalk::ink_point`](super::PathWalk::ink_point)): the point, and
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
/// budget (#310): what the ring holds back for it. Its run on past the
/// walk's end ([`MAX_RUN_ON_STEPS`] steps, `PathWalk::runs_on`) and its
/// stretches' word breaks ([`SNAP_WALKS`] walks); for a run with a letter
/// in it (`letters`), which is laid again in reading order,
/// [`SPACED_WALKS`] walks more, a cut's binary search of a walk for each
/// halving of the glyphs and one more (`PathWalk::cut`), and the point
/// each laid glyph's corner is set from. A walk of a glyph is at most
/// [`STEP_READS`] reads: a cluster's step, whatever its glyphs. Phase 2's
/// spacing solves are read against the budget (`PathWalk::spent`) and
/// are not held back for.
pub(crate) fn phase_two_reads(glyphs: usize, letters: bool) -> usize {
    let walk = glyphs.saturating_mul(STEP_READS);
    let run_on = MAX_RUN_ON_STEPS * STEP_READS;
    let snapped = run_on.saturating_add(walk.saturating_mul(SNAP_WALKS));
    if !letters {
        return snapped;
    }
    let halvings = (usize::BITS - glyphs.leading_zeros()) as usize + 1;
    snapped
        .saturating_add(walk.saturating_mul(SPACED_WALKS + halvings))
        .saturating_add(glyphs)
}

/// The glyph steps phase 2 of a ring of `glyphs` glyphs takes whatever
/// its budget: its reads ([`phase_two_reads`]) and its handling of each
/// glyph ([`PHASE_TWO_GLYPH_PASSES`]). What the ring holds back for it.
pub(crate) fn phase_two_steps(glyphs: usize, letters: bool) -> usize {
    steps_of(phase_two_reads(glyphs, letters)).saturating_add(
        glyphs
            .saturating_mul(PHASE_TWO_GLYPH_PASSES)
            .div_ceil(GLYPHS_A_STEP),
    )
}

/// Glyph steps for `reads` reads of a path.
fn steps_of(reads: usize) -> usize {
    reads.div_ceil(READS_A_STEP)
}

/// The most glyph steps one cluster of a ring of `glyphs` glyphs a copy
/// takes: [`CLUSTER_READS`] reads, and its glyphs, no more than a copy's.
pub(crate) fn cluster_steps(glyphs: usize) -> usize {
    steps_of(CLUSTER_READS).saturating_add(glyphs.div_ceil(GLYPHS_A_STEP))
}

/// The most glyph steps a ring of `glyphs` glyphs a copy may take past
/// its budget (#310): its setup ([`RING_STEPS`]), counted before its
/// budget is first read, and the cluster whose walk found it spent
/// ([`cluster_steps`]). Every other cluster laid past it is held back
/// for ([`phase_two_steps`]).
pub(crate) fn ring_overshoot(glyphs: usize) -> usize {
    RING_STEPS.saturating_add(cluster_steps(glyphs))
}

/// The most glyph steps a ring of `glyphs` glyphs a copy may take in all:
/// its budget ([`frame_budget`]) and the most past it
/// ([`ring_overshoot`]). What the flow fuzzers hold an input's ring to
/// (ruling R45.1).
pub(crate) fn ring_work_steps(glyphs: usize) -> usize {
    frame_budget(glyphs).saturating_add(ring_overshoot(glyphs))
}

/// How a ring's budget left it (#310), when it stopped its work
/// (`Trace::ring`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RingLoss {
    /// Phase 2's spacing was not solved to the end: the ring is laid at
    /// the fitting side of its bracket, its seam wider than its other
    /// gaps, or a stretch tighter than its walk's.
    Uneven,
    /// Its copies fill the path, but their spacing was not solved to
    /// close its seam: laid as walked, the seam a part-copy wide.
    Unclosed,
    /// Its first walk stopped before its copies filled the path: the
    /// copies laid so far, unclosed.
    Short,
}

/// A `Repeat` ring's work (#310): the path reads it has taken, its
/// budget and what of it is held back for phase 2.
pub(crate) struct RingWork {
    /// The path's reads when the ring began ([`ArcPath::reads`]).
    from: usize,
    /// The ring's glyphs a copy, its budget scaled by them, and whether
    /// any of them is a letter, which phase 2 lays in reading order.
    glyphs: usize,
    budget: usize,
    letters: bool,
    /// The glyphs its walks have set, and the glyph steps held back for
    /// phase 2 over those its first walk set.
    set: Cell<usize>,
    reserve: Cell<usize>,
    /// How the budget left the ring, the worst so far.
    loss: Cell<Option<RingLoss>>,
    /// Whether phase 2 has begun ([`lay_again`](Self::lay_again)).
    again: Cell<bool>,
    /// How far past what its budget left after the hold the ring was
    /// when its walk, its close or its lay first found it spent
    /// ([`crossed`](Self::crossed)).
    #[cfg(test)]
    crossed: Cell<Option<usize>>,
}

impl RingWork {
    /// A ring of copies of `glyphs` glyphs, letters among them or not,
    /// along `path`, nothing taken yet.
    pub(crate) fn new(path: &ArcPath, glyphs: usize, letters: bool) -> Self {
        RingWork {
            from: path.reads(),
            glyphs,
            budget: frame_budget(glyphs),
            letters,
            set: Cell::new(0),
            reserve: Cell::new(0),
            loss: Cell::new(None),
            again: Cell::new(false),
            #[cfg(test)]
            crossed: Cell::new(None),
        }
    }

    /// The glyph steps taken along `path`: its setup, its reads and the
    /// glyphs it has set.
    pub(crate) fn spent(&self, path: &ArcPath) -> usize {
        RING_STEPS
            .saturating_add(steps_of(path.reads().saturating_sub(self.from)))
            .saturating_add(self.set.get().div_ceil(GLYPHS_A_STEP))
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

    /// Whether the ring may walk on along `path`: what it has taken is
    /// less than its budget less what is held back for phase 2.
    pub(crate) fn affords(&self, path: &ArcPath) -> bool {
        let room = self.budget.saturating_sub(self.reserve.get());
        let spent = self.spent(path);
        #[cfg(test)]
        if spent >= room && !self.again.get() && self.crossed.get().is_none() {
            self.crossed.set(Some(spent - room));
        }
        spent < room
    }

    /// Note that phase 2 begins: the ring is laid again in reading order
    /// with what its walk laid, its work held back for.
    pub(crate) fn lay_again(&self) {
        self.again.set(true);
    }

    /// Whether the ring may lay a cluster that brings it to `glyphs`
    /// glyphs: [`affords`](Self::affords), with phase 2's work on them
    /// held back ([`phase_two_steps`]).
    pub(crate) fn admits(&self, path: &ArcPath, glyphs: usize) -> bool {
        let held = phase_two_steps(glyphs, self.letters);
        if held > self.reserve.get() {
            self.reserve.set(held);
        }
        self.affords(path)
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

    /// Its budget, and the glyphs a copy it was scaled by.
    pub(crate) fn budget(&self) -> (usize, usize) {
        (self.budget, self.glyphs)
    }
}
