// SPDX-License-Identifier: MPL-2.0

//! A map's one background (V1-B12), held to the real renderer
//! (TEST_CONVENTIONS §T8): every frame is cleared to the map's resolved
//! background, and nothing the map draws is a rect — a node has no fill,
//! and every other color comes from a glyph. Every assertion is on the
//! renderer's state — the color the render pass is cleared to and the
//! rects the frame drew — never on pixels (§T10).

use std::time::{Duration, Instant};

use baumhard::document::format::load;
use baumhard::document::resolve::resolve_background;
use baumhard::document::values::Rgba;
use baumhard::gfx_structs::camera::Camera2D;
use serde_json::json;
use wgpu::Color;

use super::gpu_harness::{headless_app, HARNESS_SIZE};
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use super::tests_gpu::TempFile;
use crate::application::ipc::run_attached;
use crate::application::renderer::{CaptureFailure, FrameSkip, ModalLayer};

fn map_path(name: &str) -> String {
    format!("{}/maps/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Draw one frame of `init`'s renderer, the modal layer included,
/// waiting out font-lock contention.
fn draw(init: &mut InitState) {
    let camera = Camera2D::new(HARNESS_SIZE.0, HARNESS_SIZE.1);
    let deadline = Instant::now() + baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    loop {
        match init
            .renderer
            .capture_frame(&camera, ModalLayer::Drawn, &mut || {})
        {
            Ok(_) => return,
            Err(CaptureFailure::Skipped(FrameSkip::FontSystemContended)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(other) => panic!("the frame was not drawn: {other:?}"),
        }
    }
}

/// The clear value a target of the harness's format — an sRGB one —
/// holds `rgb` in: each byte decoded to linear light by the sRGB
/// transfer function, written out here from its definition (IEC
/// 61966-2-1) rather than taken from the renderer, and opaque.
fn srgb_clear(rgb: [u8; 3]) -> Color {
    let decode = |byte: u8| {
        let c = f64::from(byte) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Color {
        r: decode(rgb[0]),
        g: decode(rgb[1]),
        b: decode(rgb[2]),
        a: 1.0,
    }
}

fn assert_clear(got: Color, want: Color, what: &str) {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(
        close(got.r, want.r) && close(got.g, want.g) && close(got.b, want.b) && close(got.a, want.a),
        "{what}: the frame is cleared to {got:?}, not {want:?}"
    );
}

/// A `.mandala.json` with a two-variant theme and `background` as
/// given (`None` for no key).
fn document_text(background: Option<&str>) -> String {
    let background = background
        .map(|value| format!(r#""background":"{value}","#))
        .unwrap_or_default();
    format!(
        r##"{{"format":1,"name":"bg","theme":{{"active":"dark","variants":{{"dark":{{"bg":"#80402040"}},"light":{{"bg":"#f0e0d0"}}}}}},{background}"nodes":[{{"id":1,"position":[0,0]}}]}}"##
    )
}

/// The frame is cleared to the map's resolved background: black for a
/// document that gives none, a theme variable's value in the active
/// variant — opaque, though the variable holds an alpha — and the other
/// variant's once it is the active one. The maps the app loads take the
/// same path at load (`MapView::load` resolves it): `theme_demo`'s
/// `var(--bg)` is `#141414`, `testament`'s is black.
/// Controls: each step changes the clear color the one before it left,
/// so none of these is the renderer's starting black restated.
#[test]
fn test_the_frame_is_cleared_to_the_maps_resolved_background() {
    let Some(mut app) = headless_app(&map_path("theme_demo.mandala.json")) else {
        return;
    };
    let init = &mut app.init;

    // The legacy load path: `canvas.background_color` through the theme.
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0x14, 0x14, 0x14]),
        "theme_demo's var(--bg) at load",
    );

    // A document with no background: black, the format's default.
    let doc = load(&document_text(None)).expect("the map loads");
    let background = resolve_background(doc.tree(), &mut ());
    assert_eq!(background, Rgba::opaque(0, 0, 0), "the default background");
    init.renderer.set_background(background.0);
    draw(init);
    assert_clear(init.renderer.clear_color(), srgb_clear([0, 0, 0]), "the default");

    // A theme variable: the active variant's value, its alpha not applied.
    let doc = load(&document_text(Some("var(bg)"))).expect("the map loads");
    let background = resolve_background(doc.tree(), &mut ());
    init.renderer.set_background(background.0);
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0x80, 0x40, 0x20]),
        "var(bg) in the dark variant",
    );

    // The other variant, once it is the active one.
    let light = document_text(Some("var(bg)")).replace(r#""active":"dark""#, r#""active":"light""#);
    let doc = load(&light).expect("the map loads");
    init.renderer
        .set_background(resolve_background(doc.tree(), &mut ()).0);
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0xf0, 0xe0, 0xd0]),
        "var(bg) in the light variant",
    );
}

