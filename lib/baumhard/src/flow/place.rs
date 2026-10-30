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
//! **Every stretch reads left to right or top to bottom** (the readable
//! rule, #291, rulings R25 and R25.1). Glyphs are never rotated, so text
//! walked in its own order along a path that runs leftward would read
//! backward, and along one that runs upward bottom to top. Placement is
//! therefore two phases:
//!
//! 1. **The walk** — the fit mode, as above — decides which clusters are
//!    placed and where along the path each one's step starts. The path
//!    splits into **stretches**, each a maximal run of steps of one
//!    [`Travel`]: rightward (`cos θ > ε`), leftward (`cos θ < −ε`), or
//!    downward or upward (`y` grows downward). A run of near-vertical
//!    steps is a stretch of its own only when it is clearly entered
//!    ([`VERTICAL_MIN_LINES`], [`VERTICAL_EPSILON`]); a curve turning
//!    through vertical joins the stretches on either side, split where it
//!    passes plumb. A cluster belongs to the stretch its step is in.
//! 2. **The lay** lays the same clusters forward along the path in
//!    **reading order**: stretch after stretch, each leftward and upward
//!    stretch's clusters last to first, so that stretch reads left to
//!    right, or top to bottom. Reversed, a stretch's clusters need not
//!    take the length they took walked — a wide glyph moved onto a
//!    vertical part steps the line height, not its advance — so one
//!    uniform spacing is solved for the whole placement: `Spread` and
//!    `Fit` end where the walk ended, `Once` and `Repeat` keep their
//!    spacing unless that would pass the path's end. The gap never goes
//!    below zero; where even no gap passes the end, the placement is cut
//!    — whole filler copies or repeat clusters first, the text last —
//!    and `Fit` measures text as read when it chooses its scale, so it
//!    cuts only at `min`.
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

/// ε of the readable rule (#291): a step whose direction θ has
/// `|cos θ| ≤ ε` is near vertical — within about 2° of plumb, as a
/// node's straight side is exactly. Any more horizontal motion makes it
/// rightward or leftward, so a connector that leans is read left to
/// right, along its lean.
pub(crate) const VERTICAL_EPSILON: f32 = 0.035;

