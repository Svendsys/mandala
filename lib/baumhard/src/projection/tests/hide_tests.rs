// SPDX-License-Identifier: MPL-2.0

//! Nodes hide the paths behind them (#332): a connector's glyph is left
//! out where it would come within its own gap of a node it does not end
//! at, and comes back when the node moves away; a move re-hides only the
//! paths whose glyphs the node's old or new region meets. Glyph inks are
//! read from the runs' metrics here, not through the projector's own
//! reading of them, and every test ends on the oracle.

use glam::Vec2;
use rustc_hash::FxHashSet;
use serde_json::json;

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::mutation::ops::{DocMutation, NodeOp};
use crate::document::tests::writer_support::{request, single};
use crate::flow::{Curve, Rect};
use crate::gfx_structs::run::{GlyphRunData, RunShape};
use crate::projection::state::UnitBody;
use crate::projection::tests::maps::{ident, kid, load_map, merged, node, two_paths, AWAY, ON_P, ON_Q};
use crate::projection::tests::support::Projected;
use crate::projection::{Layer, Projector, Stage};

/// The canvas box of each inked glyph of a path run: its pen the run's
/// baseline below its box's corner, its ink about the pen as shaped.
fn inks(run: &GlyphRunData) -> Vec<Rect> {
    let RunShape::Path { run: shaped, placed } = run.shaped() else {
        panic!("a path run");
    };
    placed
        .glyphs
        .iter()
        .filter_map(|g| {
            let ink = shaped.glyphs[g.glyph as usize].ink?;
            let pen = run.origin() + g.pos + Vec2::new(0.0, shaped.baseline());
            Some(Rect::new(pen + ink.min, ink.max - ink.min))
        })
        .collect()
}

/// The run of `element` the connections layer holds.
fn drawn(p: &Projected, element: NodeId) -> GlyphRunData {
    let (layer, glyph) = p
        .projector
        .glyphs_of(element)
        .into_iter()
        .find(|(l, _)| *l == Layer::Connections)
        .expect("the path's body is drawn");
    let tree = p.layers.tree(layer);
    tree.arena()[glyph].get().glyph_run().expect("a run").clone()
}

/// Whether `a` and `b` meet, edges included.
fn meet(a: &Rect, b: &Rect) -> bool {
    a.origin.x <= b.max().x && b.origin.x <= a.max().x && a.origin.y <= b.max().y && b.origin.y <= a.max().y
}

/// The gap between two boxes, 0 where they meet.
fn box_gap(a: &Rect, b: &Rect) -> f32 {
    (b.origin - a.max())
        .max(a.origin - b.max())
        .max(Vec2::ZERO)
        .length()
}

/// The path's own gap, the margin it is hidden by, measured here: the
/// median distance between its consecutive laid inks, box to box — not
/// read from how `hide` reckons it. A `repeat` path spreads its slack over
/// its gaps, so this is its font's gap and a sliver more: the checks that
/// read it allow a twentieth either way.
fn own_gap(projector: &Projector, path: NodeId) -> f32 {
    let laid = inks(projector.laid(path).expect("laid"));
    let mut gaps: Vec<f32> = laid.windows(2).map(|w| box_gap(&w[0], &w[1])).collect();
    gaps.sort_by(f32::total_cmp);
    let gap = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
    assert!(gap > 0.0, "precondition: the path's glyphs keep a gap, {gap}");
    gap
}

/// The connectors whose hide stage ran in the last call.
fn hidden_again(p: &Projected) -> FxHashSet<NodeId> {
    p.projector
        .ran
        .iter()
        .filter(|d| d.stage == Stage::Hide)
        .map(|d| d.unit)
        .filter(|&u| p.doc.get(u).is_some_and(|e| e.kind() == DocKind::Connector))
        .collect()
}

/// Move node `id` to `to`.
fn move_to(p: &mut Projected, id: u32, to: Vec2) {
    let node = ident(&p.doc, id);
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(to)));
    p.apply(request(mutation, vec![node], "move")).unwrap();
}

