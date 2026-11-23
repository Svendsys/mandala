// SPDX-License-Identifier: MPL-2.0

//! Live interaction on transactions through the real app (V1-C7, #248;
//! TEST_CONVENTIONS §T8): every drag that edits the map, the resize
//! mode's handles, the fast resize, the native touch handle and the
//! contextual color picker, driven by synthetic input — the IPC `input.*`
//! commands, which reach the handlers a winit event reaches — on the map
//! `dispatch/families/fixtures/live.mandala.json`:
//!
//! ```text
//! 1 "Root" ── portal ── 5 "Far" (two sections, min_size 160 × 80)
//! └─ 2 "Child", connector labeled "via"
//!    └─ 4 "Grand"
//! 3 "Other" ── line link to 2, one control point
//! ```
//!
//! What each drag is held to (the issue's list):
//!
//! - after its frames, the document is exactly one `apply` of the
//!   request its final total makes ([`assert_same_document`] over every
//!   element of the tree);
//! - its release makes **one** history entry, and an undo gives back the
//!   map's saved bytes;
//! - Escape gives them back with no entry.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::format;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;
use serde_json::{json, Value};

use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use super::{DragState, InteractionMode, ResizeTarget};
use crate::application::document::{Interaction, MindMapDocument, TRANSACTION_OPEN_STATUS};
use crate::application::ipc::run_attached;
use crate::application::view::handles::{Handle, HandleSet, PathHandle, ResizeSide};
use crate::application::view::inspect::children_of_kind;

/// The fixture.
fn fixture() -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/live.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn document(init: &InitState) -> &MindMapDocument {
    init.document
        .as_ref()
        .expect("the harness always builds a document")
}

fn document_mut(init: &mut InitState) -> &mut MindMapDocument {
    init.document
        .as_mut()
        .expect("the harness always builds a document")
}

/// What a save writes: the committed map.
fn bytes(init: &InitState) -> String {
    format::save(document(init).doc()).expect("the document saves")
}

/// How many entries an undo can reach.
fn entries(init: &InitState) -> usize {
    document(init).doc().history().count()
}

/// One frame: an injected sample, the drains — the one stage of a drag —
/// and the view's sync.
fn frame(init: &mut InitState) {
    init.input_context().drain_frame(Beat::Loop);
}

/// Run one IPC request through the real decoder and executor.
fn ipc(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// Send `request`, an `input.*` command, and hold that it reached the
/// handlers before its reply.
fn input(init: &mut InitState, request: Value) {
    let reply = ipc(init, request);
    assert_eq!(reply["result"]["applied"], true, "{reply}");
}

/// Move the pointer to `at` on the canvas, holding `mods`.
fn move_to(init: &mut InitState, at: Vec2, mods: &[&str]) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": at.x, "y": at.y, "space": "canvas", "mods": mods}),
    );
}

/// Press or release `button` at `at` on the canvas.
fn button(init: &mut InitState, button: &str, state: &str, at: Vec2, mods: &[&str]) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_button", "button": button, "state": state,
               "x": at.x, "y": at.y, "space": "canvas", "mods": mods}),
    );
}

/// Tap the key `key`.
fn tap(init: &mut InitState, key: &str) {
    input(init, json!({"id": 1, "cmd": "input.key", "key": key}));
}

/// `act.action` with `action`, and its outcome.
fn act(init: &mut InitState, action: Value) -> String {
    let reply = ipc(init, json!({"id": 1, "cmd": "act.action", "action": action}));
    reply["result"]["outcome"]
        .as_str()
        .unwrap_or_else(|| panic!("act.action answers: {reply}"))
        .to_string()
}

/// The node with integer id `ident`.
fn node(init: &InitState, ident: u32) -> NodeId {
    document(init)
        .doc()
        .node_by_ident(NodeIdent(ident))
        .unwrap_or_else(|| panic!("node {ident}"))
}

/// The `index`th element of `kind` directly under `parent`.
fn child(init: &InitState, parent: NodeId, kind: DocKind, index: usize) -> NodeId {
    children_of_kind(document(init).doc(), parent, kind)
        .nth(index)
        .unwrap_or_else(|| panic!("{kind:?} {index} under {parent}"))
}

/// The drawn rectangle of `node`.
fn rect(init: &InitState, node: NodeId) -> baumhard::flow::Rect {
    init.view
        .as_ref()
        .expect("a view")
        .projector()
        .node_rect(node)
        .expect("the node is drawn")
}

/// The position of `node`.
fn position(init: &InitState, node: NodeId) -> Vec2 {
    super::live::position_of(document(init).doc(), node).expect("a node")
}

/// Where `handle` is shown, failing when it is not.
fn handle_at(init: &InitState, handle: Handle) -> Vec2 {
    init.view
        .as_ref()
        .expect("a view")
        .handle_center(handle)
        .unwrap_or_else(|| panic!("{handle:?} is shown"))
}

/// The app, on the fixture; `None` (skipped) with no adapter.
fn app() -> Option<super::gpu_harness::GpuApp> {
    headless_app(&fixture())
}

/// **Every element of `actual` equals the same element of `expected`**:
/// the same tree, by arena id, element for element and child for child.
/// What "the document equals one `apply` of the request" means while a
/// stage stands, where a save would write the committed map instead.
fn assert_same_document(actual: &Document, expected: &Document, what: &str) {
    let a: Vec<NodeId> = actual.tree().descendants().collect();
    let b: Vec<NodeId> = expected.tree().descendants().collect();
    assert_eq!(a, b, "{what}: the trees hold the same elements in the same order");
    for id in a {
        assert_eq!(actual.get(id), expected.get(id), "{what}: element {id:?}");
        assert_eq!(
            actual.parent(id),
            expected.parent(id),
            "{what}: the parent of {id:?}"
        );
    }
}

/// The document a single `apply` of the running drag's request makes of
/// `before` — the map as it was at the press — for its total and pointer
/// now.
fn one_apply_of_the_drag(init: &InitState, before: &Document) -> Document {
    let live = init.drag_state.live().expect("a drag runs");
    let projector = init.view.as_ref().expect("a view").projector();
    let steps = live
        .gesture
        .steps(before, projector, live.total())
        .expect("the drag's request builds");
    let mut expected = before.clone();
    expected
        .apply(EditRequest::new(steps, "expected"), ChangeSource::Input)
        .expect("the request applies");
    expected
}

/// Hold the three rules of a drag that ran to a release: one entry, and
/// an undo back to `baseline`'s bytes. `expected` is the map one apply of
/// its final request makes, which the committed map must be.
fn assert_committed_once(init: &mut InitState, expected: &Document, baseline: &str, what: &str) {
    assert!(
        matches!(init.drag_state, DragState::None),
        "{what}: the release ended the drag"
    );
    assert_eq!(
        document(init).open_interaction(),
        None,
        "{what}: no transaction is left open"
    );
    assert_eq!(entries(init), 1, "{what}: the release made one history entry");
    assert_eq!(
        bytes(init),
        format::save(expected).expect("saves"),
        "{what}: the committed map is one apply of the final request"
    );
    assert_eq!(act(init, json!("Undo")), "handled");
    frame(init);
    assert_eq!(bytes(init), baseline, "{what}: undo gives back the saved map");
}

