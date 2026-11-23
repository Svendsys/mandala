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

/// **A frameless node's handles follow its resize** (#395's review, the
/// reviewer's own reproduction): node 2 draws no frame, so a grown
/// minimum size writes no run node 2 owns; the pass that projects it
/// still shows the south-east handle on the new corner, and writes the
/// chrome. An undo and a redo of the resize move it back and forth too.
///
/// Control, seen red: following on the runs the target owns
/// (`PassChanges::touches`) leaves the handle at the old corner and
/// writes no chrome.
#[test]
fn test_a_frameless_nodes_handles_follow_its_resize() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let n2 = node(&doc, 2);
    let se = Handle::Node {
        node: n2,
        side: ResizeSide::SE,
    };
    doc.selection.select_only(n2);
    view.follow_mode(&InteractionMode::Resize {
        target: ResizeTarget::Node(n2),
    });
    view.sync_for_test(&mut doc).expect("the mode shows the handles");
    let start = view.projector().node_rect(n2).expect("drawn");
    assert_eq!(view.handle_center(se), Some(start.max()));
    let steps = compose::set(
        doc.doc(),
        DocKind::Node,
        vec![DocMutation::Node(NodeOp::SetMinSize(Some(
            start.size + Vec2::new(100.0, 50.0),
        )))],
        &[n2],
    )
    .expect("composes");
    doc.apply(EditRequest::new(steps, "test: grow"), ChangeSource::Action)
        .expect("applies");
    let report = view.sync_for_test(&mut doc).expect("the grow projects");
    let grown = view.projector().node_rect(n2).expect("drawn");
    assert!(
        (grown.size - start.size - Vec2::new(100.0, 50.0)).length() < 1e-3,
        "control: the node grew, {} to {}",
        start.size,
        grown.size
    );
    assert_eq!(
        view.handle_center(se),
        Some(grown.max()),
        "the handle is on the new corner"
    );
    let node_runs: Vec<NodeId> = view
        .handles()
        .group(HandleSet::Node)
        .children(view.chrome().arena())
        .collect();
    assert!(
        report.chrome.iter().any(|id| node_runs.contains(id)),
        "the pass wrote the node set's chrome"
    );
    assert!(doc.undo().is_some());
    view.sync_for_test(&mut doc).expect("the undo projects");
    assert_eq!(view.handle_center(se), Some(start.max()), "after an undo");
    assert!(doc.redo().is_some());
    view.sync_for_test(&mut doc).expect("the redo projects");
    assert_eq!(view.handle_center(se), Some(grown.max()), "after a redo");
}

/// **A section's handles follow its node**: a section's box on the canvas
/// is its node's drawn top-left plus its own box, so moving node 5 moves
/// the resize handles of its second section by the move, though the
/// section's own box within the node did not change.
///
/// Control, seen red: following a section on its own geometry only (the
/// node's left out), which leaves the handles where the node was.
#[test]
fn test_a_sections_handles_follow_its_node() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let n5 = node(&doc, 5);
    let section = children_of_kind(doc.doc(), n5, DocKind::Section)
        .nth(1)
        .expect("a second section");
    let se = Handle::Section {
        section,
        side: ResizeSide::SE,
    };
    doc.selection.select_only(section);
    view.follow_mode(&InteractionMode::Resize {
        target: ResizeTarget::Section(section),
    });
    view.sync_for_test(&mut doc).expect("the mode shows the handles");
    let before = view.handle_center(se).expect("shown");
    assert_eq!(
        Some(before),
        view.projector().section_rect(section).map(|r| r.max())
    );
    nudge(&mut doc, n5, Vec2::new(30.0, 20.0));
    view.sync_for_test(&mut doc).expect("the move projects");
    let after = view.handle_center(se).expect("shown");
    assert!(
        after.distance(before + Vec2::new(30.0, 20.0)) < 1e-3,
        "the handle moved with the node: {before} to {after}"
    );
}

/// **`Projector::through_points` is where the drawn path passes**: for
/// the line link from node 3 to node 2, with one control point, its
/// start and its end are the drawn chain's, and its middle point is the
/// chord's middle moved by the control point (D34) — the end of one of
/// the drawn curves, and where the path set shows control point 0's
/// handle. A connector with no control points passes through its ends
/// only.
///
/// Control: the control point's offset itself is no curve's end.
#[test]
fn test_through_points_are_where_the_drawn_path_passes() {
    let mut doc = live();
    let mut view = MapView::for_test(&doc);
    let (n2, n3) = (node(&doc, 2), node(&doc, 3));
    let link = children_of_kind(doc.doc(), n3, DocKind::Link)
        .next()
        .expect("a line link");
    let points = view.projector().through_points(link).expect("drawn");
    let curves = view.projector().curve(link).expect("drawn").to_vec();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0], curves[0].point(0.0));
    assert_eq!(points[2], curves.last().expect("a curve").point(1.0));
    let via = Vec2::new(0.0, -40.0);
    assert!(
        points[1].distance((points[0] + points[2]) * 0.5 + via) < 1e-3,
        "the control point's place: {}",
        points[1]
    );
    let ends: Vec<Vec2> = curves.iter().map(|c| c.point(1.0)).collect();
    assert!(
        ends.iter().any(|e| e.distance(points[1]) < 1e-3),
        "the path passes through it"
    );
    assert!(!ends.iter().any(|e| e.distance(via) < 1e-3), "control");
    doc.selection.select_only(link);
    view.sync_for_test(&mut doc)
        .expect("the path's handles are shown");
    assert_eq!(
        view.handle_center(Handle::Path {
            path: link,
            handle: PathHandle::Control(0),
        }),
        Some(points[1])
    );
    let connector = children_of_kind(doc.doc(), n2, DocKind::Connector)
        .next()
        .expect("a connector");
    assert_eq!(
        view.projector().through_points(connector).map(|p| p.len()),
        Some(2)
    );
}

