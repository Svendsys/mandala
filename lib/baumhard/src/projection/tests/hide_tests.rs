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
use crate::projection::stages::gap_and_size;
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

/// The path's own gap, the margin it is hidden by.
fn own_gap(projector: &Projector, path: NodeId) -> f32 {
    let Some(UnitBody::Line(line)) = projector.units.get(&path).map(|u| &u.body) else {
        panic!("{path:?} is a line");
    };
    gap_and_size(line).0
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

/// How many of `path`'s laid glyphs, grown by its gap, meet `region`;
/// and that every drawn glyph keeps clear of it while every laid one
/// that keeps clear is drawn: the hidden glyphs are exactly those.
fn hides_exactly(p: &Projected, path: NodeId, region: Rect) -> usize {
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).expect("laid"));
    let drawn = inks(&drawn(p, path));
    let behind = laid.iter().filter(|i| meet(&i.grown(gap), &region)).count();
    for ink in &drawn {
        assert!(
            !meet(&ink.grown(gap), &region),
            "a drawn glyph at {} within the gap {gap} of the node at {region:?}",
            ink.center()
        );
    }
    assert_eq!(
        drawn.len(),
        laid.len() - behind,
        "every glyph clear of the node is drawn: {} laid, {behind} behind it",
        laid.len()
    );
    behind
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
            laid.iter().any(|i| meet(&i.grown(gap), end)),
            "precondition: a glyph comes within the gap of end {k}"
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

/// An ellipse hides by its outline: a path past the corner of its box,
/// outside the ellipse, is drawn there.
///
/// Control: with the region shaped as a rectangle, red on the glyphs
/// drawn.
#[test]
fn test_projection_an_ellipse_hides_by_its_outline() {
    // A circle 120 across, moved beside a diagonal path: its center 72 off
    // the path's line, 12 past its edge, and its box's corner 85 out
    // along the diagonal, 13 across the line.
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
    let normal = (b - a).perp().normalize();
    let toward = Vec2::new(-normal.x.signum(), -normal.y.signum());
    assert!(
        toward.dot(-normal) > 1.4,
        "precondition: the line runs near the diagonal, its normal {normal}"
    );
    let center = (a + b) * 0.5 + normal * 72.0;
    move_to(&mut p, 3, center - Vec2::splat(60.0));
    let rect = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).unwrap());
    assert!(
        laid.iter().any(|i| meet(&i.grown(gap), &rect)),
        "precondition: the path cuts the circle's box"
    );
    assert_eq!(
        inks(&drawn(&p, path)).len(),
        laid.len(),
        "the path is drawn past the circle"
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
