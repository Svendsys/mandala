// SPDX-License-Identifier: MPL-2.0

//! Hit testing (#241 §6, ruling B7.1): through the layers top-down to a
//! glyph's element and span, then the nodes' bodies by their shape; and
//! the body index's cost on a move.

use glam::Vec2;
use serde_json::json;

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::mutation::ops::{DocMutation, NodeOp};
use crate::document::tests::writer_support::{request, single};
use crate::flow::Rect;
use crate::gfx_structs::run::RunShape;
use crate::projection::tests::maps::{ident, kid, kids, load_map, node};
use crate::projection::tests::support::Projected;
use crate::projection::Layer;

/// The canvas center of glyph `index` of the box run `unit` wrote.
fn box_glyph_center(p: &Projected, unit: NodeId, index: usize) -> Vec2 {
    let run = p.projector.units[&unit].slots[0].written.clone().unwrap();
    let RunShape::Box(shaped) = run.shaped() else { unreachable!("a section is a box run") };
    let mut at = 0;
    for line in shaped.buffer.layout_runs() {
        for glyph in line.glyphs.iter() {
            if at == index {
                return run.origin()
                    + Vec2::new(glyph.x + glyph.w * 0.5, line.line_top + line.line_height * 0.5);
            }
            at += 1;
        }
    }
    unreachable!("the run has glyph {index}")
}

/// A node with one section of two spans, the second linked, and a large
/// minimum size, so its body reaches well past its text: the projected
/// map, the node, the section and its two spans.
fn linked_node() -> (Projected, NodeId, NodeId, NodeId, NodeId) {
    let doc = load_map(json!({"nodes": [node(1, None, json!({
        "position": [100.0, 100.0],
        "min_size": [400.0, 300.0],
        "sections": [{"spans": [
            {"text": "plain "},
            {"text": "linked", "link": {"url": "https://example.com"}}
        ]}]
    }))]}));
    let node = ident(&doc, 1);
    let section = kid(&doc, node, DocKind::Section);
    let spans = kids(&doc, section, DocKind::Span);
    (Projected::new(doc), node, section, spans[0], spans[1])
}

#[test]
fn test_projection_a_click_on_a_linked_span_finds_that_span() {
    let (mut p, node, section, plain, linked) = linked_node();
    let spans = {
        let run = p.projector.units[&section].slots[0].written.clone().unwrap();
        let RunShape::Box(shaped) = run.shaped() else { unreachable!() };
        shaped.glyph_spans.clone()
    };
    let first_linked = spans.iter().position(|(span, _)| *span == 1).unwrap();
    let point = box_glyph_center(&p, section, first_linked);
    let hit = p.projector.hit(&mut p.layers, point).expect("a glyph is there");
    assert_eq!(hit.node, node);
    assert_eq!(hit.element, section, "the section's run");
    assert_eq!(hit.span, Some(linked), "the linked span");
    assert_eq!(hit.glyph, Some(first_linked), "its first glyph");
    // The control: a glyph of the other span names that span.
    let point = box_glyph_center(&p, section, 0);
    let hit = p.projector.hit(&mut p.layers, point).unwrap();
    assert_eq!(hit.span, Some(plain), "the first glyph is the plain span's");
}

#[test]
fn test_projection_a_click_on_empty_canvas_hits_nothing() {
    let (mut p, node, ..) = linked_node();
    assert_eq!(
        p.projector.hit(&mut p.layers, Vec2::new(-500.0, -500.0)),
        None,
        "nothing is there"
    );
    // The control: the same projection hits the node inside its body.
    assert_eq!(
        p.projector
            .hit(&mut p.layers, Vec2::new(150.0, 150.0))
            .map(|h| h.node),
        Some(node)
    );
}

