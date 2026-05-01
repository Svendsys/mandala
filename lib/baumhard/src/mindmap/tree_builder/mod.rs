// SPDX-License-Identifier: MPL-2.0

//! Mindmap tree builder — projects a `MindMap` into a Baumhard
//! `Tree<GfxElement, GfxMutator>` and exposes per-canvas-role
//! builders (borders, portals, connections, connection-labels,
//! edge-handles) that the app crate's scene rebuilders consume.
//! `MindMapTree` and `build_mindmap_tree` live here; per-role
//! builders are re-exported from the sibling files.

use std::collections::HashMap;

use indextree::NodeId;

use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::MindMap;

mod border;
mod connection;
mod connection_label;
mod edge_handle;
mod node;
mod portal;

#[cfg(test)]
mod tests;

pub use border::{
    border_identity_sequence, border_node_data, build_border_mutator_tree,
    build_border_mutator_tree_from_nodes, build_border_tree, build_border_tree_from_nodes, BorderNodeData,
};
pub use connection::{
    build_connection_mutator_tree, build_connection_tree, connection_identity_sequence,
    ConnectionEdgeIdentity,
};
pub use connection_label::{
    build_connection_label_mutator_tree, build_connection_label_tree, connection_label_identity_sequence,
    ConnectionLabelMutator, ConnectionLabelTree,
};
pub use edge_handle::{
    build_edge_handle_mutator_tree, build_edge_handle_tree, edge_handle_channel_for,
    edge_handle_identity_sequence,
};
pub use portal::{
    build_portal_mutator_tree, build_portal_mutator_tree_from_pairs, build_portal_tree,
    build_portal_tree_from_pairs, portal_identity_sequence, portal_pair_data, PortalColorPreviewRef,
    PortalIdentity, PortalMutator, PortalPairData, PortalTree, SelectedEdgeRef,
};

use node::{append_node_sections, build_children_recursive, mindnode_container_area};

/// Result of building a Baumhard tree from a MindMap. The tree
/// mirrors the MindMap's parent-child hierarchy. Each MindNode
/// produces a three-deep subtree:
///
/// - one **container** `GlyphArea` (chrome only — background, frame
///   padding, shape, zoom window),
/// - one **section-area** `GlyphArea` per [`MindSection`], carrying
///   the section's text + theme-resolved regions (these are the
///   buffers the renderer's tree walker shapes),
/// - one **section-model** `GlyphModel` child per section-area as a
///   structural seam for future per-component / per-grapheme
///   mutations.
///
/// [`MindSection`]: crate::mindmap::model::MindSection
pub struct MindMapTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Maps MindNode ID → arena `NodeId` of the *container* area.
    /// Hit-tests that resolve to a section element climb to the
    /// container's arena ancestor before consulting this map.
    pub node_map: HashMap<String, NodeId>,
    /// Maps `(MindNode ID, section index)` → arena `NodeId` of
    /// the section-area. Empty for nodes whose sections were
    /// excluded by fold (the same fold path that excludes a whole
    /// node from `node_map`). Section-models are reachable as the
    /// only child of the section-area inside the arena, so no
    /// separate map is needed for them.
    pub section_map: HashMap<(String, usize), NodeId>,
    /// Reverse map: arena `NodeId` → MindNode ID. Populated for
    /// container areas only — section-areas live in
    /// `section_map`'s values, not here, so a hit on a section
    /// element returns `None` from [`Self::mind_id_for_node`]
    /// (callers walk one level up via the arena to find the
    /// container).
    ///
    /// Private to preserve forward-compatible API (§B10) — callers
    /// use [`MindMapTree::mind_id_for_node`] /
    /// [`MindMapTree::section_for_node`] instead.
    reverse_node_map: HashMap<NodeId, String>,
    /// Reverse map for sections: arena `NodeId` of a section-area
    /// → `(MindNode ID, section index)`. Populated alongside
    /// `section_map` during tree construction so hit tests that
    /// land on a section can recover the (node_id, section_idx)
    /// pair in O(1) without an arena climb.
    reverse_section_map: HashMap<NodeId, (String, usize)>,
}

