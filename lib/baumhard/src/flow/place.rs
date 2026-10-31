// SPDX-License-Identifier: MPL-2.0

//! Placement: upright glyphs positioned along a path (design §6.2, D9),
//! and the fit modes that decide what happens when text and path differ
//! in length (§6.4, D10).
//!
//! **Glyphs are never rotated.** A curve is made by where glyphs sit,
//! the way ASCII art makes one. The unit of placement is the **grapheme
//! cluster** (ruling R9.1): a letter with its combining marks, a ZWJ
//! sequence, whatever reads as one character. A cluster of summed
//! advance `w` and line height `h` placed at distance `s` along the path
//! occupies a step `|cos θ|·w + |sin θ|·h`: along a horizontal stretch
//! clusters sit side by side, along a vertical one they stack a line
//! apart, and on a curve the two blend. θ is the direction of the
//! **chord** the step spans, from `p(s)` to `p(s + step)` (ruling R9.2),
//! so the step turns continuously through a sharp corner instead of
//! jumping; on a straight stretch, and on a circular arc at the step's
//! middle, it is the direction of travel. The step depends on θ and θ on
//! the step, so it is resolved in a bounded number of rounds.
//!
//! The cluster's box, `w × h`, is centered on the path half a step on —
//! on a closed path pushed outward by half its extent along the chord's
//! outward normal, `|n.x|·w/2 + |n.y|·h/2`, so text sits on the outline
//! rather than across it — and every glyph of the cluster sits in that
//! box at its pen position from the shaped line, so a mark never leaves
//! its base. The next cluster starts a step (plus any spacing) later.
//!
//! **Cuts keep the start of the text as it is read** (ruling R9.4). When
//! text is longer than its path, what is kept is the longest prefix in
//! reading order — the order of the text, which for right-to-left and
//! mixed text is not the visual order the glyphs are listed and walked
//! in — and the kept clusters are laid in visual order. An RTL run so
//! keeps its beginning and loses its end, as an LTR one does.
//!
//! **A frame's text is two lines** (#291, ruling R27; [`lines`]). A ring
//! whose fit mode is `Once`, `Spread` or `Fit` holds text, and is set over
//! the top of its outline and along the bottom, each line reading left to
//! right on the level part of its half, the steep sides clear; `start`
//! and `direction` do not apply to it. A line's clusters are spaced by
//! their advances measured horizontally (ruling R28), not by the box
//! extent above. What follows places everything else: a `Repeat` ring,
//! and every connector.
//!
//! **Every stretch reads left to right or top to bottom** (the readable
//! rule, #291, rulings R25, R25.1 and R26). Glyphs are never rotated, so
//! text walked in its own order along a path that runs leftward would
//! read backward, and along one that runs upward bottom to top.
//! Placement is therefore two phases:
//!
//! 1. **The walk** — the fit mode, as above — decides which clusters are
//!    placed and where along the path each one's step starts. The path
//!    splits into **stretches**, each a maximal run of steps of one
//!    [`Travel`], classed by its dominant direction: rightward
//!    (`cos θ > ε`), leftward (`cos θ < −ε`), or downward or upward (`y`
//!    grows downward), with ε = cos 45° ([`VERTICAL_EPSILON`]). A run of
//!    steep steps is a stretch of its own only when it is clearly entered
//!    ([`VERTICAL_MIN_LINES`]); a turn through vertical shorter than that
//!    joins the stretches on either side, split where it passes plumb.
//!    A cluster belongs to the stretch its step is in — but a word a
//!    boundary would tear is moved whole into one of the two stretches
//!    (ruling R26): torn next to a reversed stretch, its head and its
//!    tail would land at the two ends of it.
//! 2. **The lay** lays the same clusters forward along the path in
//!    **reading order**: stretch after stretch, each leftward and upward
//!    stretch's clusters last to first, so that stretch reads left to
//!    right, or top to bottom. Each stretch keeps to the part of the path
//!    it held in the walk; one that needs more room — reversed, a wide
//!    glyph moved onto a vertical part steps the line height, not its
//!    advance, and a moved word takes room too — borrows it from a
//!    neighbor read the same way, where it still reads forward. Its
//!    spacing is solved within its part: `Spread` and `Fit` end where the
//!    walk ended, `Once` and `Repeat` keep their spacing where it fits.
//!    Text that ends short of the path keeps its last stretch to where
//!    the path runs on that stretch's way ([`PathWalk::runs_on`]): past a
//!    turn, its clusters would read backward.
//!    Where the stretches cannot each keep to their parts, they are laid
//!    on one from the next with one spacing (ruling R25). The gap never
//!    goes below zero; where even no gap passes the end, the placement is
//!    cut — whole filler copies or repeat clusters first, the text last —
//!    and `Fit` measures text as read, every word whole and no stretch
//!    running on into one read the other way, when it chooses its scale,
//!    so it cuts only at `min`.
//!
//! On a loop this reads like a seal: the top left to right, the right
//! side top to bottom, the bottom left to right, the left side top to
//! bottom. [`PlacedRun::glyphs`] keeps the walk's order, so every index
//! into a placement still names the glyph it named.
//!
//! A run is placed once and re-placed at will: placement reads the
//! run's [`GlyphMetrics`] and never its text, so moving a node re-places
//! its flows without reshaping. Only two modes shape anything, through
//! the font system they are handed: `Fit`, which rescales the run, and
//! `Once` with a filler, which appends the filler; both return the run
//! their placement refers to (ruling R6).

use std::ops::Range;

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::{Direction, FitMode, StartPoint};
use crate::flow::curve::Curve;
use crate::flow::outline::Outline;
use crate::flow::path::ArcPath;
use crate::flow::shape::{reshape, GlyphMetrics, ShapedRun, FILLER_SPAN};
use crate::flow::span::{point_size, LINE_HEIGHT_FACTOR};
use crate::font::fonts::{clamp_font_metric, FontSystemWriteGuard, MIN_FONT_SIZE_PT};
use crate::font::FontSystem;

mod lines;
pub(crate) use lines::frame_halves;

/// The most steps [`PathWalk::runs_on`] probes past a text's end to find
/// where the path stops running the last stretch's way: a path that long
/// in line heights is followed no further, and the last stretch's part
/// ends there.
const MAX_RUN_ON_STEPS: usize = 1 << 16;

/// The most rounds [`PathWalk::even_out`] regroups a closed ring's
/// stretches in.
const EVEN_ROUNDS: usize = 4;

/// The step the walk's box rule (#239 §5 step 2, ruling R9.2) takes for
/// a `w × h` cluster from `s` along `path`, and its direction: the test
/// seam for that rule at a sharp corner, which only a closed outline has.
pub(crate) fn box_step(path: &ArcPath, s: f32, w: f32, h: f32) -> (f32, Vec2) {
    let walk = PathWalk {
        path,
        line_height: h,
        outward: false,
        limit: path.length(),
        level: false,
        ornament: false,
    };
    walk.step(s, w, h)
}

/// The rounds of Newton's method [`PathWalk::level_len`] takes.
const LEVEL_ROUNDS: usize = 4;

/// The most glyphs one placement places (ruling R6, reading 9). A
/// repeating flow on a path long enough to need more stops here and
/// reports [`PlacedRun::truncated`].
pub const MAX_PLACED_GLYPHS: usize = 1 << 20;

/// How far inside its end condition a solved spacing lands: `Spread`
/// and `Fit` end their last cluster's step within this of the path's
/// length, and never past it.
const SOLVE_TOLERANCE: f32 = 0.05;

/// The most rounds the spacing solver takes. Each solver is bounded, so
/// no placement loops unbounded on an interactive path.
const SOLVE_ROUNDS: usize = 48;

/// The most rounds `Fit` takes to shrink its scale until the text fits.
const FIT_ROUNDS: usize = 16;

/// The chord rounds that resolve a step after its seed (ruling R9.2).
const CHORD_ROUNDS: usize = 2;

/// ε of the readable rule (#291, ruling R26): a step whose direction θ
/// has `|cos θ| ≤ ε` is steep — at least 45° from level — and so is read
/// as vertical; any shallower step is rightward or leftward. Each step
/// is classed by its dominant direction, so the steep sides of a circle
/// or an ellipse read top to bottom like a rectangle's, and a connector
/// steeper than 45° reads top to bottom too. (A smaller ε, 2° at first,
/// left a curve's steep ends in its horizontal stretches, where text read
/// bottom to top.)
pub(crate) const VERTICAL_EPSILON: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The hysteresis of the readable rule (#291), in line heights: a run of
/// steep steps is a vertical stretch of its own only when it spans at
/// least this much of the path (or is the whole placement). A shorter
/// one — a tight turn through vertical, the tip of a flat ellipse, a
/// small rounded corner between two level sides — stays with the
/// horizontal stretches around it, so the turn makes no stretch of a
/// glyph or two; a side three lines long reads as a column.
pub(crate) const VERTICAL_MIN_LINES: f32 = 3.0;

/// One glyph placed: which glyph of the run, and where its box's
/// top-left corner sits on the canvas.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// An index into the run's [`ShapedRun::glyphs`] (and its
    /// [`ShapedRun::layout_glyphs`]). Repeated text reuses indices.
    pub glyph: u32,
    /// The top-left corner of the glyph's `advance × line_height` box.
    /// The glyph is drawn upright in it, its baseline
    /// [`ShapedRun::baseline`] below this corner. Glyphs of one grapheme
    /// cluster share the cluster's box, each at its pen position in it.
    pub pos: Vec2,
}

/// A run placed along a path or at a point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacedRun {
    /// The placed glyphs: the clusters in the order the walk took them
    /// from the run — the text's visual order, copy after copy — each
    /// cluster's glyphs in visual order. On a stretch placed reversed
    /// (the readable rule) the positions run against this order along
    /// the path; the order itself, and so every index into it (a run's
    /// overrides, a hit test), is the text's.
    pub glyphs: Vec<PlacedGlyph>,
    /// Whether the flow's text could not be placed whole even once: it
    /// was cut to the longest reading-order prefix that fits, or
    /// placement stopped at [`MAX_PLACED_GLYPHS`]. A `Repeat` whose last
    /// copy is cut, or a filler that stops short of the end, is the mode
    /// working, not a truncation.
    pub truncated: bool,
}

/// The tolerance a float compare against the path's length allows, so
/// that a cluster ending exactly at the end is not cut by rounding: a
/// thousandth of a point, or a millionth of the path, whichever is
/// larger.
fn end_slack(length: f32) -> f32 {
    (length * 1e-6).max(1e-3)
}

/// A grapheme cluster of a run: the unit placement steps, cuts and
/// spaces (ruling R9.1).
#[derive(Clone, Debug)]
struct Cluster {
    /// Its glyphs, as indices into the run's glyphs, in visual order.
    glyphs: Range<usize>,
    /// The sum of its glyphs' advances: its box's width.
    advance: f32,
    /// The leftmost pen position of its glyphs in the shaped line: each
    /// glyph sits that far into the box from its own pen position.
    left: f32,
    /// Where it starts in reading order: the span and span byte of its
    /// first character.
    reading: (usize, usize),
    /// Whether the text may be parted before it without tearing a word
    /// ([`GlyphMetrics::word_break`] of its first glyph).
    word_break: bool,
    /// Whether it is whitespace ([`GlyphMetrics::blank`]).
    blank: bool,
    /// Whether any of its glyphs is a letter or a digit
    /// ([`GlyphMetrics::letter`]).
    letter: bool,
    /// Whether it is the space laid for a line break
    /// ([`GlyphMetrics::line_break`]).
    line_break: bool,
}