#[test]
fn test_projection_a_click_on_a_nodes_empty_interior_hits_the_node() {
    let doc = load_map(json!({"nodes": [node(1, None, json!({
        "position": [0.0, 0.0],
        "min_size": [300.0, 200.0],
        "shape": "ellipse",
        "sections": [{"spans": [{"text": "x"}]}],
        "frame": {"rings": [{"fit": "repeat", "spans": [{"text": "·"}]}]}
    }))]}));
    let node = ident(&doc, 1);
    let mut p = Projected::new(doc);
    let inside = Vec2::new(150.0, 100.0);
    for layer in Layer::ALL {
        assert_eq!(
            p.layers.tree_mut(layer).descendant_at(inside),
            None,
            "no glyph of the {layer:?} layer is under the point: it is the empty interior"
        );
    }
    let hit = p
        .projector
        .hit(&mut p.layers, inside)
        .expect("the node's body holds it");
    assert_eq!(
        (hit.node, hit.element, hit.span, hit.glyph),
        (node, node, None, None)
    );
    // The control: the far corner of the node's rectangle is outside its
    // ellipse, and no glyph is there — nothing is hit.
    let corner = Vec2::new(297.0, 197.0);
    for layer in Layer::ALL {
        assert_eq!(
            p.layers.tree_mut(layer).descendant_at(corner),
            None,
            "no glyph at the corner"
        );
    }
    assert_eq!(
        p.projector.hit(&mut p.layers, corner),
        None,
        "outside the ellipse"
    );
}

#[test]
fn test_projection_a_moved_nodes_body_rewrites_only_its_own_cells() {
    let nodes: Vec<_> = (0..60u32)
        .map(|i| {
            node(
                i + 1,
                None,
                json!({
                    "position": [(i % 10) as f32 * 300.0, (i / 10) as f32 * 300.0],
                    "min_size": [100.0, 100.0]
                }),
            )
        })
        .collect();
    let doc = load_map(json!({ "nodes": nodes }));
    let moved = ident(&doc, 12);
    let mut p = Projected::new(doc);
    let before = p.projector.bodies.cell_writes;
    p.apply(request(
        single(
            DocKind::Node,
            DocMutation::Node(NodeOp::Nudge(Vec2::new(10.0, 0.0))),
        ),
        vec![moved],
        "nudge",
    ))
    .unwrap();
    let writes = p.projector.bodies.cell_writes - before;
    // A 100 × 100 node in one 256-unit cell leaves it and joins it again.
    assert_eq!(writes, 2, "one cell left and one joined, of sixty nodes' cells");
    assert_eq!(
        p.projector
            .hit(&mut p.layers, Vec2::new(300.0 + 105.0, 350.0))
            .map(|h| h.node),
        Some(moved),
        "the body moved with it"
    );
}

/// Whether a glyph of `layer` is drawn at `point`: the layer's BVH finds
/// one, as [`Projector::hit`](crate::projection::Projector::hit) asks it.
fn drawn_at(p: &mut Projected, layer: Layer, point: Vec2) -> bool {
    p.layers.tree_mut(layer).descendant_at(point).is_some()
}

/// A label drawn over a connection is hit before it: the labels layer is
/// above the connections layer. A path's own glyphs stop short of its own
/// label (#340), so the connection under the label is another path's: a
/// straight connector run up and down through the label's center.
#[test]
fn test_projection_a_label_over_a_connection_is_hit_before_it() {
    // Node 2 under node 1, far to its right, so its connector runs level
    // between them and its label sits on it, half-way.
    let nodes = vec![
        node(1, None, json!({"sections": [{"spans": [{"text": "from"}]}]})),
        node(
            2,
            Some(1),
            json!({
                "position": [600.0, 0.0],
                "sections": [{"spans": [{"text": "to"}]}],
                "connector": {"label": {"t": 0.5, "spans": [{"text": "WWWW"}]}}
            }),
        ),
    ];
    let level = Projected::new(load_map(json!({ "nodes": nodes.clone() })));
    let connector = kid(&level.doc, ident(&level.doc, 2), DocKind::Connector);
    let at = level
        .projector
        .label_pos(kid(&level.doc, connector, DocKind::Label))
        .expect("the label is placed");
    // Node 4 under node 3, straight below it through the label's center.
    let mut nodes = nodes;
    nodes.push(node(
        3,
        None,
        json!({"position": [at.x - 5.0, at.y - 300.0], "min_size": [10.0, 10.0]}),
    ));
    nodes.push(node(
        4,
        Some(3),
        json!({
            "position": [at.x - 5.0, at.y + 300.0],
            "min_size": [10.0, 10.0],
            "connector": {"straight": true}
        }),
    ));
    let doc = load_map(json!({ "nodes": nodes }));
    let two = ident(&doc, 2);
    let label = kid(&doc, kid(&doc, two, DocKind::Connector), DocKind::Label);
    let mut p = Projected::new(doc);
    let center = p.projector.label_pos(label).expect("the label is placed");
    // A point both layers draw a glyph at: the label's glyphs lie on the
    // connection's.
    let mut both = None;
    'scan: for dy in -8..=8 {
        for dx in -24..=24 {
            let point = center + Vec2::new(dx as f32, dy as f32);
            if drawn_at(&mut p, Layer::Labels, point) && drawn_at(&mut p, Layer::Connections, point) {
                both = Some(point);
                break 'scan;
            }
        }
    }
    let point = both.expect("a glyph of each layer is under one point near the label");
    let hit = p.projector.hit(&mut p.layers, point).expect("glyphs are there");
    assert_eq!(
        hit.element, label,
        "the labels layer is above the connections layer"
    );
    assert_eq!(hit.node, two, "the label is its connector's node's");
    // The control: asked in draw order — bottom-up — the same point is
    // the connection's, so the order decides the answer.
    let bottom_up = Layer::ALL
        .iter()
        .find(|&&layer| drawn_at(&mut p, layer, point))
        .copied();
    assert_eq!(
        bottom_up,
        Some(Layer::Connections),
        "bottom-up, the connection is found first"
    );
}