/// How many of `path`'s laid glyphs its layer leaves out, having held
/// that they are the ones within its gap of `region`: every glyph drawn
/// is one laid, and keeps clear of the region by its gap; every glyph
/// left out comes within its gap of it.
fn hides_exactly(p: &Projected, path: NodeId, region: Rect) -> usize {
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).expect("laid"));
    let drawn = inks(&drawn(p, path));
    for ink in &drawn {
        assert!(
            laid.contains(ink),
            "a drawn glyph at {} is one laid",
            ink.center()
        );
        assert!(
            !meet(&ink.grown(gap * 0.95), &region),
            "a drawn glyph at {} within the gap {gap} of the node at {region:?}",
            ink.center()
        );
    }
    let hidden: Vec<&Rect> = laid.iter().filter(|i| !drawn.contains(i)).collect();
    for ink in &hidden {
        assert!(
            meet(&ink.grown(gap * 1.05), &region),
            "a glyph at {} left out, not within the gap {gap} of the node at {region:?}",
            ink.center()
        );
    }
    hidden.len()
}

/// A connector across a node it does not end at draws no glyph within its
/// own gap of the node, and every other glyph, on both sides of it.
///
/// Control: with `hide` leaving every glyph in, red on the drawn glyph
/// within the gap.
#[test]
fn test_projection_a_node_hides_the_glyphs_of_a_path_behind_it() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let blocker = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    let behind = hides_exactly(&p, path, blocker);
    assert!(
        behind >= 3,
        "precondition: the path runs behind the node, {behind} glyphs"
    );
    let shown = inks(&drawn(&p, path));
    for (side, x) in [("left", blocker.origin.x), ("right", blocker.max().x)] {
        assert!(
            shown.iter().any(|i| (i.center().x < x) == (side == "left")),
            "the path is drawn on the node's {side}"
        );
    }
    let other = kid(&p.doc, ident(&p.doc, 5), DocKind::Connector);
    assert_eq!(
        inks(&drawn(&p, other)).len(),
        inks(p.projector.laid(other).unwrap()).len(),
        "the path the node is not on is drawn whole"
    );
    p.check().unwrap();
}

/// A node moved off a path shows its glyphs again, without the path being
/// laid again: the hide stage is a view over the placement. Only the path
/// its old region met is hidden again.
///
/// Control: with `hide` recording no reads, red on the oracle and the
/// glyphs shown — nothing marks the path when the node leaves it.
#[test]
fn test_projection_a_node_moved_off_a_path_shows_its_glyphs_again() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let before = inks(&drawn(&p, path)).len();
    move_to(&mut p, 3, AWAY);
    let laid = inks(p.projector.laid(path).unwrap()).len();
    assert!(
        before < laid,
        "precondition: the node hid glyphs, {before} of {laid} drawn"
    );
    assert_eq!(inks(&drawn(&p, path)).len(), laid, "every glyph is drawn again");
    let again = hidden_again(&p);
    assert_eq!(
        again,
        [path].into_iter().collect(),
        "only the path it left is hidden again"
    );
    assert!(
        !p.projector
            .ran
            .iter()
            .any(|d| d.unit == path && d.stage == Stage::Place),
        "the path is not laid again"
    );
    p.check().unwrap();
}

