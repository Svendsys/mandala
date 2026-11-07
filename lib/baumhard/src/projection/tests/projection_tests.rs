// SPDX-License-Identifier: MPL-2.0

//! The projection's two bench bodies (#241, Tests): a full projection of
//! a generated map, and an update after a single-node edit. Each is a
//! test too, holding what it measures to its result.
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