/// The clusters of `glyphs[range]`, in visual order: a cluster begins
/// at every glyph marked [`GlyphMetrics::grapheme_start`] (and at the
/// range's first glyph).
///
/// Cost: O(glyphs); one `Vec` entry per cluster.
fn clusters_of(glyphs: &[GlyphMetrics], range: Range<usize>) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    for index in range {
        let glyph = &glyphs[index];
        let reading = (glyph.span, glyph.bytes.start);
        match clusters.last_mut() {
            Some(cluster) if !glyph.grapheme_start && cluster.glyphs.end == index => {
                cluster.glyphs.end = index + 1;
                cluster.advance += glyph.advance;
                cluster.left = cluster.left.min(glyph.x);
                cluster.reading = cluster.reading.min(reading);
                cluster.letter |= glyph.letter;
            }
            _ => clusters.push(Cluster {
                glyphs: index..index + 1,
                advance: glyph.advance,
                left: glyph.x,
                reading,
                word_break: glyph.word_break,
                blank: glyph.blank,
                letter: glyph.letter,
                line_break: glyph.line_break,
            }),
        }
    }
    clusters
}

/// One cluster as the walk laid it: what the readable rule (#291)
/// classifies and, on a backward stretch, places again.
#[derive(Copy, Clone, Debug)]
struct Stepped {
    /// Its glyphs in the placement: `glyphs[first..end]`.
    first: usize,
    end: usize,
    /// Where its step starts along the path, and the step's length.
    s: f32,
    step: f32,
    /// The direction the step was taken in: its chord.
    dir: Vec2,
    /// Its box's width.
    advance: f32,
    /// Its box's top-left corner, as laid.
    corner: Vec2,
    /// Where it starts in reading order ([`Cluster::reading`]).
    reading: (usize, usize),
    /// [`Cluster::word_break`].
    word_break: bool,
    /// [`Cluster::letter`].
    letter: bool,
}

/// A placement being built: the glyphs, and the step each cluster took.
#[derive(Default)]
struct Laying {
    glyphs: Vec<PlacedGlyph>,
    steps: Vec<Stepped>,
}

impl Laying {
    fn with_capacity(glyphs: usize) -> Self {
        Laying {
            glyphs: Vec::with_capacity(glyphs),
            steps: Vec::new(),
        }
    }

    /// How much has been laid, to [`truncate`](Self::truncate) back to.
    fn mark(&self) -> (usize, usize) {
        (self.glyphs.len(), self.steps.len())
    }

    fn truncate(&mut self, (glyphs, steps): (usize, usize)) {
        self.glyphs.truncate(glyphs);
        self.steps.truncate(steps);
    }
}

/// Which way a stretch of the path runs, by the direction θ of its steps
/// (#291). `y` grows downward.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Travel {
    Rightward,
    Leftward,
    Downward,
    Upward,
}

impl Travel {
    /// Whether a stretch so running is placed reversed: from its far end
    /// back toward its near end, so it reads left to right or top to
    /// bottom.
    pub(crate) fn is_backward(self) -> bool {
        matches!(self, Travel::Leftward | Travel::Upward)
    }
}

/// What a placement did, for a test to hold its result against: the
/// walk's positions before any stretch was reversed, and the stretches.
#[derive(Clone, Debug, Default)]
pub(crate) struct Trace {
    /// The placement as walked: the same glyphs, in the same order, as
    /// the result, at the positions step 1 gave them.
    pub(crate) walked: Vec<PlacedGlyph>,
    /// The direction each laid cluster's step was taken in, in order.
    pub(crate) chords: Vec<Vec2>,
    /// The stretches as laid, every boundary moved to a word break.
    pub(crate) stretches: Vec<TracedStretch>,
    /// Each stretch's glyphs as the walk classified them, before any
    /// boundary was moved to a word break: where words were torn.
    pub(crate) walked_stretches: Vec<Range<usize>>,
    /// A text frame's lines (ruling R27): none for a path walked whole.
    pub(crate) lines: Vec<TracedLine>,
    /// Each cluster's step as laid, in the order laid: where it starts
    /// along the path (along its half, for a text frame's line) and how
    /// long it is.
    pub(crate) laid: Vec<(f32, f32)>,
    /// For a text frame, the half each of [`laid`](Self::laid)'s steps is
    /// on, 0 the top and 1 the bottom; empty for a path walked whole.
    pub(crate) laid_on: Vec<usize>,
}

/// One line of a text frame ([`Trace::lines`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TracedLine {
    /// The half it is on: 0 the top, 1 the bottom ([`frame_halves`]).
    pub(crate) half: usize,
    /// Its glyphs: indices into the placement.
    pub(crate) glyphs: Range<usize>,
    /// Where along its half it starts and ends.
    pub(crate) from: f32,
    pub(crate) to: f32,
    /// Whether it was cut to fit its span.
    pub(crate) cut: bool,
}

/// One stretch of a [`Trace`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TracedStretch {
    pub(crate) travel: Travel,
    /// Its glyphs: indices into the placement.
    pub(crate) glyphs: Range<usize>,
    /// The part of the path it holds: from its first step's start to its
    /// last step's end.
    pub(crate) from: f32,
    pub(crate) to: f32,
}

/// Why a lay stopped.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Laid {
    /// Every cluster fit; the next would start at this distance.
    Fits(f32),
    /// A cluster's step would have ended past the path.
    Overran,
    /// Placing a cluster would have passed [`MAX_PLACED_GLYPHS`].
    Capped,
}

/// The extent a `w × h` box spans along unit direction `dir`.
fn extent(dir: Vec2, w: f32, h: f32) -> f32 {
    dir.x.abs() * w + dir.y.abs() * h
}

/// The walk: a path, the run's line height and whether glyphs are
/// pushed outside the outline.
#[derive(Copy, Clone)]
struct PathWalk<'p> {
    path: &'p ArcPath,
    /// `h`: every cluster's box height and vertical extent.
    line_height: f32,
    outward: bool,
    /// How far along the path a step may end: `P` plus [`end_slack`].
    limit: f32,
    /// Whether steps are **level** (a text frame's line, ruling R28): a
    /// cluster's step, and a gap, run the path until it has moved that
    /// far horizontally, rather than by its box's extent along the chord
    /// ([`step`](Self::step)).
    level: bool,
    /// Whether the flow is an ornament, with no letter or digit
    /// ([`GlyphMetrics::letter`]): its spaces are then boxes as tall as
    /// they are wide ([`height`](Self::height)), so a ring's rhythm holds
    /// where the outline turns steep (ruling R29).
    ornament: bool,
}