/// A node with a frame of one ring and two sections, and a child with a
/// connector: what the picker's stand-down is read against.
fn framed() -> MindMapDocument {
    MindMapDocument::from_json_str(
        r#"{"format": 1, "name": "Framed", "nodes": [
            {"id": 1, "position": [0.0, 0.0],
             "sections": [{"spans": [{"text": "one"}]}, {"channel": 1, "offset": [0.0, 30.0], "spans": [{"text": "two"}]}],
             "frame": {"rings": [{"spans": [{"text": "*"}]}]}},
            {"id": 2, "parent": 1, "position": [300.0, 0.0], "sections": [{"spans": [{"text": "kid"}]}],
             "connector": {"content": "-"}}
        ]}"#,
        None,
        50,
    )
    .expect("the map loads")
}

/// **While a contextual picker is open, the selection tint stands down
/// where the picker's axis colors its targets** (#248's ruling on the
/// picker's preview), so the color it stages is what is drawn:
///
/// - picking text, a selected node keeps the tint on its frame's ring and
///   not on its sections, and a selected path stands down whole;
/// - picking the frame color, the node keeps it on its sections and not
///   on its ring;
/// - a ring rewritten by the projector while it stands is drawn in the
///   tint again (the overlay re-stamps a part of a node, not only a node);
/// - with the picker gone the tint is back where it was.
///
/// Control, seen red: the picker followed and nothing stood down
/// (`stand_down` returning its targets), and the re-stamp matching owners
/// only (`overlay::target_of` replaced by `owner_of`).
#[test]
fn test_the_tint_stands_down_where_the_picker_colors() {
    use crate::application::keybinds::ColorAxis;
    use baumhard::document::mutation::ops::{RingOp, TextOp};
    use baumhard::document::values::{ColorRef, Rgba};
    let mut doc = framed();
    let mut view = MapView::for_test(&doc);
    let n1 = node(&doc, 1);
    let n2 = node(&doc, 2);
    let connector = children_of_kind(doc.doc(), n2, DocKind::Connector)
        .next()
        .expect("a connector");
    let frame = children_of_kind(doc.doc(), n1, DocKind::Frame)
        .next()
        .expect("a frame");
    let ring = children_of_kind(doc.doc(), frame, DocKind::Ring)
        .next()
        .expect("a ring");
    let sections: Vec<NodeId> = children_of_kind(doc.doc(), n1, DocKind::Section).collect();
    doc.selection.set([n1, connector]);
    view.sync_for_test(&mut doc).expect("the selection is tinted");
    assert_eq!(
        view.tinted(),
        &[n1, connector],
        "control: the whole node and the path wear the tint"
    );
    let runs = |view: &MapView, doc: &MindMapDocument, of: &[NodeId]| {
        let mut all: Vec<_> = of
            .iter()
            .flat_map(|&e| view.runs_of_for_test(doc.doc(), e))
            .collect();
        all.sort_unstable_by_key(|&(l, g)| (l.index(), g));
        all
    };
    // The runs drawn in the tint, read off the layers: what a projector
    // rewrite wipes, not the overlay's own record of what it stamped.
    let everything = runs(&view, &doc, &[n1, connector]);
    let stamped = |view: &MapView| {
        everything
            .iter()
            .copied()
            .filter(|&(layer, glyph)| view.run_is_tinted(layer, glyph))
            .collect::<Vec<_>>()
    };
    let picked = [n1, connector];

    view.follow_picker(Some((ColorAxis::Text, &picked)));
    view.sync_for_test(&mut doc).expect("the tint stands down");
    assert_eq!(
        view.tinted(),
        &[ring],
        "picking text: the ring keeps the tint, the rest stands down"
    );
    assert_eq!(
        stamped(&view),
        runs(&view, &doc, &[ring]),
        "and only the ring's runs are stamped"
    );
    assert!(!runs(&view, &doc, &sections).is_empty());

    // A rewrite of the ring's run keeps its stamp.
    let steps = baumhard::document::mutation::compose::set(
        doc.doc(),
        DocKind::Ring,
        vec![DocMutation::Ring(RingOp::Text(TextOp::SetColor(Some(
            ColorRef::Rgba(Rgba::opaque(1, 2, 3)),
        ))))],
        &[ring],
    )
    .expect("composes");
    doc.apply(EditRequest::new(steps, "test: ring color"), ChangeSource::Action)
        .expect("applies");
    let report = view.sync_for_test(&mut doc).expect("the ring is rewritten");
    assert!(
        !report.rewritten.is_empty(),
        "precondition: the projector rewrote runs"
    );
    assert_eq!(
        stamped(&view),
        runs(&view, &doc, &[ring]),
        "the rewritten ring is stamped again"
    );

    view.follow_picker(Some((ColorAxis::Border, &picked)));
    view.sync_for_test(&mut doc).expect("the tint moves");
    assert_eq!(
        view.tinted(),
        sections.as_slice(),
        "picking the frame color: the sections keep the tint"
    );
    assert_eq!(stamped(&view), runs(&view, &doc, &sections));

    view.follow_picker(None);
    view.sync_for_test(&mut doc).expect("the tint is back");
    assert_eq!(view.tinted(), &[n1, connector]);
    assert_eq!(stamped(&view), runs(&view, &doc, &[n1, connector]));
}
