// SPDX-License-Identifier: MPL-2.0

//! The pipeline, renderer-free (`MapView::sync_for_test`): what a sync
//! does when nothing changed, which runs the selection tint stamps and
//! re-stamps, and that selection and the click hit chain read the
//! projector.

use std::collections::BTreeSet;

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::writer::EditRequest;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutator::Mutator;

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

/// The color the overlays left on `glyph`'s first glyph, in `layer`.
#[cfg(not(target_arch = "wasm32"))]
fn tint_of(view: &MapView, layer: usize, glyph: NodeId) -> Option<baumhard::document::values::Rgba> {
    let tree = view.layers().tree(baumhard::projection::Layer::ALL[layer]);
    let run = tree.get(glyph)?.get().glyph_run()?;
    run.overrides().color().first().copied().flatten()
}

/// **The reparent and connect target tint stamps exactly the hovered
/// node's runs, in its own color, beside the selection's**, and clearing
/// it clears exactly those. The selection's stamp is not touched: its
/// runs keep the selection's color throughout (the control — a tint that
/// overwrote or cleared every stamp would change them).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_the_target_tint_stamps_the_hovered_node_beside_the_selection() {
    use super::overlay::{SELECTION_HIGHLIGHT, TARGET_HIGHLIGHT};
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b) = two_drawn_roots(&doc, &view);
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("the selection is stamped");
    let selected = |view: &MapView, doc: &MindMapDocument| {
        runs(view, doc, a)
            .into_iter()
            .all(|(layer, glyph)| tint_of(view, layer, glyph) == Some(SELECTION_HIGHLIGHT))
    };
    assert!(
        selected(&view, &doc),
        "precondition: a carries the selection's tint"
    );

    view.set_hover_target(Some(b));
    assert_eq!(view.hover_target(), Some(b));
    let report = view.sync_for_test(&mut doc).expect("the tint changed");
    assert_eq!(
        stamped(&report),
        runs(&view, &doc, b),
        "b's runs, and nothing else"
    );
    for (layer, glyph) in runs(&view, &doc, b) {
        assert_eq!(tint_of(&view, layer, glyph), Some(TARGET_HIGHLIGHT));
    }
    assert!(selected(&view, &doc), "the selection's stamp is untouched");

    view.set_hover_target(None);
    assert_eq!(view.hover_target(), None);
    let report = view.sync_for_test(&mut doc).expect("the tint went");
    assert_eq!(
        stamped(&report),
        runs(&view, &doc, b),
        "only b's runs are cleared"
    );
    for (layer, glyph) in runs(&view, &doc, b) {
        assert_eq!(tint_of(&view, layer, glyph), None);
    }
    assert!(selected(&view, &doc));
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "and the next pass is quiet"
    );
}

/// **A mode's source tint goes on the sources only, in place of their
/// selection tint, and gives it back when the mode ends.** With `a` and
/// `b` selected and `a` the one source, `a`'s runs turn the source color
/// in one pass — its selection stamp cleared before the source stamp, so
/// the source's wins — while `b` keeps the selection's (the control: a
/// tint on every selected node would turn `b` too). Ending the mode turns
/// `a` back, in one pass the other way.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_the_source_tint_replaces_the_selections_on_the_sources_only() {
    use super::overlay::{SELECTION_HIGHLIGHT, SOURCE_HIGHLIGHT};
    use crate::application::app::InteractionMode;
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b) = two_drawn_roots(&doc, &view);
    doc.selection.set([a, b]);
    view.sync_for_test(&mut doc).expect("the selection is stamped");
    let tinted = |view: &MapView, doc: &MindMapDocument, node: NodeId, color| {
        runs(view, doc, node)
            .into_iter()
            .all(|(layer, glyph)| tint_of(view, layer, glyph) == Some(color))
    };
    assert!(
        tinted(&view, &doc, a, SELECTION_HIGHLIGHT),
        "precondition: a is selected"
    );

    view.follow_mode(&InteractionMode::Reparent { sources: vec![a] });
    view.sync_for_test(&mut doc).expect("the source tint changed");
    assert!(
        tinted(&view, &doc, a, SOURCE_HIGHLIGHT),
        "a wears the source tint"
    );
    assert!(
        tinted(&view, &doc, b, SELECTION_HIGHLIGHT),
        "b keeps the selection's"
    );

    view.follow_mode(&InteractionMode::Default);
    view.sync_for_test(&mut doc).expect("the source tint went");
    assert!(
        tinted(&view, &doc, a, SELECTION_HIGHLIGHT),
        "a is the selection's again"
    );
    assert!(tinted(&view, &doc, b, SELECTION_HIGHLIGHT));
}

