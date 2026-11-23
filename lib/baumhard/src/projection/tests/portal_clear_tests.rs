// SPDX-License-Identifier: MPL-2.0

//! Portal ends kept clear of the paths leaving their node (#394): an end
//! whose `border_t` is unset takes the nearest place along its node's
//! outline that the node's connectors and line links leave free, as far
//! out as the end reaches (`geometry::portal_walls`,
//! `geometry::held_clear`), and an end
//! whose `border_t` is set sits where it says. Read from what the
//! projector placed — glyph inks from the placed runs' metrics — and
//! every projector test ends on the oracle.

use glam::Vec2;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::resolve::{resolve_portal_end, BorderPoint};
use crate::flow::Rect;
use crate::projection::geometry::{held_apart, held_clear, marker_size, portal_along, portal_point, reach};
use crate::projection::state::{Placed, UnitBody};
use crate::projection::tests::maps::{ident, kid, kids, load_map, node, text_section};
use crate::projection::tests::support::Projected;

/// The canvas box of each inked glyph of `placed`: its pen the run's
/// baseline below its box's corner, its ink about the pen as shaped.
fn inks(placed: &Placed) -> Vec<Rect> {
    let baseline = placed.run.baseline();
    placed
        .placed
        .glyphs
        .iter()
        .filter_map(|g| {
            let ink = placed.run.glyphs[g.glyph as usize].ink?;
            let pen = g.pos + Vec2::new(0.0, baseline);
            Some(Rect::new(pen + ink.min, ink.max - ink.min))
        })
        .collect()
}

/// Whether any box of `a` overlaps any of `b`, edges that only meet
/// apart.
fn touch(a: &[Rect], b: &[Rect]) -> bool {
    a.iter().any(|a| {
        b.iter().any(|b| {
            a.origin.x < b.max().x
                && b.origin.x < a.max().x
                && a.origin.y < b.max().y
                && b.origin.y < a.max().y
        })
    })
}

/// The issue's map: node 1, "One", at the origin, with a child, node 2,
/// at (300, 0), and a portal to node 3 at (600, 0) whose end at node 1 is
/// `end` — so the connector to node 2 and the portal toward node 3 leave
/// one side of node 1.
fn issue_map(end: Value) -> Projected {
    Projected::new(load_map(json!({"nodes": [
        node(1, None, json!({"sections": [text_section("One")], "links": [
            {"to": 3, "kind": "portal", "ends": [end, {}]}
        ]})),
        node(2, Some(1), json!({"position": [300.0, 0.0], "sections": [text_section("Two")]})),
        node(3, None, json!({"position": [600.0, 0.0], "sections": [text_section("Three")]})),
    ]})))
}

/// Node 1's portal link.
fn portal_of(p: &Projected) -> NodeId {
    kid(&p.doc, ident(&p.doc, 1), DocKind::Link)
}

/// The inks of the end at node 1 of `portal`'s marker, and where it
/// centers.
fn marker(p: &Projected, portal: NodeId) -> (Vec<Rect>, Vec2) {
    let UnitBody::Portal(state) = &p.projector.units[&portal].body else {
        panic!("a portal");
    };
    let end = &state.end[0];
    (
        inks(end.marker.placed.as_ref().expect("its marker is placed")),
        end.center.expect("its end is placed"),
    )
}

/// The inks of node 2's connector: its body and its caps.
fn connector_inks(p: &Projected) -> Vec<Rect> {
    let connector = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let UnitBody::Line(line) = &p.projector.units[&connector].body else {
        panic!("a drawn connector");
    };
    line.flows
        .iter()
        .filter_map(|f| f.placed.as_ref())
        .flat_map(inks)
        .collect()
}

/// Where the end at node 1 of `portal`, `border_t` unset, faces and where
/// it is held: its places along node 1's outline.
fn faced_and_held(p: &Projected, portal: NodeId) -> (f32, f32) {
    let UnitBody::Portal(state) = &p.projector.units[&portal].body else {
        panic!("a portal");
    };
    (state.at[0].expect("its end faces node 3").along, state.held[0])
}

/// Where `portal_place` centers the end at node 1 of `portal`'s marker at
/// `along`: the outline's point there, moved by the end's offset and
/// the marker's reach along the normal.
fn center_at(p: &Projected, portal: NodeId, along: f32) -> Vec2 {
    let UnitBody::Portal(state) = &p.projector.units[&portal].body else {
        panic!("a portal");
    };
    let end = &state.end[0];
    let id = end.marker.id.expect("its marker element");
    let offset = resolve_portal_end(p.doc.tree(), id, &mut ()).offset;
    let outline = p.projector.nodes[&ident(&p.doc, 1)].outline;
    let (point, normal) = portal_point(&outline, along, offset);
    let size = marker_size(&end.marker.flow.as_ref().expect("shaped").shaped);
    point + normal * reach(normal, size)
}

