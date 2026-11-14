// SPDX-License-Identifier: MPL-2.0

//! The GPU-backed tests of the whole app (TEST_CONVENTIONS §T8): the
//! real app, headless, on this machine's adapter, driven through the
//! funnels an agent or a user drives it through — the IPC `act.*` and
//! `input.*` commands — with every assertion on state: the document's
//! bytes, its history, what the projector put in the glyph layers, what
//! the layer renderer applied, the redraw ledger. Never pixels (§T10).
//!
//! Each test begins with [`headless_app`] and returns when it answers
//! `None`: no adapter here, the note has been printed, and the test
//! passes having proved nothing — which CI forbids by setting
//! `MANDALA_REQUIRE_GPU` (`gpu_harness`).
//!
//! What these pin that the renderer-free suite could not: that the arms
//! behind `&mut Renderer` run end to end on the document → projector →
//! glyph layers → layer renderer pipeline — `Undo` and `Redo` through
//! the view's sync, a pan that must apply nothing, the console's
//! document swap through its side-effect drain, a click aimed by node
//! id, and a frame drawn on the adapter and read back by
//! `capture.screenshot`. The window's own frame is not among them:
//! under `Shell::Headless` there is no surface to present to, so
//! `draw_frame` keeps the redraw ledger and draws nothing
//! (`Sink::presents`); the one route a headless app draws through is
//! the capture engine.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::gpu_harness::{headless_app, HARNESS_SIZE};
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use super::InteractionMode;
use crate::application::capture::png;
use crate::application::document::selection::every_node;
use crate::application::document::MindMapDocument;
use crate::application::ipc::run_attached;
use crate::application::view::{inspect, MapView};
use baumhard::arena::NodeId;
use baumhard::document::format;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::projection::Layer;

fn map_path(name: &str) -> String {
    format!("{}/maps/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The app's document wrapper; the harness always builds one.
fn document(init: &InitState) -> &MindMapDocument {
    init.document
        .as_ref()
        .expect("the harness always builds a document")
}

/// The app's view of the document; the harness always loads one.
fn view(init: &InitState) -> &MapView {
    init.view.as_ref().expect("the harness always loads a view")
}

/// The bytes a save would write: the one reading of the whole document
/// that one app before and after an undo can be compared on without
/// naming fields.
fn document_bytes(init: &InitState) -> String {
    format::save(document(init).doc()).expect("the document saves")
}

/// Run one `act.*` / `input.*` / `capture.*` request against `init`,
/// through the real decoder, executor and wire encoding. Returns the
/// reply and the repaint flag.
fn act(init: &mut InitState, request: Value) -> (Value, bool) {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init))
}

/// `act.action <action>`'s reply.
fn action(init: &mut InitState, action: &str) -> Value {
    act(init, json!({"id": 1, "cmd": "act.action", "action": action})).0
}

/// The loop's input-driven half of one frame: the drain every event-loop
/// iteration runs before its redraw, which syncs the view — projector,
/// overlay re-stamp, layer renderer — with whatever the document did.
fn frame(init: &mut InitState) -> bool {
    init.input_context().drain_frame(Beat::Loop)
}

/// How many times the layer renderer has applied journals to its state.
fn applies(init: &InitState) -> u64 {
    init.renderer.layer_renderer().log().applies
}

/// The glyphs the projector drew for `node` in the app's view.
fn glyphs_of(init: &InitState, node: NodeId) -> Vec<(Layer, NodeId)> {
    view(init).projector().glyphs_of(node)
}

/// The live element count of every glyph layer, in `Layer::ALL` order:
/// a reading two views can be compared on without naming elements.
fn layer_counts(view: &MapView) -> Vec<usize> {
    Layer::ALL
        .iter()
        .map(|&layer| view.layers().tree(layer).arena().live_count())
        .collect()
}

/// The one node of `doc` that `before` did not hold.
fn the_new_node(doc: &Document, before: &[NodeId]) -> NodeId {
    let added: Vec<NodeId> = every_node(doc).filter(|id| !before.contains(id)).collect();
    assert_eq!(added.len(), 1, "exactly one node was added: {added:?}");
    added[0]
}

/// Removes a file when dropped, so a test that writes one leaves
/// nothing behind whichever assertion stops it. Shared with the
/// sibling GPU test modules.
pub(super) struct TempFile(std::path::PathBuf);

