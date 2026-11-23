// SPDX-License-Identifier: MPL-2.0

//! The text editors' view, renderer-free (`MapView::sync_for_test`):
//! the caret moves with typing and a pass that changed nothing — a pan's
//! — writes no chrome; editing a ring re-projects that ring and the paths
//! that stop at its inks, nothing else; the
//! caret follows its owner out of the drawing; NodeEdit's dim stamps
//! every other node once and re-stamps only what a pass rewrote; the
//! section frames outline a node of several sections; the editor's
//! selected range is tinted and the selection tint stands down on what is
//! edited; and the caret and the range are read from the run each owner
//! draws, whether it is a unit of its own or a slot of its path's or its
//! portal's.

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
    let mut editor = TextEditor::Open(Box::new(
        TextSession::open(&mut doc, section, false, false).expect("opens"),
    ));
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

/// Whether `element` — what a rewritten run draws — belongs to a path
/// with an end on `node`: a connector, a line link or a portal end (and
/// their caps and labels), by the nodes the path joins in the document.
fn on_a_path_of(doc: &MindMapDocument, element: NodeId, node: NodeId) -> bool {
    use baumhard::document::element::DocElement;
    let mut at = element;
    loop {
        match doc.doc().get(at) {
            Some(DocElement::Connector(_)) => {
                let child = doc.doc().parent(at);
                let parent = child.and_then(|c| doc.doc().parent(c));
                return child == Some(node) || parent == Some(node);
            }
            Some(DocElement::Link(link)) => return doc.doc().parent(at) == Some(node) || link.to == node,
            Some(DocElement::Node(_)) | None => return false,
            Some(_) => match doc.doc().parent(at) {
                Some(parent) => at = parent,
                None => return false,
            },
        }
    }
}

/// Type `text` into `ring` with the single-line editor, sync, and hand
/// back what the pass rewrote, each with the element it draws; the
/// session is canceled and the cancel synced.
fn edit_ring(
    doc: &mut MindMapDocument,
    view: &mut MapView,
    ring: NodeId,
    text: &str,
) -> Vec<(Layer, NodeId, Option<NodeId>)> {
    use crate::application::app::single_line_edit::{LineSession, SingleLineTarget};
    let mut session = LineSession::open(doc, SingleLineTarget::Owner(ring), false).expect("opens");
    session.type_key(&Key::Character(SmolStr::new(text)));
    assert!(session.stage(doc));
    let report = view.sync_for_test(doc).expect("the ring changed");
    let rewritten = report
        .rewritten
        .iter()
        .map(|&(layer, glyph)| (layer, glyph, view.projector().element_of(layer, glyph)))
        .collect();
    session.cancel(doc);
    view.sync_for_test(doc).expect("canceled");
    rewritten
}

