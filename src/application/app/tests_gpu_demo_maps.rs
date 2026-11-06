// SPDX-License-Identifier: MPL-2.0

//! The README's demo maps (`maps/demos/*.mandala.json`, #291), drawn by
//! the real renderer (TEST_CONVENTIONS §T8) through a projection of
//! their own, the committed path the README images are made by.
//!
//! Nothing in the app projects a document into glyph runs yet (the
//! projection is V1-B7, its wiring V1-C3), so this file holds the least
//! one the demo maps need: each node's sections as boxes, each frame's
//! rings round the node's outline with the ring's resolved fit, span,
//! sides and spacing (`RingLayout::of`), each connector and line link as
//! a straight run between the two nodes' outermost rings, and every run
//! drawn at its node's opacity times its ancestors'. It is a test's
//! projection, not the app's: it reads what the demo maps hold and
//! nothing more (no palettes' own projection, no labels, caps or portal
//! ends), on the map's one background: the frame is cleared to it
//! (V1-B12), and a demo map gives none, so it is black. What a demo map shows is what the format spells: a per-glyph
//! override (a glyph's offset, scale or color, `format/mutations.md`
//! "Glyph run mutations") has no spelling in a `.mandala.json`
//! yet, so no demo map carries one and this projection applies none.
//!
//! Every assertion is structural, never a pixel (§T10): each demo map
//! loads, every ring and connector it holds is placed with no glyph
//! lost, and the frame drawn holds a run for each, read back from the
//! renderer. With `MANDALA_DEMO_OUT` set to a directory, each map's
//! frame is also written there as `<name>.png` — the README images:
//!
//! ```sh
//! MANDALA_DEMO_OUT=target/demos cargo test -p mandala demo_maps
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::format::load;
use baumhard::document::ids::Document;
use baumhard::document::resolve::{
    resolve_background, resolve_frame, resolve_node, resolve_path, resolve_ring, resolve_text, Content,
    ResolvedText,
};
use baumhard::document::values::{Align, FitMode, FontSize};
use baumhard::engine::tree::Placement;
use baumhard::flow::{
    offset, outline, place_on_closed, place_on_open, ring_offset, shape_box, shape_run, Curve, FlowSpan,
    Outline, PlacedRun, Rect, RingLayout, ShapedRun,
};
use baumhard::font::fonts::{self, acquire_font_system_write, AppFont, FontSystemWriteGuard};
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use baumhard::gfx_structs::tree::GfxTree;
use glam::Vec2;

use super::gpu_harness::headless_app;
use super::run_native_init::InitState;
use crate::application::renderer::{CanvasSource, CaptureFailure, CapturedPixels, FrameSkip, ModalLayer};
use crate::application::scene_host::CanvasRole;

/// The directory the demo maps live in.
const DEMOS: &str = "maps/demos";

/// The directory to write each map's PNG to, when set.
const OUT_ENV: &str = "MANDALA_DEMO_OUT";

/// The README images' size.
const SIZE: (u32, u32) = (1600, 900);

/// How much of the frame the maps' content may take, each way: the rest
/// is margin.
const FILL: f32 = 0.94;

/// Where the maps are drawn: a stretch of canvas far from every node of
/// the harness's own map, so the frame shows the demo and nothing else.
const FAR: Vec2 = Vec2::new(100_000.0, 100_000.0);

/// The space between a frame's outermost ring and a connector's end.
const RING_MARGIN: f32 = 6.0;