impl PathWalk<'_> {
    /// The direction of the chord from `p(s)` to `p(s + len)`, or the
    /// direction of travel at `s` where the chord has no length.
    fn chord(&self, s: f32, len: f32) -> Vec2 {
        let (from, tangent) = self.path.at(s);
        let (to, _) = self.path.at(s + len);
        (to - from).try_normalize().unwrap_or(tangent)
    }

    /// The step a `w × h` cluster takes from `s`, and the direction θ it
    /// is taken in (#239 §5 step 2, ruling R9.2).
    ///
    /// The seed is the extent along the direction of travel at `s`; each
    /// of [`CHORD_ROUNDS`] rounds then re-takes θ from the chord the step
    /// spans. Where the path is straight, or smooth at the scale of one
    /// step, that is the direction of travel. At a sharp corner the chord
    /// turns with the corner: a step that straddles it blends the two
    /// sides by how much of it lies on each.
    ///
    /// Every round is continuous in `s`. The seed jumps only when `s` is
    /// exactly at a corner, and the chord from a corner runs along the
    /// side after it whatever its length, so the jump does not survive
    /// the first round. (A seed taken half a step on would jump while the
    /// step still straddles the corner, and two chord rounds leave a
    /// visible jump: `test_flow_place_corner_moves_continuously` sees
    /// 3.1 pt.) So a cluster's position, and the end of a run of steps,
    /// move continuously as the run moves along, which is what lets
    /// `Spread`'s and `Fit`'s end conditions be solved on a sharp outline.
    fn step(&self, s: f32, w: f32, h: f32) -> (f32, Vec2) {
        if self.level {
            let step = self.level_len(s, w);
            return (step, self.chord(s, step));
        }
        let mut step = extent(self.path.at(s).1, w, h);
        for _ in 0..CHORD_ROUNDS {
            step = extent(self.chord(s, step), w, h);
        }
        (step, self.chord(s, step))
    }

    /// The height of `cluster`'s box, of a flow whose line height is `h`:
    /// `h`, but for a space of an ornament, which is its advance (at most
    /// `h`). A space has no ink to keep apart; boxed a line high it would
    /// step a whole line wherever the path runs steep, and a glyph-and-
    /// space ornament would open a gap at every turn of a ring (ruling
    /// R29). In text a space keeps its line height, which is what parts
    /// two words stacked on a steep side.
    fn height(&self, cluster: &Cluster, h: f32) -> f32 {
        if self.ornament && cluster.blank {
            cluster.advance.min(h)
        } else {
            h
        }
    }

    /// How far along the path from `s` it moves `d` horizontally, on a
    /// level walk (ruling R28): `L` with `x(s + L) − x(s) = d`. A text
    /// frame's line lies where the path runs left to right within 45° of
    /// level (`cos θ ≥ √½`), so `L` is at most `√2 · d` there, and `x` is
    /// increasing; it is solved by Newton's method on `x`, its slope
    /// held at no less than a tenth so a step run past the span stays
    /// bounded. Never negative.
    ///
    /// Cost: [`LEVEL_ROUNDS`] path lookups.
    fn level_len(&self, s: f32, d: f32) -> f32 {
        if d.is_nan() || d <= 0.0 {
            return 0.0;
        }
        let (from, tangent) = self.path.at(s);
        let mut len = d / tangent.x.abs().max(0.1);
        for _ in 0..LEVEL_ROUNDS {
            let (at, tangent) = self.path.at(s + len);
            let miss = at.x - from.x - d;
            len = (len - miss / tangent.x.max(0.1)).max(0.0);
        }
        len
    }

    /// Where along the path a gap of `gap` after a step ending at `end`
    /// ends: `gap` further on, or, on a level walk, where the path has
    /// moved `gap` horizontally (ruling R28).
    fn after_gap(&self, end: f32, gap: f32) -> f32 {
        if self.level {
            end + self.level_len(end, gap)
        } else {
            end + gap
        }
    }

    /// The top-left corner of a `w × h` cluster box whose step `step`,
    /// taken in direction `dir`, starts at `s` (#239 §5 steps 3 and 4).
    fn corner(&self, s: f32, step: f32, dir: Vec2, w: f32, h: f32) -> Vec2 {
        if self.level {
            // Its left edge where its step starts, its center on the path
            // halfway across, and pushed off the outline straight up or
            // down — never sideways, which would undo the level spacing —
            // as far as clears the outline along its normal.
            let left = self.path.at(s).0.x;
            let (mut center, _) = self.path.at(s + self.level_len(s, w * 0.5));
            if self.outward {
                let normal = self.path.outward(dir);
                let rise = extent(normal, w * 0.5, h * 0.5) / normal.y.abs().max(0.1);
                center.y += rise * normal.y.signum();
            }
            return Vec2::new(left, center.y - h * 0.5);
        }
        let (mut center, _) = self.path.at(s + step * 0.5);
        if self.outward {
            let normal = self.path.outward(dir);
            center += normal * extent(normal, w * 0.5, h * 0.5);
        }
        center - Vec2::new(w, h) * 0.5
    }

    /// Where the last cluster's step ends when every one of `clusters`
    /// is walked from 0 with `gap` between steps, at `scale` times its
    /// size, cutting nothing: the natural length `L` at `gap = 0`.
    /// Advances and line heights are linear in size (cosmic-text's
    /// default hinting is off), so `L(scale)` needs no reshape.
    fn measure(&self, clusters: &[Cluster], scale: f32, gap: f32) -> f32 {
        self.measure_clusters(clusters.iter(), scale, gap)
    }

    /// Where the last of `clusters`' steps ends, walked from 0 with `gap`
    /// between steps at `scale` times their size, each box as tall as
    /// [`height`](Self::height) says: the arithmetic of
    /// [`lay`](Self::lay), without placing anything.
    fn measure_clusters<'c>(&self, clusters: impl Iterator<Item = &'c Cluster>, scale: f32, gap: f32) -> f32 {
        let h = self.line_height * scale;
        let mut s = 0.0_f32;
        let mut end = 0.0_f32;
        for cluster in clusters {
            let w = cluster.advance * scale;
            let ch = if self.ornament && cluster.blank {
                w.min(h)
            } else {
                h
            };
            end = s + self.step(s, w, ch).0;
            s = end + gap;
        }
        end
    }

    /// `L(scale)` as a reshape at `scale` will lay `clusters` out
    /// (ruling R9.3): each glyph's advance, and the line height, scaled
    /// by [`size_factor`] rather than by `scale` itself, because the
    /// reshape floors every size at [`MIN_FONT_SIZE_PT`]. In a run of
    /// mixed sizes, a small span stops shrinking at the floor while a
    /// large one still shrinks, so `L` is not `scale` times `L(1)`
    /// there, and `Fit` must not predict it as if it were. `largest` is
    /// the largest glyph size of the run, whose line height the run's is.
    ///
    /// `L` is the length the clusters take **as they are read** (#291,
    /// [`measure_readable`](Self::measure_readable)), so the scale `Fit`
    /// settles on fits the placement the readable rule makes of it.
    ///
    /// Cost: O(glyphs); a few `Vec`s of one entry per cluster.
    fn measure_at_scale(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        scale: f32,
        largest: f32,
    ) -> f32 {
        let widths: Vec<f32> = clusters
            .iter()
            .map(|cluster| {
                glyphs[cluster.glyphs.clone()]
                    .iter()
                    .map(|glyph| glyph.advance * size_factor(glyph.size, scale))
                    .sum::<f32>()
            })
            .collect();
        let breaks: Vec<bool> = clusters.iter().map(|cluster| cluster.word_break).collect();
        self.measure_readable(&widths, &breaks, self.line_height * size_factor(largest, scale))
    }

    /// Where the last of these cluster widths' steps ends when they are
    /// walked from 0 with no gap at line height `h`, and then laid again
    /// in reading order (#291) — the two phases of
    /// [`readable`](Self::readable), arithmetically — whichever ends
    /// further: both must fit, the walk to place every cluster and the
    /// reading-order lay to keep them all.
    ///
    /// Cost: two walks; two `Vec`s of one entry per cluster, and a third
    /// for the reading order when a stretch is reversed.
    fn measure_readable(&self, widths: &[f32], breaks: &[bool], h: f32) -> f32 {
        let mut steps = Vec::with_capacity(widths.len());
        let mut s = 0.0_f32;
        for (&width, &word_break) in widths.iter().zip(breaks) {
            let (step, dir) = self.step(s, width, h);
            steps.push(Stepped {
                first: 0,
                end: 0,
                s,
                step,
                dir,
                advance: width,
                corner: Vec2::ZERO,
                reading: (0, 0),
                word_break,
                letter: true,
            });
            s += step;
        }
        let walked = steps.last().map_or(0.0, |step| step.s + step.step);
        let mut stretches = self.stretches(&steps, h, walked, true);
        let astray = self.borrow_room(&mut stretches, |index| widths[index], |_| true, walked, h);
        if !stretches.iter().any(PathStretch::is_reversed) {
            return walked;
        }
        let read = self.lay_reading(
            &stretches,
            |index| widths[index],
            |_| true,
            Spacing::Zero,
            0.0,
            walked,
            h,
            |_, _, _, _| {},
        );
        self.crowded(walked.max(read), astray)
    }

    /// How long a lay ending at `end` counts for `Fit`, when `astray` of it
    /// ran on into stretches read the other way
    /// ([`borrow_room`](Self::borrow_room)): as it is, when none did; otherwise past the
    /// path's end by that much, so that `Fit`, which keeps every lay
    /// within the path, shrinks until no cluster reads backward — though
    /// there be room for them all elsewhere along the path (ruling
    /// R25.1).
    fn crowded(&self, end: f32, astray: f32) -> f32 {
        if astray > SOLVE_TOLERANCE {
            end.max(self.path.length()) + astray
        } else {
            end
        }
    }

    /// The stretches of a placement (#291): each maximal run of steps
    /// with one [`Travel`] ([`travels`](Self::travels)), every boundary
    /// then moved to a word break ([`snap_to_words`](Self::snap_to_words)).
    /// `end` is where the last stretch's part of the path ends. `force`
    /// moves every torn word whether its taker finds room or not: `Fit`'s
    /// estimates, so it shrinks until the words have room.
    ///
    /// Cost: O(steps); one entry per stretch.
    fn stretches(&self, steps: &[Stepped], h: f32, end: f32, force: bool) -> Vec<PathStretch> {
        let mut stretches: Vec<PathStretch> = Vec::new();
        for (index, travel) in self.travels(steps, h).into_iter().enumerate() {
            match stretches.last_mut() {
                Some(last) if last.travel == travel => last.steps.end = index + 1,
                _ => stretches.push(PathStretch {
                    travel,
                    steps: index..index + 1,
                    from: steps[index].s,
                }),
            }
        }
        self.snap_to_words(&mut stretches, steps, h, end, force);
        stretches
    }

    /// Move each boundary between two stretches to a word break, so a word
    /// lies wholly in one stretch (ruling R26). A word torn by a boundary
    /// is torn far apart when one of the two stretches is placed reversed:
    /// that one begins or ends its text at its other end, so the word's
    /// head would sit at one end of the stretch and its tail at the other.
    ///
    /// A boundary inside a word moves to the word's start or its end —
    /// whichever takes fewer clusters across, the end on a tie — and the
    /// clusters it passes join the other stretch. A word's punctuation
    /// travels with it: only a word break parts the text ([`GlyphMetrics::
    /// word_break`]), and "heaven:" has none before its colon. The stretch
    /// that grows may then not fit its part of the path, and the boundary's
    /// place on the path does not move with it ([`PathStretch::from`]): a
    /// stretch started where the moved word started would reach into its
    /// neighbor's part of the path and, read the other way, read backward
    /// there. It takes the room from a neighbor read its own way instead
    /// ([`PathWalk::borrow_room`]).
    ///
    /// A boundary moves at most that one word, and never past the middle of
    /// either stretch: a move takes at most half of a stretch's clusters.
    /// The first and the last stretch are the exception — the ones beside
    /// the path's start, where the text begins and ends — which may give up
    /// all but one cluster, so a flow's last word and its punctuation
    /// ("handywork.", "heaven:") are never left with their tail alone at
    /// the path's start while a space after them stays. A word too long to
    /// move so stays parted at the boundary: the walk's split, each part
    /// still reading forward. So does a word whose taker would find no
    /// room for it but by running on into a stretch read the other way
    /// ([`finds_room`](Self::finds_room)): parted, each part reads
    /// forward; crowded in, the overflow would not. Boundaries are moved
    /// in order, each against its neighbors as already moved.
    ///
    /// Cost: O(steps): each boundary looks no further than its word.
    fn snap_to_words(&self, stretches: &mut [PathStretch], steps: &[Stepped], h: f32, end: f32, force: bool) {
        let last = stretches.len().saturating_sub(1);
        for k in 1..stretches.len() {
            let (before, after) = (stretches[k - 1].steps.clone(), stretches[k].steps.clone());
            let at = after.start;
            if at >= after.end || steps[at].word_break {
                continue;
            }
            // The straddling word's start, in `before`, and its end, in
            // `after` (or the end of the placement).
            let back = (before.start..at).rev().find(|&i| steps[i].word_break);
            let forward = (at + 1..after.end)
                .find(|&i| steps[i].word_break)
                .unwrap_or(after.end);
            let room = |range: &Range<usize>, edge: bool| {
                if edge {
                    range.len().saturating_sub(1)
                } else {
                    range.len() / 2
                }
            };
            let back = back.filter(|&start| at - start <= room(&before, k - 1 == 0));
            let forward = Some(forward).filter(|&end| end - at <= room(&after, k == last));
            let moves: [Option<usize>; 2] = match (back, forward) {
                (Some(start), Some(end)) if at - start < end - at => [Some(start), Some(end)],
                (back, forward) => [forward, back],
            };
            // The first move, in that order, whose taker still finds room.
            let moved = moves
                .into_iter()
                .flatten()
                .find(|&moved| force || self.finds_room(stretches, k, moved, steps, h, end));
            let Some(moved) = moved else {
                continue;
            };
            stretches[k - 1].steps.end = moved;
            stretches[k].steps.start = moved;
        }
    }

    /// Whether moving the boundary before stretch `k` to step `moved`
    /// leaves the stretch that takes the word room for it with no gap
    /// ([`snap_to_words`](Self::snap_to_words)): its own part of the
    /// path, plus the slack a neighbor read its own way leaves — room
    /// that, borrowed, it reads forward in
    /// ([`borrow_room`](Self::borrow_room)). When the two stretches are read the same way
    /// they share their parts of the path. A move that crowds the taker no
    /// more than it already was is allowed too. Lengths are laid from
    /// where each stretch begins.
    ///
    /// Cost: one walk of each of the two stretches and of their outer
    /// neighbors, with no gap.
    #[allow(clippy::too_many_arguments)] // The boundary, the move, and the walk's parts.
    fn finds_room(
        &self,
        stretches: &[PathStretch],
        k: usize,
        moved: usize,
        steps: &[Stepped],
        h: f32,
        end: f32,
    ) -> bool {
        // How far `range` reaches laid from `s` with no gap.
        let reach = |range: Range<usize>, mut s: f32| {
            let from = s;
            for index in range {
                s += self.step(s, steps[index].advance, h).0;
            }
            s - from
        };
        let region_end = |i: usize| stretches.get(i + 1).map_or(end, |next| next.from);
        let way = |i: usize| stretches[i].travel.is_backward();
        // The slack stretch `i` leaves in its own part of the path.
        let slack = |i: usize| {
            let stretch = &stretches[i];
            (region_end(i) - stretch.from - reach(stretch.steps.clone(), stretch.from)).max(0.0)
        };
        let (a, b) = (k - 1, k);
        // The slack of stretch `i`'s neighbor on the far side, if it is
        // read the way `i` is.
        let beside = |i: Option<usize>, of: usize| match i {
            Some(i) if i < stretches.len() && way(i) == way(of) => slack(i),
            _ => 0.0,
        };
        let (room_before, room_after) = (beside(a.checked_sub(1), a), beside(Some(b + 1), b));
        let (before, after) = (stretches[a].steps.start..moved, moved..stretches[b].steps.end);
        if way(a) == way(b) {
            let needs = reach(before, stretches[a].from) + reach(after, stretches[b].from);
            return needs <= region_end(b) - stretches[a].from + room_before + room_after;
        }
        let (taker, range, room) = if moved < stretches[b].steps.start {
            (b, after, room_after)
        } else {
            (a, before, room_before)
        };
        let from = stretches[taker].from;
        let needs = reach(range, from);
        needs <= region_end(taker) - from + room || needs <= reach(stretches[taker].steps.clone(), from)
    }

    /// Give each stretch room for its clusters (ruling R26): where a
    /// stretch, laid with no gap from where it begins
    /// ([`PathStretch::from`]), would not end before the next begins,
    /// it takes the room it lacks from a neighbor **read the same way** —
    /// both reversed, or neither — since its clusters, run on into that
    /// neighbor's part of the path, read forward there too: first from
    /// the slack the stretch before leaves, if that one is read the same
    /// way, then by moving the next stretch's beginning on. A stretch read
    /// the other way from both its neighbors runs on into the next. What
    /// the last stretch lacks runs past `end`, and phase 2 lays the
    /// placement on one gap instead ([`Spacing::Uniform`]). Returns how
    /// much, all told, ran on into a stretch read the other way: clusters
    /// that, laid there, read backward. `Fit` shrinks until that is none.
    ///
    /// Cost: one walk of each stretch with no gap; no allocation.
    fn borrow_room(
        &self,
        stretches: &mut [PathStretch],
        width: impl Fn(usize) -> f32,
        kept: impl Fn(usize) -> bool,
        end: f32,
        h: f32,
    ) -> f32 {
        let mut astray = 0.0_f32;
        // Where a stretch's clusters end, laid from `s` with no gap.
        let length = |stretch: &PathStretch, mut s: f32| {
            let mut last = s;
            for index in stretch.steps.clone().filter(|&index| kept(index)) {
                last = s + self.step(s, width(index), h).0;
                s = last;
            }
            last
        };
        for k in 0..stretches.len() {
            let next = stretches.get(k + 1).map_or(end, |next| next.from);
            let lacks = length(&stretches[k], stretches[k].from) - next;
            if lacks.is_nan() || lacks <= 0.0 {
                continue;
            }
            let backward = stretches[k].travel.is_backward();
            let mut lacks = lacks;
            if k > 0 && stretches[k - 1].travel.is_backward() == backward {
                let before = &stretches[k - 1];
                let slack = stretches[k].from - length(before, before.from);
                let taken = lacks.min(slack.max(0.0));
                stretches[k].from -= taken;
                lacks -= taken;
            }
            if let Some(next) = stretches.get_mut(k + 1) {
                next.from += lacks.max(0.0);
                if next.travel.is_backward() != backward {
                    astray += lacks.max(0.0);
                }
            }
        }
        astray
    }

    /// Lay `stretches` in reading order (#291 phase 2, rulings R25,
    /// R26): stretch after stretch, a backward stretch's clusters last to
    /// first, each stretch from where it began in step 1
    /// ([`PathStretch::from`]) — or later, if the one before runs on past
    /// that — so it stays in its own part of the path. `spacing` says the
    /// gap between steps inside each stretch ([`Spacing`]); between two
    /// stretches the gap is step 1's, `walk_gap`. Clusters not `kept` are
    /// skipped; `width` gives each cluster's box width; `visit` sees each
    /// one laid: its index, where its step starts, the step and its
    /// direction. `end` is where the last stretch is to end. Returns where
    /// the last step ends (where the first stretch begins, when none is
    /// laid).
    ///
    /// Cost: one step per laid cluster, and per stretch at most
    /// `2 · SOLVE_ROUNDS` measures of it when its spacing is solved; no
    /// allocation.
    #[allow(clippy::too_many_arguments)] // A walk's parts, and what it reports.
    fn lay_reading(
        &self,
        stretches: &[PathStretch],
        width: impl Fn(usize) -> f32,
        kept: impl Fn(usize) -> bool,
        spacing: Spacing,
        walk_gap: f32,
        end: f32,
        h: f32,
        mut visit: impl FnMut(usize, f32, f32, Vec2),
    ) -> f32 {
        let order = |stretch: &PathStretch| {
            let (forward, backward) = if stretch.travel.is_backward() {
                (0..0, stretch.steps.clone())
            } else {
                (stretch.steps.clone(), 0..0)
            };
            forward.chain(backward.rev()).filter(|&index| kept(index))
        };
        // Where a stretch's last step ends, laid from `s` with `gap`.
        let length = |stretch: &PathStretch, mut s: f32, gap: f32| {
            let mut last = s;
            for index in order(stretch) {
                last = s + self.step(s, width(index), h).0;
                s = last + gap;
            }
            last
        };
        let (junction, anchored) = match spacing {
            Spacing::Zero => (0.0, true),
            Spacing::Uniform(gap) => (gap, false),
            Spacing::Keep(_) | Spacing::Fill => (walk_gap, true),
        };
        let mut cursor = stretches.first().map_or(0.0, |stretch| stretch.from);
        let mut last = cursor;
        for (k, stretch) in stretches.iter().enumerate() {
            if order(stretch).next().is_none() {
                continue;
            }
            let start = if anchored {
                cursor.max(stretch.from)
            } else {
                cursor
            };
            let target = stretches.get(k + 1).map_or(end, |next| next.from - junction);
            let fits = |gap: f32| length(stretch, start, gap) <= target;
            let gap = match spacing {
                Spacing::Zero => 0.0,
                Spacing::Uniform(gap) => gap,
                Spacing::Keep(gap) if fits(gap) => gap,
                Spacing::Keep(gap) if fits(0.0) => {
                    solve_gap(|g| length(stretch, start, g) - target, target, gap)
                }
                Spacing::Fill if fits(0.0) => {
                    let guess = (target - length(stretch, start, 0.0)) / stretch.steps.len().max(2) as f32;
                    solve_gap(|g| length(stretch, start, g) - target, target, guess)
                }
                _ => 0.0,
            };
            let mut s = start;
            // `Fill` ends a stretch at its part's end: one cluster, which no
            // gap can move, is laid there whole.
            let mut clusters = order(stretch);
            if let (Spacing::Fill, Some(only), None) = (spacing, clusters.next(), clusters.next()) {
                for _ in 0..CHORD_ROUNDS {
                    s = (target - self.step(s, width(only), h).0).max(start);
                }
            }
            for index in order(stretch) {
                let (step, dir) = self.step(s, width(index), h);
                visit(index, s, step, dir);
                last = s + step;
                s = last + gap;
            }
            cursor = last + junction;
        }
        last
    }

    /// Place `clusters` from `s` with `gap` between steps onto `out`,
    /// each whole: its glyphs share its box, each at its pen position.
    /// Stops before the first cluster whose step would end past the path,
    /// or that would pass [`MAX_PLACED_GLYPHS`], and says which.
    fn lay<'c>(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: impl IntoIterator<Item = &'c Cluster>,
        mut s: f32,
        gap: f32,
        out: &mut Laying,
    ) -> Laid {
        let h = self.line_height;
        for cluster in clusters {
            let ch = self.height(cluster, h);
            let (step, dir) = self.step(s, cluster.advance, ch);
            // The same arithmetic, in the same order, as `measure`: a
            // solved spacing that measures within the path must lay
            // within it, to the last bit.
            let end = s + step;
            if end > self.limit {
                return Laid::Overran;
            }
            if out.glyphs.len() + cluster.glyphs.len() > MAX_PLACED_GLYPHS {
                return Laid::Capped;
            }
            let corner = self.corner(s, step, dir, cluster.advance, ch);
            out.steps.push(Stepped {
                first: out.glyphs.len(),
                end: out.glyphs.len() + cluster.glyphs.len(),
                s,
                step,
                dir,
                advance: cluster.advance,
                corner,
                reading: cluster.reading,
                word_break: cluster.word_break,
                letter: cluster.letter,
            });
            for index in cluster.glyphs.clone() {
                out.glyphs.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: corner + Vec2::new(glyphs[index].x - cluster.left, 0.0),
                });
            }
            s = self.after_gap(end, gap);
        }
        Laid::Fits(s)
    }

    /// The readable rule (#291, rulings R25, R25.1, R26), phase 2: lay
    /// the clusters step 1 laid in **reading order** — every leftward and
    /// upward stretch's clusters reversed — so every stretch reads left to
    /// right or top to bottom.
    ///
    /// Step 1, the fit mode's walk in `walked`, decided which clusters
    /// are placed and which stretch each belongs to
    /// ([`stretches`](Self::stretches)), a torn word moved whole into one of its two
    /// stretches; that membership is frozen here. Each stretch is laid in
    /// its own part of the path, from where step 1 began it, borrowing
    /// room from a neighbor read its way where it needs more
    /// ([`borrow_room`](Self::borrow_room)), its spacing solved there ([`Spacing`]):
    ///
    /// - [`Ends::Exactly`] (`Spread`, `Fit`): each stretch ends where its
    ///   part does, so the last ends where step 1 ended;
    /// - [`Ends::Within`] (`Once`, `Repeat`): each keeps step 1's spacing
    ///   where that fits its part, and takes less where not.
    ///
    /// Where the stretches cannot each keep to their parts even with no
    /// gap, they are laid on one from the next with one spacing, solved
    /// for the whole placement as above ([`Spacing::Uniform`], ruling
    /// R25). Spacing never goes below a zero gap: glyphs never overlap.
    /// Where even a zero gap passes the path's end, the placement is cut,
    /// the way `Fit` cuts text too long at `min` (§6.4 step 4): whole
    /// filler copies, or repeat clusters, from the end first, and only
    /// then the text, its last-read cluster first (ruling R9.4). That is
    /// the one case in which the readable glyphs are not step 1's: they
    /// are step 1's less the clusters cut at the end.
    ///
    /// `laying.glyphs` keeps step 1's order — the text's — so a glyph's
    /// index means what it meant; only positions move (and a cut removes
    /// clusters).
    ///
    /// Cost: O(clusters) to classify, snap and borrow, and once per lay;
    /// each stretch's spacing solve and the pooled one take at most
    /// `2 · SOLVE_ROUNDS` lays of what they solve, a cut a logarithmic
    /// number more. A few `Vec`s of one entry per cluster or stretch;
    /// nothing beyond the classification when no stretch is reversed.
    fn readable(&self, walked: Walked, trace: Option<&mut Trace>) -> (Option<ShapedRun>, PlacedRun) {
        let Walked {
            run,
            laying,
            mut truncated,
            gap,
            ends,
            text,
            copy,
            slack,
            whole_words,
            even,
        } = walked;
        let steps = &laying.steps;
        let walked_end = steps.last().map_or(0.0, |step| step.s + step.step);
        let (spacing, end) = match ends {
            Ends::Exactly => (Spacing::Fill, walked_end),
            Ends::Within => (
                Spacing::Keep(gap),
                self.runs_on(steps, self.line_height, walked_end),
            ),
        };
        let mut stretches = self.stretches(steps, self.line_height, end, whole_words);
        let mut trace = trace;
        if let Some(trace) = trace.as_deref_mut() {
            let mut walked_stretches: Vec<Range<usize>> = Vec::new();
            let travels = self.travels(steps, self.line_height);
            for (index, step) in steps.iter().enumerate() {
                match walked_stretches.last_mut() {
                    Some(range) if travels[index] == travels[index - 1] => range.end = step.end,
                    _ => walked_stretches.push(step.first..step.end),
                }
            }
            trace.walked_stretches = walked_stretches;
            trace.walked = laying.glyphs.clone();
            trace.chords = steps.iter().map(|step| step.dir).collect();
            trace.stretches = traced_stretches(&stretches, steps);
        }
        // A flow with no letter or digit — an ornament — has no reading
        // direction: it is placed as walked (ruling R29), its rhythm whole
        // where a reversed stretch would turn it about.
        let reads = steps.iter().any(|step| step.letter);
        if !reads || !stretches.iter().any(PathStretch::is_reversed) {
            if let Some(trace) = trace {
                trace.laid = steps.iter().map(|step| (step.s, step.step)).collect();
            }
            let placed = PlacedRun {
                glyphs: laying.glyphs,
                truncated,
            };
            return (run, placed);
        }
        let limit = (self.limit + slack).max(walked_end);
        self.borrow_room(
            &mut stretches,
            |index| steps[index].advance,
            |_| true,
            end,
            self.line_height,
        );
        let even_gap = if even {
            let gap = self.even_out(&mut stretches, steps);
            if let Some(trace) = trace.as_deref_mut() {
                trace.stretches = traced_stretches(&stretches, steps);
            }
            Some(gap)
        } else {
            None
        };
        let mut kept = vec![true; steps.len()];
        let width = |index: usize| steps[index].advance;
        let h = self.line_height;
        // Where the kept clusters end, laid with `spacing`.
        let measure = |spacing: Spacing, kept: &[bool]| {
            let keep = |index: usize| kept[index];
            self.lay_reading(&stretches, width, keep, spacing, gap, end, h, |_, _, _, _| {})
        };
        let spacing = if let Some(g) = even_gap {
            Spacing::Uniform(g)
        } else if measure(spacing, &kept) <= limit {
            spacing
        } else {
            // Some stretch cannot keep its own part of the path: lay them
            // on one from the next with one gap, so the slack of every
            // stretch serves any (ruling R25).
            let pooled = |gap: f32| measure(Spacing::Uniform(gap), &kept);
            let gap = match ends {
                Ends::Exactly if pooled(0.0) <= walked_end => {
                    solve_gap(|g| pooled(g) - walked_end, walked_end, gap)
                }
                Ends::Within if pooled(gap) <= limit => gap,
                Ends::Within if pooled(0.0) <= end => solve_gap(|g| pooled(g) - end, end, gap),
                _ => 0.0,
            };
            if pooled(gap) > limit {
                let shortest = |kept: &[bool]| measure(Spacing::Uniform(0.0), kept);
                truncated |= self.cut(steps, text, copy, &mut kept, &shortest, limit);
            }
            Spacing::Uniform(gap)
        };
        let lay = Lay {
            stretches: &stretches,
            kept: &kept,
            spacing,
            walk_gap: gap,
            end,
        };
        if let Some(trace) = trace {
            let mut laid = Vec::with_capacity(steps.len());
            let width = |index: usize| steps[index].advance;
            let keep = |index: usize| kept[index];
            self.lay_reading(&stretches, width, keep, spacing, gap, end, h, |_, s, step, _| {
                laid.push((s, step));
            });
            trace.laid = laid;
        }
        self.lay_readable(laying, &lay, run, truncated)
    }

    /// A closed ring's one gap, read as it is laid (ruling R29): the gap
    /// that, between every two clusters in reading order and at the seam
    /// back to the first, lays the ring exactly once round — and the
    /// stretches regrouped so that every cluster lies where the path runs
    /// its own stretch's way. Reversing a stretch changes which cluster
    /// straddles a corner, and so the stretch's length; laid on one gap,
    /// the stretches drift against the parts step 1 gave them, and a
    /// cluster that drifts past a corner would be read the other way. So
    /// the ring is laid, each laid cluster classified where it lies
    /// ([`travels`](Self::travels)), and each boundary moved past the
    /// clusters that lie on its neighbor's side — a ring's clusters are
    /// copies of one pattern, so which ones a stretch holds matters only
    /// in number — and laid again, until none has drifted, a bounded
    /// number of rounds.
    ///
    /// Cost: per round, a solve of at most `2 · SOLVE_ROUNDS` lays, one
    /// lay more, a sort and a classification; [`EVEN_ROUNDS`] rounds.
    fn even_out(&self, stretches: &mut [PathStretch], steps: &[Stepped]) -> f32 {
        let length = self.path.length();
        let h = self.line_height;
        let width = |index: usize| steps[index].advance;
        // The gap that closes the ring as `stretches` group it.
        let close = |stretches: &[PathStretch]| {
            let seam = |g: f32| {
                self.lay_reading(
                    stretches,
                    width,
                    |_| true,
                    Spacing::Uniform(g),
                    0.0,
                    length,
                    h,
                    |_, _, _, _| {},
                ) + g
                    - length
            };
            let guess = (-seam(0.0) / steps.len().max(1) as f32).max(0.0);
            solve_gap(seam, length, guess)
        };
        let mut gap = close(stretches);
        for _ in 0..EVEN_ROUNDS {
            // Each cluster as laid, in path order, and its travel there.
            let mut laid: Vec<(usize, Stepped)> = Vec::with_capacity(steps.len());
            self.lay_reading(
                stretches,
                width,
                |_| true,
                Spacing::Uniform(gap),
                0.0,
                length,
                h,
                |index, s, step, dir| {
                    laid.push((
                        index,
                        Stepped {
                            s,
                            step,
                            dir,
                            ..steps[index]
                        },
                    ));
                },
            );
            laid.sort_by(|a, b| a.1.s.total_cmp(&b.1.s));
            let probe: Vec<Stepped> = laid.iter().map(|(_, step)| *step).collect();
            let mut travel = vec![Travel::Rightward; steps.len()];
            for ((index, _), lies) in laid.iter().zip(self.travels(&probe, h)) {
                travel[*index] = lies;
            }
            let mut moved = false;
            for k in 1..stretches.len() {
                let (way_before, way) = (stretches[k - 1].travel, stretches[k].travel);
                // Stretch k's clusters in path order, and k − 1's.
                let path_order = |stretch: &PathStretch| -> Vec<usize> {
                    if stretch.travel.is_backward() {
                        stretch.steps.clone().rev().collect()
                    } else {
                        stretch.steps.clone().collect()
                    }
                };
                let ahead = path_order(&stretches[k]);
                let behind = path_order(&stretches[k - 1]);
                let lead = ahead
                    .iter()
                    .take_while(|&&index| travel[index] == way_before)
                    .count();
                let trail = behind
                    .iter()
                    .rev()
                    .take_while(|&&index| travel[index] == way)
                    .count();
                if lead > 0 && lead < ahead.len() {
                    stretches[k - 1].steps.end += lead;
                    stretches[k].steps.start += lead;
                    moved = true;
                } else if trail > 0 && trail < behind.len() {
                    stretches[k - 1].steps.end -= trail;
                    stretches[k].steps.start -= trail;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
            gap = close(stretches);
        }
        gap
    }

    /// Each step's [`Travel`] (#291 step 2).
    ///
    /// A step is rightward when `cos θ > ε`, leftward when `cos θ < −ε`
    /// ([`VERTICAL_EPSILON`], cos 45°); otherwise it is steep. A run of
    /// steep steps is a vertical stretch — downward or upward by where it
    /// ends against where it starts — only when it is clearly entered: it
    /// spans [`VERTICAL_MIN_LINES`] line heights, or it is every step
    /// there is. A shorter run is the hysteresis's case, a tight turn
    /// through vertical: it makes no stretch of its own, and each of its
    /// steps joins the horizontal stretch whose way it moves (`cos θ > 0`
    /// rightward, `< 0` leftward), so the boundary between the stretches
    /// before and after it sits where the turn passes plumb. A step with
    /// no horizontal motion at all joins the stretch that reads it top to
    /// bottom: rightward going down, leftward going up.
    ///
    /// Cost: O(steps); one allocation.
    fn travels(&self, steps: &[Stepped], line_height: f32) -> Vec<Travel> {
        let near_vertical = |dir: Vec2| dir.x.abs() <= VERTICAL_EPSILON;
        let horizontal = |dir: Vec2| {
            if dir.x > 0.0 || (dir.x == 0.0 && dir.y >= 0.0) {
                Travel::Rightward
            } else {
                Travel::Leftward
            }
        };
        let mut travels = Vec::with_capacity(steps.len());
        let mut i = 0;
        while i < steps.len() {
            if !near_vertical(steps[i].dir) {
                travels.push(horizontal(steps[i].dir));
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < steps.len() && near_vertical(steps[j].dir) {
                j += 1;
            }
            let run = &steps[i..j];
            let span = run[run.len() - 1].s + run[run.len() - 1].step - run[0].s;
            let every_step = i == 0 && j == steps.len();
            if every_step || span >= VERTICAL_MIN_LINES * line_height {
                let fall: f32 = run.iter().map(|step| step.dir.y * step.step).sum();
                let travel = if fall >= 0.0 {
                    Travel::Downward
                } else {
                    Travel::Upward
                };
                travels.extend(std::iter::repeat_n(travel, run.len()));
            } else {
                travels.extend(run.iter().map(|step| horizontal(step.dir)));
            }
            i = j;
        }
        travels
    }

    /// Where the last stretch's part of the path ends when the text ends
    /// short of the path, at `walked_end` (#291): as far as the path runs
    /// on the way that stretch is read, and no further. Past that, the
    /// path turns — the bottom of a U-turn, the middle of an S — and a
    /// cluster moved on into the turn
    /// ([`snap_to_words`](Self::snap_to_words), [`borrow_room`](Self::borrow_room)) would be
    /// laid where the path runs the other way and read backward. The path
    /// beyond `walked_end` is stepped one line height at a time and
    /// classified with the walk's own steps before it
    /// ([`travels`](Self::travels)), so a short turn through vertical joins the
    /// stretches beside it as it would in the walk. The path's end when
    /// it runs the last stretch's way all along, or nothing was walked.
    ///
    /// Cost: one step per line height of path past `walked_end`, and one
    /// classification of the walk's steps and those; one `Vec`.
    fn runs_on(&self, steps: &[Stepped], h: f32, walked_end: f32) -> f32 {
        let length = self.path.length();
        let Some(last) = steps.last() else {
            return length;
        };
        if h.is_nan() || h <= 0.0 || walked_end >= length {
            return length;
        }
        let mut probe: Vec<Stepped> = steps.to_vec();
        let mut s = walked_end;
        while s < length && probe.len() < steps.len() + MAX_RUN_ON_STEPS {
            let (step, dir) = self.step(s, h, h);
            if step.is_nan() || step <= 0.0 {
                break;
            }
            probe.push(Stepped {
                s,
                step,
                dir,
                ..*last
            });
            s += step;
        }
        let travels = self.travels(&probe, h);
        let way = travels[steps.len() - 1];
        travels[steps.len()..]
            .iter()
            .zip(&probe[steps.len()..])
            .find(|(travel, _)| **travel != way)
            .map_or(length, |(_, step)| step.s)
    }

    /// Where step 1's `steps`, laid again in reading order with no gap,
    /// end, and how far phase 2 lets them go: the end of step 1's walk,
    /// or the path's limit if that is further
    /// ([`readable`](Self::readable)). The first past the second is a placement phase
    /// 2 must cut.
    ///
    /// Cost: one classification and one walk; nothing allocated beyond
    /// them when no stretch is reversed.
    fn readable_extent(&self, steps: &[Stepped], slack: f32) -> (f32, f32) {
        let walked_end = steps.last().map_or(0.0, |step| step.s + step.step);
        let limit = (self.limit + slack).max(walked_end);
        let mut stretches = self.stretches(steps, self.line_height, walked_end, true);
        if !stretches.iter().any(PathStretch::is_reversed) {
            return (walked_end, limit);
        }
        let astray = self.borrow_room(
            &mut stretches,
            |index| steps[index].advance,
            |_| true,
            walked_end,
            self.line_height,
        );
        let width = |index: usize| steps[index].advance;
        let read = self.lay_reading(
            &stretches,
            width,
            |_| true,
            Spacing::Zero,
            0.0,
            walked_end,
            self.line_height,
            |_, _, _, _| {},
        );
        (self.crowded(read, astray), limit)
    }

    /// Cut the readable placement until, at a zero gap, it ends within
    /// `limit` (ruling R25): drop whole `copy`-cluster units of the
    /// clusters after the first `text` — filler copies, or repeat
    /// clusters — from the end first, then the text's clusters, last
    /// read first (ruling R9.4). Marks the dropped clusters in `kept` and
    /// says whether text was cut. `measure` is the readable lay with no
    /// gap, the shortest it can be.
    ///
    /// Cost: a binary search over the units, one readable walk and one
    /// pass over `kept` per probe: O(clusters · log clusters).
    #[allow(clippy::too_many_arguments)] // The walk's parts, as `readable` holds them.
    fn cut(
        &self,
        steps: &[Stepped],
        text: usize,
        copy: usize,
        kept: &mut [bool],
        measure: &dyn Fn(&[bool]) -> f32,
        limit: f32,
    ) -> bool {
        let text = text.min(steps.len());
        // The clusters in the order they are dropped, and where each unit
        // of that order ends.
        let mut drop: Vec<usize> = Vec::with_capacity(steps.len());
        let mut units: Vec<usize> = Vec::new();
        let mut end = steps.len();
        while end > text {
            let begin = end.saturating_sub(copy.max(1)).max(text);
            drop.extend(begin..end);
            units.push(drop.len());
            end = begin;
        }
        let copy_units = units.len();
        let mut read: Vec<usize> = (0..text).collect();
        read.sort_by_key(|&index| std::cmp::Reverse(steps[index].reading));
        for index in read {
            drop.push(index);
            units.push(drop.len());
        }
        let keep_all_but = |count: usize, kept: &mut [bool]| {
            kept.fill(true);
            for &index in &drop[..count] {
                kept[index] = false;
            }
        };
        // `over` drops units that still overrun (none, just seen);
        // `fits` drops units that end within `limit` (all of them do).
        let (mut over, mut fits) = (0usize, units.len());
        while fits - over > 1 {
            let middle = over + (fits - over) / 2;
            keep_all_but(units[middle - 1], kept);
            if measure(kept) <= limit {
                fits = middle;
            } else {
                over = middle;
            }
        }
        keep_all_but(if fits == 0 { 0 } else { units[fits - 1] }, kept);
        fits > copy_units
    }

    /// Lay `laying`'s clusters as `lay` says
    /// ([`lay_reading`](Self::lay_reading)): each cluster's glyphs move with its box, and
    /// a cluster not kept is removed. The glyphs keep step 1's order.
    fn lay_readable(
        &self,
        mut laying: Laying,
        lay: &Lay<'_>,
        run: Option<ShapedRun>,
        truncated: bool,
    ) -> (Option<ShapedRun>, PlacedRun) {
        let h = self.line_height;
        let steps = &laying.steps;
        let glyphs = &mut laying.glyphs;
        let width = |index: usize| steps[index].advance;
        self.lay_reading(
            lay.stretches,
            width,
            |index| lay.kept[index],
            lay.spacing,
            lay.walk_gap,
            lay.end,
            h,
            |index, s, length, dir| {
                let step = steps[index];
                let moved = self.corner(s, length, dir, step.advance, h) - step.corner;
                for glyph in &mut glyphs[step.first..step.end] {
                    glyph.pos += moved;
                }
            },
        );
        let kept = lay.kept;
        let glyphs = if kept.iter().all(|&keep| keep) {
            laying.glyphs
        } else {
            laying
                .steps
                .iter()
                .zip(kept)
                .filter(|(_, &keep)| keep)
                .flat_map(|(step, _)| laying.glyphs[step.first..step.end].iter().copied())
                .collect()
        };
        (run, PlacedRun { glyphs, truncated })
    }

    /// [`lay`](Self::lay) `clusters`; when they overrun, lay instead the
    /// longest prefix of them in reading order that fits, in visual order
    /// (ruling R9.4). Returns how the final lay stopped and whether text
    /// was cut.
    ///
    /// Cost: one lay when everything fits; otherwise a sort of the
    /// clusters by reading order and a binary search over the prefix
    /// length, O(log clusters) more lays.
    fn lay_or_cut(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        s: f32,
        gap: f32,
        out: &mut Laying,
    ) -> (Laid, bool) {
        let mark = out.mark();
        match self.lay(glyphs, clusters, s, gap, out) {
            Laid::Overran => {}
            laid => return (laid, false),
        }
        let mut order: Vec<usize> = (0..clusters.len()).collect();
        order.sort_by_key(|&index| clusters[index].reading);
        let mut rank = vec![0usize; clusters.len()];
        for (position, &index) in order.iter().enumerate() {
            rank[index] = position;
        }
        let lay_prefix = |keep: usize, out: &mut Laying| {
            out.truncate(mark);
            let kept = clusters
                .iter()
                .zip(&rank)
                .filter(|(_, &r)| r < keep)
                .map(|(c, _)| c);
            self.lay(glyphs, kept, s, gap, out)
        };
        // `fits` keeps a prefix that fits (none trivially does); `overruns`
        // one that does not (all of them, just seen).
        let (mut fits, mut overruns) = (0usize, clusters.len());
        while overruns - fits > 1 {
            let middle = fits + (overruns - fits) / 2;
            match lay_prefix(middle, out) {
                Laid::Fits(_) => fits = middle,
                Laid::Overran | Laid::Capped => overruns = middle,
            }
        }
        (lay_prefix(fits, out), true)
    }
}

/// The stretches as a [`Trace`] reports them: each non-empty one's
/// travel, glyphs, and the part of the path step 1 gave it.
fn traced_stretches(stretches: &[PathStretch], steps: &[Stepped]) -> Vec<TracedStretch> {
    stretches
        .iter()
        .filter(|stretch| !stretch.steps.is_empty())
        .map(|stretch| {
            let (head, tail) = (steps[stretch.steps.start], steps[stretch.steps.end - 1]);
            TracedStretch {
                travel: stretch.travel,
                glyphs: head.first..tail.end,
                from: head.s,
                to: tail.s + tail.step,
            }
        })
        .collect()
}

/// One stretch of a placement as phase 2 lays it (#291).
#[derive(Clone, Debug)]
struct PathStretch {
    travel: Travel,
    /// Its clusters: a range of step indices, in step 1's order.
    steps: Range<usize>,
    /// Where along the path it begins: where step 1 began its first
    /// cluster, before [`PathWalk::snap_to_words`] moved any word across its
    /// boundary. Phase 2 starts it here — earlier only where it borrows
    /// room from the stretch before ([`PathWalk::borrow_room`]), later
    /// where the stretch before runs on past it.
    from: f32,
}

impl PathStretch {
    /// Whether it is laid reversed and that changes anything: a backward
    /// stretch of more than one cluster.
    fn is_reversed(&self) -> bool {
        self.travel.is_backward() && self.steps.len() > 1
    }
}

/// Place `run` around the closed `path` (a frame ring), from `start`
/// in `direction`, as `fit` says (#239 §5, §6). A ring in `Once`,
/// `Spread` or `Fit` is text, set in two lines over the top and along
/// the bottom (#291, ruling R27), and ignores `start` and `direction`;
/// a `Repeat` ring is walked whole from `start`, every stretch read
/// forward.
///
/// With `outward`, each cluster is pushed off the outline by half its
/// extent along the outward normal, so the ring sits outside the
/// outline; without, cluster centers sit on it. A ring placed around
/// `offset(outline, ring_offset(...))` clears the rings inside it.
/// "Fits" means the last cluster's step ends at or before the path's
/// length: the start point is not reused.
///
/// `flow` is the flow's own resolved style (sizes in points), read only
/// to shape a `Once` filler (ruling R6). `fs` is the guard the caller
/// holds; only `Fit` (reshaping at the fitted size) and `Once` with a
/// filler shape anything, and nothing here acquires the font lock.
///
/// Returns the run the placement refers to when it is not `run` — the
/// rescaled run for `Fit`, or `run` plus one shaped copy of the filler
/// for `Once` — and the placement.
///
/// Cost: one arc-length path (O(1) for a rounded rectangle or a circle,
/// a table for an ellipse); O(clusters placed) steps, each a handful of
/// path lookups; `Spread` and `Fit` measure the run a bounded number of
/// times more to solve their end condition, and a cut a logarithmic
/// number; `Fit` and a filler shape once.
#[allow(clippy::too_many_arguments)] // #239's signature, as amended by ruling R6.
pub fn place_on_closed(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_closed_traced(run, path, start, direction, fit, outward, flow, fs, None)
}

/// [`place_on_closed`], filling `trace` with what the walk did: the
/// test seam for the readable rule (#291).
#[allow(clippy::too_many_arguments)] // `place_on_closed`'s, and the trace.
pub(crate) fn place_on_closed_traced(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    if !matches!(fit, FitMode::Repeat) {
        return lines::place_lines(run, path, fit, outward, flow, fs, trace);
    }
    let spacing = table_spacing(run, fit, flow);
    let arc = ArcPath::closed(path, start, direction, spacing);
    place(run, &arc, true, fit, 0.0, outward, flow, fs, trace)
}

/// Place `run` along the open `curve` (a connector or a line link),
/// from its source end, as `fit` says, with `spacing` between clusters
/// (#239 §5, §6).
///
/// Cluster centers sit on the curve. `spacing` is the gap between steps
/// for `Once` and `Repeat` (and between filler copies); `Spread` and
/// `Fit` solve the gap that fills the curve, which replaces it (ruling
/// R6, reading 7).
///
/// `flow`, `fs` and the return value are as for [`place_on_closed`].
///
/// Cost: as [`place_on_closed`]; the path is O(1) for a line and a table
/// for a Bézier.
pub fn place_on_open(
    run: &ShapedRun,
    curve: &Curve,
    fit: &FitMode,
    spacing: f32,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_open_traced(run, curve, fit, spacing, flow, fs, None)
}

/// [`place_on_open`], filling `trace` with what the walk did: the test
/// seam for the readable rule (#291).
pub(crate) fn place_on_open_traced(
    run: &ShapedRun,
    curve: &Curve,
    fit: &FitMode,
    spacing: f32,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let table = table_spacing(run, fit, flow);
    let arc = ArcPath::open(curve, table);
    let spacing = if spacing.is_finite() {
        spacing.max(0.0)
    } else {
        0.0
    };
    place(run, &arc, false, fit, spacing, false, flow, fs, trace)
}

/// Place `run` as one line centered on `center`: a portal marker.
/// Glyphs sit at their pen positions as shaped, the line as wide as its
/// advances, all on one baseline; nothing is cut. A `center` that is
/// not finite is read as the origin, so no position is `NaN`.
///
/// Cost: O(glyphs); one allocation for the placement.
pub fn place_at_point(run: &ShapedRun, center: Vec2) -> PlacedRun {
    let center = if center.is_finite() { center } else { Vec2::ZERO };
    let width: f32 = run.glyphs.iter().map(|g| g.advance).sum();
    let left = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    let corner = center - Vec2::new(width, run.line_height) * 0.5;
    let glyphs = run
        .glyphs
        .iter()
        .enumerate()
        .take(MAX_PLACED_GLYPHS)
        .map(|(index, glyph)| PlacedGlyph {
            glyph: index as u32,
            pos: corner + Vec2::new(glyph.x - left, 0.0),
        })
        .collect();
    PlacedRun {
        glyphs,
        truncated: run.glyphs.len() > MAX_PLACED_GLYPHS,
    }
}

/// The finest spacing an arc-length table is ever sampled at: a quarter
/// of the smallest line height any glyph can be placed with, since the
/// shaper floors every size at [`MIN_FONT_SIZE_PT`]. Finer samples are
/// samples no placement can use.
pub(crate) const MIN_TABLE_SPACING: f32 = 0.25 * MIN_FONT_SIZE_PT * LINE_HEIGHT_FACTOR;

/// The sample spacing for a path's arc-length table: a quarter of the
/// smallest line height placement may walk with (#239 §5 step 1) — the
/// run's, shrunk by `Fit`'s smallest scale, or a `Once` filler's — but
/// never finer than [`MIN_TABLE_SPACING`], however small `Fit`'s `min`
/// (and that, too, when the line height is not a number).
pub(crate) fn table_spacing(run: &ShapedRun, fit: &FitMode, flow: &ResolvedText) -> f32 {
    let mut line_height = run.line_height;
    match fit {
        FitMode::Fit { min, .. } if *min > 0.0 && *min < 1.0 => line_height *= min,
        FitMode::Once { filler: Some(_) } => {
            let filler = point_size(flow) * LINE_HEIGHT_FACTOR;
            if line_height <= 0.0 || filler < line_height {
                line_height = filler;
            }
        }
        _ => {}
    }
    let spacing = 0.25 * line_height;
    if spacing >= MIN_TABLE_SPACING {
        spacing
    } else {
        MIN_TABLE_SPACING
    }
}

/// Where the readable walk (ruling R25) must end.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Ends {
    /// Where step 1 ended: `Spread` and `Fit`, whose spacing was solved
    /// to end at the path's end.
    Exactly,
    /// Anywhere within the path, at step 1's spacing if it can: `Once`
    /// and `Repeat`.
    Within,
}

/// How phase 2 of the readable rule spaces a stretch's clusters
/// ([`PathWalk::lay_reading`]). Whatever it asks, a gap is never below
/// zero: a stretch that does not fit its part of the path even with no
/// gap takes no gap and runs on into the next one's.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Spacing {
    /// No gap anywhere: the shortest the placement can be laid.
    Zero,
    /// This gap (step 1's), less where the stretch would not fit its part
    /// of the path with it: `Once` and `Repeat`.
    Keep(f32),
    /// The gap that ends each stretch exactly where its part of the path
    /// ends: `Spread` and `Fit`, whose step 1 was solved to fill the path.
    Fill,
    /// This gap everywhere, every stretch laid on from the one before
    /// rather than from where it began: the pooled lay (ruling R25), for
    /// a placement whose stretches cannot each keep their own part of
    /// the path.
    Uniform(f32),
}

