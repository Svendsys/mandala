// SPDX-License-Identifier: MPL-2.0

//! Connector geometry (#296): the side a path attaches at, the curve it
//! runs along, where its glyphs stop at a frame, the fan of the paths
//! that share a side, and portal ends held apart. Each property is read
//! from what the projector placed — glyph inks from the placed runs'
//! metrics, read here and not through the projector's own reading of
//! them — and every projector test ends on the oracle.

use glam::Vec2;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::values::Anchor;
use crate::flow::{Curve, Rect};
use crate::projection::geometry::{
    fixed_samples, held_apart, near_pairs, path_through, sample, side_point, side_toward, untangle, Lane,
    CLEARANCE, SPLAY_CAP, UNTANGLE_CLOSE, UNTANGLE_GRIDDED, UNTANGLE_GRIDS, UNTANGLE_LISTED,
    UNTANGLE_MEASURED, UNTANGLE_ROUNDS, UNTANGLE_SCRATCH,
};
use crate::projection::state::{Placed, UnitBody};
use crate::projection::tests::maps::{ident, kid, load_map, node, text_section};
use crate::projection::tests::support::Projected;
use crate::projection::Projector;

/// A ring of middle dots `size` of the node's shorter side, round a node
/// at least `w × h`.
fn dotted(w: f32, h: f32) -> Value {
    json!({
        "min_size": [w, h],
        "frame": {"inset": 4.0, "rings": [{"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]}]}
    })
}

/// `fields` with `more` merged in.
fn with(mut fields: Value, more: Value) -> Value {
    if let (Some(out), Some(more)) = (fields.as_object_mut(), more.as_object()) {
        for (k, v) in more {
            out.insert(k.clone(), v.clone());
        }
    }
    fields
}

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

/// The gap between two boxes, 0 where they meet.
fn gap(a: &Rect, b: &Rect) -> f32 {
    (b.origin - a.max())
        .max(a.origin - b.max())
        .max(Vec2::ZERO)
        .length()
}

/// The median of `values`.
fn median(mut values: Vec<f32>) -> f32 {
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

/// A drawn line's body placement and curves.
fn line(projector: &Projector, id: NodeId) -> (&Placed, &[Curve]) {
    let Some(UnitBody::Line(line)) = projector.units.get(&id).map(|u| &u.body) else {
        panic!("{id:?} is a drawn line");
    };
    (
        line.flows[0].placed.as_ref().expect("its body is placed"),
        &line.path,
    )
}

/// Every ink of every ring `node` draws.
fn ring_inks(projector: &Projector, node: NodeId) -> Vec<Rect> {
    projector.nodes[&node]
        .units
        .iter()
        .filter_map(|u| match &projector.units[u].body {
            UnitBody::Ring(ring) => ring.flow.placed.as_ref(),
            _ => None,
        })
        .flat_map(inks)
        .collect()
}

/// The box round `boxes`.
fn bounds(boxes: &[Rect]) -> Rect {
    boxes.iter().skip(1).fold(boxes[0], |a, b| a.union(b))
}

/// The angle between two directions, degrees.
fn degrees(a: Vec2, b: Vec2) -> f32 {
    a.angle_to(b).abs().to_degrees()
}

#[test]
fn test_projection_auto_side_is_the_one_the_ray_leaves_through() {
    // A box 400 wide and 40 tall about the origin. Toward (300, 60) the
    // right side's midpoint is the nearer, 117 off against the bottom's
    // 301, but the ray leaves through the bottom: 60 over a half height of
    // 20 is past 300 over a half width of 200.
    let wide = Rect::new(Vec2::new(-200.0, -20.0), Vec2::new(400.0, 40.0));
    assert_eq!(
        side_toward(wide, Anchor::Auto, Vec2::new(300.0, 60.0)),
        Anchor::Bottom
    );
    assert_eq!(
        side_toward(wide, Anchor::Auto, Vec2::new(300.0, 20.0)),
        Anchor::Right
    );
    assert_eq!(
        side_toward(wide, Anchor::Auto, Vec2::new(-300.0, -20.0)),
        Anchor::Left
    );
    assert_eq!(
        side_toward(wide, Anchor::Auto, Vec2::new(10.0, -21.0)),
        Anchor::Top
    );
    assert_eq!(
        side_toward(wide, Anchor::Left, Vec2::new(300.0, 60.0)),
        Anchor::Left,
        "a side named is kept"
    );
}

#[test]
fn test_projection_a_path_leaves_and_arrives_square_and_passes_its_points() {
    // From a right side at the origin to a left side 300 on and 60 down,
    // through a wave: up 30 a third of the way, down 30 two thirds.
    let (p0, p1) = (Vec2::ZERO, Vec2::new(300.0, 60.0));
    let via = [Vec2::new(0.0, -30.0), Vec2::new(0.0, 30.0)];
    let curves = path_through((p0, Vec2::X, 0.0), (p1, -Vec2::X, 0.0), &via, 1.0);
    assert_eq!(curves.len(), 3, "a cubic from each point to the next");
    assert!(
        degrees(curves[0].derivative(0.0), Vec2::X) < 0.01,
        "it leaves square to its side"
    );
    assert!(
        degrees(curves[2].derivative(1.0), Vec2::X) < 0.01,
        "it arrives square to its side"
    );
    for (k, v) in via.iter().enumerate() {
        let at = p0 + (p1 - p0) * ((k + 1) as f32 / 3.0) + *v;
        assert!(curves[k].point(1.0).distance(at) < 1e-3, "through point {k}");
        assert!(
            curves[k + 1].point(0.0).distance(at) < 1e-3,
            "point {k} starts the next curve"
        );
        let (into, out) = (curves[k].derivative(1.0), curves[k + 1].derivative(0.0));
        assert!(
            into.distance(out) < 1e-3 * into.length(),
            "no kink at point {k}: {into} into, {out} out"
        );
    }
    // With none, an S between two facing sides as deep as they are apart:
    // its handles reach 0.5523 of the 300 the far end lies on.
    let s = path_through((p0, Vec2::X, 0.0), (p1, -Vec2::X, 0.0), &[], 1.0);
    let Curve::Cubic(_, c1, c2, _) = s[0] else { panic!("one cubic") };
    assert!(
        (c1 - Vec2::new(165.69, 0.0)).length() < 0.01,
        "the start's handle: {c1}"
    );
    assert!(
        (c2 - Vec2::new(300.0 - 165.69, 60.0)).length() < 0.01,
        "the end's handle: {c2}"
    );
    // Each end runs straight for its own lead: 20 at the start, which a
    // fan holds out, and none at the end, which no other path shares.
    let led = path_through((p0, Vec2::X, 20.0), (p1, -Vec2::X, 0.0), &[], 1.0);
    assert_eq!(
        led.first(),
        Some(&Curve::Line(p0, Vec2::new(20.0, 0.0))),
        "the start's lead: {led:?}"
    );
    assert!(
        matches!(led.last(), Some(Curve::Cubic(.., end)) if *end == p1),
        "no lead at the end: {led:?}"
    );
}

/// Node 1 and its child node 2 beside it, each in a ring of dots, the
/// child's connector a line of dots.
fn two_frames() -> Projected {
    Projected::new(load_map(json!({"nodes": [
        node(1, None, with(dotted(160.0, 60.0), json!({"sections": [text_section("one")]}))),
        node(2, Some(1), with(dotted(160.0, 60.0), json!({
            "position": [420.0, 140.0],
            "sections": [text_section("two")],
            "connector": {"size": 14.0}
        }))),
    ]})))
}

#[test]
fn test_projection_a_connector_stops_at_its_frames_outermost_ink() {
    let mut p = two_frames();
    let (one, two) = (ident(&p.doc, 1), ident(&p.doc, 2));
    let connector = kid(&p.doc, two, DocKind::Connector);
    let (body, curves) = line(&p.projector, connector);
    let own = inks(body);
    assert!(
        own.len() >= 8,
        "precondition: a connector of some length, {} glyphs",
        own.len()
    );
    let between = median(own.windows(2).map(|w| gap(&w[0], &w[1])).collect());
    let pitch = median(
        own.windows(2)
            .map(|w| w[0].center().distance(w[1].center()))
            .collect(),
    );
    let frames = [ring_inks(&p.projector, one), ring_inks(&p.projector, two)];
    assert!(
        !frames[0].is_empty() && !frames[1].is_empty(),
        "precondition: both nodes draw their rings"
    );
    let reaches = [bounds(&frames[0]), bounds(&frames[1])];
    for (k, label, end_ink, at) in [(0, "start", own[0], 0.0), (1, "end", own[own.len() - 1], 1.0)] {
        let frame = &frames[k];
        // The ink gap to the frame, against the connector's own (rule 5).
        let near = frame
            .iter()
            .map(|f| gap(&end_ink, f))
            .fold(f32::INFINITY, f32::min);
        let ratio = near / between;
        assert!(
            (0.8..=1.2).contains(&ratio),
            "the {label} glyph is {near} from its frame's ink, ×{ratio} its own gap {between}"
        );
        // At the midpoint of the frame's side facing the other node, and
        // square to it (rule 1).
        let side = side_toward(reaches[k], Anchor::Auto, reaches[1 - k].center());
        let (mid, normal) = side_point(reaches[k], side);
        let across = Vec2::new(-normal.y, normal.x);
        let start = if at == 0.0 {
            curves[0].point(0.0)
        } else {
            curves[curves.len() - 1].point(1.0)
        };
        assert!(
            start.distance(mid) < 1e-2,
            "the path's {label} is its frame's side midpoint {mid}, not {start}"
        );
        let off = (end_ink.center() - mid).dot(across).abs();
        assert!(
            off <= 0.5 * pitch,
            "the {label} glyph is {off} off its side's midpoint, half a step is {}",
            0.5 * pitch
        );
        let curve = if at == 0.0 {
            curves[0]
        } else {
            curves[curves.len() - 1]
        };
        let travel = curve.derivative(at);
        let square = if at == 0.0 { normal } else { -normal };
        assert!(
            degrees(travel, square) <= 10.0,
            "the {label} runs {}° off its side's normal",
            degrees(travel, square)
        );
        // Nothing of the connector on the frame.
        for ink in &own {
            for f in frame {
                assert!(
                    gap(ink, f) > 0.0,
                    "a connector glyph {ink:?} touches its frame's ink {f:?}"
                );
            }
        }
    }
    p.check().unwrap();
}

/// Two nodes framed by a ring of `·` and U+10FFFD in Galatia SIL (#333):
/// U+10FFFD, the last private-use code point, which no compiled face
/// maps, falls to Galatia SIL's `.notdef`, whose outline is empty — where
/// no host face maps it either, which the test's precondition reads from
/// the rasterizer. Node 2 is node 1's child, below it, joined by a
/// connector from node 1's bottom side to node 2's top: the sides a
/// line-high box reaches furthest past a dot.
fn undrawn_frames() -> Projected {
    let ring = |w: f32, h: f32| {
        json!({
            "min_size": [w, h],
            "frame": {"inset": 4.0, "rings": [
                {"fit": "repeat", "size": 0.25, "font": "Galatia SIL", "spans": [{"text": "·\u{10FFFD}"}]}
            ]}
        })
    };
    Projected::new(load_map(json!({"nodes": [
        node(1, None, with(ring(160.0, 60.0), json!({"sections": [text_section("one")]}))),
        node(2, Some(1), with(ring(160.0, 60.0), json!({
            "position": [0.0, 260.0],
            "sections": [text_section("two")],
            "connector": {"size": 14.0}
        }))),
    ]})))
}

/// Each drawn and each undrawn ring glyph of `node`, on the canvas: a
/// drawn glyph's box the rasterizer's image of it, as the renderer asks
/// for it, at its pen — read here from the rasterizer, not through the
/// engine's own reading of it; an undrawn one's, what a glyph with no
/// outline was inked as before #333, its advance by its run's line height.
fn rasterized_ring(p: &mut Projected, node: NodeId) -> (Vec<Rect>, Vec<Rect>) {
    let fs = p.fonts.fonts();
    let (mut drawn, mut undrawn) = (Vec::new(), Vec::new());
    for u in &p.projector.nodes[&node].units {
        let UnitBody::Ring(ring) = &p.projector.units[u].body else {
            continue;
        };
        let Some(placed) = ring.flow.placed.as_ref() else {
            continue;
        };
        let run = &placed.run;
        let laid: Vec<_> = run.layout_glyphs().cloned().collect();
        for g in &placed.placed.glyphs {
            let (metrics, laid) = (&run.glyphs[g.glyph as usize], &laid[g.glyph as usize]);
            if metrics.blank {
                continue;
            }
            let pen = g.pos + Vec2::new(0.0, run.baseline());
            // At the glyph's own size: its image's box is compared with
            // the ink the projector keeps of it, laid at that size.
            match crate::flow::tests::support::raster_box(fs, laid, laid.font_size) {
                Some((min, max)) => drawn.push(Rect::new(pen + min, max - min)),
                _ => undrawn.push(Rect::new(g.pos, Vec2::new(metrics.advance, run.line_height))),
            }
        }
    }
    (drawn, undrawn)
}

/// The box round `node`'s drawn rings' inks, as the projector keeps it:
/// what its paths attach to (#296).
fn ring_reach(projector: &Projector, node: NodeId) -> Option<Rect> {
    projector.nodes[&node]
        .units
        .iter()
        .filter_map(|u| match &projector.units[u].body {
            UnitBody::Ring(ring) => ring.ink,
            _ => None,
        })
        .reduce(|a, b| a.union(&b))
}

/// A ring holding a glyph the rasterizer draws nothing for reaches
/// exactly as far as its drawn inks (#333): the box round each node's
/// ring inks, what its paths attach to, is the box round the images the
/// rasterizer draws of its glyphs, each edge within the pixel the image
/// rounds its ink out to; and the connector leaves and reaches the
/// midpoints of those boxes' facing sides.
///
/// Preconditions: each ring holds glyphs the rasterizer draws nothing
/// for, and their boxes — what such a glyph was inked as before #333 —
/// reach past the drawn ones by more than a pixel, so the reach they
/// would make is not the drawn inks'. Control: a glyph with no outline
/// inked as its box whatever it draws, the rule before #333, red: each
/// node's reach 14.4 past its drawn inks ("node 1's reach Rect { .. } is
/// its drawn inks' Rect { .. }, within a pixel: 14.375 off"); with that
/// assertion stepped past, the connector's start 14.2 below its drawn
/// inks' bottom midpoint.
#[test]
fn test_projection_a_ring_reaches_as_far_as_its_drawn_inks() {
    let mut p = undrawn_frames();
    let (one, two) = (ident(&p.doc, 1), ident(&p.doc, 2));
    let rasters = [rasterized_ring(&mut p, one), rasterized_ring(&mut p, two)];
    p.release();
    for (k, (drawn, undrawn)) in rasters.iter().enumerate() {
        let node = k + 1;
        assert!(
            !drawn.is_empty() && !undrawn.is_empty(),
            "precondition: node {node}'s ring draws some glyphs and not others ({} and {})",
            drawn.len(),
            undrawn.len()
        );
        let inked = bounds(drawn);
        let boxed = undrawn.iter().fold(inked, |a, b| a.union(b));
        let past = (inked.origin - boxed.origin)
            .max(boxed.max() - inked.max())
            .max_element();
        assert!(
            past > 2.0,
            "precondition: node {node}'s undrawn glyphs' boxes reach {past} past its drawn inks"
        );
        let reach = ring_reach(&p.projector, ident(&p.doc, node as u32))
            .unwrap_or_else(|| panic!("node {node} has a reach"));
        let off = (reach.origin - inked.origin)
            .abs()
            .max((reach.max() - inked.max()).abs())
            .max_element();
        assert!(
            off <= 1.0,
            "node {node}'s reach {reach:?} is its drawn inks' {inked:?}, within a pixel: {off} off"
        );
    }
    // The connector leaves and reaches the midpoints of its drawn inks'
    // facing sides (#296, rule 1).
    let connector = kid(&p.doc, two, DocKind::Connector);
    let (_, curves) = line(&p.projector, connector);
    let drawn = [bounds(&rasters[0].0), bounds(&rasters[1].0)];
    let ends = [curves[0].point(0.0), curves[curves.len() - 1].point(1.0)];
    for (k, label) in [(0, "start"), (1, "end")] {
        let side = side_toward(drawn[k], Anchor::Auto, drawn[1 - k].center());
        let (mid, _) = side_point(drawn[k], side);
        assert!(
            ends[k].distance(mid) <= 1.0,
            "the connector's {label} {} is its drawn inks' {side:?} midpoint {mid}, within a pixel",
            ends[k]
        );
    }
    p.check().unwrap();
}

/// A ring respaced moves its inks along its sides and keeps the box round
/// them: the connector whose end gap is measured to the nearest of them is
/// laid again all the same (the review of #326). Node 1 above node 2, so
/// the connector leaves 1's bottom side, along which the respacing slides
/// its dots.
#[test]
fn test_projection_a_respaced_ring_lays_its_connector_again() {
    use crate::document::mutation::ops::{DocMutation, LayoutOp, RingOp};
    use crate::document::tests::writer_support::{request, single};
    let mut p = Projected::new(load_map(json!({"nodes": [
        node(1, None, with(dotted(160.0, 60.0), json!({"sections": [text_section("one")]}))),
        node(2, Some(1), with(dotted(160.0, 60.0), json!({
            "position": [0.0, 240.0],
            "sections": [text_section("two")],
            "connector": {"size": 14.0}
        }))),
    ]})));
    let one = ident(&p.doc, 1);
    let ring = kid(&p.doc, kid(&p.doc, one, DocKind::Frame), DocKind::Ring);
    let box_of = |p: &Projected| match &p.projector.units[&ring].body {
        UnitBody::Ring(state) => (state.ink, state.inks.clone()),
        _ => panic!("a ring"),
    };
    let before = box_of(&p);
    let edit = single(
        DocKind::Ring,
        DocMutation::Ring(RingOp::Layout(LayoutOp::SetSpacing(Some(0.7)))),
    );
    p.apply(request(edit, vec![ring], "respace the ring")).unwrap();
    let after = box_of(&p);
    assert_eq!(
        before.0, after.0,
        "precondition: the box round the ring's inks stays"
    );
    assert_ne!(before.1, after.1, "precondition: the respacing moves the inks");
    p.check().unwrap();
}

/// A connector dropped with its node reads nothing after: its ends read
/// its nodes' reach, and a later change of the parent's ring — which
/// moves the reach — must not reach the connector that is gone. Before
/// its ends' reads were dropped with it, that change marked its
/// geometry after the phase it runs in, and the debug build panicked.
#[test]
fn test_projection_a_dropped_connector_reads_no_reach() {
    use crate::document::mutation::compose;
    use crate::document::mutation::ops::{DocMutation, SpanOp};
    use crate::document::tests::writer_support::{request, single};
    let mut p = two_frames();
    let (one, two) = (ident(&p.doc, 1), ident(&p.doc, 2));
    let steps = compose::delete(&p.doc, &[two]).unwrap();
    p.apply(crate::document::writer::EditRequest::new(
        steps,
        "delete the child",
    ))
    .unwrap();
    p.check().unwrap();
    let ring = kid(&p.doc, kid(&p.doc, one, DocKind::Frame), DocKind::Ring);
    let span = kid(&p.doc, ring, DocKind::Span);
    let edit = single(DocKind::Span, DocMutation::Span(SpanOp::SetText("✦".into())));
    p.apply(request(edit, vec![span], "another ring")).unwrap();
    p.check().unwrap();
}

#[test]
fn test_projection_a_straight_connector_is_one_segment_between_midpoints() {
    // Node 2 beside and below node 1, its connector straight: one segment
    // from node 1's side midpoint to node 2's, its glyphs on it.
    let mut p = Projected::new(load_map(json!({"nodes": [
        node(1, None, dotted(160.0, 60.0)),
        node(2, Some(1), with(dotted(160.0, 60.0), json!({
            "position": [420.0, 140.0],
            "connector": {"straight": true, "size": 14.0}
        }))),
    ]})));
    let connector = kid(&p.doc, ident(&p.doc, 2), DocKind::Connector);
    let (body, curves) = line(&p.projector, connector);
    let [Curve::Line(from, to)] = curves else {
        panic!("one straight segment: {curves:?}");
    };
    let (a, b) = (from.to_owned(), to.to_owned());
    let glyphs = inks(body);
    assert!(glyphs.len() > 3, "precondition: glyphs laid: {}", glyphs.len());
    let off = |q: Vec2| ((q - a).perp_dot(b - a) / (b - a).length()).abs();
    for g in &glyphs {
        assert!(
            off(g.center()) < 1e-2,
            "a glyph {} off the segment",
            off(g.center())
        );
    }
    p.check().unwrap();
}

/// Node 1 with children below it at `xs`, each a dotted connector.
fn fan_of(xs: &[f32]) -> Projected {
    fan_of_glyph(xs, 260.0, "")
}

/// [`fan_of`], the children's tops at `y` and each connector of
/// `content` where it is not empty.
fn fan_of_glyph(xs: &[f32], y: f32, content: &str) -> Projected {
    let mut nodes = vec![node(
        1,
        None,
        with(dotted(200.0, 60.0), json!({"position": [-100.0, -30.0]})),
    )];
    for (k, &x) in xs.iter().enumerate() {
        nodes.push(node(
            10 + k as u32,
            Some(1),
            with(
                dotted(120.0, 50.0),
                json!({"position": [x - 60.0, y], "connector": if content.is_empty() {
                    json!({"size": 14.0})
                } else {
                    json!({"size": 14.0, "content": content})
                }}),
            ),
        ));
    }
    Projected::new(load_map(json!({ "nodes": nodes })))
}

/// The connectors of node 1's children at `xs`, left to right: their
/// curves and first inks.
fn branches(p: &Projected, count: usize) -> Vec<(Vec<Curve>, Vec<Rect>)> {
    (0..count)
        .map(|k| {
            let connector = kid(&p.doc, ident(&p.doc, 10 + k as u32), DocKind::Connector);
            let (body, curves) = line(&p.projector, connector);
            (curves.to_vec(), inks(body))
        })
        .collect()
}

#[test]
fn test_projection_paths_that_share_a_side_leave_one_point_fanned() {
    // Two children below, one to each side: both leave the parent's
    // bottom midpoint, turned apart evenly, each toward its own child, by
    // as little as clears their first glyphs.
    let mut p = fan_of(&[-260.0, 260.0]);
    let pair = branches(&p, 2);
    let start = pair[0].0[0].point(0.0);
    assert!(
        start.distance(pair[1].0[0].point(0.0)) < 1e-3,
        "one point: {start} and {}",
        pair[1].0[0].point(0.0)
    );
    let turns: Vec<f32> = pair
        .iter()
        .map(|b| Vec2::Y.angle_to(b.0[0].derivative(0.0)).to_degrees())
        .collect();
    assert!(
        turns[0] > 1.0 && turns[1] < -1.0,
        "each turned toward its own child: {turns:?}"
    );
    assert!(
        (turns[0] + turns[1]).abs() < 0.01,
        "turned apart evenly: {turns:?}"
    );
    assert!(
        turns[0] <= SPLAY_CAP.to_degrees() + 0.01,
        "within the cap: {turns:?}"
    );
    let own = median(pair[0].1.windows(2).map(|w| gap(&w[0], &w[1])).collect());
    let first = gap(&pair[0].1[0], &pair[1].1[0]);
    assert!(
        first >= 0.8 * own,
        "their first glyphs {first} apart, their own gap {own}"
    );
    p.check().unwrap();
    // One child alone leaves along the normal.
    let mut lone = fan_of(&[260.0]);
    let one = branches(&lone, 1);
    assert!(
        degrees(one[0].0[0].derivative(0.0), Vec2::Y) < 0.01,
        "a lone path is not turned"
    );
    lone.check().unwrap();
    // Seven children turn no further than the cap, the outermost to it.
    let mut wide = fan_of(&[-480.0, -320.0, -160.0, 0.0, 160.0, 320.0, 480.0]);
    let seven = branches(&wide, 7);
    let most = seven
        .iter()
        .map(|b| degrees(Vec2::Y, b.0[0].derivative(0.0)))
        .fold(0.0_f32, f32::max);
    assert!(
        (most - SPLAY_CAP.to_degrees()).abs() < 0.01,
        "the outermost at the cap: {most}°"
    );
    // At the cap the turn alone no longer clears their first glyphs: each
    // path is held straight on past the point until they clear. Along the
    // whole of each path, no glyph comes nearer another's than their own
    // gap allows.
    clear_along(&seven, "seven children in a row");
    wide.check().unwrap();
}

#[test]
fn test_projection_a_held_fan_clears_its_neighbors_across_the_slope() {
    // Thirteen children in a row, as testament's Joktan's: the fan at its
    // cap, each two neighbors 6.7° apart, so each path is held straight on
    // past the point. Turned that far from the normal, two neighbors' first
    // glyphs stand side by side across a slope, where an upright box
    // reaches as far as its diagonal: they clear each other only held out
    // as far as the turn between them is reckoned for.
    let xs: Vec<f32> = (-6..=6).map(|k| k as f32 * 130.0).collect();
    let mut row = fan_of_glyph(&xs, 900.0, "■");
    let thirteen = branches(&row, 13);
    let most = thirteen
        .iter()
        .map(|b| degrees(Vec2::Y, b.0[0].derivative(0.0)))
        .fold(0.0_f32, f32::max);
    assert!(
        (most - SPLAY_CAP.to_degrees()).abs() < 0.01,
        "precondition: the outermost at the cap: {most}°"
    );
    let connector = kid(&row.doc, ident(&row.doc, 10), DocKind::Connector);
    let Some(UnitBody::Line(first)) = row.projector.units.get(&connector).map(|u| &u.body) else {
        panic!("the first branch is a drawn line");
    };
    let (hold, lead) = (first.fans[0].hold, first.attach[0].map_or(0.0, |a| a.lead));
    assert!(hold > 0.0, "precondition: a fan at the cap is held out: {hold}");
    let point = thirteen[0].0[0].point(0.0);
    let near = |b: &(Vec<Curve>, Vec<Rect>)| -> Vec<Rect> {
        b.1.iter()
            .filter(|r| r.center().distance(point) <= hold + lead)
            .copied()
            .collect()
    };
    let own = median(
        thirteen
            .iter()
            .flat_map(|b| b.1.windows(2).map(|w| gap(&w[0], &w[1])))
            .collect(),
    );
    for k in 0..12 {
        let (a, b) = (near(&thirteen[k]), near(&thirteen[k + 1]));
        let nearest = a
            .iter()
            .flat_map(|x| b.iter().map(move |y| gap(x, y)))
            .fold(f32::INFINITY, f32::min);
        assert!(
            !a.is_empty() && !b.is_empty() && nearest >= 0.8 * own,
            "branches {k} and {} have glyphs {nearest} apart near the point, their own gap {own}",
            k + 1
        );
    }
    clear_along(&thirteen, "thirteen children in a row");
    row.check().unwrap();
}

/// Node 1 with seven children 230 below it at ±480, ±320, ±160 and 0,
/// those in `straight` with straight connectors: a capped fan with paths
/// that do not turn.
fn seven_with_straight(straight: &[usize]) -> Projected {
    let xs = [-480.0_f32, -320.0, -160.0, 0.0, 160.0, 320.0, 480.0];
    let mut nodes = vec![node(
        1,
        None,
        with(dotted(200.0, 60.0), json!({"position": [-100.0, -30.0]})),
    )];
    for (k, &x) in xs.iter().enumerate() {
        let connector = if straight.contains(&k) {
            json!({"size": 14.0, "straight": true})
        } else {
            json!({"size": 14.0})
        };
        nodes.push(node(
            10 + k as u32,
            Some(1),
            with(
                dotted(120.0, 50.0),
                json!({"position": [x - 60.0, 260.0], "connector": connector}),
            ),
        ));
    }
    Projected::new(load_map(json!({ "nodes": nodes })))
}

#[test]
fn test_projection_a_straight_path_in_a_fan_is_fanned_round() {
    // Seven children at the cap, each in turn the one whose connector is
    // straight: its segment leaves in the direction it points, which no
    // fan turns, so the others are ordered and turned round that
    // direction and held out as any capped fan's are. No glyph of one
    // comes nearer another's than their own gap allows, along the whole
    // of every path. The failing inputs are a straight path given a slot
    // of the even fan it does not take, its neighbors turned for that
    // slot running through its glyphs, and a hold reckoned for the fan's
    // step alone, which two straight neighbors nearer than it outrun.
    // Then two neighbors straight, nearer each other than the fan's
    // step: held out as far as those two need.
    let singles = (0..7).map(|k| vec![k]);
    for straight in singles.chain([vec![0, 1], vec![5, 6], vec![2, 3]]) {
        let mut p = seven_with_straight(&straight);
        let seven = branches(&p, 7);
        for &k in &straight {
            let [Curve::Line(..)] = seven[k].0[..] else {
                panic!("precondition: child {k}'s connector is one segment");
            };
        }
        clear_along(&seven, &format!("children {straight:?} straight"));
        p.check().unwrap();
    }
}

/// Node 1 with a child at each of `children`: its center, and whether its
/// connector is straight.
fn children_at(children: &[(Vec2, bool)]) -> Projected {
    let mut nodes = vec![node(
        1,
        None,
        with(dotted(200.0, 60.0), json!({"position": [-100.0, -30.0]})),
    )];
    for (k, &(center, straight)) in children.iter().enumerate() {
        nodes.push(node(
            10 + k as u32,
            Some(1),
            with(
                dotted(120.0, 50.0),
                json!({"position": [center.x - 60.0, center.y - 25.0],
                       "connector": {"size": 14.0, "straight": straight}}),
            ),
        ));
    }
    Projected::new(load_map(json!({ "nodes": nodes })))
}

/// Child `k`'s connector's hold at node 1, its inks, and its chord.
fn held_branch(p: &Projected, k: u32) -> (f32, Vec<Rect>, f32) {
    let connector = kid(&p.doc, ident(&p.doc, 10 + k), DocKind::Connector);
    let Some(UnitBody::Line(state)) = p.projector.units.get(&connector).map(|u| &u.body) else {
        panic!("child {k}'s connector is a drawn line");
    };
    let (body, curves) = line(&p.projector, connector);
    let chord = curves[0].point(0.0).distance(curves[curves.len() - 1].point(1.0));
    (state.fans[0].hold, inks(body), chord)
}

#[test]
fn test_projection_two_straight_paths_near_each_other_hold_only_themselves() {
    // Child 10's curved connector leaves node 1's bottom far to the left;
    // children 11 and 12 have straight connectors, 12's moved toward 11's
    // direction until the two leave as one. Only the two straight paths
    // are held out the further for it, each no further than a quarter of
    // its chord, as far as it can carry, and the more continuously the
    // nearer they come: child 10 keeps its hold and its glyphs at every
    // step. Then a curved child 13 sorted between the two straight ones,
    // whose room closes as they meet: it is held out as its own room
    // needs, never past a quarter of its chord, and keeps its glyphs, as
    // does child 10. The failing inputs are one hold for
    // the whole side, the nearest pair's (child 10 drawn with no glyph from
    // x = 130), and a pair's hold not capped (none from x = 125); and the
    // fan's step narrowed for the whole side to fit the path between.
    let alone = held_branch(&children_at(&[(Vec2::new(-400.0, 300.0), false)]), 0);
    let mut first: Option<f32> = None;
    let mut last_pair = 0.0_f32;
    for x in [160.0, 140.0, 130.0, 125.0, 123.0, 122.0, 121.5, 121.0] {
        let mut p = children_at(&[
            (Vec2::new(-400.0, 300.0), false),
            (Vec2::new(40.0, 300.0), true),
            (Vec2::new(x, 800.0), true),
        ]);
        let (hold, glyphs, _) = held_branch(&p, 0);
        assert!(
            glyphs.len() * 2 >= alone.1.len(),
            "x = {x}: child 10 lays {} glyphs, against {} alone",
            glyphs.len(),
            alone.1.len()
        );
        let first = *first.get_or_insert(hold);
        assert!(
            hold <= 1.1 * first,
            "x = {x}: child 10's hold {hold}, against {first} with the pair apart"
        );
        for k in [1, 2] {
            let (pair_hold, pair_glyphs, chord) = held_branch(&p, k);
            assert!(
                pair_hold <= 0.25 * chord + 1e-3 && !pair_glyphs.is_empty(),
                "x = {x}: child {} held {pair_hold} out of a chord {chord}, {} glyphs",
                10 + k,
                pair_glyphs.len()
            );
            if k == 2 {
                assert!(
                    pair_hold + 1e-3 >= last_pair.min(0.25 * chord),
                    "x = {x}: the hold falls as the two meet, short of its cap: {pair_hold} after {last_pair}"
                );
                last_pair = pair_hold;
            }
        }
        p.check().unwrap();
    }
    // A curved path between the two straight ones: its child far out
    // along the direction halfway between theirs, so that it sorts between.
    let between = |x: f32| {
        let (a, b) = (Vec2::new(40.0, 245.0), Vec2::new(x, 745.0));
        let halfway = (a.normalize() + b.normalize()).normalize();
        Vec2::new(0.0, 30.0) + halfway * 1500.0 + Vec2::new(0.0, 25.0)
    };
    for x in [200.0, 160.0, 140.0, 130.0, 125.0, 122.0] {
        let mut p = children_at(&[
            (Vec2::new(-400.0, 300.0), false),
            (Vec2::new(40.0, 300.0), true),
            (Vec2::new(x, 800.0), true),
            (between(x), false),
        ]);
        let (hold, glyphs, chord) = held_branch(&p, 0);
        assert!(
            glyphs.len() * 2 >= alone.1.len() && hold <= 0.1 * chord,
            "x = {x}, a path between: child 10 held {hold}, {} glyphs",
            glyphs.len()
        );
        let (between_hold, between_glyphs, chord) = held_branch(&p, 3);
        assert!(
            between_hold <= 0.25 * chord + 1e-3 && !between_glyphs.is_empty(),
            "x = {x}: child 13 held {between_hold} out of a chord {chord}, {} glyphs",
            between_glyphs.len()
        );
        p.check().unwrap();
    }
}

/// The least distance between a glyph of one of `branches` and a glyph
/// of another, against the median gap between a branch's own glyphs: no
/// two branches of one junction nearer than 0.8 of it, anywhere along
/// them (#296, rule 4, as ruled on #326).
fn clear_along(branches: &[(Vec<Curve>, Vec<Rect>)], what: &str) {
    let own = median(
        branches
            .iter()
            .flat_map(|b| b.1.windows(2).map(|w| gap(&w[0], &w[1])))
            .collect(),
    );
    for (i, a) in branches.iter().enumerate() {
        for (j, b) in branches.iter().enumerate().skip(i + 1) {
            let (nearest, at) =
                a.1.iter()
                    .flat_map(|x| b.1.iter().map(move |y| (gap(x, y), x.center())))
                    .fold((f32::INFINITY, Vec2::ZERO), |m, g| if g.0 < m.0 { g } else { m });
            assert!(
                nearest >= 0.8 * own,
                "{what}: branches {i} and {j} have glyphs {nearest} apart at {at}, their own gap {own}"
            );
        }
    }
}

/// Node 1, 240 by 60, and three children 240 by 60 in a row far to its
/// right and 200 below it, one behind another along the way to them:
/// `stress_long_edges`' rows, where the paths to the further children
/// pass the nearer ones' ends.
fn far_row() -> Projected {
    let mut nodes = vec![node(1, None, dotted(240.0, 60.0))];
    for (k, x) in [5440.0, 5760.0, 6080.0].into_iter().enumerate() {
        nodes.push(node(
            10 + k as u32,
            Some(1),
            with(
                dotted(240.0, 60.0),
                json!({"position": [x, 200.0], "connector": {"size": 14.0}}),
            ),
        ));
    }
    Projected::new(load_map(json!({ "nodes": nodes })))
}

#[test]
fn test_projection_the_paths_of_a_junction_keep_clear_along_their_length() {
    // Three paths leave node 1's right side for three children in a row
    // along their way. Leaving and arriving square, their long curves
    // would run on top of one another into the nearer children's ends;
    // each turns toward its child sooner, and keeps clear of the others.
    let mut p = far_row();
    let three = branches(&p, 3);
    clear_along(&three, "a row along the way");
    p.check().unwrap();
}

#[test]
fn test_projection_a_moved_branch_turns_its_neighbors() {
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::writer_support::{request, single};
    // A child moved from the right of its sibling to its left swaps their
    // turns: the sibling, which nothing touched, is laid again.
    let mut p = fan_of(&[-260.0, 260.0]);
    let before = branches(&p, 2)[0].0[0].derivative(0.0);
    let moved = ident(&p.doc, 11);
    let to = Vec2::new(-600.0, 260.0);
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(to)));
    p.apply(request(mutation, vec![moved], "move a child across its sibling"))
        .unwrap();
    let after = branches(&p, 2)[0].0[0].derivative(0.0);
    assert!(
        degrees(before, after) > 1.0,
        "the untouched sibling turned: {before} then {after}"
    );
    p.check().unwrap();
}

/// The distance between two places round an outline `perimeter` long,
/// the shorter way.
fn round(a: f32, b: f32, perimeter: f32) -> f32 {
    let d = (a - b).rem_euclid(perimeter);
    d.min(perimeter - d)
}

#[test]
fn test_projection_held_apart_merges_a_run_pushed_into_the_one_before() {
    // The review's probe: ends 1..=3 overlap and are pushed apart; centered
    // on where they faced alone, they would land on end 0. Merged with it,
    // all four are 10 apart and centered on where the four faced.
    let ends = [(0.0, 10.0), (10.0, 10.0), (11.0, 10.0), (12.0, 10.0)];
    let held = held_apart(&ends, 1000.0);
    for k in 0..3 {
        let apart = round(held[k], held[k + 1], 1000.0);
        assert!(
            apart >= 10.0 - 1e-3,
            "ends {k} and {} are {apart} apart, they need 10: {held:?}",
            k + 1
        );
    }
    let unwrapped: f32 = held.iter().map(|&h| if h > 500.0 { h - 1000.0 } else { h }).sum();
    assert!(
        (unwrapped - 33.0).abs() < 1e-3,
        "centered where the four faced, 33 summed: {held:?}"
    );
}

#[test]
fn test_projection_held_apart_reaches_across_the_seam() {
    // Two ends either side of the start of the top side, 7 apart round the
    // seam and 993 apart along the perimeter: they are pushed apart across
    // it, 1.5 each, and not left on top of one another.
    let held = held_apart(&[(2.0, 10.0), (995.0, 10.0)], 1000.0);
    assert!(
        (round(held[0], held[1], 1000.0) - 10.0).abs() < 1e-3,
        "the two ends across the seam are half their widths summed apart: {held:?}"
    );
    assert!(
        (held[0] - 3.5).abs() < 1e-3 && (held[1] - 993.5).abs() < 1e-3,
        "each moved 1.5: {held:?}"
    );
}

#[test]
fn test_projection_portal_ends_that_face_one_way_are_held_apart() {
    // Three ends side by side round 100: the middle two would overlap, so
    // they are pushed apart and centered where they faced; the first,
    // clear, stays.
    let ends = [(0.0, 10.0), (100.0, 20.0), (104.0, 20.0)];
    let held = held_apart(&ends, 1000.0);
    assert!(
        (held[0] - 0.0).abs() < 1e-4,
        "an end clear of the rest stays: {held:?}"
    );
    assert!(
        (held[2] - held[1] - 20.0).abs() < 1e-4,
        "two ends half their widths summed apart: {held:?}"
    );
    assert!(
        (held[1] + held[2] - 204.0).abs() < 1e-3,
        "centered where they faced: {held:?}"
    );
    // Projected: node 1's two portals to node 2 face it from one point.
    let mut p = Projected::new(load_map(json!({"nodes": [
        node(1, None, json!({"sections": [text_section("source")], "links": [
            {"to": 2, "kind": "portal"}, {"to": 2, "kind": "portal"}
        ]})),
        node(2, None, json!({"position": [400.0, 0.0], "sections": [text_section("target")]})),
    ]})));
    let one = ident(&p.doc, 1);
    let portals: Vec<NodeId> = p
        .doc
        .children(one)
        .filter(|&c| p.doc.get(c).is_some_and(|e| e.kind() == DocKind::Link))
        .collect();
    let center = |id: NodeId| match &p.projector.units[&id].body {
        UnitBody::Portal(portal) => portal.end[0].center.expect("its end is placed"),
        _ => panic!("a portal"),
    };
    let (a, b) = (center(portals[0]), center(portals[1]));
    let marker = match &p.projector.units[&portals[0]].body {
        UnitBody::Portal(portal) => {
            portal.end[0]
                .marker
                .placed
                .as_ref()
                .expect("placed")
                .run
                .line_height
        }
        _ => unreachable!(),
    };
    assert!(
        a.distance(b) >= 0.99 * marker,
        "the two ends {a} and {b} are {} apart, a marker is {marker} tall",
        a.distance(b)
    );
    p.check().unwrap();
}

/// The pairs of `samples`, each `(lower, higher)`, nearer than
/// [`CLEARANCE`] times the wider of their `widths`, each pair measured
/// point by point: what [`near_pairs`] answers from one grid.
fn near_pairwise(samples: &[Vec<Vec2>], widths: &[f32]) -> rustc_hash::FxHashSet<(usize, usize)> {
    let mut pairs = rustc_hash::FxHashSet::default();
    for k in 0..samples.len() {
        for j in k + 1..samples.len() {
            let need = CLEARANCE * widths[k].max(widths[j]);
            let close = samples[k]
                .iter()
                .any(|p| samples[j].iter().any(|q| q.distance(*p) < need));
            if close {
                pairs.insert((k, j));
            }
        }
    }
    pairs
}

/// A small deterministic source of numbers in `0..100`.
fn scatter(mut seed: u32) -> impl FnMut() -> f32 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 100.0
    }
}