/// What a map projects to: its runs, the canvas they cover, and what
/// was placed and lost on the way.
pub(super) struct Projected {
    /// Every run, in draw order: sections, rings, then paths.
    pub(super) runs: Vec<GlyphRunData>,
    /// The canvas the nodes and their rings cover, `(min, max)`.
    pub(super) bounds: (Vec2, Vec2),
    /// How many rings and paths were placed.
    rings: usize,
    paths: usize,
    /// Each ring or path that lost glyphs, by what it was.
    lost: Vec<String>,
    /// What each run is, in `runs`' order: `section N.k`, `frame N.k`
    /// (a ring of text), `ring N.k` (a `repeat` ring) or `path N>M`, by
    /// node id and place.
    pub(super) labels: Vec<String>,
    /// The face every span shaped asks for, and every ring's own (its
    /// sides' face), each once: what the map pins, `AppFont::Any` for a
    /// span that pins nothing.
    pub(super) fonts: Vec<AppFont>,
}

impl Projected {
    /// Record `fonts` in [`Projected::fonts`], each once.
    fn pin(&mut self, fonts: impl IntoIterator<Item = AppFont>) {
        for font in fonts {
            if !self.fonts.contains(&font) {
                self.fonts.push(font);
            }
        }
    }
}

/// A node as drawn: its outline, and how far past it its outermost
/// ring reaches.
struct Drawn {
    /// The node's id in the map.
    ident: u32,
    outline: Outline,
    reach: f32,
    /// The alpha it draws at ([`opacity_of`]).
    alpha: f32,
}

/// Run `f` under the font guard, released before the result returns.
pub(super) fn with_fonts<R>(f: impl FnOnce(&mut FontSystemWriteGuard) -> R) -> R {
    fonts::init();
    let mut fs = acquire_font_system_write("app::tests_gpu_demo_maps");
    f(&mut fs)
}

/// `style` with a relative size made points against `side`, a node's
/// shorter side, as a frame's sizes are (`format/schema.md`, "Size
/// units").
fn in_points(mut style: ResolvedText, side: f32) -> ResolvedText {
    if let FontSize::Rel(fraction) = style.size {
        style.size = FontSize::Pt(fraction * side);
    }
    style
}

/// The children of `id` of `kind`, in their row's order.
fn children_of(doc: &Document, id: NodeId, kind: DocKind) -> Vec<NodeId> {
    doc.children(id)
        .filter(|&c| doc.get(c).is_some_and(|e| e.kind() == kind))
        .collect()
}

