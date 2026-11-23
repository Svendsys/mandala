// SPDX-License-Identifier: MPL-2.0

//! Text editing on spans through the real app (V1-C8, #249;
//! TEST_CONVENTIONS §T8): the section editor, the single-line editor and
//! node editing, driven by synthetic input — the IPC `input.*` commands,
//! which reach the handlers a winit event reaches — on
//! `dispatch/families/fixtures/text.mandala.json`:
//!
//! ```text
//! 1 "Head" "line"(bold) | "Body"; a ring; a line link and a portal to 3
//! └─ 2 "Child"; a ring; connector "─", cap ">", label "child"
//! 3 "Lone"
//! ```
//!
//! Each editor is held to the issue's list: typing 20 characters then
//! committing is one history entry whose saved map is one `replace_spans`
//! apply; cancel leaves the map byte-identical with no entry; undo after
//! commit restores it. Node editing routes a click to a section, and the
//! section's move and resize start from there (#248's ruling C2).

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::writer::EditRequest;
use glam::Vec2;
use serde_json::{json, Value};

use super::gpu_harness::{headless_app, headless_app_with};
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use super::{InteractionMode, ResizeTarget};
use crate::application::document::MindMapDocument;
use crate::application::ipc::run_attached;
use crate::application::keybinds::KeybindConfig;
use crate::application::view::inspect::children_of_kind;

/// The fixture.
fn fixture() -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/text.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn document(init: &InitState) -> &MindMapDocument {
    init.document.as_ref().expect("the harness always builds a document")
}

/// What a save writes: the committed map.
fn bytes(init: &InitState) -> String {
    format::save(document(init).doc()).expect("the document saves")
}

/// How many entries an undo can reach.
fn entries(init: &InitState) -> usize {
    document(init).doc().history().count()
}

/// One frame: an injected sample, the drains and the view's sync.
fn frame(init: &mut InitState) {
    init.input_context().drain_frame(Beat::Loop);
}

/// Run one IPC request through the real decoder and executor.
fn ipc(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// Send `request`, an `input.*` command, and hold that it was applied.
fn input(init: &mut InitState, request: Value) {
    let reply = ipc(init, request);
    assert_eq!(reply["result"]["applied"], true, "{reply}");
    frame(init);
}

/// Press and release the left button at `at` on the canvas.
fn click(init: &mut InitState, at: Vec2) {
    for state in ["press", "release"] {
        input(
            init,
            json!({"id": 1, "cmd": "input.mouse_button", "button": "left", "state": state,
                   "x": at.x, "y": at.y, "space": "canvas"}),
        );
    }
}

/// Two clicks at `at`: a double-click, on the stopped clock of a headless
/// app.
fn double_click(init: &mut InitState, at: Vec2) {
    click(init, at);
    click(init, at);
}

/// Type `text`, one key per grapheme.
fn type_text(init: &mut InitState, text: &str) {
    input(init, json!({"id": 1, "cmd": "input.text", "text": text}));
}

/// Tap `key`.
fn tap(init: &mut InitState, key: &str) {
    input(init, json!({"id": 1, "cmd": "input.key", "key": key}));
}

/// `act.action` with `action`, and its outcome.
fn act(init: &mut InitState, action: Value) -> String {
    let reply = ipc(init, json!({"id": 1, "cmd": "act.action", "action": action}));
    frame(init);
    reply["result"]["outcome"]
        .as_str()
        .unwrap_or_else(|| panic!("act.action answers: {reply}"))
        .to_string()
}

/// Node `ident`.
fn node(init: &InitState, ident: u32) -> NodeId {
    document(init).doc().node_by_ident(NodeIdent(ident)).expect("in the fixture")
}

/// The `index`th child of `parent` of `kind`.
fn child(init: &InitState, parent: NodeId, kind: DocKind, index: usize) -> NodeId {
    children_of_kind(document(init).doc(), parent, kind)
        .nth(index)
        .expect("in the fixture")
}

/// The center of `section`'s drawn box.
fn section_center(init: &InitState, section: NodeId) -> Vec2 {
    let view = init.view.as_ref().expect("a view");
    view.projector().section_rect(section).expect("drawn").center()
}

/// A point on empty canvas, far from every node.
const EMPTY: Vec2 = Vec2::new(-2000.0, -2000.0);

/// The map one `replace_spans` apply of `owner`'s spans now makes of the
/// fixture.
fn one_replace_spans(init: &InitState, owner: NodeId) -> String {
    let spans = crate::application::app::text_edit::tests::spans(document(init), owner);
    let mut once = crate::application::app::text_edit::tests::text_fixture();
    let steps = replace_spans(once.doc(), &[owner], &spans).expect("composes");
    once.apply_for_test(EditRequest::new(steps, "once")).expect("applies");
    format::save(once.doc()).expect("saves")
}

/// **The section editor through the keyboard and the pointer**: a
/// double-click on a section opens the editor on it; twenty characters
/// typed draw — the caret moving with them — and make no entry; a click
/// outside commits one, whose saved map is one `replace_spans`; undo
/// restores the map. Escape cancels a second session, byte-identical,
/// with no entry.
#[test]
fn test_the_section_editor_types_commits_and_cancels_through_input() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let section = child(init, node(init, 3), DocKind::Section, 0);

    double_click(init, section_center(init, section));
    assert!(init.text_edit.is_open(), "a double-click on a section opens the editor");
    let caret_before = init.view.as_ref().unwrap().caret().shown().expect("the caret is drawn").0;
    type_text(init, "twenty chars typed!!");
    assert_eq!(entries(init), 0, "typing makes no entry");
    let caret_after = init.view.as_ref().unwrap().caret().shown().expect("the caret is drawn").0;
    assert!(caret_after.x > caret_before.x, "the caret moves with typing");
    assert!(
        crate::application::app::text_edit::tests::spans(document(init), section)
            .iter()
            .map(|s| s.text.as_str())
            .collect::<String>()
            .ends_with("twenty chars typed!!"),
        "the stage is the document"
    );
    click(init, EMPTY);
    assert!(!init.text_edit.is_open(), "a click outside commits");
    assert_eq!(entries(init), 1, "one entry for the session");
    assert_eq!(bytes(init), one_replace_spans(init, section));
    assert_eq!(act(init, json!("Undo")), "handled");
    assert_eq!(bytes(init), baseline, "undo restores the map");

    double_click(init, section_center(init, section));
    assert!(init.text_edit.is_open());
    type_text(init, "gone");
    tap(init, "Escape");
    assert!(!init.text_edit.is_open(), "Escape cancels");
    assert_eq!(bytes(init), baseline, "byte-identical");
    assert_eq!(entries(init), 0, "and no entry");
}

