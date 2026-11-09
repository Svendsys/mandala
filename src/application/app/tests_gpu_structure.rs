// SPDX-License-Identifier: MPL-2.0

//! The structure family through the real app (V1-C4, #245; TEST_CONVENTIONS
//! §T8): every Action of `dispatch::families::structure`, sent as IPC
//! `act.action` — which is `dispatch_action`, the funnel a key, a macro
//! step and a click reach — on the small map
//! `dispatch/families/fixtures/structure.mandala.json`.
//!
//! For each Action the document after it is compared, as the bytes a save
//! writes, with an expected file beside the fixture. Each expected file
//! is the fixture edited by hand to what the Action means — a node's
//! block moved, a key removed — not the output of a run, so a body that
//! did something else cannot pass by agreeing with itself (§T13 rule 2).
//! Each Action, whatever its selection holds — five nodes, three sections
//! — makes exactly one history entry, and `Undo` gives back the fixture's
//! bytes.
//!
//! The map:
//!
//! ```text
//! 1 "A"
//! ├─ 2 "one" / "two parts" (styled) / "three", connector colored with label "via"
//! │  └─ 4 "D"
//! └─ 3 "C"
//! 5 "E" ─ links to 4 and 3
//! 6 "F"
//! ```

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use serde_json::{json, Value};

use super::dispatch::families::structure::{CONNECTOR_STATUS, INVALID_TARGET_STATUS};
use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use super::InteractionMode;
use crate::application::document::MindMapDocument;
use crate::application::ipc::run_attached;
use crate::application::view::inspect::children_of_kind;

/// A file under the fixtures directory.
fn fixture(name: &str) -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/{name}.mandala.json",
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

/// What a save writes.
fn bytes(init: &InitState) -> String {
    format::save(document(init).doc()).expect("the document saves")
}

/// How many entries an undo can reach.
fn entries(init: &InitState) -> usize {
    document(init).doc().history().count()
}

/// One frame's input-driven half: the drain that syncs the view.
fn frame(init: &mut InitState) {
    init.input_context().drain_frame(Beat::Loop);
}