/// **A portal end with `border_t` unset does not sit on a connector that
/// leaves the same side of its node** — the issue's reproduction: the
/// portal toward node 3 faces node 1's right side's midpoint, where the
/// connector to node 2 leaves; its marker's ink is held clear of the
/// connector's.
///
/// Precondition: where it faces, the marker's ink would touch the
/// connector's — the map reproduces the issue.
///
/// Control: `held_clear` returning `held_apart` (the walls ignored) is
/// red at the assertion.
#[test]
fn test_projection_a_portal_end_keeps_clear_of_a_connector_leaving_its_side() {
    let mut p = issue_map(json!({}));
    let portal = portal_of(&p);
    let (own, center) = marker(&p, portal);
    let connector = connector_inks(&p);
    assert!(
        !own.is_empty() && connector.len() >= 4,
        "precondition: the marker and the connector draw ink, {} and {}",
        own.len(),
        connector.len()
    );
    let (faced, held) = faced_and_held(&p, portal);
    let shift = center_at(&p, portal, faced) - center;
    let there: Vec<Rect> = own.iter().map(|r| Rect::new(r.origin + shift, r.size)).collect();
    assert!(
        touch(&there, &connector),
        "precondition: where it faces, {faced}, the marker's ink would touch the connector's"
    );
    assert!(
        !touch(&own, &connector),
        "the marker, held at {held} from {faced}, centered at {center}, touches the connector's ink"
    );
    p.check().unwrap();
}

/// **An end whose `border_t` is set sits exactly where it says**, a
/// connector there or not: at the place the issue's portal faces, on the
/// connector's start, it is drawn at the outline's point `border_t` of
/// the way round, as `portal_place` sets it, and no wall moves it.
///
/// Control: an end with `border_t` set read as one with it unset (held
/// clear like one) is red at the first assertion.
#[test]
fn test_projection_a_portal_end_at_its_border_t_is_drawn_there_over_a_connector() {
    let free = issue_map(json!({}));
    let (faced, _) = faced_and_held(&free, portal_of(&free));
    let outline = free.projector.nodes[&ident(&free.doc, 1)].outline;
    let (_, perimeter) = portal_along(&outline, BorderPoint::Auto, Vec2::ZERO);
    let border_t = faced / perimeter;
    let mut p = issue_map(json!({"border_t": border_t}));
    let portal = portal_of(&p);
    let (own, center) = marker(&p, portal);
    let want = center_at(&p, portal, border_t * perimeter);
    assert!(
        center.distance(want) < 1e-3,
        "the end at border_t {border_t} centers at {center}, not {want}"
    );
    assert!(
        touch(&own, &connector_inks(&p)),
        "precondition of the claim that it is not moved: it sits on the connector's ink"
    );
    p.check().unwrap();
}

/// **A drag's first write keeps a held end where it is drawn**: the
/// `(border_t, offset)` the drag's inverse (`Projector::portal_place_at`,
/// #395) reads off the issue's end, held clear of the connector, at its
/// drawn center, written, draws it there again — set, its `border_t` is
/// taken as given, on the arc a held end is placed along, so the grab
/// does not jump.
///
/// Control: the inverse not taking off the marker's reach is red.
#[test]
fn test_projection_a_held_end_given_the_border_t_its_drag_reads_stays_put() {
    use crate::document::mutation::ops::{DocMutation, PortalEndOp};
    use crate::document::tests::writer_support::{request, single};
    let mut p = issue_map(json!({}));
    let portal = portal_of(&p);
    let (faced, held) = faced_and_held(&p, portal);
    assert!(
        (faced - held).abs() > 1.0,
        "precondition: the end is held off where it faces, {held} from {faced}"
    );
    let (_, center) = marker(&p, portal);
    let UnitBody::Portal(state) = &p.projector.units[&portal].body else {
        panic!("a portal");
    };
    let end = state.end[0].marker.id.expect("its marker element");
    let (border_t, offset) = p
        .projector
        .portal_place_at(&p.doc, end, false, center)
        .expect("a drawn end");
    for (op, why) in [
        (PortalEndOp::SetBorderT(Some(border_t)), "set its border_t"),
        (PortalEndOp::SetOffset(Some(offset)), "set its offset"),
    ] {
        let mutation = single(DocKind::PortalEnd, DocMutation::PortalEnd(op));
        p.apply(request(mutation, vec![end], why)).unwrap();
    }
    let (_, after) = marker(&p, portal);
    assert!(
        after.distance(center) < 1e-2,
        "given border_t {border_t} and offset {offset}, the end moved from {center} to {after}"
    );
    p.check().unwrap();
}

