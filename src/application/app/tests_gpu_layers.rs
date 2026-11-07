// SPDX-License-Identifier: MPL-2.0

//! GPU-backed tests (TEST_CONVENTIONS §T8) of the layer renderer
//! (`renderer::layers`, design §13.1, V1-C2 #243): documents projected by
//! the real projector into glyph layers, loaded into the real renderer
//! and drawn on this machine's adapter — lavapipe in CI — through the
//! frame every window and capture draws (`prepare_frame` /
//! `encode_frame`, reached here through `capture_frame`, as
//! `capture.screenshot` and `--headless --screenshot` reach it).
//!
//! Every assertion is structural, never a pixel (§T10): how many runs the
//! frame was handed and drew, how many were left out by their zoom
//! window, how many glyphs the atlas rasterized and found, how many
//! times the render state was applied and whether a run's state was
//! written. The frame is drawn and read back all the same, so a pipeline
//! that fails to build, bind or draw fails here with wgpu's validation
//! error rather than passing on counts alone.
//!
//! The harness app holds its own map, drawn by the old canvas path near
//! the canvas origin; every document here is moved [`FAR`] from it, so a
//! frame shows the layers and nothing else. With `MANDALA_LAYERS_OUT` set
//! to a directory, each map's frame is also written there as
//! `<name>.png`, and drawn again at twice the pixels as `<name>@2x.png`:
//!
//! ```sh
//! MANDALA_LAYERS_OUT=target/layers cargo test -p mandala tests_gpu_layers
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format::load;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::resolve::resolve_background;
use baumhard::document::writer::{ChangeSource, EditRequest};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutator::Mutator;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::projection::tests::support::Projected;
use baumhard::projection::{GlyphLayers, Layer};
use glam::Vec2;
use serde_json::{json, Value};

use super::gpu_harness::{headless_app, HARNESS_SIZE};
use super::run_native_init::InitState;
use crate::application::renderer::LayerRenderer;
use crate::application::renderer::{CanvasSource, CaptureFailure, CapturedPixels, FrameSkip, ModalLayer};

/// Where the documents are drawn: far from every node of the harness's
/// own map.
const FAR: Vec2 = Vec2::new(100_000.0, 100_000.0);

/// The directory to write each map's frame to, when set.
const OUT_ENV: &str = "MANDALA_LAYERS_OUT";

/// How much of the frame a map may take, each way.
const FILL: f32 = 0.94;

fn harness_map() -> String {
    format!("{}/maps/testament.mindmap.json", env!("CARGO_MANIFEST_DIR"))
}

/// The map `text`, every node moved by [`FAR`], loaded. A node's
/// `position` is its canvas position, so moving every node moves the
/// whole drawing and changes nothing else.
fn far(text: &str) -> Document {
    let mut value: Value = serde_json::from_str(text).expect("a map is JSON");
    if let Some(nodes) = value["nodes"].as_array_mut() {
        for node in nodes {
            let at = node["position"].as_array().map_or(Vec2::ZERO, |p| {
                Vec2::new(
                    p[0].as_f64().unwrap_or(0.0) as f32,
                    p[1].as_f64().unwrap_or(0.0) as f32,
                )
            });
            let moved = at + FAR;
            node["position"] = json!([moved.x, moved.y]);
        }
    }
    load(&value.to_string()).unwrap_or_else(|e| panic!("the map loads: {e}"))
}

