// SPDX-License-Identifier: MPL-2.0

//! The field family through the real app (V1-C5, #246; TEST_CONVENTIONS
//! §T8): its Actions sent as IPC `act.action` — `dispatch_action`, the
//! funnel a key, a macro step and a click reach — and its verbs as
//! `act.console` — `execute_console_line`, the funnel the prompt and a
//! macro line reach — on `dispatch/families/fixtures/field.mandala.json`.
//! The per-form expectations are `dispatch::families::field_nodes`'s
//! own tests; these hold the two funnels, the view's sync after each,
//! and `ExitMode`'s reverting of an open frame preview, which only the
//! app's dispatch reaches.

use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use serde_json::{json, Value};

use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use crate::application::document::{Interaction, MindMapDocument, TRANSACTION_OPEN_STATUS};
use crate::application::ipc::run_attached;

/// The fixture.
fn fixture() -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/field.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn document(init: &InitState) -> &MindMapDocument {
    init.document
        .as_ref()
        .expect("the harness always builds a document")
}

/// What a save writes.
fn bytes(init: &InitState) -> String {
    format::save(document(init).doc()).expect("the document saves")
}

/// How many entries an undo can reach.
fn entries(init: &InitState) -> usize {
    document(init).doc().history().count()
}

/// Run one IPC request, through the real decoder and executor.
fn ipc(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// `act.action` with `action`: its outcome and its reason.
fn act(init: &mut InitState, action: Value) -> (String, Value) {
    let reply = ipc(init, json!({"id": 1, "cmd": "act.action", "action": action}));
    let outcome = reply["result"]["outcome"]
        .as_str()
        .unwrap_or_else(|| panic!("act.action answers: {reply}"))
        .to_string();
    (outcome, reply["result"]["reason"].clone())
}

/// `act.console` with `line`: the whole reply.
fn console(init: &mut InitState, line: &str) -> Value {
    ipc(init, json!({"id": 1, "cmd": "act.console", "line": line}))
}

/// **The field family through `act.action` and `act.console`, one entry
/// each, and `ExitMode` reverting an open frame preview.** A color by
/// Action and a ring by console line are each one entry that `Undo`
/// takes back; a frame preview opened from the console refuses an
/// Action with the D28 reason, and `ExitMode` reverts it to the bytes
/// before it with no entry, leaving the next `ExitMode` to the mode.
///
/// Controls: the preview does change the bytes while open, and with it
/// gone the refused Action runs.
#[test]
fn test_the_field_family_runs_through_both_funnels_and_exit_mode_cancels_a_preview() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let fixture_bytes = std::fs::read_to_string(fixture()).expect("the fixture reads");
    assert_eq!(bytes(init), fixture_bytes, "precondition: the saver's spelling");
    let n1 = document(init).doc().node_by_ident(NodeIdent(1)).expect("node 1");
    init.document
        .as_mut()
        .expect("a document")
        .selection
        .select_only(n1);

    let before = entries(init);
    let color = json!({"SetColor": {"axis": "text", "value": "#ff0000"}});
    assert_eq!(act(init, color).0, "handled");
    assert_eq!(entries(init), before + 1, "an Action is one entry");
    assert!(bytes(init).contains("\"color\": \"#ff0000\""));
    assert_eq!(act(init, json!("Undo")).0, "handled");
    assert_eq!(bytes(init), fixture_bytes);

    let reply = console(init, "frame ring add text=poem size=0.1");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(entries(init), before + 1, "a console line is one entry");
    assert_eq!(act(init, json!("Undo")).0, "handled");
    assert_eq!(bytes(init), fixture_bytes);

    let reply = console(init, "frame preview fit=spread");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_ne!(bytes(init), fixture_bytes, "control: the preview shows");
    let (outcome, reason) = act(init, json!("CycleFrameContent"));
    assert_eq!(
        (outcome.as_str(), reason),
        ("unhandled", json!(TRANSACTION_OPEN_STATUS))
    );
    assert_eq!(act(init, json!("ExitMode")).0, "handled");
    init.input_context().drain_frame(Beat::Loop);
    assert_eq!(bytes(init), fixture_bytes, "ExitMode reverted the preview");
    assert_eq!(entries(init), before, "with no entry");
    assert_eq!(document(init).open_interaction(), None::<Interaction>);
    assert_eq!(
        act(init, json!("CycleFrameContent")).0,
        "handled",
        "control: it runs now"
    );
    assert_eq!(entries(init), before + 1);
}

