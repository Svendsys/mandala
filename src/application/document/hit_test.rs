// SPDX-License-Identifier: MPL-2.0

//! Hit-test / rect-select / drag / highlight helpers. None live on
//! `MindMapDocument` — they all take a `MindMapTree` / `MindMap` +
//! screen coordinates and return values, so unit tests don't need a
//! GPU or an event loop.
//!
//! `hit_test` takes `&mut MindMapTree` because the BVH descent may
//! trigger a lazy subtree-AABB recomputation on the first call after
//! a mutation. All other helpers remain read-only.

use glam::Vec2;

use baumhard::core::primitives::{Flag, Flaggable, Range};
use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::{GfxMutator, Mutation};
use baumhard::gfx_structs::tree::MutatorTree;
use baumhard::gfx_structs::tree_walker::walk_tree_from;
use baumhard::mindmap::connection;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_builder::{build_section_resize_handles, ResizeHandleSide};
use baumhard::mindmap::tree_builder::MindMapTree;

use super::types::EdgeRef;

/// Find the mindmap node ID under `canvas_pos` using BVH-accelerated
/// tree descent. Returns the innermost (smallest-area) hit, or `None`.
///
/// # Costs
///
/// O(branching_factor × depth) when subtrees are spatially disjoint;
/// O(n) worst case. One `Vec` allocation on the first call after a
/// mutation (subtree AABB recomputation); O(1) on subsequent calls.
pub fn hit_test(canvas_pos: Vec2, tree: &mut MindMapTree) -> Option<String> {
    // Sections render as `GlyphArea` children of the owning node's
    // container, so the BVH descent can land on a section-area
    // (or a section-model) rather than the container itself. Climb
    // to the owning MindNode container, then re-apply the
    // container's shape filter — a click on the rectangular AABB
    // of a section inside an ellipse-shaped node must not register
    // as a hit on that node, just like the pre-section behaviour
    // where the node area's shape was the only one consulted.
    let landed = tree.tree.descendant_at(canvas_pos)?;
    let mind_id = tree.owning_mind_id(landed)?.to_owned();
    if !point_in_node_aabb(canvas_pos, &mind_id, tree) {
        return None;
    }
    Some(mind_id)
}

/// Richer hit-test result that distinguishes a click on the node
/// chrome from a click on a specific section. Used by the click
/// handler to opt into `SelectionState::Section` when a click
/// lands inside an authored section AABB; today's typical
/// migrated single-section node treats every click as a click on
/// the chrome (Section selection bites only when the node has
/// multiple sections, so unambiguous "the user pointed at this
/// section" gestures don't appear on legacy maps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    /// Click landed on the node chrome — the container area or a
    /// section-area inside a single-section node. The
    /// SelectionState consumer turns this into `Single(node_id)`,
    /// preserving today's whole-node selection semantics.
    NodeContainer { node_id: String },
    /// Click landed on a specific section-area inside a multi-
    /// section node. The SelectionState consumer turns this into
    /// `Section { node_id, section_idx }` so per-section verbs
    /// (text edit, font, color) can operate on just this section.
    Section { node_id: String, section_idx: usize },
}

impl HitTarget {
    /// Owning node id — the same value every per-node consumer
    /// (highlight, drag, …) wants regardless of whether the user
    /// pointed at the chrome or a specific section.
    pub fn node_id(&self) -> &str {
        match self {
            HitTarget::NodeContainer { node_id } => node_id,
            HitTarget::Section { node_id, .. } => node_id,
        }
    }
}

