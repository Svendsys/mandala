// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33 and R33.1).
//!
//! The owner, on the frame text: "It should follow the path of its
//! halve, this should be adjustable in the solution. But the point here
//! was to demo the text actually taking the shape of our nodes."
//!
//! **The halves** (ruling R33). The outline is split on its diagonal, at
//! its support points in the directions `(−1, −1)` and `(1, 1)`
//! ([`Outline::support`]): the point where the tangent is at 45° on an
//! ellipse, the corner arc's midpoint on a rounded rectangle. The first
//! line takes the upper-right half — from the top-left junction
//! clockwise over the top and down the right side to the bottom-right
//! junction — and the second the lower-left half — from the top-left
//! junction down the left side and along the bottom to the bottom-right
//! one. So every vertical run reads down and every horizontal one reads
//! right, glyphs upright.
//!
//! **The span** (ruling R32, [`Span`]). R27 decides the split and the
//! size ([`place_lines`]). At [`Span::NATURAL`] each line lies at its
//! natural width: tracked at its font's own gap, centered on its half's
//! apex (where R27 centers it), and slid along its half where that
//! would reach into a junction's clearance. Above it, each end of a
//! line moves by arc length from its natural place toward its half's
//! junction, by `span` of the way, and never into the clearance.
//!
//! **Even tracking.** Over the part it covers, a line is laid with one
//! ink gap `g` between every two neighboring letters, its letters' side
//! bearings left out, and a space's advance plus `g` more between words:
//! a word gap is `2g` plus a space. Each letter after the first is set
//! by the ornament walk's pairwise rule (ruling R31), as far from the
//! last letter's ink as owed, a zero gap included, so the line's length
//! grows with `g` and `g` is solved by bisection: the line's first and
//! last inks centered on its part's ends ([`track`]).
//!
//! **Each junction is a visible break** (ruling R33.1). The two lines'
//! nearest end inks at a junction stay at least two word gaps apart —
//! of the two lines' mean, each line stopping its own word gap short —
//! so where one line ends and the other begins reads at a glance. At
//! [`Span::WHOLE`], "whole" is the half less that clearance, found in a
//! few rounds at the whole span and kept at every span, so the ends move
//! monotonically with it. No line that its half holds is laid tighter
//! than its font's own gap, its letters' mean side bearings
//! ([`place_halves`]).
//!
//! **Sides.** With `sides`, an ornament closes what the lines leave
//! round each junction: whole copies, at least one, every gap and both
//! joins to the text equal (ruling R31's lay, seeded by the text's end
//! inks), on the text's own center line, the ornament's ink no taller
//! than the text's x-height. The ornament is shaped into the run as a
//! line of its own, the way a `Once` filler is.

use std::cmp::Ordering;
use std::ops::Range;

use glam::Vec2;

use super::lines::place_lines;
use super::*;
use crate::document::values::{Edge, FontSize};
use crate::flow::shape::shape_run;
use crate::flow::span::FlowSpan;
use crate::font::fonts::glyph_outline_ink;
use crate::font::SwashCache;

/// The rounds of bisection a line's tracking is solved in: a bracket of
/// a few points, a line's gap, closes to some millionths of a point.
const TRACK_ROUNDS: usize = 20;

/// The most times a line's tracking bracket doubles looking for a gap
/// too wide to fit: past `2^24` points nothing is laid.
const TRACK_DOUBLINGS: usize = 24;

/// The most rounds a natural line is slid back from its bottom-right
/// junction in: each sets its start so that its length from there ends
/// at the clearance, its length changing only with the path's bend, so
/// the miss shrinks by a fraction each round.
const SLIDE_ROUNDS: usize = 16;

/// How near the clearance a slid line's end must come, in points.
const SLIDE_TOLERANCE: f32 = 1e-4;

/// The rounds a junction's clearance is grown in.
const CLEAR_ROUNDS: usize = 16;

/// The most copies of a side ornament one junction holds.
const MAX_SIDE_COPIES: usize = 256;

/// The rounds of bisection a side arc's gap is solved in.
const SIDE_ROUNDS: usize = 30;

/// How many word gaps apart two lines' end inks stay at a junction
/// (ruling R33.1).
pub(crate) const JUNCTION_WORD_GAPS: f32 = 2.0;

