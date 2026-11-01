// SPDX-License-Identifier: MPL-2.0

//! A text frame's two lines along its outline's diagonal halves (#291,
//! rulings R32, R33, R33.1 and R34).
//!
//! The owner, on the frame text: "It should follow the path of its
//! halve, this should be adjustable in the solution. But the point here
//! was to demo the text actually taking the shape of our nodes."
//!
//! **The halves** (ruling R33). The outline is split on its diagonal, at
//! its support points in the directions `(−1, −1)` and `(1, 1)`
//! ([`Outline::support`]): the point where the tangent is at 45° on an
//! ellipse, the corner arc's midpoint on a rounded rectangle — the
//! reading of R33's "in its own aspect" that keeps every horizontal run
//! reading rightward, where the bounding box's diagonal would start the
//! second half leftward on a wide ellipse. The first line takes the
//! upper-right half — from the top-left junction clockwise over the top
//! and down the right side to the bottom-right junction — and the second
//! the lower-left half — from the top-left junction down the left side
//! and along the bottom to the bottom-right one. So every vertical run
//! reads down and every horizontal one reads right, glyphs upright. Each
//! line carries its half with it: a text that opens with a line break
//! has one line, on the second half.
//!
//! **The lines** (rulings R27, R34). R27 parts the text and sizes a
//! `Fit` frame ([`place_lines`]); a `Once` or `Spread` frame's lines are
//! parted as R27 parts them but not cut to its level span
//! ([`line_partition`]).
//!
//! **The span** (ruling R32, [`LineSpan`]). At [`LineSpan::NATURAL`]
//! each line lies at its natural width: tracked at its font's own gap,
//! centered on its half's apex (where R27 centers it) by bisection, and
//! slid along its half where that would reach into a junction's
//! clearance. At [`LineSpan::WHOLE`] each end is at its junction less
//! the clearance; in between, each end moves by arc length that part of
//! the way from natural to whole, so the ends move only outward.
//!
//! **Even tracking.** Over the part it covers, a line is laid with one
//! ink gap `g` between every two neighboring letters, its letters' side
//! bearings left out, and a space's advance plus `g` more between words:
//! a word gap is `2g` plus a space. Each letter after the first is set
//! by the ornament walk's pairwise rule (ruling R31), as far from the
//! last letter's ink as owed, a zero gap included, so the line's length
//! grows with `g` and `g` is solved by bisection: the line's first and
//! last inks centered on its part's ends ([`track`]). A glyph with no
//! outline steps as its box; a line of one letter sits centered on its
//! part.
//!
//! **Each junction is a visible break** (rulings R33.1, R35.1). At every
//! span the two lines' end inks at a junction are at least two word
//! gaps apart, box to box, of the mean of the lines that have one (a
//! line of one word has none: the span stretches its letter gaps
//! alone), and with `sides` one whole copy of the ornament fits between
//! them; two end inks whose outward normals turn more than 135° apart
//! are lines stacked across a thin outline, and need only not touch.
//! Each line keeps its own clearance from each junction, grown round by
//! round among the lines whose drawing back parts the ends there, at
//! whole and then on at the span laid ([`Layout::lay_clear`]).
//!
//! **No line is laid tighter than its font, and no two inks touch**
//! (rulings R34, R35.2). Where the halves do not hold the frame so — a
//! line its half less the clearance cannot hold at its font's gap, a
//! junction left short of a break, two inks touching — the last resorts
//! follow: a `Fit` frame is shrunk, within its bounds; a line held even
//! so by nothing is cut at its end, the placement reporting it; and
//! what is left unheld even so is reported in the trace
//! ([`Unheld`](super::Unheld)), never silent. Every last resort is
//! bounded in work (ruling R35.3): a shrink's probe looks first whether
//! its lines fit at all, and a frame's tracking takes at most
//! [`TRACK_BUDGET`] glyph steps.
//!
//! **Sides.** With `sides`, an ornament closes what the lines leave
//! round each junction: whole copies, at least one and at most
//! [`MAX_SIDE_COPIES`], every gap and both joins to the text equal
//! (ruling R31's lay, seeded by the text's end inks), on the text's own
//! center line, the ornament's ink no taller than the text's x-height.
//! The ornament is shaped into the run as a line of its own, the way a
//! `Once` filler is.

use std::cell::Cell;
use std::cmp::Ordering;
use std::ops::Range;

use glam::Vec2;

use super::lines::{line_partition, place_lines};
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

/// How much further apart a junction's end inks must move in a round of
/// its clearance for the next round to be worth it, in points: less,
/// and the lines can give no more room there.
const PROGRESS: f32 = 1e-3;

/// The least a junction's clearance grows by in a round, in points.
const MIN_GROWTH: f32 = 0.25;

/// How many times a round of the clearance that would press a line is
/// halved before it is taken anyway.
const BACKTRACKS: usize = 6;

/// How many times a shrink's probe halves such a round (ruling R35.3):
/// a probe asks only whether its size can be held, and a round that
/// presses a line after one halving says, near enough, that it cannot.
const PROBE_BACKTRACKS: usize = 1;

/// The most rounds of bisection a natural line is centered on its apex
/// in, and how near equal its two arcs either side must come, in
/// points.
const CENTER_ROUNDS: usize = 40;
const CENTER_TOLERANCE: f32 = 1e-3;

/// How far below its font's own gap a line's solved gap may fall and
/// still have it: the bisection's precision, in points.
const GAP_TOLERANCE: f32 = 1e-4;

/// The least gap a side ornament's copy is laid at, in points: past
/// [`INK_TOLERANCE`], so the pairwise rule always sets a copy clear of
/// the ink before it — an ornament with no side bearings (`─`, `█`)
/// included — rather than where its box would fall.
const JOIN_FLOOR: f32 = 4.0 * INK_TOLERANCE;

/// The rounds of bisection a `Fit` frame is shrunk in, when its halves
/// cannot hold its lines at the size R27 gave it.
const SHRINK_ROUNDS: usize = 12;

/// How much of the clearance of the largest size a shrink has found
/// held a probe at a larger size starts from (ruling R35.3): a clearance
/// grows with the word gap, so with the size — or stays, where a wide
/// space of no size of its own sets it — and the probe's rounds grow it
/// the rest of the way, from below.
const SEED_SHARE: f32 = 0.9;

/// The most glyph steps a frame's tracking may take in all, each glyph
/// a [`track`] lays counting one (ruling R35.3): the bound on its last
/// resorts' work. A frame that reaches it keeps the best it has laid —
/// the largest size its shrink found clean, the cuts made so far — and
/// its trace reports what is left unheld. No frame of a map or a test
/// comes near it; the fuzzers' findings do.
pub(crate) const TRACK_BUDGET: usize = 1 << 21;

/// How far apart the outward normals of two lines' end inks at a
/// junction must turn, as a cosine, for the two to be lines stacked
/// across a thin outline rather than lines meeting there (ruling R35.1):
/// 135°.
const STACKED: f32 = -std::f32::consts::FRAC_1_SQRT_2;

/// How far apart two inks must be not to touch, in points: past
/// [`INK_TOLERANCE`], what the pairwise rule sets inks apart to.
const TOUCH: f32 = INK_TOLERANCE;

/// The most times a frame's lines are cut and the clearance found
/// again: a cut line, stretched over its half at whole, has wider word
/// gaps and so wants more clearance, which a round more settles.
const CUT_ROUNDS: usize = 8;

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