#[test]
fn test_projection_near_pairs_finds_the_pairs_measured_point_by_point() {
    // Lanes of scattered points and of differing widths, some crowded and
    // some apart, some of them fresh: one grid of the fresh lanes' points,
    // and the pairs of two lanes not fresh carried from before, find the
    // near pairs exactly as measuring each pair point by point does. The
    // failing inputs are a grid as wide as one lane's own distance, which
    // misses a pair held apart by the wider of the two, and a pair of a
    // fresh lane carried rather than measured.
    let mut next = scatter(0x9e37_79b9);
    let (mut some_near, mut some_clear, mut some_carried) = (0, 0, 0);
    for _ in 0..300 {
        let lanes = 2 + (next() as usize) % 7;
        let samples: Vec<Vec<Vec2>> = (0..lanes)
            .map(|_| {
                (0..(1 + next() as usize % 12))
                    .map(|_| Vec2::new(next(), next()))
                    .collect()
            })
            .collect();
        let widths: Vec<f32> = (0..lanes).map(|_| 0.5 + next() * 0.08).collect();
        let want = near_pairwise(&samples, &widths);
        let all = vec![true; lanes];
        assert_eq!(
            near_pairs(&samples, &widths, &all, &Default::default()),
            want,
            "every lane fresh: lanes {samples:?}, widths {widths:?}"
        );
        // Some lanes moved since `before` was measured, at their old
        // places: what is carried is only what they could not change.
        let fresh: Vec<bool> = (0..lanes).map(|_| next() < 50.0).collect();
        let moved: Vec<Vec<Vec2>> = samples
            .iter()
            .zip(&fresh)
            .map(|(points, &f)| {
                if f {
                    points
                        .iter()
                        .map(|p| *p + Vec2::new(next() - 50.0, 0.0))
                        .collect()
                } else {
                    points.clone()
                }
            })
            .collect();
        let before = near_pairwise(&moved, &widths);
        some_carried += before.iter().filter(|&&(k, j)| !fresh[k] && !fresh[j]).count();
        assert_eq!(
            near_pairs(&samples, &widths, &fresh, &before),
            want,
            "fresh {fresh:?}: lanes {samples:?}, widths {widths:?}"
        );
        some_near += want.len();
        some_clear += lanes * (lanes - 1) / 2 - want.len();
    }
    assert!(
        some_near > 0 && some_clear > 0 && some_carried > 0,
        "precondition: pairs near ({some_near}), clear ({some_clear}) and carried ({some_carried})"
    );
}

