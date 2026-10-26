// SPDX-License-Identifier: MPL-2.0

//! Shared element- and tree-construction helpers for the
//! `gfx_structs` test suite. Pre-consolidation `tree_walker_tests`
//! and `map_children_tests` carried byte-for-byte identical
//! `mk_area` / `append_area` definitions; this module is the
//! canonical home so a `do_*` body that grows from one suite into
//! the other doesn't have to re-discover the helper.
//!
//! Declared `pub mod` per §T2.2 — the items are reachable from
//! `do_*` bodies that the criterion bench harness imports, so they
//! must compile in non-cfg(test) builds too. Each helper is `pub`
//! for the same reason.

use glam::Vec2;
use indextree::NodeId;

use crate::engine::tree::Placement;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::tree::GfxTree;

/// Build a `GfxElement::GlyphArea` at canvas `(x, y)` with the given
/// channel and unique id. All other fields use sensible defaults
/// (scale 1.0, line_height 10, bounds 100×100). Used wherever the
/// test only cares about a node's position and does not exercise
/// the area's text or scale.
///
/// Cost: one `GfxElement::new_area_non_indexed_with_id` call —
/// allocates an empty-text `GlyphArea` and wraps it in the
/// element variant. No font-system lock; no shaping.
pub fn mk_area(x: f32, y: f32, channel: usize, unique_id: usize) -> GfxElement {
    GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new(1.0, 10.0, Vec2::new(x, y), Vec2::new(100.0, 100.0)),
        channel,
        unique_id,
    )
}

/// Add a fresh `GlyphArea` element, made by [`mk_area`], under
/// `parent` in `model` — at the end of the run of its channel — and
/// return its `NodeId`. One call for the shape every test that builds
/// a multi-node tree by hand repeats.
///
/// Cost: one [`mk_area`] call plus one `insert_child` — an arena slot
/// and a scan of the siblings keyed above `channel`.
pub fn append_area(
    model: &mut GfxTree,
    parent: NodeId,
    x: f32,
    y: f32,
    channel: usize,
    unique_id: usize,
) -> NodeId {
    model
        .insert_child(parent, mk_area(x, y, channel, unique_id), Placement::Last)
        .unwrap()
}