/// Builds a `Tree<GfxElement, GfxMutator>` from a MindMap's
/// hierarchy.
///
/// The tree structure mirrors the MindMap's parent-child
/// relationships:
/// - A Void root node at the top
/// - Each root MindNode (parent_id is None) as a child of the
///   Void root
/// - Children nested recursively following parent_id
/// - Nodes hidden by fold state are excluded
///
/// Each MindNode produces three layers:
/// - one *container* `GlyphArea` (chrome only),
/// - one *section-area* `GlyphArea` per
///   [`MindSection`](crate::mindmap::model::MindSection) as a
///   sibling of any child mind-node-areas; sections carry the
///   text + regions the renderer shapes,
/// - one structural *section-model* `GlyphModel` child per
///   section-area (a future per-component-mutation seam — the
///   renderer skips it).
pub fn build_mindmap_tree(map: &MindMap) -> MindMapTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut node_map: HashMap<String, NodeId> = HashMap::new();
    let mut section_map: HashMap<(String, usize), NodeId> = HashMap::new();
    let mut id_counter: usize = 1; // 0 is reserved for the Void root

    let vars = &map.canvas.theme_variables;
    let canvas_default_border = map.canvas.default_border.as_ref();
    let roots = map.root_nodes();
    for root in &roots {
        if map.is_hidden_by_fold(root) {
            continue;
        }
        let area = mindnode_container_area(root, vars, canvas_default_border);
        let element = GfxElement::new_area_non_indexed_with_id(area, root.channel, id_counter);
        id_counter += 1;

        let node_id = tree.arena.new_node(element);
        tree.root.append(node_id, &mut tree.arena);
        node_map.insert(root.id.clone(), node_id);

        append_node_sections(root, node_id, vars, &mut tree, &mut section_map, &mut id_counter);

        build_children_recursive(
            map,
            &root.id,
            node_id,
            &mut tree,
            &mut node_map,
            &mut section_map,
            &mut id_counter,
        );
    }

    let reverse_node_map: HashMap<NodeId, String> = node_map
        .iter()
        .map(|(mind_id, &arena_id)| (arena_id, mind_id.clone()))
        .collect();
    let reverse_section_map: HashMap<NodeId, (String, usize)> = section_map
        .iter()
        .map(|((mind_id, idx), &arena_id)| (arena_id, (mind_id.clone(), *idx)))
        .collect();
    MindMapTree {
        tree,
        node_map,
        section_map,
        reverse_node_map,
        reverse_section_map,
    }
}

impl MindMapTree {
    /// Look up the MindMap node ID for a *container* arena
    /// `NodeId`. Returns `None` for void roots, removed nodes,
    /// section-areas, and section-models — those are not
    /// node-containers. Use [`Self::section_for_node`] to resolve
    /// section-area arena ids; both maps together cover every
    /// hit-test target an interactive path can land on.
    ///
    /// O(1) hash lookup, no allocation.
    pub fn mind_id_for_node(&self, node_id: NodeId) -> Option<&str> {
        self.reverse_node_map.get(&node_id).map(|s| s.as_str())
    }

    /// Look up the `(MindNode ID, section index)` pair for an
    /// arena `NodeId`, returning `None` when the id is anything
    /// other than a section-area (containers, section-models,
    /// the void root, removed nodes).
    ///
    /// O(1) hash lookup, no allocation.
    pub fn section_for_node(&self, node_id: NodeId) -> Option<(&str, usize)> {
        self.reverse_section_map
            .get(&node_id)
            .map(|(mind_id, idx)| (mind_id.as_str(), *idx))
    }

    /// Resolve a hit-tested arena `NodeId` to the owning MindNode
    /// id. Whether the user landed on the container, a section-
    /// area, or a section-model, the caller almost always wants
    /// the MindNode id; this helper consolidates the climb so
    /// every hit-test site doesn't reimplement the dispatch.
    ///
    /// Climbs at most three arena edges (model → section →
    /// container) — O(1) in practice.
    pub fn owning_mind_id(&self, mut node_id: NodeId) -> Option<&str> {
        for _ in 0..3 {
            if let Some(id) = self.mind_id_for_node(node_id) {
                return Some(id);
            }
            node_id = self.tree.arena.get(node_id)?.parent()?;
        }
        None
    }
}
