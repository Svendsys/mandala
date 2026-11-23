// SPDX-License-Identifier: MPL-2.0

//! The text editors' view, renderer-free (`MapView::sync_for_test`):
//! the caret moves with typing and a pass that changed nothing — a pan's
//! — writes no chrome; editing a ring re-projects that ring alone; the
//! caret follows its owner out of the drawing; NodeEdit's dim stamps
//! every other node once and re-stamps only what a pass rewrote; the
//! section frames outline a node of several sections; the editor's
//! selected range is tinted and the selection tint stands down on what is
//! edited.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::NodeIdent;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::writer::EditRequest;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutator::Mutator;
use baumhard::projection::Layer;

use crate::application::app::dispatch::families::text::text_focus;
use crate::application::app::text_edit::tests::text_fixture;
use crate::application::app::text_edit::{TextEditor, TextSession};
use crate::application::app::InteractionMode;
use crate::application::document::MindMapDocument;
use crate::application::platform::input::{Key, SmolStr};
use crate::application::view::caret::EditorFocus;
use crate::application::view::inspect::{children_of_kind, runs_of};
use crate::application::view::overlay::{DIM_FADE, SELECTION_HIGHLIGHT};
use crate::application::view::MapView;

/// Node `ident`.
fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
    doc.doc().node_by_ident(NodeIdent(ident)).expect("in the fixture")
}

/// The first child of `of` of `kind`.
fn child(doc: &MindMapDocument, of: NodeId, kind: DocKind) -> NodeId {
    children_of_kind(doc.doc(), of, kind)
        .next()
        .expect("in the fixture")
}

/// Type `c` into the open section editor and stage it.
fn type_char(editor: &mut TextEditor, doc: &mut MindMapDocument, c: &str) {
    let session = editor.session_mut().expect("open");
    session.type_key(None, &Key::Character(SmolStr::new(c)));
    assert!(session.stage(doc));
}

/// Where the caret is drawn now.
fn caret_at(view: &MapView) -> glam::Vec2 {
    view.caret().shown().expect("the caret is shown").0
}

/// **The caret moves with typing, and a pan writes nothing.** Typing a
/// character moves it right along the line; a pass with no change — what
/// a pan or a zoom is to the view — places nothing and writes no chrome;
/// the caret moved left goes back.
#[test]
fn test_the_caret_moves_with_typing_and_a_pan_places_nothing() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let section = child(&doc, node(&doc, 3), DocKind::Section);
    let mut editor = TextEditor::Open(TextSession::open(&mut doc, section, false, false).expect("opens"));
    view.follow_editor(text_focus(&editor));
    let opened = view.sync_for_test(&mut doc).expect("the caret is shown");
    assert!(!opened.chrome.is_empty(), "the caret was written");
    let start = caret_at(&view);

    type_char(&mut editor, &mut doc, "x");
    view.follow_editor(text_focus(&editor));
    view.sync_for_test(&mut doc).expect("typed");
    let typed = caret_at(&view);
    assert!(
        typed.x > start.x,
        "typing moves the caret right: {start} → {typed}"
    );
    assert!((typed.y - start.y).abs() < 0.01, "along the same line");

    let placed = view.caret().placed();
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "a pan's pass does nothing"
    );
    assert_eq!(view.caret().placed(), placed, "and places no caret");

    editor
        .session_mut()
        .unwrap()
        .apply_action(&crate::application::keybinds::Action::TextEditCursorLeft);
    view.follow_editor(text_focus(&editor));
    view.sync_for_test(&mut doc).expect("the caret moved");
    assert!(caret_at(&view).x < typed.x, "control: moved left, it goes back");
}

/// **Editing a ring re-projects that ring alone** (§6.5: a flow is shaped
/// when its spans change, and only then): every run the pass rewrote is
/// the ring's. Control: the pass did rewrite something, and an edit of a
/// section of the same node rewrites that section's run, not the ring's.
#[test]
fn test_editing_a_ring_reprojects_only_that_ring() {
    use crate::application::app::single_line_edit::{LineSession, SingleLineTarget};
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let a = node(&doc, 1);
    let ring = child(&doc, child(&doc, a, DocKind::Frame), DocKind::Ring);
    let mut session = LineSession::open(&mut doc, SingleLineTarget::Owner(ring), false).expect("opens");
    session.type_key(&Key::Character(SmolStr::new("!")));
    assert!(session.stage(&mut doc));
    let report = view.sync_for_test(&mut doc).expect("the ring changed");
    assert!(!report.rewritten.is_empty(), "control: the pass rewrote the ring");
    for &(layer, glyph) in &report.rewritten {
        assert_eq!(
            view.projector().element_of(layer, glyph),
            Some(ring),
            "{layer:?} {glyph:?} is the ring's"
        );
    }
    session.cancel(&mut doc);
    view.sync_for_test(&mut doc).expect("canceled");

    let section = child(&doc, a, DocKind::Section);
    let steps = compose::insert_text(doc.doc(), section, 0, "Z".into()).expect("composes");
    doc.apply_for_test(EditRequest::new(steps, "type"))
        .expect("applies");
    let report = view.sync_for_test(&mut doc).expect("the section changed");
    assert!(
        report
            .rewritten
            .iter()
            .all(|&(layer, glyph)| view.projector().element_of(layer, glyph) != Some(ring)),
        "control: a section's edit rewrites no run of the ring"
    );
}