/// Hit-test variant that returns a [`HitTarget`] — distinguishes
/// "the user clicked on a specific section" from "the user
/// clicked on the chrome of a node that happens to have one
/// section filling it". The plain [`hit_test`] keeps the old
/// `Option<String>` shape; this one is the click-handler entry
/// point that wires `SelectionState::Section` for multi-section
/// authoring.
pub fn hit_test_target(canvas_pos: Vec2, tree: &mut MindMapTree) -> Option<HitTarget> {
    let landed = tree.tree.descendant_at(canvas_pos)?;
    // Single-walk climb: at each ancestor, check first for a
    // section identity (returns `Some` when the arena id keys
    // into `section_map`'s reverse-lookup), then for a
    // mind-node-container identity. Pre-fix, this function
    // climbed twice — once via `owning_mind_id` to find the
    // container, then again from `landed` to find the section.
    // Folding to one walk halves the cost on the click hot path
    // and keeps both lookups O(climb depth).
    let mut probe = landed;
    let mut hit_section: Option<(String, usize)> = None;
    let mut mind_id: Option<String> = None;
    loop {
        if hit_section.is_none() {
            if let Some((id, idx)) = tree.section_for_node(probe) {
                hit_section = Some((id.to_string(), idx));
            }
        }
        if let Some(id) = tree.mind_id_for_node(probe) {
            mind_id = Some(id.to_string());
            break;
        }
        match tree.tree.arena.get(probe).and_then(|n| n.parent()) {
            Some(p) => probe = p,
            None => break,
        }
    }
    let mind_id = mind_id?;
    if !point_in_node_aabb(canvas_pos, &mind_id, tree) {
        return None;
    }
    // Single-section nodes fold to NodeContainer so today's
    // whole-node click semantics survive on every migrated map.
    // The Section variant unlocks for nodes whose author gave
    // them more than one section. `section_count_for` is an O(1)
    // map lookup populated at tree-build time — no per-click
    // arena walk.
    if let Some((id, idx)) = hit_section {
        if tree.section_count_for(&id) > 1 {
            return Some(HitTarget::Section {
                node_id: id,
                section_idx: idx,
            });
        }
    }
    Some(HitTarget::NodeContainer { node_id: mind_id })
}

/// Is `canvas_pos` inside node `node_id`'s shape? Reads the tree-side
/// glyph area so drag-preview positions count (tree is authoritative
/// during in-flight mutations; identical to the model when idle). The
/// check is shape-aware *inside the container's own AABB*: rectangular
/// nodes use AABB containment, and non-rectangular shapes (e.g.
/// ellipse) delegate to `NodeShape::contains_local`, matching the BVH
/// hit-test path.
///
/// Unlike `hit_test`, this answers a point-in-specific-node question —
/// a click over a child of `node_id`, or over a section that overflows
/// the container's AABB, still counts as "inside" `node_id`. The
/// text editor's click-outside-commit gesture relies on this: a click
/// on a multi-section node's overflowing second section must not
/// commit-and-close the editor on the first section.
///
/// Overflow path uses the cached subtree AABB (populated by
/// `Tree::ensure_subtree_aabbs`, which the tree walker and
/// `descendant_near` keep fresh in normal use). When the cache is
/// dirty the function falls back to the container-only check —
/// matches pre-section behaviour for regressions in the cache path.
pub fn point_in_node_aabb(canvas_pos: Vec2, node_id: &str, tree: &MindMapTree) -> bool {
    let Some(arena_id) = tree.arena_id_for(node_id) else {
        return false;
    };
    let Some(arena_node) = tree.tree.arena.get(arena_id) else {
        return false;
    };
    let element = arena_node.get();
    let Some(area) = element.glyph_area() else {
        return false;
    };

    let x = area.position.x.0;
    let y = area.position.y.0;
    let w = area.render_bounds.x.0;
    let h = area.render_bounds.y.0;

    let in_container_aabb =
        canvas_pos.x >= x && canvas_pos.x <= x + w && canvas_pos.y >= y && canvas_pos.y <= y + h;

    if in_container_aabb {
        let local = Vec2::new(canvas_pos.x - x, canvas_pos.y - y);
        return area.shape.contains_local(local, Vec2::new(w, h));
    }

    if let Some((tl, br)) = element.subtree_aabb() {
        return canvas_pos.x >= tl.x && canvas_pos.x <= br.x && canvas_pos.y >= tl.y && canvas_pos.y <= br.y;
    }
    false
}

