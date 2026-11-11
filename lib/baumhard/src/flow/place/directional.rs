// SPDX-License-Identifier: MPL-2.0

//! A directional ring (#324): a `repeat` ring that lays one text where
//! its outline runs level and another where it runs steep, with a corner
//! where it turns from one to the other (`format/schema.md`, "Directional
//! ring"). Glyphs are never rotated (D9), so a `─` repeated down a side
//! stacks into a bar; a ring of `─` along its top and bottom and `│` down
//! its sides, with `┌ ┐ ┘ └` at its corners, reads as a box.
//!
//! **Where it turns.** A node's outline is convex — a rectangle, rounded
//! or not, or an ellipse — so its direction turns one way all round it,
//! and crosses 45° from level exactly four times: at its support points
//! in the four diagonal directions ([`Outline::support`]), a sharp
//! rectangle's corners, a rounded one's corner arcs' middles, an
//! ellipse's points where its tangent runs at 45°. Those four points part
//! the ring into four **runs**: the top and the bottom level, laid with
//! the ring's spans, the two sides steep, laid with its `vertical` text.
//! The threshold is the readable rule's ([`VERTICAL_EPSILON`], 45°), and
//! the choice cannot flicker: it changes at those four points and nowhere
//! else. **The hysteresis** is the readable rule's too
//! ([`VERTICAL_MIN_LINES`]): a run shorter than three line heights — the
//! tips of a flat ellipse, the top of a tall one — is no run of its own;
//! its two neighbors take it, parted at its middle, where the outline
//! faces straight out, and no corner is laid at its ends.
//!
//! **Its corners** are set where two runs meet at a turn, clockwise from
//! the top left: top left, top right, bottom right, bottom left. A
//! corner is set as its neighbors are, cell to cell, the way a line of
//! text sets glyphs of one face: on the baseline of the level run's text,
//! and its box centered on the box of the steep run's text — so a
//! box-drawing corner's two strokes run on in its neighbors' strokes, as
//! the face draws them on a grid, and a `◆` sits where the face sets it
//! on a line of `━`.
//!
//! **Each run is laid whole**, as a connector's pattern is between its
//! ends ([`pitch`]): whole glyphs, their inks pushed off the outline as a
//! `repeat` ring's are, every gap between two inks the same, box to box,
//! the gaps at its two ends to the corners beside it included — or, where
//! two runs meet with no corner, to the neighbor run's end ink. The gap
//! is the ring's: its spans' own, as a line sets two of them, its
//! `spacing` added; the vertical text is set that same gap apart, so the
//! steep runs read as evenly as the level ones. Each run holds as many
//! inks as bring that gap nearest its own, never tighter, and spreads
//! what is left over its gaps.
//!
//! **Where it is drawn.** Its path keeps the outline's corners as they
//! are: the projection grows a sharp rectangle by the ring's offset into
//! a sharp rectangle ([`offset_keeping_corners`]), so its corners are
//! where its runs' strokes meet.
//!
//! **What it costs** is counted and held to the ring's budget as a
//! `repeat` ring's is ([`RingWork`], #310): its path's reads, the glyphs
//! it sets and its shaping, scaled by the glyphs its runs will hold. Each
//! run's count and each round of its spacing reads the budget first, and
//! holds back what the run's last lays take ([`LAYS_HELD`]); a ring the
//! budget stops is short ([`RingLoss::Short`]), its runs laid as far as
//! their counts got.

use std::cell::Cell;
use std::ops::Range;

use glam::Vec2;

use super::halves::{inked, inking_steps, resizing_steps, resolved};
use super::pitch::{glyphs_at, natural_marks, pitch, Mark, PathEnd, Track};
use super::ring::{ring_glyphs, steps_of, RingLoss, RingWork};
use super::{
    cluster_ink, clusters_of, support, table_spacing, Cluster, PlacedGlyph, PlacedRun, Trace,
    MAX_PLACED_GLYPHS, VERTICAL_EPSILON, VERTICAL_MIN_LINES,
};
use crate::document::element::DirectionalGlyphs;
use crate::document::resolve::ResolvedText;
use crate::document::values::{Direction, Edge, FitMode, StartPoint};
use crate::flow::outline::{Outline, Rect};
use crate::flow::path::ArcPath;
use crate::flow::shape::{is_paragraph_separator, reshape, Ink, ShapedRun, FILLER_SPAN};
use crate::font::fonts::FontSystemLease;

