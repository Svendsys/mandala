// SPDX-License-Identifier: MPL-2.0

//! The projection's bench bodies (#241, Tests): a full projection of a
//! generated map, an update after a single-node edit, and a rubber band's
//! query (`Projector::nodes_in`). Each is a test too, holding what it
//! measures to its result.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use glam::Vec2;

use crate::document::element::DocKind;
use crate::document::mutation::ops::{DocMutation, NodeOp};
use crate::document::tests::writer_support::{request, single};
use crate::projection::tests::corpus::{generated, of_kind};
use crate::projection::tests::support::{fonts, Projected};
use crate::projection::{GlyphLayers, Layer, Projector};
use crate::util::tests::choices::Choices;

#[test]
fn test_projection_full_projection_of_a_generated_map() {
    do_projection_full_projection_of_a_generated_map();
}

/// A generated map of fifty nodes or more, projected in full: every
/// visible node has its group in every layer.
pub fn do_projection_full_projection_of_a_generated_map() {
    let doc = generated(&mut Choices::seeded(7));
    let mut layers = GlyphLayers::new();
    let projector = Projector::project_full(&doc, &mut layers, &mut fonts());
    assert!(projector.nodes.len() >= 20, "most of the map is visible");
    for layer in Layer::ALL {
        let groups = layers
            .tree(layer)
            .root()
            .descendants(layers.tree(layer).arena())
            .filter(|&g| layers.tree(layer).arena()[g].get().glyph_run().is_none())
            .count();
        assert_eq!(
            groups,
            projector.nodes.len() + 1,
            "{layer:?}: a group per visible node, and the root"
        );
    }
}

#[test]
fn test_projection_update_after_a_single_node_edit() {
    do_projection_update_after_a_single_node_edit();
}

/// One node of a generated map moved, and the projection updated: the
/// update writes something, and the layers equal a full projection of
/// the moved map.
pub fn do_projection_update_after_a_single_node_edit() {
    let doc = generated(&mut Choices::seeded(7));
    let node = of_kind(&doc, DocKind::Node)[0];
    let mut p = Projected::new(doc);
    p.doc
        .apply(
            request(
                single(
                    DocKind::Node,
                    DocMutation::Node(NodeOp::Nudge(Vec2::new(25.0, -10.0))),
                ),
                vec![node],
                "nudge",
            ),
            crate::document::writer::ChangeSource::Action,
        )
        .unwrap();
    let changes = p.sync();
    assert!(!changes.is_empty(), "the move wrote the node's runs again");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_nodes_in_bands_over_a_grid() {
    do_projection_nodes_in_bands_over_a_grid();
}

/// `Projector::nodes_in`, a rubber band's query: a hundred nodes on a
/// ten-by-ten grid, a thousand-unit pitch, and a band dropped over every
/// three-by-three block of them — each band finds exactly its nine
/// nodes, and a band between the rows finds none.
pub fn do_projection_nodes_in_bands_over_a_grid() {
    use crate::flow::Rect;
    use crate::projection::tests::maps::{ident, load_map, node};
    let nodes: Vec<_> = (0..100u32)
        .map(|i| {
            node(
                i + 1,
                None,
                serde_json::json!({
                    "position": [(i % 10) as f32 * 1000.0, (i / 10) as f32 * 1000.0],
                    "min_size": [100.0, 100.0]
                }),
            )
        })
        .collect();
    let doc = load_map(serde_json::json!({ "nodes": nodes }));
    let ids: Vec<_> = (1..=100).map(|i| ident(&doc, i)).collect();
    let p = Projected::new(doc);
    for row in 0..8 {
        for column in 0..8 {
            let band = Rect::new(
                Vec2::new(column as f32 * 1000.0 - 10.0, row as f32 * 1000.0 - 10.0),
                Vec2::new(2120.0, 2120.0),
            );
            let mut want: Vec<_> = (0..3)
                .flat_map(|dy| (0..3).map(move |dx| (row + dy) * 10 + column + dx))
                .map(|k| ids[k])
                .collect();
            want.sort();
            let mut got = p.projector.nodes_in(band);
            got.sort();
            assert_eq!(got, want, "the band at row {row}, column {column}");
        }
    }
    let between = Rect::new(Vec2::new(0.0, 300.0), Vec2::new(9000.0, 400.0));
    assert!(
        p.projector.nodes_in(between).is_empty(),
        "a band between the rows"
    );
}