/// [`untangle`] as it reads, each round measuring every pair of lanes
/// afresh: what the rounds that measure only the halved lanes again
/// answer.
fn untangle_afresh(lanes: &[Lane]) -> Vec<f32> {
    let mut handles = vec![1.0_f32; lanes.len()];
    if lanes.len() < 2 {
        return handles;
    }
    let widths: Vec<f32> = lanes.iter().map(|lane| lane.width).collect();
    let mut best = (usize::MAX, handles.clone());
    for _ in 0..UNTANGLE_ROUNDS {
        let samples: Vec<Vec<Vec2>> = lanes
            .iter()
            .zip(&handles)
            .map(|(lane, &h)| {
                let curves = if lane.straight {
                    vec![Curve::Line(lane.start.0, lane.end.0)]
                } else {
                    path_through(lane.start, lane.end, &lane.via, h)
                };
                sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width))
            })
            .collect();
        let mut near = vec![false; lanes.len()];
        for (k, j) in near_pairwise(&samples, &widths) {
            near[k] = true;
            near[j] = true;
        }
        let count = near.iter().filter(|&&n| n).count();
        if count < best.0 {
            best = (count, handles.clone());
        }
        if count == 0 {
            break;
        }
        for (k, h) in handles.iter_mut().enumerate() {
            if near[k] && !lanes[k].straight {
                *h = (*h * 0.5).max(1.0 / 4096.0);
            }
        }
    }
    best.1
}

