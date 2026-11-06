// SPDX-License-Identifier: MPL-2.0

//! The demo maps' frames held to the readable flow's structure, read
//! from their placed glyphs' inks (#291, ruling R44.8): the structural
//! subset of the checker the E7 review ran by hand, so that "the demo
//! maps pass the checker" is reproducible from the branch.
//!
//! The maps are projected as `tests_gpu_demo_maps` projects them for
//! drawing — every node a box, each ring on the outline past the rings
//! inside it — and each glyph's ink is its rasterized image's box at
//! eight times its size, on the canvas: geometry from the glyphs, never
//! pixels of a frame (TEST_CONVENTIONS: no pixel assertions). Three
//! properties, each a distance between inks:
//!
//! - **Each junction is a visible break** (ruling R33.1): a two-line
//!   frame's lines' end inks are at least two word gaps apart at each
//!   junction, of the mean of the lines' mean word gaps.
//! - **No two elements' inks overlap** (ruling R38.1): no ink of one
//!   run — a frame's text, its sides, a ring, a path, a section — lies
//!   over another's by more than half a pixel at the camera that frames
//!   the map.
//! - **A frame's sides are even** (ruling R33.1): round each junction,
//!   the gaps between the ornament's inks and its two joins to the text
//!   are within [`EVEN`] of one another.
//!
//! It draws nothing, so it runs wherever the GPU suite's module is
//! compiled, adapter or none.

use baumhard::document::format::load;
use baumhard::flow::FILLER_SPAN;
use baumhard::gfx_structs::run::{GlyphRunData, RunShape};
use glam::Vec2;
use glyphon::{CacheKey, SwashCache};

use super::tests_gpu_demo_maps::{camera_on, demo_maps, project, with_fonts};

/// How far apart a frame's side gaps and joins may be, the widest over
/// the narrowest: the placement sets them equal, to a thousandth of a
/// point, box to box; an ink rasterized at eight times its size and
/// measured box to box differs by a fraction of a pixel from the box the
/// placement measured.
const EVEN: f32 = 1.3;

/// How many of the lines' mean word gaps a junction's end inks are
/// apart at the least (ruling R33.1).
const JUNCTION_WORD_GAPS: f32 = 2.0;

/// One glyph's ink, a canvas rectangle.
#[derive(Clone, Copy, Debug)]
struct Ink {
    min: Vec2,
    max: Vec2,
}

impl Ink {
    /// The distance between two boxes, 0 where they meet.
    fn gap(&self, other: &Ink) -> f32 {
        let d = (other.min - self.max).max(self.min - other.max).max(Vec2::ZERO);
        d.length()
    }

    /// How deep two boxes overlap, along the axis they overlap least on;
    /// 0 where they do not.
    fn overlap(&self, other: &Ink) -> f32 {
        let o = self.max.min(other.max) - self.min.max(other.min);
        if o.x > 0.0 && o.y > 0.0 {
            o.x.min(o.y)
        } else {
            0.0
        }
    }

    fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
}

/// A placed glyph as the checker reads it: its ink (`None` for a glyph
/// with none, a space), its pen's x, whether it is blank, and whether
/// it is a side ornament's (the filler span's).
type Read = (Option<Ink>, f32, bool, bool);

/// Every placed glyph of a path run, read; a box run is one ink, its
/// bounds.
fn read(run: &GlyphRunData, fs: &mut glyphon::FontSystem, swash: &mut SwashCache) -> Vec<Read> {
    let RunShape::Path { run: shaped, placed } = run.shaped() else {
        let (min, max) = run.bounds();
        return vec![(Some(Ink { min, max }), min.x, false, false)];
    };
    let layout: Vec<_> = shaped.layout_glyphs().collect();
    let baseline = shaped.baseline();
    let origin = run.origin();
    let scales = run.overrides().scale();
    let offsets = run.overrides().offset();
    placed
        .glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let metrics = &shaped.glyphs[g.glyph as usize];
            let laid = layout[g.glyph as usize];
            let scale = scales.get(i).copied().unwrap_or(1.0);
            let offset = offsets.get(i).copied().unwrap_or(Vec2::ZERO);
            let pen = origin + g.pos + Vec2::new(0.0, baseline);
            let center = origin + g.pos + Vec2::new(metrics.advance, shaped.line_height) * 0.5;
            let pen = center + (pen - center) * scale + offset;
            let size = laid.font_size * scale;
            let canvas = pen + Vec2::new(size * laid.x_offset, -size * laid.y_offset);
            // Rasterized at eight times the size, so a pixel's rounding is
            // an eighth of a canvas point, not a whole one.
            const OVER: f32 = 8.0;
            let (key, _, _) = CacheKey::new(
                laid.font_id,
                laid.glyph_id,
                size * OVER,
                (0.0, 0.0),
                laid.font_weight,
                laid.cache_key_flags,
            );
            let ink = swash.get_image_uncached(fs, key).and_then(|image| {
                let p = image.placement;
                (p.width > 0 && p.height > 0).then(|| {
                    let min = canvas + Vec2::new(p.left as f32, -(p.top as f32)) / OVER;
                    Ink {
                        min,
                        max: min + Vec2::new(p.width as f32, p.height as f32) / OVER,
                    }
                })
            });
            (ink, pen.x, metrics.blank, metrics.span == FILLER_SPAN)
        })
        .collect()
}

