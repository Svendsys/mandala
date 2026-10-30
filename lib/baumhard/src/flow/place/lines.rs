// SPDX-License-Identifier: MPL-2.0

//! A text frame in two lines (#291, ruling R27): the owner asked for it
//! "formatted smarter like, defining lines for each halve, rather than
//! one line that must be fitted to both", and approved "Sides clear,
//! auto split".
//!
//! A frame ring whose fit mode is `Once`, `Spread` or `Fit` holds text,
//! not a pattern. Its outline is split at its leftmost and rightmost
//! points — at mid-height of a rounded rectangle's sides — into a **top
//! half**, walked left to right over the top, and a **bottom half**,
//! walked left to right along the bottom ([`frame_halves`]). Each
//! half's **span** is the part of it whose direction is level-dominant,
//! `|cos θ| ≥ √½` (the readable rule's ε); the steep sides stay clear,
//! so glyphs never stack there.
//!
//! The text is parted into a top line and a bottom line at a word break
//! ([`GlyphMetrics::word_break`]; punctuation stays on its word): at the
//! first line break the text holds, if it holds one, and otherwise where
//! the two lines best balance against their spans — the larger of the
//! two fill ratios as small as it can be. A text with no word break is
//! one line, on top. Each line is centered on its span and reads left to
//! right; both share one size.
//!
//! - `Fit`: the largest scale in `[min, max]` at which each line fits
//!   its span with no gap. A line too long even at `min` is cut at its
//!   end, at a grapheme boundary (§6.4 step 4).
//! - `Spread`: each line spread over its whole span.
//! - `Once`: the lines at their declared size, centered; a filler fills
//!   the rest of each span, on both sides of each line, in whole copies.
//!
//! `start` and `direction` do not apply: a text frame always reads over
//! the top and then along the bottom.

use super::*;
use crate::document::values::Edge;

/// The halves of a closed outline (ruling R27): the top walked left to
/// right over the top, then the bottom walked left to right along the
/// bottom, each from the outline's leftmost point — at mid-height of a
/// rounded rectangle's side — to its rightmost, with its span: from
/// where its direction first turns level-dominant (`|cos θ| ≥ √½`) to
/// where it last is. Tables are sampled no coarser than `spacing`.
///
/// Cost: two arc-length paths ([`ArcPath::closed`]); a bisection of each
/// span's ends, a bounded number of path lookups.
pub(crate) fn frame_halves(outline: &Outline, spacing: f32) -> [(ArcPath, Range<f32>); 2] {
    let left = StartPoint {
        edge: Edge::Left,
        at: 0.5,
    };
    [Direction::Clockwise, Direction::CounterClockwise].map(|direction| {
        let path = ArcPath::closed(outline, left, direction, spacing);
        let span = level_span(&path);
        (path, span)
    })
}

/// Where along the half of `path` that runs from its start over to the
/// far side the direction is level-dominant: bisected, from its middle
/// — the top or the bottom of an outline symmetric about its axes —
/// outward to where the direction turns steep.
fn level_span(path: &ArcPath) -> Range<f32> {
    let half = path.length() * 0.5;
    let middle = half * 0.5;
    let level = |s: f32| path.at(s).1.x.abs() >= VERTICAL_EPSILON;
    if !level(middle) {
        return middle..middle;
    }
    // The edge between `steep`, not level, and `flat`, level.
    let edge = |mut steep: f32, mut flat: f32| {
        if level(steep) {
            return steep;
        }
        for _ in 0..SOLVE_ROUNDS {
            let mid = 0.5 * (steep + flat);
            if level(mid) {
                flat = mid;
            } else {
                steep = mid;
            }
        }
        flat
    };
    edge(0.0, middle)..edge(half, middle)
}

