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
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
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