/// Phase 2's lay of one placement, as [`PathWalk::lay_reading`] takes it.
struct Lay<'a> {
    stretches: &'a [PathStretch],
    kept: &'a [bool],
    spacing: Spacing,
    walk_gap: f32,
    end: f32,
}

/// Step 1 of the readable rule, a fit mode's walk: what phase 2
/// ([`PathWalk::readable`]) lays again in reading order.
struct Walked {
    /// The run the placement refers to, when it is not the run given.
    run: Option<ShapedRun>,
    laying: Laying,
    /// Whether the text was cut (or placement capped).
    truncated: bool,
    /// The gap the walk left between steps.
    gap: f32,
    ends: Ends,
    /// How many of the laid clusters are the text; the rest are copies
    /// a cut drops first.
    text: usize,
    /// How many clusters one droppable copy is: a filler copy is dropped
    /// whole, a repeat cluster by itself.
    copy: usize,
    /// How far past the path's end the walk may go: `Fit`'s allowance
    /// for the float noise of its reshape (ruling R9.3), else 0.
    slack: f32,
    /// Whether every word a stretch boundary tears is moved whole, room
    /// or not ([`PathWalk::snap_to_words`]): `Fit` above `min`, whose
    /// scale was chosen so they all have room. Otherwise a word moves
    /// only where its taker finds room.
    whole_words: bool,
    /// Whether the placement is a closed ring of whole copies, its
    /// spacing to be even all the way round, the seam's included (ruling
    /// R29): phase 2 solves one gap for the reading-order lay itself.
    even: bool,
}