/// Which clusters each line holds: ranges of cluster indices, the parting
/// whitespace and each line's outer whitespace left out.
fn part(clusters: &[Cluster], spans: [f32; 2]) -> [Range<usize>; 2] {
    let n = clusters.len();
    let trim = |mut range: Range<usize>| {
        while range.start < range.end && clusters[range.start].blank {
            range.start += 1;
        }
        while range.end > range.start && clusters[range.end - 1].blank {
            range.end -= 1;
        }
        range
    };
    if let Some(at) = clusters.iter().position(|cluster| cluster.line_break) {
        return [trim(0..at), trim(at + 1..n)];
    }
    // Widths by prefix sums: the balance is judged on advances, which a
    // level-dominant span steps by, near enough.
    let mut before = Vec::with_capacity(n + 1);
    before.push(0.0_f32);
    for cluster in clusters {
        before.push(before[before.len() - 1] + cluster.advance);
    }
    let width = |range: &Range<usize>| before[range.end] - before[range.start];
    let ratio = |width: f32, span: f32| if span > 0.0 { width / span } else { f32::INFINITY };
    let mut best: Option<(f32, [Range<usize>; 2])> = None;
    for at in 1..n {
        if !clusters[at].word_break {
            continue;
        }
        let (top, bottom) = (trim(0..at), trim(at..n));
        if top.is_empty() || bottom.is_empty() {
            continue;
        }
        let worst = ratio(width(&top), spans[0]).max(ratio(width(&bottom), spans[1]));
        if best.as_ref().is_none_or(|(least, _)| worst < *least) {
            best = Some((worst, [top, bottom]));
        }
    }
    best.map_or_else(|| [trim(0..n), n..n], |(_, lines)| lines)
}

/// What a frame's lines lay: their run and clusters, and how.
struct Lines<'a> {
    run: &'a ShapedRun,
    clusters: &'a [Cluster],
    lines: [Range<usize>; 2],
    halves: &'a [(ArcPath, Range<f32>); 2],
    outward: bool,
}