/// Drag from `from` by each of `travel`, in turn, a frame after each, and
/// return where the pointer ended. The press is at `from`.
fn drag(init: &mut InitState, from: Vec2, travel: &[Vec2], mods: &[&str]) -> Vec2 {
    button(init, "left", "press", from, mods);
    let mut at = from;
    for &step in travel {
        at = from + step;
        move_to(init, at, mods);
        frame(init);
    }
    at
}

/// The travel of a gentle drag: six moves, a frame after each, to
/// `(30, 18)` from the press.
fn gentle() -> Vec<Vec2> {
    (1..=6).map(|k| Vec2::new(5.0, 3.0) * k as f32).collect()
}

/// **A node drag is one transaction**: every frame of it stages the
/// request for its total since the press, so after six frames the
/// document — the node and its subtree moved — is exactly one `apply` of
/// the final request; nothing is in the history and a save writes the
/// map as it was; the release makes one entry, which an undo takes back
/// to the saved bytes.
///
/// Controls: the node really moved by the total (an equality against a
/// request that moved nothing would pass the first check vacuously), and
/// its child moved with it.
#[test]
fn test_a_node_drag_is_one_apply_of_its_final_request_and_one_entry() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let (n2, n4) = (node(init, 2), node(init, 4));
    let (p2, p4) = (position(init, n2), position(init, n4));
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &[]);
    assert!(
        matches!(&init.drag_state, DragState::Dragging(live) if live.gesture.word() == "MoveNodes"),
        "the drag moves nodes"
    );
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "after six frames");
    let total = init.drag_state.live().expect("a drag").total();
    assert!(
        (total - Vec2::new(30.0, 18.0)).length() < 1e-3,
        "the total is the travel: {total}"
    );
    assert!(
        (position(init, n2) - (p2 + total)).length() < 1e-3,
        "the node moved by it"
    );
    assert!(
        (position(init, n4) - (p4 + total)).length() < 1e-3,
        "and its subtree with it"
    );
    assert_eq!(entries(init), 0, "a stage is not history");
    assert_eq!(
        document_mut(init).save_json().expect("saves"),
        baseline,
        "a save mid-drag writes the committed map"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "node drag");
}

/// **Alt drags a node alone; Shift drags every selected node with it**,
/// as on `main`, and the node a Shift drag grabbed joins the selection,
/// so the tint covers every node that moves (#395's review).
///
/// Controls: the plain drag above moves the child; and a plain drag of a
/// node not selected makes it the whole selection.
#[test]
fn test_alt_drags_a_node_alone_and_shift_drags_the_selection() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let (n2, n3, n4) = (node(init, 2), node(init, 3), node(init, 4));
    let (p2, p3, p4) = (position(init, n2), position(init, n3), position(init, n4));
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &["alt"]);
    button(init, "left", "release", at, &["alt"]);
    frame(init);
    assert!(position(init, n2) != p2, "Alt still drags the node");
    assert_eq!(position(init, n4), p4, "but not its child");

    document_mut(init).selection.set([n3]);
    frame(init);
    let p2 = position(init, n2);
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &["shift"]);
    button(init, "left", "release", at, &["shift"]);
    frame(init);
    let moved = position(init, n2) - p2;
    assert!(moved.length() > 1.0);
    assert!(
        (position(init, n3) - (p3 + moved)).length() < 1e-3,
        "the selected node moved as far"
    );
    assert_eq!(entries(init), 2, "each drag is one entry");
    assert_eq!(
        document(init).selection.items(),
        &[n3, n2],
        "the Shift drag's node joined the selection"
    );

    document_mut(init).selection.set([n3]);
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &[]);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(
        document(init).selection.items(),
        &[n2],
        "control: a plain drag makes the node the selection"
    );
}

/// **A stage is absolute from the press, never accumulated**: a drag to
/// +10, then +20, then back to +5 ends at +5 — mid-drag and committed.
///
/// Control: the middle frame stood at +20, so the last frame's +5 is not
/// a stage that never ran.
#[test]
fn test_a_drag_to_ten_then_twenty_then_five_ends_at_five() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n3 = node(init, 3);
    let p3 = position(init, n3);
    let from = rect(init, n3).center();
    button(init, "left", "press", from, &[]);
    move_to(init, from + Vec2::new(10.0, 0.0), &[]);
    frame(init);
    assert!((position(init, n3) - (p3 + Vec2::new(10.0, 0.0))).length() < 1e-3);
    move_to(init, from + Vec2::new(20.0, 0.0), &[]);
    frame(init);
    assert!(
        (position(init, n3) - (p3 + Vec2::new(20.0, 0.0))).length() < 1e-3,
        "control: the frame at +20 staged +20"
    );
    move_to(init, from + Vec2::new(5.0, 0.0), &[]);
    frame(init);
    assert!(
        (position(init, n3) - (p3 + Vec2::new(5.0, 0.0))).length() < 1e-3,
        "back at +5"
    );
    button(init, "left", "release", from + Vec2::new(5.0, 0.0), &[]);
    frame(init);
    assert!(
        (position(init, n3) - (p3 + Vec2::new(5.0, 0.0))).length() < 1e-3,
        "committed at +5"
    );
    assert_eq!(entries(init), 1);
}

/// **Input never writes the document; the frame stages it, once**: many
/// moves between two frames stage nothing until the frame, and the frame
/// stages one change for all of them.
///
/// Control: the frame that follows does change the document.
#[test]
fn test_moves_between_frames_stage_once_at_the_frame() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n3 = node(init, 3);
    let p3 = position(init, n3);
    let from = rect(init, n3).center();
    button(init, "left", "press", from, &[]);
    move_to(init, from + Vec2::new(8.0, 0.0), &[]);
    frame(init);
    let staged = position(init, n3);
    let fed = init.view.as_ref().expect("a view").pending_feed().len();
    for k in 1..=5 {
        move_to(init, from + Vec2::new(8.0 + 4.0 * k as f32, 0.0), &[]);
    }
    assert_eq!(position(init, n3), staged, "five moves wrote nothing");
    frame(init);
    assert!(
        (position(init, n3) - (p3 + Vec2::new(28.0, 0.0))).length() < 1e-3,
        "the frame staged all five"
    );
    let records = &init.view.as_ref().expect("a view").pending_feed()[fed..];
    assert_eq!(records.len(), 1, "one stage, one change record: {records:?}");
}