/// `node_at` asks the bodies alone: a point on a connector's glyphs, off
/// every body, is a hit but no node; a point in a body is that node,
/// whatever glyph is drawn there (the control: `node_at` is not `None`
/// everywhere).
#[test]
fn test_projection_node_at_reads_the_bodies_and_not_the_glyphs() {
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"sections": [{"spans": [{"text": "from"}]}]})),
        node(2, Some(1), json!({
            "position": [600.0, 0.0],
            "sections": [{"spans": [{"text": "to"}]}]
        })),
    ]}));
    let two = ident(&doc, 2);
    let connector = kid(&doc, two, DocKind::Connector);
    let mut p = Projected::new(doc);
    let rect = p.projector.node_rect(two).expect("the node is drawn");
    let mut on_path = None;
    'scan: for x in 0..600 {
        for y in -8..=8 {
            let point = Vec2::new(x as f32, rect.center().y + y as f32);
            if drawn_at(&mut p, Layer::Connections, point)
                && p.projector.nodes_in(Rect::new(point, Vec2::ZERO)).is_empty()
            {
                on_path = Some(point);
                break 'scan;
            }
        }
    }
    let point = on_path.expect("a connector glyph lies off both bodies");
    let hit = p.projector.hit(&mut p.layers, point).expect("the glyph is hit");
    assert_eq!(hit.element, connector);
    assert_eq!(p.projector.node_at(point), None, "no body holds it");
    assert_eq!(
        p.projector.node_at(rect.center()),
        Some(two),
        "control: a body does"
    );
}

/// What `node_at` is for: a glyph drawn **over** a node's body that is
/// another node's. Node 1 sits inside node 2, and its portal link to node 3
/// sets its end's marker just outside node 1's outline, facing node 3 —
/// over 2's body. A portal end, unlike a path's glyph (#332), is never
/// hidden behind a node. So on the marker the hit is the portal end — the
/// layers come first — and `node_at` is 2, the body under it. A `node_at`
/// written as `hit`'s node would answer 1, the link's owner.
#[test]
fn test_projection_node_at_is_the_body_under_a_portal_end_drawn_over_it() {
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({
            "sections": [{"spans": [{"text": "from"}]}],
            "links": [{"to": 3, "kind": "portal"}]
        })),
        node(2, None, json!({
            "position": [-100.0, -100.0],
            "min_size": [500.0, 250.0],
            "sections": [{"spans": [{"text": "under"}]}]
        })),
        node(3, None, json!({
            "position": [1400.0, 0.0],
            "sections": [{"spans": [{"text": "to"}]}]
        })),
    ]}));
    let (one, two) = (ident(&doc, 1), ident(&doc, 2));
    let mut p = Projected::new(doc);
    let rect = p.projector.node_rect(two).expect("the node is drawn");
    let inner = p.projector.node_rect(one).expect("the node is drawn");
    let mut over = None;
    'scan: for x in (rect.origin.x as i32)..(rect.max().x as i32) {
        for y in (rect.origin.y as i32)..(rect.max().y as i32) {
            let point = Vec2::new(x as f32, y as f32);
            let outside_one = point.x > inner.max().x
                || point.y > inner.max().y
                || point.x < inner.origin.x
                || point.y < inner.origin.y;
            if outside_one
                && p.projector.hit(&mut p.layers, point).is_some_and(|hit| {
                    p.doc
                        .get(hit.element)
                        .is_some_and(|e| e.kind() == DocKind::PortalEnd)
                })
            {
                over = Some(point);
                break 'scan;
            }
        }
    }
    let point = over.expect("precondition: the portal end's marker is drawn inside 2's body, outside 1's");
    assert_eq!(
        p.projector.hit(&mut p.layers, point).map(|h| h.node),
        Some(one),
        "the hit is the link's"
    );
    assert_eq!(p.projector.node_at(point), Some(two), "the body under it is 2");
}