/// The hysteresis of the readable rule (#291), in line heights: a run of
/// near-vertical steps is a vertical stretch of its own only when it
/// spans at least this much of the path (or is the whole placement).
/// A shorter one — a curve turning through vertical, as every circle and
/// ellipse does at its sides — stays with the horizontal stretches
/// around it, so the turn makes no stretch of a glyph or two. On a
/// circle the near-vertical band is `2·R·asin ε ≈ 0.07·R` long, so a
/// loop needs a radius of some thirty line heights before its sides read
/// as columns; a straight side three lines long already does.
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
            }
            _ => clusters.push(Cluster {
                glyphs: index..index + 1,
                advance: glyph.advance,
                left: glyph.x,
                reading,
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
    pub(crate) stretches: Vec<TracedStretch>,
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
        let mut step = extent(self.path.at(s).1, w, h);
        for _ in 0..CHORD_ROUNDS {
            step = extent(self.chord(s, step), w, h);
        }
        (step, self.chord(s, step))
    }

    /// The top-left corner of a `w × h` cluster box whose step `step`,
    /// taken in direction `dir`, starts at `s` (#239 §5 steps 3 and 4).
    fn corner(&self, s: f32, step: f32, dir: Vec2, w: f32, h: f32) -> Vec2 {
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
        self.measure_widths(
            clusters.iter().map(|cluster| cluster.advance * scale),
            self.line_height * scale,
            gap,
        )
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
        self.measure_readable(&widths, self.line_height * size_factor(largest, scale))
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
    fn measure_readable(&self, widths: &[f32], h: f32) -> f32 {
        let mut steps = Vec::with_capacity(widths.len());
        let mut s = 0.0_f32;
        for &width in widths {
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
            });
            s += step;
        }
        let walked = steps.last().map_or(0.0, |step| step.s + step.step);
        let stretches = stretches_of(&self.travels(&steps, h));
        if !stretches.iter().any(is_reversed) {
            return walked;
        }
        let read = self.measure_widths(
            reading_order(&stretches).into_iter().map(|index| widths[index]),
            h,
            0.0,
        );
        walked.max(read)
    }

    /// Where the last of these cluster widths' steps ends, walked from 0
    /// with `gap` between steps at line height `h`.
    fn measure_widths(&self, widths: impl Iterator<Item = f32>, h: f32, gap: f32) -> f32 {
        let mut s = 0.0_f32;
        let mut end = 0.0_f32;
        for width in widths {
            end = s + self.step(s, width, h).0;
            s = end + gap;
        }
        end
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
            let (step, dir) = self.step(s, cluster.advance, h);
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
            let corner = self.corner(s, step, dir, cluster.advance, h);
            out.steps.push(Stepped {
                first: out.glyphs.len(),
                end: out.glyphs.len() + cluster.glyphs.len(),
                s,
                step,
                dir,
                advance: cluster.advance,
                corner,
                reading: cluster.reading,
            });
            for index in cluster.glyphs.clone() {
                out.glyphs.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: corner + Vec2::new(glyphs[index].x - cluster.left, 0.0),
                });
            }
            s = end + gap;
        }
        Laid::Fits(s)
    }

    /// The readable rule (#291, ruling R25), phase 2: lay the clusters
    /// step 1 laid in **reading order** — every leftward and upward
    /// stretch's clusters reversed — so every stretch reads left to right
    /// or top to bottom.
    ///
    /// Step 1, the fit mode's walk in `walked`, decided which clusters
    /// are placed and which stretch each belongs to ([`travels`]
    /// (Self::travels)); that membership is frozen here. The clusters
    /// are then laid forward along the whole path from where step 1
    /// began, stretch after stretch, each taking the step its new place
    /// asks. A reversed stretch need not take the length it took forward
    /// — a wide glyph moved from a vertical part, where it stepped the
    /// line height, onto a level one steps its advance — so one uniform
    /// spacing is solved for the whole placement:
    ///
    /// - [`Ends::Exactly`] (`Spread`, `Fit`): the readable walk ends
    ///   where step 1 ended;
    /// - [`Ends::Within`] (`Once`, `Repeat`): the walk keeps step 1's
    ///   spacing unless that would pass the path's end, and then takes
    ///   the spacing that ends it there.
    ///
    /// Spacing never goes below a zero gap: glyphs never overlap. Where
    /// even a zero gap passes the path's end, the placement is cut, the
    /// way `Fit` cuts text too long at `min` (§6.4 step 4): whole filler
    /// copies, or repeat clusters, from the end first, and only then the
    /// text, its last-read cluster first (ruling R9.4). That is the one
    /// case in which the readable glyphs are not step 1's: they are
    /// step 1's less the clusters cut at the end.
    ///
    /// `laying.glyphs` keeps step 1's order — the text's — so a glyph's
    /// index means what it meant; only positions move (and a cut removes
    /// clusters).
    ///
    /// Cost: O(clusters) to classify and once per readable walk; the
    /// spacing solve takes at most `2 · SOLVE_ROUNDS` walks, a cut a
    /// logarithmic number more. A few `Vec`s of one entry per cluster;
    /// nothing at all when no stretch is placed reversed.
    fn readable(&self, walked: Walked, mut trace: Option<&mut Trace>) -> (Option<ShapedRun>, PlacedRun) {
        let Walked {
            run,
            laying,
            mut truncated,
            gap,
            ends,
            text,
            copy,
            slack,
        } = walked;
        let steps = &laying.steps;
        let stretches = stretches_of(&self.travels(steps, self.line_height));
        if let Some(trace) = trace.as_deref_mut() {
            trace.walked = laying.glyphs.clone();
            trace.chords = steps.iter().map(|step| step.dir).collect();
            trace.stretches = stretches
                .iter()
                .map(|(travel, range)| {
                    let (head, tail) = (steps[range.start], steps[range.end - 1]);
                    TracedStretch {
                        travel: *travel,
                        glyphs: head.first..tail.end,
                        from: head.s,
                        to: tail.s + tail.step,
                    }
                })
                .collect();
        }
        if !stretches.iter().any(is_reversed) {
            let placed = PlacedRun {
                glyphs: laying.glyphs,
                truncated,
            };
            return (run, placed);
        }
        let order = reading_order(&stretches);
        let start = steps[0].s;
        let walked_end = steps[steps.len() - 1].s + steps[steps.len() - 1].step;
        let limit = (self.limit + slack).max(walked_end);
        let mut kept = vec![true; steps.len()];
        // Where the last kept cluster's step ends, laid in reading order
        // from `start` with `gap` between steps.
        let measure = |gap: f32, kept: &[bool]| {
            let (mut s, mut end) = (start, start);
            for &index in order.iter().filter(|&&index| kept[index]) {
                end = s + self.step(s, steps[index].advance, self.line_height).0;
                s = end + gap;
            }
            end
        };
        let gap = match ends {
            Ends::Exactly if measure(0.0, &kept) <= walked_end => {
                solve_gap(|gap| measure(gap, &kept) - walked_end, walked_end, gap)
            }
            Ends::Within if measure(gap, &kept) <= limit => gap,
            Ends::Within if measure(0.0, &kept) <= self.path.length() => solve_gap(
                |gap| measure(gap, &kept) - self.path.length(),
                self.path.length(),
                gap,
            ),
            _ => 0.0,
        };
        if measure(gap, &kept) > limit {
            truncated |= self.cut(steps, text, copy, &mut kept, &measure, limit);
        }
        self.lay_readable(laying, &order, &kept, start, gap, run, truncated)
    }

    /// Each step's [`Travel`] (#291 step 2).
    ///
    /// A step is rightward when `cos θ > ε`, leftward when `cos θ < −ε`
    /// ([`VERTICAL_EPSILON`]); otherwise it is near vertical. A run of
    /// near-vertical steps is a vertical stretch — downward or upward by
    /// where it ends against where it starts — only when it is clearly
    /// entered: it spans [`VERTICAL_MIN_LINES`] line heights, or it is
    /// every step there is. A shorter run is the hysteresis's case, a
    /// curve turning through vertical: it makes no stretch of its own,
    /// and each of its steps joins the horizontal stretch whose way it
    /// moves (`cos θ > 0` rightward, `< 0` leftward), so the boundary
    /// between the stretches before and after it sits where the turn
    /// passes plumb. A step with no horizontal motion at all joins the
    /// stretch that reads it top to bottom: rightward going down,
    /// leftward going up. So a rightward stretch never moves left and a
    /// leftward one never right, and on a loop with no straight side the
    /// sides join the top and the bottom.
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

    /// Where step 1's `steps`, laid again in reading order with no gap,
    /// end, and how far phase 2 lets them go: the end of step 1's walk,
    /// or the path's limit if that is further ([`readable`]
    /// (Self::readable)). The first past the second is a placement phase
    /// 2 must cut.
    ///
    /// Cost: one classification and one walk; nothing allocated beyond
    /// them when no stretch is reversed.
    fn readable_extent(&self, steps: &[Stepped], slack: f32) -> (f32, f32) {
        let walked_end = steps.last().map_or(0.0, |step| step.s + step.step);
        let limit = (self.limit + slack).max(walked_end);
        let stretches = stretches_of(&self.travels(steps, self.line_height));
        if !stretches.iter().any(is_reversed) {
            return (walked_end, limit);
        }
        let widths = reading_order(&stretches)
            .into_iter()
            .map(|index| steps[index].advance);
        (self.measure_widths(widths, self.line_height, 0.0), limit)
    }

    /// Cut the readable placement until, at a zero gap, it ends within
    /// `limit` (ruling R25): drop whole `copy`-cluster units of the
    /// clusters after the first `text` — filler copies, or repeat
    /// clusters — from the end first, then the text's clusters, last
    /// read first (ruling R9.4). Marks the dropped clusters in `kept` and
    /// says whether text was cut. `measure` is the readable walk.
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
        measure: &dyn Fn(f32, &[bool]) -> f32,
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
            if measure(0.0, kept) <= limit {
                fits = middle;
            } else {
                over = middle;
            }
        }
        keep_all_but(if fits == 0 { 0 } else { units[fits - 1] }, kept);
        fits > copy_units
    }

    /// Lay `laying`'s clusters in `order`, skipping those not `kept`,
    /// from `start` with `gap` between steps: each cluster's glyphs move
    /// with its box, and a cluster not kept is removed. The glyphs keep
    /// step 1's order.
    #[allow(clippy::too_many_arguments)] // The readable walk's parts.
    fn lay_readable(
        &self,
        mut laying: Laying,
        order: &[usize],
        kept: &[bool],
        start: f32,
        gap: f32,
        run: Option<ShapedRun>,
        truncated: bool,
    ) -> (Option<ShapedRun>, PlacedRun) {
        let h = self.line_height;
        let mut s = start;
        for &index in order.iter().filter(|&&index| kept[index]) {
            let step = laying.steps[index];
            let (length, dir) = self.step(s, step.advance, h);
            let moved = self.corner(s, length, dir, step.advance, h) - step.corner;
            for glyph in &mut laying.glyphs[step.first..step.end] {
                glyph.pos += moved;
            }
            s += length + gap;
        }
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

/// The stretches of a placement (#291): each maximal run of steps with
/// one [`Travel`], as a range of step indices.
///
/// Cost: O(steps); one entry per stretch.
fn stretches_of(travels: &[Travel]) -> Vec<(Travel, Range<usize>)> {
    let mut stretches: Vec<(Travel, Range<usize>)> = Vec::new();
    for (index, &travel) in travels.iter().enumerate() {
        match stretches.last_mut() {
            Some((last, range)) if *last == travel => range.end = index + 1,
            _ => stretches.push((travel, index..index + 1)),
        }
    }
    stretches
}

/// Whether a stretch is laid reversed and that changes anything: a
/// backward stretch of more than one cluster.
fn is_reversed((travel, range): &(Travel, Range<usize>)) -> bool {
    travel.is_backward() && range.len() > 1
}

/// The step indices in reading order (#291): stretch after stretch, a
/// backward stretch's steps last to first.
///
/// Cost: O(steps); one allocation.
fn reading_order(stretches: &[(Travel, Range<usize>)]) -> Vec<usize> {
    stretches
        .iter()
        .flat_map(|(travel, range)| {
            let (forward, backward) = if travel.is_backward() {
                (0..0, range.clone())
            } else {
                (range.clone(), 0..0)
            };
            forward.chain(backward.rev())
        })
        .collect()
}

/// Place `run` around the closed `path` (a frame ring), from `start`
/// in `direction`, as `fit` says (#239 §5, §6).
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
    let spacing = table_spacing(run, fit, flow);
    let arc = ArcPath::closed(path, start, direction, spacing);
    place(run, &arc, fit, 0.0, outward, flow, fs, trace)
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
    place(run, &arc, fit, spacing, false, flow, fs, trace)
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
    };
    let walked = match fit {
        FitMode::Once { filler } => once(run, spacing, filler.as_deref(), flow, fs, &walk),
        FitMode::Repeat => repeat(run, &walk(run), spacing),
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
    }
}

