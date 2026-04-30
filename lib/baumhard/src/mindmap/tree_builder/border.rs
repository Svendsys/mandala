// SPDX-License-Identifier: MPL-2.0

//! Border-tree builder: emits one per-node Void parent and four
//! `GlyphArea` runs (top, bottom, left, right) per framed node.
//! Sorted lexicographically by node id so the per-node Void
//! channel is stable across rebuilds — the precondition for the
//! in-place mutator path `build_border_mutator_tree_from_nodes`.

use std::collections::HashMap;

use glam::Vec2;
use indextree::NodeId;

use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::Tree;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::border::{build_border_regions, resolve_border_style};
use crate::mindmap::model::MindMap;
use crate::util::color;

/// Per-node data for the border tree — single source of truth
/// consumed by both [`build_border_tree`] (initial build) and
/// [`build_border_mutator_tree`] (in-place §B2 update). The
/// `parent_channel` is the 1-based index of this node in the
/// sorted visible-framed-nodes sequence, so the channel is
/// *stable across rebuilds* as long as the identity sequence
/// (see [`border_identity_sequence`]) is unchanged.
#[derive(Clone, Debug)]
pub struct BorderNodeData {
    pub node_id: String,
    pub parent_channel: usize,
    pub border_style: crate::mindmap::border::BorderStyle,
    pub color_rgba: [f32; 4],
    pub pos_x: f32,
    pub pos_y: f32,
    pub size_x: f32,
    pub size_y: f32,
    /// Zoom window inherited from the owning node. Stamped onto
    /// each of the four border runs at both initial-build and
    /// mutator-update time so the frame disappears atomically
    /// with its node at any zoom level.
    pub zoom_visibility: ZoomVisibility,
    /// Resolved per-cycle-position colours for palette cycling,
    /// or empty when `border_style.color_palette` is unset / the
    /// named palette doesn't exist. Pre-resolved upstream so the
    /// mutator path doesn't need to thread `&MindMap` through
    /// `build_border_mutator_tree_from_nodes`. One entry per
    /// `ColorGroup` in the palette, reading the configured
    /// `palette_field` channel.
    pub palette_cycle: Vec<[f32; 4]>,
}

/// Compute the border layout for the current `(map, offsets)`
/// state. Sorted lexicographically by `MindNode.id` so per-node
/// Void parents always land at the same channel — the
/// prerequisite for the in-place mutator path.
///
/// Skips hidden-by-fold and `show_frame = false` nodes, mirroring
/// the filter in `scene_builder::build_scene`.
pub fn border_node_data(map: &MindMap, offsets: &HashMap<String, (f32, f32)>) -> Vec<BorderNodeData> {
    let vars = &map.canvas.theme_variables;
    let mut sorted_ids: Vec<&String> = map.nodes.keys().collect();
    sorted_ids.sort();

    let mut out: Vec<BorderNodeData> = Vec::new();
    let mut parent_channel: usize = 1;
    for node_id in sorted_ids {
        let Some(node) = map.nodes.get(node_id) else {
            continue;
        };
        if map.is_hidden_by_fold(node) {
            continue;
        }
        if !node.style.show_frame {
            continue;
        }
        // The glyph frame is laid out as four axis-aligned text
        // runs along the node's bounding box, which only makes
        // sense for `NodeShape::Rectangle`. For any other shape we
        // suppress the frame; a curved / shape-aware border is
        // tracked as follow-up work (see CLAUDE.md). Authors still
        // round-trip the `show_frame` flag untouched — we simply
        // don't emit the glyphs.
        //
        // We re-parse `node.style.shape` here rather than reading
        // `area.shape` off the already-built tree because the
        // border tree builder runs from the model, not the node
        // tree — it has no `GlyphArea` in scope. The two parsers
        // are the same (`NodeShape::from_style_string`), so the
        // values agree today; the invariant to preserve if this
        // ever changes is "border pass and node pass resolve the
        // same string to the same `NodeShape`".
        if NodeShape::from_style_string(&node.style.shape) != NodeShape::Rectangle {
            continue;
        }
        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let frame_color_hex = color::resolve_var(&node.style.frame_color, vars);
        // Routes through `resolve_border_style` so per-node
        // GlyphBorderConfig (preset / font / size / color /
        // pattern / palette) drives the rendered output. Without
        // this the data-model fields exist but the renderer
        // ignores them; with it, all three border-build paths
        // (this, scene_builder/node_pass, renderer/scene_buffers)
        // resolve through one shared function.
        let border_style = resolve_border_style(
            node.style.border.as_ref(),
            map.canvas.default_border.as_ref(),
            frame_color_hex,
        );
        let color_rgba = color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
        let palette_cycle =
            crate::mindmap::border::resolve_palette_cycle(&map.palettes, &border_style, color_rgba);

        let pos = node.pos_vec2();
        let size = node.size_vec2();
        out.push(BorderNodeData {
            node_id: node.id.clone(),
            parent_channel,
            border_style,
            color_rgba,
            pos_x: pos.x + ox,
            pos_y: pos.y + oy,
            size_x: size.x,
            size_y: size.y,
            zoom_visibility: node.zoom_window(),
            palette_cycle,
        });
        parent_channel += 1;
    }
    out
}

