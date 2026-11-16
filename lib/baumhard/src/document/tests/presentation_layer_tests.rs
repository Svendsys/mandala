// SPDX-License-Identifier: MPL-2.0

//! The presentation layer at the size it runs at (V1-D2, #251): a map
//! of [`NODES`] nodes with a presentation on every one of them — a
//! frame that moves every instance, a change made under them, the layer
//! set aside and brought back for a read, and a start and a stop on
//! every node.
//!
//! These are the layer's `do_*()` bodies (`lib/baumhard/CONVENTIONS.md`
//! §B8): each is a test and a row of `benches/test_bench.rs`. What an
//! instance is, and every rule of its lifecycle, is held one case at a
//! time in `presentation_tests`, whose tests read `cfg(test)` work
//! counters and so cannot be bodies here.

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind};
use crate::document::format::load;
use crate::document::ids::Document;
use crate::document::presentation::{parse_presentations, PresentationDef};
use crate::document::tests::writer_support::place;
use crate::document::writer::ChangeSource;
use glam::Vec2;

/// How many nodes the map holds: a screenful of a large map.
pub const NODES: usize = 256;

/// A map of [`NODES`] top-level nodes in a row, 300 units apart, each
/// with one section of one span, and its nodes in order.
fn row() -> (Document, Vec<NodeId>) {
    let nodes: Vec<String> = (0..NODES)
        .map(|i| {
            format!(
                r#"{{"id":{},"position":[{}.0,0.0],"sections":[{{"spans":[{{"text":"n{i}"}}]}}]}}"#,
                i + 1,
                i * 300
            )
        })
        .collect();
    let doc = load(&format!(
        r#"{{"format":1,"name":"row","nodes":[{}]}}"#,
        nodes.join(",")
    ))
    .unwrap_or_else(|error| panic!("the row loads: {error}"));
    let ids: Vec<NodeId> = doc
        .children(doc.root())
        .filter(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Node))
        .collect();
    assert_eq!(ids.len(), NODES);
    (doc, ids)
}

/// One presentation, from the JSON of its definition.
fn presentation(json: &str) -> PresentationDef {
    parse_presentations(&format!(r#"{{"presentations":[{json}]}}"#))
        .unwrap_or_else(|error| panic!("{json} reads: {error}"))
        .remove(0)
}

/// Lifts a node eight units over 200 ms, and back over 200.
const LIFT: &str = r#"{"id":"lift","name":"Lift","play":"while","target":"node",
"mutations":[{"node":{"nudge":[0.0,-8.0]}}],"animate":{"ms":200},"return":{"ms":200}}"#;

/// Fades a node to half, at once, for as long as it is held.
const DIM: &str = r#"{"id":"dim","name":"Dim","play":"hold","target":"node",
"mutations":[{"node":{"set_opacity":0.5}}]}"#;