#[cfg(doc)]
use super::pitch::place_pitched;
#[cfg(doc)]
use crate::flow::outline::offset_keeping_corners;

/// The diagonal directions an outline turns through 45° in, clockwise
/// from its top left: where each corner is set.
const TURNS: [Vec2; 4] = [
    Vec2::new(-1.0, -1.0),
    Vec2::new(1.0, -1.0),
    Vec2::new(1.0, 1.0),
    Vec2::new(-1.0, 1.0),
];

/// The direction each run between two turns faces — the top, the right
/// side, the bottom, the left side — and where a run too short to be one
/// is parted.
const FACES: [Vec2; 4] = [
    Vec2::new(0.0, -1.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(-1.0, 0.0),
];

/// The most path reads a lay of a run takes an ink
/// ([`pitch`]'s `next_at`): its box, its direction,
/// the direction at its first guess, and the guess's gap and its
/// [`REFINE`](super::pitch::REFINE) refinements where the path bends.
const INK_READS: usize = 4 + super::pitch::REFINE;

/// The most path reads a lay of a run takes at its two ends, past its
/// inks: each end's ink solved against what is beside it, its gap and its
/// refinements.
const END_READS: usize = 2 * (1 + super::pitch::REFINE);

/// The lays of a run held back for whenever its count or its spacing
/// asks whether it may go on: the two counts its spacing is solved for
/// each begin with a lay the budget is not asked about, a round of the
/// spacing is one more, and the lay at the spacing found one more.
const LAYS_HELD: usize = 4;

/// The glyph steps a lay of a run of `inks` inks takes at most.
fn lay_steps(inks: usize) -> usize {
    steps_of(inks.saturating_mul(INK_READS).saturating_add(END_READS))
}

/// Which way a run of the ring runs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunKind {
    /// Along the top or the bottom: the ring's spans.
    Level,
    /// Down a side: its `vertical` text, or its spans without one.
    Steep,
}

/// One run of a directional ring: where it lies along its path, from
/// `from` for `length`, which way it runs, and the corner at its start,
/// if one is set there (`0` the top left, clockwise).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub(crate) from: f32,
    pub(crate) length: f32,
    pub(crate) kind: RunKind,
    /// The side it runs along: `0` the top, clockwise.
    pub(crate) side: usize,
    pub(crate) corner: Option<usize>,
}

/// The runs a ring of line height `line_height` round `arc`, the path
/// `outline` was walked into, is parted into, clockwise from its first
/// turn's (see the module docs), corners set at a turn between two runs
/// where `corners` says so. None where fewer than two runs are long
/// enough to be runs: an outline too small to turn on, which is laid as
/// a ring of its spans alone.
///
/// Cost: eight support points located on the path, no path lookup.
pub(crate) fn runs(arc: &ArcPath, outline: &Outline, line_height: f32, corners: bool) -> Option<Vec<Run>> {
    let length = arc.length();
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let locate = |dir: Vec2| {
        let p = outline.support(dir);
        arc.locate(p).unwrap_or_else(|| arc.nearest(p))
    };
    let base = locate(TURNS[0]);
    // Along the path from the first turn, the four turns in order.
    let along = |dir: Vec2| (locate(dir) - base).rem_euclid(length);
    let mut turns = TURNS.map(along);
    turns[0] = 0.0;
    let faces = FACES.map(along);
    let ends = |i: usize| if i == 3 { length } else { turns[i + 1] };
    let long: [bool; 4] = std::array::from_fn(|i| {
        ends(i) - turns[i] >= VERTICAL_MIN_LINES * line_height && turns[i] <= ends(i)
    });
    if long.iter().filter(|&&l| l).count() < 2 {
        return None;
    }
    // The long run each part of the ring belongs to: run `i` its own; a
    // short one's part before its face its last long neighbor's, the part
    // after its next one's.
    let before = |i: usize| (1..4).map(|d| (i + 4 - d) % 4).find(|&j| long[j]).unwrap_or(i);
    let after = |i: usize| (1..4).map(|d| (i + d) % 4).find(|&j| long[j]).unwrap_or(i);
    let mut parts: Vec<(f32, usize)> = Vec::with_capacity(8);
    for i in 0..4 {
        if long[i] {
            parts.push((turns[i], i));
        } else {
            parts.push((turns[i], before(i)));
            parts.push((faces[i].clamp(turns[i], ends(i)), after(i)));
        }
    }
    // Where each run starts: a part whose owner differs from the part's
    // before it.
    let starts: Vec<(f32, usize)> = (0..parts.len())
        .filter(|&k| parts[k].1 != parts[(k + parts.len() - 1) % parts.len()].1)
        .map(|k| parts[k])
        .collect();
    let count = starts.len();
    Some(
        (0..count)
            .map(|k| {
                let (from, owner) = starts[k];
                let to = if k + 1 < count {
                    starts[k + 1].0
                } else {
                    length + starts[0].0
                };
                // A corner at a turn two long runs meet at: the start of a
                // long run that starts there, after another long run.
                let turn = (0..4).find(|&i| turns[i] == from && long[i] && long[(i + 3) % 4]);
                Run {
                    from: (base + from).rem_euclid(length),
                    length: to - from,
                    kind: if owner % 2 == 0 {
                        RunKind::Level
                    } else {
                        RunKind::Steep
                    },
                    side: owner,
                    corner: turn.filter(|_| corners),
                }
            })
            .collect(),
    )
}

