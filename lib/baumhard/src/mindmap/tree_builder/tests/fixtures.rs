// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures for the tree_builder tests. Bodies live in the
//! shared `mindmap::test_helpers` module — this file thins over
//! that canonical surface with the per-call-site signatures the
//! tree_builder tests grew used to (parent linkage as second arg,
//! fixed 80×40 size, frame always shown).

use crate::mindmap::model::{MindEdge, MindMap, MindNode};

pub(super) use crate::mindmap::test_helpers::{
    synthetic_portal_edge, testament_map_path as test_map_path,
};

pub(super) fn synthetic_node(id: &str, parent: Option<&str>, x: f64, y: f64) -> MindNode {
    crate::mindmap::test_helpers::synthetic_node_full(id, parent, x, y, 80.0, 40.0, true)
}

pub(super) fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    crate::mindmap::test_helpers::synthetic_map(nodes_vec, edges)
}

/// Builds an N-node linear spine: `n0 -> n1 -> n2 -> ... -> n{N-1}`.
/// Useful for depth-stress tests and O(N²) regression guards.
pub(super) fn mk_chain_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("c0", None, 0.0, 0.0));
    for i in 1..n {
        let parent = format!("c{}", i - 1);
        let id = format!("c{}", i);
        nodes.push(synthetic_node(&id, Some(&parent), 0.0, i as f64 * 50.0));
    }
    synthetic_map(nodes, vec![])
}

/// Builds a star: one root and `n - 1` sibling children.
pub(super) fn mk_star_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("root", None, 0.0, 0.0));
    for i in 1..n {
        let id = format!("s{}", i);
        nodes.push(synthetic_node(
            &id,
            Some("root"),
            (i as f64) * 100.0,
            100.0,
        ));
    }
    synthetic_map(nodes, vec![])
}

pub(super) fn glyph_area_of<'a>(
    tree: &'a crate::gfx_structs::tree::Tree<
        crate::gfx_structs::element::GfxElement,
        crate::gfx_structs::mutator::GfxMutator,
    >,
    node_id: indextree::NodeId,
) -> &'a crate::gfx_structs::area::GlyphArea {
    tree.arena.get(node_id).unwrap().get().glyph_area().unwrap()
}
