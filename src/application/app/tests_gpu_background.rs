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
use crate::application::renderer::{CanvasSource, CaptureFailure, FrameSkip, ModalLayer};

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
            .capture_frame(&camera, ModalLayer::Drawn, CanvasSource::Live, &mut || {})
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
/// variant's once it is the active one. The legacy map the app loads
/// takes the same path at load (`MindMapDocument::background`):
/// `theme_demo`'s `var(--bg)` is `#141414`, `testament`'s is black.
/// Controls: each step changes the clear color the one before it left,
/// so none of these is the renderer's starting black restated.
#[test]
fn test_the_frame_is_cleared_to_the_maps_resolved_background() {
    let Some(mut app) = headless_app(&map_path("theme_demo.mindmap.json")) else {
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
/// not only at load: a theme-variant switch fired by a click on
/// `theme_demo`'s "light" node (its `OnClick` binding to the map's
/// `switch-light` mutation, whose one document action is
/// `SetThemeVariant`), the undo of that switch, and a reload of another
/// map through the console's `open`. `theme_demo`'s background is
/// `var(--bg)`, so the variant is what moves it. (The document has no
/// redo to exercise.)
///
/// Controls: each step's expectation is checked against the clear the
/// step before it left, and the two differ, so a clear that stayed
/// where load put it — the defect this pins — fails at the first step;
/// the document's own background is asserted to have moved first, so a
/// click that missed the node cannot pass as a clear that did not
/// follow.
#[test]
fn test_the_clear_follows_a_theme_variant_switch_its_undo_and_a_reload() {
    let Some(mut app) = headless_app(&map_path("theme_demo.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    let background = |init: &InitState| {
        init.document
            .as_ref()
            .expect("the harness builds a document")
            .background()
    };

    draw(init);
    let dark = srgb_clear([0x14, 0x14, 0x14]);
    assert_clear(
        init.renderer.clear_color(),
        dark,
        "the control: theme_demo's dark variant at load",
    );

    // The click: node 2's body, the "light" switch.
    let light_node = init
        .document
        .as_ref()
        .expect("document")
        .mindmap
        .nodes
        .get("2")
        .expect("theme_demo has node 2");
    assert!(
        light_node
            .trigger_bindings
            .iter()
            .any(|binding| binding.mutation_id == "switch-light"),
        "precondition: node 2 is the light switch"
    );
    let center = glam::Vec2::new(
        light_node.position.x as f32 + light_node.size.width as f32 / 2.0,
        light_node.position.y as f32 + light_node.size.height as f32 / 2.0,
    );
    act(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": center.x, "y": center.y, "space": "canvas"}),
    );
    act(
        init,
        json!({"id": 2, "cmd": "input.mouse_button", "button": "left", "state": "click"}),
    );
    assert_eq!(
        background(init),
        [0xf5, 0xf1, 0xe8, 0xff],
        "precondition: the click switched the document to the light variant"
    );
    draw(init);
    let light = srgb_clear([0xf5, 0xf1, 0xe8]);
    assert_clear(
        init.renderer.clear_color(),
        light,
        "the light variant, switched by a click (the control is the dark clear above)",
    );

    // The undo restores the dark variant, and the clear with it.
    act(init, json!({"id": 3, "cmd": "act.action", "action": "Undo"}));
    assert_eq!(
        background(init),
        [0x14, 0x14, 0x14, 0xff],
        "precondition: the undo restored the dark variant"
    );
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        dark,
        "the dark variant after the undo (the control is the light clear above)",
    );

    // A reload: another map, through the console. The switch dirtied the
    // map, so it is saved first, as the dirty guard asks.
    let saved = TempFile::new("background-reload", "mindmap.json");
    act(
        init,
        json!({"id": 4, "cmd": "act.console", "line": format!("save {}", saved.as_str())}),
    );
    act(
        init,
        json!({"id": 5, "cmd": "act.console", "line": format!("open {}", map_path("testament.mindmap.json"))}),
    );
    assert_eq!(
        background(init),
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
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    let doc = init.document.as_ref().expect("the harness builds a document");
    let filled = doc
        .mindmap
        .nodes
        .values()
        .filter(|node| !node.style.background_color.is_empty())
        .count();
    assert!(
        filled > 100,
        "precondition: testament's nodes carry the background colors the old pass drew ({filled})"
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
        &std::fs::read_to_string(map_path("testament.mindmap.json")).expect("testament"),
    )
    .expect("testament is JSON");
    let mut sections = 0;
    for node in map["nodes"].as_object_mut().expect("nodes").values_mut() {
        for section in node["sections"].as_array_mut().into_iter().flatten() {
            section["text"] = json!("\u{2588}");
            section["text_runs"] =
                json!([{"start": 0, "end": 1, "font": "Public Pixel", "size_pt": 74.0, "color": "#ffffff"}]);
            sections += 1;
        }
    }
    assert!(
        sections > 100,
        "precondition: the map's sections are rewritten ({sections})"
    );
    let blocks = TempFile::new("fill-census", "mindmap.json");
    std::fs::write(blocks.as_str(), map.to_string()).expect("the temp map is written");
    let Some(mut blocked) = headless_app(blocks.as_str()) else {
        return;
    };
    draw(&mut blocked.init);
    let census = blocked.init.renderer.fill_census_of_last_frame();
    assert!(
        census.canvas > 0 && census.chrome == 0,
        "the control: a map whose text is fill blocks hands the canvas passes fill blocks ({census:?})"
    );
}