/// `count` paths from one point down to a row `wide` across, `straight`
/// as the flag says: the lanes of a junction.
fn row_lanes(count: usize, wide: f32, straight: impl Fn(usize) -> bool) -> Vec<Lane> {
    (0..count)
        .map(|k| Lane {
            start: (Vec2::ZERO, Vec2::Y, 4.0),
            hold: 0.0,
            end: (
                Vec2::new(k as f32 * wide / count as f32 - 0.5 * wide, 400.0),
                -Vec2::Y,
                4.0,
            ),
            via: Vec::new(),
            width: 3.0,
            straight: straight(k),
        })
        .collect()
}

#[test]
fn test_projection_untangle_answers_as_measuring_every_pair_each_round() {
    // Rows narrow and wide, some paths straight, two lying on one line
    // from the junction so that they never clear: measuring again only
    // the lanes a round halved finds the handles that measuring every
    // pair afresh each round finds.
    for (count, wide) in [(3, 30.0), (7, 60.0), (12, 300.0), (20, 120.0)] {
        let mut lanes = row_lanes(count, wide, |k| k % 5 == 3);
        lanes[1].end.0 = lanes[0].end.0 * 0.5;
        assert_eq!(
            untangle(&lanes),
            untangle_afresh(&lanes),
            "{count} paths {wide} wide"
        );
    }
    // Junctions drawn at random, some paths straight and some through a
    // point, crowded enough that rounds run: among them pairs of straight
    // paths too near each other, carried from round to round since
    // neither is ever halved, and paths halved together, one passing
    // where the other was the round before. The failing inputs are a
    // carried pair dropped, and a halved path's old points left in the
    // grid, each found near a curve that is no longer there.
    let mut next = scatter(2 * 7919);
    let (mut carried, mut rounds) = (0, 0);
    for case in 0..200 {
        let count = 2 + (next() as usize) % 6;
        let lanes: Vec<Lane> = (0..count)
            .map(|_| {
                let end = Vec2::new((next() - 50.0) * 4.0, 20.0 + next() * 3.0);
                let leaves = Vec2::from_angle((next() - 50.0) * 0.012).rotate(Vec2::Y);
                Lane {
                    start: (Vec2::ZERO, leaves, 4.0),
                    hold: 0.0,
                    end: (end, -Vec2::Y, 4.0),
                    via: if next() < 80.0 {
                        vec![Vec2::new((next() - 50.0) * 2.0, next() - 50.0)]
                    } else {
                        Vec::new()
                    },
                    width: 3.0,
                    straight: next() < 15.0,
                }
            })
            .collect();
        UNTANGLE_GRIDS.with(|c| c.set(0));
        let got = untangle(&lanes);
        rounds += UNTANGLE_GRIDS.with(std::cell::Cell::get);
        // Two straight lanes too near each other: a pair carried.
        let straight: Vec<Vec<Vec2>> = lanes
            .iter()
            .filter(|lane| lane.straight)
            .map(|lane| {
                let line = [Curve::Line(lane.start.0, lane.end.0)];
                sample(&line, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width))
            })
            .collect();
        let widths = vec![3.0; straight.len()];
        carried += usize::from(!near_pairwise(&straight, &widths).is_empty());
        assert_eq!(got, untangle_afresh(&lanes), "case {case}: {lanes:?}");
    }
    assert!(
        carried > 0 && rounds > 400,
        "precondition: straight pairs too near ({carried}) and rounds after the first ({rounds} in all)"
    );
}