/// **A hovered target that becomes selected wears the selection's tint,
/// and keeps it when the hover leaves** (review 5821011650, probe P2):
/// one tint per node holds in the view, not only where the hover is set.
/// `a` is the mode's source; `b` is hovered, then selected while the
/// pointer stands still (`SelectAll`, a sibling move), then the hover
/// leaves. Control: before `b` is selected it wears the target tint, so
/// the hover did stamp it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_a_hovered_target_that_is_selected_keeps_the_selections_tint() {
    use super::overlay::{SELECTION_HIGHLIGHT, TARGET_HIGHLIGHT};
    use crate::application::app::InteractionMode;
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b) = two_drawn_roots(&doc, &view);
    doc.selection.select_only(a);
    view.follow_mode(&InteractionMode::Reparent { sources: vec![a] });
    view.set_hover_target(Some(b));
    view.sync_for_test(&mut doc).expect("the tints are stamped");
    let tinted = |view: &MapView, doc: &MindMapDocument, color| {
        runs(view, doc, b)
            .into_iter()
            .all(|(layer, glyph)| tint_of(view, layer, glyph) == Some(color))
    };
    assert!(
        tinted(&view, &doc, TARGET_HIGHLIGHT),
        "control: b wears the target tint"
    );

    doc.selection.set([a, b]);
    view.sync_for_test(&mut doc).expect("the selection changed");
    assert!(
        tinted(&view, &doc, SELECTION_HIGHLIGHT),
        "selected, b wears the selection's"
    );
    view.set_hover_target(None);
    view.sync_for_test(&mut doc);
    assert!(
        tinted(&view, &doc, SELECTION_HIGHLIGHT),
        "and keeps it when the hover leaves"
    );
}

/// **A rescan that found a font reshapes the view in one pass that loses
/// nothing** (#329, the review of #390): a change made just before the
/// rescan, not yet synced, reaches the change feed — the IPC event
/// stream's source — and the reparent mode the view was following, its
/// sources and its status line, are still the view's after it.
///
/// Control: replacing the view, as the rescan did before
/// (`MapView::load`, here `MapView::for_test`), turns both arms red: the
/// feed is empty and the mode is gone.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_a_rescan_that_found_a_font_keeps_the_feed_and_the_mode() {
    use crate::application::app::InteractionMode;
    let mut doc = MindMapDocument::from_json_str(
        r#"{"format":1,"name":"f","nodes":[{"id":1,"position":[0.0,0.0],"font":"Nope Sans","sections":[{"spans":[{"text":"a"}]}]},{"id":2,"position":[300.0,0.0],"sections":[{"spans":[{"text":"b"}]}]}]}"#,
        None,
        50,
    )
    .expect("the map loads");
    let mut view = MapView::for_test(&doc);
    let a = doc
        .doc()
        .node_by_ident(baumhard::document::ids::NodeIdent(1))
        .expect("node 1");
    view.follow_mode(&InteractionMode::Reparent { sources: vec![a] });
    view.sync_for_test(&mut doc);
    let before = view.pending_feed().len();
    let revision = doc.create_root_for_test();
    view.faces_added_for_test(&mut doc, &["Nope Sans".to_string()]);
    assert!(
        view.pending_feed()[before.min(view.pending_feed().len())..]
            .iter()
            .any(|record| record.revision == revision),
        "the change made just before the rescan reaches the feed"
    );
    assert_eq!(view.sources(), [a], "the mode's sources survive");
    assert!(view.mode_line.is_some(), "and its status line");
}
