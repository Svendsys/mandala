// SPDX-License-Identifier: MPL-2.0

//! The first GPU-backed tests (TEST_CONVENTIONS §T8): the real app,
//! headless, on this machine's adapter, driven through the funnels an
//! agent or a user drives it through — the IPC `act.*` commands and
//! the keyboard — with every assertion on state: the document's bytes,
//! the undo stack, the installed tree, what a rebuild left in the
//! renderer, the redraw ledger. Never pixels (§T10).
//!
//! Each test begins with [`headless_app`] and returns when it answers
//! `None`: no adapter here, the note has been printed, and the test
//! passes having proved nothing — which CI forbids by setting
//! `MANDALA_REQUIRE_GPU` (`gpu_harness`).
//!
//! What these pin that the renderer-free suite could not: that the
//! arms behind `&mut Renderer` run end to end — `dispatch_action`'s
//! `DeleteSelection` and `Undo` with their rebuilds, the console's
//! document swap through its side-effect drain, a camera decree
//! through the frame's reprojection, and a frame drawn on the adapter
//! and read back by `capture.screenshot` — and leave the state they
//! promise. The window's own frame is not among them: under
//! `Shell::Headless` there is no surface to present to, so
//! `draw_frame` keeps the redraw ledger and draws nothing
//! (`Sink::presents`); the one route a headless app draws through is
//! the capture engine, which is why the third test captures.

use std::path::Path;

use serde_json::{json, Value};

use super::gpu_harness::{headless_app, HARNESS_SIZE};
use super::heartbeat::Beat;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::InteractionMode;
use crate::application::capture::png;
use crate::application::document::tests_common::first_testament_node_id;
use crate::application::document::SelectionState;
use crate::application::ipc::run_attached;
use crate::application::platform::input::{ElementState, Key, NamedKey};
use baumhard::mindmap::loader;
use baumhard::mindmap::model::MindMap;

fn map_path(name: &str) -> String {
    format!("{}/maps/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The bytes a save would write, as JSON: the one reading of the whole
/// document that two apps, or one app before and after an undo, can be
/// compared on without naming fields.
fn document_json(init: &InitState) -> Value {
    let doc = init
        .document
        .as_ref()
        .expect("the harness always builds a document");
    loader::to_json_value(&doc.mindmap).expect("the document serializes")
}

/// Run one `act.*` request against `init`, through the real decoder,
/// executor and wire encoding. Returns the reply and the repaint flag.
fn act(init: &mut InitState, request: Value) -> (Value, bool) {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init))
}

/// `true` iff `node` is in the installed tree.
fn tree_has(init: &InitState, node: &str) -> bool {
    init.mindmap_tree
        .as_ref()
        .expect("the harness installs a tree")
        .arena_id_for(node)
        .is_some()
}