/// The way each side's run reads along the canvas, clockwise from the
/// top: what its gaps are measured along.
const AXES: [Vec2; 4] = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];

/// A run of a closed path, from `from` for `length`, run on round the
/// path past either end; an ink pushed off it along its outward normal
/// (inward for `sign` `-1`) by the support of the ellipse its box bounds,
/// as a `repeat` ring's ornament is. Its gaps are measured along `axis`,
/// the way it reads: a level run sets its inks side by side, as a line
/// does, and a steep run stacks them, so that round a curve they step as
/// the curve does, the way ASCII art draws one, rather than crowd where
/// the box of a long thin ink clears the last one's across it.
struct RunPath<'p> {
    arc: &'p ArcPath,
    from: f32,
    length: f32,
    sign: f32,
    axis: Vec2,
    magnitude: f32,
    /// A distance well inside the run from either end, where the way it
    /// runs at that end is read: at a sharp corner the path has two.
    inside: f32,
}

impl Track for RunPath<'_> {
    fn length(&self) -> f32 {
        self.length
    }

    fn at(&self, s: f32) -> (Vec2, Vec2) {
        self.arc.at(self.from + s)
    }

    fn center(&self, s: f32, size: Vec2) -> Vec2 {
        let (point, tangent) = self.at(s);
        point + self.sign * support(self.arc.outward(tangent), size * 0.5)
    }

    fn ends(&self) -> (Vec2, Vec2) {
        (self.at(self.inside).1, self.at(self.length - self.inside).1)
    }

    fn magnitude(&self) -> f32 {
        self.magnitude
    }

    fn gap(&self, a: &Rect, b: &Rect) -> f32 {
        let span = |r: &Rect| {
            let (p, q) = (self.axis.dot(r.origin), self.axis.dot(r.max()));
            (p.min(q), p.max(q))
        };
        span(b).0 - span(a).1
    }

    fn tight(&self) -> bool {
        true
    }

    fn reach(&self, size: Vec2, _dir: Vec2) -> f32 {
        0.5 * size.dot(self.axis).abs()
    }
}

/// A text of the ring laid as one piece — a corner — as the face shaped
/// it: its clusters, the left edge of the first's box, its boxes' width,
/// and its ink, in its own box's coordinates.
struct Unit {
    clusters: Vec<Cluster>,
    left: f32,
    advance: f32,
    ink: Ink,
}

impl Unit {
    /// `clusters` as one piece; `None` where none has ink.
    fn of(run: &ShapedRun, clusters: Vec<Cluster>) -> Option<Self> {
        let left = clusters.iter().map(|c| c.left).fold(f32::INFINITY, f32::min);
        let right = clusters
            .iter()
            .map(|c| c.left + c.advance)
            .fold(f32::NEG_INFINITY, f32::max);
        let ink = clusters
            .iter()
            .filter_map(|c| {
                let shift = Vec2::new(c.left - left, 0.0);
                cluster_ink(&run.glyphs, c, |_| 1.0).map(|ink| Ink {
                    min: ink.min + shift,
                    max: ink.max + shift,
                })
            })
            .reduce(|a, b| Ink {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
            })?;
        Some(Unit {
            clusters,
            left,
            advance: right - left,
            ink,
        })
    }
}

/// The ring's glyphs, sorted by what they draw: its spans' clusters, its
/// vertical text's and each corner's, in the run they were shaped into.
struct Parts {
    level: Vec<Cluster>,
    steep: Vec<Cluster>,
    corners: [Vec<Cluster>; 4],
}

