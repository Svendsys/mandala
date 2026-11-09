// SPDX-License-Identifier: MPL-2.0

//! The edit funnel and its animation (V1-D1, #250): every bundled edit,
//! instant and animated, ends in the same map as one history entry that
//! undo takes back; an animation is written on the app clock, yields to
//! another change, and is taken back by an undo as R36 has it.

use baumhard::arena::NodeId;
use baumhard::document::edit::EditDef;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::writer::ChangeSource;
use glam::Vec2;

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::tests_common::testament;
use crate::application::document::MindMapDocument;
use crate::application::edits::EditRegistry;
use crate::application::view::MapView;

use super::animation;
use super::dispatch::families::edits::{run_edit, Undrawn};

/// The bundled edits; with `instant`, each with its animation taken off.
fn bundled(instant: bool) -> EditRegistry {
    let mut defs: Vec<EditDef> =
        baumhard::document::edit::parse_edits(include_str!("../../../assets/edits.json"))
            .expect("the bundled edits read");
    if instant {
        for def in &mut defs {
            def.animate = None;
        }
    }
    EditRegistry::new(defs, Vec::new())
}

/// The first top-level node of `doc` with a child node that has a child
/// of its own: somewhere every bundled edit has something to do.
fn a_family(doc: &MindMapDocument) -> NodeId {
    let d = doc.doc();
    let nodes = |id: NodeId| {
        d.children(id)
            .filter(move |&c| d.get(c).is_some_and(|e| e.kind() == DocKind::Node))
    };
    nodes(d.root())
        .find(|&top| nodes(top).any(|child| nodes(child).next().is_some()))
        .expect("testament has a node with grandchildren")
}

/// The map, a selected family, and a view to read where it is drawn.
fn setup() -> (MindMapDocument, MapView) {
    let mut doc = testament();
    let family = a_family(&doc);
    doc.selection.select_only(family);
    let view = MapView::for_test(&doc);
    (doc, view)
}

/// Run `id` at the selection from `registry`, at `now_ms`.
fn run(doc: &mut MindMapDocument, view: &mut MapView, registry: &EditRegistry, id: &str, now_ms: f64) {
    run_edit(doc, view, registry, id, None, ChangeSource::Edit, now_ms)
        .unwrap_or_else(|error| panic!("{id} runs: {error}"));
}

/// The map as saved.
fn saved(doc: &mut MindMapDocument) -> String {
    doc.save_json().expect("the map saves")
}

const BUNDLED: [&str; 5] = [
    "grow-font-2pt",
    "shrink-font-2pt",
    "nudge-right-50",
    "flower",
    "cascade",
];

#[test]
fn test_each_bundled_edit_instant_and_animated_ends_the_same_as_one_entry() {
    let (instant, animated) = (bundled(true), bundled(false));
    for id in BUNDLED {
        let (mut a, mut va) = setup();
        let (mut b, mut vb) = setup();
        let before = saved(&mut a);
        run(&mut a, &mut va, &instant, id, 0.0);
        assert!(!a.is_animating(), "{id}: instant");
        let ms = f64::from(animated.find(b.doc(), id).unwrap().0.animate.unwrap().ms);
        run(&mut b, &mut vb, &animated, id, 0.0);
        assert!(b.is_animating(), "{id}: animated");
        assert!(b.tick_animation(ms / 3.0));
        assert_eq!(b.doc().history().count(), 0, "{id}: no entry while it runs");
        assert!(b.tick_animation(ms));
        assert!(!b.is_animating(), "{id}: done at its end");
        let (after_a, after_b) = (saved(&mut a), saved(&mut b));
        assert_ne!(after_a, before, "{id}: the edit changed the map");
        assert_eq!(after_a, after_b, "{id}: instant and animated end in the same map");
        assert_eq!(a.doc().history().count(), 1, "{id}: one entry, instant");
        assert_eq!(b.doc().history().count(), 1, "{id}: one entry, animated");
        a.undo();
        b.undo();
        assert_eq!(saved(&mut a), before, "{id}: undo restores the map");
        assert_eq!(saved(&mut b), before, "{id}: undo restores the map");
    }
}