/// How far over the clearance the rounds aim, so a measure of it never
/// rounds below.
const CLEAR_MARGIN: f32 = 1.01;

/// The outline's two halves (ruling R33): each its path, the part of it
/// between its junctions, and its apex.
struct Halves {
    /// The outline walked from the middle of its left side, clockwise
    /// (the first half's) and counterclockwise (the second's).
    paths: [ArcPath; 2],
    /// Each half's junctions along its path, top-left then bottom-right,
    /// the second past the first.
    junctions: [(f32, f32); 2],
    /// Each half's apex along its path, between its junctions: the
    /// outline's support point straight up for the first, straight down
    /// for the second — where R27 centers a line.
    apexes: [f32; 2],
    /// The outline's length.
    length: f32,
}

impl Halves {
    fn new(outline: &Outline, spacing: f32) -> Self {
        let left = StartPoint {
            edge: Edge::Left,
            at: 0.5,
        };
        let paths = [
            ArcPath::closed(outline, left, Direction::Clockwise, spacing),
            ArcPath::closed(outline, left, Direction::CounterClockwise, spacing),
        ];
        let length = paths[0].length();
        let top_left = outline.support(Vec2::new(-1.0, -1.0));
        let bottom_right = outline.support(Vec2::new(1.0, 1.0));
        let tops = [
            outline.support(Vec2::new(0.0, -1.0)),
            outline.support(Vec2::new(0.0, 1.0)),
        ];
        let mut junctions = [(0.0, 0.0); 2];
        let mut apexes = [0.0; 2];
        for k in 0..2 {
            let start = paths[k].nearest(top_left);
            let end = unwrap(paths[k].nearest(bottom_right), start, length);
            junctions[k] = (start, end);
            apexes[k] = unwrap(paths[k].nearest(tops[k]), start, length);
        }
        Halves {
            paths,
            junctions,
            apexes,
            length,
        }
    }

    /// Where along half `k` the point nearest `p` lies, past its start.
    fn along(&self, k: usize, p: Vec2) -> f32 {
        unwrap(self.paths[k].nearest(p), self.junctions[k].0, self.length)
    }
}

/// `s` moved by whole turns of `length` to lie at or past `from`.
fn unwrap(s: f32, from: f32, length: f32) -> f32 {
    if length > 0.0 {
        from + (s - from).rem_euclid(length)
    } else {
        s
    }
}

/// The distance between two boxes, `(min, max)` each: 0 where they meet.
fn box_gap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    (b.0 - a.1).max(a.0 - b.1).max(Vec2::ZERO).length()
}

/// The center of a box, `(min, max)`.
fn middle(b: (Vec2, Vec2)) -> Vec2 {
    0.5 * (b.0 + b.1)
}

/// `run` with every glyph's [`GlyphMetrics::ink`] measured but a space's:
/// what tracking a line, and its side ornament, step by.
///
/// Cost: one outline scale per glyph that is not a space.
fn inked(run: &ShapedRun, fs: &mut FontSystem) -> ShapedRun {
    let mut out = run.clone();
    let mut cache = SwashCache::new();
    let laid: Vec<_> = run.layout_glyphs().cloned().collect();
    for (glyph, laid) in out.glyphs.iter_mut().zip(&laid) {
        if !glyph.blank {
            glyph.ink = glyph_outline_ink(fs, &mut cache, laid).map(|(min, max)| Ink { min, max });
        }
    }
    out
}

/// `run` with no glyph's ink: what placement returns, a run's ink being
/// an ornament's own ([`GlyphMetrics::ink`]).
fn uninked(mut run: ShapedRun) -> ShapedRun {
    for glyph in &mut run.glyphs {
        glyph.ink = None;
    }
    run
}

/// Placed glyph `g`'s ink box on the canvas, `(min, max)`.
fn ink_box(run: &ShapedRun, g: &PlacedGlyph) -> Option<(Vec2, Vec2)> {
    let ink = run.glyphs.get(g.glyph as usize)?.ink?;
    let pen = g.pos + Vec2::new(0.0, run.baseline());
    Some((pen + ink.min, pen + ink.max))
}

/// One line laid with one tracking gap ([`track`]).
struct Tracked {
    /// Its glyphs, in text order.
    glyphs: Vec<PlacedGlyph>,
    /// How far before its part's end its last ink's center falls, by arc
    /// length: positive with room left, negative past it.
    short: f32,
    /// Its first and last ink boxes.
    ends: Option<((Vec2, Vec2), (Vec2, Vec2))>,
}