/// The text of `glyphs`' vertical text and corners shaped as one line
/// after the ring's spans, as a filler is: each part its paragraph
/// separators left out, a space between two, which no part's ink reads.
/// Also each part's bytes of it: the vertical text's, then each corner's.
fn appended(glyphs: &DirectionalGlyphs) -> (String, Vec<Range<usize>>) {
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(5);
    let parts = glyphs.vertical.iter().chain(glyphs.corners.iter().flatten());
    for part in parts {
        if !text.is_empty() {
            text.push(' ');
        }
        let start = text.len();
        text.extend(part.chars().filter(|&c| !is_paragraph_separator(c)));
        ranges.push(start..text.len());
    }
    (text, ranges)
}

/// `run`'s clusters sorted into [`Parts`] by the bytes they came from,
/// `ranges` [`appended`]'s; blank clusters left out.
fn parts(run: &ShapedRun, glyphs: &DirectionalGlyphs, ranges: &[Range<usize>]) -> Parts {
    let mut out = Parts {
        level: Vec::new(),
        steep: Vec::new(),
        corners: Default::default(),
    };
    let vertical = usize::from(glyphs.vertical.is_some());
    for cluster in clusters_of(&run.glyphs, 0..run.glyphs.len()) {
        if cluster.blank {
            continue;
        }
        let first = &run.glyphs[cluster.glyphs.start];
        if first.span != FILLER_SPAN {
            out.level.push(cluster);
            continue;
        }
        let Some(part) = ranges.iter().position(|r| r.contains(&first.bytes.start)) else {
            continue;
        };
        if part < vertical {
            out.steep.push(cluster);
        } else if let Some(corner) = out.corners.get_mut(part - vertical) {
            corner.push(cluster);
        }
    }
    out
}

/// What [`place_directional`] laid a ring's corner as: its glyphs, and its
/// ink on the canvas.
struct Corner {
    glyphs: Vec<PlacedGlyph>,
    ink: Rect,
}

/// The corner `unit` set at turn `turn` (`0` the top left, clockwise), `s`
/// along `arc` (see the module docs), where the path turns there as a
/// box does: through 45° or more within the corner's own reach, half its
/// ink's diagonal either side of the turn. It is set as its neighbors are
/// where they run straight at that reach: on the baseline of the level
/// text's ink `level`, its box centered on the box of the steep text's,
/// `steep` with its cluster's advance, each off the path by the normal of
/// its own side. `None` where the path turns less, round a curve wider
/// than the corner: a corner there is no turn of a box, and its runs meet
/// with none between them. `baseline` is the run's.
#[allow(clippy::too_many_arguments)] // The turn, its two neighbors' inks, and the run's metrics.
fn corner(
    arc: &ArcPath,
    s: f32,
    turn: usize,
    unit: &Unit,
    (level, steep, steep_advance): (&Ink, &Ink, f32),
    sign: f32,
    baseline: f32,
    run: &ShapedRun,
) -> Option<Corner> {
    let size = unit.ink.max - unit.ink.min;
    let reach = 0.5 * size.length();
    let ((before, ahead), (after, beyond)) = (arc.at(s - reach), arc.at(s + reach));
    if ahead.dot(beyond) > VERTICAL_EPSILON {
        return None;
    }
    // The top left and the bottom right turn from a side into a level
    // run; the other two from a level run into a side.
    let ((level_at, level_dir), (steep_at, steep_dir)) = if turn.is_multiple_of(2) {
        ((after, beyond), (before, ahead))
    } else {
        ((before, ahead), (after, beyond))
    };
    let set =
        |at: Vec2, dir: Vec2, ink: &Ink| at + sign * support(arc.outward(dir), (ink.max - ink.min) * 0.5);
    let row = set(level_at, level_dir, level).y - (level.min.y + level.max.y) * 0.5;
    let column = set(steep_at, steep_dir, steep).x - (steep.min.x + steep.max.x - steep_advance) * 0.5;
    let ink_center = Vec2::new(column - unit.advance * 0.5, row) + (unit.ink.min + unit.ink.max) * 0.5;
    // The unit's box: its left edge, and its baseline.
    let origin = ink_center - (unit.ink.min + unit.ink.max) * 0.5;
    let top = origin.y - baseline;
    let mut glyphs = Vec::new();
    for cluster in &unit.clusters {
        for index in cluster.glyphs.clone() {
            glyphs.push(PlacedGlyph {
                glyph: index as u32,
                pos: Vec2::new(origin.x + run.glyphs[index].x - unit.left, top),
            });
        }
    }
    Some(Corner {
        glyphs,
        ink: Rect::new(origin + unit.ink.min, size),
    })
}