/// Hit test edges: find the nearest visible edge within `tolerance` canvas
/// units of `canvas_pos`. Returns an `EdgeRef` for the closest edge, or
/// `None` if nothing is within range.
///
/// Visibility filter mirrors `scene_builder::build_scene_with_offsets` — an
/// edge is eligible only if `edge.visible` is true, both endpoint nodes
/// exist, and neither endpoint is hidden by fold state.
pub fn hit_test_edge(canvas_pos: Vec2, map: &MindMap, tolerance: f32) -> Option<EdgeRef> {
    let mut best: Option<(EdgeRef, f32)> = None;
    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }

        let from_pos = from_node.pos_vec2();
        let from_size = from_node.size_vec2();
        let to_pos = to_node.pos_vec2();
        let to_size = to_node.size_vec2();

        let path = connection::build_connection_path(
            from_pos,
            from_size,
            &edge.anchor_from,
            to_pos,
            to_size,
            &edge.anchor_to,
            &edge.control_points,
        );
        let dist = connection::distance_to_path(canvas_pos, &path);
        if dist > tolerance {
            continue;
        }
        if best.as_ref().map_or(true, |(_, best_dist)| dist < *best_dist) {
            best = Some((EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type), dist));
        }
    }
    best.map(|(e, _)| e)
}

/// Find all node IDs whose shape intersects the given canvas-space rectangle.
/// The rectangle is defined by two opposite corners (min and max are computed
/// internally). Shape-aware: rectangles fall through to an AABB overlap,
/// ellipses use `NodeShape::intersects_local_aabb` so the corners of a
/// node's bounding box (outside the ellipse) don't trigger a false lasso hit.
pub fn rect_select(corner_a: Vec2, corner_b: Vec2, tree: &MindMapTree) -> Vec<String> {
    let min_x = corner_a.x.min(corner_b.x);
    let min_y = corner_a.y.min(corner_b.y);
    let max_x = corner_a.x.max(corner_b.x);
    let max_y = corner_a.y.max(corner_b.y);

    let mut hits = Vec::new();
    for (mind_id, node_id) in tree.node_ids() {
        let area = match tree.tree.arena.get(node_id).and_then(|n| n.get().glyph_area()) {
            Some(a) => a,
            None => continue,
        };
        let x = area.position.x.0;
        let y = area.position.y.0;
        let w = area.render_bounds.x.0;
        let h = area.render_bounds.y.0;

        // Translate the selection rectangle into the node's local
        // frame and let the shape decide. Rectangle keeps the old
        // pure-AABB behaviour; non-rect shapes refine.
        let local_min = Vec2::new(min_x - x, min_y - y);
        let local_max = Vec2::new(max_x - x, max_y - y);
        if area
            .shape
            .intersects_local_aabb(local_min, local_max, Vec2::new(w, h))
        {
            hits.push(mind_id.to_string());
        }
    }
    hits
}

