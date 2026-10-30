// SPDX-License-Identifier: MPL-2.0

//! GPU-backed tests (TEST_CONVENTIONS §T8) of the positioned-glyph
//! pass (`renderer::glyph_runs`, design §13.2): a `GlyphRun` in a canvas
//! tree, drawn by the real renderer on this machine's adapter —
//! lavapipe in CI — through the frame every window and capture draws
//! (`prepare_frame` / `encode_frame`, here reached through
//! `capture_frame`).
//!
//! Every assertion is structural, never a pixel (§T10): how many runs
//! were handed in and drawn, how many glyph instances and draw calls
//! the frame holds, how many glyphs the atlas rasterized and how many
//! it found, how many glyphon buffers the canvas pass holds. The frame
//! is drawn and read back all the same, so a pipeline that fails to
//! build, bind or draw fails the test with wgpu's validation error
//! rather than passing on counts alone.
//!
//! The runs are built by hand here — shaped by `baumhard::flow`, placed
//! by it or by the test, wrapped in a `GlyphRun` element and registered
//! as a canvas tree — because nothing in the app emits them yet: the
//! projection is V1-B7, its wiring V1-C3.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::{Duration, Instant};

use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{
    Direction, Edge, FitMode, FontSize, Percent, Rgba, Shape, StartPoint, Weight,
};
use baumhard::engine::tree::Placement;
use baumhard::flow::{
    outline, place_at_point, place_on_closed, place_on_open, shape_run, Curve, FlowSpan, PlacedGlyph,
    PlacedRun, Rect, ShapedRun,
};
use baumhard::font::fonts::{self, acquire_font_system_write, AppFont, FontSystemWriteGuard};
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutation;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp, RunShape};
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::mindmap::custom_mutation::apply_mutations_to_element;
use glam::Vec2;

use super::gpu_harness::{headless_app, HARNESS_SIZE};
use super::run_native_init::InitState;
use crate::application::renderer::{CanvasSource, CaptureFailure, CapturedPixels, FrameSkip, ModalLayer};
use crate::application::scene_host::CanvasRole;

