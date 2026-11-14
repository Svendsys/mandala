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
//! **An ornament steps by its ink** (#291, ruling R31). A flow with no
//! letter or digit — `✦ · `, `·:·`, `~` — has nothing to read in a line
//! height, and a line-high box would part its glyphs further wherever
//! the path runs steep than where it runs level. Its glyphs are boxed by
//! their ink instead ([`GlyphMetrics::ink`]): a glyph's step is its ink
//! box's extent along the chord plus its side bearings, `advance − ink
//! width`, which do not turn with the path, and its ink box is centered
//! on the path its left bearing into the step (pushed outward on a
//! closed path by the support of the ellipse its ink box bounds). A
//! space has no ink and steps its advance. Each glyph after the first is
//! then set where its ink box is as far from the last one's as a line
//! would set them, which is where its step put it where the path runs
//! straight, and not where it turns: round a corner, and across a
//! ring's seam. So two glyphs' inks are as far apart wherever the path
//! runs — as far as the font sets them on a line, plus the spacing.
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

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::ops::Range;

use glam::Vec2;
use rustc_hash::FxHashMap;

use crate::document::element::DirectionalGlyphs;
use crate::document::resolve::{ResolvedRing, ResolvedText};
use crate::document::values::{Direction, FitMode, LineSpan, Rgba, StartPoint};
use crate::flow::curve::Curve;
use crate::flow::outline::Outline;
use crate::flow::path::ArcPath;
use crate::flow::shape::{reshape, GlyphMetrics, Ink, ShapedRun, FILLER_SPAN};
use crate::flow::span::{point_size, LINE_HEIGHT_FACTOR};
use crate::font::fonts::{clamp_font_metric, FontSystemLease, MIN_FONT_SIZE_PT};
use crate::font::FontSystem;

pub use pitch::{pattern_gap, place_pitched, PathEnd};

pub(crate) mod directional;
pub(crate) mod halves;
mod lines;
pub(crate) mod pitch;
pub(crate) mod ring;
pub(crate) use lines::{frame_halves, place_lines};
use ring::{RingLoss, RingWork};

/// The most steps [`PathWalk::runs_on`] probes past a text's end to find
/// where the path stops running the last stretch's way: a path that long
/// in line heights is followed no further, and the last stretch's part
/// ends there.
pub(crate) const MAX_RUN_ON_STEPS: usize = 1 << 16;

/// The most rounds [`PathWalk::even_out`] regroups a closed ring's
/// stretches in.
const EVEN_ROUNDS: usize = 4;

/// The step the walk's box rule (#239 §5 step 2, ruling R9.2) takes for
/// a `w × h` cluster from `s` along `path`, and its direction: the test
/// seam for that rule at a sharp corner, which only a closed outline has.
pub(crate) fn box_step(path: &ArcPath, s: f32, w: f32, h: f32) -> (f32, Vec2) {
    let walk = PathWalk {
        path,
        memo: None,
        work: None,
        line_height: h,
        outward: false,
        limit: path.length(),
        level: false,
        ornament: false,
        baseline: 0.0,
    };
    walk.step(s, w, h)
}

/// The rounds of bisection [`PathWalk::ink_distance_at`] takes to set an
/// ornament's glyph its distance from the last: a bracket some points
/// long closes to a few millionths of a point.
const INK_ROUNDS: usize = 24;

/// The most strides [`PathWalk::ink_distance_at`] takes past where it
/// first tries to set a glyph, looking for where its ink is far enough
/// from the last: a path that bends back on itself further than that is
/// not followed, and the glyph is set where the last stride ended.
const INK_STRIDES: usize = 64;

/// The rounds [`PathWalk::ink_gap_at`] takes to bring the gap between two
/// inks to what is owed, each solving the distance between their centers
/// that leaves the gap the last round's left.
const INK_TURNS: usize = 12;

/// How near the gap the box rule leaves between two inks must be to the
/// gap owed for [`PathWalk::ink_gap_at`] to keep it, in points.
pub(crate) const INK_TOLERANCE: f32 = 1e-3;

/// The most times a frame's outline is halved for its length to be an
/// `f32` ([`place_on_closed`]): the largest outline's perimeter is about
/// `2 · f32::MAX`, so one halving does; more are slack.
const MAX_SHRINKS: usize = 8;

/// The rounds of Newton's method [`PathWalk::level_len`] takes.
pub(crate) const LEVEL_ROUNDS: usize = 4;

/// The most glyphs one placement — one path's flow — places (ruling R6,
/// reading 9; ruling R41: 65,536, chosen by the owner). Text, repeat
/// copies, fillers and a frame's sides all count against it. A flow on a
/// path long enough to need more stops here and reports
/// [`PlacedRun::truncated`].
pub const MAX_PLACED_GLYPHS: usize = 1 << 16;

/// The most glyphs a walk lays but a ring's, which has a cap of its own
/// ([`RingWork::cap`]): [`MAX_PLACED_GLYPHS`] — in a test, the frame's
/// cap seam's (`halves::CAP_OVERRIDE`), so that a short text reaches it.
fn placed_cap() -> usize {
    #[cfg(test)]
    if let Some(cap) = halves::CAP_OVERRIDE.with(Cell::get) {
        return cap;
    }
    MAX_PLACED_GLYPHS
}

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
    /// (An ornament's box is placed by its ink, ruling R31: the box is
    /// where that puts it, not centered on the path.)
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

/// How a frame's ring is laid, besides its fit (#291, rulings R32, R33,
/// R33.1, R31): the resolved ring's `span`, `sides` and `spacing`.
#[derive(Clone, Debug, PartialEq)]
pub struct RingLayout {
    /// How much of its half each line of a two-line ring covers: from
    /// its natural width ([`LineSpan::NATURAL`]) to its whole half
    /// ([`LineSpan::WHOLE`], the built-in). A `repeat` ring ignores it.
    pub span: LineSpan,
    /// The ornament closing what a two-line ring's lines leave uncovered
    /// round its junctions: its text and its color. `None` leaves those
    /// parts empty. A `repeat` ring ignores it.
    pub sides: Option<(String, Rgba)>,
    /// A `repeat` ring's extra space between steps, canvas units (ruling
    /// R31): `0.0` sets the ornament's copies as far apart as the font
    /// and the ring's slack do. A two-line ring ignores it.
    pub spacing: f32,
    /// What a `repeat` ring lays where its outline runs steep and where
    /// it turns (#324, `format/schema.md` "Directional ring"): neither set, it is a ring of its
    /// text alone. A two-line ring ignores it.
    pub directional: DirectionalGlyphs,
}

impl RingLayout {
    /// The built-in layout: each line over its whole half, no sides, no
    /// extra spacing.
    pub fn whole() -> Self {
        RingLayout {
            span: LineSpan::WHOLE,
            sides: None,
            spacing: 0.0,
            directional: DirectionalGlyphs::default(),
        }
    }

    /// Each line at its natural width — its font's own gap, centered on
    /// its half's apex — no sides, no extra spacing.
    pub fn natural() -> Self {
        RingLayout {
            span: LineSpan::NATURAL,
            ..RingLayout::whole()
        }
    }

    /// The layout a resolved ring says: its span, its sides' text and
    /// color, its spacing, its glyphs by direction. O(1), and a clone of
    /// their texts.
    pub fn of(ring: &ResolvedRing) -> Self {
        RingLayout {
            span: ring.span,
            sides: ring.sides.as_ref().map(|s| (s.content.clone(), s.color)),
            spacing: ring.spacing,
            directional: ring.directional.clone(),
        }
    }
}

impl Default for RingLayout {
    fn default() -> Self {
        RingLayout::whole()
    }
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
    /// Its glyphs' ink as one box, in its box's coordinates: `x` from
    /// its left edge, `y` from the baseline ([`cluster_ink`]). `None`
    /// where no glyph has any measured (text; a space).
    ink: Option<Ink>,
}

/// The ink of `cluster`'s glyphs as one box, in the cluster's box
/// coordinates — `x` from its left edge, `y` from the baseline — with
/// each glyph `factor(glyph)` times the size it was shaped at: its ink
/// scaled by that about its pen, and its pen's distance into the box
/// too, as a reshape would set it. `None` where no glyph has ink
/// ([`GlyphMetrics::ink`]).
///
/// Cost: O(the cluster's glyphs).
fn cluster_ink(
    glyphs: &[GlyphMetrics],
    cluster: &Cluster,
    factor: impl Fn(&GlyphMetrics) -> f32,
) -> Option<Ink> {
    glyphs[cluster.glyphs.clone()]
        .iter()
        .filter_map(|glyph| {
            let f = factor(glyph);
            let pen = Vec2::new((glyph.x - cluster.left) * f, 0.0);
            glyph.ink.map(|ink| Ink {
                min: pen + ink.min * f,
                max: pen + ink.max * f,
            })
        })
        .reduce(|a, b| Ink {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        })
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
                ink: None,
            }),
        }
    }
    for cluster in &mut clusters {
        cluster.ink = cluster_ink(glyphs, cluster, |_| 1.0);
    }
    clusters
}

/// How far, and which way, the center of an ellipse of semi-axes `half`
/// lies from where it touches, from outside, a line whose outward normal
/// is `normal` (a unit vector): the ellipse's support point along
/// `normal`. On an axis it is the semi-axis, as a box's half-extent would
/// be; between, it turns with the normal, never further out along an
/// axis than the box — so a center pushed by it off a convex outline
/// traces a convex path round a sharp corner, where a box's own reach
/// would swing it out past its side and back.
fn support(normal: Vec2, half: Vec2) -> Vec2 {
    let stretched = normal * half * half;
    let reach = (normal * half).length();
    if reach > 0.0 {
        stretched / reach
    } else {
        Vec2::ZERO
    }
}