/// A node moved onto a path hides its glyphs there, and re-hides that
/// path alone; moved on from it to the other path, it re-hides the two —
/// the one its old region met and the one its new region meets — and the
/// first is drawn whole again.
///
/// Controls: with `rehide` finding no path in the paths' index, red on
/// the glyphs hidden behind it; with it queuing every path, red on the
/// path it is not on hidden again.
#[test]
fn test_projection_a_node_moved_onto_a_path_rehides_only_the_paths_it_meets() {
    let mut p = Projected::new(two_paths(AWAY, json!({}), json!({})));
    let (one, two) = (
        kid(&p.doc, ident(&p.doc, 2), DocKind::Connector),
        kid(&p.doc, ident(&p.doc, 5), DocKind::Connector),
    );
    move_to(&mut p, 3, ON_P);
    let blocker = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    assert!(
        hides_exactly(&p, one, blocker) >= 3,
        "the path it moved onto is hidden"
    );
    assert_eq!(hidden_again(&p), [one].into_iter().collect(), "onto P: P alone");
    p.check().unwrap();
    move_to(&mut p, 3, ON_Q);
    let blocker = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    assert!(
        hides_exactly(&p, two, blocker) >= 3,
        "the path it moved onto is hidden"
    );
    assert_eq!(hides_exactly(&p, one, blocker), 0, "the path it left is whole");
    assert_eq!(
        hidden_again(&p),
        [one, two].into_iter().collect(),
        "from P to Q: both"
    );
    p.check().unwrap();
}

/// A path's own end nodes never hide it: its end glyphs and its caps sit
/// within its gap of the frames they stop at, and are drawn.
///
/// Control: with `hide` not sparing the ends, red on the glyphs drawn.
#[test]
fn test_projection_a_paths_own_ends_never_hide_it() {
    let caps = json!({"cap_start": {"spans": [{"text": "◆"}]}, "cap_end": {"spans": [{"text": "◆"}]}});
    let mut p = Projected::new(two_paths(AWAY, json!({}), caps));
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let gap = own_gap(&p.projector, path);
    let ends = [ident(&p.doc, 1), ident(&p.doc, 2)].map(|n| p.projector.node_rect(n).unwrap());
    let laid = inks(p.projector.laid(path).unwrap());
    for (k, end) in ends.iter().enumerate() {
        assert!(
            laid.iter().any(|i| meet(&i.grown(gap * 1.25), end)),
            "precondition: its end glyph comes within its gap of end {k}, as rule 5 sets it (0.8 to 1.2 gaps)"
        );
    }
    assert_eq!(
        inks(&drawn(&p, path)).len(),
        laid.len(),
        "every glyph of the body is drawn"
    );
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("a line");
    };
    for k in [1, 2] {
        let cap = line.laid[k].as_ref().expect("a cap is laid");
        assert!(
            ends.iter().any(|end| inks(cap).iter().any(|i| meet(i, end))),
            "precondition: cap {k} sits on its end's frame"
        );
        assert_eq!(
            p.projector.units[&path].slots[k].want.as_ref(),
            Some(cap),
            "cap {k} is drawn whole"
        );
    }
    p.check().unwrap();
}

