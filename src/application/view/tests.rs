// SPDX-License-Identifier: MPL-2.0

//! The pipeline, renderer-free (`MapView::sync_for_test`): what a sync
//! does when nothing changed, which runs the selection tint stamps and
//! re-stamps, and that selection and the click hit chain read the
//! projector.

use std::collections::BTreeSet;

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::writer::EditRequest;

use super::inspect::{children_of_kind, element_bounds, runs_of};
use super::MapView;
use crate::application::document::selection::every_node;
use crate::application::document::tests_common::testament;
use crate::application::document::MindMapDocument;

/// A node's own runs, as a set the stamp report can be compared with.
fn runs(view: &MapView, doc: &MindMapDocument, node: NodeId) -> BTreeSet<(usize, NodeId)> {
    runs_of(doc.doc(), view.projector(), view.layers(), node)
        .into_iter()
        .map(|(layer, id)| (layer.index(), id))
        .collect()
}

fn stamped(report: &super::SyncReport) -> BTreeSet<(usize, NodeId)> {
    report
        .stamped
        .iter()
        .map(|&(layer, id)| (layer.index(), id))
        .collect()
}

/// Two drawn top-level nodes of the testament map, each with runs of its
/// own and a section with text.
fn two_drawn_roots(doc: &MindMapDocument, view: &MapView) -> (NodeId, NodeId) {
    let roots: Vec<NodeId> = doc
        .doc()
        .children(doc.doc().root())
        .filter(|&id| doc.doc().get(id).map(|e| e.kind()) == Some(DocKind::Node))
        .filter(|&id| !runs(view, doc, id).is_empty())
        .collect();
    assert!(
        roots.len() >= 2,
        "the testament map has two drawn top-level nodes"
    );
    (roots[0], roots[1])
}

/// A child under `parent`: an edit that touches `parent`'s subtree.
fn add_child(doc: &mut MindMapDocument, parent: NodeId) {
    let steps = compose::create_child(doc.doc(), parent, glam::Vec2::new(0.0, 120.0)).expect("composes");
    doc.apply_for_test(EditRequest {
        steps,
        label: "child".into(),
    })
    .expect("applies");
}

/// Text typed into `node`'s first section: an edit that rewrites the
/// node's own runs.
fn type_into(doc: &mut MindMapDocument, node: NodeId) {
    let section = children_of_kind(doc.doc(), node, DocKind::Section)
        .next()
        .expect("a section");
    let steps = compose::insert_text(doc.doc(), section, 0, "Z".into()).expect("composes");
    doc.apply_for_test(EditRequest {
        steps,
        label: "type".into(),
    })
    .expect("applies");
}

/// A frame with nothing to do — no change, the same selection — runs
/// nothing: no projector update, no stamp. Negative control: one change
/// makes the same call do work.
#[test]
fn test_a_sync_with_nothing_changed_does_nothing() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    assert!(view.sync_for_test(&mut doc).is_none(), "nothing changed");
    assert!(view.sync_for_test(&mut doc).is_none(), "still nothing");
    let (a, _) = two_drawn_roots(&doc, &view);
    add_child(&mut doc, a);
    let report = view.sync_for_test(&mut doc).expect("a change is synced");
    assert!(report.projected > 0, "the projector wrote the change");
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "and the next frame is quiet again"
    );
}

/// Selecting a node stamps exactly its own runs; moving the selection
/// clears the old node's runs and stamps the new one's, and nothing
/// else; selecting the same node again stamps nothing.
#[test]
fn test_the_tint_stamps_the_selection_and_only_the_runs_that_change() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b) = two_drawn_roots(&doc, &view);

    doc.selection.select_only(a);
    let first = view.sync_for_test(&mut doc).expect("the selection changed");
    assert_eq!(first.projected, 0, "a selection is not a document change");
    assert_eq!(stamped(&first), runs(&view, &doc, a));
    assert_eq!(view.highlighted(), [a]);

    doc.selection.select_only(b);
    let moved = view.sync_for_test(&mut doc).expect("the selection moved");
    let expected: BTreeSet<_> = runs(&view, &doc, a)
        .union(&runs(&view, &doc, b))
        .copied()
        .collect();
    assert_eq!(
        stamped(&moved),
        expected,
        "a's runs cleared, b's stamped, nothing else"
    );

    doc.selection.select_only(b);
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "the same selection stamps nothing"
    );
}