/// `Repeat`: the text again and again until the path is full, the last
/// copy cut to the longest reading-order prefix that fits. Every copy
/// reuses the run's glyph indices.
fn repeat(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> Walked {
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
    let text = clusters.len().min(glyphs.steps.len());
    Walked {
        run: None,
        laying: glyphs,
        truncated,
        gap,
        ends: Ends::Within,
        text,
        copy: 1,
        slack: 0.0,
    }
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
        return once_text(run, &walk, 0.0).0;
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
    loop {
        let rescaled = (scale != 1.0).then(|| reshape(run, scale, None, fs));
        let placed_run = rescaled.as_ref().unwrap_or(run);
        let slack = if scale > low { SOLVE_TOLERANCE } else { 0.0 };
        let placed_walk = walk_for(placed_run);
        let walked = Walked {
            slack,
            ..spread(placed_run, &placed_walk, slack)
        };
        let (read, limit) = placed_walk.readable_extent(&walked.laying.steps, slack);
        rounds += 1;
        if read <= limit || scale <= low || rounds >= FIT_ROUNDS {
            return Walked {
                run: rescaled,
                ..walked
            };
        }
        // The spacing spread put in moved clusters between stretches, and
        // read in that order the text no longer fits even with no gap:
        // shrink by what it overruns, as `fitting_scale` does.
        scale = (scale * (limit / read) * (1.0 - 1e-3)).max(low);
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