/// **Escape cancels a drag**: the document is exactly as before the
/// press, element for element, with no entry and no transaction open.
///
/// Control: before the key the node had moved.
#[test]
fn test_escape_cancels_a_drag_and_the_map_is_as_before() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    let p2 = position(init, n2);
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &[]);
    assert!(position(init, n2) != p2, "control: the drag moved the node");
    tap(init, "Escape");
    frame(init);
    assert!(matches!(init.drag_state, DragState::None));
    assert_eq!(document(init).open_interaction(), None);
    assert_same_document(document(init).doc(), &before, "after Escape");
    assert_eq!(entries(init), 0);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 0, "the release after it commits nothing");
    assert_eq!(bytes(init), baseline);
}

/// **Undo during a drag cancels it** (R36): the map is as before the
/// press, no entry, and the drag ends at its next frame; the release
/// commits nothing.
#[test]
fn test_undo_during_a_drag_cancels_it() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    let from = rect(init, n2).center();
    let at = drag(init, from, &gentle(), &[]);
    assert_eq!(
        act(init, json!("Undo")),
        "handled",
        "the undo cancels the open drag"
    );
    frame(init);
    assert!(
        matches!(init.drag_state, DragState::None),
        "the drag ended at its frame"
    );
    assert_same_document(document(init).doc(), &before, "after the undo");
    move_to(init, at + Vec2::new(10.0, 0.0), &[]);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 0);
    assert_same_document(document(init).doc(), &before, "after the release");
}

/// **A drag while another interactive transaction is open is refused**
/// (D28): nothing is committed, no drag starts, the open transaction is
/// still open, the selection is as it was (#395's review: the press
/// reselected before the refusal), and the status line says why.
///
/// Control: once that transaction is canceled the same drag moves the
/// node, and makes it the selection.
#[test]
fn test_a_drag_while_another_transaction_is_open_is_refused() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let (n2, n3) = (node(init, 2), node(init, 3));
    let p2 = position(init, n2);
    document_mut(init).selection.set([n3]);
    assert!(document_mut(init).begin(
        ChangeSource::Action,
        TransactionKind::Interactive,
        Interaction::Test
    ));
    let from = rect(init, n2).center();
    button(init, "left", "press", from, &[]);
    move_to(init, from + Vec2::new(8.0, 0.0), &[]);
    assert!(
        matches!(init.drag_state, DragState::None),
        "the crossing started no drag, before any frame ran"
    );
    let at = drag(init, from, &gentle(), &[]);
    assert!(
        matches!(init.drag_state, DragState::None),
        "nor did any frame after it"
    );
    assert_eq!(document(init).status(), Some(TRANSACTION_OPEN_STATUS));
    assert_eq!(
        document(init).open_interaction(),
        Some(Interaction::Test),
        "and the other is still open"
    );
    assert_eq!(position(init, n2), p2);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 0, "nothing was committed");
    assert_eq!(
        document(init).selection.items(),
        &[n3],
        "the selection is as it was"
    );

    assert!(document_mut(init).cancel(Interaction::Test));
    let at = drag(init, from, &gentle(), &[]);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert!(
        position(init, n2) != p2,
        "control: with it closed, the drag moves the node"
    );
    assert_eq!(entries(init), 1);
    assert_eq!(document(init).selection.items(), &[n2], "control: and selects it");
}

/// **A lost focus cancels a drag**: no release will commit it, so the map
/// is as before and no transaction is left to refuse every later change.
#[test]
fn test_a_lost_focus_cancels_a_drag() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    let from = rect(init, n2).center();
    drag(init, from, &gentle(), &[]);
    assert!(init.input_context().on_focus_lost());
    frame(init);
    assert!(matches!(init.drag_state, DragState::None));
    assert_eq!(document(init).open_interaction(), None, "nothing is left open");
    assert_same_document(document(init).doc(), &before, "after the focus loss");
}

/// The minimum size `node` holds.
fn min_size(init: &InitState, node: NodeId) -> Option<Vec2> {
    match document(init).doc().get(node) {
        Some(DocElement::Node(data)) => data.min_size,
        Some(DocElement::Section(data)) => data.min_size,
        _ => panic!("{node:?} is a node or a section"),
    }
}

/// Enter the resize mode on the selection, and draw its handles.
fn resize_mode(init: &mut InitState) {
    assert_eq!(act(init, json!("EnterResizeMode")), "handled");
    frame(init);
}

/// **The handles follow a resize of a node that draws no frame** (#395's
/// review): node 2 draws none, so growing it writes no run it owns. Its
/// south-east handle, dragged, stays on the drawn corner —
/// `node_rect().max()` — mid-drag, after the commit, after an undo and
/// after a redo: the handle chrome follows the projector's geometry
/// (`Projector::geometry_moved`), not the runs a node owns.
///
/// Controls, seen red: following on the runs the target owns
/// (`PassChanges::touches`, the code #395 shipped with), and following
/// nothing once the targets are shown. And here, the corner moved: a
/// handle that stayed put would not be on it.
#[test]
fn test_the_handles_follow_a_frameless_nodes_resize() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n2 = node(init, 2);
    let se = Handle::Node {
        node: n2,
        side: ResizeSide::SE,
    };
    document_mut(init).selection.select_only(n2);
    resize_mode(init);
    let start = rect(init, n2);
    assert!(
        handle_at(init, se).distance(start.max()) < 1e-3,
        "the handle is on the drawn corner"
    );
    let on_the_corner = |init: &InitState, what: &str| {
        let corner = rect(init, n2).max();
        let center = handle_at(init, se);
        assert!(
            center.distance(corner) < 1e-3,
            "{what}: the handle is at {center}, the corner at {corner}"
        );
        corner
    };
    let travel: Vec<Vec2> = (1..=4).map(|k| Vec2::new(25.0, 12.0) * k as f32).collect();
    let at = drag(init, start.max(), &travel, &[]);
    let grown = on_the_corner(init, "mid-drag");
    assert!(
        grown.distance(start.max()) > 50.0,
        "control: the corner moved, from {} to {grown}",
        start.max()
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 1);
    assert!(on_the_corner(init, "after the commit").distance(grown) < 1e-3);
    assert_eq!(act(init, json!("Undo")), "handled");
    frame(init);
    assert!(on_the_corner(init, "after an undo").distance(start.max()) < 1e-3);
    assert_eq!(act(init, json!("Redo")), "handled");
    frame(init);
    assert!(on_the_corner(init, "after a redo").distance(grown) < 1e-3);
}