/// **Editing a ring re-projects that ring, and the paths of its node,
/// which stop at its inks (#296) — and nothing else** (§6.5; ruling on
/// #249 amending "only that ring"):
///
/// 1. every run the pass rewrote is the ring's own, or a path's with an
///    end on the ring's node — a connector, a line link, a portal end;
/// 2. the ring's own run is among them;
/// 3. an edit of a section of the same node — one that keeps its size,
///    since a node that grows (D5) moves the outline its rings follow —
///    rewrites no run of the ring;
/// 4. a ring edit on another node rewrites nothing of this node's own
///    runs, its sections' and its rings'.
#[test]
fn test_editing_a_ring_reprojects_the_ring_and_the_paths_that_stop_at_it() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let (a, b) = (node(&doc, 1), node(&doc, 2));
    let ring = child(&doc, child(&doc, a, DocKind::Frame), DocKind::Ring);

    let rewritten = edit_ring(&mut doc, &mut view, ring, "!");
    for &(layer, glyph, element) in &rewritten {
        let element = element.expect("a rewritten run draws an element");
        assert!(
            element == ring || on_a_path_of(&doc, element, a),
            "{layer:?} {glyph:?} draws {element:?}, which is neither the ring nor a path of its node"
        );
    }
    assert!(
        rewritten.iter().any(|&(_, _, element)| element == Some(ring)),
        "the ring's own run is rewritten"
    );
    // Control for 1: asked of b, which the re-laid line link (a to 3) has
    // no end on, the same rule rejects a rewritten run.
    assert!(
        rewritten
            .iter()
            .any(|&(_, _, element)| element.is_some_and(|e| e != ring && !on_a_path_of(&doc, e, b))),
        "control: the rule can reject a run"
    );
    let first = rewritten.clone();

    // A section edit that keeps the node's size — a color: one that grows
    // the node (D5) moves its outline, and with it the ring, as it should.
    let section = child(&doc, a, DocKind::Section);
    let red =
        baumhard::document::values::ColorRef::Rgba(baumhard::document::values::Rgba::opaque(0xff, 0, 0));
    let steps = compose::style_range(
        doc.doc(),
        section,
        0,
        1,
        vec![baumhard::document::mutation::ops::TextOp::SetColor(Some(red))],
    )
    .expect("composes");
    doc.apply_for_test(EditRequest::new(steps, "color"))
        .expect("applies");
    let report = view.sync_for_test(&mut doc).expect("the section changed");
    assert!(
        report
            .rewritten
            .iter()
            .any(|&(layer, glyph)| view.projector().element_of(layer, glyph) == Some(section)),
        "precondition: the section's run is rewritten"
    );
    assert!(
        report
            .rewritten
            .iter()
            .all(|&(layer, glyph)| view.projector().element_of(layer, glyph) != Some(ring)),
        "a section's edit rewrites no run of the ring"
    );

    let own: Vec<(Layer, NodeId)> = runs_of(doc.doc(), view.projector(), view.layers(), a);
    assert!(
        first
            .iter()
            .any(|&(layer, glyph, _)| own.contains(&(layer, glyph))),
        "control for 4: a's own ring edit does rewrite a run of a's"
    );
    let other_ring = child(&doc, child(&doc, b, DocKind::Frame), DocKind::Ring);
    let rewritten = edit_ring(&mut doc, &mut view, other_ring, "!");
    assert!(
        rewritten
            .iter()
            .any(|&(_, _, element)| element == Some(other_ring)),
        "precondition: the other ring is rewritten"
    );
    for &(layer, glyph, _) in &rewritten {
        assert!(
            !own.contains(&(layer, glyph)),
            "{layer:?} {glyph:?} is a's own, and was rewritten"
        );
    }
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
    view.sync_for_test(&mut doc).expect("an edit");
    let c_runs = runs(&view, &doc, c);
    let planned = view.dim().last_planned();
    assert!(!planned.is_empty(), "the rewritten run of c is stamped again");
    assert!(
        planned.iter().all(|run| c_runs.contains(run)),
        "an edit of c re-stamps c's runs and nothing else: {planned:?}"
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

/// The runs of `node` that carry the dim's fade now.
fn faded_runs(view: &MapView, doc: &MindMapDocument, node: NodeId) -> Vec<(Layer, NodeId)> {
    runs_of(doc.doc(), view.projector(), view.layers(), node)
        .into_iter()
        .filter(|&(layer, glyph)| faded(view, layer, glyph))
        .collect()
}

/// Edit `node`'s first section, so the next pass rewrites its run.
fn edit_section_of(doc: &mut MindMapDocument, node: NodeId) {
    let steps =
        compose::insert_text(doc.doc(), child(doc, node, DocKind::Section), 0, "Z".into()).expect("composes");
    doc.apply_for_test(EditRequest::new(steps, "type"))
        .expect("applies");
}

/// **A pass that leaves NodeEdit and rewrites a dimmed run leaves no
/// fade behind** (review of #399): the transition is planned before the
/// pass's changes are read, so the rewritten run of `c` is not stamped
/// under the mode the pass leaves, and nothing is stamped that the dim no
/// longer tracks.
///
/// Control: the same edit in a pass that stays in NodeEdit leaves `c`
/// dimmed — the rewrite is re-stamped, so an empty answer below is the
/// leave's, not a rewrite that lost its stamp. And, seen red: the changes
/// read against the state before the transition (the code this replaced)
/// leaves `c`'s section run faded with the dim tracking nothing.
#[test]
fn test_leaving_node_edit_in_a_pass_that_rewrites_a_dimmed_run_clears_it() {
    let (mut doc, mut view, a, c) = {
        let doc = text_fixture();
        let view = MapView::for_test(&doc);
        let (a, c) = (node(&doc, 1), node(&doc, 3));
        (doc, view, a, c)
    };
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    view.sync_for_test(&mut doc).expect("the dim is stamped");

    edit_section_of(&mut doc, c);
    view.sync_for_test(&mut doc).expect("an edit");
    let c_runs = runs_of(doc.doc(), view.projector(), view.layers(), c);
    assert_eq!(
        faded_runs(&view, &doc, c),
        c_runs,
        "control: staying, c is dimmed"
    );

    edit_section_of(&mut doc, c);
    view.follow_node_edit(&InteractionMode::Default);
    view.sync_for_test(&mut doc).expect("leave and edit");
    assert_eq!(faded_runs(&view, &doc, c), Vec::new(), "no fade on c");
    assert!(view.dim().stamped_runs().is_empty(), "and the dim tracks nothing");
}

/// **NodeEdit moved from one node to another in a pass that rewrites the
/// new one leaves it undimmed**, and dims the one it left: the rewritten
/// run of `b` is read against `b` being edited.
///
/// Control, seen red: the changes read against the state before the
/// transition — `b`'s rewritten run is stamped under `a`.
#[test]
fn test_moving_node_edit_to_a_node_rewritten_in_the_same_pass_leaves_it_undimmed() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let (a, b) = (node(&doc, 1), node(&doc, 2));
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    view.sync_for_test(&mut doc).expect("the dim is stamped");
    assert!(
        !faded_runs(&view, &doc, b).is_empty(),
        "precondition: b is dimmed"
    );

    edit_section_of(&mut doc, b);
    view.follow_node_edit(&InteractionMode::NodeEdit { node: b });
    view.sync_for_test(&mut doc).expect("move and edit");
    assert_eq!(
        faded_runs(&view, &doc, b),
        Vec::new(),
        "the node edited now is not dimmed"
    );
    let a_runs = runs_of(doc.doc(), view.projector(), view.layers(), a);
    assert_eq!(faded_runs(&view, &doc, a), a_runs, "the node left is");
}