/// A map with something in every layer: three framed nodes — two rings
/// of two lines each, laid on the level parts of their outlines, and a
/// repeated ornament all round — the child's connector a line of verse
/// with a label and an end cap, and a portal from the root to the third
/// node, each end a marker and a label. The screenshot the issue asks
/// for, composed so that nothing in it overlaps: the connector and the
/// portal ends attach where the rings leave the outline bare (the sides
/// of a ring laid at its natural span), and each label stands clear of
/// what it labels.
fn every_layer() -> Value {
    json!({
        "format": 1,
        "name": "every layer",
        "nodes": [
            {
                "id": 1,
                "position": [-330.0, -110.0],
                "min_size": [250.0, 150.0],
                "shape": {"rectangle": {"radius": 25}},
                "sections": [{"offset": [0.0, 55.0], "min_size": [250.0, 0.0], "align": "center",
                              "font": "Galatia SIL", "size": 22.0, "color": "#e8e0c8",
                              "spans": [{"text": "glyph layers"}]}],
                "frame": {"fit": {"fit": {"min": 0.5, "max": 1.4}}, "span": "natural", "gap": 4.0,
                          "rings": [{"font": "Galatia SIL", "size": 0.12, "color": "#f0c878",
                                     "spans": [{"text": "every glyph a run's,\ndrawn as journaled"}]}]},
                "links": [{"to": 3, "kind": "portal",
                           "ends": [{"content": "◈", "color": "#cc88ff", "size": 26.0, "border_t": 0.9, "offset": 10.0,
                                     "label": {"offset": 6.0, "size": 14.0, "color": "#cc88ff",
                                               "spans": [{"text": "to the far one"}]}},
                                    {"content": "◈", "color": "#cc88ff", "size": 26.0, "offset": 14.0,
                                     "label": {"offset": 6.0, "size": 14.0, "color": "#cc88ff",
                                               "spans": [{"text": "back"}]}}]}]
            },
            {
                "id": 2,
                "parent": 1,
                "position": [260.0, 200.0],
                "min_size": [230.0, 130.0],
                "shape": "ellipse",
                "sections": [{"offset": [0.0, 50.0], "min_size": [230.0, 0.0], "align": "center",
                              "font": "Galatia SIL", "size": 20.0, "color": "#c8c8c8",
                              "spans": [{"text": "a child"}]}],
                "frame": {"fit": "fit", "span": "natural",
                          "rings": [{"font": "Galatia SIL", "size": 0.11, "color": "#eb8caa",
                                     "spans": [{"text": "and an ellipse,\nnever rotated"}]}]},
                "connector": {"fit": "once", "size": 16.0, "color": "#a0d090",
                              "spans": [{"text": "the line to its parent"}],
                              "cap_end": {"color": "#a0d090", "spans": [{"text": "›"}]},
                              "label": {"t": 0.5, "offset": 40.0, "size": 14.0, "color": "#d0e8c0",
                                        "spans": [{"text": "a label"}]}}
            },
            {
                "id": 3,
                "position": [-300.0, 260.0],
                "min_size": [160.0, 70.0],
                "shape": {"rectangle": {"radius": 30}},
                "sections": [{"offset": [0.0, 22.0], "min_size": [160.0, 0.0], "align": "center",
                              "font": "Galatia SIL", "size": 20.0, "color": "#c8c8c8",
                              "spans": [{"text": "the far one"}]}],
                "frame": {"fit": "repeat", "gap": 4.0,
                          "rings": [{"font": "Galatia SIL", "size": 0.2, "color": "#78a0dc",
                                     "spans": [{"text": "·:·"}]}]}
            }
        ]
    })
}

/// Two lone nodes: `near`, drawn only at a zoom of 2 and up, and
/// `always`, drawn at every zoom.
fn zoom_windows() -> Value {
    json!({
        "format": 1,
        "name": "zoom windows",
        "nodes": [
            {"id": 1, "position": [0.0, 0.0], "zoom": {"min": 2.0},
             "sections": [{"font": "Galatia SIL", "size": 18.0, "spans": [{"text": "near"}]}]},
            {"id": 2, "position": [120.0, 0.0],
             "sections": [{"font": "Galatia SIL", "size": 18.0, "spans": [{"text": "always"}]}]}
        ]
    })
}

/// The demo maps and the format's portal fixture, by name.
fn maps() -> Vec<(String, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(root.join("maps/demos"))
        .expect("the demo maps are readable")
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
    out.sort();
    out.push((
        "portals".into(),
        root.join("lib/baumhard/src/document/format/fixtures/portals.mandala.json"),
    ));
    assert!(out.len() >= 5, "the demo maps and the portal fixture: {out:?}");
    out
}

/// Every run of `layers`, by layer and glyph id, read from the trees.
fn runs_of(layers: &GlyphLayers) -> Vec<((Layer, NodeId), GlyphRunData)> {
    let mut out = Vec::new();
    for layer in Layer::ALL {
        let tree = layers.tree(layer);
        for id in tree.root().descendants(tree.arena()) {
            if let Some(run) = tree.get(id).and_then(|node| node.get().glyph_run()) {
                out.push(((layer, id), run.clone()));
            }
        }
    }
    out
}

