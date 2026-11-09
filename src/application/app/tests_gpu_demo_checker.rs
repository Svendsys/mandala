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

use baumhard::arena::NodeId;
use baumhard::document::element::DocElement;
use baumhard::document::format::load;
use baumhard::document::ids::Document;
use baumhard::flow::FILLER_SPAN;
use baumhard::font::fallback::ORNAMENT_FALLBACK;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::run::{GlyphRunData, RunShape};
use baumhard::projection::{GlyphLayers, Layer, Projector};
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

/// Every glyph `run` was shaped into, as the checker's face property
/// reads it: the text it draws, its face and its glyph id. A path run's
/// placed glyphs are copies of these, so a face every shaped glyph is
/// drawn in is the face every placed one is drawn in.
fn shaped_glyphs(run: &GlyphRunData) -> Vec<(String, glyphon::fontdb::ID, u16)> {
    let buffer = match run.shaped() {
        RunShape::Path { run: shaped, .. } => &shaped.buffer,
        RunShape::Box(shaped) => &shaped.buffer,
    };
    buffer
        .layout_runs()
        .flat_map(|line| {
            line.glyphs
                .iter()
                .map(|g| (line.text[g.start..g.end].to_string(), g.font_id, g.glyph_id))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The compiled faces a demo map may draw a glyph in: every compiled
/// face of each family it pins (`fonts`: a bold span is drawn in its
/// family's bold face), read by name from the font system as a pin is
/// matched, and the ornament fallback's (#297).
fn allowed_faces(fs: &glyphon::FontSystem, fonts: &[AppFont]) -> Vec<glyphon::fontdb::ID> {
    let compiled = &baumhard::font::fonts::COMPILED_FONT_ID_MAP;
    let families_of = |ids: &[glyphon::fontdb::ID]| -> Vec<String> {
        ids.iter()
            .filter_map(|id| fs.db().face(*id))
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect()
    };
    let pinned: Vec<String> = fonts
        .iter()
        .filter_map(|font| compiled.get(font))
        .flat_map(|ids| families_of(ids))
        .collect();
    let mut allowed: Vec<glyphon::fontdb::ID> = compiled
        .values()
        .filter(|ids| families_of(ids).iter().any(|family| pinned.contains(family)))
        .flat_map(|ids| ids.iter().copied())
        .collect();
    for (font, _) in ORNAMENT_FALLBACK {
        allowed.extend(compiled.get(font).into_iter().flat_map(|ids| ids.iter().copied()));
    }
    allowed
}

/// (i) No demo map draws a glyph in a host face (#297): every glyph of
/// every run, text, ornament and connector alike, is drawn in a compiled
/// face of a family the map pins, or in an ornament fallback face, and
/// none is a `.notdef`. So the four README images show the same glyphs on
/// every install, and in the browser, which has the compiled faces alone.
///
/// The two halves past "compiled" are what keep the property from passing
/// on a host with no faces of its own (CI, or `FONTCONFIG_FILE` at an
/// empty directory, §T14). There a glyph no face holds is not drawn in a
/// host face but as the first face's empty box, and a glyph one compiled
/// face holds is drawn in whichever comes first in cosmic-text's order, a
/// decorative face the map never named, unless the ornament fallback
/// comes before it.
///
/// Controls: with `OrnamentsFirst` listing the platform's families alone
/// (the ornament fallback off), red on a desktop on the 15 glyphs drawn
/// in `DejaVuSans` and `NotoColorEmoji`, and with the host's faces hidden
/// on the connectors' `~`, drawn in `212Keyboard`. Before the ornament
/// faces were compiled in, red with the host's faces hidden on `✦`, `❦`
/// and `🌸` drawn as Galatia SIL's `.notdef`.
#[test]
fn test_demo_maps_draw_every_glyph_in_a_face_they_pin_or_the_ornament_fallback() {
    let maps = demo_maps();
    assert!(
        maps.len() >= 4,
        "the README's demo maps must be found, else this would pass over none: {maps:?}"
    );
    let compiled: Vec<glyphon::fontdb::ID> = baumhard::font::fonts::COMPILED_FONT_ID_MAP
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect();
    let mut faults: Vec<String> = Vec::new();
    let (mut glyphs, mut unpinned) = (0, 0);
    for (name, path) in maps {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let projected = with_fonts(|fs| project(&doc, fs));
        unpinned += usize::from(projected.fonts.contains(&AppFont::Any));
        with_fonts(|fs| {
            let fs = fs.fonts();
            let allowed = allowed_faces(fs, &projected.fonts);
            for (run, label) in projected.runs.iter().zip(&projected.labels) {
                for (text, face, glyph) in shaped_glyphs(run) {
                    glyphs += 1;
                    let named = fs
                        .db()
                        .face(face)
                        .map_or_else(|| format!("{face:?}"), |f| f.post_script_name.clone());
                    if !compiled.contains(&face) {
                        faults.push(format!("{name} {label}: {text:?} drawn in the host face {named}"));
                    } else if !allowed.contains(&face) {
                        faults.push(format!(
                            "{name} {label}: {text:?} drawn in {named}, a face the map does not pin"
                        ));
                    } else if glyph == 0 {
                        faults.push(format!("{name} {label}: {text:?} drawn as {named}'s .notdef"));
                    }
                }
            }
        });
    }
    assert!(glyphs > 0, "precondition: the maps' glyphs are read ({glyphs})");
    assert!(
        unpinned > 0,
        "precondition: a demo map has a span that pins no face, which only the fallback may draw"
    );
    faults.sort();
    faults.dedup();
    assert!(
        faults.is_empty(),
        "{} glyphs not drawn in a face the map pins or the ornament fallback:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

/// The README's demo maps and the four maps converted from the old format
/// (#244), each by name: what the connector checks read.
fn connector_maps() -> Vec<(String, std::path::PathBuf)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps");
    let mut maps = demo_maps();
    for name in ["palette_cascade", "stress_long_edges", "testament", "theme_demo"] {
        maps.push((name.to_string(), root.join(format!("{name}.mandala.json"))));
    }
    maps
}

/// A drawn connector or line link as the real projector drew it (#296):
/// its body's inks in order, its curves, its body's first glyph's face and
/// size, and at each end the node it attaches to.
struct DrawnPath {
    name: String,
    inks: Vec<Ink>,
    curves: Vec<baumhard::flow::Curve>,
    style: String,
    ends: [NodeId; 2],
    /// Whether it is one straight segment, which leaves and arrives along
    /// its own line rather than its sides' normals.
    straight: bool,
}

/// Every node's frame inks, by node: its rings', or for a node that
/// draws none its drawn rectangle.
type Frames = std::collections::HashMap<NodeId, Vec<Ink>>;

/// Every connector and line link of `doc` and the inks of every node's
/// frame, through the projector the app draws with
/// (`Projector::project_full`) and each glyph's rasterized ink (`read`);
/// and the zoom of the camera that frames them.
fn drawn_paths(doc: &Document) -> (Vec<DrawnPath>, Frames, f32) {
    let mut layers = GlyphLayers::new();
    let projector = with_fonts(|fs| Projector::project_full(doc, &mut layers, fs));
    let tree = doc.tree();
    let parent = |id: NodeId| tree.node(id).and_then(|n| n.parent());
    let node_of = |mut id: NodeId| loop {
        if matches!(doc.get(id), Some(DocElement::Node(_))) {
            return Some(id);
        }
        id = parent(id)?;
    };
    let mut swash = SwashCache::new();
    let mut frames: Frames = Default::default();
    let mut paths = Vec::new();
    with_fonts(|fs| {
        let fsys = fs.fonts();
        for layer in [Layer::Frames, Layer::Connections] {
            let t = layers.tree(layer);
            for glyph in t.root().descendants(t.arena()) {
                let Some(run) = t.arena()[glyph].get().glyph_run() else {
                    continue;
                };
                let Some(element) = projector.element_of(layer, glyph) else {
                    continue;
                };
                let inks: Vec<Ink> = read(run, fsys, &mut swash)
                    .into_iter()
                    .filter_map(|g| g.0)
                    .collect();
                match (layer, doc.get(element)) {
                    (Layer::Frames, _) => {
                        if let Some(node) = node_of(element) {
                            frames.entry(node).or_default().extend(inks);
                        }
                    }
                    (_, Some(DocElement::Connector(_))) | (_, Some(DocElement::Link(_))) => {
                        let Some(own) = node_of(element) else { continue };
                        let ends = match doc.get(element) {
                            Some(DocElement::Link(link)) => [own, link.to],
                            _ => match parent(own).and_then(node_of) {
                                Some(up) => [up, own],
                                None => continue,
                            },
                        };
                        let RunShape::Path { run: shaped, .. } = run.shaped() else { continue };
                        let style = shaped.layout_glyphs().next().map_or(String::new(), |g| {
                            format!(
                                "glyph {} of face {:?} at {:.1}pt",
                                g.glyph_id, g.font_id, g.font_size
                            )
                        });
                        let ident = |n: NodeId| match doc.get(n) {
                            Some(DocElement::Node(data)) => data.ident.0,
                            _ => 0,
                        };
                        paths.push(DrawnPath {
                            name: format!("{}>{}", ident(ends[0]), ident(ends[1])),
                            inks,
                            curves: projector.curve(element).map(<[_]>::to_vec).unwrap_or_default(),
                            style,
                            ends,
                            straight: match doc.get(element) {
                                Some(DocElement::Connector(c)) => c.path.straight,
                                Some(DocElement::Link(l)) => l.path.straight,
                                _ => false,
                            },
                        });
                    }
                    _ => {}
                }
            }
        }
    });
    for node in paths.iter().flat_map(|p| p.ends) {
        if let (std::collections::hash_map::Entry::Vacant(slot), Some(rect)) =
            (frames.entry(node), projector.node_rect(node))
        {
            slot.insert(vec![Ink {
                min: rect.origin,
                max: rect.max(),
            }]);
        }
    }
    let all: Vec<Ink> = frames.values().flatten().copied().collect();
    let min = all.iter().fold(Vec2::splat(f32::INFINITY), |a, i| a.min(i.min));
    let max = all
        .iter()
        .fold(Vec2::splat(f32::NEG_INFINITY), |a, i| a.max(i.max));
    (paths, frames, camera_on((min, max)).zoom)
}

/// The box round `inks`.
fn ink_bounds(inks: &[Ink]) -> Ink {
    inks.iter().skip(1).fold(inks[0], |a, b| Ink {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    })
}

/// The side of `reach` whose line `point` lies nearest: its midpoint and
/// outward normal.
fn side_of(reach: &Ink, point: Vec2) -> (Vec2, Vec2) {
    let c = (reach.min + reach.max) * 0.5;
    [
        (
            Vec2::new(c.x, reach.min.y),
            -Vec2::Y,
            (point.y - reach.min.y).abs(),
        ),
        (
            Vec2::new(reach.max.x, c.y),
            Vec2::X,
            (point.x - reach.max.x).abs(),
        ),
        (
            Vec2::new(c.x, reach.max.y),
            Vec2::Y,
            (point.y - reach.max.y).abs(),
        ),
        (
            Vec2::new(reach.min.x, c.y),
            -Vec2::X,
            (point.x - reach.min.x).abs(),
        ),
    ]
    .into_iter()
    .min_by(|a, b| a.2.total_cmp(&b.2))
    .map(|(m, n, _)| (m, n))
    .unwrap_or((c, -Vec2::Y))
}

/// The pairs of paths that share a side which (h) does not hold yet, each
/// by map and the two paths' names, and why (#326, put to the owner): no
/// curve from the shared point keeps them apart without routing round a
/// node, which #296 does not do.
///
/// - **On the way.** One path's far end lies on the other's way: its
///   node sits between the junction and the other's node, its end on the
///   line the other must cross, so the other meets it however it curves.
///
/// A pair listed here that keeps clear is a fault too, so the list only
/// shrinks.
const UNROUTED: &[(&str, &str, &str)] = &[
    ("stress_long_edges", "5>120", "5>119"),
    ("stress_long_edges", "5>121", "5>119"),
    ("stress_long_edges", "5>121", "5>120"),
    ("stress_long_edges", "5>121", "6>121"),
    ("testament", "29>31", "29>32"),
];

/// The least distance between an ink of `a` and an ink of `b`, of those
/// nearer than `within`, and where it is; `within`, nowhere, where none
/// is. A grid of cells `within`
/// wide and as wide as the largest ink holds `b`'s inks, so each of
/// `a`'s is measured to those of the nine cells round its own alone:
/// two paths of thousands of glyphs each are compared in O(inks).
fn nearest_between(a: &[Ink], b: &[Ink], within: f32) -> (f32, Option<Vec2>) {
    let largest = b
        .iter()
        .chain(a)
        .map(|i| (i.max - i.min).max_element())
        .fold(0.0_f32, f32::max);
    let cell = (within + largest).max(1e-3);
    let key = |p: Vec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (j, ink) in b.iter().enumerate() {
        grid.entry(key(ink.center())).or_default().push(j);
    }
    let mut best = (within, None);
    for ink in a {
        let (x, y) = key(ink.center());
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in grid.get(&(x + dx, y + dy)).into_iter().flatten() {
                    let gap = ink.gap(&b[j]);
                    if gap < best.0 {
                        best = (gap, Some(ink.center()));
                    }
                }
            }
        }
    }
    best
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// The checker's raster: an ink is measured to an eighth of a canvas point
/// (`read`), so a direction measured between two inks `d` apart is good
/// to the angle an eighth subtends at `d`.
fn raster_degrees(d: f32) -> f32 {
    (0.125 / d.max(1e-3)).atan().to_degrees()
}

/// One end of a drawn path, measured: where it sits, the paths that share
/// its side, and how it meets its frame.
struct End {
    path: usize,
    /// Which end: 0 its start, 1 its end.
    k: usize,
    /// Its side's midpoint and outward normal, on the frame's ink box.
    mid: Vec2,
    normal: Vec2,
    /// Where the path starts at this end, and how far it turns from the
    /// normal there, degrees.
    start: Vec2,
    turn: f32,
}

/// #296's measures, as the #291 checker named them, on every connector
/// and line link the real projector draws in the demo maps and the
/// converted maps:
///
/// - **(a)** both end gaps, from the end glyph's ink to the nearest ink of
///   its node's frame (or its drawn rectangle, for a node with none), are
///   0.8–1.2× the path's own median gap between inks — but where the
///   paths that share the side are fanned to the 40° cap, whose first
///   glyphs are held out until they clear each other (rule 4: "no glyph
///   overlaps"), and are measured by (h) instead;
/// - **(g)** each end glyph sits off its side's midpoint by at most half a
///   step and its last step runs within 10° of the side's normal — or,
///   where the paths that share the side are fanned, the path starts on
///   the midpoint, within half a pixel, and its end glyph lies in the cone
///   between the normal and its turn, which is at most 40°;
/// - **(h)** where paths share a side, no glyph of one comes nearer a
///   glyph of another than ×0.8 their own gap, anywhere along either
///   (rule 4: "no glyph overlaps", ruled on #326 to hold over the whole
///   path);
/// - **(j)** the paths of one style (glyph, face, size) are gapped within
///   ±10% of their mean ink gap (rule 5, its pitch read as the gap
///   between inks, box to box, as ruled on #326).
///
/// Distances are the inks' as `read` rasterizes them, at eight times
/// their size; every threshold is the #291 checker's, the cone's widened
/// by what that raster resolves.
///
/// Controls: with the reach a node's drawn rectangle (`Projector::reach`
/// ignoring the rings), red on (a), (g) and (h); with no lead
/// (`path_through`'s `lead` 0), red on (g); with the pitch laid as a
/// `repeat` walk (`place_on_open`), red on (a) and (h); with no hold at a
/// capped fan, red on (h); with a text ring's inks read as its glyphs'
/// boxes, red on (a) and (g) at ecclesiastes' text frame; with no fan, red
/// on the precondition that some end is fanned.
#[test]
fn test_maps_hold_the_connector_rules() {
    let mut faults: Vec<String> = Vec::new();
    let (mut measured, mut fanned, mut held) = (0, 0, 0);
    for (name, path) in connector_maps() {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let (paths, frames, zoom) = drawn_paths(&doc);
        let own: Vec<f32> = paths
            .iter()
            .map(|p| median(p.inks.windows(2).map(|w| w[0].gap(&w[1])).collect()))
            .collect();
        let pitch: Vec<f32> = paths
            .iter()
            .map(|p| {
                median(
                    p.inks
                        .windows(2)
                        .map(|w| w[0].center().distance(w[1].center()))
                        .collect(),
                )
            })
            .collect();
        let end_ink = |p: &DrawnPath, k: usize| {
            if k == 0 {
                p.inks[0]
            } else {
                p.inks[p.inks.len() - 1]
            }
        };
        let next_ink = |p: &DrawnPath, k: usize| {
            if k == 0 {
                p.inks[1]
            } else {
                p.inks[p.inks.len() - 2]
            }
        };
        // Every end, and the ends that share each side.
        let mut ends: Vec<End> = Vec::new();
        let mut sides: std::collections::HashMap<(NodeId, [i32; 2]), Vec<usize>> = Default::default();
        for (i, p) in paths.iter().enumerate() {
            if p.inks.len() < 3 || p.curves.is_empty() {
                continue;
            }
            for k in 0..2 {
                let (first, last) = (p.curves[0], p.curves[p.curves.len() - 1]);
                let (start, tangent) = if k == 0 {
                    (first.point(0.0), first.derivative(0.0))
                } else {
                    (last.point(1.0), -last.derivative(1.0))
                };
                let (mid, normal) = side_of(&ink_bounds(&frames[&p.ends[k]]), start);
                let turn = tangent
                    .normalize_or_zero()
                    .dot(normal)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                let key = (
                    p.ends[k],
                    [(mid.x * 10.0).round() as i32, (mid.y * 10.0).round() as i32],
                );
                sides.entry(key).or_default().push(ends.len());
                ends.push(End {
                    path: i,
                    k,
                    mid,
                    normal,
                    start,
                    turn,
                });
            }
        }
        // A fan at the cap holds its first glyphs out (rule 4).
        let mut capped = vec![false; ends.len()];
        for members in sides.values() {
            let widest = members.iter().map(|&e| ends[e].turn).fold(0.0_f32, f32::max);
            for &e in members {
                capped[e] = members.len() > 1 && (widest - 40.0).abs() < 0.05;
            }
        }
        for (e, end) in ends.iter().enumerate() {
            measured += 1;
            let p = &paths[end.path];
            let (ink, next) = (end_ink(p, end.k), next_ink(p, end.k));
            let tag = format!("{name} {} end {}", p.name, end.k);
            // (a)
            let near = frames[&p.ends[end.k]]
                .iter()
                .map(|f| ink.gap(f))
                .fold(f32::INFINITY, f32::min);
            let ratio = near / own[end.path];
            if capped[e] {
                held += 1;
            } else if !(0.8..=1.2).contains(&ratio) {
                faults.push(format!(
                    "(a) {tag}: {near:.2} from its frame, ×{ratio:.2} its own gap {:.2} (canvas units)",
                    own[end.path]
                ));
            }
            // (g), but for the direction a straight path leaves and arrives
            // in, which is its own line's.
            if p.straight {
                let off = end.start.distance(end.mid) * zoom;
                if off > 0.5 {
                    faults.push(format!(
                        "(g) {tag}: straight, starts {off:.2}px off its side's midpoint"
                    ));
                }
            } else if end.turn > 0.5 {
                fanned += 1;
                let reach = ink.center() - end.start;
                let cone = reach
                    .normalize_or_zero()
                    .dot(end.normal)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                let off = end.start.distance(end.mid) * zoom;
                if off > 0.5 || cone > end.turn + 0.5 + raster_degrees(reach.length()) || end.turn > 40.01 {
                    faults.push(format!(
                        "(g) {tag}: fanned {:.1}°, starts {off:.2}px off its side's midpoint, its end glyph {cone:.1}° off the normal",
                        end.turn
                    ));
                }
            } else {
                let across = Vec2::new(-end.normal.y, end.normal.x);
                let off = (ink.center() - end.mid).dot(across).abs();
                let step = next.center() - ink.center();
                let angle = step
                    .normalize_or_zero()
                    .dot(end.normal)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                if off > 0.5 * pitch[end.path] || angle > 10.0 + raster_degrees(step.length()) {
                    faults.push(format!(
                        "(g) {tag}: {off:.2} off its side's midpoint (half a step {:.2}), its last step {angle:.1}° off the normal",
                        0.5 * pitch[end.path]
                    ));
                }
            }
        }
        // (h) every glyph of each two paths that share a side.
        for members in sides.values().filter(|m| m.len() > 1) {
            for (x, &a) in members.iter().enumerate() {
                for &b in &members[x + 1..] {
                    let (ea, eb) = (&ends[a], &ends[b]);
                    if ea.path == eb.path {
                        continue;
                    }
                    let gap = own[ea.path].max(own[eb.path]);
                    let (pa, pb) = (&paths[ea.path], &paths[eb.path]);
                    let (nearest, at) = nearest_between(&pa.inks, &pb.inks, gap);
                    let listed = UNROUTED.iter().any(|&(map, x, y)| {
                        map == name && ((x == pa.name && y == pb.name) || (x == pb.name && y == pa.name))
                    });
                    if listed && nearest >= 0.8 * gap {
                        faults.push(format!(
                            "(h) {name} {} and {}: listed as unrouted, and keep clear: remove them from UNROUTED",
                            pa.name, pb.name
                        ));
                    } else if !listed && nearest < 0.8 * gap {
                        faults.push(format!(
                            "(h) {name} {} and {}: their glyphs {nearest:.2} apart at {:.0?}, ×{:.2} their own gap {gap:.2} (canvas units)",
                            pa.name,
                            pb.name,
                            at.unwrap_or_default(),
                            nearest / gap
                        ));
                    }
                }
            }
        }
        // (j) one style, one ink gap.
        let mut styles: std::collections::BTreeMap<&str, Vec<f32>> = Default::default();
        for (i, p) in paths.iter().enumerate().filter(|(_, p)| p.inks.len() >= 3) {
            styles.entry(p.style.as_str()).or_default().push(own[i]);
        }
        for (style, gaps) in &styles {
            let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
            let worst = gaps.iter().map(|g| (g / mean - 1.0).abs()).fold(0.0, f32::max);
            if worst > 0.10 {
                faults.push(format!(
                    "(j) {name} {style}: {} paths' ink gaps within {:.1}% of their mean {mean:.2}",
                    gaps.len(),
                    worst * 100.0
                ));
            }
        }
    }
    assert!(
        measured > 0 && fanned > 0 && held > 0,
        "precondition: ends are measured ({measured}), some fanned ({fanned}) and some held at the cap ({held})"
    );
    faults.sort();
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

/// Testament's paths [`test_maps_draw_no_path_through_another_node`]
/// finds still passing through another node, each by name.
const THROUGH: &[&str] = &[
    "3>4", "4>5", "6>235", "12>237", "23>26", "29>30", "29>31", "29>33", "29>39", "29>40", "29>41", "29>42",
    "50>85", "52>53", "52>54", "52>55", "52>61", "52>62", "52>63", "52>64", "74>75", "74>76", "74>77",
    "74>78", "74>79", "74>80", "74>81", "74>82", "74>83", "112>113", "122>123", "135>140", "141>148",
    "150>151", "150>153", "154>155", "163>168", "190>191", "190>192", "190>193", "199>200", "199>201",
    "199>202", "199>203", "199>204", "199>205", "199>206", "208>209", "208>210", "208>211", "208>217",
    "208>218", "208>219", "220>221", "220>227", "227>230", "227>231", "235>236", "241>243",
];

/// No connector or line link passes through another node (the owner's
/// answer on #326, which fixes the maps rather than routing round a
/// node): no glyph of a path lies inside the reach — the box round the
/// frame's inks, or the drawn rectangle — of a node other than its own
/// two, on the demo maps, testament and theme_demo. `stress_long_edges`
/// is left out: its long edges cross the rows they span by design.
///
/// Testament's paths that still pass through another node are listed in
/// [`THROUGH`], each by name: rows of children too near their parent,
/// whose paths to the far ones cross the near ones, and the like. A
/// listed path that passes through none is a fault too, so the list only
/// shrinks as the map is laid out again.
///
/// Control: with Border Toolkit's children back in the two rows they had,
/// the second row straight under the first, red on testament: the paths
/// to the second row pass through the first.
#[test]
fn test_maps_draw_no_path_through_another_node() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps");
    let mut maps = demo_maps();
    for name in ["testament", "theme_demo"] {
        maps.push((name.to_string(), root.join(format!("{name}.mandala.json"))));
    }
    let mut faults: Vec<String> = Vec::new();
    let mut checked = 0;
    for (name, path) in maps {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let (paths, frames, _) = drawn_paths(&doc);
        let reaches: Vec<(NodeId, Ink)> = frames
            .iter()
            .filter(|(_, inks)| !inks.is_empty())
            .map(|(node, inks)| (*node, ink_bounds(inks)))
            .collect();
        for p in &paths {
            let through = p.inks.iter().find_map(|ink| {
                reaches
                    .iter()
                    .find(|(node, reach)| !p.ends.contains(node) && ink.overlap(reach) > 0.0)
                    .map(|(node, _)| (ink.center(), *node))
            });
            checked += p.inks.len();
            let listed = name == "testament" && THROUGH.contains(&p.name.as_str());
            match (through, listed) {
                (Some((at, node)), false) => {
                    faults.push(format!(
                        "{name} {}: a glyph at {at:.0?} inside node {node:?}'s reach",
                        p.name
                    ));
                }
                (None, true) => faults.push(format!(
                    "{name} {}: listed in THROUGH, and passes through no node: remove it",
                    p.name
                )),
                _ => {}
            }
        }
    }
    assert!(checked > 0, "precondition: path glyphs are read");
    assert!(
        faults.is_empty(),
        "{} paths pass through another node:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