#[test]
fn test_projection_a_click_inside_a_nested_body_hits_the_innermost_node() {
    // Node 2, node 1's child, has its body wholly inside its parent's.
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({
            "position": [0.0, 0.0],
            "min_size": [600.0, 400.0],
            "sections": [{"spans": [{"text": "outer"}]}]
        })),
        node(2, Some(1), json!({
            "position": [200.0, 150.0],
            "min_size": [200.0, 150.0],
            "sections": [{"spans": [{"text": "inner"}]}]
        })),
    ]}));
    let (outer, inner) = (ident(&doc, 1), ident(&doc, 2));
    let mut p = Projected::new(doc);
    let point = Vec2::new(380.0, 280.0);
    for layer in Layer::ALL {
        assert!(
            !drawn_at(&mut p, layer, point),
            "no glyph of the {layer:?} layer is under the point"
        );
    }
    let hit = p.projector.hit(&mut p.layers, point).expect("two bodies hold it");
    assert_eq!(
        (hit.node, hit.element),
        (inner, inner),
        "the innermost body, the smaller"
    );
    // The control: a point inside the outer body only is the outer node's,
    // so both bodies are in the index and the inner one wins by size.
    let hit = p
        .projector
        .hit(&mut p.layers, Vec2::new(580.0, 380.0))
        .expect("the outer body holds it");
    assert_eq!(hit.node, outer, "outside the inner body, the outer node");
}

/// A small deterministic generator for the rubber-band oracle: the same
/// maps and rectangles on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    /// A whole number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A canvas coordinate in `lo..hi`, on a whole unit so that edges can
    /// meet exactly.
    fn coord(&mut self, lo: i64, hi: i64) -> f32 {
        (lo + self.below((hi - lo) as u64) as i64) as f32
    }
}

/// What a rubber band over `rect` must find, worked out with no index:
/// every node the generator made, tested against the rectangle and the
/// shape the generator gave it. Inclusive of edges.
fn covered_by_brute_force(bodies: &[(NodeId, Rect, bool)], rect: Rect) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = bodies
        .iter()
        .filter(|(_, body, ellipse)| {
            let (min, max) = (rect.origin, rect.max());
            let (bmin, bmax) = (body.origin, body.max());
            if body.size.x <= 0.0 || body.size.y <= 0.0 {
                return false;
            }
            if max.x < bmin.x || min.x > bmax.x || max.y < bmin.y || min.y > bmax.y {
                return false;
            }
            if !ellipse {
                return true;
            }
            // The point of the rectangle nearest the ellipse's center,
            // inside the ellipse or on it.
            let c = body.origin + body.size * 0.5;
            let r = body.size * 0.5;
            let q = Vec2::new(c.x.clamp(min.x, max.x), c.y.clamp(min.y, max.y));
            let d = (q - c) / r;
            d.x * d.x + d.y * d.y <= 1.0
        })
        .map(|(id, ..)| *id)
        .collect();
    out.sort_unstable();
    out
}