/// What a cluster takes up of the path: what its step, and where it is
/// set, are measured from.
#[derive(Copy, Clone, Debug)]
enum Body {
    /// Its `w × h` box: its advance by the line height (#239 §5, ruling
    /// R9.2).
    Boxed { w: f32, h: f32 },
    /// An ornament's glyph, by its ink (ruling R31): its advance, and its
    /// ink box in its box's coordinates. A space's ink is a point at its
    /// middle on the baseline.
    Inked { advance: f32, ink: Ink },
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

/// A placement being built: the glyphs, the step each cluster took, and
/// what an ornament's walk remembers of the ink it set last.
#[derive(Default)]
struct Laying {
    glyphs: Vec<PlacedGlyph>,
    steps: Vec<Stepped>,
    trail: Trail,
}

/// How much of a [`Laying`] has been laid, to
/// [`truncate`](Laying::truncate) back to.
#[derive(Copy, Clone)]
struct Mark {
    glyphs: usize,
    steps: usize,
    trail: Trail,
}

impl Laying {
    fn with_capacity(glyphs: usize) -> Self {
        Laying {
            glyphs: Vec::with_capacity(glyphs),
            steps: Vec::new(),
            trail: Trail::default(),
        }
    }

    /// How much has been laid, to [`truncate`](Self::truncate) back to.
    fn mark(&self) -> Mark {
        Mark {
            glyphs: self.glyphs.len(),
            steps: self.steps.len(),
            trail: self.trail,
        }
    }

    fn truncate(&mut self, mark: Mark) {
        self.glyphs.truncate(mark.glyphs);
        self.steps.truncate(mark.steps);
        self.trail = mark.trail;
    }
}

/// What an ornament's walk remembers of the ink it set last (ruling R31):
/// where the next glyph's ink must keep from.
#[derive(Copy, Clone, Debug, Default)]
struct Trail {
    /// The last glyph's ink on the canvas — its center, and half its
    /// size — and where along the path its center was set. `None` before
    /// the first.
    last: Option<(Vec2, Vec2, f32)>,
    /// How far the next glyph's ink is owed from it, but for the next
    /// glyph's own left bearing: as far as a line would set them — its
    /// right bearing, every space's advance and every gap between steps
    /// since.
    owed: f32,
}

/// Where a cluster was walked to ([`PathWalk::advance`]).
#[derive(Copy, Clone, Debug)]
struct Taken {
    /// Where its step starts along the path, and the step's length.
    s: f32,
    step: f32,
    /// The direction the step was taken in: its chord.
    dir: Vec2,
    /// For an ornament's cluster, where along the path its ink's center
    /// is set.
    center: Option<f32>,
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
    /// For a text frame, what its last resorts left unheld (ruling
    /// R35.2): empty when its halves hold it.
    pub(crate) unheld: Unheld,
    /// For a text frame, the halves whose lines were cut, or left out,
    /// because its outline is too thin to hold its two lines apart
    /// (ruling R38.1): never overlap wins over never cut.
    pub(crate) cut_apart: Vec<usize>,
    /// For a text frame, the clearance each half's line keeps at each
    /// junction, `[half][junction]`, as its last clearance search left
    /// it: never `NaN`.
    pub(crate) clear: [[f32; 2]; 2],
    /// For a text frame, the glyph steps its tracking took in all, what
    /// its budget bounds (`halves::frame_budget`).
    pub(crate) spent: usize,
    /// For a text frame or a `Repeat` ring, the samples its paths' tables
    /// were built with, counted in `spent` (ruling B16.3a): what
    /// [`setup_table_samples`] says they are.
    pub(crate) tables: usize,
    /// For a text frame, its budget, and the glyphs of its text it was
    /// scaled by (ruling R44.2).
    pub(crate) budget: usize,
    pub(crate) glyphs: usize,
    /// For a text frame, the share of its budget held back for its last
    /// resort's cut to the inks that touch nothing (`halves::untouched_steps`,
    /// ruling R46.1).
    pub(crate) reserve: usize,
    /// For a text frame, the most pairs its touch checks looked at a box
    /// (rulings R48.2, R51.2).
    pub(crate) looks: halves::Looks,
    /// For a text frame laid on its halves, the offset off the outline,
    /// along its outward normal, of its one band curve (#308, ruling
    /// B15.1): every glyph's anchor — its ink's center along the line,
    /// its span's band center across it, or a side ornament's own ink
    /// center — lies on the outline's offset curve there. `None` for a
    /// frame R27 lays on its level spans.
    pub(crate) offset: Option<f32>,
    /// For a `Repeat` ring, how its budget left it, when it stopped any of
    /// its work (#310): `None` when its work fit. A ring's `spent`,
    /// `budget` and `glyphs` are its own, as a frame's are.
    pub(crate) ring: Option<RingLoss>,
}

/// What a text frame's last resorts — a `Fit` frame's shrink, then the
/// cut — left unheld (ruling R35.2), reported rather than left silent.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Unheld {
    /// The halves whose lines their half less the clearance still
    /// cannot hold at their font's gap.
    pub(crate) pressed: Vec<usize>,
    /// The junctions, 0 the top-left and 1 the bottom-right, still short
    /// of a break.
    pub(crate) junctions: Vec<usize>,
    /// Whether two of the lines' inks still touch.
    pub(crate) overlap: bool,
    /// The junctions whose side ornament was not laid as owed, and how
    /// (rulings R44.2, R47.2, R48.2, R50.5): [`SideLoss::Empty`], the
    /// break left without its ornament — its copies would have touched
    /// the text, or the frame could not afford them and their touch check
    /// and the copies it had did not hold the equal-gap rule; or
    /// [`SideLoss::Short`], kept with the copies a search the frame could
    /// not afford to finish had laid, which hold the equal-gap rule but
    /// need not be the count whose gap is nearest the target.
    pub(crate) sides: Vec<(usize, SideLoss)>,
}

/// How a junction's side ornament was left ([`Unheld::sides`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SideLoss {
    /// Left empty: no copy laid.
    Empty,
    /// Kept short: the copies laid when the frame's budget stopped the
    /// search for their count, every gap and both joins equal.
    Short,
}

impl Unheld {
    /// Whether nothing is left unheld.
    pub(crate) fn is_empty(&self) -> bool {
        self.pressed.is_empty() && self.junctions.is_empty() && !self.overlap && self.sides.is_empty()
    }
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
    /// A ring's budget did not admit the next cluster (#310).
    Spent,
}

/// [`PathWalk::chord`] from `at`, the point and direction of travel at
/// `s` on `path`, already looked up: one lookup, of `p(s + len)`.
fn chord_from(path: &ArcPath, (from, tangent): (Vec2, Vec2), s: f32, len: f32) -> Vec2 {
    let (to, _) = path.at(s + len);
    (to - from).try_normalize().unwrap_or(tangent)
}

