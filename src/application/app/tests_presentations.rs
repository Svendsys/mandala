// SPDX-License-Identifier: MPL-2.0

//! Presentations on the app clock (V1-D2, #251): the frame's tick
//! (`app::animation`) brings every instance to the clock's reading, a
//! virtual clock lands each on exact instants, the loop is owed a frame
//! exactly while one is on its way, and an animated edit runs under
//! them and commits the map alone.
//!
//! The expected progress at each instant is worked out here from the
//! bundled definitions' numbers — `lift` goes in over 150 ms on
//! `1 - (1 - f)²` and returns over 300 ms on `f²(3 - 2f)` — not read
//! from the code that plays them.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::presentation::{parse_presentations, PresentationDef};
use baumhard::document::writer::ChangeSource;
use glam::Vec2;

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::tests_common::testament;
use crate::application::document::MindMapDocument;
use crate::application::edits::EditRegistry;
use crate::application::presentations::PresentationRegistry;

use super::animation;
use super::dispatch::families::edits::{run_edit, Undrawn};

/// A top-level node of the map.
fn a_node(doc: &MindMapDocument) -> NodeId {
    let d = doc.doc();
    d.children(d.root())
        .find(|&child| d.get(child).is_some_and(|e| e.kind() == DocKind::Node))
        .expect("testament has a node")
}

/// The position and the opacity of `node`, as the document holds them
/// now: under a presentation, what is drawn.
fn shown(doc: &MindMapDocument, node: NodeId) -> (Vec2, f32) {
    match doc.doc().get(node) {
        Some(DocElement::Node(data)) => (data.position, data.opacity),
        other => panic!("{other:?}"),
    }
}

/// The bundled presentation `id`.
fn bundled(id: &str) -> PresentationDef {
    let registry = PresentationRegistry::bundled();
    let doc = testament();
    registry
        .find(doc.doc(), id)
        .unwrap_or_else(|| panic!("`{id}` is bundled"))
        .0
        .clone()
}

/// A clock that moves only when the test moves it, and its reading.
fn stopped_clock() -> (AppClock, f64) {
    let mut clock = AppClock::default();
    clock.set_mode(ClockMode::Virtual);
    let start = clock.now_ms();
    (clock, start)
}