/// **A resize handle is a transaction**: in the resize mode on node 5,
/// its south-east handle dragged `(40, 20)` stages the node's minimum
/// size as its drawn size grown by that — one apply of the final request
/// mid-drag — and the release makes one entry an undo takes back.
///
/// Controls: outside the resize mode no node handle is shown, so the
/// press cannot be a handle's; and the north-west handle moves the
/// node's position while its far corner stays put, held at its content
/// when dragged far past it.
#[test]
fn test_a_node_resize_handle_is_one_transaction() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n5 = node(init, 5);
    let start = rect(init, n5);
    let se = Handle::Node {
        node: n5,
        side: ResizeSide::SE,
    };
    assert_eq!(
        init.view.as_ref().expect("a view").handle_center(se),
        None,
        "control: no mode, no handle"
    );
    document_mut(init).selection.select_only(n5);
    resize_mode(init);
    assert_eq!(
        init.interaction_mode,
        InteractionMode::Resize {
            target: ResizeTarget::Node(n5)
        }
    );
    let from = handle_at(init, se);
    assert!(
        (from - start.max()).length() < 1e-3,
        "the handle is on the drawn corner"
    );
    let travel: Vec<Vec2> = (1..=4).map(|k| Vec2::new(10.0, 5.0) * k as f32).collect();
    let at = drag(init, from, &travel, &[]);
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-resize");
    let grown = min_size(init, n5).expect("a minimum was staged");
    assert!(
        (grown - (start.size + Vec2::new(40.0, 20.0))).length() < 1e-3,
        "{grown}"
    );
    assert_eq!(
        position(init, n5),
        start.origin,
        "a south-east handle leaves the position"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "node resize");

    // The north-west handle, pulled far past the content: the far corner
    // holds, and the node stops at what it holds rather than slide.
    let nw = Handle::Node {
        node: n5,
        side: ResizeSide::NW,
    };
    let from = handle_at(init, nw);
    let at = drag(init, from, &[Vec2::new(20.0, 10.0), Vec2::new(400.0, 400.0)], &[]);
    let size = min_size(init, n5).expect("staged");
    let far = position(init, n5) + size;
    assert!(
        (far - start.max()).length() < 1e-3,
        "the far corner stays put: {far} vs {}",
        start.max()
    );
    assert!(
        size.x > 0.0 && size.y > 0.0 && size.x < start.size.x,
        "held at the content: {size}"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert!(
        (rect(init, n5).max() - start.max()).length() < 1e-3,
        "the node drawn has not slid"
    );
}

/// **A section's resize handles resize the section** (the resize mode on a
/// selected section, which a click in node editing selects; here the
/// harness selects it, and `tests_gpu_text` drives node editing): its
/// south-east handle dragged `(30, 12)` stages its minimum size, one
/// entry.
#[test]
fn test_a_section_resize_handle_is_one_transaction() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n5 = node(init, 5);
    let section = child(init, n5, DocKind::Section, 1);
    document_mut(init).selection.set([section]);
    resize_mode(init);
    assert_eq!(
        init.interaction_mode,
        InteractionMode::Resize {
            target: ResizeTarget::Section(section)
        }
    );
    let start = init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .section_rect(section)
        .expect("drawn");
    let from = handle_at(
        init,
        Handle::Section {
            section,
            side: ResizeSide::SE,
        },
    );
    let at = drag(init, from, &[Vec2::new(15.0, 6.0), Vec2::new(30.0, 12.0)], &[]);
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-resize");
    let grown = min_size(init, section).expect("staged");
    assert!(
        (grown - (start.size + Vec2::new(30.0, 12.0))).length() < 1e-3,
        "{grown}"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "section resize");
}

/// **A section move is one transaction** (node editing starts it, which
/// `tests_gpu_text` drives; here the harness does): its offset is staged at its start plus the
/// travel, one entry.
#[test]
fn test_a_section_move_is_one_transaction() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n5 = node(init, 5);
    let section = child(init, n5, DocKind::Section, 1);
    let start = super::live::offset_of(document(init).doc(), section).expect("a section");
    let from = rect(init, n5).center();
    move_to(init, from, &[]);
    button(init, "left", "press", from, &[]);
    let gesture = super::live::grab::move_section(document(init).doc(), section).expect("a section");
    assert!(super::event_cursor_moved::start_drag(
        &mut init.input_context(),
        gesture,
        crate::application::platform::input::MouseButton::Left,
        Vec2::ZERO,
    ));
    for k in 1..=3 {
        move_to(init, from + Vec2::new(6.0, 4.0) * k as f32, &[]);
        frame(init);
    }
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-move");
    let offset = super::live::offset_of(document(init).doc(), section).expect("a section");
    assert!(
        (offset - (start + Vec2::new(18.0, 12.0))).length() < 1e-3,
        "{offset}"
    );
    button(init, "left", "release", from + Vec2::new(18.0, 12.0), &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "section move");
}

/// The control points of `path`.
fn points(init: &InitState, path: NodeId) -> Vec<Vec2> {
    crate::application::view::handles::path_data(document(init).doc(), path)
        .expect("a path")
        .control_points
        .clone()
}

/// **A selected path's midpoint handle adds its control point** where the
/// drag takes it (#248's ruling C1): the path passes under the pointer —
/// the control point is the drag's travel past where the curve's middle
/// was — and the release makes one entry; once it has a point, the path
/// shows that point's handle and no midpoint.
///
/// Control: the path had none before, and shows no control handle.
#[test]
fn test_a_paths_midpoint_handle_adds_its_control_point() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    let connector = child(init, n2, DocKind::Connector, 0);
    assert!(points(init, connector).is_empty(), "control: no point yet");
    document_mut(init).selection.select_only(connector);
    frame(init);
    let mid = Handle::Path {
        path: connector,
        handle: PathHandle::Midpoint,
    };
    let control = Handle::Path {
        path: connector,
        handle: PathHandle::Control(0),
    };
    assert_eq!(init.view.as_ref().expect("a view").handle_center(control), None);
    let from = handle_at(init, mid);
    let (t, off) = init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .label_place_at(connector, from)
        .expect("drawn");
    assert!(
        (t - 0.5).abs() < 0.05 && off.abs() < 0.5,
        "the midpoint is halfway along the path: t {t}, off {off}"
    );
    let at = drag(init, from, &[Vec2::new(0.0, 15.0), Vec2::new(0.0, 30.0)], &[]);
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-drag");
    assert_eq!(points(init, connector).len(), 1, "one point is staged");
    let shown = handle_at(init, control);
    assert!(
        (shown - at).length() < 2.0,
        "the path passes under the pointer: {shown} vs {at}"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(
        init.view.as_ref().expect("a view").handle_center(mid),
        None,
        "no midpoint once it has a point"
    );
    assert_committed_once(init, &expected, &baseline, "midpoint drag");
}

