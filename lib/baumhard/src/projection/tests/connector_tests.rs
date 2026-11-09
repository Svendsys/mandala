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
    held_apart, near_pairs, path_through, side_point, side_toward, untangle, Lane, CLEARANCE, SPLAY_CAP,
    UNTANGLE_GRIDDED, UNTANGLE_GRIDS, UNTANGLE_ROUNDS, UNTANGLE_SCRATCH,
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
                let step = (0.25 * lane.width).max(1e-2);
                let skip = CLEARANCE * lane.hold.max(lane.width);
                points_along(&curves, step, skip)
            })
            .collect();
        let mut near = vec![false; lanes.len()];
        for (k, j) in near_pairwise(&samples, &widths) {
            near[k] = true;
            near[j] = true;
        }
        let count = near.iter().filter(|&&n| n).count();
        if count <= best.0 {
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

/// Points along `curves` at most `step` apart, from `skip` along on, as
/// the untangling samples them.
fn points_along(curves: &[Curve], step: f32, skip: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    let mut along = 0.0_f32;
    for curve in curves {
        let (a, d) = (curve.point(0.0), curve.point(1.0));
        let reach = match *curve {
            Curve::Cubic(_, b, c, _) => a.distance(b) + b.distance(c) + c.distance(d),
            Curve::Quadratic(_, b, _) => a.distance(b) + b.distance(d),
            Curve::Line(..) => a.distance(d),
        };
        if !reach.is_finite() {
            continue;
        }
        let n = ((reach / step).ceil() as usize).clamp(1, 1 << 16);
        let mut last = a;
        for i in 0..n {
            let p = curve.point(i as f32 / n as f32);
            along += p.distance(last);
            last = p;
            if along >= skip {
                out.push(p);
            }
        }
    }
    out
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
            points_along(&curves, 0.25 * lane.width, CLEARANCE * lane.hold.max(lane.width)).len()
        })
        .sum();
    assert!(
        points == first + pair_points,
        "{points} points gridded: the six clear lanes' {first} once, and the pair's {pair_points}"
    );
}

#[test]
fn test_projection_untangle_keeps_one_stamp_a_lane_on_a_very_wide_side() {
    // Four thousand paths on one side, as a node with that many children
    // in a row has: what untangling them allocates to know which pairs it
    // has found grows with the paths, one stamp each a round, not with
    // their pairs — 16 million entries a round as a table of pairs, which
    // past 46 340 paths no longer fits a 32-bit address space. Each path
    // here is short and clear of the others, so one round settles them.
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
    assert!(
        scratch <= rounds * count,
        "{scratch} entries over {rounds} rounds for {count} paths"
    );
}