/// A frame's text split into its lines: the placement lists the first
/// half's line, then the second's, each from the left junction round to
/// the right, so a line ends where the pen jumps back across the frame.
fn lines_of(text: &[Read], jump: f32) -> Vec<Vec<(Option<Ink>, bool)>> {
    let mut lines: Vec<Vec<(Option<Ink>, bool)>> = vec![Vec::new()];
    let mut last_x = f32::NEG_INFINITY;
    for &(ink, x, blank, _) in text {
        if x < last_x - jump {
            lines.push(Vec::new());
        }
        last_x = x;
        if let Some(line) = lines.last_mut() {
            line.push((ink, blank));
        }
    }
    lines
}

/// A line's word gaps: from the last ink before a space to the first
/// after it.
fn word_gaps(line: &[(Option<Ink>, bool)]) -> Vec<f32> {
    let mut gaps = Vec::new();
    let (mut prev, mut spaced): (Option<Ink>, bool) = (None, false);
    for &(ink, blank) in line {
        if blank {
            spaced = true;
            continue;
        }
        let Some(ink) = ink else { continue };
        if let (Some(p), true) = (prev, spaced) {
            gaps.push(p.gap(&ink));
        }
        prev = Some(ink);
        spaced = false;
    }
    gaps
}

/// The widest of `values` over the narrowest; 1 for fewer than two.
fn spread(values: &[f32]) -> f32 {
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if values.len() < 2 {
        1.0
    } else {
        max / min
    }
}

/// The junctions the demo maps' two-line frames have, each of which the
/// checker measures: ten, two in each of five frames. A frame it cannot
/// read as two lines is skipped, and would otherwise pass unseen (review
/// 5739829769).
const DEMO_JUNCTIONS: usize = 10;