/// The position of `node`.
fn position(doc: &MindMapDocument, node: NodeId) -> Vec2 {
    match doc.doc().get(node) {
        Some(DocElement::Node(data)) => data.position,
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_under_the_virtual_clock_an_edit_lands_on_its_eased_curve() {
    let (mut doc, mut view) = setup();
    let registry = bundled(false);
    let node = a_family(&doc);
    let x0 = position(&doc, node).x;
    let mut clock = AppClock::default();
    clock.set_mode(ClockMode::Virtual);
    let start = clock.now_ms();
    run(&mut doc, &mut view, &registry, "nudge-right-50", start);
    let def = registry.find(doc.doc(), "nudge-right-50").unwrap().0;
    let animate = def.animate.unwrap();
    assert_eq!(animate.ms, 150);
    for fraction in [0.25_f64, 0.5, 0.75] {
        clock.advance_to(start + fraction * 150.0);
        assert!(animation::tick(Some(&mut doc), &clock));
        let eased = animate.easing.ease(fraction as f32);
        let x = position(&doc, node).x;
        assert!(
            (x - (x0 + 50.0 * eased)).abs() < 1e-3,
            "at {fraction}: {x}, eased {eased}"
        );
        // Control: the eased curve is not the linear one here, so a
        // tick that ignored the easing would fail the row above.
        assert!((eased - fraction as f32).abs() > 1e-3 || fraction == 0.5);
    }
    // A virtual clock keeps no loop awake; a real one does while the
    // edit runs, and neither once it is done.
    assert!(!animation::keeps_ticking(Some(&doc), &clock));
    assert!(animation::keeps_ticking(Some(&doc), &AppClock::default()));
    clock.advance_to(start + 150.0);
    assert!(animation::tick(Some(&mut doc), &clock));
    assert_eq!(position(&doc, node).x, x0 + 50.0);
    assert!(!animation::keeps_ticking(Some(&doc), &AppClock::default()));
    assert!(!animation::tick(Some(&mut doc), &clock), "nothing left to write");
}

#[test]
fn test_a_delay_holds_the_start() {
    let (mut doc, mut view) = setup();
    let mut defs: Vec<EditDef> = baumhard::document::edit::parse_edits(
        r#"{"edits":[{"id":"late","name":"Late","reach":"self","target":"node","mutations":[{"node":{"nudge":[100.0,0.0]}}],"animate":{"ms":100,"delay_ms":40}}]}"#,
    )
    .unwrap();
    let registry = EditRegistry::new(std::mem::take(&mut defs), Vec::new());
    let node = a_family(&doc);
    let x0 = position(&doc, node).x;
    run(&mut doc, &mut view, &registry, "late", 0.0);
    doc.tick_animation(30.0);
    assert_eq!(position(&doc, node).x, x0, "held through the delay");
    doc.tick_animation(90.0);
    assert!(
        (position(&doc, node).x - (x0 + 50.0)).abs() < 1e-3,
        "halfway after it"
    );
}

#[test]
fn test_undo_mid_animation_leaves_the_map_as_it_was_and_redo_brings_the_edit_back() {
    let (mut doc, mut view) = setup();
    let registry = bundled(false);
    let before = saved(&mut doc);
    let family = a_family(&doc);
    let children: Vec<NodeId> = doc
        .doc()
        .children(family)
        .filter(|&c| matches!(doc.doc().get(c), Some(DocElement::Node(_))))
        .collect();
    let at = |doc: &MindMapDocument| children.iter().map(|&c| position(doc, c)).collect::<Vec<_>>();
    let start = at(&doc);
    run(&mut doc, &mut view, &registry, "flower", 0.0);
    doc.tick_animation(100.0);
    assert_ne!(at(&doc), start, "control: part-way, the children have moved");
    assert_eq!(
        saved(&mut doc),
        before,
        "a save part-way writes the committed map"
    );
    doc.undo();
    assert_eq!(at(&doc), start, "the undo put them back");
    assert!(!doc.is_animating(), "the undo ended the animation");
    assert_eq!(saved(&mut doc), before);
    assert_eq!(doc.doc().history().count(), 0, "no entry left");
    assert_eq!(
        doc.doc().redo_len(),
        1,
        "R36: the edit was committed, then undone"
    );
    doc.redo();
    let (mut whole, mut wv) = setup();
    run(&mut whole, &mut wv, &bundled(true), "flower", 0.0);
    assert_eq!(
        saved(&mut doc),
        saved(&mut whole),
        "redo brings the whole edit back"
    );
}

#[test]
fn test_a_second_edit_mid_animation_finishes_the_first() {
    let (mut doc, mut view) = setup();
    let registry = bundled(false);
    run(&mut doc, &mut view, &registry, "nudge-right-50", 0.0);
    doc.tick_animation(50.0);
    run(&mut doc, &mut view, &registry, "grow-font-2pt", 50.0);
    assert_eq!(
        doc.doc().history().count(),
        1,
        "the first, finished and committed"
    );
    doc.tick_animation(1000.0);
    assert_eq!(doc.doc().history().count(), 2, "then the second");
    // The first ended where it would have: 50 to the right.
    let (mut both, mut bv) = setup();
    let instant = bundled(true);
    run(&mut both, &mut bv, &instant, "nudge-right-50", 0.0);
    run(&mut both, &mut bv, &instant, "grow-font-2pt", 0.0);
    assert_eq!(saved(&mut doc), saved(&mut both));
}

#[test]
fn test_an_unknown_edit_is_refused_and_changes_nothing() {
    let (mut doc, mut view) = setup();
    let revision = doc.revision();
    let error = run_edit(
        &mut doc,
        &mut view,
        &bundled(false),
        "spiral",
        None,
        ChangeSource::Edit,
        0.0,
    )
    .unwrap_err();
    assert_eq!(error, "no edit is called `spiral`");
    assert_eq!(doc.revision(), revision);
}

/// A map with one span at 10 pt and two edits of its own: `grow` (10 pt
/// over 200 ms) and `tint`, which reds the spans above 17 pt.
const GROW_THEN_TINT: &str = r##"{"format":1,"name":"t",
"edits":[
 {"id":"grow","name":"Grow","reach":"self","target":"spans","mutations":[{"span":{"text":{"grow_size":10.0}}}],"animate":{"ms":200}},
 {"id":"tint","name":"Tint","reach":"self","target":"spans","filter":{"fields":[[{"font_size":{"pt":17.0}},{"GreaterThan":false}]]},"mutations":[{"span":{"text":{"set_color":"#ff0000"}}}]}],
"nodes":[{"id":1,"position":[0,0],"sections":[{"spans":[{"text":"x","size":10.0}]}]}]}"##;

/// **A second edit reads the map the first leaves** (review of #331,
/// blocker 3). `tint`, run while `grow` is half way (15 pt), is resolved
/// only after `grow` is fast-forwarded and committed (20 pt), so its
/// filter reaches the span: two entries, and the span red — the same map
/// as the two run one after the other.
///
/// Control: resolving before the fast-forward read 15 pt, reached
/// nothing, and lost the `tint` with no entry and no word.
#[test]
fn test_an_edit_run_mid_animation_reads_the_map_the_first_leaves() {
    let load = || {
        let mut doc = MindMapDocument::from_json_str(GROW_THEN_TINT, None, 50).expect("the map loads");
        let node = doc
            .doc()
            .node_by_ident(baumhard::document::ids::NodeIdent(1))
            .unwrap();
        doc.selection.select_only(node);
        doc
    };
    let registry = EditRegistry::new(Vec::new(), Vec::new());
    let run = |doc: &mut MindMapDocument, id: &str, now_ms: f64| {
        run_edit(doc, &mut Undrawn, &registry, id, None, ChangeSource::Edit, now_ms)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
    };
    let mut mid = load();
    run(&mut mid, "grow", 0.0);
    mid.tick_animation(100.0);
    run(&mut mid, "tint", 100.0);
    assert_eq!(mid.doc().history().count(), 2, "grow's entry, then tint's");

    let mut after = load();
    run(&mut after, "grow", 0.0);
    after.tick_animation(1000.0);
    run(&mut after, "tint", 1000.0);
    assert_eq!(saved(&mut mid), saved(&mut after));
    assert!(saved(&mut mid).contains("#ff0000"), "the span is red");
}

/// **A handler reads the map as drawn after the step before it** (review
/// of #331, blocker 3). A macro of `nudge-right-50` then `flower` has no
/// frame between its steps; the flower's first petal still sits straight
/// above the parent where the nudge left it, because the funnel syncs
/// the view before it resolves.
///
/// Control: without the sync the petal sat above the parent's old,
/// 50-unit-left rect.
#[test]
fn test_a_handler_after_a_step_reads_the_view_that_step_left() {
    let (mut doc, mut view) = setup();
    let registry = bundled(true);
    let parent = a_family(&doc);
    run(&mut doc, &mut view, &registry, "nudge-right-50", 0.0);
    run(&mut doc, &mut view, &registry, "flower", 0.0);
    let _ = view.sync_for_test(&mut doc);
    let geometry = view.projector();
    let center = |node: NodeId| {
        let rect = baumhard::projection::Geometry::node_rect(geometry, node).expect("drawn");
        rect.origin + rect.size / 2.0
    };
    let petal = doc
        .doc()
        .children(parent)
        .find(|&c| doc.doc().get(c).is_some_and(|e| e.kind() == DocKind::Node))
        .expect("the parent has a child node");
    let (petal, parent) = (center(petal), center(parent));
    assert!(petal.y < parent.y, "the first petal is above its parent");
    assert!(
        (petal.x - parent.x).abs() < 0.5,
        "straight above the parent as moved: {petal} over {parent}"
    );
}

/// Every node of `doc`, with its position. Iterative.
fn nodes_of(doc: &MindMapDocument) -> Vec<(NodeId, Vec2)> {
    let d = doc.doc();
    let mut out = Vec::new();
    let mut pending = vec![d.root()];
    while let Some(id) = pending.pop() {
        for child in d.children(id) {
            if let Some(DocElement::Node(data)) = d.get(child) {
                out.push((child, data.position));
                pending.push(child);
            }
        }
    }
    out
}

/// Run `id` at `anchors` from the bundled edits, instantly, and the
/// nodes it moved that are drawn over another drawn node, with that
/// node. A shared edge is not an overlap.
fn overlaps_after(
    doc: &mut MindMapDocument,
    view: &mut MapView,
    id: &str,
    anchors: &[NodeId],
) -> Vec<(NodeId, NodeId)> {
    let before = nodes_of(doc);
    run_edit(
        doc,
        view,
        &bundled(true),
        id,
        Some(anchors),
        ChangeSource::Edit,
        0.0,
    )
    .unwrap_or_else(|error| panic!("{id}: {error}"));
    let _ = view.sync_for_test(doc);
    let after = nodes_of(doc);
    let moved: Vec<NodeId> = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b != a)
        .map(|(_, (node, _))| *node)
        .collect();
    assert!(
        !moved.is_empty(),
        "{id}: precondition: the layout moved something"
    );
    let rect = |node| baumhard::projection::Geometry::node_rect(view.projector(), node);
    let drawn: Vec<(NodeId, baumhard::flow::Rect)> = after
        .iter()
        .filter_map(|&(node, _)| rect(node).map(|r| (node, r)))
        .collect();
    let meets = |a: &baumhard::flow::Rect, b: &baumhard::flow::Rect| {
        a.origin.x < b.max().x && b.origin.x < a.max().x && a.origin.y < b.max().y && b.origin.y < a.max().y
    };
    let mut found = Vec::new();
    for &node in &moved {
        let Some(mine) = rect(node) else { continue };
        found.extend(
            drawn
                .iter()
                .filter(|(other, r)| *other != node && meets(&mine, r))
                .map(|(other, _)| (node, *other)),
        );
    }
    found
}