/// `count` paths fanned out of one point, held out `hold`, to a row
/// `wide` across and 600 below: the lanes of a junction at its cap.
fn fanned_lanes(count: usize, wide: f32, hold: f32) -> Vec<Lane> {
    (0..count)
        .map(|k| {
            let turn = (k as f32 / (count - 1) as f32 - 0.5) * 2.0 * SPLAY_CAP;
            Lane {
                start: (Vec2::ZERO, Vec2::from_angle(-turn).rotate(Vec2::Y), 4.0 + hold),
                hold,
                end: (
                    Vec2::new(k as f32 * wide / (count - 1) as f32 - 0.5 * wide, 600.0),
                    -Vec2::Y,
                    4.0,
                ),
                via: Vec::new(),
                width: 3.0,
                straight: false,
            }
        })
        .collect()
}

#[test]
fn test_projection_untangle_measures_again_only_what_it_moved() {
    // Eight paths fanned to a wide row, all clear of each other but two
    // whose far ends lie on one line from the point, so that every round
    // runs and halves those two alone: a round after the first grids and
    // measures again only the lanes it halved, so the whole untangling
    // grids no more points than the first round and that pair's own
    // untangling between them — not every lane's, every round.
    let mut lanes = fanned_lanes(8, 1600.0, 40.0);
    lanes[1].end.0 = lanes[0].end.0 * 0.5;
    lanes[1].start.1 = lanes[0].start.1;
    let pair: Vec<Lane> = lanes.iter().take(2).cloned().collect();
    let gridded = |lanes: &[Lane]| {
        UNTANGLE_GRIDS.with(|count| count.set(0));
        UNTANGLE_GRIDDED.with(|count| count.set(0));
        let handles = untangle(lanes);
        (
            handles,
            UNTANGLE_GRIDS.with(std::cell::Cell::get),
            UNTANGLE_GRIDDED.with(std::cell::Cell::get),
        )
    };
    let (pair_handles, pair_rounds, pair_points) = gridded(&pair);
    let (handles, rounds, points) = gridded(&lanes);
    assert!(
        rounds == UNTANGLE_ROUNDS && pair_rounds == UNTANGLE_ROUNDS,
        "precondition: every round runs: {rounds}, {pair_rounds}"
    );
    assert!(
        handles[2..].iter().all(|&h| h == 1.0) && handles[..2] == pair_handles[..],
        "precondition: the pair alone is halved: {handles:?}"
    );
    let first: usize = lanes[2..]
        .iter()
        .map(|lane| {
            let curves = path_through(lane.start, lane.end, &lane.via, 1.0);
            sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width)).len()
        })
        .sum();
    assert!(
        points == first + pair_points,
        "{points} points gridded: the six clear lanes' {first} once, and the pair's {pair_points}"
    );
}