/// The body both placements share, over a walked path: step 1 of the
/// readable rule (#291) is the fit mode's walk, which decides which
/// clusters are placed and where; steps 2 to 4
/// ([`PathWalk::readable`]) then reverse every leftward and upward
/// stretch.
#[allow(clippy::too_many_arguments)] // The placements' inputs, and the trace.
fn place(
    run: &ShapedRun,
    path: &ArcPath,
    closed: bool,
    fit: &FitMode,
    spacing: f32,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystem,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let walk = |run: &ShapedRun| PathWalk {
        path,
        line_height: run.line_height,
        outward,
        limit: path.length() + end_slack(path.length()),
        level: false,
        ornament: !run.glyphs.iter().any(|glyph| glyph.letter),
    };
    let walked = match fit {
        FitMode::Once { filler } => once(run, spacing, filler.as_deref(), flow, fs, &walk),
        FitMode::Repeat => repeat(run, &walk(run), spacing, closed),
        FitMode::Spread => spread(run, &walk(run), 0.0),
        FitMode::Fit { min, max } => fit_to(run, path, *min, *max, fs, &walk),
    };
    walk(walked.run.as_ref().unwrap_or(run)).readable(walked, trace)
}

/// Every cluster once from the start, `gap` apart, cut to the longest
/// reading-order prefix that fits. Also returns where the next cluster
/// would start, when the text was placed whole.
fn once_text(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> (Walked, Option<f32>) {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let mut laying = Laying::with_capacity(run.glyphs.len());
    let (laid, cut) = walk.lay_or_cut(&run.glyphs, &clusters, 0.0, gap, &mut laying);
    let next = match laid {
        Laid::Fits(next) if !cut => Some(next),
        _ => None,
    };
    let truncated = cut || laid == Laid::Capped;
    let text = laying.steps.len();
    let walked = Walked {
        run: None,
        laying,
        truncated,
        gap,
        ends: Ends::Within,
        text,
        copy: 1,
        slack: 0.0,
        whole_words: false,
        even: false,
    };
    (walked, next)
}

/// `Once`: the text once from the start; then, with a filler and room
/// left, whole copies of the filler until the next would pass the end.
fn once<'p>(
    run: &ShapedRun,
    gap: f32,
    filler: Option<&str>,
    flow: &ResolvedText,
    fs: &mut FontSystem,
    walk_for: &dyn Fn(&ShapedRun) -> PathWalk<'p>,
) -> Walked {
    let (placed, next) = once_text(run, &walk_for(run), gap);
    let (Some(filler), Some(_)) = (filler, next) else {
        return placed;
    };
    let filled = reshape(run, 1.0, Some((filler, flow)), fs);
    let text_glyphs = filled.glyphs.iter().take_while(|g| g.span != FILLER_SPAN).count();
    if text_glyphs == filled.glyphs.len() {
        return placed;
    }
    // The filled run may be taller (a larger filler), which moves the
    // text; place everything again with its line height.
    let walk = walk_for(&filled);
    let text = clusters_of(&filled.glyphs, 0..text_glyphs);
    let copy = clusters_of(&filled.glyphs, text_glyphs..filled.glyphs.len());
    let mut glyphs = Laying::with_capacity(filled.glyphs.len());
    let Laid::Fits(mut s) = walk.lay(&filled.glyphs, &text, 0.0, gap, &mut glyphs) else {
        return placed;
    };
    let mut copies = 0usize;
    let mut capped = false;
    loop {
        let before = glyphs.mark();
        match walk.lay(&filled.glyphs, &copy, s, gap, &mut glyphs) {
            Laid::Fits(next) => {
                copies += 1;
                if next <= s {
                    // A copy that does not advance would repeat forever.
                    break;
                }
                s = next;
            }
            Laid::Overran => {
                glyphs.truncate(before);
                break;
            }
            Laid::Capped => {
                glyphs.truncate(before);
                capped = true;
                break;
            }
        }
    }
    if copies == 0 {
        return placed;
    }
    Walked {
        run: Some(filled),
        laying: glyphs,
        truncated: capped,
        gap,
        ends: Ends::Within,
        text: text.len(),
        copy: copy.len(),
        slack: 0.0,
        whole_words: false,
        even: false,
    }
}