/// How far apart two boxes, `(min, max)` each, are: their distance
/// ([`box_gap`]) where they are apart, and where they overlap how deep,
/// as a negative — the overlap along the axis it is least on, how far
/// one must move to clear the other — so that two inks drawn apart out
/// of an overlap measure the progress (ruling R35.2).
fn separation(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    let apart = (b.0 - a.1).max(a.0 - b.1);
    if apart.x > 0.0 || apart.y > 0.0 {
        apart.max(Vec2::ZERO).length()
    } else {
        apart.x.max(apart.y)
    }
}

/// The center of a box, `(min, max)`.
fn middle(b: (Vec2, Vec2)) -> Vec2 {
    0.5 * (b.0 + b.1)
}

/// `run` with every glyph's [`GlyphMetrics::ink`] measured but a space's:
/// what tracking a line, and its side ornament, step by. A glyph with
/// no outline to measure — a bitmap emoji, a face with no outlines — is
/// inked as its box, its advance by the line height, so a line keeps it
/// and steps it like any other letter.
///
/// Cost: one outline scale per glyph that is not a space.
fn inked(run: &ShapedRun, fs: &mut FontSystem) -> ShapedRun {
    let mut out = run.clone();
    let mut cache = SwashCache::new();
    let laid: Vec<_> = run.layout_glyphs().cloned().collect();
    let (baseline, height) = (run.baseline(), run.line_height);
    for (glyph, laid) in out.glyphs.iter_mut().zip(&laid) {
        if !glyph.blank {
            let boxed = Ink {
                min: Vec2::new(0.0, -baseline),
                max: Vec2::new(glyph.advance.max(0.0), height - baseline),
            };
            glyph.ink =
                Some(glyph_outline_ink(fs, &mut cache, laid).map_or(boxed, |(min, max)| Ink { min, max }));
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

thread_local! {
    /// Test counter (#294): the lines [`track`] has laid on this thread,
    /// what a frame's work is counted in.
    #[cfg(test)]
    pub(crate) static TRACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test counter (#294): the times a frame's text has been built at a
    /// size on this thread — shaped, inked and its lines prepared —
    /// whether or not its clearance was then grown.
    #[cfg(test)]
    pub(crate) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One line laid with one tracking gap ([`track`]).
#[derive(Clone)]
struct Tracked {
    /// Its glyphs, in text order.
    glyphs: Vec<PlacedGlyph>,
    /// How far before its part's end its last ink's center falls, by arc
    /// length: positive with room left, negative past it.
    short: f32,
    /// Its first and last ink boxes.
    ends: Option<((Vec2, Vec2), (Vec2, Vec2))>,
    /// Each inked cluster's ink box, in text order.
    boxes: Vec<InkBox>,
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
    #[cfg(test)]
    TRACKS.with(|count| count.set(count.get() + 1));
    let mut out = Vec::new();
    let mut boxes = Vec::new();
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
        boxes.push((at - half, at + half));
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
        boxes,
    }
}

/// One line of the frame before it is laid: the half it takes, its
/// glyphs and what tracking it needs of them.
#[derive(Clone)]
struct Prepared {
    /// The half it takes: 0 the upper-right, 1 the lower-left.
    half: usize,
    /// Its clusters, in text order.
    clusters: Vec<Cluster>,
    /// Its letters' mean ink half size: how far they sit off the outline.
    size: Vec2,
    /// Its font's own gap: its letters' mean side bearings.
    natural_gap: f32,
    /// Its widest space's advance.
    space: f32,
    /// Its natural ends along its half ([`LineSpan::NATURAL`]): its first
    /// and last ink centers at its font's own gap, centered on its half's
    /// apex.
    natural: (f32, f32),
    /// Whether R27 cut its end.
    cut: bool,
}

/// One line along its half ([`Layout::lay`]).
#[derive(Clone)]
struct Line {
    /// The half it takes.
    half: usize,
    /// Its glyphs, as [`track`] laid them at its gap.
    tracked: Tracked,
    /// Its tracking gap.
    gap: f32,
    /// Its word gap, a space's advance and two gaps: `None` for a line
    /// with no space, which has no word gap.
    word: Option<f32>,
    /// The part of its half it covers.
    part: (f32, f32),
    /// Its letters' mean ink half size.
    size: Vec2,
    /// Whether its half less the clearance was too short to hold it at
    /// its font's own gap, so it was laid tighter.
    pressed: bool,
    /// Whether it has two letters or more, and so a gap to track: a line
    /// of one sits centered on its part, which its clearance moves
    /// nowhere in particular.
    tracks: bool,
}

/// A frame's lines being laid along its halves.
struct Layout<'a> {
    halves: &'a Halves,
    outward: bool,
    /// The glyph steps this frame's tracking has taken
    /// ([`TRACK_BUDGET`]).
    spent: Cell<usize>,
}

/// The lines [`Layout::lay`] has laid in one clearance search, by line,
/// part of its half it may cover, and span, all bit for bit: a round
/// that grows one line's clearance lays the other where it lay.
type LayMemo = FxHashMap<(usize, u32, u32, u32), Line>;

/// One clearance search's state ([`Layout::lay_clear`]): the lines it
/// has laid, and how many times a round that would press a line is
/// halved in it.
struct Search {
    memo: LayMemo,
    backtracks: usize,
}

impl<'a> Layout<'a> {
    fn new(halves: &'a Halves, outward: bool) -> Self {
        Layout {
            halves,
            outward,
            spent: Cell::new(0),
        }
    }
}

impl Layout<'_> {
    /// [`track`], counted against the frame's [`TRACK_BUDGET`].
    fn track(
        &self,
        walk: &PathWalk<'_>,
        glyphs: &[GlyphMetrics],
        clusters: &[Cluster],
        part: (f32, f32),
        g: f32,
    ) -> Tracked {
        self.spent
            .set(self.spent.get().saturating_add(clusters.len().max(1)));
        track(walk, glyphs, clusters, part, g)
    }

    /// Whether the frame's tracking has taken its [`TRACK_BUDGET`].
    fn spent_all(&self) -> bool {
        self.spent.get() >= TRACK_BUDGET
    }

    /// The walk a line on half `k` is tracked along.
    fn walk<'s>(&'s self, k: usize, run: &ShapedRun) -> PathWalk<'s> {
        line_walk(&self.halves.paths[k], run, self.outward)
    }

    /// How long `clusters` are at gap `g` from their first ink center at
    /// `from` to their last, by arc length along half `k`.
    fn reach(&self, k: usize, run: &ShapedRun, clusters: &[Cluster], from: f32, g: f32) -> f32 {
        -self
            .track(&self.walk(k, run), &run.glyphs, clusters, (from, from), g)
            .short
    }

    /// The line of `run` (inked) over `range` on half `k`, ready to lay:
    /// `None` when it holds no glyph that is not a space. Its natural
    /// ends are centered on the apex by bisection: the arc from its
    /// first ink center to the apex equals the arc from the apex to its
    /// last, to [`CENTER_TOLERANCE`].
    ///
    /// Cost: [`CENTER_ROUNDS`] [`track`]s at most.
    fn prepare(&self, run: &ShapedRun, k: usize, range: Range<usize>, cut: bool) -> Option<Prepared> {
        self.prepare_clusters(run, k, clusters_of(&run.glyphs, range), cut)
    }

    /// [`prepare`](Self::prepare) over `clusters`.
    fn prepare_clusters(
        &self,
        run: &ShapedRun,
        k: usize,
        clusters: Vec<Cluster>,
        cut: bool,
    ) -> Option<Prepared> {
        let (size, natural_gap) = letters_of(&clusters)?;
        let space = clusters
            .iter()
            .filter(|c| c.blank)
            .map(|c| c.advance)
            .fold(0.0_f32, f32::max);
        let apex = self.halves.apexes[k];
        let reach = |from: f32| self.reach(k, run, &clusters, from, natural_gap);
        // Off-center by `before - after`: positive while the start is too
        // far back. It falls as the start moves on, the line's length
        // changing only with the bend of the path under it.
        let miss = |na: f32| (apex - na) - (na + reach(na) - apex);
        let whole = reach(apex).max(0.0);
        let (mut low, mut high) = (apex - whole, apex);
        let mut na = apex - 0.5 * whole;
        for _ in 0..CENTER_ROUNDS {
            na = 0.5 * (low + high);
            let m = miss(na);
            if m.abs() <= CENTER_TOLERANCE {
                break;
            }
            // A miss that is not a number reads as too far back: the
            // start moves on, toward the apex, where the line is no
            // longer than it was.
            if m.partial_cmp(&0.0) == Some(Ordering::Less) {
                high = na;
            } else {
                low = na;
            }
        }
        Some(Prepared {
            half: k,
            natural: (na, na + reach(na)),
            clusters,
            size,
            natural_gap,
            space,
            cut,
        })
    }

    /// The part of half `half` a line may cover with `clear` kept at
    /// each junction (top-left, bottom-right): `(lo, hi)`, never
    /// inverted. Clearances that together pass the whole half meet where
    /// they divide it in their ratio: the line is then a point, pressed.
    fn room(&self, half: usize, clear: [f32; 2]) -> (f32, f32) {
        let (ja, jb) = self.halves.junctions[half];
        let (lo, hi) = (ja + clear[0], jb - clear[1]);
        if lo <= hi {
            (lo, hi)
        } else {
            let mid = ja + (jb - ja) * clear[0] / (clear[0] + clear[1]).max(f32::MIN_POSITIVE);
            (mid, mid)
        }
    }

    /// The natural line `p` slid clear of the clearance, at its font's
    /// own gap, within `(lo, hi)` when that holds it: slid on from the
    /// top-left clearance, slid back from the bottom-right one. A line
    /// longer than `hi − lo` at its gap starts at `lo` and ends at `hi`.
    fn slid(&self, run: &ShapedRun, p: &Prepared, (lo, hi): (f32, f32)) -> (f32, f32) {
        let reach = |from: f32| self.reach(p.half, run, &p.clusters, from, p.natural_gap);
        let (mut na, mut nb) = p.natural;
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
        (na, nb)
    }

    /// The lines of `run` (inked), each moved by `span` from its natural
    /// place (slid clear of the clearance, [`slid`](Self::slid)) to its
    /// whole half less its `clear` (top-left, bottom-right), by arc length at
    /// each end (ruling R33), and laid at the gap that centers its end
    /// inks on its part's ends.
    ///
    /// A line laid before in this search with the same part and span
    /// is read from `memo`: the line it would lay, to the bit.
    fn lay(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        clear: Clear,
        span: f32,
        memo: &mut LayMemo,
    ) -> Vec<Line> {
        lines
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let room = self.room(p.half, clear[p.half]);
                let key = (i, room.0.to_bits(), room.1.to_bits(), span.to_bits());
                if let Some(line) = memo.get(&key) {
                    return line.clone();
                }
                let line = self.lay_one(run, p, room, span);
                memo.insert(key, line.clone());
                line
            })
            .collect()
    }

    /// [`lay`](Self::lay)'s line `p` over `(lo, hi)`.
    fn lay_one(&self, run: &ShapedRun, p: &Prepared, (lo, hi): (f32, f32), span: f32) -> Line {
        let walk = self.walk(p.half, run);
        let (na, nb) = self.slid(run, p, (lo, hi));
        // `lo ≤ na` and `nb ≤ hi` after the slide, so each end
        // moves only outward as the span grows.
        let sa = na + (lo - na) * span;
        let sb = nb + (hi - nb) * span;
        // A line of one letter has no gap to track: it sits at
        // its font's gap, centered on its part.
        let letters = p.clusters.iter().filter(|c| !c.blank).count();
        let (gap, tracked) = if letters < 2 {
            let mid = 0.5 * (sa + sb);
            let gap = p.natural_gap;
            (gap, self.track(&walk, &run.glyphs, &p.clusters, (mid, mid), gap))
        } else {
            let short = |g: f32| self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), g).short;
            let gap = solve_tracking(short);
            let tracked = self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), gap);
            if gap < p.natural_gap - GAP_TOLERANCE {
                // A part its line fits at its font's gap is not one that
                // presses it, where the slide that set it fell short of
                // its end round a bend (ruling R35.2): it is laid at the
                // font's gap from its start.
                let natural = self.track(&walk, &run.glyphs, &p.clusters, (sa, sb), p.natural_gap);
                if natural.short >= -SLIDE_TOLERANCE {
                    (p.natural_gap, natural)
                } else {
                    (gap, tracked)
                }
            } else {
                (gap, tracked)
            }
        };
        Line {
            half: p.half,
            tracked,
            gap,
            word: (p.space > 0.0).then_some(p.space + 2.0 * gap),
            part: (sa, sb),
            size: p.size,
            pressed: letters >= 2 && gap < p.natural_gap - GAP_TOLERANCE,
            tracks: letters >= 2,
        }
    }

    /// The outward normal of the outline where line `l`'s end ink at
    /// junction `j` (top-left 0, bottom-right 1) is set.
    fn normal(&self, l: &Line, j: usize) -> Vec2 {
        let path = &self.halves.paths[l.half];
        let s = if j == 0 {
            l.part.0
        } else {
            l.part.1 - l.tracked.short
        };
        path.outward(path.at(s).1)
    }

    /// How far apart the lines' end inks are at each junction (top-left,
    /// bottom-right), box to box, how far apart they must be — two of the lines' mean
    /// word gap (ruling R33.1), over the lines that have one: a line of
    /// one word has no word gap to count, the span stretching its letter
    /// gaps alone; with no word gap on any line, two of a line's letter
    /// gaps stand for one; and for two end inks whose outward normals
    /// turn more than 135° apart, lines stacked across a thin outline
    /// rather than lines meeting, only not to touch (ruling R35.1) — and,
    /// for each line, how much drawing its
    /// end back at that junction parts the two: the cosine between the
    /// way its end moves as its clearance grows and the way away from the
    /// other end. A single line is measured against the junction point
    /// on its own center line, owing half that.
    fn apart(&self, lines: &[Line]) -> Junctions {
        let words: Vec<f32> = lines.iter().filter_map(|l| l.word).collect();
        let word = if words.is_empty() {
            lines.iter().map(|l| 2.0 * l.gap).sum::<f32>() / lines.len().max(1) as f32
        } else {
            words.iter().sum::<f32>() / words.len() as f32
        };
        // Which way line `l`'s end at junction `j` moves as its
        // clearance there grows: on along its half at the top-left, back
        // at the bottom-right.
        let retreat = |l: &Line, j: usize| {
            let path = &self.halves.paths[l.half];
            let (s, sign) = if j == 0 { (l.part.0, 1.0) } else { (l.part.1, -1.0) };
            path.at(s).1 * sign
        };
        // Ends that touch have no way apart to measure against: each
        // line's drawing back parts them, as at the junction itself.
        let helps = |l: &Line, j: usize, own: (Vec2, Vec2), other: (Vec2, Vec2)| {
            if !l.tracks {
                0.0
            } else if box_gap(own, other) > 0.0 {
                retreat(l, j).dot((middle(own) - middle(other)).normalize_or_zero())
            } else {
                1.0
            }
        };
        let mut out = Junctions {
            apart: [f32::INFINITY; 2],
            depth: [f32::INFINITY; 2],
            want: [0.0; 2],
            helps: [[0.0; 2]; 2],
        };
        match lines {
            [one, two, ..] => {
                out.want = [JUNCTION_WORD_GAPS * word; 2];
                if let (Some((a1, b1)), Some((a2, b2))) = (one.tracked.ends, two.tracked.ends) {
                    for (j, (e1, e2)) in [(a1, a2), (b1, b2)].into_iter().enumerate() {
                        if self.normal(one, j).dot(self.normal(two, j)) < STACKED {
                            out.want[j] = TOUCH;
                        }
                        out.apart[j] = box_gap(e1, e2);
                        out.depth[j] = separation(e1, e2);
                        out.helps[one.half][j] = helps(one, j, e1, e2);
                        out.helps[two.half][j] = helps(two, j, e2, e1);
                    }
                }
            }
            [one] => {
                out.want = [0.5 * JUNCTION_WORD_GAPS * word; 2];
                let walk = PathWalk {
                    path: &self.halves.paths[one.half],
                    memo: None,
                    line_height: 0.0,
                    outward: self.outward,
                    limit: f32::INFINITY,
                    level: false,
                    ornament: true,
                    baseline: 0.0,
                    push: None,
                };
                let mean = Ink {
                    min: -one.size,
                    max: one.size,
                };
                let (ja, jb) = self.halves.junctions[one.half];
                let points = [walk.ink_at(ja, mean), walk.ink_at(jb, mean)];
                if let Some((a, b)) = one.tracked.ends {
                    for (j, (e, p)) in [(a, points[0]), (b, points[1])].into_iter().enumerate() {
                        out.apart[j] = box_gap(e, (p, p));
                        out.depth[j] = separation(e, (p, p));
                        out.helps[one.half][j] = helps(one, j, e, (p, p));
                    }
                }
            }
            [] => {}
        }
        out
    }
}

