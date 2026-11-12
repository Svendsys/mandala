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
//! else. A point **turns as a box does** ([`turns_as_a_box`]) where a
//! corner set there would lie on the outline: where its box's corner,
//! its two sides run straight on to meet, lies within half a line height
//! of the outline's turn — a sharp corner, or an arc about a glyph wide;
//! not a wider arc, nor a flat ellipse's tip, however fast it turns.
//! **The hysteresis** is the
//! readable rule's too ([`VERTICAL_MIN_LINES`]): a run shorter than three
//! line heights that ends on a curve — the tips of a flat ellipse, the
//! top of a tall one — is no run of its own; its two neighbors take it,
//! parted at its middle, where the outline faces straight out, and no
//! corner is laid at its ends. A side between two turns of a box is a
//! side however short: a sharp turn cannot flicker.
//!
//! **Its corners** are set where two runs meet at a turn of a box,
//! clockwise from the top left: top left, top right, bottom right, bottom
//! left. A corner is set as its neighbors are, cell to cell, the way a
//! line of text sets glyphs of one face: on the baseline of the level
//! run's text, and its box centered on the box of the steep run's text,
//! each read where its side runs straight — so a box-drawing corner's two
//! strokes run on in its neighbors' strokes, as the face draws them on a
//! grid, and a `◆` sits where the face sets it on a line of `━`. Round a
//! curve wider than that, the runs meet with no corner. A ring with no
//! text to run beside its corners sets each as its own ink sits; a side
//! too short to keep its two corners apart sets none.
//!
//! **Each run is laid whole**, as a connector's pattern is between its
//! ends ([`pitch`]): whole glyphs, every gap between two inks the same,
//! measured along the way the run reads, the gaps at its two ends to the
//! corners beside it included. Where two runs meet with no corner, each
//! ends where the outline turns, and they step there as the curve does.
//! The gap is the ring's: its spans' own, as a line sets two of them, its
//! `spacing` added; the vertical text is set that same gap apart, so the
//! steep runs read as evenly as the level ones. Each run holds as many
//! inks as leave no gap looser than its own, and presses what is left
//! evenly into its gaps. Its inks are pushed off the outline straight
//! across the way it reads, as a line sits on its row; a side of a box
//! lies straight along the outline's side, from its box's corner to the
//! next.
//!
//! **Where it is drawn.** Its path keeps the outline's corners as they
//! are: the projection grows a sharp rectangle by the ring's offset into
//! a sharp rectangle ([`offset_keeping_corners`]), so its corners are
//! where its runs' strokes meet.
//!
//! **What it holds and costs.** At most its cap of glyphs (ruling
//! B17.2a): its four corners first, all of them or none, then each run a
//! share of what is left by what it holds, a run that lays less passing
//! the rest on, and a run its share cut laid again with what the others
//! left spare — so a ring its cap holds is still laid all round, and one
//! its cap would hold whole is laid whole. Its work is counted and held
//! to the ring's budget as a `repeat` ring's is ([`RingWork`], #310): its
//! path's reads (a side of a box's included, each of its places counted
//! as one), the glyphs it sets and its shaping, scaled by the glyphs
//! its runs will hold. Each run's count and each round of its spacing
//! reads the budget first, and holds back what the run's last lays take
//! ([`LAYS_HELD`]); once it is spent no further run is laid. A ring its
//! budget stops, or its cap holds, is short ([`RingLoss::Short`]).

use std::cell::Cell;
use std::ops::Range;

use glam::Vec2;