impl Lines<'_> {
    /// The walk along half `k`, ending at its span's end.
    fn walk(&self, k: usize) -> PathWalk<'_> {
        let (path, span) = &self.halves[k];
        PathWalk {
            path,
            line_height: self.run.line_height,
            outward: self.outward,
            limit: span.end + end_slack(span.end),
        }
    }

    /// Where line `k`'s last step ends laid from `s` with `gap`.
    fn reach(&self, k: usize, s: f32, gap: f32) -> f32 {
        let walk = self.walk(k);
        let mut at = s;
        let mut end = s;
        for cluster in &self.clusters[self.lines[k].clone()] {
            end = at + walk.step(at, cluster.advance, self.run.line_height).0;
            at = end + gap;
        }
        end
    }

    /// Lay line `k` onto `out`, its glyphs in text order (ruling R27):
    /// spread over its span, or centered on it with `gap`; a line too
    /// long for its span is laid from the span's start and cut at its
    /// end, at a grapheme boundary. Returns where it starts and ends,
    /// and whether it was cut.
    fn lay(&self, k: usize, spread: bool, gap: f32, out: &mut Laying) -> (f32, f32, bool) {
        let span = self.halves[k].1.clone();
        let line = &self.clusters[self.lines[k].clone()];
        if line.is_empty() {
            let middle = 0.5 * (span.start + span.end);
            return (middle, middle, false);
        }
        let walk = self.walk(k);
        let natural = self.reach(k, span.start, 0.0);
        if natural > walk.limit {
            let (laid, cut) = walk.lay_or_cut(&self.run.glyphs, line, span.start, 0.0, out);
            let end = match laid {
                Laid::Fits(next) => next,
                _ => span.end,
            };
            return (span.start, end, cut || laid == Laid::Capped);
        }
        let (start, gap) = if spread && line.len() > 1 {
            let guess = (span.end - natural) / (line.len() - 1) as f32;
            let gap = solve_gap(|g| self.reach(k, span.start, g) - span.end, span.end, guess);
            (span.start, gap)
        } else {
            (self.center(k, gap), gap)
        };
        let laid = walk.lay(&self.run.glyphs, line, start, gap, out);
        let cut = laid == Laid::Capped;
        (start, self.reach(k, start, gap), cut)
    }

    /// Where line `k` starts laid with `gap` so that it is centered on
    /// its span's arc-length midpoint: as much of the span left before
    /// it as after it. A step's length depends on where along the half
    /// it is taken ([`PathWalk::step`]), so the start is bisected rather
    /// than worked from the line's length at one place; a line as long
    /// as its span starts at the span's start.
    ///
    /// Cost: a bounded bisection, [`SOLVE_ROUNDS`] lays of the line.
    fn center(&self, k: usize, gap: f32) -> f32 {
        let span = self.halves[k].1.clone();
        // Positive once the line lies past the middle.
        let miss = |start: f32| (start - span.start) - (span.end - self.reach(k, start, gap));
        if miss(span.start) >= 0.0 {
            return span.start;
        }
        let (mut before, mut past) = (span.start, span.end);
        for _ in 0..SOLVE_ROUNDS {
            if past - before <= end_slack(span.end) {
                break;
            }
            let middle = 0.5 * (before + past);
            if miss(middle) < 0.0 {
                before = middle;
            } else {
                past = middle;
            }
        }
        before
    }

    /// Whether line `k` fits its span laid from the span's start with no
    /// gap.
    fn fits(&self, k: usize) -> bool {
        self.reach(k, self.halves[k].1.start, 0.0) <= self.walk(k).limit
    }

    /// The lines parted again for text too long even at `Fit`'s `min`
    /// (ruling R27): the top line takes the most words that fit its span,
    /// so only the bottom line's end is cut. Text with an explicit line
    /// break keeps its parting, and so does text whose first word alone
    /// overruns the top.
    ///
    /// Cost: one lay of the text along the top half, O(clusters).
    fn fill_top(&self) -> [Range<usize>; 2] {
        let clusters = self.clusters;
        let n = clusters.len();
        if clusters.iter().any(|cluster| cluster.line_break) {
            return self.lines.clone();
        }
        let Some(first) = clusters.iter().position(|cluster| !cluster.blank) else {
            return self.lines.clone();
        };
        let (path_span, walk) = (&self.halves[0].1, self.walk(0));
        // Where each cluster's step ends, the text laid from `first`.
        let mut ends = vec![f32::INFINITY; n];
        let mut at = path_span.start;
        for (i, cluster) in clusters.iter().enumerate().skip(first) {
            at += walk.step(at, cluster.advance, self.run.line_height).0;
            ends[i] = at;
            if at > walk.limit {
                break;
            }
        }
        // The last cluster a prefix ending before `at` keeps: its trailing
        // whitespace left out.
        let last_kept = |at: usize| (first..at).rev().find(|&i| !clusters[i].blank);
        let mut best = None;
        for at in (first + 1..=n).filter(|&at| at == n || clusters[at].word_break) {
            match last_kept(at) {
                Some(last) if ends[last] <= walk.limit => best = Some(at),
                _ => {}
            }
        }
        match best {
            Some(at) => {
                let mut bottom = at..n;
                while bottom.start < bottom.end && clusters[bottom.start].blank {
                    bottom.start += 1;
                }
                let top_end = last_kept(at).map_or(first, |last| last + 1);
                [first..top_end, bottom]
            }
            None => self.lines.clone(),
        }
    }

    /// Whether every line fits its span with no gap at `scale` times its
    /// size, as a reshape at `scale` will lay it ([`size_factor`]).
    fn fit_at(&self, scale: f32, largest: f32) -> bool {
        let h = self.run.line_height * size_factor(largest, scale);
        (0..2).all(|k| {
            let span = &self.halves[k].1;
            let walk = self.walk(k);
            let mut at = span.start;
            for cluster in &self.clusters[self.lines[k].clone()] {
                let width: f32 = self.run.glyphs[cluster.glyphs.clone()]
                    .iter()
                    .map(|glyph| glyph.advance * size_factor(glyph.size, scale))
                    .sum();
                at += walk.step(at, width, h).0;
            }
            at <= walk.limit
        })
    }
}

