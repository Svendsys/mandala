// SPDX-License-Identifier: MPL-2.0

//! Placement: upright glyphs positioned along a path (design §6.2, D9),
//! and the fit modes that decide what happens when text and path differ
//! in length (§6.4, D10).
//!
//! **Glyphs are never rotated.** A curve is made by where glyphs sit,
//! the way ASCII art makes one. A glyph of advance `w` and line height
//! `h` placed at distance `s` along the path occupies a step
//! `|cos θ|·w + |sin θ|·h`, θ being the direction of travel half a step
//! on: along a horizontal stretch glyphs sit side by side, along a
//! vertical one they stack a line apart, and on a curve the two blend.
//! The step is a fixed point (θ depends on the step); it is resolved in
//! two rounds, θ first taken at `s` and then re-taken half a step on.
//! The glyph's box, `w × h`, is centered on the path half a step on —
//! on a closed path pushed outward by half its extent along the normal,
//! `|n.x|·w/2 + |n.y|·h/2`, so text sits on the outline rather than
//! across it — and the next glyph starts a step (plus any spacing)
//! later.
//!
//! A run is placed once and re-placed at will: placement reads the
//! run's [`GlyphMetrics`] and never its text, so moving a node re-places
//! its flows without reshaping. Only two modes shape anything, through
//! the font system they are handed: `Fit`, which rescales the run, and
//! `Once` with a filler, which appends the filler; both return the run
//! their placement refers to (ruling R6).

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::{Direction, FitMode, StartPoint};
use crate::flow::curve::Curve;
use crate::flow::outline::Outline;
use crate::flow::path::ArcPath;
use crate::flow::shape::{reshape, GlyphMetrics, ShapedRun, FILLER_SPAN};
use crate::flow::span::{point_size, LINE_HEIGHT_FACTOR};
use crate::font::fonts::FontSystemWriteGuard;
use crate::font::FontSystem;

/// The most glyphs one placement places (ruling R6, reading 9). A
/// repeating flow on a path long enough to need more stops here and
/// reports [`PlacedRun::truncated`].
pub const MAX_PLACED_GLYPHS: usize = 1 << 20;

/// How far inside its end condition a solved spacing lands: `Spread`
/// and `Fit` end their last glyph's step within this of the path's
/// length (and never past it).
const SOLVE_TOLERANCE: f32 = 0.05;

/// The most rounds the spacing and scale solvers take. Each is bounded,
/// so no placement loops unbounded on an interactive path.
const SOLVE_ROUNDS: usize = 48;

/// The most rounds `Fit` takes to shrink its scale until the text fits.
const FIT_ROUNDS: usize = 16;

/// One glyph placed: which glyph of the run, and where its box's
/// top-left corner sits on the canvas.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// An index into the run's [`ShapedRun::glyphs`] (and its
    /// [`ShapedRun::layout_glyphs`]). Repeated text reuses indices.
    pub glyph: u32,
    /// The top-left corner of the glyph's `advance × line_height` box.
    /// The glyph is drawn upright in it, its baseline
    /// [`ShapedRun::baseline`] below this corner.
    pub pos: Vec2,
}

/// A run placed along a path or at a point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacedRun {
    /// The placed glyphs, in the order they were walked.
    pub glyphs: Vec<PlacedGlyph>,
    /// Whether the flow's text could not be placed whole even once: it
    /// was cut at the last grapheme that fits, or placement stopped at
    /// [`MAX_PLACED_GLYPHS`]. A `Repeat` whose last copy is cut, or a
    /// filler that stops short of the end, is the mode working, not a
    /// truncation.
    pub truncated: bool,
}

/// The tolerance a float compare against the path's length allows, so
/// that a glyph ending exactly at the end is not cut by rounding: a
/// thousandth of a point, or a millionth of the path, whichever is
/// larger.
fn end_slack(length: f32) -> f32 {
    (length * 1e-6).max(1e-3)
}

