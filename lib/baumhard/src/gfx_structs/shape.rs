// SPDX-License-Identifier: MPL-2.0

//! Per-node hit-test shapes.
//!
//! [`crate::gfx_structs::shape::NodeShape`] is the silhouette a node's
//! body occupies for the hit test: the projection keeps one per visible
//! node, taken from the node's typed `Shape`
//! ([`crate::document::values::Shape`]), and asks it whether a point or a
//! rubber-band rectangle lands on the node. Nothing draws the shape
//! itself: a node draws no fill (D12), and its frame is text along the
//! outline (`flow`).
//!
//! Extending the set is deliberately local: add a variant, then a branch
//! in [`crate::gfx_structs::shape::NodeShape::contains_local`] and
//! [`crate::gfx_structs::shape::NodeShape::intersects_local_aabb`]. Both
//! `match`es are exhaustive, so a new variant does not compile until it
//! has both.

use crate::util::geometry::aabb_contains;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// The hit shape of a node. Stored on
/// [`crate::gfx_structs::area::GlyphArea`] and read by the BVH hit
/// test, and kept by the projection for each visible node's body.
///
/// The variant is copied out of the area in the hot paths, so it is
/// intentionally `Copy` and allocation-free.
///
/// # Costs
/// O(1) to copy, hash, compare. No heap allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum NodeShape {
    /// Fills the bounding box exactly — the legacy behavior and the
    /// default for any node that doesn't opt in to a different shape.
    #[default]
    Rectangle,
    /// Axis-aligned ellipse inscribed in the bounding box. A perfect
    /// circle is expressed as an `Ellipse` with `width == height`;
    /// the same variant handles stretched / "conical" cases where
    /// the box is wider than it is tall (or vice versa) without any
    /// extra parameters.
    Ellipse,
}

impl NodeShape {
    /// Point-in-shape test in the node's **local** coordinate space,
    /// where the bounding box runs from `(0, 0)` to `bounds`.
    ///
    /// Callers pre-translate into that frame, and **which two values
    /// they subtract matters**: derive `local` and `bounds` from the
    /// same `min` / `max` pair, as
    /// `local = point - min`, `bounds = max - min`. Recomputing
    /// either side independently (say `local = point - position`
    /// against a separately-stored extent) makes the boundary
    /// compare inexact — f32 addition is not associative, so
    /// `(position + extent) - position` can exceed `extent` by an
    /// ULP and a point exactly on the far edge then reports `false`.
    /// `bvh_find` in [`tree`](crate::gfx_structs::tree) is the
    /// reference caller.
    ///
    /// A degenerate `bounds` (either dimension `<= 0`) always
    /// reports `false`, matching how the BVH skips zero-size areas
    /// at the AABB stage.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn contains_local(self, local: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        match self {
            NodeShape::Rectangle => aabb_contains(local, Vec2::ZERO, bounds),
            NodeShape::Ellipse => {
                // Normalized coordinates in [-1, 1] relative to the
                // ellipse center. A perfect circle is `bounds.x ==
                // bounds.y`; a stretched conic is anything else.
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let nx = (local.x - rx) / rx;
                let ny = (local.y - ry) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }

    /// Does the shape's filled area overlap the AABB
    /// `[min, max]` (in local coordinates, same frame as
    /// [`Self::contains_local`])? Conservative — false positives
    /// are tolerated, false negatives are not. Used by rect-select.
    ///
    /// For the ellipse variant this clamps the AABB to the
    /// ellipse's bounding box, then checks whether the closest
    /// clamped point sits inside the ellipse. That's conservative
    /// in the "selection rect fully inside the ellipse" corner
    /// (the closest-point test returns true when any corner of the
    /// rect is inside the ellipse, *or* when the ellipse-center is
    /// inside the rect) — which is what we want for a lasso.
    ///
    /// Degenerate bounds (zero or negative extent on either axis)
    /// report `false` even if the AABBs would overlap numerically.
    /// This matches [`Self::contains_local`] and the BVH's
    /// `bounds.x > 0.0 && bounds.y > 0.0` guard in `bvh_find`:
    /// a zero-size node renders nothing, so selecting nothing for
    /// it is the internally consistent answer. Small behavior
    /// change from the pre-shape `rect_select` (gone) (which would have
    /// matched a point-sized node under the cursor) — considered
    /// an improvement and noted here so a future test author can
    /// find the rationale.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn intersects_local_aabb(self, min: Vec2, max: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        // First, AABB–AABB overlap. Bails on any shape whose bounds
        // don't touch the selection rectangle at all.
        if max.x < 0.0 || min.x > bounds.x || max.y < 0.0 || min.y > bounds.y {
            return false;
        }
        match self {
            NodeShape::Rectangle => true,
            NodeShape::Ellipse => {
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let cx = rx;
                let cy = ry;
                // Closest point on the AABB to the ellipse center.
                let clamped_x = cx.clamp(min.x, max.x);
                let clamped_y = cy.clamp(min.y, max.y);
                let nx = (clamped_x - cx) / rx;
                let ny = (clamped_y - cy) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }
}

// Tests live out-of-line at
// `lib/baumhard/src/gfx_structs/tests/shape_tests.rs` so the
// criterion bench harness at `lib/baumhard/benches/test_bench.rs`
// can reuse each `do_*()` body as a micro-benchmark — see
// TEST_CONVENTIONS.md §T2.2.
