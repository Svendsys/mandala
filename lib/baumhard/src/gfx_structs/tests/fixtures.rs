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

use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;

/// Build a `GfxElement::GlyphArea` at canvas `(x, y)` with the given
/// channel and unique id. All other fields use sensible defaults
/// (scale 1.0, line_height 10, bounds 100×100). Used wherever the
/// test only cares about a node's position and does not exercise
/// the area's text or scale.
pub fn mk_area(x: f32, y: f32, channel: usize, unique_id: usize) -> GfxElement {
    GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new(1.0, 10.0, Vec2::new(x, y), Vec2::new(100.0, 100.0)),
        channel,
        unique_id,
    )
}

/// Allocate a fresh `GlyphArea` element in `model` via [`mk_area`],
/// append it to `parent`, and return its `NodeId`. Collapses the
/// four-line `arena.new_node(...) + parent.append(...)` dance into
/// one call — the dance was the same shape across every test that
/// builds a multi-node tree by hand.
pub fn append_area(
    model: &mut Tree<GfxElement, GfxMutator>,
    parent: NodeId,
    x: f32,
    y: f32,
    channel: usize,
    unique_id: usize,
) -> NodeId {
    let id = model.arena.new_node(mk_area(x, y, channel, unique_id));
    parent.append(id, &mut model.arena);
    id
}