/// One IPC request through the app's own command target, its reply
/// asserted ok, then one frame's drain, as a hand's input lands.
fn act(init: &mut InitState, request: serde_json::Value) -> serde_json::Value {
    let (reply, _) = run_attached(&request.to_string(), &mut IpcCommandTarget::new(init));
    assert_eq!(reply["ok"], json!(true), "{request} is answered ok: {reply}");
    init.input_context().drain_frame(Beat::Loop);
    reply
}

/// **The clear follows the background through the app's own routes**,
/// not only at load: a reload of another map through the console's
/// `open`. `theme_demo`'s background is `var(--bg)` in its dark variant,
/// testament's is black.
///
/// Control: the step's expectation differs from the clear load left, so
/// a clear that stayed where load put it — the defect this pins — fails.
/// (The theme-variant switch a click fired, and its undo, return with
/// the map's verbs and edits: V1-D3 #252, V1-D1 #250.)
#[test]
fn test_the_clear_follows_a_reload_through_the_console() {
    let Some(mut app) = headless_app(&map_path("theme_demo.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0x14, 0x14, 0x14]),
        "the control: theme_demo's dark variant at load",
    );
    act(
        init,
        json!({"id": 5, "cmd": "act.console", "line": format!("open {}", map_path("testament.mandala.json"))}),
    );
    let background = resolve_background(init.document.as_ref().expect("document").doc().tree(), &mut ());
    assert_eq!(
        background.0,
        [0, 0, 0, 0xff],
        "precondition: testament is open, and its background is black"
    );
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0, 0, 0]),
        "testament's black after the reload (the control is the dark clear above)",
    );
}