/// Each junction's measure ([`Layout::apart`]): its lines' end inks'
/// distance, the distance wanted, and how much each half's line parts
/// them by drawing back.
struct Junctions {
    apart: [f32; 2],
    /// [`apart`](Self::apart) where the inks are apart, and how deep
    /// they overlap, as a negative, where they do ([`separation`]): what
    /// a round's progress is measured in, so that two inks drawn out of
    /// an overlap show it (ruling R35.2).
    depth: [f32; 2],
    want: [f32; 2],
    /// `[half][junction]`.
    helps: [[f32; 2]; 2],
}

/// The clearance each half's line keeps from each junction, by arc
/// length: `[half][junction]`, top-left then bottom-right.
type Clear = [[f32; 2]; 2];

/// An ink box on the canvas, `(min, max)`.
type InkBox = (Vec2, Vec2);
/// A frame's side ornament as laid round a junction (ruling R33.1): the
/// walk it is laid along — the first half's path, its inks pushed out
/// by the text's mean ink so they sit on the text's center line — and
/// one copy's clusters.
struct SideOrnament<'a> {
    walk: PathWalk<'a>,
    glyphs: &'a [GlyphMetrics],
    clusters: Vec<Cluster>,
}

impl<'a> SideOrnament<'a> {
    /// The side ornament of `run` (inked) over `range`, laid round the
    /// halves of `layout` beside text whose letters' mean ink half size
    /// is `size`.
    fn new(layout: &'a Layout<'_>, run: &'a ShapedRun, range: Range<usize>, size: Vec2) -> SideOrnament<'a> {
        SideOrnament {
            walk: PathWalk {
                push: layout.outward.then_some(size),
                ..line_walk(&layout.halves.paths[0], run, layout.outward)
            },
            glyphs: &run.glyphs,
            clusters: clusters_of(&run.glyphs, range),
        }
    }

    /// The arc of this side between two text inks, `None` without an
    /// ink in the ornament.
    fn arc(&self, from: (Vec2, Vec2), to: (Vec2, Vec2)) -> Option<SideArc<'_>> {
        SideArc::new(self, from, to)
    }

