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
    /// The placed glyphs: the clusters in the order they were walked,
    /// each cluster's glyphs in visual order.
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
    /// Cost: O(glyphs), no allocation.
    fn measure_at_scale(
        &self,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        scale: f32,
        largest: f32,
    ) -> f32 {
        self.measure_widths(
            clusters.iter().map(|cluster| {
                glyphs[cluster.glyphs.clone()]
                    .iter()
                    .map(|glyph| glyph.advance * size_factor(glyph.size, scale))
                    .sum::<f32>()
            }),
            self.line_height * size_factor(largest, scale),
            0.0,
        )
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
        out: &mut Vec<PlacedGlyph>,
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
            if out.len() + cluster.glyphs.len() > MAX_PLACED_GLYPHS {
                return Laid::Capped;
            }
            let corner = self.corner(s, step, dir, cluster.advance, h);
            for index in cluster.glyphs.clone() {
                out.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: corner + Vec2::new(glyphs[index].x - cluster.left, 0.0),
                });
            }
            s = end + gap;
        }
        Laid::Fits(s)
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
        out: &mut Vec<PlacedGlyph>,
    ) -> (Laid, bool) {
        let mark = out.len();
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
        let lay_prefix = |keep: usize, out: &mut Vec<PlacedGlyph>| {
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
    let spacing = table_spacing(run, fit, flow);
    let arc = ArcPath::closed(path, start, direction, spacing);
    place(run, &arc, fit, 0.0, outward, flow, fs)
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
    let table = table_spacing(run, fit, flow);
    let arc = ArcPath::open(curve, table);
    let spacing = if spacing.is_finite() {
        spacing.max(0.0)
    } else {
        0.0
    };
    place(run, &arc, fit, spacing, false, flow, fs)
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

/// The body both placements share, over a walked path.
fn place(
    run: &ShapedRun,
    path: &ArcPath,
    fit: &FitMode,
    spacing: f32,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystem,
) -> (Option<ShapedRun>, PlacedRun) {
    let walk = |run: &ShapedRun| PathWalk {
        path,
        line_height: run.line_height,
        outward,
        limit: path.length() + end_slack(path.length()),
    };
    match fit {
        FitMode::Once { filler } => once(run, spacing, filler.as_deref(), flow, fs, &walk),
        FitMode::Repeat => (None, repeat(run, &walk(run), spacing)),
        FitMode::Spread => (None, spread(run, &walk(run), 0.0)),
        FitMode::Fit { min, max } => fit_to(run, path, *min, *max, fs, &walk),
    }
}

/// Every cluster once from the start, `gap` apart, cut to the longest
/// reading-order prefix that fits. Also returns where the next cluster
/// would start, when the text was placed whole.
fn once_text(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> (PlacedRun, Option<f32>) {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let mut glyphs = Vec::with_capacity(run.glyphs.len());
    let (laid, cut) = walk.lay_or_cut(&run.glyphs, &clusters, 0.0, gap, &mut glyphs);
    let next = match laid {
        Laid::Fits(next) if !cut => Some(next),
        _ => None,
    };
    let truncated = cut || laid == Laid::Capped;
    (PlacedRun { glyphs, truncated }, next)
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
) -> (Option<ShapedRun>, PlacedRun) {
    let (placed, next) = once_text(run, &walk_for(run), gap);
    let (Some(filler), Some(_)) = (filler, next) else {
        return (None, placed);
    };
    let filled = reshape(run, 1.0, Some((filler, flow)), fs);
    let text_glyphs = filled.glyphs.iter().take_while(|g| g.span != FILLER_SPAN).count();
    if text_glyphs == filled.glyphs.len() {
        return (None, placed);
    }
    // The filled run may be taller (a larger filler), which moves the
    // text; place everything again with its line height.
    let walk = walk_for(&filled);
    let text = clusters_of(&filled.glyphs, 0..text_glyphs);
    let copy = clusters_of(&filled.glyphs, text_glyphs..filled.glyphs.len());
    let mut glyphs = Vec::with_capacity(filled.glyphs.len());
    let Laid::Fits(mut s) = walk.lay(&filled.glyphs, &text, 0.0, gap, &mut glyphs) else {
        return (None, placed);
    };
    let mut copies = 0usize;
    let mut capped = false;
    loop {
        let before = glyphs.len();
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
        return (None, placed);
    }
    let truncated = capped;
    (Some(filled), PlacedRun { glyphs, truncated })
}

/// `Repeat`: the text again and again until the path is full, the last
/// copy cut to the longest reading-order prefix that fits. Every copy
/// reuses the run's glyph indices.
fn repeat(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> PlacedRun {
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let mut glyphs = Vec::new();
    let mut s = 0.0_f32;
    let mut copies = 0usize;
    let truncated = loop {
        let before = glyphs.len();
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
    PlacedRun { glyphs, truncated }
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
fn spread(run: &ShapedRun, walk: &PathWalk<'_>, overrun_slack: f32) -> PlacedRun {
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
    let gap = solve_gap(walk, &clusters, length, guess);
    once_text(run, walk, gap).0
}

/// The gap `e ≥ 0` at which `clusters` walked `e` apart end within
/// [`SOLVE_TOLERANCE`] of `length`, never past it, starting from
/// `guess`. Regula falsi (Illinois) on a bracket whose low side always
/// fits; bounded by [`SOLVE_ROUNDS`]. The end moves with `e` without the
/// jumps a tangent-taken step has at a sharp corner ([`PathWalk::step`]),
/// so the bracket closes on the end condition rather than on a jump.
fn solve_gap(walk: &PathWalk<'_>, clusters: &[Cluster], length: f32, guess: f32) -> f32 {
    let miss = |gap: f32| walk.measure(clusters, 1.0, gap) - length;
    let tolerance = SOLVE_TOLERANCE.max(length * 1e-6);
    let (mut low, mut low_miss) = (0.0_f32, miss(0.0));
    // Find a gap that overshoots, doubling the guess.
    let mut high = None;
    let mut probe = guess.max(f32::EPSILON);
    for _ in 0..SOLVE_ROUNDS {
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
/// bisection between `min` and the last scale that overran. The run is
/// reshaped once, at the final scale, and the rest of the path is spread
/// as extra spacing, never below zero. Only text still too long at `min`
/// is cut (design §6.4 step 4); an overrun no larger than the solver's
/// tolerance, the float noise of the reshape, is placed as is.
fn fit_to<'p>(
    run: &ShapedRun,
    path: &'p ArcPath,
    min: f32,
    max: f32,
    fs: &mut FontSystem,
    walk_for: &dyn Fn(&ShapedRun) -> PathWalk<'p>,
) -> (Option<ShapedRun>, PlacedRun) {
    let walk = walk_for(run);
    let length = path.length();
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let natural = walk.measure(&clusters, 1.0, 0.0);
    if clusters.is_empty() || natural.is_nan() || natural <= 0.0 {
        return (None, once_text(run, &walk, 0.0).0);
    }
    let (low, high) = scale_window(min, max);
    let ideal = length / natural;
    let start = if ideal.is_nan() {
        1.0
    } else {
        ideal.clamp(low, high)
    };
    let scale = fitting_scale(&walk, &run.glyphs, &clusters, length, start, low);
    let rescaled = (scale != 1.0).then(|| reshape(run, scale, None, fs));
    let placed_run = rescaled.as_ref().unwrap_or(run);
    let slack = if scale > low { SOLVE_TOLERANCE } else { 0.0 };
    let placed = spread(placed_run, &walk_for(placed_run), slack);
    (rescaled, placed)
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