use super::halves::{inked, inking_steps, resizing_steps, resolved};
use super::pitch::{glyphs_at, natural_marks, pitch, Mark, PathEnd, Track, BRACKET_ROUNDS, BRACKET_STRIDES};
use super::ring::{ring_glyphs, steps_of, RingLoss, RingWork};
use super::{
    cluster_ink, clusters_of, support, table_spacing, Cluster, PlacedGlyph, PlacedRun, Trace,
    MAX_PLACED_GLYPHS, VERTICAL_MIN_LINES,
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
use super::VERTICAL_EPSILON;
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

/// The most calls of its gap a run's solve of one ink's place makes: it
/// is bracketed ([`Track::bracketed`]), its gap along the way the run
/// reads growing with the place all along it.
const SOLVE_READS: usize = 2 + BRACKET_STRIDES + BRACKET_ROUNDS;

/// The most path reads a lay of a run takes an ink
/// ([`pitch`]'s `next_at`): its box, its direction,
/// the direction at its first guess, and a solve of its place where the
/// path bends ([`SOLVE_READS`]).
const INK_READS: usize = 3 + SOLVE_READS;

/// The most path reads a lay of a run takes at its two ends, past its
/// inks: each end's ink solved against what is beside it, its gap and its
/// refinements.
const END_READS: usize = 2 * SOLVE_READS;

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
    /// Whether it runs from one box's turn to the next: a side of a box,
    /// laid straight along the outline's side, as its corners are set.
    pub(crate) straight: bool,
}

/// The runs a ring of line height `line_height` round `arc`, the path
/// `outline` was walked into, is parted into, clockwise from its first
/// turn's (see the module docs), corners set at a turn between two runs
/// that turns as a box does, where `corners` says so. None where fewer
/// than two runs are runs: an outline too small to turn on, which is laid
/// as a ring of its spans alone.
///
/// A turn is a box's ([`turns_as_a_box`]) where a corner set there would
/// lie on the outline. A run shorter than
/// [`VERTICAL_MIN_LINES`] line heights is a run all the same where both
/// its ends are: a short side of a box is a side, and a sharp turn cannot
/// flicker. One that ends on a curve is not.
///
/// Cost: eight support points located on the path, and three support
/// points a turn.
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
    // A path whose floats cannot tell its turns apart may locate them out
    // of order, or nowhere: it has no runs to lay.
    if !(0..4).all(|i| turns[i] <= ends(i) && faces[i].is_finite()) {
        return None;
    }
    let boxed: [bool; 4] = std::array::from_fn(|i| turns_as_a_box(outline, i, line_height));
    let long: [bool; 4] = std::array::from_fn(|i| {
        ends(i) - turns[i] >= VERTICAL_MIN_LINES * line_height || (boxed[i] && boxed[(i + 1) % 4])
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
                // A corner at a box's turn two runs meet at: the start of a
                // run that starts there, after another run.
                let turn = (0..4).find(|&i| turns[i] == from && long[i] && long[(i + 3) % 4] && boxed[i]);
                // A run takes no neighbor's part when both its neighbors
                // are runs: it is the side between two of the turns.
                let straight =
                    long[(owner + 1) % 4] && long[(owner + 3) % 4] && boxed[owner] && boxed[(owner + 1) % 4];
                Run {
                    from: (base + from).rem_euclid(length),
                    length: to - from,
                    kind: if owner.is_multiple_of(2) {
                        RunKind::Level
                    } else {
                        RunKind::Steep
                    },
                    side: owner,
                    corner: turn.filter(|_| corners),
                    straight,
                }
            })
            .collect(),
    )
}

/// Where `outline`'s two sides at turn `turn` (`0` the top left,
/// clockwise) meet, each run straight on from where it faces straight
/// out: the corner of its box. Where a corner there is set, and where a
/// side of a box runs to.
///
/// Cost: two support points.
fn corner_point(outline: &Outline, turn: usize) -> Vec2 {
    let t = TURNS[turn];
    Vec2::new(
        outline.support(Vec2::new(t.x, 0.0)).x,
        outline.support(Vec2::new(0.0, t.y)).y,
    )
}

/// Whether `outline` turns as a box does at turn `turn`: where its box's
/// corner there ([`corner_point`]) lies within half a line height of the
/// outline, at its turn — a sharp corner, or one rounded about a glyph
/// wide. Round a wider curve, or at the tip of a flat ellipse, a corner
/// set there would lie off the outline.
///
/// Cost: three support points.
fn turns_as_a_box(outline: &Outline, turn: usize, line_height: f32) -> bool {
    corner_point(outline, turn).distance(outline.support(TURNS[turn])) <= 0.5 * line_height
}

/// The way each side's run reads along the canvas, clockwise from the
/// top: what its gaps are measured along.
const AXES: [Vec2; 4] = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];