/// **A pan moves no caret**: the camera is applied when the frame is
/// drawn, so a pan and its frame place the caret nowhere new. Control:
/// a key typed places it again.
#[test]
fn test_a_pan_places_no_caret() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let section = child(init, node(init, 3), DocKind::Section, 0);
    double_click(init, section_center(init, section));
    let placed = init.view.as_ref().unwrap().caret().placed();
    assert_eq!(act(init, json!("PanCameraEast")), "handled");
    assert_eq!(init.view.as_ref().unwrap().caret().placed(), placed, "a pan places nothing");
    type_text(init, "x");
    assert!(init.view.as_ref().unwrap().caret().placed() > placed, "control: typing places it");
}

/// **An undo while the editor is open cancels its session**, with
/// nothing written (ruling R36): the editor closes at the next frame.
#[test]
fn test_an_undo_while_editing_closes_the_editor_with_nothing_written() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let section = child(init, node(init, 3), DocKind::Section, 0);
    double_click(init, section_center(init, section));
    type_text(init, "abc");
    act(init, json!("Undo"));
    assert!(!init.text_edit.is_open(), "the editor closed");
    assert_eq!(bytes(init), baseline);
}

/// **The single-line editor on a connector's label**: a double-click on
/// the label edits it (ruling K3: the element hit); typed characters
/// draw and make no entry; Enter commits one, one `replace_spans`; undo
/// restores. Escape cancels a second session byte-identically.
#[test]
fn test_the_label_editor_types_and_commits_through_input() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let connector = child(init, node(init, 2), DocKind::Connector, 0);
    let label = child(init, connector, DocKind::Label, 0);
    // On its first letter, away from its center: the first click selects
    // the connector, whose midpoint handle then sits under a label at
    // `t = 0.5`, and a press on a handle takes no part in a double-click
    // (#218).
    let at = {
        let run = init.view.as_ref().unwrap().projector().laid_label(connector).expect("the label is drawn");
        let (top_left, size) = run.bounds();
        top_left + Vec2::new(size.x * 0.1, size.y * 0.5)
    };
    let center = init.view.as_ref().unwrap().projector().label_pos(label).expect("the label is drawn");
    assert!(
        at.distance(center) > 8.0,
        "precondition: the press is clear of the midpoint handle's reach"
    );

    double_click(init, at);
    assert!(
        init.single_line_edit.session().is_some_and(|s| s.owner() == label),
        "a double-click on the label edits the label"
    );
    type_text(init, "ren");
    assert_eq!(entries(init), 0);
    tap(init, "Enter");
    assert!(!init.single_line_edit.is_open(), "Enter commits");
    assert_eq!(entries(init), 1);
    assert_eq!(bytes(init), one_replace_spans(init, label));
    act(init, json!("Undo"));
    assert_eq!(bytes(init), baseline);

    double_click(init, at);
    type_text(init, "zzz");
    tap(init, "Escape");
    assert_eq!(bytes(init), baseline, "Escape cancels, byte-identical");
    assert_eq!(entries(init), 0);
}