/// **The end is held again when the paths leaving its node change, and
/// the update equals a fresh projection.** Node 2 moved below node 1
/// takes its connector off node 1's right side: the end, which nothing
/// about the portal itself moved, goes back to where it faces; moved
/// back, it is held clear again.
///
/// Preconditions, that each move moves the end: held off where it faces
/// at first, where it faces with node 2 below, held as at first with
/// node 2 back.
///
/// Control: phase C not holding a node's portal ends again after
/// fanning its sides is red at the first `check`.
#[test]
fn test_projection_a_portal_end_is_held_again_when_a_connector_leaves_its_side() {
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::writer_support::{request, single};
    let mut p = issue_map(json!({}));
    let portal = portal_of(&p);
    let (faced, held) = faced_and_held(&p, portal);
    assert!(
        (faced - held).abs() > 1.0,
        "precondition: the end is held off where it faces, {held} from {faced}"
    );
    let two = ident(&p.doc, 2);
    for (to, clear) in [(Vec2::new(0.0, 300.0), false), (Vec2::new(300.0, 0.0), true)] {
        let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(to)));
        p.apply(request(mutation, vec![two], "move node 2")).unwrap();
        p.check().unwrap();
        let (faced, now) = faced_and_held(&p, portal);
        if clear {
            assert_eq!(
                now, held,
                "precondition: moved back, the end is held where it was"
            );
        } else {
            assert_eq!(
                now, faced,
                "precondition: with no path on its side, the end sits where it faces"
            );
        }
    }
}

/// **The end is held again when a path leaving its node is fanned again
/// at the path's other end**, and the update equals a fresh projection:
/// node 4, a child of node 2, moved to node 2's left joins the side the
/// connector from node 1 arrives at, whose fan there turns that
/// connector's curve — near node 1 too, where the portal end keeps
/// clear of it. Nothing at node 1 itself was touched.
///
/// Precondition: the move holds the end at another place.
///
/// Control: phase C not holding the portal ends at a path's other end
/// when its fan changes is red at the `check`.
#[test]
fn test_projection_a_portal_end_is_held_again_when_a_path_is_fanned_at_its_far_end() {
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::writer_support::{request, single};
    let mut p = Projected::new(load_map(json!({"nodes": [
        node(1, None, json!({"sections": [text_section("One")], "links": [
            {"to": 3, "kind": "portal"}
        ]})),
        node(2, Some(1), json!({"position": [250.0, -100.0], "sections": [text_section("Two")]})),
        node(4, Some(2), json!({"position": [250.0, 100.0], "sections": [text_section("Four")]})),
        node(3, None, json!({"position": [600.0, -100.0], "sections": [text_section("Three")]})),
    ]})));
    let portal = portal_of(&p);
    let (_, before) = faced_and_held(&p, portal);
    let four = ident(&p.doc, 4);
    let mutation = single(
        DocKind::Node,
        DocMutation::Node(NodeOp::SetPosition(Vec2::new(100.0, -60.0))),
    );
    p.apply(request(mutation, vec![four], "move node 4 to node 2's left"))
        .unwrap();
    p.check().unwrap();
    let (_, after) = faced_and_held(&p, portal);
    assert_ne!(after, before, "precondition: the move holds the end elsewhere");
}

/// The distance between two places round an outline `perimeter` long,
/// the shorter way.
fn round(a: f32, b: f32, perimeter: f32) -> f32 {
    let d = (a - b).rem_euclid(perimeter);
    d.min(perimeter - d)
}