/// `clusters` laid along `walk`'s path over the part `[sa, sb]` with
/// tracking `g`: the first ink centered at `sa`; every letter after it
/// set by the pairwise rule (ruling R31, [`PathWalk::ink_gap_at`]) `g`
/// from the letter before, a zero gap included; a space owing its
/// advance and `g` more. A space draws nothing and is placed where its
/// step starts. [`Tracked::short`] is how far before `sb` the last ink's
/// center falls, by arc length: it shrinks as `g` grows, a zero gap
/// included, so the end condition can be bisected.
///
/// Cost: O(clusters), each an [`ink_gap_at`](PathWalk::ink_gap_at).
fn track(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    (sa, sb): (f32, f32),
    g: f32,
) -> Tracked {
    let mut out = Vec::new();
    let mut last: Option<(Vec2, Vec2, f32)> = None;
    let mut first_box: Option<(Vec2, Vec2)> = None;
    let mut owed = 0.5 * g;
    let mut s = sa;
    for cluster in clusters {
        if cluster.blank {
            let at = walk.path.at(s).0;
            for index in cluster.glyphs.clone() {
                out.push(PlacedGlyph {
                    glyph: index as u32,
                    pos: at - Vec2::new(0.5 * cluster.advance, walk.baseline),
                });
            }
            owed += cluster.advance + g;
            s += cluster.advance;
            continue;
        }
        let body = walk.body_of(cluster);
        let Body::Inked { ink, .. } = body else {
            continue;
        };
        let (step, dir) = walk.take(s, body);
        let size = ink.max - ink.min;
        let natural = s + ink.min.x + extent(dir, size.x, size.y) * 0.5;
        let center = match last {
            Some((at, half, from)) => walk.ink_gap_at((at, half), from, natural, owed.max(0.0), ink),
            // The first ink centered at the part's start.
            None => sa,
        };
        let at = walk.ink_at(center, ink);
        let corner = at - (ink.min + ink.max) * 0.5 - Vec2::new(0.0, walk.baseline);
        for index in cluster.glyphs.clone() {
            out.push(PlacedGlyph {
                glyph: index as u32,
                pos: corner + Vec2::new(glyphs[index].x - cluster.left, 0.0),
            });
        }
        let half = size * 0.5;
        first_box.get_or_insert((at - half, at + half));
        last = Some((at, half, center));
        owed = g;
        s = s + (center - natural) + step;
    }
    let (short, ends) = match (last, first_box) {
        (Some((c, h, at)), Some(first)) => (sb - at, Some((first, (c - h, c + h)))),
        _ => (sb - sa, None),
    };
    Tracked {
        glyphs: out,
        short,
        ends,
    }
}

/// One line along its half ([`Layout::lay`]).
struct Line {
    /// Its glyphs, as [`track`] laid them at its gap.
    tracked: Tracked,
    /// Its tracking gap, and its font's own: its letters' mean side
    /// bearings.
    gap: f32,
    natural_gap: f32,
    /// Its word gap: a space's advance and two gaps.
    word: f32,
    /// The part of its half it covers.
    part: (f32, f32),
    /// Its letters' mean ink half size: how far they sit off the outline.
    half: Vec2,
}

/// A frame's lines being laid along its halves.
struct Layout<'a> {
    halves: &'a Halves,
    outward: bool,
    span: f32,
}