/// A clock that moves by itself: what a window's frame loop reads.
fn running_clock() -> AppClock {
    AppClock::default()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// `lift` goes in on its curve, holds, and returns on the other from
/// where it is, each instant read off the app clock by the frame's tick;
/// the loop is owed a frame while it moves and none while it holds; and
/// the map never changes.
///
/// Controls, each seen red: a tick that does not bring the
/// presentations to the clock leaves the node where it was at the first
/// row; `keeps_ticking` reading only the animated edit says no frame is
/// owed mid-way.
#[test]
fn test_a_while_goes_in_holds_and_returns_on_the_app_clock() {
    let mut doc = testament();
    let node = a_node(&doc);
    let before = doc.save_json().expect("saves");
    let (home, _) = shown(&doc, node);
    let lift = bundled("lift");
    let (mut clock, start) = stopped_clock();
    doc.start_presentation(&lift, &[node], start).expect("starts");
    assert!(doc.animations_moving() && !doc.presentation_loops());

    // In: 150 ms on 1 - (1 - f)².
    for (ms, t) in [(37.5, 0.4375_f32), (75.0, 0.75), (112.5, 0.9375)] {
        clock.advance_to(start + ms);
        assert!(
            animation::tick(Some(&mut doc), &clock),
            "at {ms} ms a frame is drawn"
        );
        let (at, _) = shown(&doc, node);
        assert!(
            close(at.y, home.y - 6.0 * t) && at.x == home.x,
            "at {ms} ms: {at}"
        );
        assert!(
            animation::keeps_ticking(Some(&doc), &running_clock()),
            "on its way at {ms} ms"
        );
        assert!(
            !animation::keeps_ticking(Some(&doc), &clock),
            "a stopped clock keeps no loop awake"
        );
    }
    clock.advance_to(start + 150.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(shown(&doc, node).0, home - Vec2::new(0.0, 6.0));

    // Held: nothing owed, nothing written, however long.
    assert!(!animation::keeps_ticking(Some(&doc), &running_clock()));
    assert!(!doc.animations_moving());
    clock.advance_to(start + 5_000.0);
    assert!(
        !animation::tick(Some(&mut doc), &clock),
        "a held instance writes nothing"
    );
    assert_eq!(doc.doc().presentations().count(), 1, "and plays on");

    // Out, from where it is: 300 ms on f²(3 - 2f), from 1 down.
    let turned = start + 5_000.0;
    assert_eq!(doc.stop_presentation("lift", &[node], turned), 1);
    assert!(animation::keeps_ticking(Some(&doc), &running_clock()));
    for (ms, t) in [(75.0, 0.84375_f32), (150.0, 0.5), (225.0, 0.15625)] {
        clock.advance_to(turned + ms);
        assert!(animation::tick(Some(&mut doc), &clock));
        let (at, _) = shown(&doc, node);
        assert!(close(at.y, home.y - 6.0 * t), "{ms} ms into the return: {at}");
    }
    clock.advance_to(turned + 300.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(shown(&doc, node).0, home);
    assert_eq!(doc.doc().presentations().count(), 0, "returned, it is over");
    assert!(!animation::keeps_ticking(Some(&doc), &running_clock()));

    // Never the map's: no entry, no revision, the same bytes.
    assert_eq!(doc.doc().history().count(), 0);
    assert!(!doc.is_dirty());
    assert_eq!(doc.save_json().expect("saves"), before);
}

/// `pulse` plays in and then out by itself, and is over: `once` needs
/// no stop.
#[test]
fn test_a_once_plays_in_and_out_and_ends_by_itself() {
    let mut doc = testament();
    let node = a_node(&doc);
    let (_, opaque) = shown(&doc, node);
    assert_eq!(opaque, 1.0, "precondition: the node is opaque on the map");
    let (mut clock, start) = stopped_clock();
    doc.start_presentation(&bundled("pulse"), &[node], start)
        .expect("starts");
    // In: 120 ms on 1 - (1 - f)², toward 0.55. Out: 240 ms on f², back.
    for (ms, t) in [
        (60.0, 0.75_f32),
        (120.0, 1.0),
        (120.0 + 120.0, 1.0 - 0.25),
        (120.0 + 180.0, 1.0 - 0.5625),
    ] {
        clock.advance_to(start + ms);
        assert!(animation::tick(Some(&mut doc), &clock));
        let (_, opacity) = shown(&doc, node);
        assert!(close(opacity, 1.0 - 0.45 * t), "at {ms} ms: {opacity}");
        assert!(
            animation::keeps_ticking(Some(&doc), &running_clock()),
            "at {ms} ms"
        );
    }
    clock.advance_to(start + 360.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(shown(&doc, node).1, 1.0);
    assert_eq!(doc.doc().presentations().count(), 0);
    assert!(!animation::keeps_ticking(Some(&doc), &running_clock()));
}

/// A `loop` owes frames for as long as it plays, and is the one play
/// that never comes to rest by itself; a `hold` with no `animate` is at
/// its end when it starts and owes none.
#[test]
fn test_a_loop_owes_frames_until_it_is_stopped_and_a_hold_owes_none() {
    let breathe = parse_presentations(
        r#"{"presentations":[{"id":"breathe","name":"Breathe","play":"loop","target":"node",
        "mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":100}}]}"#,
    )
    .expect("reads")
    .remove(0);
    let mut doc = testament();
    let node = a_node(&doc);
    let (mut clock, start) = stopped_clock();
    doc.start_presentation(&breathe, &[node], start).expect("starts");
    // Ten cycles on, mid-way in again: still playing, still moving.
    clock.advance_to(start + 2_050.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert!(close(shown(&doc, node).1, 0.75), "{}", shown(&doc, node).1);
    assert!(doc.presentation_loops() && doc.animations_moving());
    assert!(animation::keeps_ticking(Some(&doc), &running_clock()));
    // Stopped, it finishes the cycle it is in and is over.
    assert_eq!(doc.stop_presentation("breathe", &[node], start + 2_050.0), 1);
    assert!(!doc.presentation_loops(), "a stopped loop will end");
    assert!(doc.animations_moving());
    clock.advance_to(start + 2_200.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(doc.doc().presentations().count(), 0);
    assert_eq!(shown(&doc, node).1, 1.0);

    // `concealed` holds from the instant it starts.
    doc.start_presentation(&bundled("concealed"), &[node], start + 2_200.0)
        .expect("starts");
    assert_eq!(shown(&doc, node).1, 0.0);
    assert!(!doc.animations_moving() && !doc.presentation_loops());
    assert!(!animation::keeps_ticking(Some(&doc), &running_clock()));
}

/// An animated edit runs under a presentation: each frame shows the
/// edit's progress with the presentation on top, the entry it commits
/// is the edit alone, and an undo leaves the presentation playing.
///
/// Control, seen red: with the layer put back by the tree's stamp alone
/// — which a frame of an animated edit does not move — the node is
/// drawn where the edit began until the edit commits.
#[test]
fn test_an_animated_edit_runs_under_a_presentation_and_commits_the_map_alone() {
    let mut doc = testament();
    let node = a_node(&doc);
    doc.selection.select_only(node);
    let (home, _) = shown(&doc, node);
    let before = doc.save_json().expect("saves");
    let (mut clock, start) = stopped_clock();
    doc.start_presentation(&bundled("lift"), &[node], start)
        .expect("starts");
    clock.advance_to(start + 150.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    let lifted = Vec2::new(0.0, -6.0);
    assert_eq!(shown(&doc, node).0, home + lifted);

    // `nudge-right-50`: 50 units over 150 ms.
    let edits = EditRegistry::new(
        baumhard::document::edit::parse_edits(include_str!("../../../assets/edits.json"))
            .expect("the bundled edits read"),
        Vec::new(),
    );
    let began = start + 150.0;
    run_edit(
        &mut doc,
        &mut Undrawn,
        &edits,
        "nudge-right-50",
        None,
        ChangeSource::Edit,
        began,
    )
    .expect("runs");
    clock.advance_to(began + 75.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    let (at, _) = shown(&doc, node);
    assert!(
        at.x > home.x && at.x < home.x + 50.0,
        "mid-way right: {at}, home {home}"
    );
    assert_eq!(at.y, home.y - 6.0, "and lifted all the while");
    clock.advance_to(began + 150.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(shown(&doc, node).0, home + Vec2::new(50.0, 0.0) + lifted);

    // The entry is the edit's, and the save is the map's.
    let entries: Vec<(String, &str)> = doc
        .doc()
        .history()
        .map(|entry| (entry.label.to_string(), entry.source.name()))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].1, "edit");
    let after = doc.save_json().expect("saves");
    assert_ne!(after, before);
    doc.undo();
    assert_eq!(doc.save_json().expect("saves"), before, "undo restores the map");
    assert_eq!(
        shown(&doc, node).0,
        home + lifted,
        "with the presentation still on it"
    );
    assert_eq!(doc.doc().presentations().count(), 1);
}