/// Start the bundled animated `grow-font-2pt` at the selection through
/// `act.console`, and tick it part way: the map is mid-animation.
fn grow_part_way(init: &mut InitState) {
    let reply = console(init, "edit apply grow-font-2pt");
    assert_eq!(reply["ok"], true, "{reply}");
    let now = init.clock.now_ms();
    let doc = init.document.as_mut().expect("a document");
    doc.tick_animation(now + 100.0);
    assert!(doc.is_animating(), "precondition: the edit is part way");
}

/// **Both input funnels settle an animated edit before what they run
/// reads the map** (review of #331; D28). With `grow-font-2pt` part way
/// over node 1's first section, `Copy` through `act.action` copies the
/// section as the edit leaves it — what a copy of the committed map
/// holds — not a frame of it. A console verb that reads settles it too
/// (`font size=` refused, after the settle), while the camera and
/// selection Actions and the verbs that only read or drive time
/// (`ZoomIn`, `SelectAll`, `help`) leave it running.
///
/// Controls: with no settle at `dispatch_compatible`, the copy held the
/// part-way sizes; with none at `execute_console_line`, the console
/// line left the edit running; with the camera Action not in the skip
/// list, `ZoomIn` cut the animation short.
#[test]
fn test_both_funnels_settle_an_animated_edit_before_they_read() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    {
        let doc = init.document.as_mut().expect("a document");
        let d = doc.doc();
        let n1 = d.node_by_ident(NodeIdent(1)).expect("node 1");
        let section = d
            .children(n1)
            .find(|&c| {
                d.get(c)
                    .is_some_and(|e| e.kind() == baumhard::document::element::DocKind::Section)
            })
            .expect("node 1's first section");
        doc.selection.select_only(section);
    }

    grow_part_way(init);
    for (action, why) in [
        (json!("ZoomIn"), "the camera"),
        (json!("SelectAll"), "the selection"),
    ] {
        act(init, action);
        assert!(document(init).is_animating(), "{why} leaves the edit running");
    }
    {
        // SelectAll moved the selection; put the section back.
        let doc = init.document.as_mut().expect("a document");
        let d = doc.doc();
        let n1 = d.node_by_ident(NodeIdent(1)).expect("node 1");
        let section = d
            .children(n1)
            .find(|&c| {
                d.get(c)
                    .is_some_and(|e| e.kind() == baumhard::document::element::DocKind::Section)
            })
            .expect("node 1's first section");
        doc.selection.select_only(section);
    }
    assert_eq!(act(init, json!("Copy")).0, "handled");
    let settled_by_copy = !document(init).is_animating();
    // What a copy of the committed map holds: the edit at its end.
    init.document.as_mut().expect("a document").finish_animation();
    let doc = document(init);
    let committed = crate::application::clipboard::take(doc.doc(), &doc.selection, false)
        .expect("the section copies")
        .expect("something to copy")
        .0;
    assert_eq!(
        doc.clipboard.as_ref(),
        Some(&committed),
        "the copy holds the section as the edit leaves it"
    );
    assert!(settled_by_copy, "the Copy settled the edit");

    grow_part_way(init);
    assert_eq!(console(init, "help")["ok"], true);
    assert!(document(init).is_animating(), "`help` leaves the edit running");
    console(init, "font size=nonsense");
    assert!(!document(init).is_animating(), "a verb that reads settled it");
}