/// An edit re-stamps the tint only on the runs it rewrote of a selected
/// node: an edit elsewhere leaves the selected node's runs alone, and
/// typing into the selected node re-stamps its runs (the negative
/// control — the re-stamp is the projector's runs, not a fixed set).
#[test]
fn test_an_edit_restamps_only_the_selected_runs_it_rewrote() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b) = two_drawn_roots(&doc, &view);
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("stamped");

    add_child(&mut doc, b);
    let elsewhere = view.sync_for_test(&mut doc).expect("the edit is synced");
    assert!(elsewhere.projected > 0);
    let a_runs = runs(&view, &doc, a);
    assert!(
        stamped(&elsewhere).is_disjoint(&a_runs),
        "an edit under another node re-stamps none of the selected node's runs"
    );

    type_into(&mut doc, a);
    let here = view.sync_for_test(&mut doc).expect("the edit is synced");
    let a_runs = runs(&view, &doc, a);
    assert!(
        !stamped(&here).is_empty() && stamped(&here).is_subset(&a_runs),
        "typing into the selected node re-stamps its rewritten runs: {:?} of {:?}",
        stamped(&here),
        a_runs
    );
}

/// A click selects through the projector's hit test: the middle of a
/// node's drawn box hits that node or an element of it, and selection
/// resolves it to the node or one of its paths; a point off the map
/// hits nothing.
#[test]
fn test_a_click_resolves_through_the_projectors_hit() {
    use crate::application::app::{compute_click_hit, ClickHit};
    let doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _) = two_drawn_roots(&doc, &view);
    let rect = element_bounds(doc.doc(), view.projector(), view.layers(), a).expect("drawn");
    let middle = rect.origin + rect.size / 2.0;
    let hit = view.hit(middle).expect("the middle of a drawn node hits");
    assert_eq!(doc.doc().owning_node(hit.element), Some(hit.node));
    match compute_click_hit(Some(&doc), Some(&mut view), middle) {
        ClickHit::Element(element) => assert_eq!(doc.doc().owning_node(element), Some(hit.node)),
        ClickHit::Empty => panic!("a click on a drawn node selects something"),
    }
    assert_eq!(
        compute_click_hit(Some(&doc), Some(&mut view), glam::Vec2::new(-1.0e7, -1.0e7)),
        ClickHit::Empty
    );
}

/// Undo and redo move the document through its writer and the view
/// follows: the node a child edit added is drawn, gone after Undo, and
/// back after Redo.
#[test]
fn test_undo_and_redo_reach_the_layers() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _) = two_drawn_roots(&doc, &view);
    let before = every_node(doc.doc()).count();
    add_child(&mut doc, a);
    view.sync_for_test(&mut doc).expect("synced");
    assert_eq!(every_node(doc.doc()).count(), before + 1);
    // The child is the parent's last node child (`Placement::Last`), and
    // its connector is drawn.
    let added = children_of_kind(doc.doc(), a, DocKind::Node)
        .last()
        .expect("the child");
    let connector = children_of_kind(doc.doc(), added, DocKind::Connector)
        .next()
        .expect("its connector");
    assert!(
        !view.projector().glyphs_of(connector).is_empty(),
        "the new connector is drawn"
    );

    assert!(doc.undo().is_some());
    view.sync_for_test(&mut doc).expect("the undo is synced");
    assert_eq!(every_node(doc.doc()).count(), before);
    assert!(
        view.projector().glyphs_of(connector).is_empty(),
        "the undone connector draws nothing"
    );

    assert!(doc.redo().is_some());
    view.sync_for_test(&mut doc).expect("the redo is synced");
    assert_eq!(every_node(doc.doc()).count(), before + 1);
    let restored = children_of_kind(doc.doc(), a, DocKind::Node)
        .last()
        .expect("the child is back");
    let connector = children_of_kind(doc.doc(), restored, DocKind::Connector)
        .next()
        .expect("with its connector");
    assert!(
        !view.projector().glyphs_of(connector).is_empty(),
        "and drawn again"
    );
}