    /// About how long one copy and its two joins are along a path:
    /// what a junction's clearance grows by while it holds none.
    fn room(&self) -> f32 {
        let advance: f32 = self.clusters.iter().map(|c| c.advance).sum();
        let floor = self.arc_floor().unwrap_or(0.0);
        advance + 2.0 * floor
    }

    /// The least gap a copy is laid at: its first ink's side bearings,
    /// never less than [`JOIN_FLOOR`].
    fn arc_floor(&self) -> Option<f32> {
        let (advance, ink) = self
            .clusters
            .iter()
            .find(|c| !c.blank)
            .and_then(|c| c.ink.map(|i| (c.advance, i)))?;
        Some((advance - (ink.max.x - ink.min.x)).max(JOIN_FLOOR))
    }
}

/// The arc of a side between the text ink it starts past (`from`) and
/// the one it ends before (`to`), along the side's path.
struct SideArc<'s> {
    side: &'s SideOrnament<'s>,
    from: (Vec2, Vec2),
    to: (Vec2, Vec2),
    /// The first ink's left bearing and both bearings.
    lead: f32,
    bearings: f32,
    /// The least gap a copy is laid at ([`SideOrnament::arc_floor`]).
    floor: f32,
    /// Where `from`'s and `to`'s centers lie along the path, `cb` past
    /// `ca`.
    ca: f32,
    cb: f32,
}

impl<'s> SideArc<'s> {
    fn new(side: &'s SideOrnament<'s>, from: (Vec2, Vec2), to: (Vec2, Vec2)) -> Option<Self> {
        let (advance, ink) = side
            .clusters
            .iter()
            .find(|c| !c.blank)
            .and_then(|c| c.ink.map(|i| (c.advance, i)))?;
        let path = side.walk.path;
        let ca = path.nearest(middle(from));
        let cb = unwrap(path.nearest(middle(to)), ca, path.length());
        Some(SideArc {
            side,
            from,
            to,
            lead: ink.min.x,
            bearings: advance - (ink.max.x - ink.min.x),
            floor: side.arc_floor()?,
            ca,
            cb,
        })
    }

    /// `n` copies laid at `gap`: the first by the pairwise rule `gap`
    /// from `from`'s ink, each next `gap` from the last. Returns the
    /// laying, and how far the last ink is from `to` and where along
    /// the path its center lies.
    fn lay(&self, n: usize, gap: f32) -> (Laying, (f32, f32)) {
        let mut laying = Laying {
            trail: Trail {
                last: Some((middle(self.from), 0.5 * (self.from.1 - self.from.0), self.ca)),
                owed: gap - self.lead,
            },
            ..Laying::default()
        };
        let mut s = self.ca;
        for _ in 0..n {
            if let Laid::Fits(next) = self.side.walk.lay(
                self.side.glyphs,
                &self.side.clusters,
                s,
                gap - self.bearings,
                &mut laying,
            ) {
                s = next;
            }
        }
        let end = laying
            .trail
            .last
            .map(|(c, h, at)| (box_gap((c - h, c + h), self.to), at))
            .unwrap_or((f32::INFINITY, self.ca));
        (laying, end)
    }

    /// Whether `n` copies at `gap` fit: the last ink's center short of
    /// `to`'s and its ink at least `gap` from `to`'s. A miss that is not
    /// a number does not fit.
    fn fits(&self, n: usize, gap: f32) -> bool {
        let (_, (end, at)) = self.lay(n, gap);
        at < self.cb && end >= gap
    }