/// Identity sequence for a slice of [`BorderNodeData`] — the
/// sorted sequence of `node_id`s in tree-insertion order. Two
/// sequences match iff the same set of nodes is framed in the
/// same order. Drag, text-edit, color-preview, and preset-swap
/// all leave this stable (preset swaps change the character
/// content of each run but not the tree shape — the mutator's
/// `Text::Assign` picks up the new glyphs); adding or removing a
/// framed node, toggling `show_frame`, or folding an ancestor
/// drops the equality and forces a full rebuild via the
/// dispatcher in `update_border_tree_static`.
pub fn border_identity_sequence(nodes: &[BorderNodeData]) -> Vec<String> {
    nodes.iter().map(|n| n.node_id.clone()).collect()
}

/// Build the border tree from the given `MindMap` + drag offsets.
/// Convenience wrapper that calls [`border_node_data`] then
/// [`build_border_tree_from_nodes`].
///
/// Tree shape:
///
/// ```text
/// Void (root)
/// ├── Void (per node — channel = 1-based sorted index)
/// │   ├── GlyphArea (top run, channel = 1)
/// │   ├── GlyphArea (bottom run, channel = 2)
/// │   ├── GlyphArea (left column, channel = 3)
/// │   └── GlyphArea (right column, channel = 4)
/// ├── Void (next node)
/// │   └── ...
/// ```
///
/// Iteration order is the lexicographic order of `MindNode.id` —
/// stable across runs, so per-node Void parents always land at
/// the same channel. Without this, `MindMap.nodes` (a `HashMap`)
/// would yield nondeterministic order and make the in-place
/// mutator path unreliable.
///
/// # Costs
///
/// O(N log N) where N is the visible framed-node count (the sort
/// dominates for large maps). Allocates one tree arena plus one
/// `String` per run. Uses the same `BorderStyle` defaults as
/// `scene_builder::build_scene` so the two paths can't drift on
/// style choices.
pub fn build_border_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Tree<GfxElement, GfxMutator> {
    build_border_tree_from_nodes(&border_node_data(map, offsets))
}

/// Variant of [`build_border_tree`] that consumes pre-computed
/// node data. Use this in the dispatch path that already called
/// [`border_node_data`] to derive the identity sequence — saves
/// one walk over `MindMap.nodes`.
pub fn build_border_tree_from_nodes(nodes: &[BorderNodeData]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for node in nodes {
        append_border_sub_tree(&mut tree, node, &mut unique_id);
    }
    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered border tree to the current
/// `(map, offsets)` state without rebuilding the arena. Pairs
/// with [`build_border_tree`] — both consume
/// [`border_node_data`], so applying this mutator to a tree built
/// from a node slice with the same
/// [`border_identity_sequence`] updates each run's variable
/// fields in place.
///
/// The hot-path case this closes: when the color picker is open,
/// every throttled `AboutToWait` drain re-runs the scene build,
/// which previously re-allocated the entire border tree every
/// frame. With this dispatch, picker hover leaves the border
/// tree's arena untouched and only overwrites text / position /
/// color fields on the existing GlyphAreas.
pub fn build_border_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    build_border_mutator_tree_from_nodes(&border_node_data(map, offsets))
}