/// A node's region is the box round its rectangle and its rings' inks: a
/// path that passes over its ring and clear of its rectangle is hidden
/// there.
///
/// Control: with the region the node's rectangle alone, red on the glyphs
/// hidden.
#[test]
fn test_projection_a_nodes_ring_hides_a_path_as_its_text_does() {
    // The blocker's rectangle runs from y = 26 down, past the gap of the
    // path along y = 20; its ring of dots, 6 further out, reaches over it.
    let ring = json!({"inset": 4.0, "rings": [{"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]}]});
    let doc = two_paths(Vec2::new(480.0, 26.0), json!({"frame": ring}), json!({}));
    let blocker = ident(&doc, 3);
    let mut p = Projected::new(doc);
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let rect = p.projector.node_rect(blocker).unwrap();
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).unwrap());
    assert!(
        !laid.iter().any(|i| meet(&i.grown(gap), &rect)),
        "precondition: the path keeps its gap from the rectangle"
    );
    let reach = p.projector.reach(blocker).unwrap();
    assert!(
        reach.origin.y < rect.origin.y,
        "precondition: the ring reaches above the rectangle"
    );
    assert!(hides_exactly(&p, path, reach) >= 1, "the ring hides the path");
    p.check().unwrap();
}

/// An ellipse hides by its outline, from both sides: a circle centered
/// on a diagonal path hides the glyphs that come within the path's gap of
/// the circle, and draws those inside its box but clear of the circle,
/// where the diagonal crosses the box's corners. The circle is tested here
/// on its own terms: the nearest point of each grown ink to its center,
/// inside its radius or out.
///
/// Controls: with the region shaped as a rectangle, red on the glyphs
/// drawn in the corners; with no region set for an ellipse, red on the
/// glyphs hidden inside it.
#[test]
fn test_projection_an_ellipse_hides_by_its_outline() {
    let circle = json!({"min_size": [120.0, 120.0], "shape": "ellipse"});
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"min_size": [100.0, 40.0]})),
        node(2, Some(1), json!({"position": [900.0, 900.0], "min_size": [100.0, 40.0], "connector": {"straight": true}})),
        node(3, None, merged(json!({"position": [AWAY.x, AWAY.y]}), circle)),
    ]}));
    let mut p = Projected::new(doc);
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let Some(&[Curve::Line(a, b)]) = p.projector.curve(path) else {
        panic!("a straight path");
    };
    move_to(&mut p, 3, (a + b) * 0.5 - Vec2::splat(60.0));
    let rect = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    let (center, half) = (rect.center(), rect.size * 0.5);
    let gap = own_gap(&p.projector, path);
    let drawn = inks(&drawn(&p, path));
    let (mut inside, mut corner) = (0, 0);
    for ink in inks(p.projector.laid(path).unwrap()) {
        let grown = ink.grown(gap);
        let nearest = center.clamp(grown.origin, grown.max());
        let reach = ((nearest - center) / half).length();
        if reach <= 0.95 {
            inside += 1;
            assert!(
                !drawn.contains(&ink),
                "a glyph at {} inside the circle is drawn",
                ink.center()
            );
        } else if reach >= 1.05 && meet(&grown, &rect) {
            corner += 1;
            assert!(
                drawn.contains(&ink),
                "a glyph at {} past the circle, in its box, is left out",
                ink.center()
            );
        }
    }
    assert!(
        inside >= 3,
        "precondition: the path runs through the circle, {inside} glyphs"
    );
    assert!(
        corner >= 1,
        "precondition: the path crosses the box's corners, {corner} glyphs"
    );
    p.check().unwrap();
}

/// A node that stops being drawn — hidden — shows the glyphs it hid.
///
/// Control: with `rehide` keeping a node that left in the regions' index,
/// red on the glyphs shown and the oracle.
#[test]
fn test_projection_a_node_hidden_shows_the_path_behind_it() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let blocker = ident(&p.doc, 3);
    let laid = inks(p.projector.laid(path).unwrap()).len();
    assert!(
        inks(&drawn(&p, path)).len() < laid,
        "precondition: the node hides glyphs"
    );
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(true)));
    p.apply(request(mutation, vec![blocker], "hide")).unwrap();
    assert_eq!(inks(&drawn(&p, path)).len(), laid, "every glyph is drawn again");
    p.check().unwrap();
}

/// A path whose two nodes both lie inside a third node's region is hidden
/// whole: the region is the box round the node's rectangle and rings, and
/// a container's empty inside is in it. This pins the rule as built while
/// the owner's question 1 on #334 (hide by the box, or by what the node
/// draws) is open.
///
/// Control: with `hide` leaving every glyph in, red on the glyphs drawn.
#[test]
fn test_projection_a_path_inside_another_nodes_region_is_hidden_whole() {
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"min_size": [100.0, 40.0]})),
        node(2, Some(1), json!({"position": [1000.0, 0.0], "min_size": [100.0, 40.0], "connector": {"straight": true}})),
        node(3, None, json!({"position": [-200.0, -280.0], "min_size": [1600.0, 600.0]})),
    ]}));
    let mut p = Projected::new(doc);
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let laid = inks(p.projector.laid(path).unwrap()).len();
    assert!(laid >= 100, "precondition: the path is laid, {laid} glyphs");
    assert_eq!(inks(&drawn(&p, path)).len(), 0, "no glyph of the path is drawn");
    p.check().unwrap();
}