/// Place `run` as a text frame around `outline` (ruling R27): two lines,
/// over the top and along the bottom, as `fit` says (`Once`, `Spread` or
/// `Fit`). The placement lists the top line's glyphs, then the bottom's,
/// in text order — the whitespace they part at and end with left out —
/// then a `Once` filler's. Returns the run the placement refers to when
/// it is not `run`, as [`place_on_closed`] does.
///
/// Cost: the two halves ([`frame_halves`]); the parting, O(clusters);
/// `Fit`'s scale, a bounded bisection of arithmetic lays and one reshape;
/// each line laid once, its centering or spreading a bounded number of
/// lays more; a filler one reshape and a lay per copy.
pub(super) fn place_lines(
    run: &ShapedRun,
    outline: &Outline,
    fit: &FitMode,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let halves = frame_halves(outline, table_spacing(run, fit, flow));
    let spans = [0, 1].map(|k| halves[k].1.end - halves[k].1.start);
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let lines = part(&clusters, spans);
    let shaped = clusters.len();
    let parted = Lines {
        run,
        clusters: &clusters,
        lines: lines.clone(),
        halves: &halves,
        outward,
    };
    let (placed_run, filler) = match fit {
        FitMode::Fit { min, max } => {
            let (low, high) = scale_window(*min, *max);
            let largest = run.glyphs.iter().map(|glyph| glyph.size).fold(0.0_f32, f32::max);
            let scale = if parted.fit_at(high, largest) {
                high
            } else if !parted.fit_at(low, largest) {
                low
            } else {
                let (mut under, mut over) = (low, high);
                for _ in 0..SOLVE_ROUNDS {
                    if over - under <= under * 1e-6 {
                        break;
                    }
                    let middle = 0.5 * (under + over);
                    if parted.fit_at(middle, largest) {
                        under = middle;
                    } else {
                        over = middle;
                    }
                }
                under
            };
            (fit_reshape(run, &lines, &halves, outward, scale, low, fs), None)
        }
        FitMode::Once { filler: Some(filler) } => {
            (Some(reshape(run, 1.0, Some((filler, flow)), fs)), Some(()))
        }
        _ => (None, None),
    };
    let laid_run = placed_run.as_ref().unwrap_or(run);
    let clusters = if placed_run.is_some() {
        clusters_of(&laid_run.glyphs, 0..laid_run.glyphs.len())
    } else {
        clusters
    };
    let text_clusters = clusters
        .iter()
        .take_while(|c| laid_run.glyphs[c.glyphs.start].span != FILLER_SPAN)
        .count();
    // A reshape that shaped the text into other clusters is parted anew.
    let lines = if text_clusters == shaped {
        lines
    } else {
        part(&clusters[..text_clusters], spans)
    };
    let mut laid = Lines {
        run: laid_run,
        clusters: &clusters[..text_clusters],
        lines,
        halves: &halves,
        outward,
    };
    if matches!(fit, FitMode::Fit { .. }) && !(laid.fits(0) && laid.fits(1)) {
        laid.lines = laid.fill_top();
    }
    let spread = matches!(fit, FitMode::Spread);
    let mut out = Laying::with_capacity(laid_run.glyphs.len());
    let mut traced = Vec::with_capacity(2);
    let mut extents = [(0.0, 0.0); 2];
    let mut truncated = false;
    let mut laid_on = Vec::new();
    for (k, extent) in extents.iter_mut().enumerate() {
        let first = out.glyphs.len();
        let (from, to, cut) = laid.lay(k, spread, 0.0, &mut out);
        laid_on.resize(out.steps.len(), k);
        truncated |= cut;
        *extent = (from, to);
        traced.push(TracedLine {
            half: k,
            glyphs: first..out.glyphs.len(),
            from,
            to,
            cut,
        });
    }
    let used_filler =
        filler.is_some() && fill(&laid, &clusters[text_clusters..], extents, &mut out, &mut laid_on);
    if let Some(trace) = trace {
        trace.walked = out.glyphs.clone();
        trace.chords = out.steps.iter().map(|step| step.dir).collect();
        trace.stretches.clear();
        trace.walked_stretches.clear();
        trace.lines = traced;
        trace.laid = out.steps.iter().map(|step| (step.s, step.step)).collect();
        trace.laid_on = laid_on;
    }
    let placed = PlacedRun {
        glyphs: out.glyphs,
        truncated,
    };
    if used_filler || (placed_run.is_some() && filler.is_none()) {
        (placed_run, placed)
    } else {
        (None, placed)
    }
}