/// Variant of [`build_border_mutator_tree`] that consumes
/// pre-computed node data. Use this in the dispatch path that
/// already called [`border_node_data`].
pub fn build_border_mutator_tree_from_nodes(
    nodes: &[BorderNodeData],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for node in nodes {
        let parent_node = mt.arena.new_node(GfxMutator::new_void(node.parent_channel));
        mt.root.append(parent_node, &mut mt.arena);

        let specs = crate::mindmap::border::border_run_specs(
            &node.border_style,
            (node.pos_x, node.pos_y),
            (node.size_x, node.size_y),
        );
        for spec in specs {
            let regions = build_border_regions(
                spec.cluster_count,
                &node.palette_cycle,
                node.color_rgba,
                spec.palette_offset,
            );
            let mut area = GlyphArea::new_with_str(
                &spec.text,
                spec.font_size_pt,
                spec.font_size_pt,
                Vec2::new(spec.position.0, spec.position.1),
                Vec2::new(spec.bounds.0, spec.bounds.1),
            );
            area.regions = regions;
            area.zoom_visibility = node.zoom_visibility;
            let delta = DeltaGlyphArea::full_assign_from(&area);
            let leaf = mt.arena.new_node(GfxMutator::new(
                Mutation::AreaDelta(Box::new(delta)),
                spec.channel,
            ));
            parent_node.append(leaf, &mut mt.arena);
        }
    }
    mt
}

/// Build one per-node sub-tree (Void parent + 4 GlyphArea runs) and
/// append it under `tree.root`. Kept as a private helper so
/// `build_border_tree` stays readable. `BorderNodeData.parent_channel`
/// is the stable 1-based sorted-index channel — see
/// [`BorderNodeData::parent_channel`].
fn append_border_sub_tree(
    tree: &mut Tree<GfxElement, GfxMutator>,
    node: &BorderNodeData,
    unique_id: &mut usize,
) {
    let specs = crate::mindmap::border::border_run_specs(
        &node.border_style,
        (node.pos_x, node.pos_y),
        (node.size_x, node.size_y),
    );

    // Per-node Void parent — groups the four runs for targeted
    // mutation. The parent's channel is the stable sorted-index
    // value so distinct nodes never collide across rebuilds.
    let parent_id = tree
        .arena
        .new_node(GfxElement::new_void_with_id(node.parent_channel, *unique_id));
    tree.root.append(parent_id, &mut tree.arena);
    *unique_id += 1;

    // Stable channels 1..=4 inside each border sub-tree. The
    // per-node Void parent already disambiguates across nodes.
    // Palette offsets sweep top → right → bottom → left around
    // the rectangle so a colour cycle wraps cleanly. See
    // `BorderRunSpec` for the spec contract.
    for spec in &specs {
        append_border_run(
            tree,
            parent_id,
            spec.channel,
            *unique_id,
            &spec.text,
            spec.font_size_pt,
            spec.position,
            spec.bounds,
            node.color_rgba,
            node.zoom_visibility,
            &node.palette_cycle,
            spec.palette_offset,
            spec.cluster_count,
        );
        *unique_id += 1;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_border_run(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    channel: usize,
    unique_id: usize,
    text: &str,
    font_size: f32,
    position: (f32, f32),
    bounds: (f32, f32),
    color_rgba: [f32; 4],
    zoom_visibility: ZoomVisibility,
    palette_cycle: &[[f32; 4]],
    palette_offset: usize,
    cluster_count: usize,
) {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        font_size,
        Vec2::new(position.0, position.1),
        Vec2::new(bounds.0, bounds.1),
    );
    area.zoom_visibility = zoom_visibility;

    // Per-cluster ColorFontRegions when the user opts into palette
    // cycling, otherwise a single uniform region (matches the
    // pre-pattern path's cost). `cluster_count` is pre-computed by
    // `border_run_specs` so this body never re-walks the string.
    area.regions = build_border_regions(cluster_count, palette_cycle, color_rgba, palette_offset);

    let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
    let node = tree.arena.new_node(element);
    parent_id.append(node, &mut tree.arena);
}