impl TempFile {
    /// A fresh path under the temp dir, `mandala-tests-gpu-<stem>-…
    /// .<ext>`, unique to this process and test thread; whatever was
    /// there is removed first.
    pub(super) fn new(stem: &str, ext: &str) -> Self {
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

    pub(super) fn as_str(&self) -> &str {
        self.0.to_str().expect("a UTF-8 temp path")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// **Undo and redo, through `act.action`, reach the document and the
/// glyph layers.** An edit made through the document's writer, then
/// `Undo`, lands on the baseline's bytes and takes the edit's glyphs out
/// of the layers at the next frame; `Redo` lands on the edit's bytes and
/// puts them back.
///
/// Controls, before anything is edited: `Undo` over an empty history is
/// unhandled and changes nothing, and `DeleteSelection` with nothing
/// selected is unhandled and changes nothing either, so a funnel that
/// edited with no target, or one that reported every Action handled,
/// fails here.
/// After the edit, the bytes differ from the baseline and the new node
/// draws, so an undo that did nothing cannot pass by agreeing with a
/// baseline it never left.
#[test]
fn test_undo_and_redo_through_act_action_reach_the_document_and_the_layers() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let baseline = document_bytes(init);

    let reply = action(init, "Undo");
    assert_eq!(
        reply["result"]["outcome"], "unhandled",
        "nothing to undo: {reply}"
    );
    let reply = action(init, "DeleteSelection");
    assert_eq!(
        reply["result"]["outcome"], "unhandled",
        "a delete of nothing is unhandled: {reply}"
    );
    assert_eq!(document_bytes(init), baseline, "the controls changed nothing");
    assert!(!document(init).is_dirty(), "and dirtied nothing");

    let before: Vec<NodeId> = every_node(document(init).doc()).collect();
    init.document.as_mut().expect("document").create_root_for_test();
    let node = the_new_node(document(init).doc(), &before);
    let ident: NodeIdent = inspect::ident_of(document(init).doc(), node).expect("a node has an ident");
    frame(init);
    let edited = document_bytes(init);
    assert_ne!(edited, baseline, "precondition: the edit changed the bytes");
    assert!(
        !glyphs_of(init, node).is_empty(),
        "precondition: the new node draws after the frame's sync"
    );

    let reply = action(init, "Undo");
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert_eq!(
        document_bytes(init),
        baseline,
        "the undo restored the baseline's bytes"
    );
    frame(init);
    assert!(
        document(init).doc().node_by_ident(ident).is_none(),
        "the undone node is gone from the document"
    );
    assert!(
        glyphs_of(init, node).is_empty(),
        "and its glyphs are gone from the layers"
    );

    let reply = action(init, "Redo");
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert_eq!(document_bytes(init), edited, "the redo restored the edit's bytes");
    frame(init);
    let redone = document(init)
        .doc()
        .node_by_ident(ident)
        .expect("the redone node is back, under its ident");
    assert!(
        !glyphs_of(init, redone).is_empty(),
        "and it draws again after the frame's sync"
    );
    let reply = action(init, "Redo");
    assert_eq!(
        reply["result"]["outcome"], "unhandled",
        "nothing left to redo: {reply}"
    );
}

/// **A pan applies nothing.** `act.action PanCameraEast` moves the
/// camera, and the frame after it hands the layer renderer no journal:
/// the glyph layers are in canvas space, so a camera move is a uniform
/// and nothing else.
///
/// The negative control is an edit through the same frame: it applies
/// exactly once, so an apply counter that never moved — or a frame that
/// never synced — fails here rather than passing the pan vacuously.
#[test]
fn test_a_pan_through_act_action_applies_nothing_to_the_layers() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let applied = applies(init);
    let position = init.renderer.camera().position;

    let reply = action(init, "PanCameraEast");
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    assert_ne!(
        init.renderer.camera().position,
        position,
        "precondition: the pan moved the camera"
    );
    frame(init);
    assert_eq!(applies(init), applied, "a pan applied render state");

    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    assert_eq!(
        applies(init),
        applied + 1,
        "the control: an edit applies once, through the same frame"
    );
}

/// The warning codes an IPC reply carries. Gated as its callers, the
/// `cfg(unix)` save-through-a-link tests, are.
#[cfg(unix)]
fn warning_codes(reply: &Value) -> Vec<String> {
    reply["warnings"]
        .as_array()
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(|w| w["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether some warning of `reply` carries `code` and a message holding
/// every one of `words`. Gated as its callers are.
#[cfg(unix)]
fn warns(reply: &Value, code: &str, words: &[&str]) -> bool {
    reply["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|w| {
            w["code"] == code
                && w["message"]
                    .as_str()
                    .is_some_and(|m| words.iter().all(|word| m.contains(word)))
        })
    })
}

/// **`SaveDocumentAs` through `act.action` to a link that leads to a
/// file that is not a map** is refused as `format/ipc.md` says: the
/// reply is `ok`, the outcome `unhandled`, with a `console_error`
/// warning naming the link and that it leads to another file (cut to
/// the log's excerpt width); nothing is written, and the binding and
/// the unsaved change stay.
///
/// Control: checking the extension on the name typed alone lets the
/// save through (`handled`), and `notamap.txt` holds the map.
#[cfg(unix)]
#[test]
fn test_an_ipc_save_document_as_through_a_link_to_a_file_that_is_not_a_map_is_unhandled() {
    // The reply carries the warnings a logger hands it.
    baumhard::util::test_logger::install();
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    let bound = document(init).file_path.clone();
    let dir = baumhard::util::test_temp::TempDir::new("gpu-ipc-save-as-link");
    let other = dir.join("notamap.txt");
    std::fs::write(&other, "notes").expect("the plain file is written");
    let link = dir.join("sneaky.mandala.json");
    std::os::unix::fs::symlink(&other, &link).expect("the link is made");
    let (reply, _) = act(
        init,
        json!({"id": 8, "cmd": "act.action", "action": {"SaveDocumentAs": link.to_str().expect("a UTF-8 path")}}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["outcome"], "unhandled", "{reply}");
    assert!(
        // The console's warning carries the line's failure cut to the log's
        // excerpt width, so the link's target may be cut off; the full
        // words are the `act.console` reply's (the test below this one).
        warns(&reply, "console_error", &["sneaky.mandala.json: links to `"]),
        "a console_error warning names the link and that it leads elsewhere: {reply} ({:?})",
        warning_codes(&reply)
    );
    assert_eq!(std::fs::read_to_string(&other).expect("reads"), "notes");
    assert_eq!(document(init).file_path, bound);
    assert!(document(init).is_dirty());
}

/// **`SaveDocument` on a map opened through a git-annex-style link** —
/// a `*.mandala.json` link to a key ending `.json` — opens it, takes an
/// edit, and then the bound save is refused as `format/ipc.md` says:
/// `outcome: "unhandled"` with a `save_failed` warning naming both paths;
/// the key is not written, and the binding and the unsaved change stay.
///
/// Control: checking the extension on the name typed alone lets the
/// save through (`handled`), and the key holds the edit.
#[cfg(unix)]
#[test]
fn test_an_ipc_save_document_of_a_map_opened_through_an_annex_link_is_unhandled() {
    // The reply carries the warnings a logger hands it.
    baumhard::util::test_logger::install();
    let dir = baumhard::util::test_temp::TempDir::new("gpu-ipc-save-annex");
    let key = dir.join("SHA256E-s1--0123abcd.json");
    std::fs::copy(map_path("theme_demo.mandala.json"), &key).expect("the key is written");
    let before = std::fs::read(&key).expect("reads");
    let link = dir.join("annexed.mandala.json");
    std::os::unix::fs::symlink(&key, &link).expect("the link is made");
    let link_str = link.to_str().expect("a UTF-8 path").to_string();
    let Some(mut app) = headless_app(&link_str) else {
        return;
    };
    let init = &mut app.init;
    assert_eq!(
        document(init).file_path.as_deref(),
        Some(link_str.as_str()),
        "precondition: the map opened through the link"
    );
    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    let reply = action(init, "SaveDocument");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["outcome"], "unhandled", "{reply}");
    assert!(
        warns(
            &reply,
            "save_failed",
            &[
                "annexed.mandala.json: links to `",
                "0123abcd.json`, which is not a map"
            ]
        ),
        "a save_failed warning names both paths: {reply} ({:?})",
        warning_codes(&reply)
    );
    assert_eq!(
        std::fs::read(&key).expect("reads"),
        before,
        "the key is not written"
    );
    assert_eq!(document(init).file_path.as_deref(), Some(link_str.as_str()));
    assert!(document(init).is_dirty());
}

/// **A save driven over IPC cannot write map bytes into a file that is
/// not a map** (amendment E1.1, second addendum): `save <path>` through
/// `act.console`, where the path is a link named like a map that leads
/// to `notamap.txt`, is refused naming both, and the file it leads to,
/// the document's binding and its unsaved change are all as they were.
///
/// Control: checking the extension on the name typed alone lets the
/// save through, and `notamap.txt` holds the map.
#[cfg(unix)]
#[test]
fn test_an_ipc_save_through_a_link_to_a_file_that_is_not_a_map_is_refused() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    assert!(
        document(init).is_dirty(),
        "precondition: the edit dirtied the map"
    );
    let bound = document(init).file_path.clone();
    let dir = baumhard::util::test_temp::TempDir::new("gpu-ipc-save-link");
    let other = dir.join("notamap.txt");
    std::fs::write(&other, "notes").expect("the plain file is written");
    let link = dir.join("sneaky.mandala.json");
    std::os::unix::fs::symlink(&other, &link).expect("the link is made");
    let line = format!("save {}", link.to_str().expect("a UTF-8 path"));
    let (reply, _) = act(init, json!({"id": 7, "cmd": "act.console", "line": line}));
    assert_eq!(reply["ok"], false, "the save is refused: {reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("sneaky.mandala.json: links to `")
                && m.contains("notamap.txt`, which is not a map")),
        "the refusal names both paths: {reply}"
    );
    assert_eq!(
        std::fs::read_to_string(&other).expect("reads"),
        "notes",
        "nothing was written through the link"
    );
    assert_eq!(document(init).file_path, bound, "the binding is as it was");
    assert!(document(init).is_dirty(), "the unsaved change is still unsaved");
}