/// `Repeat`: the text again and again until the path is full. Every copy
/// reuses the run's glyph indices.
///
/// Along an open path the last copy is cut to the longest reading-order
/// prefix that fits. Around a `closed` one (ruling R29) the ring holds a
/// whole number of copies — as many as fit with `gap` — and the slack is
/// spread evenly as extra spacing between every two clusters, the seam
/// from the last cluster back round to the first included, so the ring
/// closes without a stray part-copy or a gap where it began. A ring too
/// short for even one copy is cut as an open path's is.
///
/// Cost: one lay per copy; around a closed path, a solve of the spacing
/// of at most `2 · SOLVE_ROUNDS` measures of the copies and one lay more.
fn repeat(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32, closed: bool) -> Walked {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let mut glyphs = Laying::default();
    let mut s = 0.0_f32;
    let mut copies = 0usize;
    let truncated = loop {
        let before = glyphs.mark();
        match walk.lay(&run.glyphs, &clusters, s, gap, &mut glyphs) {
            Laid::Fits(next) => {
                copies += 1;
                if next <= s {
                    // A copy that does not advance would repeat forever.
                    break false;
                }
                s = next;
            }
            Laid::Overran => {
                glyphs.truncate(before);
                let (laid, _) = walk.lay_or_cut(&run.glyphs, &clusters, s, gap, &mut glyphs);
                break copies == 0 || laid == Laid::Capped;
            }
            Laid::Capped => {
                glyphs.truncate(before);
                break true;
            }
        }
    };
    let closing = if closed && copies > 0 && !truncated {
        close_ring(run, &clusters, walk, copies, gap)
    } else {
        None
    };
    // A closed ring's walk ends where its spacing was solved to; phase 2
    // solves its spacing again as the ring is read (`Walked::even`).
    let even = closing.is_some();
    let (glyphs, truncated, gap, ends) = match closing {
        Some((glyphs, gap)) => (glyphs, false, gap, Ends::Exactly),
        None => (glyphs, truncated, gap, Ends::Within),
    };
    let text = clusters.len().min(glyphs.steps.len());
    Walked {
        run: None,
        laying: glyphs,
        truncated,
        gap,
        ends,
        text,
        copy: 1,
        slack: 0.0,
        whole_words: false,
        even,
    }
}