    /// The widest gap at which `n` copies fit with every gap and both
    /// joins equal, `None` when not even the floor's does.
    fn solve(&self, n: usize) -> Option<f32> {
        let (mut low, mut high) = (self.floor, self.floor + (self.cb - self.ca).max(0.0));
        if !self.fits(n, low) {
            return None;
        }
        for _ in 0..SIDE_ROUNDS {
            let mid = 0.5 * (low + high);
            if self.fits(n, mid) {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some(low)
    }

    /// How many copies fit at `target` (at least the floor) in a row
    /// from `from`, to [`MAX_SIDE_COPIES`].
    fn count_at(&self, target: f32) -> usize {
        let gap = target.max(self.floor);
        let mut laying = Laying {
            trail: Trail {
                last: Some((middle(self.from), 0.5 * (self.from.1 - self.from.0), self.ca)),
                owed: gap - self.lead,
            },
            ..Laying::default()
        };
        let mut s = self.ca;
        let mut count = 0;
        while count < MAX_SIDE_COPIES {
            let before = laying.trail;
            if let Laid::Fits(next) = self.side.walk.lay(
                self.side.glyphs,
                &self.side.clusters,
                s,
                gap - self.bearings,
                &mut laying,
            ) {
                s = next;
            }
            match laying.trail.last {
                Some((c, h, at)) if at < self.cb && box_gap((c - h, c + h), self.to) >= gap => count += 1,
                _ => {
                    laying.trail = before;
                    break;
                }
            }
        }
        count
    }

    /// Fill the arc (ruling R33.1): the whole number of copies whose
    /// equal gap is nearest `target`, at least one and at most
    /// [`MAX_SIDE_COPIES`] or as many as `budget` glyphs hold — weighing
    /// the most that fit at the target and one more, whose gap is
    /// narrower — every gap and both joins equal. Nothing when not even
    /// one copy fits, or the budget holds none.
    fn fill(&self, target: f32, budget: usize, out: &mut Vec<PlacedGlyph>) {
        let per_copy = self
            .side
            .clusters
            .iter()
            .map(|c| c.glyphs.len())
            .sum::<usize>()
            .max(1);
        let most = MAX_SIDE_COPIES.min(budget / per_copy);
        if most == 0 {
            return;
        }
        let at_target = self.count_at(target);
        let mut best: Option<(usize, f32)> = None;
        for n in at_target.clamp(1, most)..=(at_target + 1).clamp(1, most) {
            let Some(gap) = self.solve(n) else { continue };
            if best.is_none_or(|(_, g)| (gap - target).abs() < (g - target).abs()) {
                best = Some((n, gap));
            }
        }
        if best.is_none() {
            best = self.solve(1).map(|gap| (1, gap));
        }
        if let Some((n, gap)) = best {
            out.extend(self.lay(n, gap).0.glyphs);
        }
    }
}

/// The text inks each arc of a side runs between, and which junctions
/// (top-left, bottom-right) the arc lies round: with two lines one arc
/// per junction; with one, one arc round both, over the half it leaves.
fn side_arcs(lines: &[Line]) -> Vec<([bool; 2], InkBox, InkBox)> {
    let ends: Vec<(usize, (InkBox, InkBox))> = lines
        .iter()
        .filter_map(|l| Some((l.half, l.tracked.ends?)))
        .collect();
    match ends.as_slice() {
        [(_, (a1, b1)), (_, (a2, b2)), ..] => vec![([true, false], *a2, *a1), ([false, true], *b1, *b2)],
        // The first half's line leaves the second: clockwise from its
        // last ink round to its first.
        [(0, (a, b))] => vec![([true, true], *b, *a)],
        // The second half's line leaves the first: clockwise from its
        // first ink over the top to its last.
        [(_, (a, b))] => vec![([true, true], *a, *b)],
        [] => Vec::new(),
    }
}

/// The lines' letters' mean ink half size: what pushes a side ornament
/// onto the text's center line.
fn mean_size(lines: &[Line]) -> Vec2 {
    lines.iter().map(|l| l.size).sum::<Vec2>() / lines.len().max(1) as f32
}

impl Layout<'_> {
    /// Whether junction `j` (top-left 0, bottom-right 1) of `laid` is a
    /// break (ruling R33.1): the lines' end inks there at least as far
    /// apart as [`apart`](Self::apart) wants — a distance that is not a
    /// number is not — and, with `side` (the ornament's run and glyphs),
    /// one whole copy fitting between them at its own floor gap: the
    /// test [`SideArc::solve`] makes, so the two cannot disagree. With
    /// it, the junction's measure.
    fn met(&self, laid: &[Line], side: Option<(&ShapedRun, &Range<usize>)>, j: usize) -> (bool, Junctions) {
        let measured = self.apart(laid);
        let words = measured.apart[j] >= measured.want[j] * CLEAR_MARGIN;
        let copy = side.is_none_or(|(drawn, range)| {
            let side = SideOrnament::new(self, drawn, range.clone(), mean_size(laid));
            side_arcs(laid)
                .into_iter()
                .filter(|(at, _, _)| at[j])
                .all(|(_, from, to)| side.arc(from, to).is_none_or(|arc| arc.solve(1).is_some()))
        });
        (words && copy, measured)
    }

    /// The lines laid at `span`, each line's clearance from each
    /// junction grown from `seed` at the whole span (ruling R33.1) and
    /// then, below whole, grown on at `span` until each junction is a
    /// break there too (ruling R35.1): so a line at a narrower span lies
    /// within where it lies at whole. Returns the lines, their clearance,
    /// and whether each junction (top-left, bottom-right) is a break.
    fn lay_clear(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        seed: Clear,
        backtracks: usize,
    ) -> Cleared {
        let mut search = Search {
            memo: LayMemo::default(),
            backtracks,
        };
        let whole = self.grow(run, lines, side, 1.0, seed, &mut search);
        if span >= 1.0 {
            return whole;
        }
        self.grow(run, lines, side, span, whole.clear, &mut search)
    }

    /// The lines laid at `span` with each line's clearance grown from
    /// `clear` until each junction is a break ([`met`](Self::met)).
    ///
    /// A junction's clearance grows, round by round, while it is not a
    /// break ([`met`](Self::met)). Each round shares what is missing —
    /// the words' shortfall, or for a copy of the side ornament the room
    /// along the path it lacks — among the lines whose drawing back
    /// parts their ends there, by how much it does, measured afresh each
    /// round: where both halves run on side by side, as on a long flat
    /// outline, only the line whose end moves away is drawn back.
    /// Drawing a line back narrows its tracking, and so the word gap the
    /// break is measured in: a round that would press a line it left
    /// unpressed is halved until it does not ([`BACKTRACKS`] times at
    /// most), so the rounds close on the least clearance from below
    /// rather than overshoot. A junction stops growing when no line's
    /// drawing back parts its ends, or a round leaves them no further
    /// apart — below whole, a round that moved them: a natural line's end
    /// does not move at all until the clearance reaches it.
    fn grow(
        &self,
        run: &ShapedRun,
        lines: &[Prepared],
        side: Option<(&ShapedRun, &Range<usize>)>,
        span: f32,
        mut clear: Clear,
        search: &mut Search,
    ) -> Cleared {
        let backtracks = search.backtracks;
        let memo = &mut search.memo;
        let mut laid = self.lay(run, lines, clear, span, memo);
        // Each junction's measure before the round that grew it, if the
        // last round did: a junction is judged by what its own growth did.
        let mut before = [f32::NEG_INFINITY; 2];
        let mut parts_before: Vec<(f32, f32)> = Vec::new();
        let mut stuck = [false; 2];
        let mut met_now: Option<[bool; 2]> = None;
        for _ in 0..CLEAR_ROUNDS {
            if self.spent_all() {
                break;
            }
            let mut grow: Clear = [[0.0; 2]; 2];
            let mut grown = false;
            let mut measured_depth = [f32::NEG_INFINITY; 2];
            let mut met_here = [false; 2];
            let parts: Vec<(f32, f32)> = laid.iter().map(|l| l.part).collect();
            // Below whole, a round that moved no end proves nothing.
            let moved = span >= 1.0
                || parts.len() != parts_before.len()
                || parts
                    .iter()
                    .zip(&parts_before)
                    .any(|(a, b)| (a.0 - b.0).abs() > PROGRESS || (a.1 - b.1).abs() > PROGRESS);
            for j in 0..2 {
                let (met, measured) = self.met(&laid, side, j);
                met_here[j] = met;
                measured_depth[j] = measured.depth[j];
                if met || stuck[j] {
                    continue;
                }
                let apart = measured.apart[j];
                let depth = measured.depth[j];
                if moved && depth.is_finite() && before[j].is_finite() && depth <= before[j] + PROGRESS {
                    stuck[j] = true;
                    continue;
                }
                let helps = [0, 1].map(|k| measured.helps[k][j].max(0.0));
                let total = helps[0] + helps[1];
                if total.partial_cmp(&0.0) != Some(Ordering::Greater) {
                    stuck[j] = true;
                    continue;
                }
                let aim = measured.want[j] * CLEAR_MARGIN;
                // Ends whose distance is not a number are short of it.
                let shortfall = if matches!(apart.partial_cmp(&aim), None | Some(Ordering::Less)) {
                    aim - apart
                } else {
                    // Room for a copy of the side ornament is room along
                    // the path between the end inks: the arcs from the
                    // junction to each line's end ink, less the inks'
                    // own halves.
                    let room = side.map_or(0.0, |(drawn, range)| {
                        SideOrnament::new(self, drawn, range.clone(), mean_size(&laid)).room()
                    });
                    let arc: f32 = laid
                        .iter()
                        .map(|l| {
                            let (ja, jb) = self.halves.junctions[l.half];
                            let from = if j == 0 { l.part.0 - ja } else { jb - l.part.1 };
                            from - l.size.x
                        })
                        .sum();
                    // The arc measured on the lines' paths is not quite
                    // the room the ornament's own walk finds: while the
                    // copy still does not fit, it grows by a quarter of a
                    // copy at the least.
                    (room - if laid.len() > 1 { arc } else { apart }).max(0.25 * room)
                };
                let step = shortfall.max(MIN_GROWTH);
                for k in 0..2 {
                    // Two lines that both draw back part by about the
                    // sum: each gives its share.
                    grow[k][j] = step * helps[k] / total.max(1.0);
                }
                grown = true;
            }
            for j in 0..2 {
                before[j] = if grow[0][j] > 0.0 || grow[1][j] > 0.0 {
                    measured_depth[j]
                } else {
                    f32::NEG_INFINITY
                };
            }
            parts_before = parts;
            met_now = Some(met_here);
            if !grown {
                break;
            }
            let pressed: Vec<bool> = laid.iter().map(|l| l.pressed).collect();
            let mut scale = 1.0;
            for _ in 0..=backtracks {
                let mut next = clear;
                for k in 0..2 {
                    for j in 0..2 {
                        next[k][j] += grow[k][j] * scale;
                    }
                }
                let tried = self.lay(run, lines, next, span, memo);
                let presses = tried.iter().zip(&pressed).any(|(l, was)| l.pressed && !was);
                if !presses || scale < 0.5_f32.powi(backtracks as i32) * 1.5 {
                    clear = next;
                    laid = tried;
                    met_now = None;
                    break;
                }
                scale *= 0.5;
            }
        }
        let met = met_now.unwrap_or_else(|| [0, 1].map(|j| self.met(&laid, side, j).0));
        Cleared {
            lines: laid,
            clear,
            met,
        }
    }
}

/// A frame's lines laid with their clearance ([`Layout::lay_clear`]).
struct Cleared {
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction, top-left and bottom-right, is a break.
    met: [bool; 2],
}

/// Whether two of `lines`' inks touch (ruling R35.2): any two of
/// different lines, or of one line but not neighbors — two neighbors
/// are as far apart as the line's tracking sets them, a font with no
/// side bearings' zero gap included. Sorted by their left edges, each
/// box is held only against those that start before it ends.
///
/// Cost: O(n log n) in the lines' inks, and a pair per two that share
/// some of their width.
fn overlapping(lines: &[Line]) -> bool {
    let mut boxes: Vec<(usize, usize, InkBox)> = lines
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.tracked.boxes.iter().enumerate().map(move |(i, b)| (k, i, *b)))
        .collect();
    boxes.sort_by(|a, b| a.2 .0.x.total_cmp(&b.2 .0.x));
    for (n, (k, i, a)) in boxes.iter().enumerate() {
        for (l, j, b) in &boxes[n + 1..] {
            // Every box after starts no sooner; one that is not a number
            // ends the scan.
            if !b.0.x.le(&(a.1.x + TOUCH)) {
                break;
            }
            if k == l && i.abs_diff(*j) <= 1 {
                continue;
            }
            if box_gap(*a, *b) <= TOUCH {
                return true;
            }
        }
    }
    false
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
        memo: None,
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
/// shaped copy of the ornament at the size the x-height cap sets, a line
/// of its own) and the ornament's glyphs in it. `None` without sides,
/// or when the ornament shapes to nothing.
fn side_run(
    run: &ShapedRun,
    sides: &Option<(String, Rgba)>,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> Option<(ShapedRun, Range<usize>)> {
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
    Some((with, range))
}

impl Layout<'_> {
    /// Line `p` cut at its end to the longest prefix of whole clusters
    /// that its half less `clear` holds at its font's own gap — its
    /// trailing spaces left out, at least its first cluster kept — or
    /// `None` when it holds the whole line (ruling R34: a `Once` frame
    /// never cuts text its half holds).
    ///
    /// Cost: a bisection of [`track`]s over the line's clusters.
    fn cut(&self, run: &ShapedRun, p: &Prepared, clear: [f32; 2]) -> Option<Prepared> {
        let (lo, hi) = self.room(p.half, clear);
        let fits = |k: usize| {
            let kept = trimmed(&p.clusters[..k]);
            kept.is_empty() || self.reach(p.half, run, kept, lo, p.natural_gap) <= hi - lo + SLIDE_TOLERANCE
        };
        let n = p.clusters.len();
        if fits(n) {
            return None;
        }
        // `fits` holds for no clusters and fails for all of them; the
        // longest prefix it holds for lies between.
        let (mut held, mut over) = (0, n);
        while over - held > 1 {
            let mid = held + (over - held) / 2;
            if fits(mid) {
                held = mid;
            } else {
                over = mid;
            }
        }
        let kept = trimmed(&p.clusters[..held.max(1)]).to_vec();
        let kept = if kept.is_empty() {
            p.clusters[..1].to_vec()
        } else {
            kept
        };
        // Nothing left to cut: a line of one cluster its half cannot
        // hold is laid pressed.
        if kept.len() == n {
            return None;
        }
        self.prepare_clusters(run, p.half, kept, true)
    }
}

/// A frame's text laid at one size ([`place_halves`]).
struct Built {
    /// The run as drawn, inked.
    drawn: ShapedRun,
    /// The side ornament's glyphs in it.
    side: Option<Range<usize>>,
    prepared: Vec<Prepared>,
    lines: Vec<Line>,
    clear: Clear,
    /// Whether each junction is a break ([`Layout::met`]).
    met: [bool; 2],
    /// Whether two of its inks touch ([`overlapping`]).
    overlap: bool,
}

/// What a frame's `lines` leave unheld (ruling R35.2), with whether
/// each junction is a break and whether two inks touch: a line its half
/// less the clearance cannot hold at its font's gap, a junction left
/// short of a break, two inks touching.
fn unheld_of(lines: &[Line], met: [bool; 2], overlap: bool) -> Unheld {
    Unheld {
        pressed: lines.iter().filter(|l| l.pressed).map(|l| l.half).collect(),
        junctions: [0, 1].into_iter().filter(|&j| !met[j]).collect(),
        overlap,
        sides: Vec::new(),
    }
}

impl Built {
    /// Whether its halves hold it: nothing left unheld (ruling R35.2).
    fn holds(&self) -> bool {
        unheld_of(&self.lines, self.met, self.overlap).is_empty()
    }