/// **A ring's poem through the console** (`frame ring 0 edit`): the
/// single-line editor opens on the ring, typing stages it, Enter commits
/// one entry. A portal link's label is refused with the way in named
/// (ruling K4), and leaves the status line saying so.
#[test]
fn test_a_ring_edits_from_the_console_and_a_portal_label_is_refused() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let a = node(init, 1);
    let ring = child(init, child(init, a, DocKind::Frame, 0), DocKind::Ring, 0);
    init.document.as_mut().unwrap().selection.select_only(a);
    let reply = ipc(init, json!({"id": 1, "cmd": "act.console", "line": "frame ring 0 edit"}));
    frame(init);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(init.single_line_edit.session().is_some_and(|s| s.owner() == ring));
    type_text(init, " anew");
    tap(init, "Enter");
    assert_eq!(entries(init), 1);
    assert!(bytes(init).contains("a ring of words anew"));

    let portal = children_of_kind(document(init).doc(), a, DocKind::Link)
        .find(|&l| !crate::application::view::handles::is_line_path(document(init).doc(), l))
        .expect("a portal link");
    init.document.as_mut().unwrap().selection.select_only(portal);
    let outcome = act(init, json!("LabelEditOnSelection"));
    assert_eq!(outcome, "unhandled", "refused");
    assert!(!init.single_line_edit.is_open());
    assert_eq!(
        document(init).status(),
        Some(crate::application::app::single_line_edit::LineRefusal::PORTAL_LABEL)
    );
}

/// **`LabelEditOnSelection` on a path with no label inserts one** (ruling
/// K3): typing then Enter is one entry holding the label and its text;
/// Escape leaves no label at all.
#[test]
fn test_label_edit_on_a_path_with_no_label_inserts_one() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let a = node(init, 1);
    let line = children_of_kind(document(init).doc(), a, DocKind::Link)
        .find(|&l| crate::application::view::handles::is_line_path(document(init).doc(), l))
        .expect("a line link");
    let labels = |init: &InitState| children_of_kind(document(init).doc(), line, DocKind::Label).count();
    init.document.as_mut().unwrap().selection.select_only(line);

    assert_eq!(act(init, json!("LabelEditOnSelection")), "handled");
    assert_eq!(labels(init), 1, "the label is there while editing");
    tap(init, "Escape");
    assert_eq!(labels(init), 0, "canceled, it is gone");
    assert_eq!(bytes(init), baseline);

    assert_eq!(act(init, json!("LabelEditOnSelection")), "handled");
    type_text(init, "via");
    tap(init, "Enter");
    assert_eq!((labels(init), entries(init)), (1, 1), "one entry: the label and its text");
}

