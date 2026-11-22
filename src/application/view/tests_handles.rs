// SPDX-License-Identifier: MPL-2.0

//! The handle chrome (`view::handles`; V1-C7 #248), without a renderer:
//! what each set shows, when the chrome is written, and that two sets
//! shown at once are each updated in place — the shape the note on #248
//! from #274's review asks this layer not to repeat.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, NodeOp};
use baumhard::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;

use super::handles::{Handle, HandleSet, PathHandle, ResizeSide};
use super::inspect::children_of_kind;
use super::MapView;
use crate::application::app::{InteractionMode, ResizeTarget};
use crate::application::document::MindMapDocument;

/// The live fixture: a connector from node 2 to node 1, nodes to resize.
fn live() -> MindMapDocument {
    let path = format!(
        "{}/src/application/app/dispatch/families/fixtures/live.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    );
    MindMapDocument::load(&path, 50).expect("the fixture loads")
}

fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
    doc.doc()
        .node_by_ident(baumhard::document::ids::NodeIdent(ident))
        .expect("the node")
}

/// The `(origin, opacity)` of every run of `set` in the chrome, slot by
/// slot.
fn runs(view: &MapView, set: HandleSet) -> Vec<(Vec2, f32)> {
    let chrome = view.chrome();
    view.handles()
        .group(set)
        .children(chrome.arena())
        .map(|id| {
            let run = chrome
                .get(id)
                .and_then(|node| node.get().glyph_run())
                .expect("every slot is a run");
            (run.origin(), run.opacity())
        })
        .collect()
}

/// Move `node` by `by` through the writer.
fn nudge(doc: &mut MindMapDocument, node: NodeId, by: Vec2) {
    let at = match doc.doc().get(node) {
        Some(baumhard::document::element::DocElement::Node(data)) => data.position,
        _ => panic!("a node"),
    };
    let steps = compose::set(
        doc.doc(),
        DocKind::Node,
        vec![DocMutation::Node(NodeOp::SetPosition(at + by))],
        &[node],
    )
    .expect("composes");
    doc.apply(EditRequest::new(steps, "test: nudge"), ChangeSource::Action)
        .expect("applies");
}

/// **Two handle sets shown at once are each updated in place**: the resize
/// mode's node set on node 2 and the path set on node 2's connector, both
/// shown; moving node 2 moves both — every node handle by exactly the
/// move, and the connector's end at node 2 — in one pass whose chrome
/// journal touches runs of both sets and nothing else.
///
/// Negative control, seen red (`HandleSet::channel` with every set on
/// channels from 0, and `Handles::plant` putting every set in the first
/// set's group): the node set's update, keyed by channels the path set's
/// runs share under one parent, moves the path set's runs as well, and
/// the path set's own update reaches nothing.
#[test]
fn test_two_handle_sets_are_each_updated_in_place() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let n2 = node(&doc, 2);
    let connector = children_of_kind(doc.doc(), n2, DocKind::Connector)
        .next()
        .expect("node 2 has a connector");
    view.follow_mode(&InteractionMode::Resize {
        target: ResizeTarget::Node(n2),
    });
    doc.selection.select_only(connector);
    let shown = view.sync_for_test(&mut doc).expect("the handles are shown");
    let (node_set, path_set) = (runs(&view, HandleSet::Node), runs(&view, HandleSet::Path));
    assert!(
        node_set.iter().all(|&(_, a)| a == 1.0),
        "every node handle shows: {node_set:?}"
    );
    let path_shown = path_set.iter().filter(|&&(_, a)| a == 1.0).count();
    assert_eq!(path_shown, 3, "start, end and midpoint show: {path_set:?}");
    let groups = |set| {
        view.handles()
            .group(set)
            .children(view.chrome().arena())
            .collect::<Vec<_>>()
    };
    let (node_runs, path_runs) = (groups(HandleSet::Node), groups(HandleSet::Path));
    assert!(shown.chrome.iter().any(|id| node_runs.contains(id)));
    assert!(shown.chrome.iter().any(|id| path_runs.contains(id)));

    let (from, to) = view.projector().path_ends(connector).expect("drawn");
    let at_two = if from == n2 {
        PathHandle::Start
    } else {
        PathHandle::End
    };
    let end_before = view
        .handle_center(Handle::Path {
            path: connector,
            handle: at_two,
        })
        .expect("shown");
    assert_ne!(from, to);
    let by = Vec2::new(40.0, 0.0);
    nudge(&mut doc, n2, by);
    let moved = view.sync_for_test(&mut doc).expect("the move redraws");
    let (node_after, path_after) = (runs(&view, HandleSet::Node), runs(&view, HandleSet::Path));
    for ((before, _), (after, alpha)) in node_set.iter().zip(&node_after) {
        assert_eq!(*alpha, 1.0);
        assert!(
            (*after - (*before + by)).length() < 1e-3,
            "a node handle moved by the move: {before} → {after}"
        );
    }
    let end_after = view
        .handle_center(Handle::Path {
            path: connector,
            handle: at_two,
        })
        .expect("shown");
    assert!(
        (end_after - end_before).length() > 1.0,
        "the path set's end at node 2 moved too"
    );
    assert_ne!(path_after, path_set, "the path set was rewritten in place");
    let both: Vec<NodeId> = node_runs.iter().chain(&path_runs).copied().collect();
    assert!(
        moved.chrome.iter().all(|id| both.contains(id)),
        "nothing but the two sets was written"
    );
    assert!(moved.chrome.iter().any(|id| node_runs.contains(id)));
    assert!(moved.chrome.iter().any(|id| path_runs.contains(id)));
}