/// [`untangle`] of `lanes`, and what it did: its rounds, the samples it
/// gridded, the points it measured another lane's against, and the points
/// of another lane it measured them against.
fn untangle_counted(lanes: &[Lane]) -> (Vec<f32>, [usize; 4]) {
    for counter in [
        &UNTANGLE_GRIDS,
        &UNTANGLE_GRIDDED,
        &UNTANGLE_CLOSE,
        &UNTANGLE_MEASURED,
    ] {
        counter.with(|c| c.set(0));
    }
    let handles = untangle(lanes);
    let read =
        |counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>| counter.with(std::cell::Cell::get);
    (
        handles,
        [
            read(&UNTANGLE_GRIDS),
            read(&UNTANGLE_GRIDDED),
            read(&UNTANGLE_CLOSE),
            read(&UNTANGLE_MEASURED),
        ],
    )
}

#[test]
fn test_projection_untangle_work_does_not_grow_with_how_thin_a_lane_is() {
    // A wide path and two thin ones beside it, straight and 2,000 long,
    // none near another: the wide one 5 from the first thin one, which is
    // 1 from the second; upright, turned half a radian, and diagonal. The
    // thin ones are an eighth as wide as the wide one, a sixty-fourth, a
    // thousandth: the thinner they are, the more points they are sampled
    // in, to the sampling's cap of 65,536 a curve, ten times the wide
    // lane's and more. Yet a point is measured against at most the 22
    // points a straight lane sampled every quarter of its width has in the
    // nine of its own fine cells round it (their diagonal over its step,
    // and one), and each point at most twice — not against every point in
    // the coarse cells as wide as the wide path's distance, which the thin
    // lanes' own points fill (#346: a link a twenty-fourth as wide as the
    // connector beside it timed the loader's fuzz target out). The
    // failing inputs are the coarse cells alone, and fine cells as wide as
    // the coarse ones.
    for turn in [0.0_f32, 0.5, std::f32::consts::FRAC_PI_4] {
        for thinner in [8.0, 64.0, 1024.0] {
            let at = |x: f32, y: f32| Vec2::from_angle(turn).rotate(Vec2::new(x, y));
            let lane = |x: f32, width: f32| Lane {
                start: (at(x, 0.0), at(0.0, 1.0), 4.0),
                hold: 0.0,
                end: (at(x, 2000.0), -at(0.0, 1.0), 4.0),
                via: Vec::new(),
                width,
                straight: true,
            };
            let lanes = [lane(0.0, 3.0), lane(5.0, 3.0 / thinner), lane(6.0, 3.0 / thinner)];
            let (handles, [rounds, gridded, close, measured]) = untangle_counted(&lanes);
            assert!(
                rounds == 1 && handles == [1.0; 3] && gridded > 10 * 2000 * 4 / 3,
                "precondition: {turn}, {thinner}: one round, all clear, the thin lanes finely sampled: {rounds}, {handles:?}, {gridded}"
            );
            // The thin lanes run beside the wide one, so their points are
            // measured against it, and counted.
            assert!(
                close > 0 && measured > 0,
                "precondition: {turn}, {thinner}: points measured: {close}, {measured}"
            );
            assert!(
                measured <= 22 * close && close <= 2 * gridded,
                "{turn}, {thinner}: {measured} points measured against {close} times, of {gridded} gridded"
            );
        }
    }
}