/// **A control point's handle moves the point by the drag's travel**, and
/// **an end's handle attaches the end to the side nearest the pointer**:
/// each one transaction, one entry.
#[test]
fn test_a_control_point_and_an_end_handle_are_transactions() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n3 = node(init, 3);
    let link = child(init, n3, DocKind::Link, 0);
    let start = points(init, link);
    document_mut(init).selection.select_only(link);
    frame(init);
    let from = handle_at(
        init,
        Handle::Path {
            path: link,
            handle: PathHandle::Control(0),
        },
    );
    let at = drag(init, from, &[Vec2::new(10.0, 5.0), Vec2::new(20.0, 10.0)], &[]);
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "control point mid-drag");
    assert!((points(init, link)[0] - (start[0] + Vec2::new(20.0, 10.0))).length() < 1e-3);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "control point drag");

    // The start, at node 3, dragged above it: the start attaches on top.
    let before = document(init).doc().clone();
    let from = handle_at(
        init,
        Handle::Path {
            path: link,
            handle: PathHandle::Start,
        },
    );
    let top = rect(init, n3);
    let target = Vec2::new(top.center().x, top.origin.y - 20.0);
    let at = drag(init, from, &[(from + target) * 0.5 - from, target - from], &[]);
    let anchors = crate::application::view::handles::path_data(document(init).doc(), link)
        .expect("a path")
        .anchors;
    assert_eq!(
        anchors.0,
        baumhard::document::values::Anchor::Top,
        "the start is on top"
    );
    let expected = one_apply_of_the_drag(init, &before);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 1, "one entry for the end's drag");
    assert_eq!(bytes(init), format::save(&expected).expect("saves"));
}

/// **A label slides along its path and off it**: pressing the connector's
/// label and dragging it stages the label's `t` and offset that put it
/// under the pointer (`Projector::label_place_at`), one entry.
#[test]
fn test_a_label_drag_is_one_transaction() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    let connector = child(init, n2, DocKind::Connector, 0);
    let label = child(init, connector, DocKind::Label, 0);
    let from = init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .label_pos(label)
        .expect("drawn");
    let at = drag(init, from, &[Vec2::new(12.0, 6.0), Vec2::new(24.0, 12.0)], &[]);
    assert!(
        matches!(&init.drag_state, DragState::Dragging(live) if live.gesture.word() == "Label"),
        "the press grabbed the label"
    );
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-drag");
    let centered = init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .label_pos(label)
        .expect("drawn");
    assert!(
        (centered - at).length() < 1.0,
        "the label centers under the pointer: {centered} vs {at}"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "label drag");
}

/// Where the projection draws portal end `end`'s marker, and the label
/// `label`'s box: what a portal-end drag is held to, read where it is
/// drawn rather than from the place the drag wrote.
fn drawn_marker(init: &InitState, end: NodeId) -> Vec2 {
    init.view
        .as_ref()
        .expect("a view")
        .projector()
        .portal_end_pos(document(init).doc(), end)
        .expect("the end is drawn")
}

/// See [`drawn_marker`].
fn drawn_label(init: &InitState, label: NodeId) -> Vec2 {
    init.view
        .as_ref()
        .expect("a view")
        .projector()
        .label_pos(label)
        .expect("the label is drawn")
}

/// Drag what is under `from` to `to` in four moves, a frame after each,
/// and release there.
fn drag_to(init: &mut InitState, from: Vec2, to: Vec2) {
    let travel: Vec<Vec2> = (1..=4).map(|k| (to - from) * (k as f32 / 4.0)).collect();
    let at = drag(init, from, &travel, &[]);
    button(init, "left", "release", at, &[]);
    frame(init);
}

/// **A portal-end drag puts what it holds under the pointer** (#395's
/// review): the end's label, grabbed off its center, is drawn where the
/// pointer takes it — at the place on it the press held — for the
/// review's own drag of `(20, 40)`, then below node 1's bottom side and
/// past its bottom-right corner, where the offset curve sweeps round; the
/// marker, grabbed itself, past the top-right corner; a re-grab of the
/// label that moves 6 px moves it 6 px; and a press that does not move
/// moves nothing. Read where they are drawn (`Projector::label_pos`,
/// `Projector::portal_end_pos`), not from the place the drag wrote. Each
/// drag is one apply of its request mid-drag and one entry; undoing them
/// all gives back the map.
///
/// Every target is out past where the end draws what it holds with no
/// offset (the label is some 90 units out from node 1, whose marker is
/// taller than the node): nearer, the end would have to sit inside the
/// node, whose offset curve inward folds over (`geometry::portal_at`).
///
/// Controls, seen red: the inverse that leaves out the marker's and the
/// label's reach (`Projector::portal_place_at` handing `portal_at` no
/// reach); a drag that puts the pointer, not the grabbed point, where the
/// end goes (`start` the press's pointer). How a sharp corner is solved
/// is held where it is computed (`projection::tests::inverse_tests`).
#[test]
fn test_a_portal_end_drag_puts_what_it_holds_under_the_pointer() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let n1 = node(init, 1);
    let link = child(init, n1, DocKind::Link, 0);
    let end = child(init, link, DocKind::PortalEnd, 0);
    let label = child(init, end, DocKind::Label, 0);
    let r = rect(init, n1);
    let held = Vec2::new(3.0, 2.0);

    // The review's drag: the label, off its center, by (20, 40). Mid-drag
    // one apply of the request, and drawn where the pointer took it.
    let before = document(init).doc().clone();
    let from = drawn_label(init, label) + held;
    let to = from + Vec2::new(20.0, 40.0);
    let travel: Vec<Vec2> = (1..=4).map(|k| (to - from) * (k as f32 / 4.0)).collect();
    let at = drag(init, from, &travel, &[]);
    assert!(
        matches!(&init.drag_state, DragState::Dragging(live) if live.gesture.word() == "PortalEnd"),
        "the press grabbed the end"
    );
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-drag");
    let drawn = drawn_label(init, label);
    assert!(
        (drawn + held).distance(at) < 1.0,
        "mid-drag, the label is drawn at {drawn}, the pointer at {at} holding it at {held}"
    );
    button(init, "left", "release", at, &[]);
    frame(init);
    assert_eq!(entries(init), 1, "one entry");

    // A re-grab at its drawn center that moves 6 px moves it 6 px.
    let grabbed = drawn_label(init, label);
    drag_to(init, grabbed, grabbed + Vec2::new(6.0, 0.0));
    let moved = drawn_label(init, label);
    assert!(
        moved.distance(grabbed + Vec2::new(6.0, 0.0)) < 1.0,
        "a 6 px re-grab drew the label at {moved}, from {grabbed}"
    );
    assert_eq!(entries(init), 2);

    // Below the bottom side, then past the bottom-right corner, where the
    // offset curve sweeps round it.
    let side = Vec2::new(r.center().x + 10.0, r.max().y + 130.0);
    drag_to(init, moved + held, side);
    let drawn = drawn_label(init, label);
    assert!(
        (drawn + held).distance(side) < 1.0,
        "at a side: {drawn} for {side}"
    );
    let corner = r.max() + Vec2::new(90.0, 110.0);
    drag_to(init, drawn + held, corner);
    let swept = drawn_label(init, label);
    assert!(
        (swept + held).distance(corner) < 1.0,
        "past a corner, the label is drawn at {swept} for {corner}"
    );

    // The marker itself past the top-right corner.
    let marker = drawn_marker(init, end);
    let corner = Vec2::new(r.max().x + 50.0, r.origin.y - 60.0);
    drag_to(init, marker, corner);
    let drawn = drawn_marker(init, end);
    assert!(
        drawn.distance(corner) < 1.0,
        "past a corner, the marker is drawn at {drawn} for {corner}"
    );
    assert_eq!(entries(init), 5, "each drag made one entry");

    // A press on the label that does not move moves nothing, and makes no
    // entry (last, so no press after it pairs with it as a double-click).
    let still = drawn_label(init, label);
    button(init, "left", "press", still, &[]);
    button(init, "left", "release", still, &[]);
    frame(init);
    assert!(drawn_label(init, label).distance(still) < 1e-3);
    assert_eq!(entries(init), 5, "a press that did not move makes no entry");
    for _ in 0..5 {
        assert_eq!(act(init, json!("Undo")), "handled");
        frame(init);
    }
    assert_eq!(bytes(init), baseline, "undoing them all gives back the saved map");
}