/// **With no walls, or walls that leave no room, `held_clear` is
/// `held_apart` bit for bit**, so a node no path leaves holds its portal
/// ends as before #394.
///
/// Control: the no-walls branch holding the ends on an outline one
/// longer is red.
#[test]
fn test_projection_held_clear_with_no_walls_is_held_apart() {
    let rows: [(&[(f32, f32)], f32); 4] = [
        (&[(0.0, 10.0), (10.0, 10.0), (11.0, 10.0), (12.0, 10.0)], 1000.0),
        (&[(2.0, 10.0), (995.0, 10.0)], 1000.0),
        (&[(0.0, 10.0), (100.0, 20.0), (104.0, 20.0)], 1000.0),
        (&[(3.0, 7.0), (40.0, 30.0), (41.5, 12.0), (90.0, 25.0)], 97.5),
    ];
    for (ends, perimeter) in rows {
        let apart = held_apart(ends, perimeter);
        let none = held_clear(ends, &[], perimeter);
        let whole = held_clear(ends, &[(10.0, perimeter + 1.0)], perimeter);
        for k in 0..ends.len() {
            assert_eq!(
                none[k].to_bits(),
                apart[k].to_bits(),
                "no walls: end {k} at {} not {}",
                none[k],
                apart[k]
            );
            assert_eq!(
                whole[k].to_bits(),
                apart[k].to_bits(),
                "a wall round the whole outline: end {k} at {} not {}",
                whole[k],
                apart[k]
            );
        }
    }
}

/// **An end facing into a wall takes the nearer of its edges' free
/// places, the later on a tie, as near as the wall allows** — half its
/// width past the edge — and ends that meet past an edge are held apart
/// there. Ends facing 10 into a wall 40 wide from its counterclockwise
/// edge, 20 (its middle) and 30 go to 475, 525 and 535.
///
/// Control: a tie going counterclockwise is red.
#[test]
fn test_projection_held_clear_takes_the_nearer_free_place_past_a_wall() {
    let ends = [(490.0, 10.0), (500.0, 10.0), (510.0, 10.0)];
    let held = held_clear(&ends, &[(500.0, 40.0)], 1000.0);
    for (k, want) in [475.0_f32, 525.0, 535.0].into_iter().enumerate() {
        assert!(
            (held[k] - want).abs() < 1e-3,
            "end {k}, facing {}, held at {} not {want}: {held:?}",
            ends[k].0,
            held[k]
        );
    }
}

/// Whether every one of `held` clears every wall by half its own width,
/// and its neighbors by half their widths summed, round an outline
/// `perimeter` long.
fn all_clear(ends: &[(f32, f32)], held: &[f32], walls: &[(f32, f32)], perimeter: f32) -> Result<(), String> {
    for (k, (&(_, width), &at)) in ends.iter().zip(held).enumerate() {
        for &(center, wall) in walls {
            let need = 0.5 * (width + wall);
            let apart = round(at, center, perimeter);
            if apart < need - 1e-3 {
                return Err(format!(
                    "end {k} at {at} is {apart} from the wall at {center}, needs {need}"
                ));
            }
        }
        for (j, (&(_, other), &there)) in ends.iter().zip(held).enumerate().skip(k + 1) {
            let need = 0.5 * (width + other);
            let apart = round(at, there, perimeter);
            if apart < need - 1e-3 {
                return Err(format!("ends {k} and {j} are {apart} apart, need {need}"));
            }
        }
    }
    Ok(())
}

/// **A stretch between two walls too short for the ends facing into it
/// spills one across a wall, and every end clears every wall** — three
/// ends 20 wide facing the middle of a stretch 40 long: two stay, packed
/// from wall to wall, the third is carried past the clockwise wall (the
/// two ways tie) to the stretch beyond it.
///
/// Control: no spill (the overflow kept, packed) is red.
#[test]
fn test_projection_held_clear_spills_an_end_past_a_wall_to_clear_them_all() {
    let walls = [(100.0, 20.0), (160.0, 20.0)];
    let ends = [(130.0, 20.0), (130.0, 20.0), (130.0, 20.0)];
    let held = held_clear(&ends, &walls, 1000.0);
    all_clear(&ends, &held, &walls, 1000.0).unwrap_or_else(|e| panic!("{e}: {held:?}"));
    for (k, want) in [120.0_f32, 140.0, 180.0].into_iter().enumerate() {
        assert!(
            (held[k] - want).abs() < 1e-3,
            "end {k} held at {} not {want}: {held:?}",
            held[k]
        );
    }
}

/// **Ends the free outline cannot hold are packed tight, centered on
/// their stretch, and over both its walls alike** — four ends 20 wide in
/// the one stretch 60 long a wall 40 wide leaves: 20 apart, centered on
/// the stretch's middle, each wall overlapped by 10.
///
/// Control: the packed ends set against the counterclockwise wall, not
/// centered, is red.
#[test]
fn test_projection_held_clear_packs_ends_the_outline_cannot_hold_centered() {
    let walls = [(50.0, 40.0)];
    let ends = [(100.0, 20.0), (100.0, 20.0), (100.0, 20.0), (100.0, 20.0)];
    let held = held_clear(&ends, &walls, 100.0);
    assert!(
        all_clear(&ends, &held, &walls, 100.0).is_err(),
        "precondition: they cannot all clear the wall"
    );
    // Round from the wall's clockwise edge, 70.
    let from: Vec<f32> = held.iter().map(|&h| (h - 70.0).rem_euclid(100.0)).collect();
    for (k, want) in [0.0_f32, 20.0, 40.0, 60.0].into_iter().enumerate() {
        assert!(
            (from[k] - want).abs() < 1e-3,
            "end {k} is {} past the wall, not {want}: {held:?}",
            from[k]
        );
    }
}

