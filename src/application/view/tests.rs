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

/// A chrome builder that follows one node, for the hook's test: one run,
/// planted hidden at load, set visible each time a pass writes a run
/// the node owns. Where its run is and how often it was handed a pass
/// are shared with the test.
#[cfg(not(target_arch = "wasm32"))]
struct Follower {
    target: NodeId,
    run: std::rc::Rc<std::cell::Cell<Option<NodeId>>>,
    calls: std::rc::Rc<std::cell::Cell<usize>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl super::chrome::ChromeBuilder for Follower {
    fn plant(&mut self, chrome: &mut baumhard::gfx_structs::tree::GfxTree) {
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("view::tests (follower plant)");
        let run = super::band::run_of("", glam::Vec2::ZERO, 0.0, &mut fonts);
        let id = chrome
            .insert_child(
                chrome.root(),
                baumhard::gfx_structs::element::GfxElement::new_run(
                    run,
                    baumhard::projection::RUN_CHANNEL,
                    0,
                ),
                baumhard::engine::tree::Placement::Last,
            )
            .expect("the chrome's root takes a child");
        self.run.set(Some(id));
    }

    fn follow(
        &mut self,
        pass: &super::chrome::PassChanges<'_>,
    ) -> Vec<(
        baumhard::gfx_structs::tree::GfxMutatorTree,
        baumhard::engine::anchor::Anchors,
    )> {
        use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
        self.calls.set(self.calls.get() + 1);
        if !pass.touches(self.target) {
            return Vec::new();
        }
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("view::tests (follower)");
        let run = super::band::run_of("\u{25A1}", glam::Vec2::ZERO, 1.0, &mut fonts);
        vec![(
            baumhard::gfx_structs::tree::GfxMutatorTree::new_with(GfxMutator::new(
                GfxMutation::RunSet(Box::new(run)),
                baumhard::projection::RUN_CHANNEL,
            )),
            baumhard::engine::anchor::Anchors::Nodes(vec![self.run.get().expect("planted at load")]),
        )]
    }
}

/// **A chrome builder is handed every pass's `LayerChanges`, before the
/// pass discards them, and its chrome is written in the same pass**
/// (`view::chrome`, the hook V1-C7's handles and V1-C8's caret register
/// on). A builder that follows node `a` re-stamps its run when an edit
/// writes `a`'s runs, and the pass's chrome journal names that run and
/// nothing else.
///
/// Controls: a frame with no document change — what a pan or a zoom is
/// to the view, which applies the camera when it draws — runs no pass
/// and reaches no builder; a selection alone changes no layer and
/// reaches no builder either; an edit to another node is handed to the
/// builder, which finds `a` untouched and re-stamps nothing.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_a_chrome_builder_follows_the_layer_changes_that_touch_its_target() {
    let mut doc = testament();
    let (a, b) = two_drawn_roots(&doc, &MapView::for_test(&doc));
    let run = std::rc::Rc::new(std::cell::Cell::new(None));
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let follower = Follower {
        target: a,
        run: run.clone(),
        calls: calls.clone(),
    };
    let mut view = MapView::for_test_with_chrome(&doc, vec![Box::new(follower)]);
    let planted = run.get().expect("the builder is planted at load");
    let opacity = |view: &MapView| {
        view.chrome()
            .get(planted)
            .and_then(|node| node.get().glyph_run().map(|run| run.opacity()))
            .expect("the planted run is in the chrome")
    };
    assert_eq!(opacity(&view), 0.0, "planted hidden");

    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "control: with no document change there is no pass"
    );
    assert_eq!(calls.get(), 0, "control: a pan reaches no builder");

    doc.selection.select_only(a);
    let selected = view.sync_for_test(&mut doc).expect("the selection changed");
    assert_eq!(selected.projected, 0, "a selection is not a document change");
    assert_eq!(calls.get(), 0, "control: a selection alone reaches no builder");
    assert!(selected.chrome.is_empty());

    type_into(&mut doc, b);
    let elsewhere = view.sync_for_test(&mut doc).expect("an edit");
    assert!(elsewhere.projected > 0, "the edit wrote b's runs");
    assert_eq!(
        calls.get(),
        1,
        "a pass that changed the layers reaches the builder"
    );
    assert!(
        elsewhere.chrome.is_empty(),
        "control: an edit to another node re-stamps nothing"
    );
    assert_eq!(opacity(&view), 0.0, "control: the run is as planted");

    type_into(&mut doc, a);
    let on_target = view.sync_for_test(&mut doc).expect("an edit");
    assert_eq!(calls.get(), 2);
    assert_eq!(
        on_target.chrome,
        vec![planted],
        "an edit to the target re-stamps its run, and only it, in the same pass"
    );
    assert_eq!(opacity(&view), 1.0, "the run was rewritten");

    // A removal is a write: a node folded away, hidden or deleted stops
    // drawing, and its follower must hear of it. `c` is a drawn node
    // whose parent `p` is drawn too, so folding `p` takes `c` away.
    let (p, c) = every_node(doc.doc())
        .filter(|&c| !runs(&view, &doc, c).is_empty())
        .find_map(|c| {
            let p = doc.doc().parent(c)?;
            (doc.doc().get(p).map(|e| e.kind()) == Some(DocKind::Node) && !runs(&view, &doc, p).is_empty())
                .then_some((p, c))
        })
        .expect("the testament map has a drawn node under a drawn node");
    let run = std::rc::Rc::new(std::cell::Cell::new(None));
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut view = MapView::for_test_with_chrome(
        &doc,
        vec![Box::new(Follower {
            target: c,
            run: run.clone(),
            calls: calls.clone(),
        })],
    );
    let planted = run.get().expect("planted at load");
    let node_op = |doc: &mut MindMapDocument, node: NodeId, op: NodeOp| {
        let mutator = DocMutatorTree::new_with(Mutator::new(
            DocMutation::Node(op),
            BranchKey::new(DocKind::Node, 0),
        ));
        doc.apply_for_test(EditRequest::new(
            vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(vec![node])))],
            "view::tests (removal)",
        ))
        .expect("applies");
    };

    node_op(&mut doc, p, NodeOp::SetFolded(true));
    let folded = view.sync_for_test(&mut doc).expect("the fold is synced");
    assert!(
        runs(&view, &doc, c).is_empty(),
        "precondition: folding p takes c away"
    );
    assert_eq!(
        folded.chrome,
        vec![planted],
        "a node folded away touches its follower"
    );
    node_op(&mut doc, p, NodeOp::SetFolded(false));
    view.sync_for_test(&mut doc).expect("the unfold is synced");
    assert!(
        !runs(&view, &doc, c).is_empty(),
        "precondition: unfolding p draws c again"
    );

    node_op(&mut doc, c, NodeOp::SetHidden(true));
    let hidden = view.sync_for_test(&mut doc).expect("the hide is synced");
    assert!(runs(&view, &doc, c).is_empty(), "precondition: c is hidden");
    assert_eq!(hidden.chrome, vec![planted], "a hidden node touches its follower");
    node_op(&mut doc, c, NodeOp::SetHidden(false));
    view.sync_for_test(&mut doc).expect("the unhide is synced");

    let steps = compose::delete(doc.doc(), &[c]).expect("composes");
    doc.apply_for_test(EditRequest {
        steps,
        label: "delete".into(),
    })
    .expect("applies");
    let deleted = view.sync_for_test(&mut doc).expect("the delete is synced");
    assert!(
        !every_node(doc.doc()).any(|n| n == c) && runs(&view, &doc, c).is_empty(),
        "precondition: c is gone from the document and draws nothing"
    );
    assert_eq!(
        deleted.chrome,
        vec![planted],
        "a deleted node touches its follower"
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

    assert!(view.set_hover_target(Some(b)));
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

    assert!(view.set_hover_target(None));
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