/// A camera holding every run of `layers` in [`FILL`] of the frame.
fn framing(layers: &GlyphLayers) -> Camera2D {
    let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for (_, run) in runs_of(layers) {
        let (top_left, size) = run.bounds();
        min = min.min(top_left);
        max = max.max(top_left + size);
    }
    assert!(
        min.is_finite() && max.is_finite(),
        "precondition: the layers draw runs"
    );
    let size = (max - min).max(Vec2::ONE);
    let mut camera = Camera2D::new(HARNESS_SIZE.0, HARNESS_SIZE.1);
    camera.position = 0.5 * (min + max);
    camera.zoom = FILL * (HARNESS_SIZE.0 as f32 / size.x).min(HARNESS_SIZE.1 as f32 / size.y);
    camera
}

/// A camera on `center` at `zoom`.
fn camera_at(center: Vec2, zoom: f32) -> Camera2D {
    let mut camera = Camera2D::new(HARNESS_SIZE.0, HARNESS_SIZE.1);
    camera.position = center;
    camera.zoom = zoom;
    camera
}

/// Draw a frame through `camera` and read it back, retrying while
/// another test's shaping holds the font lock.
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

/// Write `pixels` to `<MANDALA_LAYERS_OUT>/<name>.png`, when it is set.
fn write_png(name: &str, pixels: &CapturedPixels) {
    let Some(dir) = std::env::var_os(OUT_ENV).map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let file = dir.join(format!("{name}.png"));
    let mut handle = std::fs::File::create(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    crate::application::capture::png::encode(pixels, &mut handle, &mut || {})
        .unwrap_or_else(|e| panic!("{}: {e:?}", file.display()));
    eprintln!("{name}: wrote {}", file.display());
}

/// Load `p`'s layers into the renderer, cleared to its map's background.
fn show(init: &mut InitState, p: &Projected) {
    init.renderer
        .set_background(resolve_background(p.doc.tree(), &mut ()).0);
    init.renderer.load_layers(&p.layers);
}

/// Bring the renderer up to date with every change `p`'s document
/// recorded since the last call, as the app's pipeline will (V1-C3):
/// the projector's update, applied, then its journals discarded.
fn sync(init: &mut InitState, p: &mut Projected) {
    let changes = p.sync();
    init.renderer
        .apply_layer_journals(&p.layers, LayerRenderer::journals_of(&changes));
    changes.discard(&mut p.layers);
}

/// A request of one `op` of the node `ident` of `doc`.
fn node_edit(doc: &Document, ident: u32, op: NodeOp) -> EditRequest {
    let node = doc.node_by_ident(NodeIdent(ident)).expect("the map has the node");
    let mutator = DocMutatorTree::new_with(Mutator::new(
        DocMutation::Node(op),
        BranchKey::new(DocKind::Node, 0),
    ));
    EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(vec![node])))],
        "app::tests_gpu_layers",
    )
}

/// Every run's state stamp, in draw order: equal across two reads
/// exactly when no state was written in between.
fn stamps(init: &InitState, layers: &GlyphLayers) -> Vec<u64> {
    runs_of(layers)
        .iter()
        .map(|((layer, id), _)| {
            init.renderer
                .layer_renderer()
                .state(*layer, *id)
                .expect("every run is kept")
                .stamp()
        })
        .collect()
}