/// The position and the opacity the document holds for `node` now.
fn shown(doc: &Document, node: NodeId) -> (Vec2, f32) {
    match doc.get(node) {
        Some(DocElement::Node(data)) => (data.position, data.opacity),
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_presentation_a_frame_moves_every_instance() {
    do_presentation_a_frame_moves_every_instance();
}

/// A lift on every node of the row, brought through ten frames 20 ms
/// apart: at each, every node is where its progress puts it, and at the
/// last every one holds. The map saves as it was.
pub fn do_presentation_a_frame_moves_every_instance() {
    let (mut doc, nodes) = row();
    let before = doc.save_json().expect("saves");
    doc.start_presentation(&presentation(LIFT), &nodes, 0.0)
        .expect("starts");
    for frame in 1..=10u32 {
        assert!(
            doc.tick_presentations(f64::from(frame) * 20.0),
            "frame {frame} draws"
        );
        let lift = -8.0 * frame as f32 / 10.0;
        for (i, &node) in nodes.iter().enumerate() {
            let (at, _) = shown(&doc, node);
            assert_eq!(at.x, (i * 300) as f32);
            assert!((at.y - lift).abs() < 1e-4, "frame {frame}, node {i}: {at}");
        }
    }
    assert!(!doc.presentations_moving(), "each holds");
    assert_eq!(doc.presentations().count(), NODES);
    assert_eq!(doc.save_json().expect("saves"), before);
}

#[test]
fn test_presentation_a_change_under_the_layer_is_the_change_alone() {
    do_presentation_a_change_under_the_layer_is_the_change_alone();
}

/// With every node dimmed and lifted, one node is moved and the move
/// undone: the moved node is drawn lifted from where the move put it,
/// every other node is as it was, the history holds the move alone, and
/// the undo leaves the map and the layer as before.
pub fn do_presentation_a_change_under_the_layer_is_the_change_alone() {
    let (mut doc, nodes) = row();
    let before = doc.save_json().expect("saves");
    doc.start_presentation(&presentation(DIM), &nodes, 0.0)
        .expect("starts");
    doc.start_presentation(&presentation(LIFT), &nodes, 0.0)
        .expect("starts");
    doc.tick_presentations(200.0);
    let moved = nodes[NODES / 2];
    doc.apply(place(vec![moved], Vec2::new(-500.0, 40.0)), ChangeSource::Action)
        .expect("applies");
    assert_eq!(shown(&doc, moved), (Vec2::new(-500.0, 32.0), 0.5));
    assert_eq!(shown(&doc, nodes[0]), (Vec2::new(0.0, -8.0), 0.5));
    assert_eq!(doc.history().count(), 1);
    assert_ne!(doc.save_json().expect("saves"), before);
    doc.undo().expect("undoes");
    assert_eq!(doc.save_json().expect("saves"), before);
    assert_eq!(
        shown(&doc, moved),
        (Vec2::new((NODES / 2 * 300) as f32, -8.0), 0.5)
    );
    assert_eq!(doc.presentations().count(), 2 * NODES);
}

#[test]
fn test_presentation_a_suspension_reads_the_map_and_brings_the_layer_back() {
    do_presentation_a_suspension_reads_the_map_and_brings_the_layer_back();
}

/// With every node dimmed and lifted, the layer is set aside for a
/// read: while it is aside every node is the map's own, and once it is
/// resumed every node is drawn as before, with nothing left on the
/// change feed for a view that drained it before the read.
pub fn do_presentation_a_suspension_reads_the_map_and_brings_the_layer_back() {
    let (mut doc, nodes) = row();
    doc.start_presentation(&presentation(DIM), &nodes, 0.0)
        .expect("starts");
    doc.start_presentation(&presentation(LIFT), &nodes, 0.0)
        .expect("starts");
    doc.tick_presentations(200.0);
    doc.take_changes();
    doc.suspend_presentations();
    for (i, &node) in nodes.iter().enumerate() {
        assert_eq!(shown(&doc, node), (Vec2::new((i * 300) as f32, 0.0), 1.0));
    }
    doc.resume_presentations();
    for (i, &node) in nodes.iter().enumerate() {
        assert_eq!(shown(&doc, node), (Vec2::new((i * 300) as f32, -8.0), 0.5));
    }
    assert!(
        doc.take_changes().is_empty(),
        "set aside and back, nothing is news"
    );
}

#[test]
fn test_presentation_starts_and_stops_on_every_node() {
    do_presentation_starts_and_stops_on_every_node();
}

/// A lift started on every node, restarted on every node, stopped on
/// every other node and run out: one instance a node, never two; the
/// stopped half is over after its return, the rest holds.
pub fn do_presentation_starts_and_stops_on_every_node() {
    let (mut doc, nodes) = row();
    let lift = presentation(LIFT);
    assert_eq!(
        doc.start_presentation(&lift, &nodes, 0.0).expect("starts").len(),
        NODES
    );
    doc.tick_presentations(100.0);
    assert_eq!(
        doc.start_presentation(&lift, &nodes, 100.0)
            .expect("restarts")
            .len(),
        NODES
    );
    assert_eq!(doc.presentations().count(), NODES, "a restart stacks nothing");
    doc.tick_presentations(300.0);
    let half: Vec<NodeId> = nodes.iter().copied().step_by(2).collect();
    assert_eq!(doc.stop_presentation("lift", &half, 300.0), half.len());
    doc.tick_presentations(500.0);
    assert_eq!(doc.presentations().count(), NODES - half.len());
    assert_eq!(shown(&doc, nodes[0]).0, Vec2::new(0.0, 0.0));
    assert_eq!(shown(&doc, nodes[1]).0, Vec2::new(300.0, -8.0));
}