/// A map draws no fill (V1-B12): a frame of `testament` — whose every
/// node carries a `style.background_color`, which the old content pass
/// drew one rect for — hands no pass a single fill block. The frame has
/// no rect pipeline left to draw a fill through, so a fill would come
/// back as glyphs, the backdrops' own (Public Pixel's `█`); the census
/// counts those in every list the three glyph passes are handed
/// (`backdrop::FillCensus`), the canvas's and the chrome's, so it sees
/// a fill whichever pass and whichever batch it came back in.
///
/// Controls, one per half, so neither zero is a counter that never
/// moves: opening the console hands the chrome exactly the cells its
/// backdrop's rectangle needs, `⌈w / 64⌉ × ⌈h / 64⌉` of the pixels it
/// covers, while the canvas stays at zero; and the same map with every
/// section's text a Public Pixel `█` hands the canvas one block per
/// section on screen — a content fill, counted.
#[test]
fn test_a_frame_draws_no_content_fill() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    let doc = init.document.as_ref().expect("the harness builds a document");
    let nodes = crate::application::document::selection::every_node(doc.doc()).count();
    assert!(
        nodes > 100,
        "precondition: testament is drawn whole ({nodes} nodes)"
    );

    draw(init);
    let census = init.renderer.fill_census_of_last_frame();
    assert_eq!(
        (census.canvas, census.chrome),
        (0, 0),
        "no node, section or other element of the map draws a fill, and no chrome is open"
    );

    let (reply, _) = run_attached(
        &json!({"id": 1, "cmd": "act.action", "action": "OpenConsole"}).to_string(),
        &mut IpcCommandTarget::new(init),
    );
    assert_eq!(reply["ok"], json!(true), "the console opens: {reply}");
    assert!(init.console_state.is_open(), "precondition: the console is open");
    draw(init);
    let (left, top, width, height) = init
        .renderer
        .console_backdrop_extent()
        .expect("an open console has a backdrop");
    // The pixels whose centers the rectangle holds, at the window's ratio
    // of one, and the 64-pixel cells that cover them.
    let edge = |at: f32| (f64::from(at) - 0.5).ceil() as i64;
    let cells = |from: f32, to: f32| ((edge(to) - edge(from)) as f64 / 64.0).ceil() as usize;
    let expected = cells(left, left + width) * cells(top, top + height);
    assert!(
        expected > 1,
        "precondition: the console's backdrop spans cells ({expected})"
    );
    let census = init.renderer.fill_census_of_last_frame();
    assert_eq!(
        (census.canvas, census.chrome),
        (0, expected),
        "the control: the console's backdrop is fill blocks, one per cell, all in the chrome pass"
    );
    // One app at a time (`gpu_harness`): this one goes before the next.
    drop(app);

    // The canvas half's control: testament with every section's text a
    // fill block in the backdrop's face.
    let mut map: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(map_path("testament.mandala.json")).expect("testament"),
    )
    .expect("testament is JSON");
    fn block_every_section(value: &mut serde_json::Value, sections: &mut usize) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(spans) = object.get_mut("spans") {
                    *spans = json!([{"text": "\u{2588}", "font": "Public Pixel", "size": 74.0, "color": "#ffffff"}]);
                    *sections += 1;
                }
                // A frame's rings are sized by the ring, not by their
                // spans: only a section's text is rewritten.
                for (key, child) in object.iter_mut() {
                    if key != "frame" {
                        block_every_section(child, sections);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    block_every_section(item, sections);
                }
            }
            _ => {}
        }
    }
    let mut sections = 0;
    block_every_section(&mut map, &mut sections);
    assert!(
        sections > 100,
        "precondition: the map's sections are rewritten ({sections})"
    );
    let blocks = TempFile::new("fill-census", "mandala.json");
    std::fs::write(blocks.as_str(), map.to_string()).expect("the temp map is written");
    let Some(mut blocked) = headless_app(blocks.as_str()) else {
        return;
    };
    let loaded = blocked.init.document.as_ref().expect("a document");
    assert!(
        !loaded.is_load_failure_placard(),
        "precondition: the rewritten map loads ({:?})",
        loaded.status()
    );
    draw(&mut blocked.init);
    let census = blocked.init.renderer.fill_census_of_last_frame();
    assert!(
        census.canvas > 0 && census.chrome == 0,
        "the control: a map whose text is fill blocks hands the canvas passes fill blocks ({census:?})"
    );
}