/// **A node that joins `excluded` in a pass that rewrites its run keeps no
/// fade under the selection's tint**: no two overlays stamp one run.
///
/// Control, seen red: the changes read against the state before the
/// transition — `c`'s rewritten run is stamped under the old `excluded`,
/// and wears both the fade and the tint.
#[test]
fn test_a_node_joining_excluded_while_its_run_is_rewritten_is_not_dimmed() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let (a, c) = (node(&doc, 1), node(&doc, 3));
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    view.sync_for_test(&mut doc).expect("the dim is stamped");
    assert!(
        !faded_runs(&view, &doc, c).is_empty(),
        "precondition: c is dimmed"
    );

    edit_section_of(&mut doc, c);
    doc.selection.select_only(c);
    view.sync_for_test(&mut doc).expect("select and edit");
    assert_eq!(faded_runs(&view, &doc, c), Vec::new(), "c is tinted, not dimmed");
    assert!(
        view.dim()
            .stamped_runs()
            .iter()
            .all(|run| !runs_of(doc.doc(), view.projector(), view.layers(), c).contains(run)),
        "and the dim does not track c"
    );
}

/// An outline model with round numbers: cells 8 wide, rows 16 high, the
/// horizontal stroke 8 below its run's top and the vertical 4 right of
/// its left edge, each one unit thick. It stood 6 above, 8 below, 4 left
/// and 4 right of what it outlined.
fn round_model() -> crate::application::view::outline::StrokeModel {
    crate::application::view::outline::StrokeModel {
        advance: 8.0,
        pitch: 16.0,
        h_center: 8.0,
        h_half: 0.5,
        v_center: 4.0,
        v_half: 0.5,
    }
}

/// Whether two boxes share more than a touch.
fn crosses(a: baumhard::flow::Rect, b: baumhard::flow::Rect) -> bool {
    const TOUCH: f32 = 1e-3;
    a.origin.x < b.max().x - TOUCH
        && b.origin.x < a.max().x - TOUCH
        && a.origin.y < b.max().y - TOUCH
        && b.origin.y < a.max().y - TOUCH
}

/// A box from `(x, y)` to `(x2, y2)`.
fn rect(x: f32, y: f32, x2: f32, y2: f32) -> baumhard::flow::Rect {
    baumhard::flow::Rect::new(glam::Vec2::new(x, y), glam::Vec2::new(x2 - x, y2 - y))
}