/// `run` reshaped at `scale`, the size `Fit` settled on, or `None` at
/// scale 1. The scale was found on the run's own advances times each
/// glyph's [`size_factor`]; should the reshape lay a line longer than
/// that predicted, past its span, the scale is shrunk by the overrun and
/// the run reshaped again — at most [`FIT_ROUNDS`] times, never below
/// `low` — so a line is cut only at `min` (ruling R27).
///
/// Cost: one reshape when the prediction holds, and per further round a
/// reshape and a lay of each line.
fn fit_reshape(
    run: &ShapedRun,
    lines: &[Range<usize>; 2],
    halves: &[(ArcPath, Range<f32>); 2],
    outward: bool,
    mut scale: f32,
    low: f32,
    fs: &mut FontSystemWriteGuard,
) -> Option<ShapedRun> {
    let mut reshaped = (scale != 1.0).then(|| reshape(run, scale, None, fs));
    for _ in 0..FIT_ROUNDS {
        let laid_run = reshaped.as_ref().unwrap_or(run);
        let clusters = clusters_of(&laid_run.glyphs, 0..laid_run.glyphs.len());
        let laid = Lines {
            run: laid_run,
            clusters: &clusters,
            lines: lines.clone(),
            halves,
            outward,
        };
        // The longer of the lines' lengths over its span's room.
        let overrun = (0..2)
            .map(|k| {
                let span = &halves[k].1;
                let room = laid.walk(k).limit - span.start;
                (laid.reach(k, span.start, 0.0) - span.start) / room.max(f32::EPSILON)
            })
            .fold(0.0_f32, f32::max);
        if overrun <= 1.0 || scale <= low {
            break;
        }
        scale = (scale / overrun * (1.0 - 1e-4)).max(low);
        reshaped = Some(reshape(run, scale, None, fs));
    }
    reshaped
}

/// `Once`'s filler around a frame's lines: whole copies of `copy` fill
/// the rest of each span, on both sides of each line — after it from its
/// end on, and before it back to the span's start — never touching it.
/// Returns whether any copy was placed; `laid_on` gains the half each
/// copy's steps are on.
///
/// Cost: a lay per copy, and per side a bounded number of measures.
fn fill(
    lines: &Lines<'_>,
    copy: &[Cluster],
    extents: [(f32, f32); 2],
    out: &mut Laying,
    laid_on: &mut Vec<usize>,
) -> bool {
    if copy.is_empty() {
        return false;
    }
    let before = out.glyphs.len();
    let h = lines.run.line_height;
    for (k, (from, to)) in extents.into_iter().enumerate() {
        let span = lines.halves[k].1.clone();
        let walk = lines.walk(k);
        let empty = lines.lines[k].is_empty();
        // How far `count` copies reach laid from `s`.
        let reach = |mut s: f32, count: usize| {
            for _ in 0..count {
                for cluster in copy {
                    s += walk.step(s, cluster.advance, h).0;
                }
            }
            s
        };
        let (after, room_before) = if empty {
            (span.start, span.start)
        } else {
            (to, from)
        };
        // After the line: copies until the next would pass the span.
        let mut s = after;
        loop {
            let mark = out.mark();
            match walk.lay(&lines.run.glyphs, copy, s, 0.0, out) {
                Laid::Fits(next) if next > s => s = next,
                _ => {
                    out.truncate(mark);
                    break;
                }
            }
        }
        laid_on.resize(out.steps.len(), k);
        if empty {
            continue;
        }
        // Before it: as many copies as fit between the span's start and
        // the line, laid so the last ends against the line.
        let mut count = 0;
        while reach(span.start, count + 1) <= room_before {
            count += 1;
        }
        if count > 0 {
            let mut start = room_before - (reach(span.start, count) - span.start);
            for _ in 0..CHORD_ROUNDS {
                start = (room_before - (reach(start, count) - start)).max(span.start);
            }
            if reach(start, count) > room_before + end_slack(room_before) {
                count -= 1;
            }
            let mut s = start;
            for _ in 0..count {
                if let Laid::Fits(next) = walk.lay(&lines.run.glyphs, copy, s, 0.0, out) {
                    s = next;
                }
            }
            laid_on.resize(out.steps.len(), k);
        }
    }
    out.glyphs.len() > before
}
