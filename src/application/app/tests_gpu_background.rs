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
    init.renderer.set_background(resolve_background(doc.tree(), &mut ()).0);
    draw(init);
    assert_clear(
        init.renderer.clear_color(),
        srgb_clear([0xf0, 0xe0, 0xd0]),
        "var(bg) in the light variant",
    );
}

/// A map draws no rect: the content fill pass is gone (V1-B12), so a
/// frame of `testament` — whose every node carries a
/// `style.background_color`, which the old pass drew one rect for —
/// draws none. The control re-adds a fill through the one route left,
/// the console's screen-space backdrop: opening the console draws
/// exactly one rect, so the zero is the absence of a pass and not a
/// counter that never moves.
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
    assert_eq!(
        init.renderer.rect_quads_in_last_frame(),
        0,
        "no node, section or other element of the map draws a rect"
    );

    let (reply, _) = run_attached(
        &json!({"id": 1, "cmd": "act.action", "action": "OpenConsole"}).to_string(),
        &mut IpcCommandTarget::new(init),
    );
    assert_eq!(reply["ok"], json!(true), "the console opens: {reply}");
    assert!(init.console_state.is_open(), "precondition: the console is open");
    draw(init);
    assert_eq!(
        init.renderer.rect_quads_in_last_frame(),
        1,
        "the control: the console's backdrop is a rect, and the frame counts it"
    );
}