fn map_path(name: &str) -> String {
    format!("{}/maps/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Where the runs are drawn: a stretch of canvas far from every node of
/// the fixture map, so the frame shows the runs and nothing else.
const FAR: Vec2 = Vec2::new(100_000.0, 100_000.0);

fn style(font: AppFont, points: f32, color: Rgba) -> ResolvedText {
    ResolvedText {
        font,
        size: FontSize::Pt(points),
        color,
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// Run `f` under the font guard, released before the result returns, so
/// a failing assertion never holds the lock.
fn with_fonts<R>(f: impl FnOnce(&mut FontSystemWriteGuard) -> R) -> R {
    fonts::init();
    let mut fs = acquire_font_system_write("app::tests_gpu_glyph_runs");
    f(&mut fs)
}

fn shape(text: &str, style: &ResolvedText, fs: &mut FontSystemWriteGuard) -> ShapedRun {
    shape_run(
        &[FlowSpan {
            text,
            style,
            link: None,
        }],
        1.0,
        fs,
    )
}

fn path_run(run: Arc<ShapedRun>, placed: PlacedRun, opacity: f32) -> GlyphRunData {
    GlyphRunData::new(
        RunShape::Path {
            run,
            placed: Arc::new(placed),
        },
        Vec2::ZERO,
        GlyphOverrides::none(),
        opacity,
        None,
    )
    .expect("no overrides, so no length to get wrong")
}

/// Register `runs` as one canvas tree, one element each, and bring the
/// renderer's canvas pass in step with it. The role is one the fixture
/// leaves empty in its default mode, so the tree replaces nothing drawn.
fn show(init: &mut InitState, runs: Vec<GlyphRunData>) {
    let mut tree: GfxTree = GfxTree::new_non_indexed();
    for (i, run) in runs.into_iter().enumerate() {
        tree.insert_child(tree.root, GfxElement::new_run(run, 0, 9_000 + i), Placement::Last)
            .expect("a child of the root");
    }
    init.app_scene
        .register_canvas(CanvasRole::SectionFrames, tree, Vec2::ZERO);
    init.renderer.rebuild_canvas_scene_buffers(&mut init.app_scene);
}

/// A camera on `center` at `zoom`, framing the harness surface.
fn camera_on(center: Vec2, zoom: f32) -> Camera2D {
    let mut camera = Camera2D::new(HARNESS_SIZE.0, HARNESS_SIZE.1);
    camera.position = center;
    camera.zoom = zoom;
    camera
}

/// Draw a frame through `camera` and read it back, retrying while
/// another test's shaping holds the font lock — the one transient skip —
/// for as long as the lock's own budget treats a hold as legitimate.
fn draw(init: &mut InitState, camera: &Camera2D) -> CapturedPixels {
    const PATIENCE: Duration = baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    let deadline = Instant::now() + PATIENCE;
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

/// `count` glyph boxes centered on a circle of `radius` around
/// `center`, cycling through the run's `glyphs` glyphs: a placement as
/// `flow` would hand one over, at positions the test chose.
fn circle(count: usize, glyphs: usize, center: Vec2, radius: f32, box_size: Vec2) -> PlacedRun {
    PlacedRun {
        glyphs: (0..count)
            .map(|i| {
                let angle = TAU * i as f32 / count as f32;
                PlacedGlyph {
                    glyph: (i % glyphs) as u32,
                    pos: center + Vec2::new(angle.cos(), angle.sin()) * radius - box_size * 0.5,
                }
            })
            .collect(),
        truncated: false,
    }
}

/// **The issue's evidence: 500 glyphs placed on a circle draw from one
/// shaped run, through one draw path, as 500 instances in one draw
/// call — and a re-placement reuses every atlas entry.**
///
/// One `shape_run` makes the run; a placement of 500 glyphs on a circle
/// refers to it; the element is the only thing in its tree. The frame:
///
/// - hands the pass one run and draws it, as 500 glyph instances in one
///   instanced draw call, skipping none;
/// - adds no glyphon buffer — the canvas pass holds as many as before
///   the run was registered: no per-glyph `Buffer`, no `TextArea`;
/// - rasterizes each distinct glyph once: the atlas holds between the
///   run's 8 glyph ids and 8 × 4 subpixel bins of them, one
///   rasterization per entry — not one per placed glyph.
///
/// Re-placing the run — a new placement, the whole circle moved by whole
/// canvas units, the same `Arc` of shaping — and drawing again
/// rasterizes nothing: all 500 glyphs are atlas hits. A pan rasterizes
/// nothing either, and touches no run (the canvas pass is not even
/// asked). The control is a zoom: the same run at zoom 2 is rasterized
/// at twice the size, so the atlas grows — the counter can move, and
/// the re-placement's zero is a zero it chose.
#[test]
fn test_a_glyph_run_of_500_glyphs_on_a_circle_draws_from_one_shaped_run_in_one_draw_call() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    let buffers_before = init.renderer.canvas_text_buffer_count();

    let style = style(AppFont::GalatiaSIL, 24.0, Rgba::opaque(240, 220, 160));
    let shaped = Arc::new(with_fonts(|fs| shape("GLYPHRUN", &style, fs)));
    let glyph_ids: std::collections::BTreeSet<u16> = shaped.layout_glyphs().map(|g| g.glyph_id).collect();
    assert_eq!(
        (shaped.glyphs.len(), glyph_ids.len()),
        (8, 8),
        "precondition: eight glyphs, all distinct"
    );
    let box_size = Vec2::new(16.0, shaped.line_height);
    let placed = circle(500, 8, FAR, 250.0, box_size);
    show(init, vec![path_run(shaped.clone(), placed.clone(), 1.0)]);

    let camera = camera_on(FAR, 1.0);
    draw(init, &camera);
    let first = init.renderer.glyph_run_counts();
    assert_eq!(
        (first.runs, first.runs_drawn),
        (1, 1),
        "one run handed in and drawn: {first:?}"
    );
    assert_eq!(first.instances, 500, "one instance per placed glyph: {first:?}");
    assert_eq!(first.skipped, 0, "no glyph skipped: {first:?}");
    assert_eq!(first.draws, 1, "one draw call for the whole run: {first:?}");
    assert_eq!(
        init.renderer.canvas_text_buffer_count(),
        buffers_before,
        "the run added no glyphon buffer: no per-glyph Buffer or TextArea"
    );
    assert!(
        (8..=8 * 4).contains(&first.atlas_entries),
        "each distinct glyph rasterized per subpixel bin it lands in, not per placement: {first:?}"
    );
    assert_eq!(
        first.atlas.rasterized as usize, first.atlas_entries,
        "one rasterization per entry: {first:?}"
    );
    assert_eq!(
        first.atlas.hits + first.atlas.rasterized,
        500,
        "every placed glyph asked the atlas exactly once: {first:?}"
    );

    // Re-place: a new placement of the same shaping, moved by whole
    // canvas units, so every glyph lands on the same subpixel bin.
    let moved = PlacedRun {
        glyphs: placed
            .glyphs
            .iter()
            .map(|g| PlacedGlyph {
                glyph: g.glyph,
                pos: g.pos + Vec2::new(37.0, -12.0),
            })
            .collect(),
        truncated: false,
    };
    let replaced = path_run(shaped.clone(), moved, 1.0);
    let RunShape::Path { run: held, .. } = replaced.shaped() else { unreachable!("built as a path run") };
    assert!(
        Arc::ptr_eq(held, &shaped),
        "the re-placement refers to the one shaped run"
    );
    show(init, vec![replaced]);
    draw(init, &camera);
    let second = init.renderer.glyph_run_counts();
    assert_eq!((second.instances, second.draws), (500, 1), "{second:?}");
    assert_eq!(
        second.atlas.rasterized, first.atlas.rasterized,
        "a re-placement rasterizes nothing: {second:?}"
    );
    assert_eq!(
        second.atlas.hits - first.atlas.hits,
        500,
        "every glyph of the re-placed run was an atlas hit: {second:?}"
    );
    assert_eq!(second.prepares, first.prepares + 1, "one prepare per frame");

    // A pan by whole pixels: no run is touched and nothing rasterized.
    draw(init, &camera_on(FAR + Vec2::new(25.0, 0.0), 1.0));
    let panned = init.renderer.glyph_run_counts();
    assert_eq!(
        panned.atlas.rasterized, first.atlas.rasterized,
        "a pan rasterizes nothing"
    );

    // Control: a zoom rasterizes at a new size.
    draw(init, &camera_on(FAR, 2.0));
    let zoomed = init.renderer.glyph_run_counts();
    assert!(
        zoomed.atlas.rasterized > panned.atlas.rasterized,
        "control: at zoom 2 the glyphs are rasterized anew: {zoomed:?}"
    );
}

/// **The issue's picture: text around a rounded rectangle and an
/// ellipse**, each frame one run shaped once and placed by
/// `baumhard::flow` — `Fit` around the rounded rectangle, `Repeat` on a
/// second ring outside it, `Fit` around the ellipse with a few glyphs
/// recolored and scaled by overrides — plus a connector between them at
/// half opacity, `Repeat` along a cubic, and two point markers inside
/// the ellipse, one at half opacity. Seven runs, every glyph of them in
/// one draw call.
///
/// Asserted: every run is drawn; every placed glyph is either an
/// instance or a glyph with nothing to draw (a space); one draw call;
/// and the next frame rasterizes nothing, the spaces included.
/// With `MANDALA_GLYPH_RUN_PNG` set to a path, the frame is also
/// written there as a PNG — the picture the issue asks for, made by this
/// test and never committed.
#[test]
fn test_frames_of_text_around_a_rounded_rectangle_and_an_ellipse_draw_in_one_call() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;

    let gold = style(AppFont::GalatiaSIL, 22.0, Rgba::opaque(240, 200, 120));
    let rule = style(AppFont::GalatiaSIL, 16.0, Rgba::opaque(120, 160, 220));
    let rose = style(AppFont::GalatiaSIL, 20.0, Rgba::opaque(235, 140, 170));
    let link = style(AppFont::GalatiaSIL, 18.0, Rgba::opaque(200, 200, 200));
    let rounded = Rect::new(FAR + Vec2::new(-345.0, -100.0), Vec2::new(260.0, 170.0));
    let ellipse = Rect::new(FAR + Vec2::new(115.0, -110.0), Vec2::new(230.0, 190.0));
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let top = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };

    let runs: Vec<GlyphRunData> = with_fonts(|fs| {
        let mut runs = Vec::new();
        let frame = |text: &str, style: &ResolvedText, path, fit: &FitMode, fs: &mut FontSystemWriteGuard| {
            let shaped = shape(text, style, fs);
            let (rescaled, placed) =
                place_on_closed(&shaped, &path, top, Direction::Clockwise, fit, true, style, fs);
            let run = Arc::new(rescaled.unwrap_or(shaped));
            path_run(run, placed, 1.0)
        };
        let rounded_outline = outline(
            rounded,
            Shape::Rectangle {
                radius: Percent::new(60),
            },
        );
        runs.push(frame(
            "A frame of text around a rounded rectangle, shaped once and placed glyph by glyph. ",
            &gold,
            rounded_outline,
            &fit,
            fs,
        ));
        let ring = baumhard::flow::offset(&rounded_outline, 30.0);
        runs.push(frame("·:·", &rule, ring, &FitMode::Repeat, fs));
        runs.push(frame(
            "and an ellipse, every glyph upright, never rotated · ",
            &rose,
            outline(ellipse, Shape::Ellipse),
            &fit,
            fs,
        ));
        let from = Vec2::new(rounded.max().x + 48.0, rounded.center().y);
        let to = Vec2::new(ellipse.origin.x - 34.0, ellipse.center().y);
        let curve = Curve::Cubic(
            from,
            from + Vec2::new(70.0, -90.0),
            to + Vec2::new(-70.0, 90.0),
            to,
        );
        let shaped = shape("~", &link, fs);
        let (rescaled, placed) = place_on_open(&shaped, &curve, &FitMode::Repeat, 2.0, &link, fs);
        runs.push(path_run(Arc::new(rescaled.unwrap_or(shaped)), placed, 0.5));
        // Two markers at points inside the ellipse, the second at half
        // opacity: an emoji, where the machine has a color face for one,
        // is drawn from the color page and takes the opacity too.
        for (text, dy, opacity) in [
            ("\u{1F338} opaque \u{1F338}", -22.0, 1.0),
            ("\u{1F338} half \u{1F338}", 22.0, 0.5),
        ] {
            let shaped = shape(text, &link, fs);
            let placed = place_at_point(&shaped, ellipse.center() + Vec2::new(0.0, dy));
            runs.push(path_run(Arc::new(shaped), placed, opacity));
        }
        runs
    });

    // Presentation effects on the ellipse's run, through the glyph
    // mutations a presentation writes them with.
    let mut runs = runs;
    let mut element = GfxElement::new_run(runs.remove(2), 0, 1);
    apply_mutations_to_element(
        &[
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 0..14,
                value: Some(Rgba::opaque(255, 255, 255)),
            }),
            GfxMutation::RunOverrides(RunOverrideOp::Scale {
                range: 4..11,
                value: 1.4,
            }),
        ],
        &mut element,
    );
    runs.insert(2, element.glyph_run().cloned().expect("a run"));
    let placed_total: usize = runs.iter().map(|run| run.shaped().glyph_count()).sum();
    let run_count = runs.len();
    show(init, runs);

    let pixels = draw(init, &camera_on(FAR, 1.0));
    let counts = init.renderer.glyph_run_counts();
    assert_eq!(
        (counts.runs, counts.runs_drawn),
        (run_count, run_count),
        "{counts:?}"
    );
    assert_eq!(
        counts.instances + counts.skipped,
        placed_total,
        "every placed glyph is drawn or has nothing to draw: {counts:?}"
    );
    assert!(
        counts.instances > placed_total / 2,
        "most glyphs have ink: {counts:?}"
    );
    assert!(
        counts.skipped > 0,
        "precondition: the spaces have nothing to draw: {counts:?}"
    );
    draw(init, &camera_on(FAR, 1.0));
    let again = init.renderer.glyph_run_counts();
    assert_eq!(
        again.atlas.rasterized, counts.atlas.rasterized,
        "the next frame rasterizes nothing, spaces included — a glyph with no bitmap is remembered: {again:?}"
    );
    assert_eq!(counts.draws, 1, "every run in one draw call: {counts:?}");

    if let Some(path) = std::env::var_os("MANDALA_GLYPH_RUN_PNG") {
        let mut file = std::fs::File::create(&path).expect("the PNG path can be written");
        crate::application::capture::png::encode(&pixels, &mut file, &mut || {}).expect("the PNG encodes");
    }
}