/// The walk: a path, the run's line height and whether glyphs are
/// pushed outside the outline.
struct PathWalk<'p> {
    path: &'p ArcPath,
    /// `h`: every glyph's box height and vertical extent.
    line_height: f32,
    outward: bool,
    /// `P` plus [`end_slack`].
    limit: f32,
}

impl PathWalk<'_> {
    /// The step a glyph of advance `w` (and height `h`) takes at `s`
    /// (#239 §5 step 2): θ at `s`, then re-taken at `s + step/2`.
    fn step(&self, s: f32, w: f32, h: f32) -> f32 {
        let extent = |tangent: Vec2| tangent.x.abs() * w + tangent.y.abs() * h;
        let first = extent(self.path.at(s).1);
        extent(self.path.at(s + first * 0.5).1)
    }

    /// The top-left corner of a `w × h` glyph whose step `step` starts
    /// at `s` (#239 §5 steps 3 and 4).
    fn corner(&self, s: f32, step: f32, w: f32, h: f32) -> Vec2 {
        let (point, tangent) = self.path.at(s + step * 0.5);
        let mut center = point;
        if self.outward {
            let normal = self.path.outward(tangent);
            center += normal * (normal.x.abs() * w * 0.5 + normal.y.abs() * h * 0.5);
        }
        center - Vec2::new(w, h) * 0.5
    }

    /// Where the last glyph's step ends when every glyph of `glyphs`
    /// is walked from 0 with `gap` between steps, at `scale` times its
    /// size, cutting nothing: the natural length `L` at `gap = 0`.
    /// Advances and line heights are linear in size (cosmic-text's
    /// default hinting is off), so `L(scale)` needs no reshape.
    fn measure(&self, glyphs: &[GlyphMetrics], scale: f32, gap: f32) -> f32 {
        let h = self.line_height * scale;
        let mut s = 0.0_f32;
        let mut end = 0.0_f32;
        for glyph in glyphs {
            end = s + self.step(s, glyph.advance * scale, h);
            s = end + gap;
        }
        end
    }

    /// Place `glyphs[range]` (indices into `glyphs`) from `s` with `gap`
    /// between steps, onto `out`, while each step ends within the path.
    ///
    /// Returns where the glyph after the last one would start, or `None`
    /// when a glyph did not fit: then every glyph of its grapheme
    /// cluster placed so far is taken back off `out`, so the cut keeps
    /// whole graphemes. Also `None` at [`MAX_PLACED_GLYPHS`].
    fn lay(
        &self,
        glyphs: &[GlyphMetrics],
        range: std::ops::Range<usize>,
        mut s: f32,
        gap: f32,
        out: &mut Vec<PlacedGlyph>,
    ) -> Option<f32> {
        let mut cluster_start = out.len();
        for index in range {
            let glyph = &glyphs[index];
            if glyph.grapheme_start {
                cluster_start = out.len();
            }
            let step = self.step(s, glyph.advance, self.line_height);
            if s + step > self.limit || out.len() >= MAX_PLACED_GLYPHS {
                out.truncate(cluster_start);
                return None;
            }
            out.push(PlacedGlyph {
                glyph: index as u32,
                pos: self.corner(s, step, glyph.advance, self.line_height),
            });
            s += step + gap;
        }
        Some(s)
    }
}