impl Layout<'_> {
    /// The lines of `run` (inked), each over its glyph range, its ends
    /// moved by `span` from their natural places toward its half's
    /// junctions less `clear` (top-left, bottom-right), at the gap that
    /// centers its end inks on its part's ends. A natural line that
    /// reaches into the clearance is slid out of it, keeping its font's
    /// gap, and is pressed only where its half less the clearance is too
    /// short for it: no end comes nearer its junction than the clearance.
    fn lay(
        &self,
        run: &ShapedRun,
        lines: &[(Range<usize>, (f32, f32))],
        clear: [f32; 2],
        span: f32,
    ) -> Vec<Line> {
        let mut out = Vec::with_capacity(lines.len());
        for (k, (range, (na, nb))) in lines.iter().enumerate().take(2) {
            let clusters = clusters_of(&run.glyphs, range.clone());
            let Some((half, natural_gap)) = letters_of(&clusters) else {
                continue;
            };
            let space = clusters
                .iter()
                .filter(|c| c.blank)
                .map(|c| c.advance)
                .fold(0.0_f32, f32::max);
            let walk = line_walk(&self.halves.paths[k], run, self.outward);
            let (ja, jb) = self.halves.junctions[k];
            let (lo, hi) = (ja + clear[0], jb - clear[1]);
            // The natural line slid clear of the clearance, at its
            // font's own gap, where its half less the clearance holds it.
            let reach = |from: f32| -track(&walk, &run.glyphs, &clusters, (from, from), natural_gap).short;
            let (mut na, mut nb) = (*na, *nb);
            if na < lo {
                na = lo;
                nb = lo + reach(lo);
            }
            if nb > hi {
                let mut long = reach(na);
                for _ in 0..SLIDE_ROUNDS {
                    na = hi - long;
                    long = reach(na);
                    if (na + long - hi).abs() <= SLIDE_TOLERANCE {
                        break;
                    }
                }
                if na < lo {
                    na = lo;
                    long = reach(lo);
                }
                nb = (na + long).min(hi);
            }
            // Its ends moved toward the junctions by the span, never into
            // the clearance.
            let sa = (na + (ja - na) * span).max(lo);
            let sb = (nb + (jb - nb) * span).min(hi);
            let short = |g: f32| track(&walk, &run.glyphs, &clusters, (sa, sb), g).short;
            let gap = solve_tracking(short);
            let tracked = track(&walk, &run.glyphs, &clusters, (sa, sb), gap);
            out.push(Line {
                tracked,
                gap,
                natural_gap,
                word: space + 2.0 * gap,
                part: (sa, sb),
                half,
            });
        }
        out
    }

    /// How far apart the lines' end inks are at each junction (top-left,
    /// bottom-right), and how far apart they must be: two of the lines'
    /// mean word gap, or, with `side` (a side ornament's ink extent), room
    /// for one copy and its two joins at the lines' mean natural gap,
    /// whichever is more. A single line is measured against the junction
    /// point on its center line, owing half that.
    fn clearance(&self, lines: &[Line], side: Option<f32>) -> ([f32; 2], [f32; 2]) {
        let count = lines.len().max(1) as f32;
        let word = lines.iter().map(|l| l.word).sum::<f32>() / count;
        let natural = lines.iter().map(|l| l.natural_gap).sum::<f32>() / count;
        let room = side.map_or(0.0, |ink| ink + 2.0 * natural.max(0.0));
        match lines {
            [one, two, ..] => {
                let want = (JUNCTION_WORD_GAPS * word).max(room);
                match (one.tracked.ends, two.tracked.ends) {
                    (Some((a1, b1)), Some((a2, b2))) => ([box_gap(a1, a2), box_gap(b1, b2)], [want; 2]),
                    _ => ([f32::INFINITY; 2], [want; 2]),
                }
            }
            [one] => {
                let want = (0.5 * JUNCTION_WORD_GAPS * word).max(0.5 * room);
                let walk = PathWalk {
                    path: &self.halves.paths[0],
                    line_height: 0.0,
                    outward: self.outward,
                    limit: f32::INFINITY,
                    level: false,
                    ornament: true,
                    baseline: 0.0,
                    push: None,
                };
                let mean = Ink {
                    min: -one.half,
                    max: one.half,
                };
                let (ja, jb) = self.halves.junctions[0];
                let (pa, pb) = (walk.ink_at(ja, mean), walk.ink_at(jb, mean));
                match one.tracked.ends {
                    Some((a, b)) => ([box_gap(a, (pa, pa)), box_gap(b, (pb, pb))], [want; 2]),
                    None => ([f32::INFINITY; 2], [want; 2]),
                }
            }
            [] => ([f32::INFINITY; 2], [0.0; 2]),
        }
    }

    /// The lines laid at the span, each junction's clearance found at the
    /// whole span (ruling R33.1): grown, a few rounds, until the lines'
    /// end inks there are as far apart as [`clearance`](Self::clearance)
    /// wants with each line over its whole half less it. A narrower span
    /// moves the ends toward that, so the ends move monotonically with it.
    fn lay_clear(
        &self,
        run: &ShapedRun,
        lines: &[(Range<usize>, (f32, f32))],
        side: Option<f32>,
    ) -> (Vec<Line>, [f32; 2]) {
        let mut clear = [0.0_f32; 2];
        let mut laid = self.lay(run, lines, clear, 1.0);
        for _ in 0..CLEAR_ROUNDS {
            let (apart, want) = self.clearance(&laid, side);
            let mut grown = false;
            for j in 0..2 {
                let aim = want[j] * CLEAR_MARGIN;
                if apart[j] < aim {
                    // Both lines draw back by the clearance, so their ends
                    // part by about twice it: half the shortfall closes it
                    // from below, never past it.
                    clear[j] += 0.5 * (aim - apart[j]).max(0.25);
                    grown = true;
                }
            }
            if !grown {
                break;
            }
            laid = self.lay(run, lines, clear, 1.0);
        }
        if self.span < 1.0 {
            laid = self.lay(run, lines, clear, self.span);
        }
        (laid, clear)
    }
}