/// Apply a set of node highlights as baumhard mutations. For each
/// `(mind_node_id, color)` pair, the node's existing text-run ranges
/// are collected from its `GlyphArea` and a `GfxMutator::Macro` of one
/// `SetRegionColor(range, color)` mutation per range is applied through
/// `walk_tree_from` — i.e. the highlight is expressed in the same
/// mutation language as the rest of baumhard's tree-walker flow rather
/// than reaching into the arena imperatively.
///
/// Later pairs override earlier ones when the same node appears twice,
/// which is what the reparent/connect modes rely on: callers pass
/// selection highlights first (cyan), then source (orange), then target
/// (green), and the last write wins on conflicts.
///
/// Architectural note: this replaces an earlier trio of per-purpose
/// highlight helpers that each did the same direct arena patching
/// with different constants. The single function here is both shorter
/// and aligns with the mutation-first interaction model — highlights
/// flow through the same mutator vocabulary as every other tree
/// edit.
pub fn apply_tree_highlights<'a, I>(tree: &mut MindMapTree, highlights: I)
where
    I: IntoIterator<Item = (&'a str, Option<usize>, [f32; 4])>,
{
    for (mind_id, only_section_idx, color) in highlights {
        let Some(node_id) = tree.arena_id_for(mind_id) else { continue };

        // Sections live as `GlyphArea` children of the node
        // container, and that's where the text-runs (and therefore
        // `ColorFontRegions`) actually live post-refactor. Iterate
        // every immediate child marked `Flag::SectionRoot` and
        // apply the highlight macro to each — unless the caller
        // restricted the highlight to a specific section index
        // (Section selection), in which case only that section's
        // runs paint cyan.
        //
        // §B7 borrow-split note: collecting into a `Vec<NodeId>`
        // here is the smallest correct shape, not laziness.
        // `node_id.children(&tree.tree.arena)` borrows the arena
        // immutably; the loop body needs `&mut tree.tree` to call
        // `walk_tree_from`. Holding the iterator across the mutable
        // borrow won't compile, so the fix is to materialise the
        // section ids first and drop the immutable borrow before
        // the mutation pass starts. The vec is `O(sections per
        // node)` — bounded by user authoring (typically 1–4 per
        // node), and per-mind-id-per-call, so allocation cost is
        // negligible relative to the highlight macro itself.
        let section_ids: Vec<indextree::NodeId> = node_id
            .children(&tree.tree.arena)
            .filter(|cid| {
                tree.tree
                    .arena
                    .get(*cid)
                    .map(|n| n.get().flag_is_set(Flag::SectionRoot))
                    .unwrap_or(false)
            })
            .collect();

        // When a Section selection is active, filter the section
        // list to that one element so visual feedback matches what
        // the user pointed at.
        let section_ids: Vec<indextree::NodeId> = match only_section_idx {
            Some(target_idx) => section_ids
                .into_iter()
                .filter(|sid| {
                    tree.section_for_node(*sid)
                        .map(|(_, idx)| idx == target_idx)
                        .unwrap_or(false)
                })
                .collect(),
            None => section_ids,
        };

        for section_node_id in section_ids {
            let (ranges, target_channel): (Vec<Range>, usize) = {
                let Some(node) = tree.tree.arena.get(section_node_id) else { continue };
                let element = node.get();
                let Some(area) = element.glyph_area() else { continue };
                let ranges = area.regions.all_regions().iter().map(|r| r.range).collect();
                let channel = {
                    use baumhard::gfx_structs::tree::BranchChannel;
                    element.channel()
                };
                (ranges, channel)
            };
            if ranges.is_empty() {
                continue;
            }

            let mutations: Vec<Mutation> = ranges
                .into_iter()
                .map(|r| Mutation::area_command(GlyphAreaCommand::SetRegionColor(r, color)))
                .collect();
            let mutator_tree = MutatorTree::new_with(GfxMutator::new_macro(mutations, target_channel));
            walk_tree_from(&mut tree.tree, &mutator_tree, section_node_id, mutator_tree.root);
        }
    }
}

/// Apply a position delta directly to nodes in the Baumhard tree (in-place mutation).
/// Used during drag for fast visual preview without rebuilding from the MindMap model.
///
/// `include_descendants` toggles whether *child mind-nodes* track the
/// drag — but the node's own sections always come along regardless,
/// because they live as `Flag::SectionRoot` children of the container
/// and store absolute canvas positions in the tree (a stationary
/// section beneath a moving container would visibly detach from its
/// node).
pub fn apply_drag_delta(tree: &mut MindMapTree, node_id: &str, dx: f32, dy: f32, include_descendants: bool) {
    let tree_node_id = match tree.arena_id_for(node_id) {
        Some(id) => id,
        None => return,
    };

    if include_descendants {
        apply_delta_recursive(&mut tree.tree.arena, tree_node_id, dx, dy);
    } else {
        apply_delta_node_and_sections(&mut tree.tree.arena, tree_node_id, dx, dy);
    }
    // Position writes go through `area.move_position` directly,
    // bypassing `MutatorTree::apply_to`'s wrapper that owns the
    // cache invalidation. Mark the geometry caches dirty so the
    // next `ensure_subtree_aabbs()` call recomputes — pre-fix the
    // overflow-aware `point_in_node_aabb` could read a stale
    // subtree AABB after a drag tick.
    tree.tree.invalidate_caches();
}

/// Move a node container plus every section-area / section-model
/// descendant under it. Skips child mind-node containers (and
/// their subtrees) — the "drag this node only" path.
fn apply_delta_node_and_sections(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32,
    dy: f32,
) {
    if let Some(node) = arena.get_mut(node_id) {
        if let Some(area) = node.get_mut().glyph_area_mut() {
            area.move_position(dx, dy);
        }
    }
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        let is_section = arena
            .get(cid)
            .map(|n| n.get().flag_is_set(Flag::SectionRoot))
            .unwrap_or(false);
        if is_section {
            apply_delta_recursive(arena, cid, dx, dy);
        }
    }
}