/// Run one IPC request, through the real decoder and executor.
fn ipc(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// `act.action` with `action` — a unit Action's name, or a payload
/// Action's `{"Variant": payload}` — and its outcome.
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

/// Select `ids`, as the selection's own clicks would.
fn select(init: &mut InitState, ids: Vec<NodeId>) {
    document_mut(init).selection.set(ids);
}

/// One step of a case: change the selection, or send an Action — with
/// the pointer at a canvas point, for `ActAt`.
enum Step {
    Select(Vec<NodeId>),
    Act(Value),
    ActAt(Value, f32, f32),
}

/// Run `steps` on the fixture as it was loaded, then hold the result to
/// `expected`'s bytes and the history to one entry more, run `after` on
/// the result, and hold `Undo` to the fixture's bytes. Every
/// `act.action` is handled.
fn check(init: &mut InitState, expected: &str, steps: Vec<Step>, after: impl FnOnce(&InitState)) {
    let baseline = bytes(init);
    let before = entries(init);
    for step in steps {
        let (request, action) = match step {
            Step::Select(ids) => {
                select(init, ids);
                continue;
            }
            Step::Act(action) => (json!({"id": 1, "cmd": "act.action", "action": action}), action),
            Step::ActAt(action, x, y) => (
                json!({"id": 1, "cmd": "act.action", "action": action, "x": x, "y": y}),
                action,
            ),
        };
        let reply = ipc(init, request);
        assert_eq!(
            reply["result"]["outcome"], "handled",
            "{expected}: {action}: {reply}"
        );
    }
    frame(init);
    let want = std::fs::read_to_string(fixture(expected)).expect("the expected file reads");
    assert_eq!(bytes(init), want, "{expected}: the saved document");
    assert_eq!(entries(init), before + 1, "{expected}: exactly one history entry");
    after(init);
    assert_eq!(act(init, json!("Undo")), "handled");
    assert_eq!(
        bytes(init),
        baseline,
        "{expected}: undo gives back the fixture's bytes"
    );
    frame(init);
}

/// The selection, in order.
fn selection(init: &InitState) -> Vec<NodeId> {
    document(init).selection.iter().collect()
}

/// How many connectors `node` has.
fn connectors(init: &InitState, node: NodeId) -> usize {
    children_of_kind(document(init).doc(), node, DocKind::Connector).count()
}

/// **Every structure Action, through `act.action`, makes the document its
/// expected file says, as one history entry, and `Undo` restores it.**
///
/// Beside the bytes: the selection names what each Action leaves for the
/// user — the new node, the moved nodes, the new links — and the two
/// connector rules a saved file cannot show for a connector with nothing
/// set: a root moved under a node gains one, and an orphaned node loses
/// its own (with its color and label, which the file does show).
///
/// Precondition: the fixture is in the saver's spelling, so every
/// comparison is with bytes the saver could have written.
#[test]
fn test_every_structure_action_makes_its_expected_document_in_one_entry() {
    let Some(mut app) = headless_app(&fixture("structure")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let fixture_bytes = std::fs::read_to_string(fixture("structure")).expect("the fixture reads");
    assert_eq!(
        bytes(init),
        fixture_bytes,
        "precondition: the fixture is in the saver's spelling"
    );
    let (n1, n2, n3, n5, n6) = (
        node(init, 1),
        node(init, 2),
        node(init, 3),
        node(init, 5),
        node(init, 6),
    );
    let n4 = node(init, 4);

    // A new root where the pointer dispatch landed, with one empty
    // section, selected.
    check(
        init,
        "create",
        vec![Step::ActAt(json!("CreateOrphanNode"), 100.0, 800.0)],
        |init| assert_eq!(selection(init), vec![node(init, 7)], "the new node is selected"),
    );

    // A node with its subtree, and the link into the subtree from a node
    // that stays; the selection keeps nothing that went.
    check(
        init,
        "delete",
        vec![Step::Select(vec![n2]), Step::Act(json!("DeleteSelection"))],
        |init| assert!(selection(init).is_empty()),
    );
    // Five nodes, one nested under another: still one entry.
    check(
        init,
        "delete_five",
        vec![
            Step::Select(vec![n2, n4, n3, n5, n6]),
            Step::Act(json!("DeleteSelection")),
        ],
        |_| {},
    );
    // A connector with its label selected: the label goes, the connector
    // stays.
    let connector = child(init, n2, DocKind::Connector, 0);
    let label = child(init, connector, DocKind::Label, 0);
    check(
        init,
        "delete_label",
        vec![
            Step::Select(vec![connector, label]),
            Step::Act(json!("DeleteSelection")),
        ],
        |init| assert_eq!(selection(init), vec![connector]),
    );

    // Two nodes become roots where they are: 2 loses its connector.
    assert_eq!(connectors(init, n2), 1, "precondition: 2 has its connector");
    check(
        init,
        "orphan",
        vec![Step::Select(vec![n2, n3]), Step::Act(json!("OrphanSelection"))],
        |init| {
            assert_eq!(connectors(init, n2), 0, "the orphan has no connector");
            assert_eq!(selection(init), vec![n2, n3], "the orphans are selected");
        },
    );

    // A root under a node, through the mode: it gains a connector.
    check(
        init,
        "reparent",
        vec![
            Step::Select(vec![n6]),
            Step::Act(json!("EnterReparentMode")),
            Step::Act(json!({"ReparentToTarget": 3})),
        ],
        |init| {
            assert_eq!(
                init.interaction_mode,
                InteractionMode::Default,
                "the click left the mode"
            );
            assert_eq!(connectors(init, n6), 1, "the moved root has a connector");
            assert_eq!(selection(init), vec![n6]);
        },
    );
    assert_eq!(connectors(init, n6), 0, "and its undo takes it away");

    // Two nodes linked to a third, through the mode; the links are
    // selected.
    check(
        init,
        "connect",
        vec![
            Step::Select(vec![n5, n6]),
            Step::Act(json!("EnterConnectMode")),
            Step::Act(json!({"ConnectToTarget": 1})),
        ],
        |init| {
            let doc = document(init).doc();
            let new: Vec<NodeId> = [n5, n6]
                .into_iter()
                .map(|n| children_of_kind(doc, n, DocKind::Link).last().expect("a link"))
                .collect();
            assert_eq!(selection(init), new, "the new links are selected");
        },
    );

    // A section's spans and style onto three sections, two of one node
    // and one of another: one entry.
    let (one, two, three) = (
        child(init, n2, DocKind::Section, 0),
        child(init, n2, DocKind::Section, 1),
        child(init, n2, DocKind::Section, 2),
    );
    let a = child(init, n1, DocKind::Section, 0);
    check(
        init,
        "paste_sections",
        vec![
            Step::Select(vec![two]),
            Step::Act(json!("Copy")),
            Step::Select(vec![a, one, three]),
            Step::Act(json!("Paste")),
        ],
        |init| assert_eq!(selection(init), vec![a, one, three], "the targets stay selected"),
    );

    // A node's text out: its section is emptied, and the clipboard holds
    // the text.
    check(
        init,
        "cut_node",
        vec![Step::Select(vec![n3]), Step::Act(json!("Cut"))],
        |_| {},
    );
    let held = document(init).clipboard.as_ref().map(|c| c.text.clone());
    assert_eq!(held.as_deref(), Some("C"));

    // A connector's color onto a link.
    let link = child(init, n5, DocKind::Link, 1);
    check(
        init,
        "paste_color",
        vec![
            Step::Select(vec![connector]),
            Step::Act(json!("Copy")),
            Step::Select(vec![link]),
            Step::Act(json!("Paste")),
        ],
        |init| assert_eq!(selection(init), vec![link]),
    );
}

/// **Reparent mode does not land on a source's own subtree.** With 2
/// selected, the pointer over its child 4 tints nothing and over 2 itself
/// nothing, and a click on 4 is refused: no entry, the document as it
/// was, the refusal on the status line and the mode left.
///
/// Negative control: over 2's sibling 3 the tint is 3, and the click on 3
/// is accepted, as one entry that puts 2 under 3.
#[test]
fn test_reparent_mode_refuses_a_descendant_and_takes_a_sibling() {
    let Some(mut app) = headless_app(&fixture("structure")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let (n2, n3, n4) = (node(init, 2), node(init, 3), node(init, 4));
    let baseline = bytes(init);
    select(init, vec![n2]);
    assert_eq!(act(init, json!("EnterReparentMode")), "handled");

    let over = |init: &mut InitState, target: NodeId| {
        let rect = init
            .view
            .as_ref()
            .and_then(|view| view.projector().node_rect(target))
            .expect("the node is drawn");
        let c = rect.center();
        let reply = ipc(
            init,
            json!({"id": 2, "cmd": "input.mouse_move", "x": c.x, "y": c.y, "space": "canvas"}),
        );
        assert_eq!(reply["result"]["applied"], true, "{reply}");
        frame(init);
        init.view.as_ref().expect("a view").hover_target()
    };
    assert_eq!(over(init, n4), None, "a descendant is not a target");
    assert_eq!(over(init, n2), None, "nor is the source itself");
    assert_eq!(over(init, n3), Some(n3), "control: the sibling is");

    assert_eq!(act(init, json!({"ReparentToTarget": 4})), "unhandled");
    assert_eq!(bytes(init), baseline, "the refused reparent changed nothing");
    assert_eq!(entries(init), 0, "and made no entry");
    assert_eq!(document(init).status(), Some(INVALID_TARGET_STATUS));
    assert_eq!(init.interaction_mode, InteractionMode::Default);

    // The click itself, through the pointer: the release dispatches
    // `ReparentToTarget` with the node under it.
    assert_eq!(act(init, json!("EnterReparentMode")), "handled");
    let at = init
        .view
        .as_ref()
        .and_then(|view| view.projector().node_rect(n3))
        .expect("3 is drawn")
        .center();
    let reply = ipc(
        init,
        json!({"id": 3, "cmd": "input.mouse_button", "button": "left", "state": "click", "x": at.x, "y": at.y, "space": "canvas"}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    frame(init);
    assert_eq!(entries(init), 1);
    assert_eq!(
        document(init).doc().parent(n2),
        Some(n3),
        "2 is under its sibling"
    );
    assert_eq!(
        init.view.as_ref().expect("a view").hover_target(),
        None,
        "the tint left with the mode"
    );
}

/// **Connect adds no second link of the same kind and direction.** 5
/// already has a line link to 4: connecting 5 to 4 makes no entry and
/// changes nothing. Control: connecting 5 to 6 makes one entry.
///
/// **A connector alone is not deleted:** the delete makes no entry and
/// says why on the status line (the control is the label case of
/// `test_every_structure_action_makes_its_expected_document_in_one_entry`,
/// where the label goes).
#[test]
fn test_connect_does_not_duplicate_and_a_connector_is_not_deleted() {
    let Some(mut app) = headless_app(&fixture("structure")) else {
        return;
    };
    let init = &mut app.init;
    frame(init);
    let (n2, n5) = (node(init, 2), node(init, 5));
    let baseline = bytes(init);

    select(init, vec![n5]);
    assert_eq!(act(init, json!("EnterConnectMode")), "handled");
    assert_eq!(act(init, json!({"ConnectToTarget": 4})), "unhandled");
    assert_eq!(bytes(init), baseline, "no second link");
    assert_eq!(entries(init), 0);
    assert_eq!(act(init, json!("EnterConnectMode")), "handled");
    assert_eq!(act(init, json!({"ConnectToTarget": 6})), "handled");
    assert_eq!(entries(init), 1, "control: a new pair is linked");

    let connector = child(init, n2, DocKind::Connector, 0);
    let before = bytes(init);
    select(init, vec![connector]);
    assert_eq!(act(init, json!("DeleteSelection")), "unhandled");
    assert_eq!(bytes(init), before);
    assert_eq!(entries(init), 1, "the delete made no entry");
    assert_eq!(document(init).status(), Some(CONNECTOR_STATUS));
}