/// **Every run of the layers is drawn, through the one draw of the
/// positioned-glyph pass**, for each demo map, the portal fixture and a
/// map with something in every layer: the frame is handed exactly the
/// layers' runs (the harness's own map draws none), draws them as one
/// instanced draw call, and adds no glyphon text buffer — the node text,
/// frames and connections that used to be one buffer per element or per
/// glyph are runs.
#[test]
fn test_gpu_layers_draw_every_run_of_the_layers_in_one_draw() {
    let Some(mut app) = headless_app(&harness_map()) else {
        return;
    };
    let init = &mut app.init;
    let buffers_before = init.renderer.canvas_text_buffer_count();
    let mut cases: Vec<(String, Document)> = maps()
        .into_iter()
        .map(|(name, path)| {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, far(&text))
        })
        .collect();
    cases.push(("every_layer".into(), far(&every_layer().to_string())));
    let mut per_layer = [0usize; 5];
    for (name, doc) in cases {
        let p = Projected::new(doc);
        let runs = runs_of(&p.layers);
        for ((layer, _), _) in &runs {
            per_layer[layer.index()] += 1;
        }
        show(init, &p);
        let camera = framing(&p.layers);
        let pixels = draw(init, &camera);
        let counts = init.renderer.glyph_run_counts();
        assert_eq!(
            counts.runs,
            runs.len(),
            "{name}: the frame is handed every run: {counts:?}"
        );
        assert_eq!(
            counts.runs_drawn + counts.runs_outside_zoom,
            runs.len(),
            "{name}: every run on screen is drawn: {counts:?}"
        );
        assert_eq!(counts.draws, 1, "{name}: one draw call: {counts:?}");
        assert!(counts.instances > 0, "{name}: glyphs are drawn: {counts:?}");
        assert_eq!(
            init.renderer.canvas_text_buffer_count(),
            buffers_before,
            "{name}: the layers add no glyphon buffer"
        );
        eprintln!("{name}: {} runs, {counts:?}", runs.len());
        write_png(&name, &pixels);
        if std::env::var_os(OUT_ENV).is_some() {
            // The same frame at twice the pixels, as a display with a
            // device pixel ratio of 2 draws it: `<name>@2x.png`.
            let mut double = Camera2D::new(2 * HARNESS_SIZE.0, 2 * HARNESS_SIZE.1);
            double.position = camera.position;
            double.zoom = 2.0 * camera.zoom;
            write_png(&format!("{name}@2x"), &draw(init, &double));
        }
    }
    assert!(
        per_layer.iter().all(|&n| n > 0),
        "precondition: the maps draw runs in every layer: {per_layer:?}"
    );
}

/// **A run outside its zoom window issues no draw**, evaluated at draw
/// time: at zoom 1 the node whose window opens at 2 hands the frame its
/// run and draws none of it; at zoom 2.5 — nothing applied in between,
/// only the camera changed — it is drawn. The control is the second
/// frame: the same state draws it once the camera is inside its window.
#[test]
fn test_gpu_layers_a_run_outside_its_zoom_window_issues_no_draw() {
    let Some(mut app) = headless_app(&harness_map()) else {
        return;
    };
    let init = &mut app.init;
    let p = Projected::new(far(&zoom_windows().to_string()));
    assert_eq!(
        runs_of(&p.layers).len(),
        2,
        "precondition: one section run per node"
    );
    show(init, &p);
    let center = FAR + Vec2::new(90.0, 10.0);
    let applies = init.renderer.layer_renderer().log().applies;

    draw(init, &camera_at(center, 1.0));
    let outside = init.renderer.glyph_run_counts();
    assert_eq!(
        (outside.runs, outside.runs_drawn, outside.runs_outside_zoom),
        (2, 1, 1),
        "at zoom 1 the near node's run is left out by its window: {outside:?}"
    );

    draw(init, &camera_at(center, 2.5));
    let inside = init.renderer.glyph_run_counts();
    assert_eq!(
        (inside.runs, inside.runs_drawn, inside.runs_outside_zoom),
        (2, 2, 0),
        "at zoom 2.5 both are drawn: {inside:?}"
    );
    assert!(
        inside.instances > outside.instances,
        "the near node's glyphs are drawn only inside its window: {outside:?} then {inside:?}"
    );
    assert_eq!(
        init.renderer.layer_renderer().log().applies,
        applies,
        "a zoom applies nothing: the window is evaluated at draw time"
    );
}