/// Apply a position delta and return `(unique_id, new_position)` for
/// every node that was moved. The renderer uses these patches to
/// update buffer positions in-place without reshaping text.
///
/// O(moved_nodes) — no text shaping, no font-system lock. Uses
/// `first_child` / `next_sibling` iteration instead of collecting
/// descendants into a `Vec` (§B7).
pub fn apply_drag_delta_and_collect_patches(
    tree: &mut MindMapTree,
    node_id: &str,
    dx: f32,
    dy: f32,
    include_descendants: bool,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    let tree_node_id = match tree.arena_id_for(node_id) {
        Some(id) => id,
        None => return,
    };

    if include_descendants {
        collect_patches_recursive(&mut tree.tree.arena, tree_node_id, dx, dy, patches);
    } else {
        // Container plus every section sub-element (`Flag::SectionRoot`).
        // Sections store absolute canvas positions and must move
        // with the node container or they'll visibly detach. Child
        // mind-nodes (without `Flag::SectionRoot`) are skipped —
        // the historical `include_descendants=false` semantic.
        collect_patches_node_and_sections(&mut tree.tree.arena, tree_node_id, dx, dy, patches);
    }
    // Same invalidation contract as `apply_drag_delta` — position
    // writes go through `area.move_position` directly.
    tree.tree.invalidate_caches();
}

/// Per-frame tree mutation for the section-drag gesture: move only
/// the targeted section's subtree (the section-area `GlyphArea` plus
/// its structural `GlyphModel` grand-descendants), leaving the
/// owning node's container and sibling sections untouched.
///
/// Mirrors [`apply_drag_delta_and_collect_patches`]'s patch-emission
/// shape so the renderer's `patch_drag_positions` can update buffer
/// positions in place — no text reshaping, no font-system lock.
/// Per-frame safe; release-commit syncs the model via
/// `set_section_offset` (which AABB-validates and pushes a single
/// undo entry).
pub fn apply_section_drag_delta_and_collect_patches(
    tree: &mut MindMapTree,
    node_id: &str,
    section_idx: usize,
    dx: f32,
    dy: f32,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    let section_root = match tree.section_arena_id(node_id, section_idx) {
        Some(id) => id,
        None => return,
    };
    collect_patches_recursive(&mut tree.tree.arena, section_root, dx, dy, patches);
    tree.tree.invalidate_caches();
}