/// **The issue's first consumer: undo fidelity through the real
/// arms.** `act.action DeleteSelection` then `act.action Undo`, with
/// a device behind every rebuild, lands exactly where the keyboard's
/// `Delete` then `Undo` land — the same document bytes after the
/// delete, and the baseline's bytes after the undo — and each step
/// leaves the tree and the renderer's text buffers agreeing with the
/// document.
///
/// The IPC route and the keyboard route each run in their own app
/// (the harness builds one at a time), on the same fixture and the
/// same node, so the comparison is between two complete runs of the
/// production funnels, not between a funnel and a hand-built
/// expectation. The negative controls are in the middle: the post-
/// delete bytes differ from the baseline, the node is gone from
/// document, tree and buffers, and one undo entry stands — so a route
/// that silently did nothing cannot pass by agreeing with a baseline
/// it never left.
#[test]
fn test_delete_selection_then_undo_through_act_action_matches_the_keyboard_path() {
    let Some(mut ipc) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let node = first_testament_node_id(ipc.init.document.as_ref().expect("document"));
    let baseline = document_json(&ipc.init);
    let buffers_before = ipc.init.renderer.text_buffer_owner_count();
    assert!(
        tree_has(&ipc.init, &node),
        "precondition: the fixture node is in the installed tree"
    );

    // The IPC route.
    ipc.init.document.as_mut().expect("document").selection = SelectionState::Single(node.clone());
    let (reply, repaint) = act(
        &mut ipc.init,
        json!({"id": 1, "cmd": "act.action", "action": "DeleteSelection"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(repaint, "a delete changes what the window draws");
    let after_delete = document_json(&ipc.init);
    assert_ne!(
        after_delete, baseline,
        "control: the delete landed in the document"
    );
    {
        let doc = ipc.init.document.as_ref().expect("document");
        assert!(
            !doc.mindmap.nodes.contains_key(&node),
            "the node is gone from the model"
        );
        assert_eq!(doc.undo_stack.len(), 1, "one entry for the delete");
    }
    assert!(
        !tree_has(&ipc.init, &node),
        "the rebuild that followed the delete dropped the node from the tree"
    );
    let buffers_after_delete = ipc.init.renderer.text_buffer_owner_count();
    assert!(
        buffers_after_delete < buffers_before,
        "the rebuild dropped the node's text buffers: {buffers_after_delete} of {buffers_before} remain"
    );

    let (reply, repaint) = act(
        &mut ipc.init,
        json!({"id": 2, "cmd": "act.action", "action": "Undo"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(repaint);
    assert_eq!(
        document_json(&ipc.init),
        baseline,
        "undo through act.action restores the document byte for byte"
    );
    assert!(
        ipc.init
            .document
            .as_ref()
            .expect("document")
            .undo_stack
            .is_empty(),
        "the entry was consumed"
    );
    assert!(
        tree_has(&ipc.init, &node),
        "the rebuild that followed the undo put the node back in the tree"
    );
    assert_eq!(
        ipc.init.renderer.text_buffer_owner_count(),
        buffers_before,
        "the rebuild that followed the undo shaped the node's text again"
    );
    drop(ipc);

    // The keyboard route, in a second app: the default binds are
    // `Delete` → DeleteSelection and the `Undo` key → Undo.
    let Some(mut keyboard) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    assert_eq!(
        document_json(&keyboard.init),
        baseline,
        "precondition: both apps start from one document"
    );
    keyboard.init.document.as_mut().expect("document").selection = SelectionState::Single(node.clone());
    assert!(keyboard
        .init
        .input_context()
        .on_keyboard_input(Key::Named(NamedKey::Delete), ElementState::Pressed));
    assert_eq!(
        document_json(&keyboard.init),
        after_delete,
        "the keyboard's Delete and act.action DeleteSelection land the same document"
    );
    assert!(!tree_has(&keyboard.init, &node));
    assert_eq!(
        keyboard.init.renderer.text_buffer_owner_count(),
        buffers_after_delete
    );
    assert!(keyboard
        .init
        .input_context()
        .on_keyboard_input(Key::Named(NamedKey::Undo), ElementState::Pressed));
    assert_eq!(
        document_json(&keyboard.init),
        baseline,
        "the keyboard's Undo restores the baseline as act.action Undo did"
    );
    assert!(tree_has(&keyboard.init, &node));
    assert_eq!(keyboard.init.renderer.text_buffer_owner_count(), buffers_before);
}

/// Removes a file when dropped, so a test that writes one leaves
/// nothing behind whichever assertion stops it.
struct TempFile(std::path::PathBuf);

impl TempFile {
    /// A fresh path under the temp dir, `mandala-tests-gpu-<stem>-…
    /// .<ext>`, unique to this process and test thread; whatever was
    /// there is removed first.
    fn new(stem: &str, ext: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mandala-tests-gpu-{stem}-{}-{}.{ext}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test").replace("::", "-")
        ));
        let _ = std::fs::remove_file(&path);
        TempFile(path)
    }

    /// The file `suffix` appended to this one's path names — the
    /// geometry sidecar beside a PNG — so it is removed with it.
    fn sibling(&self, suffix: &str) -> Self {
        TempFile(std::path::PathBuf::from(format!("{}{suffix}", self.as_str())))
    }

    fn as_str(&self) -> &str {
        self.0.to_str().expect("a UTF-8 temp path")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// **The issue's second consumer: `act.console` through the whole
/// side-effect drain.** The dirty guard — the one thing between a
/// typo'd `open` and lost work — refuses `open` and `new` over an
/// unsaved change, through the same drain, leaving everything as it
/// was; `save <path>` clears it; `open <path>` then swaps the
/// document, drops the installed tree and the post-line rebuild
/// installs one for the new map; and `new` swaps in a blank map the
/// same way.
///
/// The expected node counts come from an independent load of the same
/// file and from `MindMap::new_blank` directly, never from the app
/// under test (§T13's mirror rule). The precondition that the two
/// fixtures differ in node count is asserted, or a swap that did
/// nothing would read as one that worked; and the refusals are
/// asserted against the document's bytes, not only the reply.
#[test]
fn test_act_console_open_and_new_replace_the_document_through_the_side_effect_drain() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let other = map_path("theme_demo.mindmap.json");
    let expected = loader::load_from_file(Path::new(&other)).expect("the fixture loads");
    let testament_nodes = app.init.document.as_ref().expect("document").mindmap.nodes.len();
    assert_ne!(
        expected.nodes.len(),
        testament_nodes,
        "the fixtures must differ in node count or this test proves nothing"
    );

    // The dirty guard, through the drain: delete a node, then try to
    // swap the document out from under the unsaved change.
    let node = first_testament_node_id(app.init.document.as_ref().expect("document"));
    app.init.document.as_mut().expect("document").selection = SelectionState::Single(node);
    let (reply, _) = act(
        &mut app.init,
        json!({"id": 1, "cmd": "act.action", "action": "DeleteSelection"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(
        app.init.document.as_ref().expect("document").dirty,
        "precondition: the delete dirtied the map"
    );
    let dirty_json = document_json(&app.init);
    let buffers_dirty = app.init.renderer.text_buffer_owner_count();
    for (id, line) in [(2, format!("open {other}")), (3, "new".to_string())] {
        let (reply, repaint) = act(
            &mut app.init,
            json!({"id": id, "cmd": "act.console", "line": line}),
        );
        assert_eq!(
            reply["ok"], false,
            "`{line}` over unsaved work is refused: {reply}"
        );
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("unsaved")),
            "the refusal names the unsaved work: {reply}"
        );
        assert!(!repaint, "a refused `{line}` changes nothing the window draws");
        assert_eq!(
            document_json(&app.init),
            dirty_json,
            "the refused `{line}` left the document as it was"
        );
        assert_eq!(app.init.renderer.text_buffer_owner_count(), buffers_dirty);
    }

    // `save <path>` clears the guard and binds the path.
    let saved = TempFile::new("saved", "mindmap.json");
    let (reply, _) = act(
        &mut app.init,
        json!({"id": 4, "cmd": "act.console", "line": format!("save {}", saved.as_str())}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    {
        let doc = app.init.document.as_ref().expect("document");
        assert!(!doc.dirty, "saved: clean");
        assert_eq!(
            doc.file_path.as_deref(),
            Some(saved.as_str()),
            "saved: rebound to the new path"
        );
    }
    assert_eq!(
        loader::load_from_file(&saved.0)
            .expect("the saved file loads")
            .nodes
            .len(),
        testament_nodes - 1,
        "the file on disk holds the map minus the deleted node"
    );

    // `open <path>`: the swap, the dropped tree, the rebuild.
    let (reply, repaint) = act(
        &mut app.init,
        json!({"id": 5, "cmd": "act.console", "line": format!("open {other}")}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(repaint, "a document swap repaints");
    {
        let doc = app.init.document.as_ref().expect("document");
        assert_eq!(
            doc.mindmap.nodes.len(),
            expected.nodes.len(),
            "the opened map is installed"
        );
        assert_eq!(
            doc.file_path.as_deref(),
            Some(other.as_str()),
            "and bound to its path"
        );
        assert!(!doc.dirty, "a freshly opened map is clean");
        assert_eq!(
            doc.selection,
            SelectionState::None,
            "the old selection did not survive the swap"
        );
        let tree = app
            .init
            .mindmap_tree
            .as_ref()
            .expect("the post-line rebuild installed a tree");
        for id in expected.nodes.keys() {
            assert!(
                tree.arena_id_for(id).is_some(),
                "node {id} of the opened map is in the tree"
            );
        }
    }
    assert_eq!(app.init.interaction_mode, InteractionMode::Default);
    let buffers_after_open = app.init.renderer.text_buffer_owner_count();
    assert_ne!(
        buffers_after_open, buffers_dirty,
        "the rebuild shaped the opened map's text, not the old one's"
    );

    // `new`: a blank map through the same swap.
    let (reply, repaint) = act(
        &mut app.init,
        json!({"id": 6, "cmd": "act.console", "line": "new"}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(repaint);
    let blank_nodes = MindMap::new_blank("independent").nodes.len();
    {
        let doc = app.init.document.as_ref().expect("document");
        assert_eq!(doc.mindmap.nodes.len(), blank_nodes, "a blank map is installed");
        assert_eq!(doc.file_path, None, "`new` with no path binds none");
        assert!(!doc.dirty);
        let tree = app
            .init
            .mindmap_tree
            .as_ref()
            .expect("the rebuild installed the blank map's tree");
        for id in expected.nodes.keys() {
            assert!(
                tree.arena_id_for(id).is_none(),
                "the opened map's node {id} is gone from the tree"
            );
        }
    }
    assert_eq!(
        app.init.renderer.text_buffer_owner_count(),
        0,
        "a blank map shapes no text at all"
    );
}

/// **The issue's third consumer: a scene rebuild and a frame that
/// need a device.** `act.action ZoomIn` moves the camera through a
/// `CameraZoom` decree and flags the connection geometry dirty; the
/// frame's drain reprojects and clears the flag; then
/// `capture.screenshot` draws the window's view on the adapter —
/// `Renderer::prepare_frame`, `encode_frame`, the readback — and
/// writes the PNG and its geometry sidecar. The headless app's own
/// `draw_frame` is asked for too, and asserted for what it is:
/// bookkeeping. With no surface to present to it marks the redraw
/// ledger current and draws nothing, which is the "rendered" half of
/// `clock.wait until=settled` on a headless launch.
///
/// Assertions are on state, never pixels (§T10): the reply, the PNG's
/// dimensions, the sidecar's nodes against the document, the live
/// camera the capture promised to leave alone. Controls: the zoom
/// before the action, the dirty flag clear after the first drain, set
/// by the zoom and clear again after the next; the PNG absent before
/// the capture; and the ledger owing a frame before `draw_frame` and
/// none after.
#[test]
fn test_a_zoom_through_act_action_reprojects_and_a_capture_draws_the_frame() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    // A built app has its geometry flagged from the camera fit at
    // load, and the loop's first iteration drains that before anything
    // else runs; this is that iteration.
    app.init.input_context().drain_frame(Beat::Loop);
    let zoom_before = app.init.renderer.camera_zoom();
    assert!(
        !app.init.renderer.connection_geometry_dirty(),
        "precondition: after the first frame's drain the geometry is current"
    );

    let (reply, repaint) = act(
        &mut app.init,
        json!({"id": 1, "cmd": "act.action", "action": "ZoomIn"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(repaint);
    let zoomed = app.init.renderer.camera_zoom();
    assert!(
        zoomed > zoom_before,
        "ZoomIn moved the camera: {zoomed} from {zoom_before}"
    );
    assert!(
        app.init.renderer.connection_geometry_dirty(),
        "the zoom decree flagged the connection geometry for reprojection"
    );

    // The frame's input-driven half: the drain the event loop runs
    // before every redraw.
    app.init.input_context().drain_frame(Beat::Loop);
    assert!(
        !app.init.renderer.connection_geometry_dirty(),
        "the drain reprojected the connections and consumed the flag"
    );

    // The frame itself, drawn on the adapter and read back: the one
    // route a headless app draws through.
    let shot = TempFile::new("zoomed", "png");
    let sidecar = shot.sibling(".geometry.json");
    assert!(
        !Path::new(shot.as_str()).exists(),
        "control: no PNG before the capture"
    );
    let reply = capture_screenshot(&mut app.init, shot.as_str());
    assert_eq!(reply["ok"], true, "{reply}");
    let result = &reply["result"];
    assert_eq!(result["rendered"], true, "the frame was drawn: {reply}");
    assert_eq!(
        (result["width"].as_u64(), result["height"].as_u64()),
        (Some(u64::from(HARNESS_SIZE.0)), Some(u64::from(HARNESS_SIZE.1))),
        "the window's view is the harness surface: {reply}"
    );
    assert_eq!(result["path"], shot.as_str(), "{reply}");
    assert_eq!(result["sidecar_path"], sidecar.as_str(), "{reply}");

    let png_bytes = std::fs::read(shot.as_str()).expect("the PNG was written where the reply says");
    let (width, height, rgba) = png::tests::decode(&png_bytes);
    assert_eq!(
        (width, height),
        HARNESS_SIZE,
        "the PNG decodes to the harness surface's size"
    );
    assert_eq!(
        rgba.len(),
        (width * height * 4) as usize,
        "one RGBA pixel per surface pixel"
    );

    let sidecar_json: Value = serde_json::from_slice(
        &std::fs::read(sidecar.as_str()).expect("the sidecar was written beside the PNG"),
    )
    .expect("the sidecar is JSON");
    assert_eq!(
        (
            sidecar_json["surface"]["width"].as_u64(),
            sidecar_json["surface"]["height"].as_u64()
        ),
        (Some(u64::from(HARNESS_SIZE.0)), Some(u64::from(HARNESS_SIZE.1))),
        "the sidecar maps the surface the PNG shows"
    );
    let shown: Vec<&str> = sidecar_json["nodes"]
        .as_array()
        .expect("the sidecar lists nodes")
        .iter()
        .map(|node| node["id"].as_str().expect("every entry names a node"))
        .collect();
    assert!(
        !shown.is_empty(),
        "the capture framed the map: at least one node is in view"
    );
    {
        let doc = app.init.document.as_ref().expect("document");
        for id in &shown {
            assert!(
                doc.mindmap.nodes.contains_key(*id),
                "sidecar node {id} is a node of the document"
            );
        }
        assert_eq!(
            sidecar_json["document_revision"].as_u64(),
            Some(doc.revision()),
            "the sidecar is tied to the revision it was drawn from"
        );
    }
    assert_eq!(
        app.init.renderer.camera_zoom(),
        zoomed,
        "a capture draws through a camera of its own and leaves the live one alone"
    );
    assert!(
        !app.init.renderer.connection_geometry_dirty(),
        "and leaves the geometry as it found it"
    );

    // The headless app's own frame: ledger bookkeeping, no drawing.
    app.init.request_redraw();
    assert!(
        app.init.settling().redraw_owed,
        "control: a frame is owed before draw_frame runs"
    );
    app.init.draw_frame();
    assert!(
        !app.init.settling().redraw_owed,
        "draw_frame settled the ledger — with no surface to present to, that is all it does"
    );
}

/// `capture.screenshot path=<path>` through the real decoder and
/// executor, retried while the engine reports the one transient skip
/// — `font_system_contended`, the font lock held by another test's
/// shaping in this process — the way the heartbeat retries a skipped
/// frame on the next beat. Any other outcome is returned as is.
fn capture_screenshot(init: &mut InitState, path: &str) -> Value {
    const ATTEMPTS: usize = 200;
    let request = json!({"id": 2, "cmd": "capture.screenshot", "path": path});
    let mut reply = Value::Null;
    for _ in 0..ATTEMPTS {
        reply = act(init, request.clone()).0;
        if reply["result"]["skipped"] != "font_system_contended" {
            return reply;
        }
        std::thread::yield_now();
    }
    panic!("the font system stayed contended through {ATTEMPTS} capture attempts: {reply}");
}

/// **A finger on a resize handle resizes; a finger anywhere else pans**
/// (#160), through the real `dispatch_touch_event` on a device, which
/// no renderer-free test reaches: the claim is decided next to the
/// renderer that converts the finger to canvas space.
///
/// In `Resize` mode on node 1, a finger that lands on its SE handle
/// and drags drives the handle drag: the node grows, the camera does
/// not move, the drag ends at `None` on lift, and one undo entry
/// stands. Before #160 that drag was the recognizer's one-finger pan.
///
/// The control is the same finger, same mode, same distance, started
/// in the node's middle instead: it pans the camera and resizes
/// nothing. So the handle, and only the handle, is what changes where
/// the finger goes — which fails on a claim that never fires (every
/// touch to the recognizer: the size assertion) and on one that fires
/// everywhere (the control's camera assertion).
///
/// Two preconditions are asserted first, with their own messages, so a
/// drifted fixture does not fail as a claim that resized nothing: the
/// zoom-in moved the camera, and the corner the press sees is the
/// node's resize handle.
#[test]
fn test_a_finger_on_a_resize_handle_resizes_and_a_finger_elsewhere_pans() {
    use crate::application::platform::input::TouchPhase;
    use crate::application::platform::window::PhysicalPosition;

    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    init.document.as_mut().expect("document").selection = SelectionState::Single("1".into());
    let (reply, _) = act(
        init,
        json!({"id": 1, "cmd": "act.action", "action": "EnterResizeMode"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(matches!(init.interaction_mode, InteractionMode::Resize { .. }));

    let node_rect = |init: &InitState| {
        let n = &init.document.as_ref().expect("document").mindmap.nodes["1"];
        (
            glam::Vec2::new(n.position.x as f32, n.position.y as f32),
            glam::Vec2::new(n.size.width as f32, n.size.height as f32),
        )
    };
    // One finger, `id`, from `from` (surface pixels) 40 px down-right,
    // a frame between moves as a hand's samples land.
    let drag_finger = |init: &mut InitState, id: u64, from: glam::Vec2| {
        let at = |i: f32| PhysicalPosition::new(f64::from(from.x + i * 8.0), f64::from(from.y + i * 8.0));
        init.input_context()
            .dispatch_touch_event(TouchPhase::Started, id, at(0.0));
        for i in 1..=5 {
            init.input_context()
                .dispatch_touch_event(TouchPhase::Moved, id, at(i as f32));
            init.input_context().drain_frame(Beat::Loop);
        }
        init.input_context()
            .dispatch_touch_event(TouchPhase::Ended, id, at(5.0));
        init.input_context().drain_frame(Beat::Loop);
    };

    // Zoom in about the node so its middle, where the control drags
    // from, lies well clear of its handles: at the fixture's opening
    // zoom the node is a few handle tolerances across.
    let (pos, size) = node_rect(init);
    let center = pos + size / 2.0;
    let zoom_before = init.renderer.camera_zoom();
    for id in 0..12 {
        let (reply, _) = act(
            init,
            json!({"id": 10 + id, "cmd": "act.action", "action": "ZoomIn", "x": center.x, "y": center.y}),
        );
        assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    }
    init.input_context().drain_frame(Beat::Loop);
    let zoomed = init.renderer.camera_zoom();
    assert!(
        zoomed > zoom_before,
        "precondition: ZoomIn must zoom in about the node, or its middle may lie within a handle's tolerance: \
         {zoomed} from {zoom_before}"
    );
    // That the press at the corner captures the handle, asked of the
    // same hit-test the press runs and of nothing the touch claim
    // decides, so a fixture that drifts fails here and not as a claim
    // that resized nothing.
    let se = init.renderer.canvas_to_screen(pos + size);
    // The corner as the press sees it: through surface pixels and back.
    let corner = init.renderer.screen_to_canvas(se.x, se.y);
    let tolerance = super::HANDLE_HIT_TOLERANCE_PX * init.renderer.canvas_per_pixel();
    assert!(
        super::event_mouse_click::resize_handle_press(
            init.document.as_ref(),
            &init.interaction_mode,
            corner,
            tolerance
        )
        .node
        .is_some(),
        "precondition: node 1's SE corner must be its resize handle in Resize mode"
    );
    let undo_before = init.document.as_ref().expect("document").undo_stack.len();

    let camera = init.renderer.camera().position;
    drag_finger(init, 2, se);
    let (_, resized) = node_rect(init);
    assert!(
        resized.x > size.x && resized.y > size.y,
        "a finger dragged from the SE handle must grow the node: {size:?} -> {resized:?}"
    );
    assert_eq!(
        init.renderer.camera().position,
        camera,
        "a finger holding a handle is never the recognizer's, so it must not pan"
    );
    assert!(
        matches!(init.drag_state, super::DragState::None),
        "the lift ended the drag"
    );
    assert_eq!(
        init.document.as_ref().expect("document").undo_stack.len(),
        undo_before + 1,
        "the lift committed the resize with one undo entry of its own"
    );

    // Control: the same drag from the node's middle, still in Resize.
    let (pos, size) = node_rect(init);
    let camera = init.renderer.camera().position;
    let middle = init.renderer.canvas_to_screen(pos + size / 2.0);
    drag_finger(init, 3, middle);
    assert_eq!(node_rect(init).1, size, "off a handle the finger resizes nothing");
    assert_ne!(
        init.renderer.camera().position,
        camera,
        "off a handle the finger is the recognizer's one-finger pan"
    );
}

/// Node 1's canvas AABB as (min, size).
fn node_1_rect(init: &InitState) -> (glam::Vec2, glam::Vec2) {
    let n = &init.document.as_ref().expect("document").mindmap.nodes["1"];
    (
        glam::Vec2::new(n.position.x as f32, n.position.y as f32),
        glam::Vec2::new(n.size.width as f32, n.size.height as f32),
    )
}

/// Zoom in about node 1, so a press near one of its corners lands on
/// its body with pixels to spare: at the fixture's opening zoom an edge
/// label overlaps its SE corner, and a label outranks the node's body
/// in a press's hit-test. Each test still asserts where its own press
/// lands ([`press_lands_on`]); this only makes that likely.
fn zoom_onto_node_1(init: &mut InitState) {
    let (pos, size) = node_1_rect(init);
    let center = pos + size / 2.0;
    let zoom_before = init.renderer.camera_zoom();
    for id in 0..12 {
        let (reply, _) = act(
            init,
            json!({"id": 100 + id, "cmd": "act.action", "action": "ZoomIn", "x": center.x, "y": center.y}),
        );
        assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    }
    init.input_context().drain_frame(Beat::Loop);
    let zoomed = init.renderer.camera_zoom();
    assert!(
        zoomed > zoom_before,
        "precondition: ZoomIn must zoom in about node 1: {zoomed} from {zoom_before}"
    );
}

/// What a press at canvas `at` lands on, asked as the press asks it:
/// through surface pixels and back, then the click chain's hit-tests.
fn press_lands_on(init: &mut InitState, at: glam::Vec2) -> super::ClickHit {
    let surface = init.renderer.canvas_to_screen(at);
    let canvas = init.renderer.screen_to_canvas(surface.x, surface.y);
    super::compute_click_hit(canvas, init.mindmap_tree.as_mut(), &mut init.app_scene).click_hit
}

/// **`act.action FastResizeStart` with a pointer is refused, says
/// why, and starts nothing** (#194). The synthetic pointer IPC places
/// holds no button, so no release could end the drag the Action would
/// arm. Through the real decoder, `IpcCommandTarget` and the dispatch
/// arm, over node 1: the reply is `unhandled` with
/// `FastResizeRefusal::NO_HOLDER` as its `reason`, the drag state stays
/// `None`, and the document and undo stack are untouched.
///
/// The control is the same Action with no pointer at all, which the
/// arm refuses for a different, named cause (`NO_POINTER`): the
/// `reason` is the arm's verdict on this request, not a constant the
/// reply carries for every refusal. Its positive counterpart, a held
/// button through the same Action, is the next test.
#[test]
fn test_act_action_fast_resize_start_is_refused_by_name_and_starts_nothing() {
    use super::dispatch::native::FastResizeRefusal;

    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    zoom_onto_node_1(init);
    let baseline = document_json(init);
    let (pos, size) = node_1_rect(init);
    let near_se = pos + size * 0.9;

    let (reply, _) = act(
        init,
        json!({"id": 1, "cmd": "act.action", "action": "FastResizeStart", "x": near_se.x, "y": near_se.y}),
    );
    assert_eq!(reply["result"]["outcome"], "unhandled", "{reply}");
    assert_eq!(reply["result"]["reason"], FastResizeRefusal::NO_HOLDER, "{reply}");
    assert!(
        matches!(init.drag_state, super::DragState::None),
        "a refused fast-resize arms no drag"
    );
    init.input_context().drain_frame(Beat::Loop);
    assert_eq!(document_json(init), baseline, "a refusal writes nothing");
    assert!(init.document.as_ref().expect("document").undo_stack.is_empty());

    let (reply, _) = act(
        init,
        json!({"id": 2, "cmd": "act.action", "action": "FastResizeStart"}),
    );
    assert_eq!(reply["result"]["outcome"], "unhandled", "{reply}");
    assert_eq!(
        reply["result"]["reason"],
        FastResizeRefusal::NO_POINTER,
        "control: a request with no pointer is refused for its own cause: {reply}"
    );
}

/// **A `Ctrl+RightDrag` anchors at the press, through the real
/// threshold cross** (#194). A right press with Ctrl just inside node
/// 1's SE corner, then one move deep into its NW quadrant: the cross
/// fires on that move, with the cursor nearest NW, and the drag it
/// arms must be a node resize anchored SE and owned by the right
/// button. A cross that built its hit from the cursor at the cross
/// would anchor NW.
///
/// The control is the same press and move mirrored — pressed inside
/// the NW corner, crossing in the SE quadrant — which anchors NW: the
/// side follows the press, rather than being SE for every drag.
#[test]
fn test_a_ctrl_right_drag_anchors_its_resize_at_the_press() {
    use baumhard::mindmap::tree_builder::ResizeHandleSide;

    use super::throttled_interaction::ThrottledDrag;

    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    zoom_onto_node_1(init);
    let (pos, size) = node_1_rect(init);

    // Press the right button with Ctrl at canvas `press`, then one
    // move to canvas `cross`; answers the side and owner of the drag
    // the cross armed, then releases.
    let mut press_then_cross = |press: glam::Vec2, cross: glam::Vec2| {
        // Asked first, so a label drifting onto the press point fails
        // as that, not as an anchor that came out wrong.
        assert!(
            matches!(press_lands_on(init, press), super::ClickHit::Node(ref id, None) if id == "1"),
            "precondition: the press at {press:?} must land on node 1's body, with nothing over it"
        );
        for (id, request) in [
            json!({"cmd": "input.mouse_move", "x": press.x, "y": press.y, "space": "canvas", "mods": ["ctrl"]}),
            json!({"cmd": "input.mouse_button", "button": "right", "state": "press", "mods": ["ctrl"]}),
            json!({"cmd": "input.mouse_move", "x": cross.x, "y": cross.y, "space": "canvas", "mods": ["ctrl"]}),
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = request;
            request["id"] = json!(id);
            let (reply, _) = act(init, request);
            assert!(reply.get("error").is_none(), "{reply}");
            init.input_context().drain_frame(Beat::Loop);
        }
        let armed = match &init.drag_state {
            super::DragState::Throttled(drag) => match drag.as_ref() {
                ThrottledDrag::NodeResize(i) => Some((i.side, drag.as_dyn().started_with_right())),
                _ => None,
            },
            _ => None,
        };
        let (reply, _) = act(
            init,
            json!({"id": 9, "cmd": "input.mouse_button", "button": "right", "state": "release", "mods": ["ctrl"]}),
        );
        assert!(reply.get("error").is_none(), "{reply}");
        init.input_context().drain_frame(Beat::Loop);
        armed
    };

    let near_se = pos + size * 0.9;
    let nw_quadrant = pos + size * 0.2;
    assert_eq!(
        press_then_cross(near_se, nw_quadrant),
        Some((ResizeHandleSide::SE, true)),
        "a press inside the SE corner anchors SE wherever the cross lands, and the right button owns it"
    );
    assert_eq!(
        press_then_cross(nw_quadrant, near_se),
        Some((ResizeHandleSide::NW, true)),
        "control: the mirrored press anchors NW"
    );
}

/// Run one `input.*` request and the frame after it, asserting the app
/// took it.
fn input(init: &mut InitState, request: Value) {
    let (reply, _) = act(init, request);
    assert!(reply.get("error").is_none(), "{reply}");
    init.input_context().drain_frame(Beat::Loop);
}

/// A left click, press and release in place, at canvas `at`.
fn left_click_at(init: &mut InitState, at: glam::Vec2) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": at.x, "y": at.y, "space": "canvas"}),
    );
    input(
        init,
        json!({"id": 2, "cmd": "input.mouse_button", "button": "left", "state": "click"}),
    );
}

/// A left drag from canvas `from`, 40 surface pixels down-right in five
/// moves, a frame between each as a hand's samples land.
fn left_drag_from(init: &mut InitState, from: glam::Vec2) {
    left_press_at(init, from);
    left_drag_on(init, from);
}

/// The left button pressed at canvas `at`, and held.
fn left_press_at(init: &mut InitState, at: glam::Vec2) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": at.x, "y": at.y, "space": "canvas"}),
    );
    input(
        init,
        json!({"id": 2, "cmd": "input.mouse_button", "button": "left", "state": "press"}),
    );
}

/// The held left button's drag from canvas `from`, as
/// [`left_drag_from`] makes it, then its release.
fn left_drag_on(init: &mut InitState, from: glam::Vec2) {
    let start = init.renderer.canvas_to_screen(from);
    for i in 1..=5 {
        let to = start + glam::Vec2::splat(8.0 * i as f32);
        input(
            init,
            json!({"id": 3, "cmd": "input.mouse_move", "x": to.x, "y": to.y, "space": "surface"}),
        );
    }
    input(
        init,
        json!({"id": 4, "cmd": "input.mouse_button", "button": "left", "state": "release"}),
    );
}

/// Node 1 selected and in `Resize` mode.
fn resize_node_1(init: &mut InitState) {
    init.document.as_mut().expect("document").selection = SelectionState::Single("1".into());
    let (reply, _) = act(
        init,
        json!({"id": 1, "cmd": "act.action", "action": "EnterResizeMode"}),
    );
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(matches!(init.interaction_mode, InteractionMode::Resize { .. }));
}

/// Whether a press at canvas `at`, through surface pixels and back as a
/// press sees it, lands on one of node 1's resize handles.
fn on_node_1_resize_handle(init: &InitState, at: glam::Vec2) -> bool {
    let surface = init.renderer.canvas_to_screen(at);
    let canvas = init.renderer.screen_to_canvas(surface.x, surface.y);
    let tolerance = super::HANDLE_HIT_TOLERANCE_PX * init.renderer.canvas_per_pixel();
    super::event_mouse_click::resize_handle_press(
        init.document.as_ref(),
        &init.interaction_mode,
        canvas,
        tolerance,
    )
    .node
    .is_some()
}

/// The preconditions of a handle press paired with a body press beside
/// it: `corner` is on node 1's resize handle; both presses land on node
/// 1's body, so the two share a `ClickHit` and only the handle press
/// can keep them from pairing into a double-click; `beside` is off the
/// handle.
fn assert_handle_and_body_beside_it(init: &mut InitState, corner: glam::Vec2, beside: glam::Vec2) {
    assert!(
        on_node_1_resize_handle(init, corner),
        "precondition: the corner is node 1's resize handle in Resize mode"
    );
    assert!(
        matches!(press_lands_on(init, corner), super::ClickHit::Node(ref id, None) if id == "1"),
        "precondition: a press on the corner lands on node 1's body too"
    );
    assert!(
        matches!(press_lands_on(init, beside), super::ClickHit::Node(ref id, None) if id == "1"),
        "precondition: the spot beside the corner is node 1's body"
    );
    assert!(
        !on_node_1_resize_handle(init, beside),
        "precondition: the spot beside the corner is off the handle"
    );
}

/// **A second press on a resize handle grabs it, inside the
/// double-click window** (#218). In `Resize` mode on node 1, a click on
/// its SE handle and then, at the same instant (the harness clock is
/// virtual and stands still), a press-drag from that handle: the node
/// grows, no editor opens, and the release commits one undo entry.
/// Before #218 the second press completed a double-click on the node
/// the handle's corner also hits, opened the text editor and never
/// grabbed the handle.
///
/// A handle press does not start a double-click either: a click on
/// the handle and then one on the node's body just beside it, inside
/// the double-click distance and outside the handle's tolerance, open
/// no editor.
///
/// The control is one more click on that same spot, still in `Resize`
/// mode: with the body click before it, it double-clicks into the text
/// editor. So the spot is double-clickable, the window is live, and a
/// double-click off the handles works as it did; only the handle press
/// before it kept the first pair from being one.
#[test]
fn test_a_second_press_on_a_resize_handle_grabs_it_inside_the_double_click_window() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    resize_node_1(init);
    zoom_onto_node_1(init);
    let (pos, size) = node_1_rect(init);
    // The grip: 3 px inside the SE corner, within the handle's 8 px
    // tolerance and on the node's body (the exact corner is empty
    // canvas to the click chain). Beside it: 11 px in and 2 px up from
    // the corner, so outside the handle's tolerance and within the
    // 16 px double-click distance of the grip.
    let cpp = init.renderer.canvas_per_pixel();
    let corner = pos + size - glam::Vec2::splat(3.0) * cpp;
    let beside = pos + size - glam::Vec2::new(11.0, 2.0) * cpp;
    assert_handle_and_body_beside_it(init, corner, beside);
    let undo_before = init.document.as_ref().expect("document").undo_stack.len();

    left_click_at(init, corner);
    left_drag_from(init, corner);
    let (_, resized) = node_1_rect(init);
    assert!(
        resized.x > size.x && resized.y > size.y,
        "the second press on the SE handle must grab it and grow the node: {size:?} -> {resized:?}"
    );
    assert_eq!(
        init.text_edit_state.node_id(),
        None,
        "the second press on a handle must not complete a double-click into the text editor"
    );
    assert!(
        matches!(init.drag_state, super::DragState::None),
        "the release ended the drag"
    );
    assert_eq!(
        init.document.as_ref().expect("document").undo_stack.len(),
        undo_before + 1,
        "the release committed the resize with one undo entry of its own"
    );

    let (pos, size) = node_1_rect(init);
    let corner = pos + size - glam::Vec2::splat(3.0) * cpp;
    let beside = pos + size - glam::Vec2::new(11.0, 2.0) * cpp;
    assert_handle_and_body_beside_it(init, corner, beside);
    left_click_at(init, corner);
    left_click_at(init, beside);
    assert_eq!(
        init.text_edit_state.node_id(),
        None,
        "a handle press must not start a double-click that a press beside it completes"
    );

    left_click_at(init, beside);
    assert_eq!(
        init.text_edit_state.node_id(),
        Some("1"),
        "control: a second click on the same body spot double-clicks into node 1's text editor"
    );
}

/// **A finger's tap and then drag on a resize handle resizes** (#218):
/// since #160 a finger on a handle is the left button, so the tap is a
/// click and the drag a second press inside the double-click window,
/// through the real `dispatch_touch_event`. Before #218 the drag
/// opened the text editor and resized nothing, exactly as the mouse
/// did; the mouse test above holds the double-click control.
#[test]
fn test_a_finger_tapping_then_dragging_a_resize_handle_resizes() {
    use crate::application::platform::input::TouchPhase;
    use crate::application::platform::window::PhysicalPosition;

    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    resize_node_1(init);
    zoom_onto_node_1(init);
    let (pos, size) = node_1_rect(init);
    let se = init.renderer.canvas_to_screen(pos + size);
    assert!(
        on_node_1_resize_handle(init, pos + size),
        "precondition: node 1's SE corner must be its resize handle in Resize mode"
    );
    let at = |i: f32| PhysicalPosition::new(f64::from(se.x + i * 8.0), f64::from(se.y + i * 8.0));

    init.input_context()
        .dispatch_touch_event(TouchPhase::Started, 1, at(0.0));
    init.input_context()
        .dispatch_touch_event(TouchPhase::Ended, 1, at(0.0));
    init.input_context().drain_frame(Beat::Loop);
    init.input_context()
        .dispatch_touch_event(TouchPhase::Started, 2, at(0.0));
    for i in 1..=5 {
        init.input_context()
            .dispatch_touch_event(TouchPhase::Moved, 2, at(i as f32));
        init.input_context().drain_frame(Beat::Loop);
    }
    init.input_context()
        .dispatch_touch_event(TouchPhase::Ended, 2, at(5.0));
    init.input_context().drain_frame(Beat::Loop);

    let (_, resized) = node_1_rect(init);
    assert!(
        resized.x > size.x && resized.y > size.y,
        "a finger's tap and drag on the SE handle must grow the node: {size:?} -> {resized:?}"
    );
    assert_eq!(
        init.text_edit_state.node_id(),
        None,
        "the drag must not complete a double-click into the text editor"
    );
}

/// The grabs the pending press recorded, by the names `query.mode`
/// reports, in the order the drag resolves them.
fn pending_grabs(init: &InitState) -> Vec<&'static str> {
    match &init.drag_state {
        super::DragState::Pending(press) => press.grabs().map(super::Grab::word).collect(),
        _ => panic!("expected a pending left press"),
    }
}

/// The edge `from` → `to` as the document holds it, serialized.
fn edge_json(init: &InitState, from: &str, to: &str) -> Value {
    let doc = init.document.as_ref().expect("document");
    let edge = doc
        .mindmap
        .edges
        .iter()
        .find(|e| e.from_id == from && e.to_id == to)
        .expect("the fixture carries the edge");
    serde_json::to_value(edge).expect("an edge serializes")
}

/// The grab handles of the edge `from` → `to`, by kind and canvas
/// position, as the press's hit-test builds them.
fn edge_handles(
    init: &InitState,
    from: &str,
    to: &str,
) -> Vec<(baumhard::mindmap::tree_builder::EdgeHandleKind, glam::Vec2)> {
    use baumhard::mindmap::{connection, tree_builder};
    let doc = init.document.as_ref().expect("document");
    let edge = doc
        .mindmap
        .edges
        .iter()
        .find(|e| e.from_id == from && e.to_id == to)
        .expect("the fixture carries the edge");
    let (f, t) = (&doc.mindmap.nodes[&edge.from_id], &doc.mindmap.nodes[&edge.to_id]);
    let path = connection::build_connection_path(
        f.pos_vec2(),
        f.size_vec2(),
        &edge.anchor_from,
        t.pos_vec2(),
        t.size_vec2(),
        &edge.anchor_to,
        &edge.control_points,
    );
    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
    tree_builder::build_edge_handles(
        edge,
        &key,
        &path,
        f.pos_vec2(),
        f.size_vec2(),
        t.pos_vec2(),
        t.size_vec2(),
    )
    .into_iter()
    .map(|h| (h.kind, glam::Vec2::new(h.position.0, h.position.1)))
    .collect()
}

/// **In `Resize` mode a resize handle outranks a label lying over it**
/// (#218). At the fixture's opening zoom the label of the edge 1 →
/// 1.2 covers node 1's SE handle, and a press there records both
/// grabs (asserted first, so the test cannot pass on a press that
/// grabbed only one). In `Resize` mode the drag resizes node 1 and
/// leaves the edge, its label included, as it was.
///
/// The control is the same drag in `Default` mode, where no resize
/// handle is captured: the press records the label alone, the drag
/// moves the label, and node 1 keeps its size. So the label is still
/// grabbable, and outside the mode nothing changed.
#[test]
fn test_in_resize_mode_a_resize_handle_outranks_a_label_lying_over_it() {
    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    resize_node_1(init);
    let (pos, size) = node_1_rect(init);
    let corner = pos + size;
    let edge_before = edge_json(init, "1", "1.2");

    left_press_at(init, corner);
    assert_eq!(
        pending_grabs(init),
        vec!["node_resize_handle", "edge_label"],
        "precondition: the press on node 1's SE handle records the label lying over it too, \
         and the handle first"
    );
    left_drag_on(init, corner);
    let (_, resized) = node_1_rect(init);
    assert!(
        resized.x > size.x && resized.y > size.y,
        "in Resize mode the drag must take the handle and grow the node: {size:?} -> {resized:?}"
    );
    assert_eq!(
        edge_json(init, "1", "1.2"),
        edge_before,
        "the label under the handle must stay where it was"
    );

    // Back to the node's own size, so the control presses the same
    // corner under the same label.
    let (reply, _) = act(init, json!({"id": 2, "cmd": "act.action", "action": "Undo"}));
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert_eq!(node_1_rect(init), (pos, size), "the undo restored node 1");
    let (reply, _) = act(init, json!({"id": 3, "cmd": "act.action", "action": "ExitMode"}));
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert!(matches!(init.interaction_mode, InteractionMode::Default));
    left_press_at(init, corner);
    assert_eq!(
        pending_grabs(init),
        vec!["edge_label"],
        "control: outside Resize mode the same press grabs the label alone"
    );
    left_drag_on(init, corner);
    assert_ne!(
        edge_json(init, "1", "1.2"),
        edge_before,
        "control: outside Resize mode the drag moves the label"
    );
    assert_eq!(node_1_rect(init).1, size, "control: and resizes nothing");
}

/// **A second press on a selected edge's bare handle grabs it, inside
/// the double-click window** (#218). With the edge 1 → 1.2 selected, a
/// click on its first control point and then, at the same instant, a
/// drag from it: the control point moves and no editor opens. Before
/// #218 the second press completed a double-click on the empty canvas
/// under the handle and never grabbed it.
///
/// The control is the other half of the rule, "what a press would drag
/// is what it is": on the edge 1.2 → 1.2.0, selected, whose label lies
/// over its midpoint handle, the press records the label first, so two
/// clicks there still double-click into the label's editor. A rule
/// that took every press with a handle grab out of double-click
/// detection, whatever the grab's rank, fails it.
#[test]
fn test_a_selected_edges_bare_handle_grabs_and_its_label_still_double_clicks() {
    use baumhard::mindmap::tree_builder::EdgeHandleKind;

    use crate::application::document::EdgeRef;

    let Some(mut app) = headless_app(&map_path("testament.mindmap.json")) else {
        return;
    };
    let init = &mut app.init;
    let editors_closed =
        |init: &InitState| init.text_edit_state.node_id().is_none() && !init.single_line_edit_state.is_open();

    init.document.as_mut().expect("document").selection =
        SelectionState::Edge(EdgeRef::new("1", "1.2", "parent_child"));
    let (_, point) = edge_handles(init, "1", "1.2")
        .into_iter()
        .find(|(kind, _)| *kind == EdgeHandleKind::ControlPoint(0))
        .expect("the edge 1 -> 1.2 is curved");
    assert!(
        matches!(press_lands_on(init, point), super::ClickHit::Empty),
        "precondition: the control point lies on empty canvas, so it is a bare handle"
    );
    let edge_before = edge_json(init, "1", "1.2");
    left_click_at(init, point);
    assert_eq!(
        init.document.as_ref().expect("document").selection,
        SelectionState::Edge(EdgeRef::new("1", "1.2", "parent_child")),
        "precondition: a click on the bare control point keeps its edge selected, so the next press can grab it"
    );
    left_drag_from(init, point);
    assert_ne!(
        edge_json(init, "1", "1.2"),
        edge_before,
        "the second press on the control point must grab it and move it"
    );
    assert!(editors_closed(init), "and must open no editor");

    init.document.as_mut().expect("document").selection =
        SelectionState::Edge(EdgeRef::new("1.2", "1.2.0", "parent_child"));
    let (_, midpoint) = edge_handles(init, "1.2", "1.2.0")
        .into_iter()
        .find(|(kind, _)| *kind == EdgeHandleKind::Midpoint)
        .expect("the edge 1.2 -> 1.2.0 is straight");
    left_press_at(init, midpoint);
    assert_eq!(
        pending_grabs(init),
        vec!["edge_label", "edge_handle"],
        "precondition: the press on the label records the midpoint handle under it, after the label"
    );
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_button", "button": "left", "state": "release"}),
    );
    left_click_at(init, midpoint);
    assert!(
        init.single_line_edit_state.is_open(),
        "control: a double-click on the label over the midpoint handle opens the label's editor"
    );
}