/// Place `run` around the closed `path` (a frame ring), from `start`
/// in `direction`, as `fit` says (#239 §5, §6).
///
/// With `outward`, each glyph is pushed off the outline by half its
/// extent along the outward normal, so the ring sits outside the
/// outline; without, glyph centers sit on it. A ring placed around
/// `offset(outline, ring_offset(...))` clears the rings inside it.
/// "Fits" means the last glyph's step ends at or before the path's
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
/// a table for an ellipse); O(glyphs placed) steps, each two path
/// lookups; `Spread` and `Fit` measure the run a bounded number of times
/// more to solve their end condition; `Fit` and a filler shape once.
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
/// from its source end, as `fit` says, with `spacing` between glyphs
/// (#239 §5, §6).
///
/// Glyph centers sit on the curve. `spacing` is the gap between steps
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
/// Glyphs sit side by side at their advances, all on one baseline;
/// nothing is cut.
///
/// Cost: O(glyphs); one allocation for the placement.
pub fn place_at_point(run: &ShapedRun, center: Vec2) -> PlacedRun {
    let width: f32 = run.glyphs.iter().map(|g| g.advance).sum();
    let mut corner = center - Vec2::new(width, run.line_height) * 0.5;
    let glyphs = run
        .glyphs
        .iter()
        .enumerate()
        .take(MAX_PLACED_GLYPHS)
        .map(|(index, glyph)| {
            let placed = PlacedGlyph {
                glyph: index as u32,
                pos: corner,
            };
            corner.x += glyph.advance;
            placed
        })
        .collect();
    PlacedRun {
        glyphs,
        truncated: run.glyphs.len() > MAX_PLACED_GLYPHS,
    }
}

/// The sample spacing for a path's arc-length table: a quarter of the
/// smallest line height placement may walk with (#239 §5 step 1) — the
/// run's, shrunk by `Fit`'s smallest scale, or a `Once` filler's.
fn table_spacing(run: &ShapedRun, fit: &FitMode, flow: &ResolvedText) -> f32 {
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
    0.25 * line_height
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
        FitMode::Once { filler } => once(run, &walk(run), spacing, filler.as_deref(), flow, fs, &walk),
        FitMode::Repeat => (None, repeat(run, &walk(run), spacing)),
        FitMode::Spread => (None, spread(run, &walk(run))),
        FitMode::Fit { min, max } => fit_to(run, path, *min, *max, fs, &walk),
    }
}

/// Every glyph once from the start, `gap` apart, cut at the last
/// grapheme that fits.
fn once_text(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> (PlacedRun, Option<f32>) {
    let mut glyphs = Vec::with_capacity(run.glyphs.len());
    let next = walk.lay(&run.glyphs, 0..run.glyphs.len(), 0.0, gap, &mut glyphs);
    (
        PlacedRun {
            glyphs,
            truncated: next.is_none(),
        },
        next,
    )
}

/// `Once`: the text once from the start; then, with a filler and room
/// left, whole copies of the filler until the next would pass the end.
fn once<'p>(
    run: &ShapedRun,
    walk: &PathWalk<'p>,
    gap: f32,
    filler: Option<&str>,
    flow: &ResolvedText,
    fs: &mut FontSystem,
    walk_for: &dyn Fn(&ShapedRun) -> PathWalk<'p>,
) -> (Option<ShapedRun>, PlacedRun) {
    let (placed, next) = once_text(run, walk, gap);
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
    let mut glyphs = Vec::with_capacity(filled.glyphs.len());
    let Some(mut s) = walk.lay(&filled.glyphs, 0..text_glyphs, 0.0, gap, &mut glyphs) else {
        return (None, placed);
    };
    let mut copies = 0usize;
    loop {
        let before = glyphs.len();
        match walk.lay(
            &filled.glyphs,
            text_glyphs..filled.glyphs.len(),
            s,
            gap,
            &mut glyphs,
        ) {
            Some(next) if next > s => {
                copies += 1;
                s = next;
            }
            Some(_) => {
                // A copy that does not advance would repeat forever.
                copies += 1;
                break;
            }
            None => {
                glyphs.truncate(before);
                break;
            }
        }
    }
    if copies == 0 {
        return (None, placed);
    }
    (
        Some(filled),
        PlacedRun {
            glyphs,
            truncated: false,
        },
    )
}