    /// Its clearance as a seed for the text at a larger size
    /// ([`SEED_SHARE`] of it).
    fn seed(&self) -> Clear {
        self.clear.map(|half| half.map(|c| c * SEED_SHARE))
    }
}

/// The ink box on the canvas of `glyph`, placed from `run` (inked):
/// `None` for one with no ink, a space.
fn glyph_box(run: &ShapedRun, glyph: &PlacedGlyph) -> Option<InkBox> {
    let ink = run.glyphs.get(glyph.glyph as usize)?.ink?;
    let pen = glyph.pos + Vec2::new(0.0, run.baseline());
    Some((pen + ink.min, pen + ink.max))
}

/// `clusters` without their trailing spaces.
fn trimmed(clusters: &[Cluster]) -> &[Cluster] {
    let end = clusters.iter().rposition(|c| !c.blank).map_or(0, |i| i + 1);
    &clusters[..end]
}

/// Place `run` as a text frame around `outline` (rulings R27, R32, R33,
/// R33.1, R34): its two lines laid at `layout.span` along the diagonal
/// halves, each junction a break, `layout.sides` closing what the lines
/// leave. The placement lists the first half's line's glyphs, then the
/// second's, in text order, then the side ornament's copies. Returns the
/// run the placement refers to when it is not `run` — the rescaled run
/// for `Fit`, with the side ornament as a line of its own — and the
/// placement.
///
/// R27 parts the text and sizes a `Fit` frame ([`place_lines`]); its
/// lines, cut where R27 cut them at `min`, are laid here. A `Once` or
/// `Spread` frame is laid at its own size, its lines parted as R27 parts
/// them ([`line_partition`]) but never cut to R27's level span: a line
/// is cut at its end only where its half less the clearance cannot hold
/// it at its font's own gap (ruling R34), and the placement then
/// reports [`PlacedRun::truncated`]. A `Once` filler is not drawn: the
/// resolver gives it to the frame as its `sides` when `sides` is unset
/// ([`resolve_ring`](crate::document::resolve::resolve_ring)).
///
/// Cost: [`place_lines`]; per line a bisection of [`track`]s to center
/// it and one per lay; the junction clearance [`CLEAR_ROUNDS`] rounds at
/// most, at whole and again below it, each with a one-copy solve per
/// side arc; a shrink [`SHRINK_ROUNDS`] probes, each one [`track`] a line
/// and, where they fit, a clearance search; a cut a bisection of
/// [`track`]s more and the clearance again, [`CUT_ROUNDS`] times at
/// most; a side arc a bisection of lays per copy count weighed. In all,
/// at most [`TRACK_BUDGET`] glyph steps of tracking, and a few searches'
/// worth past it.
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
    // An outline whose halves cannot be measured — one with a side that
    // is not a number — keeps R27's placement, which holds every
    // position finite there. (`place_on_closed` shrinks an outline too
    // large for its length to be an `f32` before it reaches here.)
    let ends = [
        halves.junctions[0],
        halves.junctions[1],
        (halves.apexes[0], halves.apexes[1]),
    ];
    if !(halves.length.is_finite() && ends.iter().all(|(a, b)| a.is_finite() && b.is_finite())) {
        if let Some(trace) = trace {
            *trace = natural_trace;
        }
        return (fitted, natural);
    }
    let lay = Layout::new(&halves, outward);