/// Apply `op` to `node`.
fn node_op(doc: &mut MindMapDocument, node: NodeId, op: NodeOp) {
    let mutator = DocMutatorTree::new_with(Mutator::new(
        DocMutation::Node(op),
        BranchKey::new(DocKind::Node, 0),
    ));
    doc.apply_for_test(EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(vec![node])))],
        "view::tests_text (node op)",
    ))
    .expect("applies");
}

/// **The caret follows its owner out of the drawing**: folded away,
/// hidden or deleted, the owner draws nothing, and the pass that took it
/// away places the caret again — on nothing, so it hides. The caret is the
/// view's (ruling K1 on #249): it follows a focus handed to it, whatever
/// handed it. Control: an edit of another node places no caret.
#[test]
fn test_the_caret_follows_its_owner_out_of_the_drawing() {
    for removal in ["fold", "hide", "delete"] {
        let mut doc = text_fixture();
        let mut view = MapView::for_test(&doc);
        let (parent, owner_node) = (node(&doc, 1), node(&doc, 2));
        let section = child(&doc, owner_node, DocKind::Section);
        view.follow_editor(Some(EditorFocus {
            owner: section,
            host: owner_node,
            caret: 0,
            anchor: None,
        }));
        view.sync_for_test(&mut doc).expect("the caret is shown");
        assert!(view.caret().shown().is_some());

        let placed = view.caret().placed();
        let steps = compose::insert_text(
            doc.doc(),
            child(&doc, node(&doc, 3), DocKind::Section),
            0,
            "Z".into(),
        )
        .expect("composes");
        doc.apply_for_test(EditRequest::new(steps, "elsewhere"))
            .expect("applies");
        view.sync_for_test(&mut doc).expect("an edit elsewhere");
        assert_eq!(
            view.caret().placed(),
            placed,
            "{removal}: control: elsewhere places nothing"
        );

        match removal {
            "fold" => node_op(&mut doc, parent, NodeOp::SetFolded(true)),
            "hide" => node_op(&mut doc, owner_node, NodeOp::SetHidden(true)),
            _ => {
                let steps = compose::delete(doc.doc(), &[owner_node]).expect("composes");
                doc.apply_for_test(EditRequest::new(steps, "delete"))
                    .expect("applies");
            }
        }
        let gone = view.sync_for_test(&mut doc).expect("the removal is synced");
        assert!(
            view.caret().placed() > placed,
            "{removal}: the caret was placed again"
        );
        assert!(view.caret().shown().is_none(), "{removal}: on nothing, so hidden");
        assert!(!gone.chrome.is_empty(), "{removal}: and its chrome written");
    }
}

/// Whether the run `glyph` of `layer` carries the dim's fade.
fn faded(view: &MapView, layer: Layer, glyph: NodeId) -> bool {
    view.layers()
        .tree(layer)
        .get(glyph)
        .and_then(|n| n.get().glyph_run())
        .is_some_and(|run| run.overrides().fade().contains(&DIM_FADE))
}