/// Every node of `doc`, parents before children.
fn nodes_of(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if doc.get(id).is_some_and(|e| e.kind() == DocKind::Node) {
            out.push(id);
        }
        let mut kids: Vec<NodeId> = doc.children(id).collect();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

/// The spans under `owner`, each with its resolved style made points
/// against `side`.
fn spans_of(doc: &Document, owner: NodeId, side: f32) -> Vec<(String, ResolvedText)> {
    children_of(doc, owner, DocKind::Span)
        .into_iter()
        .filter_map(|span| match doc.get(span) {
            Some(DocElement::Span(data)) => Some((
                data.text.clone(),
                in_points(resolve_text(doc.tree(), span, &mut ()), side),
            )),
            _ => None,
        })
        .collect()
}

fn flow_spans(spans: &[(String, ResolvedText)]) -> Vec<FlowSpan<'_>> {
    spans
        .iter()
        .map(|(text, style)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect()
}

/// A path run drawn at `opacity`.
fn path_run(run: ShapedRun, placed: PlacedRun, opacity: f32) -> GlyphRunData {
    GlyphRunData::new(
        RunShape::Path {
            run: Arc::new(run),
            placed: Arc::new(placed),
        },
        Vec2::ZERO,
        GlyphOverrides::none(),
        opacity,
        None,
    )
    .expect("no overrides")
}

/// Whether `p` lies inside `o`.
fn inside(o: &Outline, p: Vec2) -> bool {
    match *o {
        Outline::Ellipse { rect } => {
            let q = (p - rect.center()) / (rect.size * 0.5);
            q.length_squared() <= 1.0
        }
        Outline::RoundedRect { rect, radius } => {
            let q = (p - rect.center()).abs() - (rect.size * 0.5 - Vec2::splat(radius));
            q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - radius <= 0.0
        }
    }
}

/// Where a ray from `node`'s center toward `toward` leaves its outline
/// grown past its outermost ring: where a path to it ends.
fn end_on(node: &Drawn, toward: Vec2) -> Vec2 {
    let grown = offset(&node.outline, node.reach + RING_MARGIN);
    let center = node.outline.rect().center();
    let dir = (toward - center).normalize_or_zero();
    let (mut low, mut high) = (0.0_f32, 1.0e6_f32);
    for _ in 0..64 {
        let mid = 0.5 * (low + high);
        if inside(&grown, center + dir * mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    center + dir * low
}

/// The alpha `node` draws at: its `opacity` times every ancestor
/// node's, as a node's opacity multiplies everything it and its subtree
/// draw (`format/schema.md`, "Node").
fn opacity_of(doc: &Document, node: NodeId) -> f32 {
    let mut opacity = 1.0;
    let mut at = Some(node);
    while let Some(id) = at {
        if let Some(DocElement::Node(data)) = doc.get(id) {
            opacity *= data.opacity;
        }
        at = doc.parent(id);
    }
    opacity
}

/// Project `doc`: every node's sections and frame, then every
/// connector and line link.
pub(super) fn project(doc: &Document, fs: &mut FontSystemWriteGuard) -> Projected {
    let tree = doc.tree();
    let mut out = Projected {
        runs: Vec::new(),
        bounds: (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        rings: 0,
        paths: 0,
        lost: Vec::new(),
        labels: Vec::new(),
        fonts: Vec::new(),
    };
    let mut drawn: Vec<(NodeId, Drawn)> = Vec::new();
    for node in nodes_of(doc) {
        let Some(DocElement::Node(data)) = doc.get(node) else {
            continue;
        };
        let origin = FAR + data.position;
        let alpha = opacity_of(doc, node);
        // The sections, each a box at its offset; the node is as large
        // as its minimum size and every box.
        let (mut min, mut max) = (origin, origin + data.min_size.unwrap_or(Vec2::ZERO));
        for (k, section) in children_of(doc, node, DocKind::Section).into_iter().enumerate() {
            let Some(DocElement::Section(s)) = doc.get(section) else {
                continue;
            };
            let spans = spans_of(doc, section, 0.0);
            out.pin(spans.iter().map(|(_, style)| style.font));
            let shaped = shape_box(
                &flow_spans(&spans),
                s.min_size,
                s.align.unwrap_or(Align::Left),
                fs,
            );
            let at = origin + s.offset;
            min = min.min(at);
            max = max.max(at + shaped.size);
            let run = GlyphRunData::new(
                RunShape::Box(Arc::new(shaped)),
                at,
                GlyphOverrides::none(),
                alpha,
                None,
            )
            .expect("no overrides");
            out.labels.push(format!("section {}.{k}", data.ident.0));
            out.runs.push(run);
        }
        let rect = Rect::new(min, max - min);
        let base = outline(rect, resolve_node(tree, node, &mut ()).shape);
        let mut reach = 0.0;
        for frame in children_of(doc, node, DocKind::Frame) {
            let f = resolve_frame(tree, frame, &mut ());
            if !f.visible {
                continue;
            }
            let side = rect.size.x.min(rect.size.y);
            let mut heights: Vec<f32> = Vec::new();
            for ring in children_of(doc, frame, DocKind::Ring) {
                let spans = spans_of(doc, ring, side);
                if spans.is_empty() {
                    continue;
                }
                let d = ring_offset(f.inset, f.gap, &heights);
                let path = offset(&base, d);
                let style = in_points(resolve_text(tree, ring, &mut ()), side);
                out.pin(spans.iter().map(|(_, style)| style.font).chain([style.font]));
                let resolved = resolve_ring(tree, ring, &mut ());
                let shaped = shape_run(&flow_spans(&spans), 1.0, fs);
                let (rescaled, placed) = place_on_closed(
                    &shaped,
                    &path,
                    f.start,
                    f.direction,
                    &resolved.fit,
                    &RingLayout::of(&resolved),
                    true,
                    &style,
                    fs,
                );
                if placed.truncated {
                    out.lost
                        .push(format!("node {} ring {}", data.ident.0, heights.len()));
                }
                let run = rescaled.unwrap_or(shaped);
                heights.push(run.line_height);
                reach = d + run.line_height;
                let kind = if matches!(resolved.fit, FitMode::Repeat) {
                    "ring"
                } else {
                    "frame"
                };
                out.labels
                    .push(format!("{kind} {}.{}", data.ident.0, heights.len() - 1));
                out.runs.push(path_run(run, placed, alpha));
                out.rings += 1;
            }
        }
        let grown = offset(&base, reach).rect();
        out.bounds = (
            out.bounds.0.min(grown.origin),
            out.bounds.1.max(grown.origin + grown.size),
        );
        drawn.push((
            node,
            Drawn {
                ident: data.ident.0,
                outline: base,
                reach,
                alpha,
            },
        ));
    }
    let find = |id: NodeId| drawn.iter().find(|(n, _)| *n == id).map(|(_, d)| d);
    for (node, child) in &drawn {
        // Its connector from its parent node, and its line links: drawn
        // at the alpha of the node they belong to.
        let mut paths: Vec<(NodeId, &Drawn, &Drawn)> = Vec::new();
        if let Some(parent) = doc.parent(*node).and_then(find) {
            for connector in children_of(doc, *node, DocKind::Connector) {
                if let Some(DocElement::Connector(c)) = doc.get(connector) {
                    if c.path.visible {
                        paths.push((connector, parent, child));
                    }
                }
            }
        }
        for link in children_of(doc, *node, DocKind::Link) {
            if let Some(DocElement::Link(l)) = doc.get(link) {
                if l.kind == baumhard::document::values::LinkKind::Line && l.path.visible {
                    if let Some(to) = find(l.to) {
                        paths.push((link, child, to));
                    }
                }
            }
        }
        for (path, from, to) in paths {
            let points = match doc.get(path) {
                Some(DocElement::Connector(c)) => c.path.control_points.clone(),
                Some(DocElement::Link(l)) => l.path.control_points.clone(),
                _ => Vec::new(),
            };
            // TODO(#296): a connector's ends, its pitch and a shared
            // side's splay are #296's. Until then each path runs from
            // where the ray between the two nodes' centers leaves the
            // one's outermost ring to where it meets the other's, its
            // control points offsets from those ends, at the path's own
            // spacing.
            let (a, b) = (from.outline.rect().center(), to.outline.rect().center());
            let (p0, p1) = (end_on(from, b), end_on(to, a));
            let control: Vec<Vec2> = match points.as_slice() {
                [c] => vec![p0 + *c],
                [c1, c2] => vec![p0 + *c1, p1 + *c2],
                _ => Vec::new(),
            };
            let Some(curve) = Curve::from_points(p0, &control, p1) else {
                continue;
            };
            let resolved = resolve_path(tree, path, &mut ());
            let style = resolve_text(tree, path, &mut ());
            let spans = match &resolved.content {
                Content::Spans => spans_of(doc, path, 0.0),
                Content::Text(text) => vec![(text.clone(), style)],
            };
            out.pin(spans.iter().map(|(_, style)| style.font));
            let shaped = shape_run(&flow_spans(&spans), 1.0, fs);
            let (rescaled, placed) =
                place_on_open(&shaped, &curve, &resolved.fit, resolved.spacing, &style, fs);
            out.labels.push(format!("path {}>{}", from.ident, to.ident));
            out.runs
                .push(path_run(rescaled.unwrap_or(shaped), placed, child.alpha));
            out.paths += 1;
        }
    }
    out
}

/// The demo maps, by name, sorted.
pub(super) fn demo_maps() -> Vec<(String, PathBuf)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEMOS);
    let mut maps: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter_map(|path| {
            let name = path
                .file_name()?
                .to_str()?
                .strip_suffix(".mandala.json")?
                .to_string();
            Some((name, path))
        })
        .collect();
    maps.sort();
    maps
}

/// A camera on `bounds`, as large as `FILL` of the frame holds it.
pub(super) fn camera_on(bounds: (Vec2, Vec2)) -> Camera2D {
    let size = bounds.1 - bounds.0;
    let mut camera = Camera2D::new(SIZE.0, SIZE.1);
    camera.position = 0.5 * (bounds.0 + bounds.1);
    camera.zoom = FILL * (SIZE.0 as f32 / size.x).min(SIZE.1 as f32 / size.y);
    camera
}

fn show(init: &mut InitState, runs: Vec<GlyphRunData>) {
    let mut tree: GfxTree = GfxTree::new_non_indexed();
    for (i, run) in runs.into_iter().enumerate() {
        tree.insert_child(
            tree.root(),
            GfxElement::new_run(run, 0, 9_000 + i),
            Placement::Last,
        )
        .expect("a child of the root");
    }
    init.app_scene
        .register_canvas(CanvasRole::SectionFrames, tree, Vec2::ZERO);
    init.renderer.rebuild_canvas_scene_buffers(&mut init.app_scene);
}

fn draw(init: &mut InitState, camera: &Camera2D) -> CapturedPixels {
    let deadline = Instant::now() + baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    loop {
        match init
            .renderer
            .capture_frame(camera, ModalLayer::Omitted, CanvasSource::Live, &mut || {})
        {
            Ok(pixels) => return pixels,
            Err(CaptureFailure::Skipped(FrameSkip::FontSystemContended)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(other) => panic!("the frame was not drawn: {other:?}"),
        }
    }
}

#[test]
fn test_demo_maps_draw_every_ring_and_path() {
    let maps = demo_maps();
    assert!(
        maps.len() >= 4,
        "the README's demo maps must be found, else this would pass over none: {maps:?}"
    );
    let harness = format!("{}/maps/testament.mindmap.json", env!("CARGO_MANIFEST_DIR"));
    let Some(mut app) = headless_app(&harness) else {
        return;
    };
    let out = std::env::var_os(OUT_ENV).map(PathBuf::from);
    for (name, path) in maps {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = load(&text).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        let projected = with_fonts(|fs| project(&doc, fs));
        assert!(!projected.runs.is_empty(), "{name}: a demo map shows something");
        assert!(
            projected.lost.is_empty(),
            "{name}: rings lost glyphs: {:?}",
            projected.lost
        );
        let runs = projected.runs.len();
        assert_eq!(projected.labels.len(), runs, "{name}: every run is named");
        let placed: usize = projected.runs.iter().map(|run| run.shaped().glyph_count()).sum();
        let camera = camera_on(projected.bounds);
        let init = &mut app.init;
        init.renderer
            .set_background(resolve_background(doc.tree(), &mut ()).0);
        show(init, projected.runs);
        let pixels = draw(init, &camera);
        let counts = init.renderer.glyph_run_counts();
        assert_eq!(
            (counts.runs, counts.runs_drawn),
            (runs, runs),
            "{name}: every run handed in is drawn: {counts:?}"
        );
        assert!(counts.instances > 0, "{name}: glyphs drawn: {counts:?}");
        eprintln!(
            "{name}: {runs} runs ({}), {} rings, {} paths, {placed} glyphs placed, {counts:?}",
            projected.labels.join(", "),
            projected.rings,
            projected.paths
        );
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            let file = dir.join(format!("{name}.png"));
            let mut handle =
                std::fs::File::create(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            crate::application::capture::png::encode(&pixels, &mut handle, &mut || {})
                .unwrap_or_else(|e| panic!("{}: {e:?}", file.display()));
            eprintln!("{name}: wrote {}", file.display());
        }
    }
}
