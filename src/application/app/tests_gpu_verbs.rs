// SPDX-License-Identifier: MPL-2.0

//! Map verbs end to end in the real app (V1-D3, #252; TEST_CONVENTIONS
//! §T8): the sequential reveal of `fixtures/reveal.mandala.json` run
//! from the console and over IPC on a stepped clock, with the nodes
//! drawn and hidden as each step leaves them; a hover held by injected
//! pointer moves; and `act.verb`'s refusals, each before anything runs.

use serde_json::{json, Value};

use baumhard::document::element::DocElement;
use baumhard::document::ids::NodeIdent;

use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use crate::application::ipc::run_attached;

fn fixture(name: &str) -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/{name}.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The reply to `request`, run as an attached IPC client runs it.
fn reply(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// The result of `request`, which must succeed.
fn result(init: &mut InitState, request: Value) -> Value {
    let answer = reply(init, request.clone());
    assert_eq!(answer["ok"], true, "{request}: {answer}");
    answer["result"].clone()
}

/// The hidden flag and the opacity of the node `ident` as the document
/// holds them now — under a presentation, what is drawn — and whether
/// the projection draws it.
fn shown(init: &InitState, ident: u32) -> (bool, f32, bool) {
    let doc = init.document.as_ref().expect("a document").doc();
    let node = doc.node_by_ident(NodeIdent(ident)).expect("a node");
    let (hidden, opacity) = match doc.get(node) {
        Some(DocElement::Node(data)) => (data.hidden, data.opacity),
        other => panic!("{other:?}"),
    };
    let drawn = !init
        .view
        .as_ref()
        .expect("a view")
        .projector()
        .glyphs_of(node)
        .is_empty();
    (hidden, opacity, drawn)
}

/// **A sequential reveal, end to end** (#252 "Tests"): `verb run begin`
/// conceals the three days — hidden, at opacity 0, not drawn — and
/// `next-1` and `next-2`, one from the console and one over `act.verb`,
/// each bring one back: drawn at once, its opacity rising over the
/// 400 ms of the presentation's return to 1, the days not revealed
/// still concealed. The saved map shows every node all along, and
/// nothing of it reaches the history.
///
/// Control, seen red: a `stop` step that stops nothing leaves day one
/// hidden after `next-1`.
#[test]
fn test_a_sequential_reveal_runs_end_to_end() {
    let Some(mut app) = headless_app(&fixture("reveal")) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let saved = init
        .document
        .as_mut()
        .expect("a document")
        .save_json()
        .expect("saves");
    for day in [2, 3, 4] {
        assert_eq!(
            shown(init, day),
            (false, 1.0, true),
            "precondition: day {day} is drawn"
        );
    }
    let console =
        |init: &mut InitState, line: &str| result(init, json!({"id": 1, "cmd": "act.console", "line": line}));
    let listed = console(init, "verb list");
    assert_eq!(
        listed["output"],
        json!([
            "begin  Begin  (performance)",
            "next-1  The first day  (performance)",
            "next-2  The second day  (performance)",
            "next-3  The third day  (performance)",
            "running: none"
        ])
    );
    console(init, "verb run begin");
    init.input_context().drain_frame(Beat::Loop);
    for day in [2, 3, 4] {
        assert_eq!(shown(init, day), (true, 0.0, false), "day {day} is concealed");
    }
    assert_eq!(
        shown(init, 1),
        (false, 1.0, true),
        "the title is not on channel 1"
    );

    console(init, "verb run next-1");
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 200}));
    let (hidden, opacity, drawn) = shown(init, 2);
    assert!(!hidden && drawn, "day one is shown as soon as its return begins");
    assert!(
        opacity > 0.0 && opacity < 1.0,
        "half way through its fade: {opacity}"
    );
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 200}));
    assert_eq!(shown(init, 2), (false, 1.0, true), "day one is revealed");
    assert_eq!(shown(init, 3), (true, 0.0, false));

    let ran = result(init, json!({"id": 1, "cmd": "act.verb", "verb": "next-2"}));
    assert_eq!(ran, json!({"changes_the_map": false}));
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 400}));
    assert_eq!(shown(init, 3), (false, 1.0, true), "day two is revealed");
    assert_eq!(shown(init, 4), (true, 0.0, false), "day three is still concealed");

    let doc = init.document.as_mut().expect("a document");
    assert_eq!(doc.save_json().expect("saves"), saved, "the map saves as it was");
    assert_eq!(
        doc.doc().history().count(),
        0,
        "a performance is not in the history"
    );
}