/// `text`'s glyphs on a grid of `columns`, `step` canvas units apart,
/// from `corner`: whole units, so at a whole zoom every glyph lands on
/// subpixel bin zero and each glyph id is one atlas entry.
fn grid(text: &str, style: &ResolvedText, corner: Vec2, columns: usize, step: f32) -> GlyphRunData {
    let shaped = with_fonts(|fs| shape(text, style, fs));
    let placed = PlacedRun {
        glyphs: (0..shaped.glyphs.len())
            .map(|i| PlacedGlyph {
                glyph: i as u32,
                pos: corner + Vec2::new((i % columns) as f32, (i / columns) as f32) * step,
            })
            .collect(),
        truncated: false,
    };
    path_run(Arc::new(shaped), placed, 1.0)
}

/// **The atlas grows, evicts and refuses without failing a frame.**
///
/// Twenty-six capitals at the camera's largest zoom do not fit the
/// atlas's first page: drawing them grows it, growing writes the
/// capitals the page already held into the larger texture, and the next
/// frame finds every one of them again. With growth then frozen at that size, twenty-six
/// lowercase letters in their place evict the capitals, which the frame
/// no longer draws, and fit. Both alphabets at once do not fit at all:
/// the frame is still drawn, the glyphs that found room are drawn, and
/// the rest are counted as refused — never a panic, never a skipped
/// frame (CODE_CONVENTIONS §9).
#[test]
fn test_the_glyph_atlas_grows_evicts_and_refuses_without_failing_a_frame() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    let big = style(AppFont::GalatiaSIL, 24.0, Rgba::opaque(255, 255, 255));
    let corner = FAR - Vec2::new(75.0, 55.0);
    let upper = grid("ABCDEFGHIJKLMNOPQRSTUVWXYZ", &big, corner, 7, 22.0);
    let lower = grid("abcdefghijklmnopqrstuvwxyz", &big, corner, 7, 22.0);
    let camera = camera_on(FAR, Camera2D::MAX_ZOOM);

    show(init, vec![upper.clone()]);
    draw(init, &camera);
    let grown = init.renderer.glyph_run_counts();
    assert_eq!(
        (grown.instances, grown.skipped),
        (26, 0),
        "precondition: all 26 capitals on screen: {grown:?}"
    );
    assert!(
        grown.atlas.growths >= 1,
        "26 capitals at 120 px outgrow a 256² page: {grown:?}"
    );
    assert!(
        (1..26).contains(&grown.atlas.reuploaded),
        "growing wrote the capitals it already held into the larger page, and only those: {grown:?}"
    );
    assert_eq!(
        grown.atlas.rasterized, 26,
        "one rasterization per capital: {grown:?}"
    );

    draw(init, &camera);
    let again = init.renderer.glyph_run_counts();
    assert_eq!(
        (again.atlas.rasterized, again.atlas.hits - grown.atlas.hits),
        (26, 26),
        "after growing, every capital is still found: {again:?}"
    );

    init.renderer.freeze_glyph_atlas_growth();
    show(init, vec![lower.clone()]);
    draw(init, &camera);
    let swapped = init.renderer.glyph_run_counts();
    assert!(
        swapped.atlas.evicted > 0,
        "the full page made room by evicting capitals this frame does not draw: {swapped:?}"
    );
    assert_eq!(
        (swapped.instances, swapped.atlas.refused),
        (26, 0),
        "and every lowercase letter found room: {swapped:?}"
    );

    show(init, vec![upper, lower]);
    draw(init, &camera);
    let crowded = init.renderer.glyph_run_counts();
    assert!(
        crowded.atlas.refused > 0,
        "both alphabets cannot share the frozen page: {crowded:?}"
    );
    assert_eq!(
        crowded.instances as u64 + crowded.atlas.refused - swapped.atlas.refused,
        52,
        "every glyph is drawn or refused, and the frame is drawn: {crowded:?}"
    );
    assert_eq!(crowded.draws, 1, "{crowded:?}");
}
