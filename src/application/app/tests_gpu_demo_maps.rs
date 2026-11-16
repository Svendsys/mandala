// SPDX-License-Identifier: MPL-2.0

//! The README's demo maps (`maps/demos/*.mandala.json`, #291), drawn by
//! the real renderer (TEST_CONVENTIONS §T8), two ways.
//!
//! **Each map as it is saved**, through a projection of this file's
//! own, the committed path the README images are made by: each node's
//! sections as boxes, each frame's rings round the node's outline with
//! the ring's resolved fit, span, sides and spacing (`RingLayout::of`),
//! and every run drawn at its node's opacity times its ancestors'. Each
//! connector and line link is the projector's own
//! (`Projector::project_full`, #296): its ends, its curve through its
//! control points and its glyphs as the app lays them. The rest is a
//! test's projection, not the app's: it reads what the demo maps hold
//! and nothing more (no palettes' own projection, no portal ends), on
//! the map's one background: the frame is cleared to it (V1-B12), and a
//! demo map gives none, so it is black. It plays no presentation: what
//! it draws is the map with nothing playing.
//!
//! **The maps that carry presentations, with them playing** (V1-D2,
//! #251): a per-glyph effect — a word swelling, letters lifting, a
//! trail fading — is a glyph-layer presentation, stored with the map
//! (`format/presentations.md`). `frames` and `dickinson` carry theirs,
//! and
//! [`test_demo_presentations_play_on_the_words_they_name_at_a_fixed_time`]
//! draws them through the app itself: the app loads the map and
//! projects it with its own projector, the console starts the
//! presentations, the clock is stepped to a fixed time, and the frame
//! is the app's.
//!
//! Every assertion is structural, never a pixel (§T10): each demo map
//! loads, every ring and connector it holds is placed with no glyph
//! lost, and the frame drawn holds a run for each, read back from the
//! renderer. With `MANDALA_DEMO_OUT` set to a directory, each map's
//! frame is also written there as `<name>.png` — the README images —
//! and a presented map's as `<name>-presented.png`:
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
use baumhard::document::values::{Align, FitMode, FontSize, LinkKind};
use baumhard::engine::tree::Placement;
use baumhard::flow::{
    offset, outline, place_on_closed, ring_offset, shape_box, shape_run, FlowSpan, PlacedRun, Rect,
    RingLayout, ShapedRun,
};
use baumhard::font::fonts::{self, AppFont, FontSystemLease};
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::{glyph_mutations, GlyphLayers, Layer, Projector};
use glam::Vec2;

use super::gpu_harness::headless_app;
use super::run_native_init::InitState;
use crate::application::renderer::{CaptureFailure, CapturedPixels, FrameSkip, ModalLayer};

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