/// **Node editing** (#248's ruling C2): `EnterNodeEdit` on a node of two
/// sections enters NodeEdit; a click on its second section selects it; a
/// drag of it moves the section within its node, one entry, its offset
/// changed by the drag; `EnterResizeMode` on the selected section shows
/// the section's handles; a click outside the node leaves NodeEdit.
#[test]
fn test_node_edit_routes_clicks_to_sections_and_starts_their_move_and_resize() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let a = node(init, 1);
    let body = child(init, a, DocKind::Section, 1);
    init.document.as_mut().unwrap().selection.select_only(a);
    assert_eq!(act(init, json!("EnterNodeEdit")), "handled");
    assert_eq!(init.interaction_mode, InteractionMode::NodeEdit { node: a });
    assert!(!init.text_edit.is_open(), "a node of two sections waits for a pick");

    let at = section_center(init, body);
    click(init, at);
    assert!(document(init).selection.contains(body), "the click picked the section");
    assert_eq!(init.interaction_mode, InteractionMode::NodeEdit { node: a }, "and stayed in NodeEdit");

    let offset = |init: &InitState| super::live::offset_of(document(init).doc(), body).unwrap_or(Vec2::ZERO);
    let before = offset(init);
    // Past the double-click window, so the press that starts the drag is
    // not the second click of a double-click, which would open the editor.
    ipc(init, json!({"id": 1, "cmd": "clock.step", "ms": 1000}));
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_button", "button": "left", "state": "press", "x": at.x, "y": at.y, "space": "canvas"}),
    );
    for k in 1..=6 {
        let to = at + Vec2::new(4.0, 2.0) * k as f32;
        input(init, json!({"id": 1, "cmd": "input.mouse_move", "x": to.x, "y": to.y, "space": "canvas"}));
    }
    let end = at + Vec2::new(24.0, 12.0);
    input(
        init,
        json!({"id": 1, "cmd": "input.mouse_button", "button": "left", "state": "release", "x": end.x, "y": end.y, "space": "canvas"}),
    );
    assert_eq!(entries(init), 1, "the section move is one entry");
    let moved = offset(init) - before;
    assert!(moved.distance(Vec2::new(24.0, 12.0)) < 1e-3, "moved by the drag: {moved}");

    assert_eq!(act(init, json!("EnterResizeMode")), "handled");
    assert_eq!(
        init.interaction_mode,
        InteractionMode::Resize {
            target: ResizeTarget::Section(body)
        },
        "the selected section's handles"
    );
    use crate::application::view::handles::{Handle, ResizeSide};
    assert!(
        init.view
            .as_ref()
            .unwrap()
            .handle_center(Handle::Section {
                section: body,
                side: ResizeSide::SE
            })
            .is_some(),
        "the section's handles are shown"
    );

    act(init, json!("ExitMode"));
    assert_eq!(act(init, json!("EnterNodeEdit")), "handled", "a selected section enters its node's");
    assert_eq!(init.interaction_mode, InteractionMode::NodeEdit { node: a });
    click(init, EMPTY);
    assert_eq!(init.interaction_mode, InteractionMode::Default, "a click outside leaves NodeEdit");
}

/// **`EnterNodeEdit` on a node of one section opens the editor at once**,
/// and closing it leaves NodeEdit too, as on `main`.
#[test]
fn test_enter_node_edit_on_one_section_opens_the_editor_and_leaves_with_it() {
    let Some(mut app) = headless_app(&fixture()) else { return };
    let init = &mut app.init;
    let lone = node(init, 3);
    init.document.as_mut().unwrap().selection.select_only(lone);
    assert_eq!(act(init, json!("EnterNodeEdit")), "handled");
    assert!(init.text_edit.is_open(), "the editor opened at once");
    assert_eq!(init.interaction_mode, InteractionMode::NodeEdit { node: lone });
    tap(init, "Escape");
    assert_eq!(init.interaction_mode, InteractionMode::Default, "closing it leaves NodeEdit");
}

/// **A double-click on empty canvas creates a node to edit — one
/// transaction — when `CreateOrphanNodeAndEdit` is bound**, as on `main`:
/// typing then a click outside is one entry holding the node and its
/// text; Escape leaves no node. Control: with it unbound, the double-click
/// creates nothing.
#[test]
fn test_create_orphan_on_empty_canvas_is_one_transaction() {
    let nodes = |init: &InitState| crate::application::document::selection::every_node(document(init).doc()).count();
    {
        let Some(mut app) = headless_app(&fixture()) else { return };
        let init = &mut app.init;
        let count = nodes(init);
        double_click(init, EMPTY);
        assert_eq!(nodes(init), count, "control: unbound, nothing is created");
        assert!(!init.text_edit.is_open());
    }
    let config = KeybindConfig {
        create_orphan_node_and_edit: vec!["Ctrl+Shift+n".to_string()],
        ..KeybindConfig::default()
    };
    let Some(mut app) = headless_app_with(&fixture(), config) else { return };
    let init = &mut app.init;
    let baseline = bytes(init);
    let count = nodes(init);
    double_click(init, EMPTY);
    assert!(init.text_edit.session().is_some_and(|s| s.created()), "the orphan's editor is open");
    assert_eq!(nodes(init), count + 1);
    type_text(init, "new");
    click(init, Vec2::new(2000.0, -2000.0));
    assert_eq!(entries(init), 1, "one entry: the node and its text");
    assert_eq!(nodes(init), count + 1);
    act(init, json!("Undo"));
    assert_eq!(bytes(init), baseline);

    double_click(init, EMPTY);
    type_text(init, "never");
    tap(init, "Escape");
    assert_eq!(nodes(init), count, "cancel leaves no node");
    assert_eq!(bytes(init), baseline);
}