/// Hit-test the 8 resize handles of a `Some`-sized section at
/// `canvas_pos`. Returns the closest handle whose canvas-space
/// center is within `tolerance` of the cursor, or `None` if no
/// handle is in range. Mirrors
/// [`super::MindMapDocument::hit_test_edge_handle`] — same
/// "compute live positions, scan within tolerance" shape.
///
/// Returns `None` for `None`-sized sections (fill-parent — no
/// handles emitted) and for missing nodes / sections. Bounded
/// cost: 8 distance comparisons per call.
pub fn hit_test_section_resize_handle(
    map: &MindMap,
    canvas_pos: Vec2,
    node_id: &str,
    section_idx: usize,
    tolerance: f32,
) -> Option<ResizeHandleSide> {
    let node = map.nodes.get(node_id)?;
    let section = node.sections.get(section_idx)?;
    let section_size = section.size.as_ref()?;
    let section_pos = Vec2::new(
        node.position.x as f32 + section.offset.x as f32,
        node.position.y as f32 + section.offset.y as f32,
    );
    let size = Vec2::new(section_size.width as f32, section_size.height as f32);
    let handles = build_section_resize_handles(node_id, section_idx, section_pos, Some(size));

    let mut best: Option<(ResizeHandleSide, f32)> = None;
    for h in handles {
        let pos = Vec2::new(h.position.0, h.position.1);
        let dist = canvas_pos.distance(pos);
        if dist > tolerance {
            continue;
        }
        if best.as_ref().map_or(true, |(_, d)| dist < *d) {
            best = Some((h.side, dist));
        }
    }
    best.map(|(s, _)| s)
}

/// Collect drag patches for the container plus its section
/// descendants only — siblings that carry `Flag::SectionRoot`
/// recurse, child mind-nodes are skipped.
fn collect_patches_node_and_sections(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32,
    dy: f32,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    if let Some(node) = arena.get_mut(node_id) {
        let elem = node.get_mut();
        if let Some(area) = elem.glyph_area_mut() {
            area.move_position(dx, dy);
        }
        let pos = elem.position();
        patches.push((elem.unique_id(), (pos.x, pos.y)));
    }
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        let is_section = arena
            .get(cid)
            .map(|n| n.get().flag_is_set(Flag::SectionRoot))
            .unwrap_or(false);
        if is_section {
            collect_patches_recursive(arena, cid, dx, dy, patches);
        }
    }
}

/// Recursively apply delta and collect patches via `first_child` /
/// `next_sibling` — zero allocations per call (§B7).
fn apply_delta_recursive(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32,
    dy: f32,
) {
    // Move this node.
    if let Some(node) = arena.get_mut(node_id) {
        if let Some(area) = node.get_mut().glyph_area_mut() {
            area.move_position(dx, dy);
        }
    }
    // Recurse into children.
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        apply_delta_recursive(arena, cid, dx, dy);
    }
}

/// Recursively apply delta, collect patches, via `first_child` /
/// `next_sibling` — zero allocations per call (§B7).
///
/// Patches are only emitted for elements that carry a renderer
/// buffer entry — i.e. `GlyphArea`-bearing variants. Section-
/// model `GlyphModel` siblings have no buffer key (the renderer
/// doesn't shape them), so emitting their `unique_id` would
/// drive a hash miss in `patch_drag_positions` per drag tick on
/// every section-model in the dragged subtree. Dropping them
/// here costs one branch per arena entry; the avoided hash
/// misses cost an order of magnitude more.
fn collect_patches_recursive(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32,
    dy: f32,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    // Move this node and collect patch.
    if let Some(node) = arena.get_mut(node_id) {
        let elem = node.get_mut();
        if let Some(area) = elem.glyph_area_mut() {
            area.move_position(dx, dy);
            let pos = elem.position();
            patches.push((elem.unique_id(), (pos.x, pos.y)));
        }
    }
    // Recurse into children.
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        collect_patches_recursive(arena, cid, dx, dy, patches);
    }
}