/// **The chrome is written when the selection or the mode changes, and
/// not otherwise**: a pass with nothing changed writes nothing; a pass
/// after the selection changed writes the path set; one after the mode
/// left Resize hides the node set.
#[test]
fn test_the_handles_follow_the_selection_and_the_mode_only() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let n2 = node(&doc, 2);
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "nothing changed, nothing written"
    );
    let connector = children_of_kind(doc.doc(), n2, DocKind::Connector)
        .next()
        .expect("a connector");
    doc.selection.select_only(connector);
    let report = view
        .sync_for_test(&mut doc)
        .expect("the selection shows the path's handles");
    let path_runs: Vec<NodeId> = view
        .handles()
        .group(HandleSet::Path)
        .children(view.chrome().arena())
        .collect();
    assert!(
        report.chrome.iter().all(|id| path_runs.contains(id)),
        "only the path set"
    );
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "and a pass after it writes nothing"
    );

    doc.selection.select_only(n2);
    view.follow_mode(&InteractionMode::Resize {
        target: ResizeTarget::Node(n2),
    });
    view.sync_for_test(&mut doc)
        .expect("the mode shows the node's handles");
    assert!(runs(&view, HandleSet::Node).iter().all(|&(_, a)| a == 1.0));
    assert!(
        runs(&view, HandleSet::Path).iter().all(|&(_, a)| a == 0.0),
        "the path's went with the selection"
    );
    view.follow_mode(&InteractionMode::Default);
    view.sync_for_test(&mut doc).expect("leaving the mode hides them");
    assert!(runs(&view, HandleSet::Node).iter().all(|&(_, a)| a == 0.0));
}

/// **A press grabs the nearest shown handle within its tolerance, a
/// resize handle before a path's**: the south-east handle of node 2 at
/// its corner, nothing a tolerance away.
#[test]
fn test_a_press_grabs_the_nearest_shown_handle() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let n2 = node(&doc, 2);
    assert_eq!(
        view.handle_at(Vec2::ZERO, 1e6),
        None,
        "no handle is shown before a mode"
    );
    view.follow_mode(&InteractionMode::Resize {
        target: ResizeTarget::Node(n2),
    });
    view.sync_for_test(&mut doc);
    let corner = view.projector().node_rect(n2).expect("drawn").max();
    assert_eq!(
        view.handle_at(corner + Vec2::new(2.0, 1.0), 8.0),
        Some(Handle::Node {
            node: n2,
            side: ResizeSide::SE
        })
    );
    assert_eq!(
        view.handle_at(corner + Vec2::new(20.0, 0.0), 8.0),
        None,
        "out of reach"
    );
}
