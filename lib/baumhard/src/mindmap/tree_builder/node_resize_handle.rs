// SPDX-License-Identifier: MPL-2.0

//! Node resize-handle tree builder. Sibling of
//! [`super::section_resize_handle`] — same role pattern, different
//! domain. One `GlyphArea` per visible handle (8 per selected
//! node); the channel is derived from the side so the identity
//! sequence is stable across drags that preserve the handle set.

use glam::Vec2;

use crate::core::primitives::ColorFontRegions;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::util::color;

/// Identity sequence for a set of `NodeResizeHandleElement`s — the
/// side-derived channel of each handle, in tree-insertion order.
/// Two handle sets produce the same identity iff their structural
/// shape is identical. Selection-gated 0 ↔ 8 transitions drop the
/// equality and force a full rebuild; a steady drag stays on the
/// in-place mutator path.
pub fn node_resize_handle_identity_sequence(
    elements: &[crate::mindmap::scene_builder::NodeResizeHandleElement],
) -> Vec<usize> {
    elements.iter().map(|e| e.side.channel()).collect()
}

/// Lay out one resize handle as the `(channel, GlyphArea)` pair
/// both the initial-build and in-place mutator paths emit. Single
/// source of truth.
fn node_resize_handle_layout(
    elem: &crate::mindmap::scene_builder::NodeResizeHandleElement,
) -> (usize, GlyphArea) {
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.0, 0.9, 1.0, 1.0]);
    let half_w = elem.font_size_pt * 0.3;
    let half_h = elem.font_size_pt * 0.5;
    let pos = Vec2::new(elem.position.0 - half_w, elem.position.1 - half_h);
    let bounds = Vec2::new(elem.font_size_pt, elem.font_size_pt);

    let mut area = GlyphArea::new_with_str(&elem.glyph, elem.font_size_pt, elem.font_size_pt, pos, bounds);
    let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(&elem.glyph);
    area.regions = ColorFontRegions::single_span(cluster_count, Some(color_rgba), None);

    (elem.side.channel(), area)
}

/// Build a baumhard tree of every node-resize-handle glyph from a
/// pre-computed slice. Handles only exist while a node is selected
/// (and finite + positive size), so this tree is typically empty
/// or has 8 leaves.
pub fn build_node_resize_handle_tree(
    elements: &[crate::mindmap::scene_builder::NodeResizeHandleElement],
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for elem in elements {
        let (channel, area) = node_resize_handle_layout(elem);
        let element_node = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
        unique_id += 1;
        let leaf = tree.arena.new_node(element_node);
        tree.root.append(leaf, &mut tree.arena);
    }

    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered node-resize-handle tree to
/// the current `elements` state without rebuilding the arena.
pub fn build_node_resize_handle_mutator_tree(
    elements: &[crate::mindmap::scene_builder::NodeResizeHandleElement],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for elem in elements {
        let (channel, area) = node_resize_handle_layout(elem);
        let delta = DeltaGlyphArea::full_assign_from(&area);
        let leaf = mt
            .arena
            .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel));
        mt.root.append(leaf, &mut mt.arena);
    }
    mt
}