/// **NodeEdit dims every other node once, and re-stamps only what a pass
/// rewrote** (§12.5): entering it stamps the sections' and frames' runs of
/// every node but the one edited, at half alpha; an edit of one of them
/// re-stamps that node's run and no other; leaving clears exactly what was
/// stamped. Control: before entering, no run is faded, and the edited
/// node's runs never are.
#[test]
fn test_node_edit_dims_every_other_node_and_restamps_only_what_changed() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let (a, b, c) = (node(&doc, 1), node(&doc, 2), node(&doc, 3));
    let runs = |view: &MapView, doc: &MindMapDocument, n: NodeId| {
        runs_of(doc.doc(), view.projector(), view.layers(), n)
    };
    for n in [a, b, c] {
        assert!(
            runs(&view, &doc, n).iter().all(|&(l, g)| !faded(&view, l, g)),
            "control: no fade yet"
        );
    }

    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    view.sync_for_test(&mut doc).expect("the dim is stamped");
    for n in [b, c] {
        assert!(
            runs(&view, &doc, n).iter().all(|&(l, g)| faded(&view, l, g)),
            "{n:?} is dimmed"
        );
    }
    assert!(
        runs(&view, &doc, a).iter().all(|&(l, g)| !faded(&view, l, g)),
        "the edited node is not"
    );
    let stamped = view.dim().stamped_runs().len();
    assert_eq!(stamped, runs(&view, &doc, b).len() + runs(&view, &doc, c).len());

    let steps =
        compose::insert_text(doc.doc(), child(&doc, c, DocKind::Section), 0, "Z".into()).expect("composes");
    doc.apply_for_test(EditRequest::new(steps, "type"))
        .expect("applies");
    let report = view.sync_for_test(&mut doc).expect("an edit");
    let b_runs = runs(&view, &doc, b);
    assert!(
        report.stamped.iter().all(|stamp| !b_runs.contains(stamp)),
        "an edit of c re-stamps nothing of b"
    );
    assert!(
        runs(&view, &doc, c).iter().all(|&(l, g)| faded(&view, l, g)),
        "c is dimmed again"
    );

    view.follow_node_edit(&InteractionMode::Default);
    view.sync_for_test(&mut doc).expect("the dim is cleared");
    for n in [a, b, c] {
        assert!(
            runs(&view, &doc, n).iter().all(|&(l, g)| !faded(&view, l, g)),
            "cleared"
        );
    }
    assert!(view.dim().stamped_runs().is_empty());
}

/// **The section frames outline a node of several sections, the focused
/// one heavy**, and follow the node and the focus; a pass that changed
/// nothing writes none; a node of one section has none, as on `main`.
#[test]
fn test_section_frames_outline_a_node_of_several_sections() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let a = node(&doc, 1);
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    let entered = view.sync_for_test(&mut doc).expect("the frames are drawn");
    assert!(!entered.chrome.is_empty());
    let shown = |view: &MapView| view.frames().shown().iter().filter(|f| f.is_some()).count();
    assert_eq!(shown(&view), 2, "one per section");
    assert!(view
        .frames()
        .shown()
        .iter()
        .flatten()
        .all(|&(_, focused)| !focused));

    let second = children_of_kind(doc.doc(), a, DocKind::Section)
        .nth(1)
        .expect("two sections");
    doc.selection.select_only(second);
    view.sync_for_test(&mut doc).expect("the focus moved");
    assert_eq!(
        view.frames().shown()[1].map(|(_, focused)| focused),
        Some(true),
        "the selected one is heavy"
    );
    assert!(view.sync_for_test(&mut doc).is_none(), "a pan writes no frame");

    view.follow_node_edit(&InteractionMode::NodeEdit { node: node(&doc, 3) });
    view.sync_for_test(&mut doc).expect("the frames are hidden");
    assert_eq!(shown(&view), 0, "a node of one section has none");
    assert_eq!(view.frames().slots(), 2, "the pool keeps its slots");
}

/// **An editor's selected range is tinted, and the selection tint stands
/// down on what is edited**: a shift-select colors exactly its glyphs of
/// the section's run; the node the selection names is no target while
/// its section is edited. Control: with no anchor no range is stamped, and
/// once the editor closes the selection tint is back.
#[test]
fn test_the_editors_range_is_tinted_and_the_selection_stands_down() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let a = node(&doc, 1);
    let section = child(&doc, a, DocKind::Section);
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("selected");
    assert!(view.highlighted().contains(&a), "precondition: a is tinted");

    let focus = |anchor| EditorFocus {
        owner: section,
        host: a,
        caret: 3,
        anchor,
    };
    view.follow_editor(Some(focus(None)));
    view.sync_for_test(&mut doc).expect("the editor opened");
    assert!(
        !view.highlighted().contains(&a),
        "the selection tint stands down on what is edited"
    );
    assert!(
        view.range_tint().stamped().is_none(),
        "control: no anchor, no range"
    );

    view.follow_editor(Some(focus(Some(1))));
    view.sync_for_test(&mut doc).expect("a range");
    let (layer, glyph, range) = view
        .range_tint()
        .stamped()
        .cloned()
        .expect("the range is stamped");
    assert_eq!(range, 1..3, "graphemes 1..3 of `Headline` are its glyphs 1..3");
    let run = view
        .layers()
        .tree(layer)
        .get(glyph)
        .and_then(|n| n.get().glyph_run())
        .expect("a run");
    let colors = run.overrides().color();
    assert_eq!(colors[1..3], [Some(SELECTION_HIGHLIGHT); 2]);
    assert_eq!((colors[0], colors[3]), (None, None), "exactly the range");

    view.follow_editor(None);
    view.sync_for_test(&mut doc).expect("the editor closed");
    assert!(view.highlighted().contains(&a), "control: the tint is back");
    assert!(view.range_tint().stamped().is_none());
}
