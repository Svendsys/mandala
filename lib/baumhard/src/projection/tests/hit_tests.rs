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

#[test]
fn test_projection_a_label_over_its_connection_is_hit_before_it() {
    // Node 2 under node 1, far to its right, so its connector runs level
    // between them and its label sits on it, half-way.
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"sections": [{"spans": [{"text": "from"}]}]})),
        node(2, Some(1), json!({
            "position": [600.0, 0.0],
            "sections": [{"spans": [{"text": "to"}]}],
            "connector": {"label": {"t": 0.5, "spans": [{"text": "WWWW"}]}}
        })),
    ]}));
    let two = ident(&doc, 2);
    let connector = kid(&doc, two, DocKind::Connector);
    let label = kid(&doc, connector, DocKind::Label);
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
    assert_eq!(hit.element, label, "the labels layer is above the connections layer");
    assert_eq!(hit.node, two, "the label is its connector's node's");
    // The control: asked in draw order — bottom-up — the same point is
    // the connection's, so the order decides the answer.
    let bottom_up = Layer::ALL
        .iter()
        .find(|&&layer| drawn_at(&mut p, layer, point))
        .copied();
    assert_eq!(bottom_up, Some(Layer::Connections), "bottom-up, the connection is found first");
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
        assert!(!drawn_at(&mut p, layer, point), "no glyph of the {layer:?} layer is under the point");
    }
    let hit = p.projector.hit(&mut p.layers, point).expect("two bodies hold it");
    assert_eq!(
        (hit.node, hit.element),
        (inner, inner),
        "the innermost body, the smaller"
    );
    // The control: a point inside the outer body only is the outer node's,
    // so both bodies are in the index and the inner one wins by size.
    let hit = p.projector.hit(&mut p.layers, Vec2::new(580.0, 380.0)).expect("the outer body holds it");
    assert_eq!(hit.node, outer, "outside the inner body, the outer node");
}