/// The least gap at which `short` (a line's room left, growing smaller as
/// the gap grows) is no longer positive: 0 where the line fills its part
/// at no gap, and at most `2^TRACK_DOUBLINGS`.
fn solve_tracking(short: impl Fn(f32) -> f32) -> f32 {
    // No room even at no gap, or a room that is not a number: no gap.
    if short(0.0).partial_cmp(&0.0) != Some(Ordering::Greater) {
        return 0.0;
    }
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    let mut doublings = 0;
    while short(high) > 0.0 && doublings < TRACK_DOUBLINGS {
        low = high;
        high *= 2.0;
        doublings += 1;
    }
    for _ in 0..TRACK_ROUNDS {
        let mid = 0.5 * (low + high);
        if short(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// Each line's glyphs in `run`: from the first to the last glyph R27's
/// placement gives it, by index.
fn line_ranges(placed: &PlacedRun, lines: &[TracedLine]) -> Vec<Range<usize>> {
    lines
        .iter()
        .take(2)
        .filter_map(|line| {
            let ids = placed.glyphs[line.glyphs.clone()]
                .iter()
                .map(|g| g.glyph as usize);
            Some(ids.clone().min()?..ids.max()? + 1)
        })
        .collect()
}

/// Each line of `run` (inked), over its glyph range, and its natural
/// ends along its half ([`Span::NATURAL`]): the line [`track`]ed at its
/// font's own gap, its first and last ink centers centered on its half's
/// apex — R27's width, evenly tracked. So at the least span the line's
/// gap is its font's own, and every wider span lays it longer.
///
/// Cost: per line, two [`track`]s.
fn natural_ends(
    halves: &Halves,
    run: &ShapedRun,
    ranges: &[Range<usize>],
    outward: bool,
) -> Vec<(Range<usize>, (f32, f32))> {
    let mut out = Vec::with_capacity(ranges.len());
    for (k, range) in ranges.iter().enumerate().take(2) {
        let clusters = clusters_of(&run.glyphs, range.clone());
        let Some(gap) = letters_of(&clusters).map(|(_, gap)| gap) else {
            continue;
        };
        let walk = line_walk(&halves.paths[k], run, outward);
        let apex = halves.apexes[k];
        // How long the line is from the apex, and again from where that
        // centers it: the second is what it is laid over.
        let reach = |from: f32| -track(&walk, &run.glyphs, &clusters, (from, from), gap).short;
        let na = apex - 0.5 * reach(apex);
        out.push((range.clone(), (na, na + reach(na))));
    }
    out
}

/// A line's letters' mean ink half size and their font's own gap — their
/// mean side bearings; `None` without a letter.
fn letters_of(clusters: &[Cluster]) -> Option<(Vec2, f32)> {
    let letters: Vec<(f32, Ink)> = clusters
        .iter()
        .filter(|c| !c.blank)
        .filter_map(|c| c.ink.map(|i| (c.advance, i)))
        .collect();
    if letters.is_empty() {
        return None;
    }
    let count = letters.len() as f32;
    let half = letters.iter().map(|(_, i)| (i.max - i.min) * 0.5).sum::<Vec2>() / count;
    let gap = letters.iter().map(|(a, i)| a - (i.max.x - i.min.x)).sum::<f32>() / count;
    Some((half, gap))
}

/// The walk a line is tracked along on `path`.
fn line_walk<'p>(path: &'p ArcPath, run: &ShapedRun, outward: bool) -> PathWalk<'p> {
    PathWalk {
        path,
        line_height: run.line_height,
        outward,
        limit: f32::INFINITY,
        level: false,
        ornament: true,
        baseline: run.baseline(),
        push: None,
    }
}

/// The frame's side ornament (ruling R33.1): its run (`run` and one
/// shaped copy of the ornament, a line of its own), the ornament's
/// glyphs in it, and its ink's extent at the size the x-height cap sets.
/// `None` without sides, or when the ornament shapes to nothing.
fn side_run(
    run: &ShapedRun,
    sides: &Option<(String, Rgba)>,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> Option<(ShapedRun, Range<usize>, f32)> {
    let (content, color) = sides.as_ref()?;
    let size = run
        .glyphs
        .iter()
        .filter(|g| g.span != FILLER_SPAN)
        .map(|g| g.size)
        .fold(0.0_f32, f32::max);
    if size.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return None;
    }
    let at = |text: &str, size: f32, fs: &mut FontSystemWriteGuard| {
        let style = ResolvedText {
            size: FontSize::Pt(size),
            ..*flow
        };
        let shaped = shape_run(
            &[FlowSpan {
                text,
                style: &style,
                link: None,
            }],
            1.0,
            fs,
        );
        let fsys: &mut FontSystem = fs;
        inked(&shaped, fsys)
            .glyphs
            .iter()
            .filter_map(|g| g.ink.map(|i| i.max.y - i.min.y))
            .fold(0.0_f32, f32::max)
    };
    let x_height = at("x", size, fs);
    let tall = at(content, size, fs);
    let ornament = if tall > x_height && tall > 0.0 && x_height > 0.0 {
        size * x_height / tall
    } else {
        size
    };
    let style = ResolvedText {
        size: FontSize::Pt(ornament),
        color: *color,
        ..*flow
    };
    let fsys: &mut FontSystem = fs;
    let with = reshape(run, 1.0, Some((content.as_str(), &style)), fsys);
    let start = with.glyphs.iter().position(|g| g.span == FILLER_SPAN)?;
    let range = start..with.glyphs.len();
    let fsys: &mut FontSystem = fs;
    let probe = inked(&with, fsys);
    let extent = probe.glyphs[range.clone()]
        .iter()
        .filter_map(|g| g.ink.map(|i| (i.max - i.min).max_element()))
        .fold(0.0_f32, f32::max);
    Some((with, range, extent))
}

/// Fill the arc of `walk`'s path from just past `from` (a text ink box)
/// to just before `to` (another) with copies of the ornament `clusters`
/// (ruling R33.1): the most-nearly `target`-gapped whole number of them,
/// at least one, every gap and both joins equal — the first copy set by
/// the pairwise rule `gap` from `from`'s ink, the gap solved so the last
/// ink is `gap` from `to`'s. The counts weighed are the most that fit
/// at the target gap and one more (whose gap is narrower). Nothing when
/// not even one copy fits.
fn fill_arc(
    walk: &PathWalk<'_>,
    glyphs: &[GlyphMetrics],
    clusters: &[Cluster],
    (from, to): ((Vec2, Vec2), (Vec2, Vec2)),
    target: f32,
    out: &mut Vec<PlacedGlyph>,
) {
    let Some(first) = clusters
        .iter()
        .find(|c| !c.blank)
        .and_then(|c| c.ink.map(|i| (c.advance, i)))
    else {
        return;
    };
    let (advance, ink) = first;
    let (lead, bearings) = (ink.min.x, advance - (ink.max.x - ink.min.x));
    let length = walk.path.length();
    let ca = walk.path.nearest(middle(from));
    let cb = unwrap(walk.path.nearest(middle(to)), ca, length);
    let lay = |n: usize, gap: f32| {
        let mut laying = Laying {
            trail: Trail {
                last: Some((middle(from), 0.5 * (from.1 - from.0), ca)),
                owed: gap - lead,
            },
            ..Laying::default()
        };
        let mut s = ca;
        for _ in 0..n {
            if let Laid::Fits(next) = walk.lay(glyphs, clusters, s, gap - bearings, &mut laying) {
                s = next;
            }
        }
        let end = laying
            .trail
            .last
            .map(|(c, h, at)| (box_gap((c - h, c + h), to), at))
            .unwrap_or((f32::INFINITY, ca));
        (laying, end)
    };
    let solve = |n: usize| -> Option<f32> {
        let fits = |gap: f32| {
            let (_, (end, at)) = lay(n, gap);
            at < cb && end >= gap
        };
        let (mut low, mut high) = (bearings.max(0.0), bearings.max(0.0) + (cb - ca));
        if !fits(low) {
            return None;
        }
        for _ in 0..SIDE_ROUNDS {
            let mid = 0.5 * (low + high);
            if fits(mid) {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some(low)
    };
    // How many copies fit at the target gap, and so which counts can be
    // nearest it: that many, and one fewer.
    let at_target = {
        let mut laying = Laying {
            trail: Trail {
                last: Some((middle(from), 0.5 * (from.1 - from.0), ca)),
                owed: target.max(bearings) - lead,
            },
            ..Laying::default()
        };
        let mut s = ca;
        let mut count = 0;
        while count < MAX_SIDE_COPIES {
            let before = laying.trail;
            if let Laid::Fits(next) =
                walk.lay(glyphs, clusters, s, target.max(bearings) - bearings, &mut laying)
            {
                s = next;
            }
            match laying.trail.last {
                Some((c, h, at)) if at < cb && box_gap((c - h, c + h), to) >= target.max(bearings) => {
                    count += 1
                }
                _ => {
                    laying.trail = before;
                    break;
                }
            }
        }
        count
    };
    let mut best: Option<(usize, f32)> = None;
    for n in at_target.max(1)..=(at_target + 1).max(1) {
        let Some(gap) = solve(n) else { continue };
        if best.is_none_or(|(_, g)| (gap - target).abs() < (g - target).abs()) {
            best = Some((n, gap));
        }
    }
    if best.is_none() {
        best = solve(1).map(|gap| (1, gap));
    }
    if let Some((n, gap)) = best {
        out.extend(lay(n, gap).0.glyphs);
    }
}

/// Place `run` as a text frame around `outline` (rulings R27, R32, R33,
/// R33.1): its two lines as [`place_lines`] splits and sizes them, laid
/// at `layout.span` along the diagonal halves, each junction a break,
/// `layout.sides` closing what the lines leave. The placement lists the
/// first line's glyphs, then the second's, in text order, then the side
/// ornament's copies. Returns the run the placement refers to when it is
/// not `run` — the rescaled run for `Fit`, with the side ornament as a
/// line of its own — and the placement.
///
/// No line is laid tighter than its font while its half less the two
/// clearances holds it at its font's gap: at [`Span::NATURAL`] its gap
/// is its font's own, a natural line that reaches into a junction's
/// clearance slid clear of it rather than pressed, and a wider span only
/// lengthens it. A line R27 sizes is about as long as its level-dominant
/// span, a quarter of the outline or so, and its half less both
/// clearances is about twice that, so only a line too long for its half
/// is pressed, to its half.
///
/// Cost: [`place_lines`]; then per line a bisection of [`track`]s, the
/// junction clearance [`CLEAR_ROUNDS`] of those at most; a side arc a
/// bisection of lays per copy count.
#[allow(clippy::too_many_arguments)] // `place_on_closed`'s, and the trace.
pub(super) fn place_halves(
    run: &ShapedRun,
    outline: &Outline,
    fit: &FitMode,
    layout: &RingLayout,
    outward: bool,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
    trace: Option<&mut Trace>,
) -> (Option<ShapedRun>, PlacedRun) {
    let span = layout.span.get();
    let mut natural_trace = Trace::default();
    let (fitted, natural) = place_lines(run, outline, fit, outward, flow, fs, Some(&mut natural_trace));
    if natural_trace.lines.is_empty() {
        if let Some(trace) = trace {
            *trace = natural_trace;
        }
        return (fitted, natural);
    }
    let halves = Halves::new(outline, table_spacing(run, fit, flow));
    let lay = Layout {
        halves: &halves,
        outward,
        span,
    };
    let base = fitted.clone().unwrap_or_else(|| run.clone());

    // The run as drawn: the text, and the side ornament on a line of its
    // own.
    let (drawn, side) = match side_run(&base, &layout.sides, flow, fs) {
        Some((with, range, extent)) => (with, Some((range, extent))),
        None => (base.clone(), None),
    };
    let fsys: &mut FontSystem = fs;
    let drawn_inked = inked(&drawn, fsys);

    // The lines, from their natural ends.
    let ranges = line_ranges(&natural, &natural_trace.lines);
    let ends = natural_ends(&halves, &drawn_inked, &ranges, outward);
    let (lines, _) = lay.lay_clear(&drawn_inked, &ends, side.as_ref().map(|(_, extent)| *extent));
    let mut text = Vec::new();
    let mut line_ranges = Vec::new();
    for line in &lines {
        line_ranges.push(text.len()..text.len() + line.tracked.glyphs.len());
        text.extend(line.tracked.glyphs.iter().copied());
    }
    let parts: Vec<(f32, f32)> = lines.iter().map(|l| l.part).collect();
    let gaps: Vec<f32> = lines.iter().map(|l| l.gap).collect();

    // The side ornament round the junctions.
    let mut sides = Vec::new();
    if let Some((range, _)) = &side {
        let line_ends: Vec<((Vec2, Vec2), (Vec2, Vec2))> = line_ranges
            .iter()
            .filter_map(|r| {
                let inks: Vec<(Vec2, Vec2)> = text[r.clone()]
                    .iter()
                    .filter_map(|g| ink_box(&drawn_inked, g))
                    .collect();
                Some((*inks.first()?, *inks.last()?))
            })
            .collect();
        let text_inks: Vec<Vec2> = text
            .iter()
            .filter_map(|g| ink_box(&drawn_inked, g).map(|(a, b)| 0.5 * (b - a)))
            .collect();
        let half = text_inks.iter().copied().sum::<Vec2>() / text_inks.len().max(1) as f32;
        let walk = PathWalk {
            path: &halves.paths[0],
            line_height: drawn.line_height,
            outward,
            limit: f32::INFINITY,
            level: false,
            ornament: true,
            baseline: drawn.baseline(),
            push: outward.then_some(half),
        };
        let clusters = clusters_of(&drawn_inked.glyphs, range.clone());
        let target = gaps.iter().sum::<f32>() / gaps.len().max(1) as f32;
        let arcs: Vec<((Vec2, Vec2), (Vec2, Vec2))> = match line_ends.as_slice() {
            [(a1, b1), (a2, b2), ..] => vec![(*a2, *a1), (*b1, *b2)],
            [(a, b)] => vec![(*b, *a)],
            [] => Vec::new(),
        };
        for arc in arcs {
            fill_arc(&walk, &drawn_inked.glyphs, &clusters, arc, target, &mut sides);
        }
    }

    if let Some(trace) = trace {
        trace.lines = line_ranges
            .iter()
            .zip(&parts)
            .enumerate()
            .map(|(k, (glyphs, &(from, to)))| TracedLine {
                half: k,
                glyphs: glyphs.clone(),
                from,
                to,
                cut: natural_trace.lines.get(k).is_some_and(|l| l.cut),
            })
            .collect();
    }
    let changed = fitted.is_some() || side.is_some();
    let mut glyphs = text;
    glyphs.extend(sides);
    let placed = PlacedRun {
        glyphs,
        truncated: natural.truncated,
    };
    (changed.then(|| uninked(drawn)), placed)
}

/// Test seam (#291, ruling R32): where along `outline`'s first half the
/// last ink of `run` (one line, every glyph) ends, laid over the whole
/// half with each of `gaps` — how [`track`]'s length moves with its
/// gap, a zero gap included.
pub(crate) fn track_seam(run: &ShapedRun, outline: &Outline, gaps: &[f32], fs: &mut FontSystem) -> Vec<f32> {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let run = inked(run, fs);
    let clusters = clusters_of(&run.glyphs, 0..run.glyphs.len());
    let walk = PathWalk {
        path: &halves.paths[0],
        line_height: run.line_height,
        outward: true,
        limit: f32::INFINITY,
        level: false,
        ornament: true,
        baseline: run.baseline(),
        push: None,
    };
    let (sa, sb) = halves.junctions[0];
    gaps.iter()
        .map(|&g| {
            let tracked = track(&walk, &run.glyphs, &clusters, (sa, sb), g);
            tracked.ends.map_or(sa, |(_, last)| halves.along(0, middle(last)))
        })
        .collect()
}