/// A closed ring of `copies` whole copies of `clusters` (ruling R29):
/// the gap, at least `gap`, that ends the last copy's last step that same
/// gap short of the path's end, where the first copy began — so every
/// two consecutive clusters, the seam's included, are one gap apart — and
/// the copies laid with it. `None` when the copies would pass
/// [`MAX_PLACED_GLYPHS`] or do not lay within the path.
///
/// Cost: a solve of at most `2 · SOLVE_ROUNDS` measures of the copies,
/// and one lay of them.
fn close_ring(
    run: &ShapedRun,
    clusters: &[Cluster],
    walk: &PathWalk<'_>,
    copies: usize,
    gap: f32,
) -> Option<(Laying, f32)> {
    let length = walk.path.length();
    let copied = || (0..copies).flat_map(|_| clusters.iter());
    // How far the copies, `extra` past `gap` apart, leave the seam short
    // of `gap + extra`: zero when the seam is one gap like the others.
    let miss = |extra: f32| {
        let g = gap + extra;
        walk.measure_clusters(copied(), 1.0, g) + g - length
    };
    let extra = if miss(0.0) >= 0.0 {
        0.0
    } else {
        let guess = -miss(0.0) / (copies * clusters.len()) as f32;
        solve_gap(miss, length, guess)
    };
    let g = gap + extra;
    let mut laying = Laying::default();
    let mut s = 0.0_f32;
    for _ in 0..copies {
        match walk.lay(&run.glyphs, clusters, s, g, &mut laying) {
            Laid::Fits(next) => s = next,
            _ => return None,
        }
    }
    Some((laying, g))
}