thread_local! {
    /// Test counter (#294): the steps [`PathWalk::step_afresh`] has taken
    /// on this thread — the walks' work, a step looked up in a
    /// [`StepMemo`] not counted.
    #[cfg(test)]
    pub(crate) static STEPS_TAKEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test switch (#309, review 5810838863): R27's fit walks every cluster
    /// of a line, past its span's end too — what a test lays a frame with
    /// to hold the stop there to the walk it cuts short.
    #[cfg(test)]
    pub(crate) static NO_FIT_STOP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test counter (#294): the glyphs [`place`] has placed on this
    /// thread, what the work above is weighed against.
    #[cfg(test)]
    pub(crate) static GLYPHS_PLACED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#310): the last `Repeat` ring laid round a closed
    /// path on this thread ([`LastRing`]).
    #[cfg(test)]
    pub(crate) static LAST_RING: std::cell::Cell<Option<LastRing>> = const { std::cell::Cell::new(None) };
    /// Test counter (ruling B17.2c): what the last `Repeat` ring laid round
    /// a closed path on this thread estimated its glyphs from — its glyphs
    /// a copy, a copy's advance and its path's length
    /// ([`ring::ring_glyphs`]) — so that its budget at another cap is
    /// arithmetic, not a lay.
    #[cfg(test)]
    pub(crate) static LAST_RING_ESTIMATE: std::cell::Cell<Option<(usize, f64, f32)>> = const { std::cell::Cell::new(None) };
    /// Test counter (ruling B17.1d): the probes of phase 2's spacing
    /// solves, and the rounds of its regrouping, the last ring laid on
    /// this thread began with its budget spent (`RingWork::late`).
    #[cfg(test)]
    pub(crate) static RING_LATE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#310): how far past what its budget left after its
    /// hold the last ring laid on this thread was when a walk of it first
    /// found it spent (`RingWork::crossed`).
    #[cfg(test)]
    pub(crate) static RING_CROSSED: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test counter (#310): the rings their budgets have stopped on this
    /// thread.
    #[cfg(test)]
    pub(crate) static RINGS_STOPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#313): the last placement along an open path laid on
    /// this thread ([`LastRing`]), how far past what its budget left after
    /// its hold it was when a walk of it first found it spent
    /// (`RingWork::crossed`), and the probes of phase 2's spacing solves it
    /// began with its budget spent (`RingWork::late`).
    #[cfg(test)]
    pub(crate) static LAST_OPEN: std::cell::Cell<Option<(LastRing, Option<usize>, usize)>> = const { std::cell::Cell::new(None) };
    /// Test counter (#313): the placements along open paths their budgets
    /// have stopped on this thread.
    #[cfg(test)]
    pub(crate) static OPENS_STOPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A `Repeat` ring as laid (#310), what [`LAST_RING`] keeps: its glyphs a
/// copy, the glyphs its budget was scaled by and its cap, the glyphs it
/// placed, what it spent of its budget, and how its budget left it.
#[cfg(test)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct LastRing {
    pub(crate) copy: usize,
    pub(crate) glyphs: usize,
    pub(crate) cap: usize,
    pub(crate) placed: usize,
    pub(crate) spent: usize,
    pub(crate) budget: usize,
    /// The samples its path's table was built with (ruling B16.3a).
    pub(crate) tables: usize,
    pub(crate) loss: Option<RingLoss>,
    /// What was held back for phase 2 when it began, and whether the ring
    /// had closed then (`RingWork::held_again`).
    pub(crate) held: (usize, bool),
}

/// The extent a `w × h` box spans along unit direction `dir`.
fn extent(dir: Vec2, w: f32, h: f32) -> f32 {
    dir.x.abs() * w + dir.y.abs() * h
}

/// The steps one placement's walks have taken (#294): each [`PathWalk::step`]
/// answer, by where it was taken, the cluster's width and the line
/// height, all bit for bit. The walk lays a placement, and its readable
/// pass lays the same clusters again from the same places — measuring a
/// stretch before laying it, and the reading order before laying that —
/// so a step it has taken is looked up, not taken again. A step is a
/// function of those three numbers and the path alone, so the answer is
/// the one it would compute, to the bit.
///
/// Holds the steps of one line height: a walk at another, as `Fit` lays
/// at each scale it tries, starts it afresh, so it never holds more than
/// the walks at one scale took. Only the walks of one placement
/// ([`place`]) share one, all with the same path and none level, the
/// two things besides those three numbers a step depends on.
#[derive(Default)]
struct StepMemo {
    /// The line height's bits, and each step by its start's and its
    /// width's.
    taken: RefCell<(u32, Steps)>,
}

/// [`StepMemo`]'s steps: `(s, w)`, as bits, to the step and its
/// direction.
type Steps = FxHashMap<(u32, u32), (f32, Vec2)>;

impl StepMemo {
    /// The step taken from `s` by a cluster `w` wide at line height `h`,
    /// if one has been.
    fn get(&self, s: f32, w: f32, h: f32) -> Option<(f32, Vec2)> {
        let taken = self.taken.borrow();
        if taken.0 != h.to_bits() {
            return None;
        }
        taken.1.get(&(s.to_bits(), w.to_bits())).copied()
    }

    /// Remember the step `step` taken from `s` by a cluster `w` wide at
    /// line height `h`.
    fn put(&self, s: f32, w: f32, h: f32, step: (f32, Vec2)) {
        let mut taken = self.taken.borrow_mut();
        if taken.0 != h.to_bits() {
            taken.0 = h.to_bits();
            taken.1.clear();
        }
        taken.1.insert((s.to_bits(), w.to_bits()), step);
    }
}

/// The walk: a path, the run's line height and whether glyphs are
/// pushed outside the outline.
#[derive(Copy, Clone)]
struct PathWalk<'p> {
    path: &'p ArcPath,
    /// The steps this placement's walks have taken ([`StepMemo`]), when
    /// it keeps them.
    memo: Option<&'p StepMemo>,
    /// A `Repeat` ring's work and budget (#310), read before each cluster
    /// its walks lay or measure ([`lay`](Self::lay),
    /// [`walk_bodies`](Self::walk_bodies)) and before each probe of
    /// phase 2's spacing solves ([`spent`](Self::spent)); `None` for every
    /// other placement, which these walks do not budget.
    work: Option<&'p RingWork>,
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
    /// ([`GlyphMetrics::letter`]): its glyphs then step by their ink, and
    /// its spaces by their advance ([`body`](Self::body), ruling R31), so
    /// a ring's rhythm holds where the outline turns steep. Not on a
    /// level walk, whose steps are horizontal advances already.
    ornament: bool,
    /// Where the run's baseline sits below a box's top
    /// ([`ShapedRun::baseline`]): where an ornament's ink is in its box.
    baseline: f32,
}

impl PathWalk<'_> {
    /// Whether this walk's ring has spent its budget, less what it holds
    /// back for phase 2 (#310, [`RingWork::affords`]): what stops a
    /// spacing solve. Never, for a walk no budget bounds.
    fn spent(&self) -> bool {
        self.work.is_some_and(|work| !work.affords(self.path))
    }

    /// Note that a probe of phase 2's spacing solves, or a round of its
    /// regrouping, begins ([`RingWork::begin`], ruling B17.1d): nothing,
    /// for a walk no budget bounds.
    fn begin(&self) {
        if let Some(work) = self.work {
            work.begin(self.path);
        }
    }

    /// Note that this walk's ring's budget left it so (#310): nothing,
    /// for a walk no budget bounds.
    fn lose(&self, loss: RingLoss) {
        if let Some(work) = self.work {
            work.lose(loss);
        }
    }

    /// A phase 2 spacing solve ([`solve_gap`]), read against this walk's
    /// ring's budget ([`spent`](Self::spent), #310): stopped by it, the
    /// solve keeps the fitting side of its bracket, and the ring is uneven
    /// ([`RingLoss::Uneven`]).
    fn solve(&self, miss: impl Fn(f32) -> f32, length: f32, guess: f32) -> f32 {
        let stopped = Cell::new(false);
        let probe = |gap: f32| {
            self.begin();
            miss(gap)
        };
        let gap = solve_gap_until(probe, length, guess, &|| {
            let spent = self.spent();
            stopped.set(stopped.get() || spent);
            spent
        });
        if stopped.get() {
            self.lose(RingLoss::Uneven);
        }
        gap
    }

    /// The direction of the chord from `p(s)` to `p(s + len)`, or the
    /// direction of travel at `s` where the chord has no length.
    fn chord(&self, s: f32, len: f32) -> Vec2 {
        chord_from(self.path, self.path.at(s), s, len)
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
    ///
    /// The point at `s` is looked up once and every chord taken from it,
    /// and a step this placement's walks have taken already is read from
    /// its [`StepMemo`]: off a level walk, [`CHORD_ROUNDS`] `+ 2` path
    /// lookups for a step taken afresh, none for one taken before.
    fn step(&self, s: f32, w: f32, h: f32) -> (f32, Vec2) {
        if let Some(step) = self.memo.and_then(|memo| memo.get(s, w, h)) {
            // The memo read in place of the path, counted as one read of
            // it ([`ArcPath::reads`]).
            self.path.read();
            return step;
        }
        let step = self.step_afresh(s, w, h);
        if let Some(memo) = self.memo {
            memo.put(s, w, h, step);
        }
        step
    }

    /// How long the step a `w × h` cluster takes from `s` is: its
    /// [`step`](Self::step)'s length, for a caller that reads no more of
    /// it. On a level walk that is [`level_len`](Self::level_len) alone,
    /// without the chord the step's direction is read from — two path
    /// lookups a step that R27's measures would only discard (#309).
    ///
    /// Cost: [`LEVEL_ROUNDS`] `+ 1` path lookups on a level walk;
    /// [`step`](Self::step)'s on any other.
    fn step_len(&self, s: f32, w: f32, h: f32) -> f32 {
        if self.level {
            #[cfg(test)]
            STEPS_TAKEN.with(|count| count.set(count.get() + 1));
            return self.level_len(s, w);
        }
        self.step(s, w, h).0
    }

    /// [`step`](Self::step), taken.
    fn step_afresh(&self, s: f32, w: f32, h: f32) -> (f32, Vec2) {
        #[cfg(test)]
        STEPS_TAKEN.with(|count| count.set(count.get() + 1));
        if self.level {
            let step = self.level_len(s, w);
            return (step, self.chord(s, step));
        }
        let (from, tangent) = self.path.at(s);
        let chord = |len: f32| chord_from(self.path, (from, tangent), s, len);
        let mut step = extent(tangent, w, h);
        for _ in 0..CHORD_ROUNDS {
            step = extent(chord(step), w, h);
        }
        (step, chord(step))
    }

    /// What a cluster of advance `advance`, ink `ink` (in its box's
    /// coordinates) and whitespace or not (`blank`) takes up of the path,
    /// on a walk whose line height is `h`: its `advance × h` box, but for
    /// an ornament's glyph off a level walk, which is its ink (ruling
    /// R31) — a space's a point, so it steps its advance. A space has no
    /// ink to keep apart; boxed a line high it would step a whole line
    /// wherever the path runs steep, and a glyph-and-space ornament would
    /// open a gap at every turn of a ring. In text a space keeps its line
    /// height, which is what parts two words stacked on a steep side. An
    /// ornament's glyph that draws nothing has no ink either (#333,
    /// [`GlyphMetrics::ink`]), and steps its advance as a space does.
    fn body(&self, advance: f32, ink: Option<Ink>, blank: bool, h: f32) -> Body {
        if !self.ornament || self.level {
            return Body::Boxed { w: advance, h };
        }
        match ink.filter(|_| !blank) {
            Some(ink) => Body::Inked { advance, ink },
            None => {
                let middle = Vec2::new(advance * 0.5, 0.0);
                Body::Inked {
                    advance,
                    ink: Ink {
                        min: middle,
                        max: middle,
                    },
                }
            }
        }
    }

    /// [`body`](Self::body) for `cluster` as shaped.
    fn body_of(&self, cluster: &Cluster) -> Body {
        self.body(cluster.advance, cluster.ink, cluster.blank, self.line_height)
    }

    /// The step `body` takes from `s`, and the direction it is taken in:
    /// a box's by the box rule ([`step`](Self::step)); an ink box's by
    /// the same rule on the ink box, plus its side bearings — the advance
    /// less the ink's width, which do not turn with the path (ruling
    /// R31). On a level line that is the advance, as shaped. Never
    /// negative.
    fn take(&self, s: f32, body: Body) -> (f32, Vec2) {
        let (advance, ink) = match body {
            Body::Boxed { w, h } => return self.step(s, w, h),
            Body::Inked { advance, ink } => (advance, ink),
        };
        let size = ink.max - ink.min;
        let bearings = advance - size.x;
        let along = |dir: Vec2| {
            let step = extent(dir, size.x, size.y) + bearings;
            if step > 0.0 {
                step
            } else {
                0.0
            }
        };
        let (from, tangent) = self.path.at(s);
        let chord = |len: f32| chord_from(self.path, (from, tangent), s, len);
        let mut step = along(tangent);
        for _ in 0..CHORD_ROUNDS {
            step = along(chord(step));
        }
        (step, chord(step))
    }

    /// The center on the canvas of the ink of a glyph whose ink is `ink`
    /// (in its box's coordinates) set at `c` along the path (ruling R31):
    /// on the path there — pushed outward on a closed path by the
    /// [`support`] of the ellipse its ink box bounds, along the normal of
    /// the chord across its own reach, so the ink, not its box, clears a
    /// side of the outline, and the push turns with the chord through a
    /// sharp corner rather than jumping. A frame's text is anchored on its
    /// band curve instead (#308, `halves::Band`).
    ///
    /// Cost: one path lookup; three where the ink is pushed outward.
    fn ink_at(&self, c: f32, ink: Ink) -> Vec2 {
        let size = ink.max - ink.min;
        let (mut center, tangent) = self.path.at(c);
        if self.outward {
            let reach = 0.5 * size.length();
            let dir = if reach > 0.0 {
                self.chord(c - reach, 2.0 * reach)
            } else {
                tangent
            };
            center += support(self.path.outward(dir), size * 0.5);
        }
        center
    }

    /// Walk a cluster of `body` from `s`, `trail` remembering an
    /// ornament's last ink (ruling R31). A box steps by the box rule from
    /// `s` ([`take`](Self::take)). An ornament's space steps its advance,
    /// which it adds to what the next ink is owed. An ornament's glyph is
    /// set its left bearing plus half its ink's extent along the chord
    /// into its step, where a line would set it — unless a glyph came
    /// before, whose ink it is then set as far from as owed
    /// ([`ink_gap_at`](Self::ink_gap_at)), its step moved with it. Either
    /// way, what the next ink is owed starts again at its right bearing.
    ///
    /// Where the path is straight the two agree; where it turns — the
    /// ink pushed outward round a tight corner, two inks either side of a
    /// sharp one — only the second keeps the inks as far apart as a line
    /// sets them.
    fn advance(&self, s: f32, body: Body, trail: &mut Trail) -> Taken {
        let (step, dir) = self.take(s, body);
        let (advance, ink) = match body {
            Body::Boxed { .. } => {
                *trail = Trail::default();
                return Taken {
                    s,
                    step,
                    dir,
                    center: None,
                };
            }
            Body::Inked { advance, ink } => (advance, ink),
        };
        let size = ink.max - ink.min;
        let lead = ink.min.x;
        let natural = s + lead + extent(dir, size.x, size.y) * 0.5;
        if size == Vec2::ZERO {
            // A space: nothing to keep apart, its advance owed.
            trail.owed += advance;
            return Taken {
                s,
                step,
                dir,
                center: Some(natural),
            };
        }
        let owed = trail.owed + lead;
        let center = match trail.last {
            Some((last, half, from)) if owed > INK_TOLERANCE => {
                self.ink_gap_at((last, half), from, natural, owed, ink)
            }
            _ => natural,
        };
        trail.last = Some((self.ink_at(center, ink), size * 0.5, center));
        trail.owed = advance - ink.max.x;
        Taken {
            s: s + (center - natural),
            step,
            dir,
            center: Some(center),
        }
    }

    /// Where along the path to set the center of a glyph whose ink is
    /// `ink` so that its ink box is `owed` from `last`'s — the center and
    /// half size of the ink of the glyph before, set at `from` — the
    /// distance between the two boxes (ruling R31). `natural`, the box
    /// rule's center, where that is within [`INK_TOLERANCE`]; otherwise
    /// up to [`INK_TURNS`] rounds, each solving for the distance between
    /// the two centers that leaves the gap the last round's centers left,
    /// so the gap converges on `owed`, stopping at the first round whose
    /// gap is within the tolerance.
    ///
    /// The centers' distance, unlike the boxes' gap, never stands still
    /// as the glyph moves on — the gap does wherever one box slides past
    /// the other's corner, round a sharp turn — since the distance from a
    /// point on a convex path grows as the other moves away, until it is
    /// half way round. So each round's point, and the glyph, moves
    /// continuously with the glyphs before it, as a ring's spacing solve
    /// needs ([`close_ring`]).
    ///
    /// Cost: at most [`INK_TURNS`] solves
    /// ([`ink_distance_at`](Self::ink_distance_at)), each a path lookup or
    /// two where the path runs straight.
    fn ink_gap_at(&self, (last, half): (Vec2, Vec2), from: f32, natural: f32, owed: f32, ink: Ink) -> f32 {
        let reach = half + (ink.max - ink.min) * 0.5;
        let gap_at = |center: f32| {
            let between = (self.ink_at(center, ink) - last).abs();
            (between.length(), (between - reach).max(Vec2::ZERO).length())
        };
        let mut center = natural;
        for _ in 0..INK_TURNS {
            let (distance, gap) = gap_at(center);
            if (gap - owed).abs() <= INK_TOLERANCE {
                break;
            }
            let next = self.ink_distance_at(last, owed + distance - gap, from, center, ink);
            // A turn that leaves the center where it was leaves it there
            // every turn after: each is a function of the center alone.
            if next.to_bits() == center.to_bits() {
                break;
            }
            center = next;
        }
        center
    }

    /// Where along the path past `from` to set the center of a glyph
    /// whose ink is `ink` so that it is `apart` from `last`: `guess`,
    /// where that is within [`INK_TOLERANCE`]; otherwise by bisection,
    /// between `from` and `guess` where `guess` is too far, and between
    /// it and a stride on where it is too near ([`ink_gap_at`]).
    ///
    /// [`ink_gap_at`]: Self::ink_gap_at
    fn ink_distance_at(&self, last: Vec2, apart: f32, from: f32, guess: f32, ink: Ink) -> f32 {
        let miss = |c: f32| self.ink_at(c, ink).distance(last) - apart;
        let at_guess = miss(guess);
        if at_guess.abs() <= INK_TOLERANCE {
            return guess;
        }
        let (mut low, mut high) = if at_guess > 0.0 {
            (from, guess)
        } else {
            let stride = apart.max((ink.max - ink.min).max_element());
            let mut low = guess;
            let mut high = guess + stride;
            let mut strides = 1;
            // A miss that is not a number is not far enough: stride on.
            while !matches!(
                miss(high).partial_cmp(&0.0),
                Some(Ordering::Greater | Ordering::Equal)
            ) {
                if strides == INK_STRIDES {
                    return high;
                }
                low = high;
                high += stride;
                strides += 1;
            }
            (low, high)
        };
        for _ in 0..INK_ROUNDS {
            let middle = 0.5 * (low + high);
            let before = (low.to_bits(), high.to_bits());
            if miss(middle) < 0.0 {
                low = middle;
            } else {
                high = middle;
            }
            // A round that leaves the bracket as it was — its ends a
            // float apart, where the path's precision gives out — leaves
            // it so every round after: each is a function of the bracket
            // alone.
            if (low.to_bits(), high.to_bits()) == before {
                break;
            }
        }
        high
    }

    /// How far along the path from `s` it moves `d` horizontally, on a
    /// level walk (ruling R28): `L` with `x(s + L) − x(s) = d`. A text
    /// frame's line lies where the path runs left to right within 45° of
    /// level (`cos θ ≥ √½`), so `L` is at most `√2 · d` there, and `x` is
    /// increasing; it is solved by Newton's method on `x`, its slope
    /// held at no less than a tenth so a step run past the span stays
    /// bounded. Never negative.
    ///
    /// Cost: [`LEVEL_ROUNDS`] `+ 1` path lookups at most: fewer where a
    /// round leaves the length where it was, which ends the rounds.
    fn level_len(&self, s: f32, d: f32) -> f32 {
        if d.is_nan() || d <= 0.0 {
            return 0.0;
        }
        let (from, tangent) = self.path.at(s);
        let mut len = d / tangent.x.abs().max(0.1);
        for _ in 0..LEVEL_ROUNDS {
            let (at, tangent) = self.path.at(s + len);
            let miss = at.x - from.x - d;
            let next = (len - miss / tangent.x.max(0.1)).max(0.0);
            // A round is a function of `len` alone: one that leaves it
            // where it was leaves it there every round after (#309).
            if next.to_bits() == len.to_bits() {
                break;
            }
            len = next;
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
    /// is walked from 0 with `gap` between steps, cutting nothing: the
    /// natural length `L` at `gap = 0`.
    fn measure(&self, clusters: &[Cluster], gap: f32) -> f32 {
        self.measure_clusters(clusters.iter(), gap)
    }

    /// Where the last of `clusters`' steps ends, walked from 0 with `gap`
    /// between steps, each taking up what [`body_of`](Self::body_of)
    /// says: the arithmetic of [`lay`](Self::lay), without placing
    /// anything.
    fn measure_clusters<'c>(&self, clusters: impl Iterator<Item = &'c Cluster>, gap: f32) -> f32 {
        self.walk_bodies(
            clusters.map(|cluster| self.body_of(cluster)),
            0.0,
            gap,
            Trail::default(),
        )
        .0
    }

    /// Where the last of `bodies`' steps ends, walked from `from` with
    /// `gap` between steps and `trail` remembering an ornament's last ink
    /// ([`advance`](Self::advance)), and what it remembers after them;
    /// `from` when there are none. The arithmetic of [`lay`](Self::lay).
    /// A ring's walk that finds its budget spent before a body stops
    /// there and ends at `NaN` (#310): its caller reads the budget, not
    /// the end.
    fn walk_bodies(
        &self,
        bodies: impl Iterator<Item = Body>,
        from: f32,
        gap: f32,
        mut trail: Trail,
    ) -> (f32, Trail) {
        let mut s = from;
        let mut end = from;
        for body in bodies {
            if self.spent() {
                return (f32::NAN, trail);
            }
            let taken = self.advance(s, body, &mut trail);
            end = taken.s + taken.step;
            s = end + gap;
            trail.owed += gap;
        }
        (end, trail)
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
    /// settles on fits the placement the readable rule makes of it — but
    /// for an ornament, which is placed as walked (ruling R29), each
    /// glyph by its ink scaled the same way ([`cluster_ink`], ruling
    /// R31).
    ///
    /// Cost: O(glyphs); a few `Vec`s of one entry per cluster, none for
    /// an ornament.
    fn measure_at_scale(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        scale: f32,
        largest: f32,
    ) -> f32 {
        let width = |cluster: &Cluster| {
            glyphs[cluster.glyphs.clone()]
                .iter()
                .map(|glyph| glyph.advance * size_factor(glyph.size, scale))
                .sum::<f32>()
        };
        let h = self.line_height * size_factor(largest, scale);
        if self.ornament && !self.level {
            let bodies = clusters.iter().map(|cluster| {
                let ink = cluster_ink(glyphs, cluster, |glyph| size_factor(glyph.size, scale));
                self.body(width(cluster), ink, cluster.blank, h)
            });
            let end = self.walk_bodies(bodies, 0.0, 0.0, Trail::default()).0;
            // Its walk stopped by its budget (#313): `Fit`'s scale is not
            // solved to the end.
            if self.spent() {
                self.lose(RingLoss::Uneven);
            }
            return end;
        }
        // A measure of the text as read walks it as phase 2 would, reading
        // no budget as it goes: read before it runs, at the most phase 2
        // takes of its glyphs (#313). One the placement cannot afford is
        // not made: the scale is not solved to the end.
        if let Some(work) = self.work {
            if !work.affords_more(self.path, ring::phase_two_steps(glyphs.len(), true)) {
                self.lose(RingLoss::Uneven);
                return f32::NAN;
            }
        }
        let widths: Vec<f32> = clusters.iter().map(width).collect();
        let breaks: Vec<bool> = clusters.iter().map(|cluster| cluster.word_break).collect();
        self.measure_readable(&widths, &breaks, h)
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
                    self.solve(|g| length(stretch, start, g) - target, target, gap)
                }
                Spacing::Fill if fits(0.0) => {
                    let guess = (target - length(stretch, start, 0.0)) / stretch.steps.len().max(2) as f32;
                    self.solve(|g| length(stretch, start, g) - target, target, guess)
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
    /// that would pass [`MAX_PLACED_GLYPHS`] — on a ring's walk, its cap
    /// ([`RingWork::cap`], ruling B17.2a) — or, on a ring's walk, that its
    /// budget does not admit with phase 2's walks of it held back
    /// ([`RingWork::admits`], #310), and says which.
    fn lay<'c>(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: impl IntoIterator<Item = &'c Cluster>,
        mut s: f32,
        gap: f32,
        out: &mut Laying,
    ) -> Laid {
        for cluster in clusters {
            if let Some(work) = self.work {
                if !work.admits(self.path, out.glyphs.len() + cluster.glyphs.len()) {
                    return Laid::Spent;
                }
            }
            let body = self.body_of(cluster);
            let mut trail = out.trail;
            let Taken {
                s: start,
                step,
                dir,
                center,
            } = self.advance(s, body, &mut trail);
            // The same arithmetic, in the same order, as `measure`: a
            // solved spacing that measures within the path must lay
            // within it, to the last bit.
            let end = start + step;
            // An `end` that is not a number overshoots, as it does in
            // `solve_gap`: read as fitting, it would place the cluster and
            // report `Fits(NaN)`. No input reaches it today — every path
            // point is finite (`ArcPath`), and the gaps `lay` is handed are
            // finite (`solve_gap`'s answer, or a spacing `place_on_open`
            // makes finite) — so this holds the next caller to it, not a
            // case a test can construct. (`!(end <= limit)` says the same;
            // clippy's `neg_cmp_op_on_partial_ord` asks for this spelling.)
            if end.is_nan() || end > self.limit {
                return Laid::Overran;
            }
            if out.glyphs.len() + cluster.glyphs.len() > self.work.map_or_else(placed_cap, RingWork::cap) {
                return Laid::Capped;
            }
            let corner = match (body, center) {
                (Body::Inked { ink, .. }, Some(center)) => {
                    self.ink_at(center, ink) - (ink.min + ink.max) * 0.5 - Vec2::new(0.0, self.baseline)
                }
                (Body::Inked { advance, .. }, None) => {
                    self.corner(start, step, dir, advance, self.line_height)
                }
                (Body::Boxed { w, h }, _) => self.corner(start, step, dir, w, h),
            };
            out.trail = trail;
            out.steps.push(Stepped {
                first: out.glyphs.len(),
                end: out.glyphs.len() + cluster.glyphs.len(),
                s: start,
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
            if let Some(work) = self.work {
                work.set(cluster.glyphs.len());
            }
            s = self.after_gap(end, gap);
            out.trail.owed += gap;
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
            gap
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
                    self.solve(|g| pooled(g) - walked_end, walked_end, gap)
                }
                Ends::Within if pooled(gap) <= limit => gap,
                Ends::Within if pooled(0.0) <= end => self.solve(|g| pooled(g) - end, end, gap),
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
    /// A ring's budget (#310) is read before each lay, and its solves
    /// read it before each probe ([`solve`](Self::solve)): spent before
    /// the ring's first gap is solved, the ring is not laid on one gap
    /// (`None`), but as a ring whose walk was solved to end at the path's
    /// end; spent later, it keeps the gap and the stretches of the round
    /// before. Either way it is uneven ([`RingLoss::Uneven`]).
    ///
    /// Cost: per round, a solve of at most `2 · SOLVE_ROUNDS` lays, one
    /// lay more, a sort and a classification; [`EVEN_ROUNDS`] rounds.
    fn even_out(&self, stretches: &mut [PathStretch], steps: &[Stepped]) -> Option<f32> {
        let length = self.path.length();
        let h = self.line_height;
        let width = |index: usize| steps[index].advance;
        // The gap that closes the ring as `stretches` group it, if the
        // ring's budget affords looking for it.
        let close = |stretches: &[PathStretch]| {
            if self.spent() {
                return None;
            }
            let seam = |g: f32| {
                self.begin();
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
            let at_zero = seam(0.0);
            if self.spent() {
                return None;
            }
            let guess = (-at_zero / steps.len().max(1) as f32).max(0.0);
            Some(self.solve(seam, length, guess))
        };
        let Some(mut gap) = close(stretches) else {
            self.lose(RingLoss::Uneven);
            return None;
        };
        for _ in 0..EVEN_ROUNDS {
            if self.spent() {
                self.lose(RingLoss::Uneven);
                break;
            }
            self.begin();
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
            let round_before = stretches.to_vec();
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
            let Some(next) = close(stretches) else {
                // The budget spent before the regrouped ring's gap was
                // looked for: the round before stands.
                stretches.clone_from_slice(&round_before);
                self.lose(RingLoss::Uneven);
                break;
            };
            gap = next;
        }
        Some(gap)
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
    /// was cut. On a ring's walk, a search cut short by its budget or its
    /// cap rather than by the path — a probe stopped by either, whose
    /// prefix may have fit the path — says so: [`Laid::Spent`] or
    /// [`Laid::Capped`], whatever its final lay, which keeps the prefix
    /// that fit (ruling B17.1b; review 5761697746: a ring whose every probe
    /// found its budget spent kept no cluster and reported nothing). Every
    /// other walk's search reads a capped probe as one that overran. On
    /// every walk, clusters that pass the cap are cut as clusters that
    /// pass the path's end are — the longest prefix in reading order that
    /// fits under it — and the cut says so ([`Laid::Capped`]).
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
        // The worst stop of a lay below that a ring's budget or the cap
        // stopped, not the path.
        let mut stopped = None;
        match self.lay(glyphs, clusters, s, gap, out) {
            Laid::Overran => {}
            // The cap reached part way through the whole lay: what it
            // reached is a prefix in visual order too, so the cap cuts as
            // the path does — the longest prefix in reading order that
            // fits under it — and says so (`Laid::Capped`).
            Laid::Capped => stopped = Some(Laid::Capped),
            // A ring's budget spent part way through the whole lay leaves
            // none of it, as below: what it reached is a prefix in visual
            // order, which need not be one in reading order (review
            // 5800041728).
            Laid::Spent => {
                out.truncate(mark);
                return (Laid::Spent, false);
            }
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
        // one that does not (all of them, just seen), or that a ring's
        // budget or its cap stopped: `stopped`, the worst such stop.
        let (mut fits, mut overruns) = (0usize, clusters.len());
        while overruns - fits > 1 {
            let middle = fits + (overruns - fits) / 2;
            match lay_prefix(middle, out) {
                Laid::Fits(_) => fits = middle,
                Laid::Overran => overruns = middle,
                stop @ (Laid::Capped | Laid::Spent) => {
                    overruns = middle;
                    if self.work.is_some() && stopped != Some(Laid::Spent) {
                        stopped = Some(stop);
                    }
                }
            }
        }
        let laid = lay_prefix(fits, out);
        // A ring's budget spent part way through the prefix leaves none of
        // it: a part of a prefix in visual order need not be one in
        // reading order (#310).
        if laid == Laid::Spent {
            out.truncate(mark);
        }
        match (laid, stopped) {
            (Laid::Fits(_), Some(stop)) => (stop, true),
            _ => (laid, true),
        }
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
/// in `direction`, as `fit` and `layout` say (#239 §5, §6).
///
/// A ring in `Once`, `Spread` or `Fit` is text, set in two lines (#291,
/// ruling R27) and ignoring `start` and `direction`: R27 parts and sizes
/// it, and each line runs along its half of the outline split on the
/// diagonal (ruling R33) — the first from the top-left junction over
/// the top and down the right side, the second from it down the left
/// side and along the bottom — with even ink tracking (ruling R32). At
/// [`LineSpan::NATURAL`] a line is at its font's own gap, centered on its
/// half's apex; above it, its ends move from there toward the
/// junctions by `span`, each junction a visible break of two word gaps
/// (ruling R33.1). `sides` closes what the lines leave round the
/// junctions with an ornament.
/// A `Repeat` ring is walked whole from `start`, every stretch read
/// forward, `layout.spacing` more between its steps.
///
/// With `outward`, each cluster is pushed off the outline by half its
/// extent along the outward normal, so the ring sits outside the
/// outline; without, cluster centers sit on it. A ring placed around
/// `offset(outline, ring_offset(...))` clears the rings inside it.
/// "Fits" means the last cluster's step ends at or before the path's
/// length: the start point is not reused.
///
/// `flow` is the flow's own resolved style (sizes in points), read to
/// shape a `Once` filler (ruling R6) and a frame's sides. `fs` is the
/// caller's lease on the font system: `Fit` (reshaping at the fitted
/// size), a filler, sides and a line's ink measure take the lock from
/// it to shape or scale glyphs, and a walk or a lay, which reads no
/// font, gives it back ([`FontSystemLease::unlocked`], #308), so the
/// lease may hold nothing when this returns.
///
/// Returns the run the placement refers to when it is not `run` — the
/// rescaled run for `Fit`, `run` plus one shaped copy of a filler or a
/// frame's side ornament, each a line of its own — and the placement.
///
/// Cost: one arc-length path (O(1) for a rounded rectangle or a circle,
/// a table for an ellipse); O(clusters placed) steps, each a handful of
/// path lookups; `Spread` and `Fit` measure the run a bounded number of
/// times more to solve their end condition, and a cut a logarithmic
/// number; `Fit` and a filler shape once. A line along its half solves
/// its tracking by bisection, and its junction clearance in a few
/// rounds more (`halves::place_halves`).
#[allow(clippy::too_many_arguments)] // #239's signature, as amended by rulings R6 and R32.
pub fn place_on_closed(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_closed_traced(run, path, start, direction, fit, layout, outward, flow, fs, None)
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
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_closed_capped(
        run,
        path,
        start,
        direction,
        fit,
        layout,
        outward,
        flow,
        fs,
        MAX_PLACED_GLYPHS,
        trace,
    )
}

/// [`place_on_closed_traced`], a `Repeat` ring laying at most `ring_cap`
/// glyphs, and its budget scaled by no more ([`MAX_PLACED_GLYPHS`] at
/// most; ruling B17.2a): what the flow fuzzers lay a ring with, so that
/// its work fits what an input may take whatever its text and outline. A
/// ring its cap stops is short ([`RingLoss::Short`]). Every other fit
/// mode is laid as [`place_on_closed_traced`] lays it.
#[allow(clippy::too_many_arguments)] // `place_on_closed_traced`'s, and the cap.
pub(crate) fn place_on_closed_capped(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    ring_cap: usize,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let laid = lay_on_closed(
        run, path, start, direction, fit, layout, outward, flow, fs, ring_cap, trace,
    );
    #[cfg(test)]
    identity::note(&laid);
    laid
}

/// [`place_on_closed_capped`]'s placement, which a test notes for the
/// identity dump (`identity::note`) as it returns.
#[allow(clippy::too_many_arguments)] // `place_on_closed_capped`'s.
fn lay_on_closed(
    run: &ShapedRun,
    path: &Outline,
    start: StartPoint,
    direction: Direction,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    ring_cap: usize,
    mut trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    if !matches!(fit, FitMode::Repeat) {
        // A frame whose outline is too large for its length to be an
        // `f32` is laid on the outline shrunk about its center until its
        // length is one, and its positions grown back: every position
        // finite, the lines where they would lie, spread to the outline.
        // Only a rounded rectangle's can be, which builds no table to say
        // so (`closed_length_is_finite`, #309).
        if !ArcPath::closed_length_is_finite(path, start, direction) {
            let center = path.rect().center();
            let mut factor = 1.0_f32;
            let mut small = *path;
            for _ in 0..MAX_SHRINKS {
                if ArcPath::closed_length_is_finite(&small, start, direction) {
                    break;
                }
                factor *= 0.5;
                small = path.scaled(factor);
            }
            let (replaced, mut placed) =
                halves::place_halves(run, &small, fit, layout, outward, flow, fs, trace);
            for glyph in &mut placed.glyphs {
                glyph.pos = center + (glyph.pos - center) / factor;
            }
            return (replaced, placed);
        }
        return halves::place_halves(run, path, fit, layout, outward, flow, fs, trace);
    }
    if layout.directional.is_set() {
        if let Some(laid) = directional::place_directional(
            run,
            path,
            layout,
            outward,
            flow,
            fs,
            ring_cap,
            trace.as_deref_mut(),
        ) {
            return laid;
        }
    }
    let spacing = table_spacing(run, fit, flow);
    let arc = ArcPath::closed(path, start, direction, spacing);
    // A gap past the whole outline lays one copy: the walk stops at the
    // first step that ends past the path (`PathWalk::lay`).
    let gap = if layout.spacing.is_finite() {
        layout.spacing.max(0.0)
    } else {
        0.0
    };
    place(run, &arc, true, fit, gap, outward, flow, fs, ring_cap, trace)
}

/// Place `run` along the open `path` (a connector or a line link) — a
/// [`Curve`], or a chain of them — from its source end, as `fit` says,
/// with `spacing` between clusters (#239 §5, §6).
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
    curve: &(impl AsRef<[Curve]> + ?Sized),
    fit: &FitMode,
    spacing: f32,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_open_traced(run, curve, fit, spacing, flow, fs, None)
}

/// [`place_on_open`], filling `trace` with what the walk did: the test
/// seam for the readable rule (#291).
pub(crate) fn place_on_open_traced(
    run: &ShapedRun,
    curve: &(impl AsRef<[Curve]> + ?Sized),
    fit: &FitMode,
    spacing: f32,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    place_on_open_capped(run, curve, fit, spacing, flow, fs, MAX_PLACED_GLYPHS, trace)
}

/// [`place_on_open_traced`], laying at most `cap` glyphs, and its budget
/// scaled by no more ([`MAX_PLACED_GLYPHS`] at most; #313, as ruling
/// B17.2a has a ring's): what the flow fuzzers lay an open path's
/// placement with, so that its work fits what an input may take whatever
/// its text and its path. A placement its cap stops is short
/// ([`RingLoss::Short`]).
#[allow(clippy::too_many_arguments)] // `place_on_open_traced`'s, and the cap.
pub(crate) fn place_on_open_capped(
    run: &ShapedRun,
    curve: &(impl AsRef<[Curve]> + ?Sized),
    fit: &FitMode,
    spacing: f32,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    cap: usize,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let table = table_spacing(run, fit, flow);
    let arc = ArcPath::open(curve, table);
    let spacing = if spacing.is_finite() {
        spacing.max(0.0)
    } else {
        0.0
    };
    let laid = place(run, &arc, false, fit, spacing, false, flow, fs, cap, trace);
    #[cfg(test)]
    identity::note(&laid);
    laid
}

/// Test seam (#309, review 5810838863): a `w`-wide cluster's step from `s`
/// along `path` on a level walk (ruling R28), `h` tall — the length R27's
/// measures read of it ([`PathWalk::step_len`]), the length of the step
/// itself ([`PathWalk::step`]), and the path lookups the measure took.
#[cfg(test)]
pub(crate) fn level_step_seam(path: &ArcPath, s: f32, w: f32, h: f32) -> (f32, f32, usize) {
    use crate::flow::path::PATH_LOOKUPS;
    let walk = PathWalk {
        path,
        memo: None,
        work: None,
        line_height: h,
        outward: true,
        limit: f32::INFINITY,
        level: true,
        ornament: false,
        baseline: 0.0,
    };
    let before = PATH_LOOKUPS.with(std::cell::Cell::get);
    let len = walk.step_len(s, w, h);
    let lookups = PATH_LOOKUPS.with(std::cell::Cell::get) - before;
    (len, walk.step(s, w, h).0, lookups)
}

/// Test seam (#309): every placement a test makes, hashed bit for bit
/// into `$MANDALA_FLOW_DUMP/<test>.txt` when that is set, so that two
/// trees' placements can be held to one another over the whole suite.
#[cfg(test)]
pub(crate) mod identity {
    use super::{PlacedRun, ShapedRun};
    use std::io::Write;

    /// FNV-1a over `words`, from `seed`.
    fn fnv(seed: u64, words: impl IntoIterator<Item = u32>) -> u64 {
        let mut h = seed;
        for w in words {
            for b in w.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    pub(crate) fn note((run, placed): &(Option<ShapedRun>, PlacedRun)) {
        let Some(dir) = std::env::var_os("MANDALA_FLOW_DUMP") else {
            return;
        };
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").replace("::", ".");
        let mut h = fnv(
            0xcbf2_9ce4_8422_2325,
            [placed.glyphs.len() as u32, u32::from(placed.truncated)],
        );
        h = fnv(
            h,
            placed
                .glyphs
                .iter()
                .flat_map(|g| [g.glyph, g.pos.x.to_bits(), g.pos.y.to_bits()]),
        );
        if let Some(run) = run {
            h = fnv(
                h,
                run.glyphs
                    .iter()
                    .flat_map(|g| [g.size.to_bits(), g.advance.to_bits(), g.x.to_bits()]),
            );
        }
        let path = std::path::Path::new(&dir).join(format!("{name}.txt"));
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{} {} {h:016x}", placed.glyphs.len(), placed.truncated);
            // With `MANDALA_FLOW_DUMP_POSITIONS`, every position too, so two
            // dumps' placements can be measured against one another.
            if std::env::var_os("MANDALA_FLOW_DUMP_POSITIONS").is_some() {
                for g in &placed.glyphs {
                    let _ = writeln!(file, "  {} {} {}", g.glyph, g.pos.x, g.pos.y);
                }
            }
        }
    }
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

/// The samples the paths of a placement of `run` round `outline` in
/// `fit` build their tables with ([`ArcPath::table_samples`]), counted
/// in its setup (ruling B16.3a): a `Repeat` ring's one path, or a frame's
/// two halves ([`frame_halves`]), at [`table_spacing`]. What the flow
/// fuzzers bound a frame's setup by, before it is laid; a frame that
/// builds other tables than these fails their property.
///
/// Cost: O(1), no table built ([`ArcPath::closed_table_samples`]).
pub(crate) fn setup_table_samples(
    run: &ShapedRun,
    outline: &Outline,
    fit: &FitMode,
    flow: &ResolvedText,
) -> usize {
    let path = ArcPath::closed_table_samples(outline, table_spacing(run, fit, flow));
    if matches!(fit, FitMode::Repeat) {
        path
    } else {
        path.saturating_mul(2)
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
    fs: &mut FontSystemLease,
    cap: usize,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let memo = StepMemo::default();
    // Every placement here is budgeted as a frame is, by the glyphs it
    // will lay, at most `cap`: a ring of repeats (#310; rulings B17.1a,
    // B17.2a), and every mode along an open path (#313) — `Repeat` by the
    // copies its path holds, as a ring, the others by their text, and
    // `Once` with a filler by its copies too once the filler is shaped
    // (`once`).
    // An open path's cap is the one every walk but a ring's has: in a
    // test, the frame's cap seam's (`placed_cap`).
    let cap = cap.min(if closed { MAX_PLACED_GLYPHS } else { placed_cap() });
    let glyphs = if matches!(fit, FitMode::Repeat) {
        let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len()).len();
        let advance = ring::copy_advance(&run.glyphs, clusters, spacing);
        #[cfg(test)]
        if closed {
            LAST_RING_ESTIMATE.with(|last| last.set(Some((run.glyphs.len(), advance, path.length()))));
        }
        ring::ring_glyphs(run.glyphs.len(), advance, path.length(), cap)
    } else {
        run.glyphs.len().min(cap)
    };
    let work = RingWork::new(
        path,
        run.glyphs.len(),
        run.glyphs.iter().any(|glyph| glyph.letter),
        run.line_height,
        glyphs,
        cap,
    );
    let walk = |run: &ShapedRun| PathWalk {
        path,
        memo: Some(&memo),
        work: Some(&work),
        line_height: run.line_height,
        outward,
        limit: path.length() + end_slack(path.length()),
        level: false,
        ornament: !run.glyphs.iter().any(|glyph| glyph.letter),
        baseline: run.baseline(),
    };
    // A `Repeat` or `Spread` walk, and every readable lay, reads the run
    // as shaped and no font: they run with the font system's lock
    // released ([`FontSystemLease::unlocked`], #308).
    let walked = match fit {
        FitMode::Once { filler } => once(run, spacing, filler.as_deref(), flow, fs.fonts(), &walk),
        FitMode::Repeat => fs.unlocked(|| repeat(run, &walk(run), spacing, closed)),
        FitMode::Spread => fs.unlocked(|| spread(run, &walk(run), 0.0)),
        FitMode::Fit { min, max } => fit_to(run, path, *min, *max, fs.fonts(), &walk),
    };
    work.lay_again();
    let mut trace = trace;
    let placed = fs.unlocked(|| walk(walked.run.as_ref().unwrap_or(run)).readable(walked, trace.as_deref_mut()));
    work.placed(placed.1.glyphs.len());
    let spent = work.spent(path);
    let (budget, glyphs, _) = work.budget();
    if let Some(trace) = trace {
        trace.spent = spent;
        trace.tables = work.tables();
        trace.budget = budget;
        trace.glyphs = glyphs;
        trace.ring = work.loss();
    }
    // All the placement takes past its budget (ruling R50.4): what is held
    // back for phase 2 is within it.
    #[cfg(test)]
    {
        halves::PAST_BUDGET.with(|past| past.set(past.get().saturating_add(spent.saturating_sub(budget))));
        let laid = LastRing {
            copy: work.budget().2,
            glyphs,
            cap: work.cap(),
            placed: placed.1.glyphs.len(),
            spent,
            budget,
            tables: work.tables(),
            loss: work.loss(),
            held: work.held_again(),
        };
        if closed {
            LAST_RING.with(|last| last.set(Some(laid)));
            RING_CROSSED.with(|crossed| crossed.set(work.crossed()));
            RING_LATE.with(|late| late.set(work.late()));
            if work.loss().is_some() {
                RINGS_STOPPED.with(|count| count.set(count.get() + 1));
            }
        } else {
            LAST_OPEN.with(|last| last.set(Some((laid, work.crossed(), work.late()))));
            if work.loss().is_some() {
                OPENS_STOPPED.with(|count| count.set(count.get() + 1));
            }
        }
    }
    #[cfg(test)]
    GLYPHS_PLACED.with(|count| count.set(count.get() + placed.1.glyphs.len()));
    placed
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
    // Its lay, or its cut, stopped by its budget or its cap, not the path
    // (#313): short of the text, and said so.
    if matches!(laid, Laid::Spent | Laid::Capped) {
        walk.lose(RingLoss::Short);
    }
    let truncated = cut || matches!(laid, Laid::Spent | Laid::Capped);
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
    let (filled, _) = reshape(run, 1.0, Some((filler, flow)), fs);
    let text_glyphs = filled.glyphs.iter().take_while(|g| g.span != FILLER_SPAN).count();
    if text_glyphs == filled.glyphs.len() {
        return placed;
    }
    // The filled run may be taller (a larger filler), which moves the
    // text; place everything again with its line height.
    let walk = walk_for(&filled);
    let text = clusters_of(&filled.glyphs, 0..text_glyphs);
    let copy = clusters_of(&filled.glyphs, text_glyphs..filled.glyphs.len());
    // Its budget scaled by its copies too, as a `Repeat`'s is by the copies
    // its path holds (#313; ruling B17.1a): the text and the copies of the
    // filler the path past it holds.
    if let Some(work) = walk.work {
        let filler_glyphs = filled.glyphs.len() - text_glyphs;
        let advance = ring::copy_advance(&filled.glyphs[text_glyphs..], copy.len(), gap);
        let room = walk.path.length() - next.unwrap_or(0.0);
        work.scale(text_glyphs.saturating_add(ring::ring_glyphs(filler_glyphs, advance, room, work.cap())));
    }
    let mut glyphs = Laying::with_capacity(filled.glyphs.len());
    let mut s = match walk.lay(&filled.glyphs, &text, 0.0, gap, &mut glyphs) {
        Laid::Fits(s) => s,
        // Its budget spent laying its text again, or its cap reached, it
        // keeps its text as first laid, short of its copies (#313).
        Laid::Spent | Laid::Capped => {
            walk.lose(RingLoss::Short);
            return placed;
        }
        Laid::Overran => return placed,
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
            Laid::Capped | Laid::Spent => {
                // Its budget spent, or its cap reached, before its copies
                // fill the path (#313): the copies it has, said so.
                glyphs.truncate(before);
                walk.lose(RingLoss::Short);
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
/// Around a closed path the ring's work is budgeted as a frame's is
/// (#310, [`ring`]): its walks read the budget before each cluster, and
/// one that finds it spent stops there — the walk with the copies it has
/// laid, the ring's close with its walk unclosed — and says so
/// ([`RingLoss`]). Its walk stops at its cap too ([`RingWork::cap`],
/// ruling B17.2a), and says so.
///
/// Cost: one lay per copy; around a closed path, a solve of the spacing
/// of at most `2 · SOLVE_ROUNDS` measures of the copies and one lay more,
/// within the ring's budget.
fn repeat(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32, closed: bool) -> Walked {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let mut glyphs = Laying::default();
    let mut s = 0.0_f32;
    let mut copies = 0usize;
    let truncated = loop {
        let before = glyphs.mark();
        // Stopped in this copy, the walk keeps the copies before it, which
        // end a gap before it starts: what phase 2's run on is held back
        // from (ruling B17.1d).
        if let Some(work) = walk.work {
            work.ends_by(s - gap);
        }
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
                // Its last copy's cut stopped by the budget or the cap,
                // not the path (ruling B17.1b).
                if matches!(laid, Laid::Spent | Laid::Capped) {
                    walk.lose(RingLoss::Short);
                }
                break copies == 0 || laid == Laid::Capped;
            }
            Laid::Capped => {
                // The cap reached before the copies fill the path (ruling
                // B17.2a): the copies it has, unclosed.
                glyphs.truncate(before);
                walk.lose(RingLoss::Short);
                break true;
            }
            Laid::Spent => {
                // The ring's budget spent before its copies fill the path:
                // the copies it has, unclosed.
                glyphs.truncate(before);
                walk.lose(RingLoss::Short);
                break copies == 0;
            }
        }
    };
    let closing = if closed && copies > 0 && !truncated && walk.work.is_none_or(|w| w.loss().is_none()) {
        close_ring(run, &clusters, walk, copies, gap)
    } else {
        None
    };
    // Closed, the ring takes no run on in phase 2: its hold lets it go
    // (ruling B17.1d).
    if let (Some((laying, _)), Some(work)) = (&closing, walk.work) {
        work.closed(laying.glyphs.len());
    }
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

/// The copies [`close_ring`] drops one at a time before it bisects for
/// the most whose seam closes.
const RING_DROPS: usize = 4;

/// The most of `copies` copies, at least one, whose ring `closes` — its
/// seam as wide as its other gaps, which it is for fewer copies and not
/// for more: [`close_ring`]'s count. A copy dropped at a time, as a ring
/// a copy or two short of closing takes, and past [`RING_DROPS`] of them
/// by bisection.
///
/// Cost: `closes` asked at most `RING_DROPS + 1 + log₂(copies)` times.
pub(crate) fn ring_copies(mut copies: usize, closes: impl Fn(usize) -> bool) -> usize {
    let mut dropped = 0;
    while copies > 1 && !closes(copies) {
        if dropped == RING_DROPS {
            // `copies` does not close; the most below it that does, or 1.
            let (mut low, mut high) = (1, copies);
            while high - low > 1 {
                let mid = low + (high - low) / 2;
                if closes(mid) {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            return low;
        }
        copies -= 1;
        dropped += 1;
    }
    copies
}

/// A closed ring of whole copies of `clusters` (ruling R29): the most
/// copies, at most `copies`, whose seam — from the last copy's last
/// cluster back round to the first's first — can be as wide as every
/// other gap with `gap` or more between steps, and the copies laid with
/// the gap, at least `gap`, that makes it so. The seam is measured as the
/// gaps are: between steps along the path, or, for an ornament, between
/// inks (ruling R31) — the last glyph's ink box as far from the first's
/// as a line would set them, the gaps between steps included. `None`
/// when the copies would pass [`MAX_PLACED_GLYPHS`] or do not lay within
/// the path, or when the ring's budget is spent before they are laid
/// (#310; [`RingLoss::Unclosed`]).
///
/// Cost: a solve of at most `2 · SOLVE_ROUNDS` measures of the copies,
/// a measure more per copy dropped, [`RING_DROPS`] of them and then a
/// bisection of the counts below, and one lay of them.
fn close_ring(
    run: &ShapedRun,
    clusters: &[Cluster],
    walk: &PathWalk<'_>,
    copies: usize,
    gap: f32,
) -> Option<(Laying, f32)> {
    let length = walk.path.length();
    let bodies = |count: usize| {
        (0..count)
            .flat_map(|_| clusters.iter())
            .map(|cluster| walk.body_of(cluster))
    };
    // The first copy's clusters up to its first glyph with ink, when the
    // ring's seam is measured between inks.
    let first_ink = clusters
        .iter()
        .position(|cluster| matches!(walk.body_of(cluster), Body::Inked { ink, .. } if ink.min != ink.max));
    // How far the seam, the copies `extra` past `gap` apart, falls short
    // of the other gaps: zero when it is one like them, positive when it
    // is narrower. It narrows continuously as the copies spread, the
    // last glyph nearing the first.
    let miss = |count: usize, extra: f32| {
        let g = gap + extra;
        let (end, trail) = walk.walk_bodies(bodies(count), 0.0, g, Trail::default());
        let Some(k) = first_ink else {
            return end + g - length;
        };
        let head = || clusters[..k].iter().map(|cluster| walk.body_of(cluster));
        let first = walk.body_of(&clusters[k]);
        let (_, start) = walk.walk_bodies(head().chain(std::iter::once(first)), 0.0, g, Trail::default());
        // What the first glyph is owed from the last: what is owed after
        // the copies, every space before the first glyph, and its bearing.
        let (_, owing) = walk.walk_bodies(head(), end + g, g, trail);
        let (Some((last, last_half, last_c)), Some((first_at, first_half, first_c)), Body::Inked { ink, .. }) =
            (trail.last, start.last, first)
        else {
            return end + g - length;
        };
        let owed = owing.owed + ink.min.x;
        // Past the first glyph, round the ring again, the seam is as
        // narrow as it gets, and narrower the further on.
        let ahead = first_c + length - last_c;
        if ahead <= 0.0 {
            return owed - ahead;
        }
        let between = (first_at - last).abs() - last_half - first_half;
        owed - between.max(Vec2::ZERO).length()
    };
    // The most copies whose seam is as wide as the other gaps: a copy
    // dropped at a time, as a ring a copy or two short of closing takes,
    // and past [`RING_DROPS`] of them by bisection, the seam narrowing as
    // the copies grow. Dropped one at a time all the way, a ring whose
    // seam no count closes measured every count from its most down: on a
    // path too far out for an `f32` to set its glyphs apart, some 22,000
    // counts of up to 65,536 glyphs (seed 2771 of the structured flow
    // generator, an ellipse 10²² from the origin).
    // A miss that is not a number closes it, as the drop it replaces read.
    // A ring's budget (#310) is read before each body its measures walk:
    // spent, they stop, and so does the close, its walk unclosed.
    let unclosed = || {
        walk.lose(RingLoss::Unclosed);
        None
    };
    let copies = ring_copies(copies, |count| {
        miss(count, 0.0).partial_cmp(&SOLVE_TOLERANCE) != Some(Ordering::Greater)
    });
    if walk.spent() {
        return unclosed();
    }
    let at_zero = miss(copies, 0.0);
    if walk.spent() {
        return unclosed();
    }
    let extra = if at_zero >= 0.0 {
        0.0
    } else {
        let guess = -at_zero / (copies * clusters.len()) as f32;
        solve_gap_until(|extra| miss(copies, extra), length, guess, &|| walk.spent())
    };
    if walk.spent() {
        return unclosed();
    }
    let g = gap + extra;
    // The seam was measured; the path's end is not where the ring ends.
    // An ornament's last space, or its last glyph where the ring's inks
    // were set further on than its steps, may step past it, its ink then
    // where the path begins again, as far from the first as owed.
    let unbounded = PathWalk {
        limit: f32::INFINITY,
        ..*walk
    };
    let mut laying = Laying::default();
    let mut s = 0.0_f32;
    for _ in 0..copies {
        match unbounded.lay(&run.glyphs, clusters, s, g, &mut laying) {
            Laid::Fits(next) => s = next,
            Laid::Spent => return unclosed(),
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
    let natural = walk.measure(&clusters, 0.0);
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
    // Read against the placement's budget before each probe (#313): a
    // solve it stops keeps the fitting side of its bracket, the text laid
    // short of the path's end, and is uneven.
    let gap = walk.solve(|gap| walk.measure(&clusters, gap) - length, length, guess);
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
///
/// A doubling that reaches a gap as long as the whole path and leaves
/// the walk ending no further on than the probe before stops there
/// (#310): the path's floats cannot move a step by any gap, so no larger
/// one moves it either — the far-out case, where the doubling took every
/// one of its rounds, each a walk of the whole run. A gap that small
/// moves nothing sooner where the path's floats are coarser than it, and
/// a larger one may: before the whole path's length the doubling goes
/// on.
fn solve_gap(miss: impl Fn(f32) -> f32, length: f32, guess: f32) -> f32 {
    solve_gap_until(miss, length, guess, &|| false)
}

/// Test seam (#310): [`solve_gap`], and how many times it looked at
/// `miss`.
#[cfg(test)]
pub(crate) fn solve_gap_seam(miss: impl Fn(f32) -> f32, length: f32, guess: f32) -> (f32, usize) {
    let looks = Cell::new(0);
    let gap = solve_gap(
        |e| {
            looks.set(looks.get() + 1);
            miss(e)
        },
        length,
        guess,
    );
    (gap, looks.get())
}

/// [`solve_gap`], read against a budget (#310): `stop` is read before
/// each look at `miss`, and once it answers `true` the solve ends at the
/// fitting side of its bracket as it stands — `0.0`, before any look.
fn solve_gap_until(miss: impl Fn(f32) -> f32, length: f32, guess: f32, stop: &dyn Fn() -> bool) -> f32 {
    let tolerance = SOLVE_TOLERANCE.max(length * 1e-6);
    if stop() {
        return 0.0;
    }
    let (mut low, mut low_miss) = (0.0_f32, miss(0.0));
    // Find a gap that overshoots, doubling the guess.
    let mut high = None;
    let mut probe = guess.max(f32::EPSILON);
    for _ in 0..SOLVE_ROUNDS {
        if stop() {
            return low;
        }
        let probe_miss = miss(probe);
        // A miss that is not a number overshoots: the walk ran past every
        // `f32` (a gap that large on a path near the largest one), so it
        // is no fitting low side. Taken as one, the doubling ran on to an
        // infinite gap and placed every cluster after the first at `NaN`
        // (a CI smoke finding of #293).
        if probe_miss.is_nan() || probe_miss >= 0.0 {
            high = Some((probe, probe_miss));
            break;
        }
        if probe >= length && probe_miss <= low_miss {
            return low;
        }
        (low, low_miss) = (probe, probe_miss);
        probe *= 2.0;
    }
    let Some((mut high, mut high_miss)) = high else {
        return low;
    };
    let mut side = 0i8;
    for _ in 0..SOLVE_ROUNDS {
        if low_miss >= -tolerance || high - low <= f32::EPSILON * high || stop() {
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
    let natural = walk.measure(&clusters, 0.0);
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
        let rescaled = (scale != 1.0).then(|| reshape(run, scale, None, fs).0);
        let placed_run = rescaled.as_ref().unwrap_or(run);
        let slack = if scale > low { SOLVE_TOLERANCE } else { 0.0 };
        let placed_walk = walk_for(placed_run);
        // Reshaped smaller, its walks step more often past where they end:
        // the run on held back for is at its line height (#313).
        if let Some(work) = placed_walk.work {
            work.steps_at(placed_run.line_height);
        }
        let walked = Walked {
            slack,
            whole_words: scale > low,
            even: false,
            ..spread(placed_run, &placed_walk, slack)
        };
        // Its read measured as phase 2 would lay it, which reads no budget
        // as it goes: read before it runs, at the most phase 2 takes of the
        // glyphs laid (#313). A placement that cannot afford it keeps this
        // round's walk, which phase 2 cuts where it overruns: its scale is
        // not solved to the end.
        if let Some(work) = placed_walk.work {
            let glyphs = walked.laying.glyphs.len();
            if !work.affords_more(path, ring::phase_two_steps(glyphs, true)) {
                work.lose(RingLoss::Uneven);
                return Walked {
                    run: rescaled,
                    ..walked
                };
            }
        }
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
pub(crate) fn size_factor(size: f32, scale: f32) -> f32 {
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