/// **`act.verb` refuses before anything runs**: a verb the map does not
/// define and a node it does not have are `not_found`, a verb that acts
/// on `self` sent with no `node` is `invalid_params` naming `node`, and
/// a `node` that is not a node id is `invalid_params` too. The same verb
/// with a `node` runs, and says it changes nothing of the map.
///
/// Control, seen red: without the handler's check of the node, node 99
/// reaches the runner and is answered `internal`, not `not_found`.
#[test]
fn test_act_verb_refuses_before_anything_runs() {
    let Some(mut app) = headless_app(&fixture("reveal")) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let refused = |init: &mut InitState, request: Value| {
        let answer = reply(init, request.clone());
        assert_eq!(answer["ok"], false, "{request}: {answer}");
        (
            answer["error"]["code"].as_str().unwrap_or_default().to_string(),
            answer["error"]["data"]["param"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    };
    assert_eq!(
        refused(init, json!({"id": 1, "cmd": "act.verb", "verb": "nope"})).0,
        "not_found"
    );
    assert_eq!(
        refused(
            init,
            json!({"id": 1, "cmd": "act.verb", "verb": "next-1", "node": 99})
        )
        .0,
        "not_found"
    );
    assert_eq!(
        refused(
            init,
            json!({"id": 1, "cmd": "act.verb", "verb": "next-1", "node": -1})
        ),
        ("invalid_params".to_string(), "node".to_string())
    );
    assert_eq!(
        refused(init, json!({"id": 1, "cmd": "act.verb"})),
        ("invalid_params".to_string(), "verb".to_string())
    );
    let doc = init.document.as_ref().expect("a document");
    assert_eq!(doc.doc().presentations().count(), 0, "nothing ran");
    assert_eq!(
        result(
            init,
            json!({"id": 1, "cmd": "act.verb", "verb": "next-1", "node": 1})
        ),
        json!({"changes_the_map": false})
    );
}

/// **A verb that acts on `self`, over IPC with no node**, is refused
/// naming `node`; with one it runs. The click demo map's verb is one.
///
/// Control, seen red: without the handler's `needs_self` check the
/// runner refuses it, answered `internal`, not `invalid_params`.
#[test]
fn test_act_verb_needs_the_node_a_verb_acts_on() {
    let path = format!("{}/maps/demos/frames.mandala.json", env!("CARGO_MANIFEST_DIR"));
    let Some(mut app) = headless_app(&path) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let answer = reply(
        init,
        json!({"id": 1, "cmd": "act.verb", "verb": "swell-the-ellipse"}),
    );
    assert_eq!(answer["error"]["code"], "invalid_params", "{answer}");
    assert_eq!(answer["error"]["data"]["param"], "node", "{answer}");
    assert_eq!(
        init.document
            .as_ref()
            .expect("a document")
            .doc()
            .presentations()
            .count(),
        0
    );
    result(
        init,
        json!({"id": 1, "cmd": "act.verb", "verb": "swell-the-ellipse", "node": 2}),
    );
    let playing = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(
        playing["presentations"][0]["presentation"], "an-ellipse",
        "{playing}"
    );
    assert_eq!(playing["presentations"][0]["node"], 2, "{playing}");
}

/// **A hover is held by the pointer's own moves**: injected
/// `CursorMoved`s onto node 1 — through the cursor-move handler and its
/// one hit test (`Projector::hit`) — invoke its `on.hover` verb held,
/// so its `while` presentation goes in; moved off onto empty canvas,
/// the hold is released and the presentation plays its way out; and
/// the hand cursor shows over the node whose click runs a verb, and not
/// off it.
///
/// Control, seen red: `follow_pointer` not calling `Host::hover` leaves
/// nothing playing.
#[test]
fn test_the_pointer_holds_a_hover_and_releases_it() {
    let dir = baumhard::util::test_temp::TempDir::new("verb-hover");
    let path = dir.path().join("hover.mandala.json");
    std::fs::write(
        &path,
        r#"{"format":1,"name":"hover",
        "presentations":[{"id":"p","name":"P","play":"while","target":"node","mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":100}}],
        "verbs":[{"id":"lift","name":"Lift","steps":[{"present":"p","on":"self"}]}],
        "nodes":[{"id":1,"position":[0,0],"min_size":[200,100],"on":{"hover":"lift","click":"lift"},"sections":[{"spans":[{"text":"hover me"}]}]}]}"#,
    )
    .expect("writes");
    let Some(mut app) = headless_app(path.to_str().expect("UTF-8")) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let move_to = |init: &mut InitState, x: f64, y: f64| {
        result(
            init,
            json!({"id": 1, "cmd": "input.mouse_move", "x": x, "y": y, "space": "canvas"}),
        );
        init.input_context().drain_frame(Beat::Loop);
    };
    let playing = |init: &mut InitState| {
        let animations = result(init, json!({"id": 1, "cmd": "query.animations"}));
        animations["presentations"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|instance| instance["direction"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
    };
    move_to(init, -500.0, -500.0);
    assert!(
        playing(init).is_empty() && !init.cursor_is_hand,
        "precondition: off the node"
    );
    move_to(init, 100.0, 50.0);
    assert_eq!(playing(init), ["in"], "entering holds the verb");
    assert!(
        init.cursor_is_hand,
        "the hand over a node whose click runs a verb"
    );
    let held = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(held["verbs"][0]["held"], true, "{held}");
    move_to(init, 110.0, 55.0);
    assert_eq!(playing(init), ["in"], "moving within it changes nothing");
    move_to(init, -500.0, -500.0);
    assert_eq!(
        playing(init),
        ["out"],
        "leaving releases it: it plays its way out"
    );
    assert!(!init.cursor_is_hand);
    let released = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(released["verbs"], json!([]), "{released}");
}

/// **Leaving the window releases a held hover** (#380 review; ruling
/// D3.2 item 4): the pointer held on node 1, then `WindowEvent::CursorLeft`
/// delivered to the handler its arm calls — no move off the node first,
/// as when a node at the canvas's edge is left past it — releases the
/// run, and its `while` presentation plays its way out. Leaving again
/// changes nothing.
///
/// Control, seen red: `handle_cursor_left` returning before
/// `Host::hover` leaves the run held and the presentation in.
#[test]
fn test_the_pointer_leaving_the_window_releases_a_hover() {
    let dir = baumhard::util::test_temp::TempDir::new("verb-hover-left");
    let path = dir.path().join("hover.mandala.json");
    std::fs::write(
        &path,
        r#"{"format":1,"name":"hover",
        "presentations":[{"id":"p","name":"P","play":"while","target":"node","mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":100}}],
        "verbs":[{"id":"lift","name":"Lift","steps":[{"present":"p","on":"self"}]}],
        "nodes":[{"id":1,"position":[0,0],"min_size":[200,100],"on":{"hover":"lift"},"sections":[{"spans":[{"text":"hover me"}]}]}]}"#,
    )
    .expect("writes");
    let Some(mut app) = headless_app(path.to_str().expect("UTF-8")) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    result(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": 100.0, "y": 50.0, "space": "canvas"}),
    );
    init.input_context().drain_frame(Beat::Loop);
    let animations = |init: &mut InitState| result(init, json!({"id": 1, "cmd": "query.animations"}));
    let held = animations(init);
    assert_eq!(held["verbs"][0]["held"], true, "precondition: held: {held}");
    assert_eq!(held["presentations"][0]["direction"], "in", "{held}");
    assert!(
        init.input_context().on_cursor_left(),
        "the hover changed: a redraw"
    );
    init.input_context().drain_frame(Beat::Loop);
    let left = animations(init);
    assert_eq!(left["verbs"], json!([]), "released: {left}");
    assert_eq!(left["presentations"][0]["direction"], "out", "{left}");
    assert!(!init.input_context().on_cursor_left(), "nothing left to release");
}