/// **A frame's strokes stand between the section's box and its rings'
/// ink, never across it** (ruling on #249), row by row on one section
/// box `(0, 0)–(100, 20)`:
///
/// - **room**: ink far off on every side — each stroke stands where the
///   outline always stood, outside the box;
/// - **tight**: ink 6 above — the top stroke stands between the box and
///   the ink, clear of both;
/// - **flush**: ink right on the box's top edge — no room, so the top
///   stroke hugs the box from inside, touching neither the ink nor
///   crossing it, and is drawn;
/// - **into**: ink beyond the top edge that reaches into the box, as a
///   ring's letters reach into a section's line box — the top stroke goes
///   inside the box, under the ink, and is drawn;
/// - **over**: ink centered on the box, lying across its top edge — no
///   placement keeps the top stroke clear of it, so that side is not
///   drawn, and the others are;
/// - **letters**: the section's letters 8 inside its top, ink reaching 3
///   into it — the top stroke goes inside, between the ink and the
///   letters, a stroke's width off them;
/// - **no room**: the letters 2 inside the top, ink reaching 3 into it —
///   no stroke fits between them without crossing one or the other, so
///   the top is not drawn.
///
/// In every row no drawn stroke overlaps an ink box.
#[test]
fn test_a_frames_strokes_stand_between_its_section_and_its_rings_ink() {
    use crate::application::view::frames::place;
    let model = round_model();
    let section = rect(0.0, 0.0, 100.0, 20.0);
    let far = [
        rect(0.0, -40.0, 100.0, -30.0),
        rect(0.0, 50.0, 100.0, 60.0),
        rect(-40.0, 0.0, -30.0, 20.0),
        rect(130.0, 0.0, 140.0, 20.0),
    ];
    let into = vec![rect(0.0, -8.0, 100.0, 3.0)];
    let rows: [(&str, Option<baumhard::flow::Rect>, Vec<baumhard::flow::Rect>); 7] = [
        ("room", None, far.to_vec()),
        ("tight", None, vec![rect(0.0, -10.0, 100.0, -6.0)]),
        ("flush", None, vec![rect(0.0, -5.0, 100.0, 0.0)]),
        ("into", None, into.clone()),
        ("over", None, vec![rect(0.0, -8.0, 100.0, 12.0)]),
        ("letters", Some(rect(2.0, 8.0, 98.0, 18.0)), into.clone()),
        ("no room", Some(rect(2.0, 2.0, 98.0, 18.0)), into),
    ];
    for (row, text, inks) in rows {
        let (strokes, reach, shown) = place(section, text, &inks, &model);
        let segments = crate::application::view::frames::segments(&strokes, reach, &model);
        for (side, segment) in segments.iter().enumerate() {
            if shown[side] {
                assert!(
                    inks.iter().all(|ink| !crosses(*segment, *ink)),
                    "{row}: side {side} crosses ink: {segment:?}"
                );
            }
        }
        match row {
            "room" => {
                assert_eq!(shown, [true; 4], "{row}");
                assert!(
                    strokes.top < -model.h_half && strokes.bottom > 20.0 + model.h_half,
                    "{row}: outside"
                );
                assert!(
                    strokes.left < -model.v_half && strokes.right > 100.0 + model.v_half,
                    "{row}: outside"
                );
            }
            "tight" => {
                assert!(shown[0], "{row}: drawn");
                assert!(
                    strokes.top + model.h_half < 0.0 && strokes.top - model.h_half > -6.0,
                    "{row}: between the box and the ink: {}",
                    strokes.top
                );
            }
            "flush" => {
                assert!(shown[0], "{row}: drawn");
                assert!(
                    strokes.top - model.h_half >= -1e-3
                        && strokes.top + model.h_half <= 2.0 * model.h_half + 1e-3,
                    "{row}: hugs the box from inside: {}",
                    strokes.top
                );
            }
            "into" => {
                assert!(shown[0], "{row}: drawn");
                assert!(
                    strokes.top - model.h_half >= 3.0 - 1e-3 && strokes.top < 20.0,
                    "{row}: inside the box, under the ink: {}",
                    strokes.top
                );
            }
            "letters" => {
                assert!(shown[0], "{row}: drawn");
                assert!(
                    strokes.top - model.h_half >= 3.0 - 1e-3
                        && strokes.top + model.h_half <= 8.0 - 2.0 * model.h_half + 1e-3,
                    "{row}: between the ink and the letters: {}",
                    strokes.top
                );
            }
            _ => assert_eq!(shown, [false, true, true, true], "{row}: the top is hidden"),
        }
    }
}