/// **A resize lays the open console out again** at the new size: its
/// backdrop — the full-block cells — and its glyphs are bottom-anchored
/// against the surface, so a resize that left them as they were would
/// draw the panel where the old size put it, until the next keystroke.
/// After `resize_surface` (the body of the window's `Resized` arm), the
/// console's backdrop is the one a fresh rebuild at the new size lays
/// out, and its bottom edge sits at the new bottom.
///
/// Control: the backdrop before the resize differs from the one after,
/// so a resize that kept the old layout fails the comparison with the
/// fresh rebuild rather than passing it by coincidence.
#[test]
fn test_a_resize_lays_the_open_console_out_again() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    act(
        init,
        json!({"id": 1, "cmd": "act.action", "action": "OpenConsole"}),
    );
    assert!(init.console_state.is_open(), "precondition: the console is open");
    draw(init);
    let before = init
        .renderer
        .console_backdrop_extent()
        .expect("an open console has a backdrop");

    let (width, height) = (HARNESS_SIZE.0 + 320, HARNESS_SIZE.1 + 240);
    init.resize_surface(width, height);
    let after = init
        .renderer
        .console_backdrop_extent()
        .expect("the console is still open");
    assert_ne!(
        after, before,
        "the resize moves the console's backdrop to the new size, where a kept layout leaves it"
    );

    // A fresh rebuild at the new size, as the next keystroke would make.
    let doc = init.document.as_ref().expect("the harness builds a document");
    super::console_input::rebuild_console_overlay(
        &init.console_state,
        doc,
        &mut init.app_scene,
        &mut init.renderer,
        &init.keybinds,
    );
    let fresh = init
        .renderer
        .console_backdrop_extent()
        .expect("the console is still open");
    assert_eq!(
        after, fresh,
        "the resize lays the console out as a rebuild at the new size does"
    );
    let (_, top, _, h) = after;
    assert!(
        (top + h) <= height as f32 && (height as f32 - (top + h)) < 64.0,
        "the backdrop is bottom-anchored at the new height {height}: {after:?}"
    );
    draw(init);
}

/// **A renderer on a linear target hands both glyph passes the linear
/// encoding**: built in `Bgra8Unorm` — the kind of target the browser's
/// WebGPU surface always is, and one lavapipe renders to — glyphon's
/// atlas is given `ColorMode::Web`, the glyph-run pass writes for
/// `Linear`, and its color page is `Rgba8Unorm`, so a glyph colored like
/// the background matches it there as on native. Then one frame is
/// prepared and encoded into a texture of that format, so the pipelines
/// built for it are the ones a frame uses.
///
/// Control: the app's own headless renderer, on its sRGB target, is
/// handed the other answer for each — `Accurate`, `Srgb`,
/// `Rgba8UnormSrgb` — so the linear answers are the target's and not
/// constants.
#[test]
fn test_a_linear_target_hands_both_glyph_passes_the_linear_encoding() {
    use super::gpu_harness::headless_renderer_in;
    use crate::application::renderer::TargetEncoding;
    use glyphon::ColorMode;
    use wgpu::TextureFormat::{Bgra8Unorm, Rgba8Unorm, Rgba8UnormSrgb};

    // The control first: the app's own renderer.
    {
        let Some(app) = headless_app(&map_path("testament.mandala.json")) else {
            return;
        };
        assert_eq!(
            app.init.renderer.glyph_pass_encodings(),
            (ColorMode::Accurate, (TargetEncoding::Srgb, Rgba8UnormSrgb)),
            "the control: the app's sRGB target is handed the sRGB answers"
        );
    }
    let Some(mut linear) = headless_renderer_in(Bgra8Unorm) else {
        return;
    };
    assert_eq!(
        linear.renderer.glyph_pass_encodings(),
        (ColorMode::Web, (TargetEncoding::Linear, Rgba8Unorm)),
        "a Bgra8Unorm renderer hands glyphon `Web`, and the run pass `Linear` with a linear color page"
    );
    linear
        .renderer
        .process_decree(crate::application::common::RenderDecree::StartRender);
    let camera = Camera2D::new(HARNESS_SIZE.0, HARNESS_SIZE.1);
    let deadline = Instant::now() + baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    loop {
        match linear
            .renderer
            .capture_frame(&camera, ModalLayer::Drawn, &mut || {})
        {
            Ok(_) => break,
            Err(CaptureFailure::Skipped(FrameSkip::FontSystemContended)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(other) => panic!("a frame on the linear target is drawn: {other:?}"),
        }
    }
}