/// `Spread`: the text once, with the equal extra spacing between
/// clusters that ends its last step at the path's end (ruling R6). The
/// spacing starts from `(P − L)/(n − 1)` — exact on a path of constant
/// direction — and is solved numerically from there, because spacing
/// moves clusters onto stretches where their steps differ. Too long for
/// the path even with no spacing, the text is cut to the longest
/// reading-order prefix that fits — unless it overruns by no more than
/// `overrun_slack`, which `Fit` allows for the float noise of a reshape
/// (ruling R9.3). One cluster takes no spacing.
fn spread(run: &ShapedRun, walk: &PathWalk<'_>, overrun_slack: f32) -> Walked {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let length = walk.path.length();
    if clusters.len() <= 1 {
        return once_text(run, walk, 0.0).0;
    }
    let natural = walk.measure(&clusters, 1.0, 0.0);
    if natural.is_nan() || natural >= length {
        let walk = PathWalk {
            limit: walk.limit + overrun_slack,
            ..*walk
        };
        // Placed whole, the text fills the path with no spacing: the
        // readable lay must end where it ends, too.
        let mut walked = once_text(run, &walk, 0.0).0;
        if !walked.truncated {
            walked.ends = Ends::Exactly;
        }
        return walked;
    }
    let guess = (length - natural) / (clusters.len() - 1) as f32;
    let gap = solve_gap(|gap| walk.measure(&clusters, 1.0, gap) - length, length, guess);
    let mut walked = once_text(run, walk, gap).0;
    walked.ends = Ends::Exactly;
    walked
}

/// The gap `e ≥ 0` at which a walk `e` apart ends within
/// [`SOLVE_TOLERANCE`] of `length`, never past it, starting from
/// `guess`; `miss(e)` is where that walk ends less `length`, and grows
/// with `e`. Regula falsi (Illinois) on a bracket whose low side always
/// fits; bounded by [`SOLVE_ROUNDS`]. The end moves with `e` without the
/// jumps a tangent-taken step has at a sharp corner ([`PathWalk::step`]),
/// so the bracket closes on the end condition rather than on a jump.
fn solve_gap(miss: impl Fn(f32) -> f32, length: f32, guess: f32) -> f32 {
    let tolerance = SOLVE_TOLERANCE.max(length * 1e-6);
    let (mut low, mut low_miss) = (0.0_f32, miss(0.0));
    // Find a gap that overshoots, doubling the guess.
    let mut high = None;
    let mut probe = guess.max(f32::EPSILON);
    for _ in 0..SOLVE_ROUNDS {
        let probe_miss = miss(probe);
        if probe_miss >= 0.0 {
            high = Some((probe, probe_miss));
            break;
        }
        (low, low_miss) = (probe, probe_miss);
        probe *= 2.0;
    }
    let Some((mut high, mut high_miss)) = high else {
        return low;
    };
    let mut side = 0i8;
    for _ in 0..SOLVE_ROUNDS {
        if low_miss >= -tolerance || high - low <= f32::EPSILON * high {
            break;
        }
        let mut next = low - low_miss * (high - low) / (high_miss - low_miss);
        if next.is_nan() || next <= low || next >= high {
            next = 0.5 * (low + high);
        }
        let next_miss = miss(next);
        if next_miss <= 0.0 {
            (low, low_miss) = (next, next_miss);
            if side == -1 {
                high_miss *= 0.5;
            }
            side = -1;
        } else {
            (high, high_miss) = (next, next_miss);
            if side == 1 {
                low_miss *= 0.5;
            }
            side = 1;
        }
    }
    low
}

/// `Fit { min, max }`: the run rescaled so it fills the path exactly once
/// (rulings R6, R9.3). The scale starts at `clamp(P / L, min, max)`.
/// Whenever the text at that scale is longer than the path — rescaling
/// moves clusters onto stretches where their steps differ, and a scale
/// held at `max` can overrun too — it shrinks by `P / L(s)`, measured
/// arithmetically, until the text fits or the scale reaches `min`. Where
/// that shrink creeps — on an outline whose sides are short beside the
/// glyphs, `L(s)` can fall much more slowly than `s` — it is finished by
/// bisection between `min` and the last scale that overran. `L(s)` is
/// measured as the text will be read (#291): the longer of the walk and
/// its reading-order lay ([`PathWalk::measure_readable`]). The run is
/// reshaped at the final scale, and the rest of the path is spread as
/// extra spacing, never below zero. Only text still too long at `min` is
/// cut (design §6.4 step 4); an overrun no larger than the solver's
/// tolerance, the float noise of the reshape, is placed as is.
///
/// Spreading moves clusters along the path, and with them, now and then,
/// between stretches; read in that order the text can then overrun even
/// with no gap. Then the scale shrinks by what it overruns, and the run
/// is reshaped again, at most [`FIT_ROUNDS`] times in all (ruling
/// R25.1): `Fit` keeps the whole text rather than cut it above `min`.
fn fit_to<'p>(
    run: &ShapedRun,
    path: &'p ArcPath,
    min: f32,
    max: f32,
    fs: &mut FontSystem,
    walk_for: &dyn Fn(&ShapedRun) -> PathWalk<'p>,
) -> Walked {
    let walk = walk_for(run);
    let length = path.length();
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let natural = walk.measure(&clusters, 1.0, 0.0);
    if clusters.is_empty() || natural.is_nan() || natural <= 0.0 {
        return once_text(run, &walk, 0.0).0;
    }
    let (low, high) = scale_window(min, max);
    let ideal = length / natural;
    let start = if ideal.is_nan() {
        1.0
    } else {
        ideal.clamp(low, high)
    };
    let mut scale = fitting_scale(&walk, &run.glyphs, &clusters, length, start, low);
    let mut rounds = 0;
    let mut previous: Option<(f32, f32)> = None;
    loop {
        let rescaled = (scale != 1.0).then(|| reshape(run, scale, None, fs));
        let placed_run = rescaled.as_ref().unwrap_or(run);
        let slack = if scale > low { SOLVE_TOLERANCE } else { 0.0 };
        let placed_walk = walk_for(placed_run);
        let walked = Walked {
            slack,
            whole_words: scale > low,
            even: false,
            ..spread(placed_run, &placed_walk, slack)
        };
        let (read, limit) = placed_walk.readable_extent(&walked.laying.steps, slack);
        // A walk that had to cut the text overran, however its kept
        // clusters read.
        let read = if walked.truncated {
            read.max(limit * (1.0 + 1e-3))
        } else {
            read
        };
        rounds += 1;
        if read <= limit || scale <= low || rounds >= FIT_ROUNDS {
            return Walked {
                run: rescaled,
                ..walked
            };
        }
        // The spacing spread put in moved clusters between stretches, and
        // read in that order the text no longer fits even with no gap:
        // shrink by what it overruns, as `fitting_scale` does — and once
        // two rounds have measured how the overrun falls with the scale,
        // by the secant through them, a little past it, since the overrun
        // can fall more slowly than the scale.
        let proportional = scale * (limit / read) * (1.0 - 1e-3);
        let next = match previous {
            Some((before, was)) if was > read && before > scale => {
                let rate = (was - read) / (before - scale);
                (scale - 1.25 * (read - limit) / rate).min(proportional)
            }
            _ => proportional,
        };
        previous = Some((scale, read));
        scale = next.max(low);
    }
}

/// The largest scale, at most `start`, at which `clusters` fit `length`
/// (ruling R9.3), or `low` when even that overruns.
///
/// First the shrink `s ← s·P/L(s)` (a hair under, so it converges from
/// above), at most [`FIT_ROUNDS`] times; then, if the text still
/// overruns, bisection between `low` and the last overrunning scale, at
/// most [`SOLVE_ROUNDS`] times, keeping the side that fits. Every round
/// is one arithmetic measure ([`PathWalk::measure_at_scale`], which
/// predicts the reshape's size floor); nothing is reshaped.
fn fitting_scale(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    length: f32,
    start: f32,
    low: f32,
) -> f32 {
    let largest = glyphs.iter().map(|glyph| glyph.size).fold(0.0_f32, f32::max);
    let measure = |scale: f32| walk.measure_at_scale(glyphs, clusters, scale, largest);
    let fits = |scale: f32| measure(scale) <= length;
    let mut over = start;
    for _ in 0..FIT_ROUNDS {
        let at_scale = measure(over);
        if at_scale <= length {
            return over;
        }
        if over <= low {
            return low;
        }
        over = (over * (length / at_scale) * (1.0 - 1e-6)).max(low);
    }
    if fits(over) {
        return over;
    }
    if !fits(low) {
        return low;
    }
    let mut under = low;
    for _ in 0..SOLVE_ROUNDS {
        if over - under <= under * 1e-6 {
            break;
        }
        let middle = 0.5 * (under + over);
        if fits(middle) {
            under = middle;
        } else {
            over = middle;
        }
    }
    under
}

/// What a reshape at `scale` multiplies a glyph of `size` points by:
/// `scale`, unless that would take it out of the shaper's size domain,
/// which the reshape clamps it to (`clamp_font_metric`, as
/// `shape::scaled` does) — so below
/// [`MIN_FONT_SIZE_PT`] a glyph stops shrinking. A size that is not a
/// positive number (never shaped) scales by `scale`.
fn size_factor(size: f32, scale: f32) -> f32 {
    if size > 0.0 && size.is_finite() {
        clamp_font_metric(size * scale, MIN_FONT_SIZE_PT) / size
    } else {
        scale
    }
}

/// `Fit`'s scale window, made usable whatever it holds: a reversed pair
/// is read as the window it describes, a non-positive or non-finite
/// bound falls back to the scale it cannot be (bounds are checked at
/// load, `check_fit`; this keeps a hand-built mode from panicking
/// `f32::clamp`).
fn scale_window(min: f32, max: f32) -> (f32, f32) {
    let usable = |v: f32, fallback: f32| if v.is_finite() && v > 0.0 { v } else { fallback };
    let (min, max) = (usable(min, f32::MIN_POSITIVE), usable(max, f32::MAX));
    if min <= max {
        (min, max)
    } else {
        (max, min)
    }
}