/// **A NodeEdit frame never crosses its node's ring**, on a node of three
/// sections spaced as a node lays them and one ring round them all
/// (`fixtures/frames.mandala.json`), whose letters reach into the
/// sections' line boxes. For every frame, thin and then heavy:
///
/// - no drawn stroke crosses a glyph the ring inks;
/// - a side is left undrawn only where there was no room for it: where a
///   ring glyph comes within two stroke widths of the section's own
///   letters on that side, so no stroke fits between them;
/// - and most sides are drawn (at least two of each frame's four).
///
/// Controls, seen red: the outline where it always stood
/// (`StrokeModel::standing`, no clearance) crosses the ring where there
/// was room, so the check hides sides it had no cause to; and the
/// clearance read from the ring's one outer ink box (`ring.ink`, which
/// lies over the sections) leaves every side undrawn.
#[test]
fn test_section_frames_never_cross_their_rings_ink() {
    use crate::application::view::frames::{ring_inks, text_ink, HEAVY, THIN};
    use crate::application::view::outline::StrokeModel;
    let path = format!(
        "{}/src/application/app/dispatch/families/fixtures/frames.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut doc = MindMapDocument::load(&path, baumhard::document::writer::DEFAULT_HISTORY_LIMIT)
        .expect("the fixture loads");
    let mut view = MapView::for_test(&doc);
    let a = node(&doc, 1);
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    view.sync_for_test(&mut doc).expect("the frames are drawn");
    let inks = ring_inks(doc.doc(), view.projector(), view.layers(), a);
    assert!(!inks.is_empty(), "precondition: the ring inks");
    let sections: Vec<NodeId> = children_of_kind(doc.doc(), a, DocKind::Section).collect();
    let mut swash = glyphon::SwashCache::new();
    // The lease is taken in each check and given back before the next
    // sync, which takes the font system itself.
    let mut check = |view: &MapView, when: &str| {
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("test");
        let models = [
            StrokeModel::of(&THIN, &mut fonts),
            StrokeModel::of(&HEAVY, &mut fonts),
        ];
        let frames: Vec<_> = view.frames().shown().iter().flatten().copied().collect();
        assert_eq!(frames.len(), 3, "{when}: one per section");
        for (frame, &section) in frames.iter().zip(&sections) {
            let model = models[usize::from(frame.focused)];
            let text = text_ink(view.projector(), section, &mut fonts, &mut swash).expect("letters");
            let off = 2.0 * model.h_half.max(model.v_half);
            let drawn = crate::application::view::frames::segments(&frame.strokes, frame.reach, &model);
            for (side, segment) in drawn.into_iter().enumerate() {
                if frame.shown[side] {
                    assert!(
                        inks.iter().all(|ink| !crosses(segment, *ink)),
                        "{when}: side {side} crosses the ring: {segment:?}"
                    );
                    continue;
                }
                // The strip a stroke on this side would need between the
                // letters and the ink: two stroke widths out from them.
                let (lo, hi) = (text.origin, text.max());
                let strip = match side {
                    0 => rect(lo.x, lo.y - 2.0 * off, hi.x, lo.y),
                    1 => rect(lo.x, hi.y, hi.x, hi.y + 2.0 * off),
                    2 => rect(lo.x - 2.0 * off, lo.y, lo.x, hi.y),
                    _ => rect(hi.x, lo.y, hi.x + 2.0 * off, hi.y),
                };
                assert!(
                    inks.iter().any(|ink| crosses(strip, *ink)),
                    "{when}: side {side} of {section:?} is undrawn though there was room"
                );
            }
            assert!(
                frame.shown.iter().filter(|&&drawn| drawn).count() >= 2,
                "{when}: most of the frame is drawn: {:?}",
                frame.shown
            );
        }
    };
    check(&view, "thin");
    doc.selection.select_only(sections[1]);
    view.sync_for_test(&mut doc).expect("the focus moved");
    check(&view, "heavy");
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
    assert!(view.frames().shown().iter().flatten().all(|frame| !frame.focused));

    let second = children_of_kind(doc.doc(), a, DocKind::Section)
        .nth(1)
        .expect("two sections");
    doc.selection.select_only(second);
    view.sync_for_test(&mut doc).expect("the focus moved");
    assert_eq!(
        view.frames().shown()[1].map(|frame| frame.focused),
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

/// **The caret and the range are read from the owner's own run** — for a
/// ring, a unit of its own, and for what is a slot of its path's or its
/// portal's unit: a connector's content, its cap and its label, a portal
/// end's text and its label. At each owner the caret before its first
/// grapheme and after its last are drawn apart, a range of two graphemes
/// of a longer owner covers glyphs, and no two owners put the caret before
/// their first grapheme in one place (a sibling slot's run is not the
/// owner's).
///
/// Control, seen red: looking the run up by the owner's id alone
/// (`Projector::glyphs_of(owner)`, the code this replaced) finds no run for
/// a label, a cap or a portal end, so the caret falls back to where the
/// owner is drawn whatever the caret is, and the range covers nothing.
#[test]
fn test_the_caret_and_range_are_read_from_each_owners_own_run() {
    use crate::application::view::caret::{caret_place, range_glyphs};
    let doc = text_fixture();
    let view = MapView::for_test(&doc);
    let (a, b) = (node(&doc, 1), node(&doc, 2));
    let connector = child(&doc, b, DocKind::Connector);
    let portal = children_of_kind(doc.doc(), a, DocKind::Link)
        .nth(1)
        .expect("node 1's second link is a portal");
    let end = child(&doc, portal, DocKind::PortalEnd);
    let owners = [
        (
            "the ring",
            child(&doc, child(&doc, a, DocKind::Frame), DocKind::Ring),
            15,
        ),
        ("the connector's content", connector, 1),
        ("the connector's cap", child(&doc, connector, DocKind::CapEnd), 1),
        ("the connector's label", child(&doc, connector, DocKind::Label), 5),
        ("the portal end's text", end, 1),
        ("the portal end's label", child(&doc, end, DocKind::Label), 3),
    ];
    let mut firsts: Vec<(&str, glam::Vec2)> = Vec::new();
    for (what, owner, len) in owners {
        let place = |caret| {
            caret_place(doc.doc(), view.projector(), view.layers(), owner, caret)
                .unwrap_or_else(|| panic!("{what} is drawn"))
                .0
        };
        let (first, last) = (place(0), place(len));
        assert!(
            first.distance(last) > 0.5,
            "{what}: the caret before and after its text is drawn in one place, {first}"
        );
        if len > 2 {
            assert!(
                range_glyphs(doc.doc(), view.projector(), view.layers(), owner, 0, 2).is_some(),
                "{what}: a range of its text covers glyphs"
            );
        }
        for &(other, at) in &firsts {
            assert!(
                first.distance(at) > 0.5,
                "{what} puts its caret where {other} does, {first}"
            );
        }
        firsts.push((what, first));
    }
}

/// The centers of the clusters `owner`'s path run places, in text order,
/// the first copy's — each with its grapheme index in the owner's one
/// span (an ASCII span: a byte is a grapheme) — and the run's advance.
fn cluster_centers(view: &MapView, owner: NodeId) -> (Vec<(usize, glam::Vec2)>, f32) {
    use baumhard::gfx_structs::run::RunShape;
    let (layer, glyph) = view.projector().glyphs_of(owner)[0];
    let run = view
        .layers()
        .tree(layer)
        .get(glyph)
        .and_then(|n| n.get().glyph_run())
        .expect("a run");
    let RunShape::Path { run: shaped, placed } = run.shaped() else { panic!("a path run") };
    let mut centers: Vec<(usize, glam::Vec2)> = Vec::new();
    for g in &placed.glyphs {
        let metrics = &shaped.glyphs[g.glyph as usize];
        if !metrics.grapheme_start || centers.iter().any(|&(k, _)| k == metrics.bytes.start) {
            continue;
        }
        let center = run.origin() + g.pos + glam::Vec2::new(metrics.advance * 0.5, shaped.line_height * 0.5);
        centers.push((metrics.bytes.start, center));
    }
    centers.sort_by_key(|&(k, _)| k);
    let advance = shaped.glyphs.first().map_or(0.0, |g| g.advance);
    (centers, advance)
}

/// **On a ring's reversed line the caret stays between its neighbors**
/// (review of #399): the fixture's ring lays its second line reversed
/// against its path (the readable rule, D33: along the bottom the path
/// runs left, the text right), and for every `k` the caret before grapheme
/// `k` lies on the segment between the centers of clusters `k - 1` and
/// `k` — which a caret that ran the wrong way along a stretch, or sat
/// between the wrong pair, would not.
///
/// Control, seen red: the centers taken in reverse order.
#[test]
fn test_the_caret_on_a_reversed_ring_line_lies_between_its_neighbors() {
    use crate::application::view::caret::caret_place;
    let doc = text_fixture();
    let view = MapView::for_test(&doc);
    let ring = child(&doc, child(&doc, node(&doc, 1), DocKind::Frame), DocKind::Ring);
    let (centers, _) = cluster_centers(&view, ring);
    assert!(centers.len() > 3, "precondition: the ring places its text");
    // The second line runs over the lower-left half, where the clockwise
    // path runs leftward and up: laid reversed, it still reads left to
    // right and down.
    let middle = view.projector().node_rect(node(&doc, 1)).expect("drawn").center();
    let lower: Vec<glam::Vec2> = centers
        .iter()
        .map(|&(_, c)| c)
        .filter(|c| c.y > middle.y + 1.0)
        .collect();
    assert!(
        lower.windows(2).any(|pair| pair[1].x > pair[0].x + 1.0),
        "precondition: the second line is laid along the bottom, reading right"
    );
    for k in 1..centers.len() {
        let (before, after) = (centers[k - 1].1, centers[k].1);
        let at = caret_place(doc.doc(), view.projector(), view.layers(), ring, centers[k].0)
            .expect("placed")
            .0;
        let off = at.distance(before) + at.distance(after) - before.distance(after);
        assert!(
            off < 0.01,
            "caret {k} is not between clusters {} and {k}: {at}",
            k - 1
        );
    }
}

/// **On a `Repeat` path the caret stays with the first copy**: the
/// connector's content `─` is laid copy after copy, and the caret after
/// its one grapheme sits half a cell past the first copy, not past the
/// last.
///
/// Control, seen red: without the walk's stop at a repeated grapheme
/// (`g <= prev`), every copy is a cluster and the caret lands on the last.
#[test]
fn test_the_caret_on_a_repeat_path_stays_with_the_first_copy() {
    use crate::application::view::caret::caret_place;
    use baumhard::gfx_structs::run::RunShape;
    let doc = text_fixture();
    let view = MapView::for_test(&doc);
    let connector = child(&doc, node(&doc, 2), DocKind::Connector);
    let (layer, glyph) = view.projector().glyphs_of(connector)[0];
    let run = view
        .layers()
        .tree(layer)
        .get(glyph)
        .and_then(|n| n.get().glyph_run())
        .expect("a run");
    let RunShape::Path { run: shaped, placed } = run.shaped() else { panic!("a path run") };
    assert!(placed.glyphs.len() > 3, "precondition: the content repeats");
    let advance = shaped.glyphs[placed.glyphs[0].glyph as usize].advance;
    let first =
        run.origin() + placed.glyphs[0].pos + glam::Vec2::new(advance * 0.5, shaped.line_height * 0.5);
    let at = caret_place(doc.doc(), view.projector(), view.layers(), connector, 1)
        .expect("placed")
        .0;
    assert!(
        at.distance(first) <= advance * 0.5 + 0.01,
        "caret 1 is next to the first copy at {first}: {at}"
    );
}

/// **A one-cluster owner's caret steps the way its text reads there**
/// (review of #399): with node 2 moved under node 1 its connector runs
/// down, and the one-glyph cap's carets before and after it sit above and
/// below it, on its column — not beside it.
///
/// Control, seen red: the step taken along `x` whatever the path does.
#[test]
fn test_a_one_cluster_caret_steps_along_a_steep_path() {
    use crate::application::view::caret::caret_place;
    let mut doc = text_fixture();
    let b = node(&doc, 2);
    node_op(&mut doc, b, NodeOp::SetPosition(glam::Vec2::new(0.0, 400.0)));
    let view = MapView::for_test(&doc);
    let connector = child(&doc, b, DocKind::Connector);
    let cap = child(&doc, connector, DocKind::CapEnd);
    let place = |caret| {
        caret_place(doc.doc(), view.projector(), view.layers(), cap, caret)
            .expect("placed")
            .0
    };
    let (before, after) = (place(0), place(1));
    assert!(
        (before.x - after.x).abs() < 0.01 && after.y - before.y > 1.0,
        "above and below the cap: {before} then {after}"
    );
}

/// **While the section editor is open in NodeEdit the status line counts
/// the section being edited** (review of #399): `editing: 1 — section [2
/// of 2]`, from the editor's focus though nothing is selected — and it
/// says nothing of picking or of keys, which an open editor gives other
/// meanings. Control: with the editor closed and nothing of the node
/// selected, the place is `-`.
#[test]
fn test_the_node_edit_status_line_counts_the_section_being_edited() {
    let mut doc = text_fixture();
    let mut view = MapView::for_test(&doc);
    let a = node(&doc, 1);
    let second = children_of_kind(doc.doc(), a, DocKind::Section)
        .nth(1)
        .expect("two sections");
    view.follow_node_edit(&InteractionMode::NodeEdit { node: a });
    doc.selection.select_only(node(&doc, 3));
    assert_eq!(
        view.status(&doc).as_deref(),
        Some("editing: 1 \u{2014} section [- of 2]"),
        "control: nothing of the node focused"
    );
    let editor = TextEditor::Open(Box::new(
        TextSession::open(&mut doc, second, false, false).expect("opens"),
    ));
    view.follow_editor(text_focus(&editor));
    doc.selection.select_only(node(&doc, 3));
    let line = view.status(&doc).expect("a line");
    assert_eq!(line, "editing: 1 \u{2014} section [2 of 2]");
    assert!(!line.contains("pick") && !line.contains("Enter"), "{line}");
}

/// **An outline's sides meet its corners** (review of #399): on boxes of
/// several sizes, in the frames' thin glyphs and the rubber band's, each
/// side's run starts where the top edge's line ends and ends where the
/// bottom edge's line begins — so no side runs past a corner, nor falls
/// short of one — and the edges' corners sit on the left and right
/// strokes.
///
/// Control, seen red: the sides' rows counted by the point size rather
/// than the line height they are laid at (the code this replaced), which
/// left each side a fifth longer than its edges.
#[test]
fn test_an_outlines_sides_meet_its_corners() {
    use crate::application::view::outline::{runs_for, StrokeModel, Strokes};
    use baumhard::gfx_structs::run::RunShape;
    let mut fonts = baumhard::font::fonts::FontSystemLease::new("test");
    let height = |run: &baumhard::gfx_structs::run::GlyphRunData| match run.shaped() {
        RunShape::Box(shaped) => shaped.size.y,
        RunShape::Path { .. } => panic!("an outline run is a box run"),
    };
    for style in [
        &crate::application::view::frames::THIN,
        &crate::application::view::band::BAND,
    ] {
        let model = StrokeModel::of(style, &mut fonts);
        for (w, h) in [(40.0, 10.0), (102.5, 28.8), (300.0, 120.0), (17.0, 61.0)] {
            let strokes = Strokes::around(rect(0.0, 0.0, w, h), model.standing());
            let reach = [(strokes.top, strokes.bottom); 2];
            let [top, bottom, left, right] =
                runs_for(Some(strokes), reach, [true; 4], &model, style, &mut fonts);
            for side in [&left, &right] {
                assert!(
                    (side.origin().y - (top.origin().y + height(&top))).abs() < 0.01,
                    "{w}x{h}: a side starts where the top edge's line ends"
                );
                assert!(
                    (side.origin().y + height(side) - bottom.origin().y).abs() < 0.01,
                    "{w}x{h}: a side ends where the bottom edge's line begins: {} vs {}",
                    side.origin().y + height(side),
                    bottom.origin().y
                );
            }
            assert!(
                (right.origin().x - left.origin().x - (strokes.right - strokes.left)).abs() < 0.01,
                "{w}x{h}: the sides stand on the left and right strokes"
            );
        }
    }
}