/// Lay `run`, a `repeat` ring's spans as shaped, round `path` with the
/// glyphs by direction `layout` holds (see the module docs): its spans
/// where the outline runs level, its vertical text where it runs steep,
/// its corners where it turns, each run whole and evenly gapped. `outward`
/// pushes the inks off the outline outward, as a `repeat` ring's are; at
/// most `ring_cap` glyphs, the budget scaled by no more.
///
/// Returns the run the placement refers to — `run` with the vertical text
/// and the corners shaped after it as a line of their own, in `flow`, the
/// ring's own text style — and the placement; `None` where the outline is
/// too small for two runs, or the spans have no ink, which the caller
/// lays as a ring of its spans alone.
///
/// Cost: one reshape of the run, and an inking where it holds a letter;
/// eight support points located; per run, a pitched lay
/// ([`pitch`]): O(its inks) path lookups a lay, for
/// a few lays and a bounded number of rounds of its spacing. Within the
/// ring's budget ([`RingWork`]).
#[allow(clippy::too_many_arguments)] // `place_on_closed`'s, and the cap.
pub(super) fn place_directional(
    run: &ShapedRun,
    path: &Outline,
    layout: &super::RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemLease,
    ring_cap: usize,
    trace: Option<&mut Trace>,
) -> Option<(Option<ShapedRun>, PlacedRun)> {
    let glyphs = &layout.directional;
    let line_height = run.line_height;
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    let arc = ArcPath::closed(
        path,
        start,
        Direction::Clockwise,
        table_spacing(run, &FitMode::Repeat, flow),
    );
    let runs = runs(&arc, path, line_height, glyphs.corners.is_some())?;
    let (text, ranges) = appended(glyphs);
    let (combined, shaped) = reshape(run, 1.0, Some((text.as_str(), flow)), fs.fonts());
    let letters = combined.glyphs.iter().any(|g| g.letter);
    let measured;
    let m = if letters {
        measured = inked(&combined, fs.fonts());
        &measured
    } else {
        &combined
    };
    let parts = parts(m, glyphs, &ranges);
    let level = natural_marks(m, &parts.level, layout.spacing);
    if level.is_empty() {
        return None;
    }
    // The ring's gap: its spans', as a line sets two of them.
    let gap = level.iter().map(|mark| mark.gap).sum::<f32>() / level.len() as f32;
    let mut steep = natural_marks(m, &parts.steep, 0.0);
    for mark in &mut steep {
        mark.gap = gap;
    }
    let (steep, steep_clusters) = if steep.is_empty() {
        (level.clone(), &parts.level)
    } else {
        (steep, &parts.steep)
    };
    let units: [Option<Unit>; 4] = parts.corners.map(|clusters| {
        if clusters.is_empty() {
            None
        } else {
            Unit::of(m, clusters)
        }
    });
    let of = |kind: RunKind| match kind {
        RunKind::Level => (&level, &parts.level),
        RunKind::Steep => (&steep, steep_clusters),
    };
    let widest = |clusters: &[Cluster]| clusters.iter().map(|c| c.glyphs.len()).max().unwrap_or(1).max(1);

    // Its budget: scaled by the glyphs its runs will hold, each run's
    // length over its inks' mean step along it, and its corners'.
    let cap = ring_cap.min(MAX_PLACED_GLYPHS);
    let corner_glyphs: usize = units
        .iter()
        .flatten()
        .map(|u| u.clusters.iter().map(|c| c.glyphs.len()).sum::<usize>())
        .sum();
    let estimate = runs
        .iter()
        .map(|r| {
            let (marks, clusters) = of(r.kind);
            let along = |mark: &Mark| match r.kind {
                RunKind::Level => mark.size().x,
                RunKind::Steep => mark.size().y,
            };
            let step = marks.iter().map(|mark| along(mark) + mark.gap).sum::<f32>() / marks.len() as f32;
            ring_glyphs(widest(clusters), f64::from(step), r.length, cap)
        })
        .fold(corner_glyphs, usize::saturating_add)
        .min(cap);
    let work = RingWork::new(&arc, widest(&parts.level), letters, line_height, estimate, cap);
    work.took(resizing_steps(&combined, shaped));
    if letters {
        work.took(inking_steps(&combined));
    }
    let (budget, glyph_budget, _) = work.budget();
    let stopped = Cell::new(false);

    let rect = path.rect();
    let magnitude = arc
        .length()
        .max(rect.origin.abs().max_element())
        .max(rect.max().abs().max_element());
    let eps = resolved(1e-3 * line_height, magnitude);
    let sign = if outward { 1.0 } else { -1.0 };
    let baseline = combined.baseline();

    let steep_advance = steep_clusters[steep[0].cluster].advance;
    let corners: Vec<Option<Corner>> = runs
        .iter()
        .map(|r| {
            let turn = r.corner?;
            let unit = units[turn].as_ref()?;
            corner(
                &arc,
                r.from,
                turn,
                unit,
                (&level[0].ink, &steep[0].ink, steep_advance),
                sign,
                baseline,
                m,
            )
        })
        .collect();

    let mut placed: Vec<PlacedGlyph> = Vec::new();
    let mut truncated = false;
    // Each run's first and last ink, what a run beside it with no corner
    // between them is measured to.
    let mut first_ink: Option<Rect> = None;
    let mut last_ink: Option<Rect> = None;
    let count = runs.len();
    for (k, r) in runs.iter().enumerate() {
        if let Some(c) = &corners[k] {
            placed.extend_from_slice(&c.glyphs);
            work.set(c.glyphs.len());
        }
        let (marks, clusters) = of(r.kind);
        // An end with nothing beside it is measured to where the run
        // ends, along the way it reads.
        let point = |s: f32| Rect::new(arc.at(s).0, Vec2::ZERO);
        let start_inks = [corners[k]
            .as_ref()
            .map(|c| c.ink)
            .or(last_ink)
            .unwrap_or_else(|| point(r.from))];
        let next = (k + 1) % count;
        let end_inks = [corners[next]
            .as_ref()
            .map(|c| c.ink)
            .or_else(|| first_ink.filter(|_| next == 0))
            .unwrap_or_else(|| point(r.from + r.length))];
        let ends = [&start_inks[..], &end_inks[..]].map(|inks| PathEnd {
            inks,
            back: 0.0,
            hold: 0.0,
        });
        let track = RunPath {
            arc: &arc,
            from: r.from,
            length: r.length,
            sign,
            axis: AXES[r.side],
            magnitude,
            inside: eps.min(0.5 * r.length),
        };
        let room = cap.saturating_sub(placed.len()) / widest(clusters);
        let go = |inks: usize| {
            let held = LAYS_HELD.saturating_mul(lay_steps(inks));
            let ok = work.spent(&arc).saturating_add(held) < budget;
            if !ok {
                stopped.set(true);
            }
            ok
        };
        last_ink = None;
        // Never so tight that an ink passes the one before it.
        let floor = marks
            .iter()
            .map(|mark| -(mark.size().dot(track.axis).abs() + mark.gap))
            .fold(0.0, f32::min);
        let Some(pitched) = pitch(&track, marks, room, floor, ends, &go) else {
            continue;
        };
        truncated |= pitched.truncated;
        let centers: Vec<(usize, Vec2)> = pitched
            .centers
            .iter()
            .enumerate()
            .map(|(j, &s)| {
                let j = j % marks.len();
                (j, track.center(s, marks[j].size()))
            })
            .collect();
        let ink_of = |&(j, center): &(usize, Vec2)| {
            let size = marks[j].size();
            Rect::new(center - size * 0.5, size)
        };
        if k == 0 {
            first_ink = centers.first().map(ink_of);
        }
        last_ink = centers.last().map(ink_of);
        let laid = glyphs_at(m, clusters, marks, &centers);
        work.set(laid.len());
        placed.extend(laid);
    }
    if stopped.get() {
        work.lose(RingLoss::Short);
    }
    if let Some(trace) = trace {
        trace.spent = work.spent(&arc);
        trace.tables = work.tables();
        trace.budget = budget;
        trace.glyphs = glyph_budget;
        trace.ring = work.loss();
    }
    #[cfg(test)]
    super::halves::PAST_BUDGET
        .with(|past| past.set(past.get().saturating_add(work.spent(&arc).saturating_sub(budget))));
    Some((
        Some(combined),
        PlacedRun {
            glyphs: placed,
            truncated,
        },
    ))
}