/// A random map of `count` nodes, some ellipses, some large enough to
/// leave the cell grid, and every node's body as the generator made it.
fn random_bodies(rng: &mut Lcg, count: u32) -> (Projected, Vec<(NodeId, Rect, bool)>) {
    let mut made = Vec::new();
    let nodes: Vec<_> = (1..=count)
        .map(|i| {
            let at = Vec2::new(rng.coord(-3000, 3000), rng.coord(-3000, 3000));
            let size = if rng.below(10) == 0 {
                Vec2::new(rng.coord(2500, 5000), rng.coord(2500, 5000))
            } else {
                Vec2::new(rng.coord(1, 600), rng.coord(1, 600))
            };
            let ellipse = rng.below(3) == 0;
            made.push((i, at, size, ellipse));
            let mut fields = json!({"position": [at.x, at.y], "min_size": [size.x, size.y]});
            if ellipse {
                fields["shape"] = json!("ellipse");
            }
            node(i, None, fields)
        })
        .collect();
    let doc = load_map(json!({ "nodes": nodes }));
    let bodies = made
        .into_iter()
        .map(|(i, at, size, ellipse)| (ident(&doc, i), Rect::new(at, size), ellipse))
        .collect();
    (Projected::new(doc), bodies)
}

/// `Projector::nodes_in` is an index over the bodies and must answer as
/// a scan of every body would: over random maps and random rectangles,
/// rectangles that only touch a body's edge, and zero-area rectangles at
/// a body's corner and inside it.
#[test]
fn test_projection_nodes_in_agrees_with_a_scan_of_every_body() {
    let mut rng = Lcg(0x5eed);
    let mut asked = 0;
    let mut found_any = 0;
    for _ in 0..12 {
        let (p, bodies) = random_bodies(&mut rng, 40);
        for &(id, body, _) in &bodies {
            assert_eq!(
                p.projector.node_rect(id),
                Some(body),
                "the body is the generator's"
            );
        }
        let mut rects = Vec::new();
        for _ in 0..40 {
            let at = Vec2::new(rng.coord(-4000, 4000), rng.coord(-4000, 4000));
            let size = Vec2::new(rng.coord(0, 3000), rng.coord(0, 3000));
            rects.push(Rect::new(at, size));
        }
        for &(_, body, _) in bodies.iter().take(8) {
            // Ending exactly on the body's left edge, starting exactly on
            // its right, and the two zero-area points.
            rects.push(Rect::new(
                body.origin - Vec2::new(50.0, 0.0),
                Vec2::new(50.0, 10.0),
            ));
            rects.push(Rect::new(
                Vec2::new(body.max().x, body.origin.y),
                Vec2::new(40.0, 40.0),
            ));
            rects.push(Rect::new(body.origin, Vec2::ZERO));
            rects.push(Rect::new(body.origin + body.size * 0.5, Vec2::ZERO));
        }
        for rect in rects {
            let want = covered_by_brute_force(&bodies, rect);
            assert_eq!(p.projector.nodes_in(rect), want, "a rubber band over {rect:?}");
            asked += 1;
            found_any += usize::from(!want.is_empty());
        }
    }
    assert!(
        found_any > asked / 4 && found_any < asked,
        "the corpus must hold both hits and misses: {found_any} of {asked}"
    );
}

/// A rubber band visits only the cells it covers and tries only the
/// bodies listed there: over a grid of a hundred far-apart nodes, a band
/// around one tries that one, not the hundred.
#[test]
fn test_projection_nodes_in_visits_only_the_cells_it_covers() {
    let nodes: Vec<_> = (0..100u32)
        .map(|i| {
            node(
                i + 1,
                None,
                json!({
                    "position": [(i % 10) as f32 * 1000.0, (i / 10) as f32 * 1000.0],
                    "min_size": [100.0, 100.0]
                }),
            )
        })
        .collect();
    let doc = load_map(json!({ "nodes": nodes }));
    let target = ident(&doc, 34);
    let p = Projected::new(doc);
    // Node 34 is at (3000, 3000); the band runs from (2990, 2990) to
    // (3120, 3120), which cells 11 and 12 cover each way at 256 units.
    let rect = Rect::new(Vec2::new(2990.0, 2990.0), Vec2::new(130.0, 130.0));
    p.projector.bodies.within_cells.set(0);
    p.projector.bodies.within_tried.set(0);
    assert_eq!(p.projector.nodes_in(rect), vec![target]);
    assert_eq!(p.projector.bodies.within_cells.get(), 4, "the band's four cells");
    assert_eq!(
        p.projector.bodies.within_tried.get(),
        1,
        "the one body listed in them, of a hundred"
    );
}