#[test]
fn test_projection_untangle_measures_a_point_only_against_lanes_near_it() {
    // A wide path and 4, 16, then 32 thin ones fanned from one point to
    // ends 40 apart, 2,000 below, all straight, and held out far enough
    // to be clear of each other: they share the coarse cells round the
    // junction and part within some of them. A point is measured against
    // a lane at most once, and only against a lane with points in the
    // nine coarse cells round it — cells as wide as the widest path's
    // distance — however many paths the junction has; and the lanes near
    // each cell of a path are listed in one step for each lane read in
    // the nine cells round it, whatever the lanes listed already (on
    // #347). The failing inputs are each pair measured over the whole of
    // one lane once the two share any coarse cells, which measures every
    // point of a path against every other path of the junction, the paths
    // squared; and the list searched for each lane before it is added,
    // the lanes of a cell squared, and a crowded junction's cubed.
    for count in [4, 16, 32] {
        let lane = |x: f32, width: f32| Lane {
            start: (Vec2::ZERO, Vec2::Y, 4.0),
            hold: 20.0,
            end: (Vec2::new(x, 2000.0), -Vec2::Y, 4.0),
            via: Vec::new(),
            width,
            straight: true,
        };
        let lanes: Vec<Lane> = std::iter::once(lane(-400.0, 3.0))
            .chain((0..count).map(|k| lane(k as f32 * 40.0, 0.1)))
            .collect();
        UNTANGLE_LISTED.with(|c| c.set(0));
        let (handles, [rounds, _, close, _]) = untangle_counted(&lanes);
        let listed = UNTANGLE_LISTED.with(std::cell::Cell::get);
        assert!(
            rounds == 1 && handles.iter().all(|&h| h == 1.0) && close > 0,
            "precondition: {count} paths: one round, all clear, the junction measured: {rounds}, {close}"
        );
        // Each lane's points, and the lanes with points in each coarse cell.
        let samples: Vec<Vec<Vec2>> = lanes
            .iter()
            .map(|lane| {
                let line = [Curve::Line(lane.start.0, lane.end.0)];
                sample(&line, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width))
            })
            .collect();
        let side = CLEARANCE * 3.0;
        let cell = |p: Vec2| ((p.x / side).floor() as i64, (p.y / side).floor() as i64);
        let mut held: rustc_hash::FxHashMap<(i64, i64), rustc_hash::FxHashSet<usize>> = Default::default();
        for (k, points) in samples.iter().enumerate() {
            for &p in points {
                held.entry(cell(p)).or_default().insert(k);
            }
        }
        let near: usize = samples
            .iter()
            .enumerate()
            .flat_map(|(k, points)| points.iter().map(move |&p| (k, p)))
            .map(|(k, p)| {
                let (x, y) = cell(p);
                let mut lanes: rustc_hash::FxHashSet<usize> = Default::default();
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        lanes.extend(
                            held.get(&(x + dx, y + dy))
                                .into_iter()
                                .flatten()
                                .filter(|&&j| j != k),
                        );
                    }
                }
                lanes.len()
            })
            .sum();
        assert!(
            close <= near,
            "{count} paths: {close} points measured against a lane, of {near} with the lane near them"
        );
        // Every lane is fresh in the one round: each reads, for each of its
        // cells, the lanes in the nine round it.
        let read: usize = samples
            .iter()
            .map(|points| {
                let cells: rustc_hash::FxHashSet<(i64, i64)> = points.iter().map(|&p| cell(p)).collect();
                cells
                    .iter()
                    .map(|&(x, y)| {
                        (-1..=1)
                            .flat_map(|dx| (-1..=1).map(move |dy| (x + dx, y + dy)))
                            .map(|at| held.get(&at).map_or(0, |lanes| lanes.len()))
                            .sum::<usize>()
                    })
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(
            listed, read,
            "{count} paths: steps listing the lanes near each cell"
        );
    }
}

/// Two paths from one point to one point along one line, as a line link
/// and the connector beside it: one as wide as a connector, one as thin
/// as a link at the default size. Its leads are as `refan` gives them.
fn coincident_lanes() -> Vec<Lane> {
    let lane = |width: f32, lead: f32| Lane {
        start: (Vec2::ZERO, Vec2::Y, lead + 3.2),
        hold: 3.2,
        end: (Vec2::new(0.0, 5000.0), -Vec2::Y, lead),
        via: Vec::new(),
        width,
        straight: false,
    };
    vec![lane(3.457, 6.914), lane(0.144, 0.288)]
}

#[test]
fn test_projection_untangle_stops_when_no_round_can_part_its_paths() {
    // Two paths that arrive at one point along one line meet in every
    // round, whatever their handles, along the leads they arrive on: no
    // round can part them, so the first is the only one, and its samples
    // the only ones gridded — not fourteen rounds of each path's, the
    // thin one at the sampling's cap (#346: 140 rounds and 9.3 million
    // points sampled, gridded and taken out again in one check of the
    // loader's fuzz target). A third path clear of them leaves the count
    // as it is. The pair keeps its curves, as every round would keep them.
    //
    // Junctions drawn at random, some of their paths sharing one far end
    // along one lead, as many from them sharing a far end on another lead
    // that halving parts: what is kept is what every round would keep.
    // The failing inputs are the fourteen rounds, and a stop at the first
    // round with paths too near, whether or not a later one parts them.
    let pair = coincident_lanes();
    let mut three = pair.clone();
    three.push(Lane {
        start: (Vec2::new(-1000.0, 0.0), Vec2::Y, 4.0),
        hold: 3.2,
        end: (Vec2::new(-3000.0, 4000.0), -Vec2::Y, 4.0),
        via: Vec::new(),
        width: 3.457,
        straight: false,
    });
    for lanes in [pair, three] {
        let first: usize = lanes
            .iter()
            .map(|lane| {
                let curves = path_through(lane.start, lane.end, &lane.via, 1.0);
                sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width)).len()
            })
            .sum();
        let (handles, [rounds, gridded, _, _]) = untangle_counted(&lanes);
        assert!(
            handles[..2] == [1.0, 1.0] && first > 65_536,
            "precondition: {} paths: the pair keeps its curves, the thin one sampled at the cap: {handles:?}, {first}",
            lanes.len()
        );
        assert!(
            rounds == 1 && gridded == first,
            "{} paths: {rounds} rounds, {gridded} points gridded, {first} in the first",
            lanes.len()
        );
    }
    let mut next = scatter(3 * 7919);
    let (mut stopped, mut parted) = (0, 0);
    for case in 0..200 {
        // Junctions drawn as in the test of `untangle` against every pair
        // measured afresh, some paths arriving at one shared end along one
        // lead.
        let shared = Vec2::new((next() - 50.0) * 4.0, 25.0);
        let count = 2 + (next() as usize) % 6;
        let lanes: Vec<Lane> = (0..count)
            .map(|_| {
                let end = if next() < 40.0 {
                    shared
                } else {
                    Vec2::new((next() - 50.0) * 4.0, 20.0 + next() * 3.0)
                };
                let leaves = Vec2::from_angle((next() - 50.0) * 0.012).rotate(Vec2::Y);
                Lane {
                    start: (Vec2::ZERO, leaves, 4.0),
                    hold: 0.0,
                    end: (end, -Vec2::Y, 4.0),
                    via: if next() < 80.0 {
                        vec![Vec2::new((next() - 50.0) * 2.0, next() - 50.0)]
                    } else {
                        Vec::new()
                    },
                    width: 3.0,
                    straight: next() < 15.0,
                }
            })
            .collect();
        let (handles, [rounds, ..]) = untangle_counted(&lanes);
        assert_eq!(handles, untangle_afresh(&lanes), "case {case}: {lanes:?}");
        let kept: Vec<Vec<Vec2>> = lanes
            .iter()
            .zip(&handles)
            .map(|(lane, &h)| {
                let curves = if lane.straight {
                    vec![Curve::Line(lane.start.0, lane.end.0)]
                } else {
                    path_through(lane.start, lane.end, &lane.via, h)
                };
                sample(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width))
            })
            .collect();
        let widths: Vec<f32> = lanes.iter().map(|lane| lane.width).collect();
        stopped += usize::from(rounds < UNTANGLE_ROUNDS && !near_pairwise(&kept, &widths).is_empty());
        parted += usize::from(handles.iter().any(|&h| h < 1.0));
    }
    assert!(
        stopped > 0 && parted > 0,
        "precondition: junctions stopped early ({stopped}) and junctions parted by halving ({parted})"
    );
}