/// A run of a closed path, from `from` for `length`, run on round the
/// path past either end. Its gaps are measured along `axis`, the way it
/// reads: a level run sets its inks side by side, as a line does, and a
/// steep run stacks them, so that round a curve they step as the curve
/// does, the way ASCII art draws one, rather than crowd where the box of
/// a long thin ink clears the last one's across it. Its inks are pushed
/// off the path straight across that way, outward (inward for `sign`
/// `-1`), by half their extent across it, as a line sits on its row: a
/// level run's up off the top or down off the bottom, a steep run's out
/// to the side, as a corner's are. (Pushed along the path's normal, a long
/// thin ink near an outline's tip swings far across with the slightest
/// turn of the normal, and its gaps cannot be held to one.)
struct RunPath<'p> {
    arc: &'p ArcPath,
    from: f32,
    length: f32,
    sign: f32,
    /// For a side of a box, where it starts: its box's corner, from which
    /// it runs straight along the outline's side to the next, however its
    /// corners are rounded, and on straight past either end.
    line: Option<Vec2>,
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
        match self.line {
            Some(origin) => {
                // Read as a path is, and counted with its reads.
                self.arc.read();
                (origin + self.axis * s, self.axis)
            }
            None => self.arc.at(self.from + s),
        }
    }

    fn center(&self, s: f32, size: Vec2) -> Vec2 {
        // Outward from the way it reads, clockwise round the outline.
        let across = Vec2::new(self.axis.y, -self.axis.x);
        self.at(s).0 + self.sign * across * across.dot(size * 0.5).abs()
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

    fn bracketed(&self) -> bool {
        // A side of a box runs straight: its gaps are box to box.
        self.line.is_none()
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

/// The corner `unit` set at turn `turn` (`0` the top left, clockwise) of
/// `outline`, a turn of a box ([`turns_as_a_box`], see the module docs).
/// It is set as its neighbors are where their sides run straight, read at
/// the outline's support points in the directions its sides face: on the
/// baseline of the level text's ink `level` set on the top or the bottom,
/// and its box centered on the box of the steep text's, `steep` with its
/// cluster's advance, set on its side — each off the outline by that
/// side's own normal. Round a rectangle, sharp or rounded no wider than a
/// glyph, those are the lines its runs' strokes lie on. `baseline` is the
/// run's.
///
/// Cost: O(the unit's glyphs); no path lookup.
fn corner(
    outline: &Outline,
    turn: usize,
    unit: &Unit,
    (level, steep, steep_advance): (&Ink, &Ink, f32),
    sign: f32,
    baseline: f32,
    run: &ShapedRun,
) -> Corner {
    // The level side the turn is on, the top for the top two, and its
    // steep side, the left for the left two.
    let level_face = if turn < 2 { FACES[0] } else { FACES[2] };
    let steep_face = if turn == 0 || turn == 3 {
        FACES[3]
    } else {
        FACES[1]
    };
    let set = |face: Vec2, ink: &Ink| outline.support(face) + sign * support(face, (ink.max - ink.min) * 0.5);
    let row = set(level_face, level).y - (level.min.y + level.max.y) * 0.5;
    let column = set(steep_face, steep).x - (steep.min.x + steep.max.x - steep_advance) * 0.5;
    // The unit's box: its left edge, and its baseline.
    let origin = Vec2::new(column - unit.advance * 0.5, row);
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
    Corner {
        glyphs,
        ink: Rect::new(origin + unit.ink.min, unit.ink.max - unit.ink.min),
    }
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
/// too small for two runs, which the caller lays as a ring of its spans
/// alone, before anything is shaped. Spans with no ink lay the vertical
/// text all round, or, with none, the corners alone.
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
    let mut level = natural_marks(m, &parts.level, layout.spacing);
    let mut steep = natural_marks(m, &parts.steep, 0.0);
    // The ring's gap: its spans', as a line sets two of them; spans with
    // no ink lay the vertical text all round, at its own.
    let (level, level_clusters) = if level.is_empty() {
        steep = natural_marks(m, &parts.steep, layout.spacing);
        (steep.clone(), &parts.steep)
    } else {
        let gap = level.iter().map(|mark| mark.gap).sum::<f32>() / level.len() as f32;
        for mark in &mut steep {
            mark.gap = gap;
        }
        (std::mem::take(&mut level), &parts.level)
    };
    let (steep, steep_clusters) = if steep.is_empty() {
        (level.clone(), level_clusters)
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
        RunKind::Level => (&level, level_clusters),
        RunKind::Steep => (&steep, steep_clusters),
    };
    let widest = |clusters: &[Cluster]| clusters.iter().map(|c| c.glyphs.len()).max().unwrap_or(1).max(1);
    let glyph_count = |unit: &Unit| unit.clusters.iter().map(|c| c.glyphs.len()).sum::<usize>();

    // Its budget: scaled by the glyphs its runs will hold, each run's
    // length over its inks' mean step along it, and its corners'.
    let cap = ring_cap.min(MAX_PLACED_GLYPHS);
    let corner_glyphs: usize = units.iter().flatten().map(glyph_count).sum();
    let needs: Vec<usize> = runs
        .iter()
        .map(|r| {
            let (marks, clusters) = of(r.kind);
            if marks.is_empty() {
                return 0;
            }
            let along = |mark: &Mark| match r.kind {
                RunKind::Level => mark.size().x,
                RunKind::Steep => mark.size().y,
            };
            let step = marks.iter().map(|mark| along(mark) + mark.gap).sum::<f32>() / marks.len() as f32;
            ring_glyphs(widest(clusters), f64::from(step), r.length, cap)
        })
        .collect();
    let need = needs.iter().copied().fold(0, usize::saturating_add);
    let estimate = need.saturating_add(corner_glyphs).min(cap);
    let work = RingWork::new(&arc, widest(level_clusters), letters, line_height, estimate, cap);
    work.took(resizing_steps(&combined, shaped));
    if letters {
        work.took(inking_steps(&combined));
    }
    let (budget, glyph_budget, _) = work.budget();
    // Whether the budget stopped the ring, and whether its cap held it.
    let stopped = Cell::new(false);
    let mut capped = false;

    let rect = path.rect();
    let magnitude = arc
        .length()
        .max(rect.origin.abs().max_element())
        .max(rect.max().abs().max_element());
    let eps = resolved(1e-3 * line_height, magnitude);
    let sign = if outward { 1.0 } else { -1.0 };
    let baseline = combined.baseline();

    // Its corners first: set as its runs' texts are, or, with no text to
    // run beside, as their own ink sits.
    let mut corners: Vec<Option<Corner>> = runs
        .iter()
        .map(|r| {
            let turn = r.corner?;
            let unit = units[turn].as_ref()?;
            let beside = match (level.first(), steep.first()) {
                (Some(level), Some(steep)) => (&level.ink, &steep.ink, steep_clusters[steep.cluster].advance),
                _ => (&unit.ink, &unit.ink, unit.advance),
            };
            Some(corner(path, turn, unit, beside, sign, baseline, m))
        })
        .collect();
    // All of them or none: none where its cap cannot hold them all, and
    // the ring is short, or where a side too short to part them would
    // set two over each other.
    let set = corners.iter().flatten();
    let mut set_glyphs: usize = set.clone().map(|c| c.glyphs.len()).sum();
    let apart = |a: &Rect, b: &Rect| {
        let (lo, hi) = (a.origin.max(b.origin), a.max().min(b.max()));
        hi.x <= lo.x || hi.y <= lo.y
    };
    let overlap = set
        .clone()
        .enumerate()
        .any(|(i, a)| set.clone().skip(i + 1).any(|b| !apart(&a.ink, &b.ink)));
    if set_glyphs > cap || overlap {
        capped |= set_glyphs > cap;
        corners.iter_mut().for_each(|c| *c = None);
        set_glyphs = 0;
    }
    work.set(set_glyphs);
    let room = cap - set_glyphs;
    // What the cap leaves its runs: each a share of what is left by what
    // it needs against what the runs still to be laid need, so a run that
    // lays less than its share passes the rest on, and a ring its cap
    // holds is still laid all round.
    let share = |k: usize, laid: usize| {
        let left = room.saturating_sub(laid);
        let rest: usize = needs[k..].iter().copied().fold(0, usize::saturating_add);
        if rest == 0 {
            return left;
        }
        ((left as f64 * needs[k] as f64 / rest as f64).ceil() as usize).min(left)
    };
    let count = runs.len();
    // Lay run `k` at most `inks` inks: its glyphs, and whether its cap
    // cut it. `None` where it lays none.
    let lay_run = |k: usize, inks: usize| -> Option<(Vec<PlacedGlyph>, bool)> {
        let r = &runs[k];
        let (marks, clusters) = of(r.kind);
        // A side of a box: the outline's side from its box's corner to the
        // next.
        let axis = AXES[r.side];
        let line = r.straight.then(|| {
            let (from, to) = (corner_point(path, r.side), corner_point(path, (r.side + 1) % 4));
            (from, axis.dot(to - from))
        });
        let length = line.map_or(r.length, |(_, length)| length);
        let track = RunPath {
            arc: &arc,
            from: r.from,
            length,
            sign,
            line: line.map(|(from, _)| from),
            axis,
            magnitude,
            inside: eps.min(0.5 * length),
        };
        // An end with nothing beside it is measured to where the run
        // ends, along the way it reads.
        let point = |s: f32| Rect::new(track.at(s).0, Vec2::ZERO);
        let start_inks = [corners[k].as_ref().map_or_else(|| point(0.0), |c| c.ink)];
        let next = (k + 1) % count;
        let end_inks = [corners[next].as_ref().map_or_else(|| point(length), |c| c.ink)];
        let ends = [&start_inks[..], &end_inks[..]].map(|inks| PathEnd {
            inks,
            back: 0.0,
            hold: 0.0,
        });
        let go = |inks: usize| {
            let held = LAYS_HELD.saturating_mul(lay_steps(inks));
            let ok = work.spent(&arc).saturating_add(held) < budget;
            if !ok {
                stopped.set(true);
            }
            ok
        };
        // Never so tight that an ink passes the one before it.
        let floor = marks
            .iter()
            .map(|mark| -(mark.size().dot(track.axis).abs() + mark.gap))
            .fold(0.0, f32::min);
        let pitched = pitch(&track, marks, inks, floor, ends, &go)?;
        let centers: Vec<(usize, Vec2)> = pitched
            .centers
            .iter()
            .enumerate()
            .map(|(j, &s)| {
                let j = j % marks.len();
                (j, track.center(s, marks[j].size()))
            })
            .collect();
        let laid = glyphs_at(m, clusters, marks, &centers);
        work.set(laid.len());
        Some((laid, pitched.truncated))
    };
    let mut by_run: Vec<(Vec<PlacedGlyph>, bool)> = vec![(Vec::new(), false); count];
    let mut laid_by_runs = 0usize;
    for k in 0..count {
        let (marks, clusters) = of(runs[k].kind);
        // A ring its budget stopped lays no run after the one it stopped.
        if marks.is_empty() || stopped.get() {
            continue;
        }
        let inks = share(k, laid_by_runs) / widest(clusters);
        if inks == 0 {
            capped = true;
            continue;
        }
        if let Some(laid) = lay_run(k, inks) {
            laid_by_runs += laid.0.len();
            by_run[k] = laid;
        }
    }
    // What the shares left spare, to a run its share cut, laid again: a
    // share by what each run needs can fall a glyph short of what one
    // holds where the cap would hold the whole ring.
    for k in 0..count {
        let spare = room.saturating_sub(laid_by_runs);
        let (laid, cut) = &by_run[k];
        if !cut || spare == 0 || stopped.get() {
            continue;
        }
        let had = laid.len();
        let inks = (had + spare) / widest(of(runs[k].kind).1);
        if let Some(again) = lay_run(k, inks) {
            laid_by_runs = laid_by_runs - had + again.0.len();
            by_run[k] = again;
        }
    }
    let truncated = by_run.iter().any(|(_, cut)| *cut);
    let mut placed: Vec<PlacedGlyph> = Vec::new();
    for (corner, (laid, _)) in corners.iter().zip(by_run) {
        if let Some(c) = corner {
            placed.extend_from_slice(&c.glyphs);
        }
        placed.extend(laid);
    }
    if stopped.get() || capped || truncated {
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
            truncated: truncated || capped,
        },
    ))
}