/// Run `f` under the font guard, released before the result returns.
pub(super) fn with_fonts<R>(f: impl FnOnce(&mut FontSystemLease) -> R) -> R {
    fonts::init();
    let mut fs = FontSystemLease::new("app::tests_gpu_demo_maps");
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

/// Where each glyph of `run` is laid out on the canvas: the run's origin
/// and each glyph's place in its shaping, before an override moves it.
fn laid_out(run: &GlyphRunData) -> Vec<Vec2> {
    let origin = run.origin();
    match run.shaped() {
        RunShape::Box(shaped) => shaped
            .buffer
            .layout_runs()
            .flat_map(|line| {
                let top = line.line_top;
                line.glyphs
                    .iter()
                    .map(move |glyph| origin + Vec2::new(glyph.x, top))
            })
            .collect(),
        RunShape::Path { placed, .. } => placed.glyphs.iter().map(|glyph| origin + glyph.pos).collect(),
    }
}

/// Every element below `id` and `id` itself, parents first.
fn subtree_of(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    while let Some(at) = stack.pop() {
        out.push(at);
        stack.extend(doc.children(at));
    }
    out
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
pub(super) fn project(doc: &Document, fs: &mut FontSystemLease) -> Projected {
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
    }
    // Each connector and line link as the projector the app draws with
    // lays it (`Projector::project_full`): its ends, its curve and its
    // glyphs are the projector's own, moved to where this projection
    // draws the map.
    let mut layers = GlyphLayers::new();
    let projector = Projector::project_full(doc, &mut layers, fs);
    let ident = |n: NodeId| match doc.get(n) {
        Some(DocElement::Node(data)) => data.ident.0,
        _ => 0,
    };
    let connections = layers.tree(Layer::Connections);
    for glyph in connections.root().descendants(connections.arena()) {
        let Some(run) = connections.arena()[glyph].get().glyph_run() else {
            continue;
        };
        let Some(path) = projector.element_of(Layer::Connections, glyph) else {
            continue;
        };
        let Some(own) = doc.parent(path) else {
            continue;
        };
        let ends = match doc.get(path) {
            Some(DocElement::Connector(_)) => doc.parent(own).map(|up| (up, own)),
            Some(DocElement::Link(l)) if l.kind == LinkKind::Line => Some((own, l.to)),
            _ => None,
        };
        let Some((from, to)) = ends else {
            continue;
        };
        // The faces it asks for: its spans', or its text's style's.
        match resolve_path(tree, path, &mut ()).content {
            Content::Spans => out.pin(spans_of(doc, path, 0.0).iter().map(|(_, style)| style.font)),
            Content::Text(_) => out.pin([resolve_text(tree, path, &mut ()).font]),
        }
        let moved = GlyphRunData::new(
            run.shaped().clone(),
            run.origin() + FAR,
            run.overrides().clone(),
            run.opacity(),
            run.zoom(),
        )
        .expect("the projector's own overrides");
        out.labels.push(format!("path {}>{}", ident(from), ident(to)));
        out.runs.push(moved);
        out.paths += 1;
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
    // The runs go in as the chrome layer, over no glyph layers at all,
    // so the frame draws exactly them through the layer renderer.
    tree.layer = baumhard::projection::Layer::ALL.len();
    init.renderer
        .load_view(&baumhard::projection::GlyphLayers::new(), &tree);
}

fn draw(init: &mut InitState, camera: &Camera2D) -> CapturedPixels {
    let deadline = Instant::now() + baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    loop {
        match init
            .renderer
            .capture_frame(camera, ModalLayer::Omitted, &mut || {})
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
    let harness = format!("{}/maps/testament.mandala.json", env!("CARGO_MANIFEST_DIR"));
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

/// What a demo map's presentations are held to: the node they are
/// started on, the frame's time on their timeline, and, per
/// presentation in start order, its layer, its progress then and — on
/// the glyph layer — the words each of its ranges names.
struct Presented {
    map: &'static str,
    node: u64,
    at_ms: u64,
    presentations: &'static [(&'static str, &'static str, f32, &'static [&'static str])],
}

/// The README's two presented maps (#251, the acceptance cases moved
/// from #250). Frames: the words "an ellipse" of the ellipse's ring, at
/// 1.15 times their size. Dickinson, 1200 ms on: "Hope" three quarters
/// of the way through its swell (`f²(3 - 2f)` at `f = 0.75`), the line
/// held where centering put it and the trail of dots written after
/// "feathers" (two document-layer presentations: the map saves the line
/// centered and the span empty), and the letters of "feathers" and the
/// trail half way through their lift in time and three quarters in
/// progress (`1 - (1 - f)²` at `f = 0.5`).
const PRESENTED: [Presented; 2] = [
    Presented {
        map: "frames",
        node: 2,
        at_ms: 0,
        presentations: &[("an-ellipse", "glyph", 1.0, &["an ellipse"])],
    },
    Presented {
        map: "dickinson",
        node: 1,
        at_ms: 1200,
        presentations: &[
            ("hope-swells", "glyph", 0.84375, &["Hope"]),
            ("feathers-line", "document", 1.0, &[]),
            ("feathers-trail", "document", 1.0, &[]),
            ("feathers-rest", "glyph", 1.0, &["feathers", "·····"]),
            (
                "feathers-lift",
                "glyph",
                0.75,
                &[
                    "f", "f", "f", "e", "e", "e", "a", "a", "a", "t", "t", "t", "h", "h", "h", "e", "e", "e",
                    "r", "r", "r", "s", "s", "s", "·", "·", "·", "·", "·", "·", "·", "·", "·", "·",
                ],
            ),
        ],
    },
];

/// The text the placed glyphs `range` of `run` were shaped from, read
/// off the run's own record of which span and which bytes each glyph
/// came from.
fn text_of(run: &GlyphRunData, spans: &[String], range: std::ops::Range<usize>) -> String {
    let piece = |span: usize, bytes: &std::ops::Range<usize>| {
        spans
            .get(span)
            .and_then(|text| text.get(bytes.clone()))
            .unwrap_or("\u{fffd}")
            .to_string()
    };
    match run.shaped() {
        RunShape::Box(shaped) => range
            .filter_map(|glyph| shaped.glyph_spans.get(glyph))
            .map(|(span, bytes)| piece(*span, bytes))
            .collect(),
        RunShape::Path { run, placed } => range
            .filter_map(|glyph| placed.glyphs.get(glyph))
            .filter_map(|placed| run.glyphs.get(placed.glyph as usize))
            .map(|glyph| piece(glyph.span, &glyph.bytes))
            .collect(),
    }
}

/// **The demo maps' presentations are drawn through the path the app
/// runs a presentation by, at a fixed time on their timeline**: the
/// real app loads the map and projects it, the console's `present`
/// verb starts each presentation the map carries, the clock is stepped
/// to the frame's time, and the frame is drawn. (What starts them when
/// a map opens is a trigger, V1-D3's; until then the console is the
/// route a person has.)
///
/// Held: the map carries exactly the presentations listed; with
/// nothing playing no span holds Dickinson's trail and none is drawn;
/// what plays moves no glyph of the run that was there before it (on
/// Dickinson, `feathers-line` holds the line where centering put it
/// while `feathers-trail` writes after it), read off the run's own
/// layout and so for any face; every range of every run mutation names
/// the words it is for, read off the shaped run; each instance is at the
/// progress its timing puts it at; and the run on the glyph layer
/// carries each scale at that progress, with no other glyph of it
/// scaled. With `MANDALA_DEMO_OUT`
/// set, the frame is written there as `<name>-presented.png`.
///
/// Controls, each seen red: a range moved by one glyph names another
/// word; with the glyph-layer stage not stamping what plays, the run
/// carries no scale; the trail saved in the map's span, as it once was,
/// is drawn with nothing playing; `feathers-line`'s offset one unit off
/// moves glyph 0 of the line.
#[test]
fn test_demo_presentations_play_on_the_words_they_name_at_a_fixed_time() {
    use baumhard::document::presentation::PresentationBody;
    use baumhard::gfx_structs::mutator::GfxMutation;
    use baumhard::gfx_structs::run::RunOverrideOp;
    use serde_json::json;

    use super::ipc_target::IpcCommandTarget;
    use super::step::Beat;
    use crate::application::document::selection::every_node;
    use crate::application::ipc::run_attached;

    let out = std::env::var_os(OUT_ENV).map(PathBuf::from);
    for case in PRESENTED {
        let name = case.map;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(DEMOS)
            .join(format!("{name}.mandala.json"));
        let Some(mut app) = headless_app(path.to_str().expect("a UTF-8 path")) else {
            return;
        };
        let init = &mut app.init;
        init.input_context().drain_frame(Beat::Loop);
        let ipc = |init: &mut InitState, request: serde_json::Value| {
            let reply = run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0;
            assert_eq!(reply["ok"], true, "{name}: {request}: {reply}");
            reply["result"].clone()
        };

        // The map carries these, and no other.
        let carried: Vec<String> = init
            .document
            .as_ref()
            .expect("the map loaded")
            .doc()
            .definitions()
            .presentations
            .iter()
            .map(|def| def.id.clone())
            .collect();
        let listed: Vec<&str> = case.presentations.iter().map(|(id, _, _, _)| *id).collect();
        assert_eq!(carried, listed, "{name}: the presentations the map carries");
        // With nothing playing the map draws, and holds, no text of a
        // presentation's: Dickinson's trail of five dots is written by
        // one, into a span the map saves empty. (The frames map's
        // ornaments are single dots of its own.)
        let found = |init: &mut InitState| {
            let reply = ipc(init, json!({"id": 1, "cmd": "scene.find_text", "text": "·····"}));
            reply["matches"].as_array().map_or(0, Vec::len)
        };
        assert_eq!(found(init), 0, "{name}: nothing playing, no trail is drawn");
        let doc = init.document.as_ref().expect("the map loaded").doc();
        let held: Vec<String> = every_node(doc)
            .flat_map(|node| children_of(doc, node, DocKind::Section))
            .flat_map(|section| children_of(doc, section, DocKind::Span))
            .filter_map(|span| match doc.get(span) {
                Some(DocElement::Span(span)) if span.text.contains("·····") => {
                    Some(span.text.to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            held,
            Vec::<String>::new(),
            "{name}: the map itself holds no trail"
        );

        // Where every run of the node's elements is laid out now, with
        // nothing playing, by element.
        let laid_before: Vec<(NodeId, Vec<Vec2>)> = {
            let view = init.view.as_ref().expect("the app has a view");
            let doc = init.document.as_ref().expect("the map loaded").doc();
            let node = doc
                .node_by_ident(baumhard::document::ids::NodeIdent(case.node as u32))
                .expect("the node the presentations start on");
            subtree_of(doc, node)
                .into_iter()
                .filter_map(|element| {
                    let (layer, glyph) = *view.projector().glyphs_of(element).first()?;
                    let run = view.layers().tree(layer).get(glyph)?.get().glyph_run()?;
                    Some((element, laid_out(run)))
                })
                .collect()
        };

        for id in &listed {
            ipc(
                init,
                json!({"id": 1, "cmd": "act.console", "line": format!("present start {id} {}", case.node)}),
            );
        }
        if case.at_ms > 0 {
            ipc(init, json!({"id": 1, "cmd": "clock.step", "ms": case.at_ms}));
        }
        init.input_context().drain_frame(Beat::Loop);
        let playing = ipc(init, json!({"id": 1, "cmd": "query.animations"}));
        for (at, (id, layer, t, _)) in case.presentations.iter().enumerate() {
            let instance = &playing["presentations"][at];
            assert_eq!(
                (&instance["presentation"], &instance["layer"], &instance["node"]),
                (&json!(id), &json!(layer), &json!(case.node)),
                "{name}: {instance}"
            );
            let reported = instance["t"].as_f64().expect("t") as f32;
            assert!(
                (reported - t).abs() < 1e-6,
                "{name}: `{id}` is at {reported}, not {t}"
            );
        }

        // The one run they play on, and the text it was shaped from.
        let view = init.view.as_ref().expect("the app has a view");
        let doc = init.document.as_ref().expect("the map loaded").doc();
        let based = view.stage().based_runs();
        assert_eq!(based.len(), 1, "{name}: they play on one run: {based:?}");
        let (layer, glyph) = based[0];
        let element = view
            .projector()
            .element_of(layer, glyph)
            .expect("the run is an element's");
        // What plays moves no glyph of the run that was there before it
        // started: on Dickinson, the line is held where centering put it
        // while the trail is written after it. Read off the run's own
        // layout, so it holds for any face.
        let before = laid_before
            .iter()
            .find(|(of, _)| *of == element)
            .map(|(_, laid)| laid.clone())
            .expect("the run was laid out before anything played");
        {
            let run = view
                .layers()
                .tree(layer)
                .get(glyph)
                .and_then(|node| node.get().glyph_run())
                .expect("a run");
            let now = laid_out(run);
            assert!(
                now.len() >= before.len(),
                "{name}: {} glyphs, then {}",
                before.len(),
                now.len()
            );
            for (at, (was, is)) in before.iter().zip(&now).enumerate() {
                assert!(
                    (*is - *was).length() < 1e-3,
                    "{name}: glyph {at} was laid out at {was}, and is at {is} while it plays"
                );
            }
        }
        let spans: Vec<String> = children_of(doc, element, DocKind::Span)
            .into_iter()
            .filter_map(|span| match doc.get(span) {
                Some(DocElement::Span(span)) => Some(span.text.to_string()),
                _ => None,
            })
            .collect();
        let run = view
            .layers()
            .tree(layer)
            .get(glyph)
            .and_then(|node| node.get().glyph_run())
            .expect("a run");

        // Every range names the words it is for; and the scales and the
        // offsets the run should carry now, glyph by glyph, with which
        // glyphs a color was given.
        let mut scales = vec![1.0_f32; run.shaped().glyph_count()];
        let mut offsets = vec![Vec2::ZERO; scales.len()];
        let mut colored = vec![false; scales.len()];
        for (def, (id, layer, t, words)) in doc.definitions().presentations.iter().zip(case.presentations) {
            let PresentationBody::Glyph(ops) = &def.body else {
                assert_eq!(*layer, "document", "{name}: `{id}` is a glyph-layer presentation");
                continue;
            };
            let mut named = Vec::new();
            for mutation in glyph_mutations(ops) {
                let GfxMutation::RunOverrides(op) = mutation else {
                    panic!("{name}: `{id}` writes overrides only: {mutation:?}");
                };
                let range = match &op {
                    RunOverrideOp::Offset { range, .. }
                    | RunOverrideOp::Color { range, .. }
                    | RunOverrideOp::Scale { range, .. } => range.clone(),
                    RunOverrideOp::Clear => panic!("{name}: `{id}` clears nothing"),
                };
                assert!(
                    range.end <= scales.len(),
                    "{name}: `{id}`: {range:?} is inside the run's {} glyphs",
                    scales.len()
                );
                named.push(text_of(run, &spans, range.clone()));
                for glyph in range {
                    match &op {
                        RunOverrideOp::Scale { value, .. } => scales[glyph] = 1.0 + (value - 1.0) * t,
                        RunOverrideOp::Offset { value, .. } => offsets[glyph] = *value * *t,
                        RunOverrideOp::Color { .. } => colored[glyph] = true,
                        RunOverrideOp::Clear => {}
                    }
                }
            }
            assert_eq!(named, *words, "{name}: the words `{id}`'s ranges name");
        }
        let carried = run.overrides();
        assert_eq!(carried.len(), scales.len(), "{name}: an override a glyph");
        for glyph in 0..scales.len() {
            let (scale, offset, color) = (
                carried.scale()[glyph],
                carried.offset()[glyph],
                carried.color()[glyph],
            );
            assert!(
                (scale - scales[glyph]).abs() < 1e-5,
                "{name}: glyph {glyph} is drawn at {scale} times its size, not {}",
                scales[glyph]
            );
            assert!(
                (offset - offsets[glyph]).length() < 1e-4,
                "{name}: glyph {glyph} is moved by {offset}, not {}",
                offsets[glyph]
            );
            assert_eq!(
                color.is_some(),
                colored[glyph],
                "{name}: glyph {glyph} has a color of a presentation's: {color:?}"
            );
        }
        assert!(
            scales.iter().any(|scale| *scale != 1.0),
            "{name}: precondition: something is scaled"
        );

        // And the frame is drawn, through the app's own camera fitted to
        // the map.
        ipc(init, json!({"id": 1, "cmd": "act.action", "action": "ZoomFit"}));
        init.input_context().drain_frame(Beat::Loop);
        let camera = *init.renderer.camera();
        let pixels = draw(init, &camera);
        let counts = init.renderer.glyph_run_counts();
        assert!(
            counts.runs_drawn > 0 && counts.instances > 0,
            "{name}: drawn: {counts:?}"
        );
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            let file = dir.join(format!("{name}-presented.png"));
            let mut handle =
                std::fs::File::create(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            crate::application::capture::png::encode(&pixels, &mut handle, &mut || {})
                .unwrap_or_else(|e| panic!("{}: {e:?}", file.display()));
            eprintln!("{name}: wrote {}", file.display());
        }
    }
}