/// **Testament's Ishmael flower lays no son over another node** (owner,
/// on #250): on `main`'s placement Hadad and Massa landed on Hagar,
/// Ishmael's mother. Read through the real projection.
///
/// Control: `main`'s placement went red here.
#[test]
fn test_ishmael_flower_lays_no_son_over_another_node() {
    let (mut doc, mut view) = setup();
    let ishmael = doc
        .doc()
        .node_by_ident(baumhard::document::ids::NodeIdent(52))
        .expect("Ishmael");
    let found = overlaps_after(&mut doc, &mut view, "flower", &[ishmael]);
    assert!(found.is_empty(), "{} overlaps: {found:?}", found.len());
}

/// **Over the whole of testament at once, neither layout lays a node over
/// another** (owner, on #250): `flower` and `cascade`, each run with
/// every node that has a child as an anchor — nested, dense, the map's
/// every family — leave no moved node drawn over any other.
///
/// Control: `main`'s placement went red here.
#[test]
fn test_layouts_at_every_family_lay_no_node_over_another() {
    for id in ["flower", "cascade"] {
        let (mut doc, mut view) = setup();
        let d = doc.doc();
        let families: Vec<NodeId> = nodes_of(&doc)
            .into_iter()
            .map(|(node, _)| node)
            .filter(|&node| {
                d.children(node)
                    .any(|c| d.get(c).is_some_and(|e| e.kind() == DocKind::Node))
            })
            .collect();
        let found = overlaps_after(&mut doc, &mut view, id, &families);
        assert!(
            found.is_empty(),
            "{id}: {} overlaps, first {:?}",
            found.len(),
            found.first()
        );
    }
}