/// **A fast resize — Ctrl and the right button, dragged on a node — grabs
/// the corner of the quadrant it was pressed in and resizes from there**,
/// a transaction the right button's release commits, as on `main`.
#[test]
fn test_a_fast_resize_resizes_from_the_pressed_corner() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n5 = node(init, 5);
    let start = rect(init, n5);
    let from = start.max() - Vec2::new(10.0, 10.0);
    move_to(init, from, &["ctrl"]);
    button(init, "right", "press", from, &["ctrl"]);
    for k in 1..=3 {
        move_to(init, from + Vec2::new(10.0, 6.0) * k as f32, &["ctrl"]);
        frame(init);
    }
    assert!(
        matches!(&init.drag_state, DragState::Dragging(live)
            if matches!(live.gesture, super::live::Gesture::ResizeNode { side: ResizeSide::SE, .. })),
        "the south-east corner is grabbed"
    );
    let expected = one_apply_of_the_drag(init, &before);
    assert_same_document(document(init).doc(), &expected, "mid-resize");
    button(init, "right", "release", from + Vec2::new(30.0, 18.0), &["ctrl"]);
    frame(init);
    assert_committed_once(init, &expected, &baseline, "fast resize");
}

/// One finger event, on the canvas.
fn touch(init: &mut InitState, phase: &str, at: Vec2) {
    input(
        init,
        json!({"id": 1, "cmd": "input.touch", "phase": phase, "pointer_id": 0,
               "x": at.x, "y": at.y, "space": "canvas"}),
    );
}

/// **A finger on a resize handle resizes as the mouse does** (native,
/// `touch_handle`): in the resize mode, a finger that starts on a node's
/// handle grabs it, its moves stage the resize, and its lift commits one
/// entry.
///
/// Control: a finger that starts off every handle is the gesture
/// recognizer's one-finger pan, and changes nothing in the map.
#[test]
fn test_a_finger_on_a_resize_handle_resizes_as_the_mouse_does() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let n5 = node(init, 5);
    let start = rect(init, n5);
    document_mut(init).selection.select_only(n5);
    resize_mode(init);

    let off = start.origin - Vec2::new(80.0, 80.0);
    touch(init, "started", off);
    for k in 1..=4 {
        touch(init, "moved", off + Vec2::new(10.0, 10.0) * k as f32);
        frame(init);
    }
    touch(init, "ended", off + Vec2::new(40.0, 40.0));
    frame(init);
    assert_eq!(
        bytes(init),
        baseline,
        "control: a finger off the handles pans, and edits nothing"
    );
    assert_eq!(entries(init), 0);

    let from = handle_at(
        init,
        Handle::Node {
            node: n5,
            side: ResizeSide::SE,
        },
    );
    touch(init, "started", from);
    for k in 1..=4 {
        touch(init, "moved", from + Vec2::new(8.0, 4.0) * k as f32);
        frame(init);
    }
    assert!(
        matches!(init.drag_state, DragState::Dragging(_)),
        "the finger drives the resize"
    );
    touch(init, "ended", from + Vec2::new(32.0, 16.0));
    frame(init);
    assert_eq!(entries(init), 1, "the lift committed one entry");
    let grown = min_size(init, n5).expect("a minimum");
    assert!(
        (grown - (start.size + Vec2::new(32.0, 16.0))).length() < 1e-3,
        "{grown}"
    );
}

/// **An OS cancel of a finger on a resize handle cancels the resize**
/// (#395's review): the OS takes the touch away (a system gesture, a
/// call) mid-resize, no lift is coming, and the transaction is canceled
/// — the map as saved, no entry, no drag left — as a lost focus cancels
/// one; the next finger on the handle resizes from scratch.
///
/// Control: the same resize ended by a lift commits one entry (the
/// cancel is the OS's doing, not a resize that staged nothing). And
/// seen red: `Phase::Cancelled` routed to the release, as #395 shipped.
#[test]
fn test_an_os_touch_cancel_cancels_a_handle_resize() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let n5 = node(init, 5);
    document_mut(init).selection.select_only(n5);
    resize_mode(init);
    let from = handle_at(
        init,
        Handle::Node {
            node: n5,
            side: ResizeSide::SE,
        },
    );
    let saved = min_size(init, n5);
    let finger = |init: &mut InitState, end: &str| {
        touch(init, "started", from);
        for k in 1..=4 {
            touch(init, "moved", from + Vec2::new(8.0, 4.0) * k as f32);
            frame(init);
        }
        assert!(
            matches!(init.drag_state, DragState::Dragging(_)),
            "the finger drives the resize"
        );
        assert_ne!(min_size(init, n5), saved, "a size is staged");
        touch(init, end, from + Vec2::new(32.0, 16.0));
        frame(init);
    };
    finger(init, "canceled");
    assert!(matches!(init.drag_state, DragState::None), "no drag is left");
    assert_eq!(document(init).open_interaction(), None, "no transaction is left");
    assert_eq!(entries(init), 0, "a cancel makes no entry");
    assert_eq!(bytes(init), baseline, "the map is as saved");

    finger(init, "ended");
    assert_eq!(entries(init), 1, "control: the lift commits one entry");
}

/// The open picker's layout.
fn layout(init: &InitState) -> crate::application::color_picker::ColorPickerLayout {
    init.color_picker_state
        .as_open()
        .and_then(|picker| picker.layout.clone())
        .expect("the picker is open and laid out")
}

/// Move the pointer to `at`, a point on the surface.
fn move_on_surface(init: &mut InitState, at: (f32, f32)) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": at.0, "y": at.1}),
    );
}

