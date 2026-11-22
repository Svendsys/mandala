// SPDX-License-Identifier: MPL-2.0

//! The path family through the real app (V1-C6, #247; TEST_CONVENTIONS
//! §T8): its Actions sent as IPC `act.action` — `dispatch_action`, the
//! funnel a key, a macro step and a click reach — and its verbs as
//! `act.console` — `execute_console_line`, the funnel the prompt and a
//! macro line reach — on `dispatch/families/fixtures/paths.mandala.json`.
//! The per-form expectations are `dispatch::families::field_paths`'s own
//! tests; these hold the two funnels, one entry each, and the reason a
//! refusal gives.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use serde_json::{json, Value};

use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use crate::application::document::MindMapDocument;
use crate::application::fields::path::LINK_KIND_ON_A_CONNECTOR;
use crate::application::ipc::run_attached;
use crate::application::view::inspect::children_of_kind;

/// The fixture.
fn fixture() -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/paths.mandala.json",
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

/// Select the elements of `kind` under nodes `idents`, the first of
/// each.
fn select(init: &mut InitState, kind: DocKind, idents: &[u32]) {
    let doc = init.document.as_mut().expect("a document");
    let ids: Vec<NodeId> = idents
        .iter()
        .map(|&ident| {
            let node = doc.doc().node_by_ident(NodeIdent(ident)).expect("the node");
            children_of_kind(doc.doc(), node, kind).next().expect("the path")
        })
        .collect();
    doc.selection.set(ids);
}

/// **The path family through `act.action` and `act.console`, one entry
/// each, and a connector's link kind refused with its reason.** A
/// content for three links by Action is one entry, and so is a path
/// line by console; `Undo` takes each back to the fixture's bytes.
/// `SetLinkKind` on a connector answers `unhandled` with the reason the
/// status line shows, and changes nothing.
///
/// Controls: the same `SetLinkKind` on a link is handled; each edit does
/// change the bytes before its undo.
#[test]
fn test_the_path_family_runs_through_both_funnels() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let fixture_bytes = std::fs::read_to_string(fixture()).expect("the fixture reads");
    assert_eq!(bytes(init), fixture_bytes, "precondition: the saver's spelling");

    let before = entries(init);
    select(init, DocKind::Link, &[1, 3, 4]);
    assert_eq!(act(init, json!({"SetPathContent": "+"})).0, "handled");
    assert_eq!(entries(init), before + 1, "three links, one entry");
    assert_eq!(
        bytes(init).matches(r#""content": "+""#).count(),
        3,
        "each of the three"
    );
    assert_eq!(act(init, json!("Undo")).0, "handled");
    assert_eq!(bytes(init), fixture_bytes);

    select(init, DocKind::Connector, &[2]);
    let reply = console(init, "path content=· straight");
    assert_eq!(reply["ok"], false, "a subverb and a field are two forms: {reply}");
    let reply = console(init, "path content=·");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(entries(init), before + 1, "a console line is one entry");
    assert_ne!(bytes(init), fixture_bytes, "control: it changed the map");
    assert_eq!(act(init, json!("Undo")).0, "handled");
    assert_eq!(bytes(init), fixture_bytes);

    let (outcome, reason) = act(init, json!({"SetLinkKind": "portal"}));
    assert_eq!(
        (outcome.as_str(), reason),
        ("unhandled", json!(LINK_KIND_ON_A_CONNECTOR)),
        "a connector is always a line"
    );
    assert_eq!(bytes(init), fixture_bytes, "nothing changed");
    assert_eq!(entries(init), before);

    select(init, DocKind::Link, &[1]);
    assert_eq!(
        act(init, json!({"SetLinkKind": "portal"})).0,
        "handled",
        "control: a link takes it"
    );
    assert_eq!(entries(init), before + 1);
}