/// **A pan applies nothing and writes no render state**: a pan by whole
/// pixels draws the same runs, applies nothing, writes no run's state
/// (every stamp is the one it had) and rasterizes no glyph — every glyph
/// is an atlas hit. The control is a zoom, which rasterizes anew, so the
/// pan's zero is a zero the counter chose.
#[test]
fn test_gpu_layers_a_pan_applies_nothing_and_writes_no_render_state() {
    let Some(mut app) = headless_app(&harness_map()) else {
        return;
    };
    let init = &mut app.init;
    let p = Projected::new(far(&every_layer().to_string()));
    show(init, &p);
    let mut camera = framing(&p.layers);
    camera.zoom = 1.0;
    draw(init, &camera);
    let before = init.renderer.glyph_run_counts();
    let applies = init.renderer.layer_renderer().log().applies;
    let stamps_before = stamps(init, &p.layers);
    assert!(
        before.instances > 0,
        "precondition: the frame draws glyphs: {before:?}"
    );

    // At zoom 1 and a pixel ratio of 1, a canvas unit is a pixel.
    camera.position += Vec2::new(37.0, -12.0);
    draw(init, &camera);
    let panned = init.renderer.glyph_run_counts();
    assert_eq!(
        init.renderer.layer_renderer().log().applies,
        applies,
        "a pan applied render state"
    );
    assert_eq!(
        stamps(init, &p.layers),
        stamps_before,
        "a pan wrote a run's state"
    );
    assert_eq!(
        panned.runs, before.runs,
        "the same runs are handed in: {panned:?}"
    );
    assert_eq!(
        panned.atlas.rasterized, before.atlas.rasterized,
        "a whole-pixel pan rasterized glyphs: {before:?} then {panned:?}"
    );
    assert!(
        panned.atlas.hits > before.atlas.hits,
        "the pan drew from the atlas: {panned:?}"
    );

    camera.zoom = 1.5;
    draw(init, &camera);
    let zoomed = init.renderer.glyph_run_counts();
    assert!(
        zoomed.atlas.rasterized > panned.atlas.rasterized,
        "the control: a zoom rasterizes anew: {panned:?} then {zoomed:?}"
    );
    assert_eq!(
        stamps(init, &p.layers),
        stamps_before,
        "a zoom wrote a run's state"
    );
}

/// **A re-placement reuses the shaped run and its atlas entries**: a node
/// dragged by whole canvas units — through the document, the projector
/// and the renderer's apply, as the app will — re-places its section and
/// its frame's rings, reshapes nothing, and its next frame rasterizes no
/// glyph. The control is a drag by a fraction of a pixel, which lands
/// the glyphs in other subpixel bins and so rasterizes them.
#[test]
fn test_gpu_layers_a_re_placement_reuses_the_shaped_run_and_its_atlas_entries() {
    let Some(mut app) = headless_app(&harness_map()) else {
        return;
    };
    let init = &mut app.init;
    let mut map = every_layer();
    // The framed root alone, with no path to it: each of its runs moves
    // with it by exactly the drag.
    map["nodes"] = json!([map["nodes"][0].clone()]);
    map["nodes"][0]["links"] = json!([]);
    let mut p = Projected::new(far(&map.to_string()));
    show(init, &p);
    let mut camera = framing(&p.layers);
    camera.zoom = 1.0;
    camera.position = camera.position.round();
    draw(init, &camera);
    let before = init.renderer.glyph_run_counts();

    p.doc
        .apply(
            node_edit(&p.doc, 1, NodeOp::Nudge(Vec2::new(40.0, -25.0))),
            ChangeSource::Action,
        )
        .expect("the drag applies");
    sync(init, &mut p);
    let log = init.renderer.layer_renderer().log().clone();
    assert!(
        log.moved + log.replaced > 0,
        "precondition: the drag re-placed runs: {log:?}"
    );
    assert_eq!(log.reshaped, 0, "a drag adopted a new shaping: {log:?}");
    draw(init, &camera);
    let dragged = init.renderer.glyph_run_counts();
    assert_eq!(dragged.runs, before.runs, "the same runs: {dragged:?}");
    assert_eq!(
        dragged.atlas.rasterized, before.atlas.rasterized,
        "a drag by whole units rasterized glyphs: {before:?} then {dragged:?}"
    );

    p.doc
        .apply(
            node_edit(&p.doc, 1, NodeOp::Nudge(Vec2::new(0.3, 0.0))),
            ChangeSource::Action,
        )
        .expect("the nudge applies");
    sync(init, &mut p);
    draw(init, &camera);
    let nudged = init.renderer.glyph_run_counts();
    assert!(
        nudged.atlas.rasterized > dragged.atlas.rasterized,
        "the control: a sub-pixel drag rasterizes: {dragged:?} then {nudged:?}"
    );
}