/// Click the left button at `at`, a point on the surface.
fn click_on_surface(init: &mut InitState, at: (f32, f32)) {
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_button", "button": "left", "state": "click", "x": at.0, "y": at.1}),
    );
}

/// Open the contextual picker on the selection's text color, through the
/// console line that opens it.
fn open_picker(init: &mut InitState) {
    let reply = ipc(init, json!({"id": 1, "cmd": "act.console", "line": "color text"}));
    assert_eq!(reply["ok"], true, "{reply}");
    frame(init);
    assert!(init.color_picker_state.is_open(), "{reply}");
    assert_eq!(document(init).open_interaction(), Some(Interaction::ColorPicker));
}

/// **The contextual picker is a transaction**: hovering over five colors
/// stages each on the selected node in its frame — the map shows it —
/// and a cancel leaves the document byte-for-byte as it was, with the
/// history unchanged.
///
/// Control: each hover did change the map while the picker was open.
#[test]
fn test_picker_hovers_then_cancel_leave_the_map_as_it_was() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let before = document(init).doc().clone();
    let n2 = node(init, 2);
    document_mut(init).selection.select_only(n2);
    open_picker(init);
    let slots = layout(init).hue_slot_positions;
    let mut shown = Vec::new();
    for slot in [0, 3, 6, 9, 12] {
        move_on_surface(init, slots[slot]);
        frame(init);
        let color = baumhard::document::resolve::resolve_text(document(init).doc().tree(), n2, &mut ()).color;
        shown.push(color);
    }
    shown.dedup();
    assert_eq!(shown.len(), 5, "control: each hover staged its color: {shown:?}");
    assert_eq!(entries(init), 0, "hovers are not history");
    tap(init, "Escape");
    frame(init);
    assert!(!init.color_picker_state.is_open());
    assert_eq!(document(init).open_interaction(), None);
    assert_same_document(document(init).doc(), &before, "after the cancel");
    assert_eq!(bytes(init), baseline);
    assert_eq!(entries(init), 0);
}

/// **Picking a color and committing it is one entry**: hover, click a hue
/// cell, click ࿕ — the committed map is one apply of the picked color on
/// the selection; an undo gives the saved map back.
///
/// Control: a picker committed with nothing picked — hovered only —
/// makes no entry, as on `main`.
#[test]
fn test_picker_hover_then_click_commits_one_entry() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let n2 = node(init, 2);
    document_mut(init).selection.select_only(n2);

    open_picker(init);
    let lay = layout(init);
    move_on_surface(init, lay.hue_slot_positions[4]);
    frame(init);
    click_on_surface(init, lay.center);
    frame(init);
    assert!(!init.color_picker_state.is_open(), "control: ࿕ closed it");
    assert_eq!(entries(init), 0, "control: nothing picked, nothing committed");
    assert_eq!(bytes(init), baseline);

    let before = document(init).doc().clone();
    open_picker(init);
    let lay = layout(init);
    for slot in [2, 5] {
        move_on_surface(init, lay.hue_slot_positions[slot]);
        frame(init);
    }
    click_on_surface(init, lay.hue_slot_positions[7]);
    frame(init);
    let picked = {
        let picker = init.color_picker_state.as_open().expect("still open");
        (picker.hue_deg, picker.sat, picker.val)
    };
    let target = crate::application::color_picker::PickerTarget {
        axis: crate::application::keybinds::ColorAxis::Text,
        selection: document(init).selection.clone(),
    };
    let steps = super::color_picker_flow::session::steps_for_test(document(init), &target, picked)
        .expect("the color applies to the node");
    let mut expected = before.clone();
    expected
        .apply(EditRequest::new(steps, "expected"), ChangeSource::Input)
        .expect("applies");
    click_on_surface(init, lay.center);
    frame(init);
    assert!(!init.color_picker_state.is_open());
    assert_committed_once(init, &expected, &baseline, "picker commit");
}

/// **While the picker is open, a drag is refused** (D28: the picker is an
/// interactive transaction) and the picker stays open; the cancel then
/// lets the same drag through.
#[test]
fn test_a_drag_while_the_picker_is_open_is_refused() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n2 = node(init, 2);
    let n3 = node(init, 3);
    document_mut(init).selection.select_only(n2);
    open_picker(init);
    // The contextual picker takes every press inside its backdrop; this
    // drag goes through the document's own refusal, which is the rule.
    assert!(!document_mut(init).begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        Interaction::Drag
    ));
    assert_eq!(document(init).status(), Some(TRANSACTION_OPEN_STATUS));
    assert!(init.color_picker_state.is_open());
    tap(init, "Escape");
    frame(init);
    let p3 = position(init, n3);
    let from = rect(init, n3).center();
    let at = drag(init, from, &gentle(), &[]);
    button(init, "left", "release", at, &[]);
    frame(init);
    assert!(
        position(init, n3) != p3,
        "control: with the picker closed the drag moves the node"
    );
}

/// **A presentation keeps playing on top of a drag** (§10.2): a `hold`
/// presentation lowering node 2's opacity stays on it through every frame
/// of a drag of that node; the drag's entry holds the move alone, the
/// presentation never reaches the file, and it plays on after the commit
/// and after the undo.
///
/// Control: the presentation does show (the opacity read is its, not the
/// map's).
#[test]
fn test_a_presentation_stays_on_top_during_a_drag() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let n2 = node(init, 2);
    let p2 = position(init, n2);
    let dim = baumhard::document::presentation::parse_presentations(
        r#"{"presentations":[{"id":"dim","name":"Dim","play":"hold","target":"node",
        "mutations":[{"node":{"set_opacity":0.25}}]}]}"#,
    )
    .expect("reads")
    .remove(0);
    let now = init.clock.now_ms();
    document_mut(init)
        .start_presentation(&dim, &[n2], now)
        .expect("starts");
    frame(init);
    let opacity = |init: &InitState| match document(init).doc().get(n2) {
        Some(DocElement::Node(data)) => data.opacity,
        _ => panic!("node 2"),
    };
    assert_eq!(opacity(init), 0.25, "control: the presentation shows");
    let from = rect(init, n2).center();
    button(init, "left", "press", from, &[]);
    for step in gentle() {
        move_to(init, from + step, &[]);
        frame(init);
        assert_eq!(opacity(init), 0.25, "the presentation plays on top of the stage");
        assert_eq!(document(init).doc().presentations().count(), 1);
    }
    let moved = position(init, n2);
    assert!(moved != p2, "the stage moved the node under it");
    button(init, "left", "release", from + Vec2::new(30.0, 18.0), &[]);
    frame(init);
    assert_eq!(entries(init), 1);
    assert_eq!(opacity(init), 0.25, "and on top of the commit");
    assert!(
        !bytes(init).contains("\"opacity\""),
        "a presentation never reaches the file"
    );
    assert_eq!(act(init, json!("Undo")), "handled");
    frame(init);
    assert_eq!(bytes(init), baseline, "the entry held the move alone");
    assert_eq!(
        opacity(init),
        0.25,
        "and the presentation plays on after the undo"
    );
}