/// A junction's samples do not move with where it lies (#356): a lane's
/// curves sampled from its skip on keep the same samples at the origin and
/// moved 2²³ to 2⁶³ units of their own scale from it, where every control
/// point is still exactly a float, so the curves are the same curves —
/// `sample` measures how far along a sample lies in the curve's own frame
/// (`flow::Local`). Summed between points as large as where the curve
/// lies, that distance counted their rounding: far out the skip was
/// reached early, and more samples were kept than at the origin.
///
/// Control: `sample` measuring along between its points as `f32`, where
/// they lie, red far out (2,627 samples kept 2²³ from the origin, against
/// 2,598 at it).
#[test]
fn test_projection_a_far_out_junction_keeps_the_samples_it_keeps_at_the_origin() {
    for scale in [0, 10, 20, 30, 40] {
        // A unit at which every control point, moved by `far`, is a float.
        let unit = 2f32.powi(scale);
        let far = 2f32.powi(scale + 23);
        let cubic = |at: f32| {
            let point = |x: f32, y: f32| Vec2::new(at + x * unit, at + y * unit);
            Curve::Cubic(
                point(0.0, 0.0),
                point(3_000.0, -2_000.0),
                point(7_000.0, 2_000.0),
                point(10_000.0, 0.0),
            )
        };
        let lead = |at: f32| Curve::Line(Vec2::splat(at), Vec2::new(at + 64.0 * unit, at));
        let (step, skip) = (4.0 * unit, 2_000.0 * unit);
        let kept = |at: f32| sample(&[lead(at), cubic(at)], step, skip).len();
        let (here, there) = (kept(0.0), kept(far));
        let all = sample(&[lead(0.0), cubic(0.0)], step, 0.0).len();
        assert!(
            here > 0 && here < all,
            "precondition: the skip keeps some samples and drops some ({here} of {all})"
        );
        assert_eq!(
            here,
            there,
            "at 2^{} from the origin: {there} samples kept, against {here} at it",
            scale + 23
        );
    }
}

#[test]
fn test_projection_a_lanes_fixed_samples_are_in_every_round() {
    // What the stop rests on: every sample `fixed_samples` reads is one
    // that every round of `untangle` takes, bit for bit, at every share of
    // the handles a round can give, 1 down to 1/4096. Curved lanes drawn
    // at random, their length near the distance a round's samples start
    // from, where whether the last lead's are taken is decided: widths
    // from a hundredth to 40, held out up to ten widths, through 0 to 2
    // points, leads of any length, far from the origin and near it. The
    // failing input is the last lead's samples read whenever the chord of
    // the curves before it reaches that distance (#346, on #347).
    let mut next = scatter(0x1234_5678);
    let offsets = [0.0_f32, 1.0e3, 1.0e5, 1.0e6, 4.0e6, 1.6e7];
    let (mut with_last, mut without_last) = (0, 0);
    for case in 0..20_000 {
        let at = Vec2::new(
            offsets[next() as usize % offsets.len()],
            offsets[next() as usize % offsets.len()],
        );
        let width = [0.01_f32, 0.05, 0.144, 1.0, 3.457, 40.0][next() as usize % 6];
        let hold = [0.0, 0.5, 2.0, 10.0][next() as usize % 4] * width;
        let skip = CLEARANCE * hold.max(width);
        let length = skip * (0.5 + next() / 10.0);
        let turn = |n: f32| Vec2::from_angle((n - 50.0) * 0.0628);
        let via = match next() as usize % 4 {
            0 => Vec::new(),
            1 => vec![Vec2::ZERO],
            2 => vec![Vec2::new(next() - 50.0, next() - 50.0) * length / 50.0],
            _ => vec![
                Vec2::new(next() - 50.0, 0.0) * length / 10.0,
                Vec2::new(0.0, next() - 50.0) * length / 10.0,
            ],
        };
        let lane = Lane {
            start: (at, turn(next()).rotate(Vec2::Y), next() * length / 100.0),
            hold,
            end: (
                at + turn(next()).rotate(Vec2::Y) * length,
                turn(next()).rotate(-Vec2::Y),
                next() * length / 100.0,
            ),
            via,
            width,
            straight: false,
        };
        let fixed = fixed_samples(&lane);
        let curves = path_through(lane.start, lane.end, &lane.via, 1.0);
        let first = match curves.first() {
            Some(lead @ Curve::Line(..)) => sample(&[*lead], 0.25 * width, skip).len(),
            _ => 0,
        };
        if matches!(curves.last(), Some(Curve::Line(..))) {
            if fixed.len() > first {
                with_last += 1;
            } else {
                without_last += 1;
            }
        }
        for k in 0..=12 {
            let taken: rustc_hash::FxHashSet<(u32, u32)> = sample(
                &path_through(lane.start, lane.end, &lane.via, 0.5_f32.powi(k)),
                0.25 * width,
                skip,
            )
            .iter()
            .map(|p| (p.x.to_bits(), p.y.to_bits()))
            .collect();
            assert!(
                fixed
                    .iter()
                    .all(|p| taken.contains(&(p.x.to_bits(), p.y.to_bits()))),
                "case {case}: a fixed sample not taken at share 2^-{k}: {lane:?}"
            );
        }
    }
    assert!(
        with_last > 0 && without_last > 0,
        "precondition: last leads read ({with_last}) and not ({without_last})"
    );
}

#[test]
fn test_projection_untangle_takes_every_round_on_two_paths_that_meet_along_their_curves() {
    // Two curved paths leaving one point the same way, held out past
    // their first lead, to far ends on one line from it, the near one a
    // link at the default size: they meet through their curves in every
    // round, and through no sample a halving cannot move, so `untangle`
    // takes every round, each sampling the thin path's curve again at the
    // sampling's cap (on #347). What holds for such a junction is the
    // bound: at most `UNTANGLE_ROUNDS` rounds, each sampling each lane's
    // curves at most to the cap — not one round, as for the seeds' pair,
    // whose paths meet along a lead.
    let lane = |width: f32, end: f32| Lane {
        start: (Vec2::ZERO, Vec2::Y, 44.0),
        hold: 40.0,
        end: (Vec2::new(0.0, end), -Vec2::Y, 2.0 * width),
        via: Vec::new(),
        width,
        straight: false,
    };
    let lanes = [lane(3.457, 5000.0), lane(0.144, 2500.0)];
    let curves: usize = lanes
        .iter()
        .map(|l| path_through(l.start, l.end, &l.via, 1.0).len())
        .sum();
    let (handles, [rounds, gridded, _, _]) = untangle_counted(&lanes);
    assert_eq!(handles, [1.0, 1.0], "the first round's curves kept");
    assert!(
        rounds == UNTANGLE_ROUNDS && gridded > (UNTANGLE_ROUNDS - 1) * 65_536,
        "every round, the thin path at the cap each: {rounds} rounds, {gridded} points gridded"
    );
    assert!(
        gridded <= UNTANGLE_ROUNDS * 65_536 * curves,
        "{gridded} points gridded, past {UNTANGLE_ROUNDS} rounds of {curves} curves at the cap"
    );
}

#[test]
fn test_projection_untangle_keeps_one_stamp_a_lane_on_a_very_wide_side() {
    // Four thousand paths on one side, as a node with that many children
    // in a row has: what untangling them allocates to know which pairs it
    // has found grows with the paths, one stamp each a round, not with
    // their pairs — 16 million entries a round as a table of pairs, which
    // past 46 340 paths no longer fits a 32-bit address space. Each path
    // here is short and clear of the others, so one round settles them
    // and no stop is read (`always_near` runs only when a round leaves a
    // path too near): exactly one stamp a path a round. The failing
    // inputs are a table of pairs, and stamps not counted.
    let count = 4096;
    let lanes: Vec<Lane> = (0..count)
        .map(|k| {
            let at = Vec2::new(k as f32 * 40.0, 0.0);
            Lane {
                start: (at, Vec2::Y, 4.0),
                hold: 0.0,
                end: (at + Vec2::new(0.0, 30.0), -Vec2::Y, 4.0),
                via: Vec::new(),
                width: 3.0,
                straight: false,
            }
        })
        .collect();
    UNTANGLE_GRIDS.with(|c| c.set(0));
    UNTANGLE_SCRATCH.with(|c| c.set(0));
    let handles = untangle(&lanes);
    let (rounds, scratch) = (
        UNTANGLE_GRIDS.with(std::cell::Cell::get),
        UNTANGLE_SCRATCH.with(std::cell::Cell::get),
    );
    assert!(
        rounds >= 1 && handles.iter().all(|&h| h == 1.0),
        "precondition: the paths are measured and clear: {rounds} rounds"
    );
    assert_eq!(
        scratch,
        rounds * count,
        "entries over {rounds} rounds for {count} paths"
    );
}