/// R44.8: the demo maps hold the checker's structure — every two-line
/// frame's junctions a break of two word gaps or more, no two elements'
/// inks overlapping, and every frame's sides even round each junction.
///
/// Controls: with a junction's break wanted at one word gap
/// (`JUNCTION_WORD_GAPS` in the placement halved), red on the
/// junctions; with a side's copies laid at their floor gap rather than
/// their solved one, red on the sides; with `lines_of`'s jump threshold
/// a tenth of its own, so that no frame reads as two lines, red on the
/// precondition.
#[test]
fn test_demo_maps_hold_the_readable_structure() {
    let maps = demo_maps();
    assert!(
        maps.len() >= 4,
        "the README's demo maps must be found, else this would pass over none: {maps:?}"
    );
    let mut faults: Vec<String> = Vec::new();
    let (mut junctions, mut arcs, mut elements) = (0, 0, 0);
    for (name, path) in maps {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let projected = with_fonts(|fs| project(&doc, fs));
        let zoom = camera_on(projected.bounds).zoom;
        let mut swash = SwashCache::new();
        let read: Vec<Vec<Read>> = with_fonts(|fs| {
            let fsys = fs.fonts();
            projected
                .runs
                .iter()
                .map(|run| read(run, fsys, &mut swash))
                .collect()
        });
        // Each element's inks: a frame's text and its sides apart.
        let mut inks: Vec<(String, Vec<Ink>)> = Vec::new();
        for (glyphs, label) in read.iter().zip(&projected.labels) {
            let (sides, own): (Vec<&Read>, Vec<&Read>) = glyphs.iter().partition(|g| g.3);
            inks.push((label.clone(), own.iter().filter_map(|g| g.0).collect()));
            if !sides.is_empty() {
                inks.push((
                    format!("{label} sides"),
                    sides.iter().filter_map(|g| g.0).collect(),
                ));
            }
            if !label.starts_with("frame") {
                continue;
            }
            let text: Vec<Read> = own.into_iter().copied().collect();
            let lines = lines_of(&text, 50.0 / zoom.max(1e-3));
            let first = |l: &Vec<(Option<Ink>, bool)>| l.iter().find_map(|g| g.0);
            let last = |l: &Vec<(Option<Ink>, bool)>| l.iter().rev().find_map(|g| g.0);
            let [top, bottom] = lines.as_slice() else {
                continue;
            };
            let (Some(a), Some(b), Some(c), Some(d)) = (first(top), first(bottom), last(top), last(bottom))
            else {
                continue;
            };
            // The mean of the lines' mean word gaps, of the lines that
            // have one, as the placement measures a break.
            let means: Vec<f32> = [top, bottom]
                .iter()
                .map(|line| word_gaps(line))
                .filter(|gaps| !gaps.is_empty())
                .map(|gaps| gaps.iter().sum::<f32>() / gaps.len() as f32)
                .collect();
            if means.is_empty() {
                continue;
            }
            let word = means.iter().sum::<f32>() / means.len() as f32;
            for (junction, p, q) in [("top-left", a, b), ("bottom-right", c, d)] {
                junctions += 1;
                let ratio = p.gap(&q) / word;
                if ratio.is_nan() || ratio < JUNCTION_WORD_GAPS {
                    faults.push(format!(
                        "{name} {label} {junction} junction: ends {:.1}px apart, ×{ratio:.2} the lines' mean word gap {:.1}px",
                        p.gap(&q) * zoom,
                        word * zoom
                    ));
                }
            }
            // The sides round each junction: each ornament ink the arc of
            // the junction it is nearer, in the order laid; the gaps
            // between them and the joins to the text's nearest inks.
            let side: Vec<Ink> = sides.iter().filter_map(|g| g.0).collect();
            let text_inks: Vec<Ink> = text.iter().filter_map(|g| g.0).collect();
            let near = |ink: &Ink| text_inks.iter().map(|t| ink.gap(t)).fold(f32::INFINITY, f32::min);
            let meets = [0.5 * (a.center() + b.center()), 0.5 * (c.center() + d.center())];
            for (k, meet) in meets.iter().enumerate() {
                let other = meets[1 - k];
                let arc: Vec<Ink> = side
                    .iter()
                    .copied()
                    .filter(|o| o.center().distance(*meet) <= o.center().distance(other))
                    .collect();
                let (Some(head), Some(tail)) = (arc.first(), arc.last()) else {
                    continue;
                };
                arcs += 1;
                let mut spaced: Vec<f32> = arc.windows(2).map(|w| w[0].gap(&w[1])).collect();
                spaced.extend([near(head), near(tail)]);
                if spread(&spaced).is_nan() || spread(&spaced) > EVEN {
                    faults.push(format!(
                        "{name} {label} sides round junction {k}: {} inks, gaps and joins {:?}px, ×{:.2} apart",
                        arc.len(),
                        spaced.iter().map(|g| (g * zoom * 10.0).round() / 10.0).collect::<Vec<_>>(),
                        spread(&spaced)
                    ));
                }
            }
        }
        // No ink of one element over another's.
        for i in 0..inks.len() {
            elements += 1;
            for j in i + 1..inks.len() {
                let worst = inks[i]
                    .1
                    .iter()
                    .flat_map(|a| inks[j].1.iter().map(move |b| a.overlap(b)))
                    .fold(0.0_f32, f32::max);
                if worst > 0.5 / zoom {
                    faults.push(format!(
                        "{name}: {} and {} overlap by {:.1}px",
                        inks[i].0,
                        inks[j].0,
                        worst * zoom
                    ));
                }
            }
        }
    }
    assert!(
        junctions >= DEMO_JUNCTIONS && arcs > 0 && elements > 0,
        "precondition: the maps' frames are checked ({junctions} junctions, at least {DEMO_JUNCTIONS}; {arcs} side arcs, {elements} elements)"
    );
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