/// **The handle chrome is rebuilt only when what it is built from
/// changes**: selecting a path shows its handles (the renderer applies a
/// chrome journal); a pan then writes no chrome at all; moving the path's
/// end node moves its handles in place.
///
/// Control: the selection's chrome apply visited the path set's runs.
#[test]
fn test_a_pan_writes_no_chrome_and_a_selection_does() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n2 = node(init, 2);
    let connector = child(init, n2, DocKind::Connector, 0);
    frame(init);
    let applies = |init: &InitState| init.renderer.layer_renderer().log().chrome_applies;
    let before = applies(init);
    document_mut(init).selection.select_only(connector);
    frame(init);
    assert_eq!(applies(init), before + 1, "the selection wrote the handles");
    let group = init
        .view
        .as_ref()
        .expect("a view")
        .handles()
        .group(HandleSet::Path);
    let visited = init.renderer.layer_renderer().log().chrome_visited.clone();
    let chrome = init.view.as_ref().expect("a view").chrome();
    let in_path_set: Vec<NodeId> = group.children(chrome.arena()).collect();
    assert!(
        visited.iter().any(|id| in_path_set.contains(id)),
        "control: it wrote the path set's runs"
    );
    // The end of the connector that is at node 1, its parent.
    let n1 = node(init, 1);
    let (from, _) = init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .path_ends(connector)
        .expect("drawn");
    let at_one = if from == n1 {
        PathHandle::Start
    } else {
        PathHandle::End
    };
    let at = handle_at(
        init,
        Handle::Path {
            path: connector,
            handle: at_one,
        },
    );

    let panned = applies(init);
    let rebuilt = |init: &InitState| init.view.as_ref().expect("a view").handles().rebuilt();
    let rebuilt_before = rebuilt(init);
    assert_eq!(act(init, json!("PanCameraEast")), "handled");
    frame(init);
    assert_eq!(act(init, json!("ZoomIn")), "handled");
    frame(init);
    assert_eq!(applies(init), panned, "a pan and a zoom write no chrome");
    assert_eq!(rebuilt(init), rebuilt_before, "nor rebuild any handle set");

    // Moving the node the path ends at moves the path's handles.
    let r1 = rect(init, n1);
    let steps = baumhard::document::mutation::compose::set(
        document(init).doc(),
        DocKind::Node,
        vec![baumhard::document::mutation::ops::DocMutation::Node(
            baumhard::document::mutation::ops::NodeOp::SetPosition(r1.origin + Vec2::new(0.0, 60.0)),
        )],
        &[n1],
    )
    .expect("composes");
    document_mut(init)
        .apply(EditRequest::new(steps, "test: move 1"), ChangeSource::Action)
        .expect("applies");
    frame(init);
    assert_eq!(
        applies(init),
        panned + 1,
        "the touched path's handles were written"
    );
    let moved = handle_at(
        init,
        Handle::Path {
            path: connector,
            handle: at_one,
        },
    );
    assert!(moved != at, "and its end handle moved with it");
}

/// **A stage owed keeps the loop running until a frame stages it**: a
/// drag the pointer moved, and a contextual picker whose hover changed,
/// each owe their transaction a stage the frame has not run — what a
/// throttled frame also leaves — so the loop is not let park
/// (`InitState::needs_continuation`); once a frame has staged it, nothing
/// is owed. #395's review: the picker's held-back stage parked the loop
/// with the wheel showing a color its target did not.
///
/// Controls: before the move and the hover nothing is owed (the loop is
/// idle on this map), and seen red: the picker's owed stage left out of
/// `needs_continuation`.
#[test]
fn test_a_stage_owed_keeps_the_loop_running() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    assert!(!init.needs_continuation(), "control: an idle map owes nothing");
    let n3 = node(init, 3);
    let from = rect(init, n3).center();
    button(init, "left", "press", from, &[]);
    move_to(init, from + Vec2::new(8.0, 0.0), &[]);
    frame(init);
    assert!(!init.needs_continuation(), "the drag is staged");
    move_to(init, from + Vec2::new(20.0, 0.0), &[]);
    assert!(init.needs_continuation(), "a move owes the drag a stage");
    frame(init);
    assert!(!init.needs_continuation(), "the frame staged it");
    button(init, "left", "release", from + Vec2::new(20.0, 0.0), &[]);
    frame(init);

    let n2 = node(init, 2);
    document_mut(init).selection.select_only(n2);
    open_picker(init);
    frame(init);
    assert!(!init.needs_continuation(), "the picker staged what it shows");
    move_on_surface(init, layout(init).hue_slot_positions[9]);
    assert!(init.needs_continuation(), "a hover owes the picker a stage");
    frame(init);
    assert!(!init.needs_continuation(), "the frame staged it");
    tap(init, "Escape");
    frame(init);
}

/// **An open picker's target draws the color it stages** (#248's ruling on
/// the picker's preview): with node 2 still selected, the selection tint
/// stands down on its text while the picker is open, so a hovered color is
/// what node 2 is drawn in — its runs carry no tint, and the document holds
/// the hovered color — and the tint is back once the picker closes.
///
/// Control: before the picker opens, node 2 wears the tint (the stand-down
/// is the picker's doing, not a selection that tints nothing).
#[test]
fn test_an_open_pickers_target_draws_the_staged_color() {
    let Some(mut app) = app() else { return };
    let init = &mut app.init;
    let n2 = node(init, 2);
    document_mut(init).selection.select_only(n2);
    frame(init);
    let tinted_runs = |init: &InitState| {
        let view = init.view.as_ref().expect("a view");
        let own = view.runs_of_for_test(document(init).doc(), n2);
        let stamped = view.tinted_runs();
        own.iter().filter(|run| stamped.contains(run)).count()
    };
    assert!(tinted_runs(init) > 0, "control: the selected node wears the tint");

    open_picker(init);
    let slot = layout(init).hue_slot_positions[9];
    move_on_surface(init, slot);
    frame(init);
    assert_eq!(tinted_runs(init), 0, "the tint stood down on the picker's target");
    let shown = init.color_picker_state.as_open().expect("open").shown_hsv();
    let [r, g, b] = baumhard::util::color_conversion::hsv_to_rgb(shown.0, shown.1, shown.2);
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let drawn = baumhard::document::resolve::resolve_text(document(init).doc().tree(), n2, &mut ()).color;
    assert_eq!(
        drawn.0[..3],
        [byte(r), byte(g), byte(b)],
        "node 2 is drawn in the hovered color"
    );

    tap(init, "Escape");
    frame(init);
    assert!(tinted_runs(init) > 0, "the tint is back once the picker closes");
}