/// **A leave during a press releases the hover when the press ends**
/// (ruling D3.3 item 4): the pointer held on node 1 presses there, then
/// leaves the window while the press is down — which changes nothing, as
/// a move during a gesture leaves the hover — and moves on outside it, as
/// a captured drag reports (winit on Windows sends a `CursorMoved`
/// outside with its `CursorLeft`; ruling D3.4 item 3); and the release
/// releases the run, its `while` presentation playing its way out.
///
/// Controls, each seen red: the release not deciding the hover again
/// (`on_mouse_input` without `release_hover_if_outside`) leaves the run
/// held; a move setting `pointer_inside` whatever its position, as it did
/// at fed68afd, leaves it held too.
#[test]
fn test_a_leave_during_a_press_releases_the_hover_at_the_release() {
    use crate::application::platform::input::{ElementState, MouseButton};
    let dir = baumhard::util::test_temp::TempDir::new("verb-hover-press-left");
    let path = dir.path().join("hover.mandala.json");
    std::fs::write(
        &path,
        r#"{"format":1,"name":"hover",
        "presentations":[{"id":"p","name":"P","play":"while","target":"node","mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":100}}],
        "verbs":[{"id":"lift","name":"Lift","steps":[{"present":"p","on":"self"}]}],
        "nodes":[{"id":1,"position":[0,0],"min_size":[200,100],"on":{"hover":"lift"},"sections":[{"spans":[{"text":"hover me"}]}]}]}"#,
    )
    .expect("writes");
    let Some(mut app) = headless_app(path.to_str().expect("UTF-8")) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    result(
        init,
        json!({"id": 1, "cmd": "input.mouse_move", "x": 100.0, "y": 50.0, "space": "canvas"}),
    );
    init.input_context().drain_frame(Beat::Loop);
    let animations = |init: &mut InitState| result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(animations(init)["verbs"][0]["held"], true, "precondition: held");
    init.input_context()
        .on_mouse_input(ElementState::Pressed, MouseButton::Left);
    assert!(
        !init.input_context().on_cursor_left(),
        "a leave during the press changes nothing"
    );
    init.input_context()
        .on_cursor_moved(crate::application::platform::window::PhysicalPosition::new(
            -400.0, -300.0,
        ));
    assert!(!init.pointer_inside, "a move outside the window is a pointer out");
    init.input_context().drain_frame(Beat::Loop);
    let pressed = animations(init);
    assert_eq!(pressed["verbs"][0]["held"], true, "still held: {pressed}");
    init.input_context()
        .on_mouse_input(ElementState::Released, MouseButton::Left);
    init.input_context().drain_frame(Beat::Loop);
    let released = animations(init);
    assert_eq!(
        released["verbs"],
        json!([]),
        "released at the release: {released}"
    );
    assert_eq!(released["presentations"][0]["direction"], "out", "{released}");
}