/// **Ends the free outline cannot hold share the overflow between the
/// stretches**: four ends 50 wide facing one of two stretches 80 long go
/// two to each, each pair packed 50 apart and centered on its stretch —
/// over each wall by 10 at most, not three in one stretch over its walls
/// by 35.
///
/// Control: an end moved only to a stretch with room for it whole is
/// red.
#[test]
fn test_projection_held_clear_shares_an_overflow_between_stretches() {
    let walls = [(0.0, 20.0), (100.0, 20.0)];
    let ends = [(50.0, 50.0); 4];
    let held = held_clear(&ends, &walls, 200.0);
    let mut sorted = held.clone();
    sorted.sort_by(f32::total_cmp);
    for (k, want) in [25.0_f32, 75.0, 125.0, 175.0].into_iter().enumerate() {
        assert!(
            (sorted[k] - want).abs() < 1e-3,
            "the ends, in order round the outline, are at {sorted:?}, not 25, 75, 125 and 175"
        );
    }
}

/// The issue's map grown into a hub: connectors to three nodes off its
/// right, the outer two curving away along its top and bottom, and one
/// to a node below; three portals facing its right side and its bottom.
fn hub_map() -> Projected {
    Projected::new(load_map(json!({"nodes": [
        node(1, None, json!({"sections": [text_section("The hub of this map")], "links": [
            {"to": 6, "kind": "portal"}, {"to": 7, "kind": "portal"}, {"to": 8, "kind": "portal"}
        ]})),
        node(2, Some(1), json!({"position": [300.0, -120.0], "sections": [text_section("North-east")]})),
        node(3, Some(1), json!({"position": [300.0, 0.0], "sections": [text_section("East")]})),
        node(4, Some(1), json!({"position": [300.0, 120.0], "sections": [text_section("South-east")]})),
        node(5, Some(1), json!({"position": [0.0, 200.0], "sections": [text_section("South")]})),
        node(6, None, json!({"position": [650.0, 10.0], "sections": [text_section("Far east")]})),
        node(7, None, json!({"position": [650.0, 60.0], "sections": [text_section("Far east, lower")]})),
        node(8, None, json!({"position": [10.0, 450.0], "sections": [text_section("Far south")]})),
    ]})))
}

/// **Portal ends keep clear of the whole of each path near their node,
/// not only its straight lead**: at the hub, connectors that curve away
/// along its sides past their leads, and three portal ends, none of whose
/// markers' ink touches any connector's.
///
/// Precondition: where they face, some marker's ink would touch a
/// connector's.
///
/// Control: the walls read from each path's first curve alone — its
/// straight lead — is red at the assertion.
#[test]
fn test_projection_portal_ends_keep_clear_of_paths_curving_along_their_node() {
    let mut p = hub_map();
    let hub = ident(&p.doc, 1);
    let portals = kids(&p.doc, hub, DocKind::Link);
    assert_eq!(portals.len(), 3, "precondition: three portals");
    let connectors: Vec<Rect> = (2..=5)
        .flat_map(|n| {
            let connector = kid(&p.doc, ident(&p.doc, n), DocKind::Connector);
            let UnitBody::Line(line) = &p.projector.units[&connector].body else {
                panic!("a drawn connector");
            };
            line.flows
                .iter()
                .filter_map(|f| f.placed.as_ref())
                .flat_map(inks)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut faced_touch = false;
    for &portal in &portals {
        let (own, center) = marker(&p, portal);
        let (faced, held) = faced_and_held(&p, portal);
        let shift = center_at(&p, portal, faced) - center;
        let there: Vec<Rect> = own.iter().map(|r| Rect::new(r.origin + shift, r.size)).collect();
        faced_touch |= touch(&there, &connectors);
        assert!(
            !touch(&own, &connectors),
            "the marker held at {held} from {faced}, centered at {center}, touches a connector's ink"
        );
    }
    assert!(
        faced_touch,
        "precondition: where they face, some marker's ink would touch a connector's"
    );
    p.check().unwrap();
}