/// `Repeat`: the text again and again until the path is full, the last
/// copy cut at the last grapheme that fits. Every copy reuses the run's
/// glyph indices.
fn repeat(run: &ShapedRun, walk: &PathWalk<'_>, gap: f32) -> PlacedRun {
    let mut glyphs = Vec::new();
    let mut s = 0.0_f32;
    let mut copies = 0usize;
    while let Some(next) = walk.lay(&run.glyphs, 0..run.glyphs.len(), s, gap, &mut glyphs) {
        copies += 1;
        if next <= s {
            // A copy that does not advance would repeat forever.
            break;
        }
        s = next;
    }
    let capped = glyphs.len() >= MAX_PLACED_GLYPHS;
    PlacedRun {
        glyphs,
        truncated: copies == 0 || capped,
    }
}

/// `Spread`: the text once, with the equal extra spacing that ends its
/// last glyph's step at the path's end (ruling R6). The spacing starts
/// from `(P − L)/(n − 1)` — exact on a path of constant direction — and
/// is solved numerically from there, because spacing moves glyphs onto
/// stretches where their steps differ. Too long for the path even with
/// no spacing, the text is cut at the last grapheme that fits. One
/// glyph takes no spacing.
fn spread(run: &ShapedRun, walk: &PathWalk<'_>) -> PlacedRun {
    let count = run.glyphs.len();
    let length = walk.path.length();
    if count <= 1 {
        return once_text(run, walk, 0.0).0;
    }
    let natural = walk.measure(&run.glyphs, 1.0, 0.0);
    if natural.is_nan() || natural >= length {
        return once_text(run, walk, 0.0).0;
    }
    let gap = solve_gap(walk, &run.glyphs, length, (length - natural) / (count - 1) as f32);
    once_text(run, walk, gap).0
}

/// The gap `e ≥ 0` at which `glyphs` walked `e` apart end within
/// [`SOLVE_TOLERANCE`] of `length`, never past it, starting from
/// `guess`. Regula falsi (Illinois) on a bracket whose low side always
/// fits; bounded by [`SOLVE_ROUNDS`].
fn solve_gap(walk: &PathWalk<'_>, glyphs: &[GlyphMetrics], length: f32, guess: f32) -> f32 {
    let miss = |gap: f32| walk.measure(glyphs, 1.0, gap) - length;
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
/// (ruling R6). The scale starts at `clamp(P / L, min, max)`; while it is
/// unclamped and the text at that scale is still longer than the path —
/// rescaling moves glyphs onto stretches where their steps differ — it
/// shrinks by `P / L(s)`, measured arithmetically. The run is reshaped
/// once at the final scale, and the rest of the path is spread as extra
/// spacing, never below zero; text still too long at `min` is cut at the
/// last grapheme that fits.
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
    let natural = walk.measure(&run.glyphs, 1.0, 0.0);
    if run.glyphs.is_empty() || natural.is_nan() || natural <= 0.0 {
        return (None, once_text(run, &walk, 0.0).0);
    }
    let (low, high) = scale_window(min, max);
    let ideal = length / natural;
    let mut scale = if ideal.is_nan() {
        1.0
    } else {
        ideal.clamp(low, high)
    };
    let mut clamped = !(low..=high).contains(&ideal);
    for _ in 0..FIT_ROUNDS {
        if clamped {
            break;
        }
        let at_scale = walk.measure(&run.glyphs, scale, 0.0);
        if at_scale <= length {
            break;
        }
        // A hair under the ratio, so the shrink converges from above
        // instead of settling a rounding error past the end.
        let next = scale * (length / at_scale) * (1.0 - 1e-6);
        if next <= low {
            scale = low;
            clamped = true;
        } else {
            scale = next;
        }
    }
    let rescaled = (scale != 1.0).then(|| reshape(run, scale, None, fs));
    let placed_run = rescaled.as_ref().unwrap_or(run);
    let placed = spread(placed_run, &walk_for(placed_run));
    (rescaled, placed)
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