/// A node hides the path behind it whatever its opacity and its zoom
/// window: at opacity 0, and when it renders only zoomed far in. A run has
/// one zoom window, so a glyph cannot be hidden only while the node shows.
/// This pins the rule as built while the owner's question 3 on #334 is
/// open.
///
/// Control: with `hide` sparing a node at opacity 0 or with a zoom window,
/// red on each.
#[test]
fn test_projection_a_node_hides_whatever_its_opacity_and_zoom() {
    for blocker in [json!({"opacity": 0.0}), json!({"zoom": {"min": 50.0}})] {
        let mut p = Projected::new(two_paths(ON_P, blocker.clone(), json!({})));
        let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
        let rect = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
        assert!(
            hides_exactly(&p, path, rect) >= 3,
            "a node of {blocker} hides the path"
        );
        p.check().unwrap();
    }
}

/// A path dropped with its node leaves the paths' index, and a node
/// deleted leaves the regions, as a full projection holds them: the
/// oracle compares both indices.
///
/// Control: with `drop_unit` not calling `unhide`, red on the oracle's
/// path cells.
#[test]
fn test_projection_a_deleted_path_and_node_leave_the_indices() {
    use crate::document::mutation::compose::delete;
    use crate::document::writer::EditRequest;
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    p.check().unwrap();
    for id in [2, 3] {
        let node = ident(&p.doc, id);
        let steps = delete(&p.doc, &[node]).unwrap();
        p.apply(EditRequest::new(steps, "delete")).unwrap();
        p.check().unwrap();
    }
}

/// Paths that leave one hub share its cells in the paths' index; one of
/// them hidden again leaves those cells and joins them anew, and the
/// others' entries there stay pointed at right: the oracle compares the
/// index, back-pointers and all, after the middle one is hidden and shown.
///
/// Control: with `PathCells::remove` not pointing the entry it moves at
/// its new place, red on the oracle's path cells.
#[test]
fn test_projection_paths_that_share_a_hubs_cells_are_hidden_one_by_one() {
    let child = |id: u32, y: f32| {
        node(
            id,
            Some(1),
            json!({"position": [1000.0, y], "min_size": [100.0, 40.0], "connector": {}}),
        )
    };
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"min_size": [100.0, 40.0]})),
        child(2, -300.0),
        child(3, 0.0),
        child(4, 300.0),
        node(5, None, json!({"position": [AWAY.x, AWAY.y], "min_size": [60.0, 60.0]})),
    ]}));
    let mut p = Projected::new(doc);
    let middle = kid(&p.doc, ident(&p.doc, 3), DocKind::Connector);
    move_to(&mut p, 5, ON_P);
    assert!(
        hidden_again(&p).contains(&middle),
        "precondition: the middle path is hidden again"
    );
    p.check().unwrap();
    move_to(&mut p, 5, AWAY);
    p.check().unwrap();
}

/// A node whose region spans more grid cells than hold any path glyph
/// finds the paths it meets by scanning those cells, not by walking its
/// own: moved onto P, a node 3000 wide hides it.
///
/// Control: with the scan finding nothing, red on the glyphs hidden.
#[test]
fn test_projection_a_node_wider_than_the_paths_cells_finds_them_by_scanning() {
    let wide = json!({"min_size": [3000.0, 3000.0]});
    let mut p = Projected::new(two_paths(Vec2::new(-1000.0, 3000.0), wide, json!({})));
    let path = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    move_to(&mut p, 3, Vec2::new(-1000.0, 10.0));
    let rect = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    assert!(hides_exactly(&p, path, rect) >= 3, "the wide node hides the path");
    p.check().unwrap();
}