    // The run the lines are laid from, and each line: its half, its
    // glyphs, and whether R27 cut it.
    let sized = matches!(fit, FitMode::Fit { .. });
    let (base, specs): (ShapedRun, Vec<(usize, Range<usize>, bool)>) = if sized {
        let base = fitted.clone().unwrap_or_else(|| run.clone());
        let specs = natural_trace
            .lines
            .iter()
            .filter_map(|line| {
                let ids = natural.glyphs[line.glyphs.clone()]
                    .iter()
                    .map(|g| g.glyph as usize);
                Some((line.half, ids.clone().min()?..ids.max()? + 1, line.cut))
            })
            .collect();
        (base, specs)
    } else {
        let specs = line_partition(run, outline, fit, flow)
            .into_iter()
            .enumerate()
            .filter(|(_, range)| !range.is_empty())
            .map(|(k, range)| (k, range, false))
            .collect();
        (run.clone(), specs)
    };

    // The text at `scale` times its size: the run as drawn — the text,
    // and the side ornament on a line of its own — its lines, laid with
    // their clearance grown from `seed`. With `cheap`, `None` when a line
    // does not fit its half less the seed at its font's gap (one `track`
    // a line), before any clearance is grown: what a shrink's probe
    // looks at first (ruling R35.3).
    let build = |scale: f32, seed: Clear, cheap: bool, fs: &mut FontSystemWriteGuard| {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        let scaled = if scale == 1.0 {
            base.clone()
        } else {
            let fsys: &mut FontSystem = fs;
            reshape(&base, scale, None, fsys)
        };
        let (drawn, side) = match side_run(&scaled, &layout.sides, flow, fs) {
            Some((with, range)) => (with, Some(range)),
            None => (scaled, None),
        };
        let fsys: &mut FontSystem = fs;
        let drawn = inked(&drawn, fsys);
        let prepared: Vec<Prepared> = specs
            .iter()
            .filter_map(|(k, range, cut)| lay.prepare(&drawn, *k, range.clone(), *cut))
            .collect();
        let too_long = |p: &Prepared| {
            let (lo, hi) = lay.room(p.half, seed[p.half]);
            p.clusters.iter().filter(|c| !c.blank).count() >= 2
                && !lay
                    .reach(p.half, &drawn, &p.clusters, lo, p.natural_gap)
                    .le(&(hi - lo + SLIDE_TOLERANCE))
        };
        if cheap && prepared.iter().any(too_long) {
            return None;
        }
        let backtracks = if cheap { PROBE_BACKTRACKS } else { BACKTRACKS };
        let cleared = lay.lay_clear(
            &drawn,
            &prepared,
            side.as_ref().map(|r| (&drawn, r)),
            span,
            seed,
            backtracks,
        );
        let overlap = overlapping(&cleared.lines);
        Some(Built {
            drawn,
            side,
            prepared,
            lines: cleared.lines,
            clear: cleared.clear,
            met: cleared.met,
            overlap,
        })
    };
    let zero: Clear = [[0.0; 2]; 2];
    let Some(mut built) = build(1.0, zero, false, fs) else {
        unreachable!("only a cheap build is refused")
    };
    let mut scale = 1.0_f32;
    // A `Fit` frame that its halves do not hold at R27's size — a line
    // its half less the clearance cannot hold at its font's gap, a
    // junction left short of a break, two inks touching: a flat outline,
    // its half barely longer than the level span R27 sized the text for —
    // is shrunk, as the last resort, to the largest size within its
    // bounds at which they hold (§6.2 step 6, rulings R34 and R35.2).
    // Each probe looks first at whether its lines fit their halves less
    // a clearance seeded from the last size held, and grows that
    // clearance only if they do (ruling R35.3).
    if let (false, FitMode::Fit { min, .. }) = (built.holds(), fit) {
        let fitted_size = base.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let declared = run.glyphs.iter().map(|g| g.size).fold(0.0_f32, f32::max);
        let floor = if fitted_size > 0.0 && declared > 0.0 {
            (min * declared / fitted_size).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let Some(at_floor) = build(floor, zero, false, fs) else {
            unreachable!("only a cheap build is refused")
        };
        if at_floor.holds() {
            let (mut held, mut over) = (floor, 1.0_f32);
            let mut best = at_floor;
            for _ in 0..SHRINK_ROUNDS {
                if lay.spent_all() {
                    break;
                }
                let mid = 0.5 * (held + over);
                match build(mid, best.seed(), true, fs) {
                    Some(probe) if probe.holds() => {
                        (held, best) = (mid, probe);
                    }
                    _ => over = mid,
                }
            }
            (built, scale) = (best, held);
        } else {
            (built, scale) = (at_floor, floor);
        }
    }
    let Built {
        drawn,
        side,
        mut prepared,
        mut lines,
        mut clear,
        mut met,
        mut overlap,
    } = built;
    let side_of = side.as_ref().map(|range| (&drawn, range));

    // A frame its halves hold even so by nothing is cut (rulings R34,
    // R35.2): a line its half less the clearance cannot hold at its
    // font's gap — a `Once` or `Spread` line at its size, a `Fit` line at
    // its least — to what it can. A line its half holds is never cut
    // (ruling R34): a junction short of a break, or two inks touching,
    // where no line is pressed, is reported in the trace instead.
    let mut truncated = natural.truncated && sized;
    for _ in 0..CUT_ROUNDS {
        let holds = !lines.iter().any(|l| l.pressed) && met.iter().all(|&m| m) && !overlap;
        if holds || lay.spent_all() {
            break;
        }
        let mut changed = false;
        for (p, line) in prepared.iter_mut().zip(&lines) {
            if !line.pressed {
                continue;
            }
            if let Some(shorter) = lay.cut(&drawn, p, clear[p.half]) {
                *p = shorter;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        truncated = true;
        let cleared = lay.lay_clear(&drawn, &prepared, side_of, span, [[0.0; 2]; 2], BACKTRACKS);
        (lines, clear, met) = (cleared.lines, cleared.clear, cleared.met);
        overlap = overlapping(&lines);
    }
    let mut unheld = unheld_of(&lines, met, overlap);

    let mut text = Vec::new();
    let mut line_ranges = Vec::new();
    for line in &lines {
        line_ranges.push(text.len()..text.len() + line.tracked.glyphs.len());
        text.extend(line.tracked.glyphs.iter().copied());
    }

    // The side ornament round the junctions.
    let mut sides = Vec::new();
    if let Some(range) = &side {
        let side = SideOrnament::new(&lay, &drawn, range.clone(), mean_size(&lines));
        let target = lines.iter().map(|l| l.gap).sum::<f32>() / lines.len().max(1) as f32;
        let text_boxes: Vec<InkBox> = lines
            .iter()
            .flat_map(|l| l.tracked.boxes.iter().copied())
            .collect();
        for (at, from, to) in side_arcs(&lines) {
            let budget = MAX_PLACED_GLYPHS.saturating_sub(text.len() + sides.len());
            if let Some(arc) = side.arc(from, to) {
                let before = sides.len();
                arc.fill(target, budget, &mut sides);
                // An arc whose copies would touch the text, the copies
                // already laid, or one another but for neighbors — round
                // an outline thinner than its lines, inward, where an arc
                // folds back across itself — is left empty, and reported
                // (ruling R35.2).
                let boxes: Vec<InkBox> = sides[before..]
                    .iter()
                    .filter_map(|g| glyph_box(&drawn, g))
                    .collect();
                let laid: Vec<InkBox> = sides[..before]
                    .iter()
                    .filter_map(|g| glyph_box(&drawn, g))
                    .collect();
                let touches = boxes.iter().enumerate().any(|(i, b)| {
                    text_boxes.iter().chain(&laid).any(|t| box_gap(*b, *t) <= TOUCH)
                        || boxes.iter().skip(i + 2).any(|c| box_gap(*b, *c) <= TOUCH)
                });
                if touches {
                    sides.truncate(before);
                    unheld.sides.extend((0..2).filter(|&j| at[j]));
                }
            }
        }
    }

    if let Some(trace) = trace {
        *trace = Trace::default();
        trace.lines = lines
            .iter()
            .zip(&prepared)
            .zip(&line_ranges)
            .map(|((line, p), glyphs)| TracedLine {
                half: line.half,
                glyphs: glyphs.clone(),
                from: line.part.0,
                to: line.part.1,
                cut: p.cut,
            })
            .collect();
        trace.unheld = unheld;
    }
    let changed = (sized && fitted.is_some()) || side.is_some() || scale != 1.0;
    let mut glyphs = text;
    glyphs.extend(sides);
    let placed = PlacedRun { glyphs, truncated };
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
    let walk = line_walk(&halves.paths[0], &run, true);
    let (sa, sb) = halves.junctions[0];
    gaps.iter()
        .map(|&g| {
            let tracked = track(&walk, &run.glyphs, &clusters, (sa, sb), g);
            tracked.ends.map_or(sa, |(_, last)| halves.along(0, middle(last)))
        })
        .collect()
}

/// Test seam (#291, ruling R33.1): `n` copies of `ornament` laid as a
/// junction lays them at their floor gap ([`SideOrnament::arc_floor`]),
/// along `outline`'s first half from a point at its top-left junction,
/// beside `run`'s text: the run they are shaped into, the copies, and the
/// floor. `None` when the ornament shapes to nothing or has no ink.
pub(crate) fn side_floor_seam(
    run: &ShapedRun,
    outline: &Outline,
    ornament: &str,
    n: usize,
    flow: &ResolvedText,
    fs: &mut FontSystemWriteGuard,
) -> Option<(ShapedRun, Vec<PlacedGlyph>, f32)> {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true);
    let sides = Some((ornament.to_owned(), Rgba::opaque(0, 0, 0)));
    let (with, range) = side_run(run, &sides, flow, fs)?;
    let fsys: &mut FontSystem = fs;
    let drawn = inked(&with, fsys);
    let (size, _) = letters_of(&clusters_of(&drawn.glyphs, 0..range.start))?;
    let side = SideOrnament::new(&lay, &drawn, range, size);
    let (ja, jb) = halves.junctions[0];
    let point = |s: f32| {
        let p = halves.paths[0].at(s).0;
        (p, p)
    };
    let arc = side.arc(point(ja), point(jb))?;
    let glyphs = arc.lay(n, arc.floor).0.glyphs;
    let floor = arc.floor;
    Some((uninked(drawn), glyphs, floor))
}

/// Test seam (#291, ruling R34): the part of `outline`'s half `half` a
/// line may cover with `clear` kept at its junctions, and the half's
/// junctions — [`Layout::room`], which never inverts.
pub(crate) fn room_seam(outline: &Outline, half: usize, clear: [f32; 2]) -> ((f32, f32), (f32, f32)) {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    let lay = Layout::new(&halves, true);
    (lay.room(half, clear), halves.junctions[half])
}

/// Test seam (#291, ruling R33): `outline`'s two halves, each as its
/// junctions and apex along its path — from the middle of the left
/// side, clockwise for the first, counterclockwise for the second — and
/// the outline's length.
pub(crate) fn halves_seam(outline: &Outline) -> ([(f32, f32); 2], [f32; 2], f32) {
    let halves = Halves::new(outline, MIN_TABLE_SPACING);
    (halves.junctions, halves.apexes, halves.length)
}