/// **`act.console` through the whole side-effect drain.** The dirty
/// guard — the one thing between a typo'd `open` and lost work —
/// refuses `open` and `new` over an unsaved change, through the same
/// drain, leaving everything as it was; `save <path>` clears it; `open
/// <path>` then swaps the document and loads a view of the new map; and
/// `new` swaps in a blank map the same way.
///
/// The expected node counts and layers come from an independent load of
/// the same file — a renderer-free [`MapView::for_test`] of it — and from
/// [`MindMapDocument::new_blank`] directly, never from the app under test
/// (§T13's mirror rule). The precondition that the two fixtures differ
/// in node count is asserted, or a swap that did nothing would read as
/// one that worked; and the refusals are asserted against the document's
/// bytes, not only the reply.
#[test]
fn test_act_console_open_and_new_replace_the_document_through_the_side_effect_drain() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    let other = map_path("theme_demo.mandala.json");
    let expected = MindMapDocument::load(&other, 50).expect("the fixture loads");
    let expected_nodes = every_node(expected.doc()).count();
    let testament_nodes = every_node(document(init).doc()).count();
    assert_ne!(
        expected_nodes, testament_nodes,
        "the fixtures must differ in node count or this test proves nothing"
    );
    let generation = document(init).generation();

    // The dirty guard, through the drain: an edit, then a swap attempted
    // out from under the unsaved change.
    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    assert!(
        document(init).is_dirty(),
        "precondition: the edit dirtied the map"
    );
    let dirty_bytes = document_bytes(init);
    let dirty_layers = layer_counts(view(init));
    for (id, line) in [(2, format!("open {other}")), (3, "new".to_string())] {
        let (reply, repaint) = act(init, json!({"id": id, "cmd": "act.console", "line": line}));
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
            document_bytes(init),
            dirty_bytes,
            "the refused `{line}` left the document as it was"
        );
        assert_eq!(document(init).generation(), generation, "and kept it");
        assert_eq!(layer_counts(view(init)), dirty_layers, "and its view");
    }

    // `save <path>` clears the guard and binds the path.
    let saved = TempFile::new("saved", "mandala.json");
    let (reply, _) = act(
        init,
        json!({"id": 4, "cmd": "act.console", "line": format!("save {}", saved.as_str())}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!document(init).is_dirty(), "saved: clean");
    assert_eq!(
        document(init).file_path.as_deref(),
        Some(saved.as_str()),
        "saved: rebound to the new path"
    );
    let on_disk = MindMapDocument::load(saved.as_str(), 50).expect("the saved file loads");
    assert_eq!(
        every_node(on_disk.doc()).count(),
        testament_nodes + 1,
        "the file on disk holds the map and the added node"
    );

    // `open <path>`: the swap and the new view.
    let (reply, repaint) = act(
        init,
        json!({"id": 5, "cmd": "act.console", "line": format!("open {other}")}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(repaint, "a document swap repaints");
    let opened_generation = document(init).generation();
    assert_ne!(opened_generation, generation, "the opened map is a new document");
    assert_eq!(
        every_node(document(init).doc()).count(),
        expected_nodes,
        "the opened map is installed"
    );
    assert_eq!(
        document_bytes(init),
        format::save(expected.doc()).expect("saves"),
        "byte for byte"
    );
    assert_eq!(
        document(init).file_path.as_deref(),
        Some(other.as_str()),
        "and bound to its path"
    );
    assert!(!document(init).is_dirty(), "a freshly opened map is clean");
    assert_eq!(
        document(init).selection.first(),
        None,
        "the old selection did not survive the swap"
    );
    assert_eq!(
        layer_counts(view(init)),
        layer_counts(&MapView::for_test(&expected)),
        "the view holds the opened map's layers, as an independent projection of it does"
    );
    assert_ne!(
        layer_counts(view(init)),
        dirty_layers,
        "control: the old map's layers are not the opened map's"
    );
    assert_eq!(init.interaction_mode, InteractionMode::Default);

    // `new`: a blank map through the same swap.
    let (reply, repaint) = act(init, json!({"id": 6, "cmd": "act.console", "line": "new"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(repaint);
    let blank = MindMapDocument::new_blank(None, 50);
    assert_ne!(
        document(init).generation(),
        opened_generation,
        "a new document again"
    );
    assert_eq!(
        every_node(document(init).doc()).count(),
        every_node(blank.doc()).count(),
        "a blank map is installed"
    );
    assert_eq!(document(init).file_path, None, "`new` with no path binds none");
    assert!(!document(init).is_dirty());
    assert_eq!(
        layer_counts(view(init)),
        layer_counts(&MapView::for_test(&blank)),
        "the view holds the blank map's layers"
    );
}

/// **A zoom through `act.action`, then a frame drawn on the adapter.**
/// `act.action ZoomIn` moves the camera; then `capture.screenshot` draws
/// the window's view through the layer renderer — `prepare_frame`,
/// `encode_frame`, the readback — and writes the PNG and its geometry
/// sidecar, whose nodes are named by their integer ids. The headless
/// app's own `draw_frame` is asked for too, and asserted for what it is:
/// bookkeeping. With no surface to present to it marks the redraw ledger
/// current and draws nothing, which is the "rendered" half of
/// `clock.wait until=settled` on a headless launch.
///
/// Assertions are on state, never pixels (§T10): the reply, the PNG's
/// dimensions, the sidecar's nodes against the document, the live
/// camera the capture promised to leave alone. Controls: the zoom before
/// the action; the PNG absent before the capture; and the ledger owing a
/// frame before `draw_frame` and none after.
#[test]
fn test_a_zoom_through_act_action_and_a_capture_draws_the_frame() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let zoom_before = init.renderer.camera_zoom();
    let reply = action(init, "ZoomIn");
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    let zoomed = init.renderer.camera_zoom();
    assert!(
        zoomed > zoom_before,
        "ZoomIn moved the camera: {zoomed} from {zoom_before}"
    );
    frame(init);

    let shot = TempFile::new("zoomed", "png");
    let sidecar = shot.sibling(".geometry.json");
    assert!(
        !Path::new(shot.as_str()).exists(),
        "control: no PNG before the capture"
    );
    let reply = capture_screenshot(init, shot.as_str());
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
    let shown: Vec<u64> = sidecar_json["nodes"]
        .as_array()
        .expect("the sidecar lists nodes")
        .iter()
        .map(|node| {
            node["id"]
                .as_u64()
                .expect("every entry names a node by its integer id")
        })
        .collect();
    assert!(
        !shown.is_empty(),
        "the capture framed the map: at least one node is in view"
    );
    let doc = document(init);
    for id in &shown {
        let ident = NodeIdent(u32::try_from(*id).expect("a node id is a u32"));
        assert!(
            doc.doc().node_by_ident(ident).is_some(),
            "sidecar node {id} is a node of the document"
        );
    }
    assert_eq!(
        sidecar_json["document_revision"].as_u64(),
        Some(doc.revision().0),
        "the sidecar is tied to the revision it was drawn from"
    );
    assert_eq!(
        init.renderer.camera_zoom(),
        zoomed,
        "a capture draws through a camera of its own and leaves the live one alone"
    );

    // The headless app's own frame: ledger bookkeeping, no drawing.
    init.request_redraw();
    assert!(
        init.settling().redraw_owed,
        "control: a frame is owed before draw_frame runs"
    );
    init.draw_frame();
    assert!(
        !init.settling().redraw_owed,
        "draw_frame settled the ledger — with no surface to present to, that is all it does"
    );
}

/// `capture.screenshot path=<path>` through the real decoder and
/// executor, retried while the engine reports the one transient skip
/// — `font_system_contended`, the font lock held by another test's
/// shaping in this process — the way the heartbeat retries a skipped
/// frame on the next beat. Any other outcome is returned as is.
///
/// The retry is bounded by **time**, not by a count of attempts: how
/// long another test holds the lock is one shaping pass on whatever
/// the runner is doing, while a yield returns in microseconds, so a
/// fixed number of yields ran out under CI load while the lock was
/// still legitimately held. A lock that is never released — the
/// failure this guards against — still fails, at the deadline.
fn capture_screenshot(init: &mut InitState, path: &str) -> Value {
    /// How long to wait out a held font lock: the lock's own budget, so
    /// the helper waits exactly as long as any blocking acquire treats a
    /// hold as legitimate rather than a deadlock.
    const PATIENCE: Duration = baumhard::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT;
    /// The pause between tries, so a wait spends its time asleep rather
    /// than spinning against the holder.
    const BACKOFF: Duration = Duration::from_millis(1);
    let request = json!({"id": 2, "cmd": "capture.screenshot", "path": path});
    let deadline = Instant::now() + PATIENCE;
    let mut attempts = 0_usize;
    loop {
        let reply = act(init, request.clone()).0;
        attempts += 1;
        if reply["result"]["skipped"] != "font_system_contended" {
            return reply;
        }
        if Instant::now() >= deadline {
            panic!(
                "the font system stayed contended for {PATIENCE:?} ({attempts} capture attempts): {reply}"
            );
        }
        std::thread::sleep(BACKOFF);
    }
}

/// **`input.click_node` aims by integer node id and the click selects
/// the node**, through the injected-input queue, the frame's drain, the
/// projector's hit test and the selection.
///
/// The node is the first of the document's that the live camera shows
/// whole-window visible, chosen from the view's own visibility reading,
/// so the test aims at what the window draws. Controls: the selection is
/// empty before the click, and a click aimed at an id no node holds is
/// refused by name and selects nothing — so a route that selected
/// whatever it was told, or nothing at all, fails.
#[test]
fn test_input_click_node_aims_by_integer_id_and_selects_the_node() {
    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let camera = *init.renderer.camera();
    let node = {
        let doc = document(init).doc();
        let view = view(init);
        every_node(doc)
            .find(|&id| {
                inspect::element_visibility(doc, view.projector(), view.layers(), id, &camera)
                    == Some("visible")
            })
            .expect("precondition: the fitted camera shows a node")
    };
    let ident = inspect::ident_of(document(init).doc(), node).expect("a node has an ident");
    assert_eq!(
        document(init).selection.first(),
        None,
        "control: nothing is selected at load"
    );

    let (reply, _) = act(
        init,
        json!({"id": 7, "cmd": "input.click_node", "node_id": u32::MAX}),
    );
    assert_eq!(reply["ok"], false, "an id no node holds is refused: {reply}");
    frame(init);
    assert_eq!(document(init).selection.first(), None, "and selects nothing");

    let (reply, _) = act(
        init,
        json!({"id": 8, "cmd": "input.click_node", "node_id": ident.0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(
        reply["result"]["applied"], true,
        "a burst into an empty queue lands at once: {reply}"
    );
    frame(init);
    let selection: Vec<NodeId> = document(init).selection.iter().collect();
    assert_eq!(
        selection,
        vec![node],
        "the click selected exactly the node it aimed at"
    );
}

/// **A refused change says so on the screen** (D28): with an
/// interactive transaction open, an `apply` is refused, and the next
/// frame's sync hands the document's status to the renderer's status
/// line — what the user sees, read from the renderer
/// (`mode_status_text`), not from the document. An undo cancels the
/// transaction and clears the status, and the next frame clears the
/// line.
///
/// Controls: before the refusal the line is empty; the refusal alone
/// draws nothing until a frame syncs (so the line is the sync's work,
/// not the document's); and after the undo the frame clears it, so a
/// line that stuck once shown fails.
#[test]
fn test_a_refused_change_is_drawn_on_the_status_line() {
    use crate::application::document::TRANSACTION_OPEN_STATUS;
    use baumhard::document::mutation::compose;
    use baumhard::document::writer::error::ApplyError;
    use baumhard::document::writer::transaction::TransactionKind;
    use baumhard::document::writer::{ChangeSource, EditRequest};

    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    assert_eq!(
        init.renderer.mode_status_text(),
        None,
        "control: nothing refused, nothing shown"
    );

    let doc = init.document.as_mut().expect("document");
    assert!(doc.begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        crate::application::document::Interaction::Test
    ));
    let req = EditRequest {
        steps: compose::create_root(doc.doc(), glam::Vec2::new(10.0, 10.0)).expect("a root composes"),
        label: "test: create root".to_string(),
    };
    assert_eq!(
        doc.apply(req, ChangeSource::Action),
        Err(ApplyError::TransactionOpen)
    );
    assert_eq!(
        init.renderer.mode_status_text(),
        None,
        "control: the refusal alone draws nothing until a frame syncs"
    );
    assert!(frame(init), "the status line owes a redraw");
    assert_eq!(init.renderer.mode_status_text(), Some(TRANSACTION_OPEN_STATUS));

    init.document.as_mut().expect("document").undo();
    frame(init);
    assert_eq!(
        init.renderer.mode_status_text(),
        None,
        "the undo closed the transaction and the frame cleared the line"
    );
}

/// **The change feed's second consumer, the IPC event stream, through
/// the real loop** (#244 amendment 3): the frame's sync drains the
/// document's records once, keeps them for the iteration
/// (`MapView::pending_feed`), and an app-driven `document_changed`
/// names the source of the last of them. A Ctrl+Z pressed on the
/// keyboard — resolved by the keybinds and dispatched by the native
/// keyboard handler — reports `"undo"`; the iteration's last step
/// (`clear_feed`) leaves the feed empty.
///
/// The steps are the iteration's, in its order (`pump::iterate`): the
/// frame's drain, the server's observation (`Session::observe` with
/// `Driver::App`, as `IpcServer::observe` makes it), the feed cleared.
///
/// Controls: the same frame after a plain edit reports `"action"`, so
/// the source is read from the records rather than restated; an
/// observation with no records falls back to `"input"`, which is what a
/// feed that never reached the stream would read; and the feed is
/// non-empty before the clear.
#[test]
fn test_an_app_driven_change_reports_its_source_through_the_change_feed() {
    use super::event_keyboard::handle_keyboard_input;
    use super::pump::{Iteration, Running};
    use crate::application::ipc::events::{Class, Classes, Driver, Session};
    use crate::application::platform::input::{Key, Modifiers};

    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let mut session = Session::default();
    let mut classes = Classes::default();
    classes.insert(Class::DocumentChanged);
    session.subscribe(classes, true);
    session.baseline(&init.observed());

    let source_of = |session: &mut Session| -> Option<String> {
        let queued = session.pop_pending()?;
        let event = serde_json::to_value(&queued.event).expect("an event serializes");
        event["data"]["source"].as_str().map(str::to_string)
    };
    let clear = |init: &mut InitState| {
        let (mut ipc, mut one_shot) = (None, None);
        Running {
            init,
            ipc: &mut ipc,
            one_shot: &mut one_shot,
        }
        .clear_feed();
    };

    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    assert!(
        !view(init).pending_feed().is_empty(),
        "the frame kept the records"
    );
    session.observe(&init.observed(), Driver::App);
    assert_eq!(
        source_of(&mut session).as_deref(),
        Some("action"),
        "control: a plain edit"
    );
    clear(init);
    assert!(
        view(init).pending_feed().is_empty(),
        "the iteration's end clears the feed"
    );

    init.modifiers = Modifiers::CONTROL;
    handle_keyboard_input(Key::Character("z".into()), &mut init.input_context());
    init.modifiers = Modifiers::empty();
    frame(init);
    session.observe(&init.observed(), Driver::App);
    assert_eq!(
        source_of(&mut session).as_deref(),
        Some("undo"),
        "Ctrl+Z on the keyboard"
    );
    clear(init);
    assert!(view(init).pending_feed().is_empty());

    // What a feed that never reached the stream would read.
    init.document.as_mut().expect("document").create_root_for_test();
    frame(init);
    clear(init);
    session.observe(&init.observed(), Driver::App);
    assert_eq!(
        source_of(&mut session).as_deref(),
        Some("input"),
        "control: with no records the source falls back"
    );
}

/// **The rubber band is drawn on the chrome layer, through its journaled
/// write path** (design §12.1; `MapView::apply_chrome`): a band's frame
/// applies one chrome journal that names exactly the band's four runs,
/// and a move of the band touches those four and nothing else in the
/// chrome; a pan applies nothing, to the glyph layers or to the chrome.
///
/// Controls: the same rectangle on the next frame applies nothing (the
/// band is not redrawn per frame); the band's runs are hidden before it
/// and after it ends and shown while it is out; and the move changes
/// the runs' drawn origin, so the apply is of a real change.
#[test]
fn test_the_rubber_band_is_drawn_on_the_chrome_layer() {
    use super::DragState;
    use glam::Vec2;

    let Some(mut app) = headless_app(&map_path("testament.mandala.json")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let chrome_applies = |init: &InitState| init.renderer.layer_renderer().log().chrome_applies;
    let band_opacity = |init: &InitState| -> Vec<f32> {
        let view = view(init);
        view.band_runs()
            .iter()
            .map(|&id| {
                view.chrome().arena()[id]
                    .get()
                    .glyph_run()
                    .expect("a band element is a run")
                    .opacity()
            })
            .collect()
    };
    let mut band_runs = view(init).band_runs().to_vec();
    band_runs.sort();
    assert_eq!(
        band_opacity(init),
        vec![0.0; 4],
        "control: the band is hidden before it starts"
    );

    let (layer_applies, chrome_before) = (applies(init), chrome_applies(init));
    let reply = action(init, "PanCameraEast");
    assert_eq!(reply["result"]["outcome"], "handled", "{reply}");
    frame(init);
    assert_eq!(
        applies(init),
        layer_applies,
        "a pan applies nothing to the glyph layers"
    );
    assert_eq!(chrome_applies(init), chrome_before, "nor to the chrome");

    init.drag_state = DragState::SelectingRect {
        start_canvas: Vec2::new(-400.0, -300.0),
        current_canvas: Vec2::new(200.0, 100.0),
    };
    frame(init);
    assert_eq!(
        chrome_applies(init),
        chrome_before + 1,
        "the band's first frame applies the chrome once"
    );
    let mut visited = init.renderer.layer_renderer().log().chrome_visited.clone();
    visited.sort();
    assert_eq!(visited, band_runs, "and names exactly the band's four runs");
    assert_eq!(band_opacity(init), vec![1.0; 4], "the band is shown");
    let origin = |init: &InitState| {
        let view = view(init);
        view.chrome().arena()[view.band_runs()[0]]
            .get()
            .glyph_run()
            .expect("a run")
            .origin()
    };
    let first = origin(init);

    frame(init);
    assert_eq!(
        chrome_applies(init),
        chrome_before + 1,
        "control: the same band applies nothing again"
    );

    init.drag_state = DragState::SelectingRect {
        start_canvas: Vec2::new(-250.0, -300.0),
        current_canvas: Vec2::new(200.0, 100.0),
    };
    frame(init);
    assert_eq!(chrome_applies(init), chrome_before + 2);
    let mut visited = init.renderer.layer_renderer().log().chrome_visited.clone();
    visited.sort();
    assert_eq!(visited, band_runs, "a band move touches only its own runs");
    assert_ne!(origin(init), first, "control: the move moved the band");

    init.drag_state = DragState::None;
    frame(init);
    assert_eq!(
        chrome_applies(init),
        chrome_before + 3,
        "the band's end is one more apply"
    );
    assert_eq!(band_opacity(init), vec![0.0; 4], "and hides it");
}
