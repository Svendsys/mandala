// SPDX-License-Identifier: MPL-2.0

//! Node resize-handle emission for the currently-selected node.
//! Sibling of [`super::section_resize_handle`] — same role pattern,
//! different domain. Emits 8 handles (corners + edge midpoints) on
//! top of the selected node when its size is finite + positive;
//! produces zero handles otherwise.
//!
//! Handle placement is on the node's `(position, size)` AABB. Unlike
//! sections, nodes have no parent AABB to fit inside, so the resize
//! validation rules (in `set_node_aabb`) only check finite + positive
//! + non-astronomical — there's no "extends past parent" guard.

use glam::Vec2;

use super::section_resize_handle::{
    ResizeHandleSide, SECTION_RESIZE_HANDLE_FONT_SIZE_PT, SECTION_RESIZE_HANDLE_GLYPH,
};
use super::SELECTED_EDGE_COLOR;

/// One resize-handle glyph emitted on top of a selected node.
/// Sibling of [`super::SectionResizeHandleElement`]; differs only in
/// the carried identity (`node_id` here, `node_id + section_idx` on
/// the section variant). The renderer treats `node_resize_handles`
/// as its own buffer family — small (≤ 8), only for the currently-
/// selected node.
pub struct NodeResizeHandleElement {
    /// Owning MindNode id.
    pub node_id: String,
    /// Which of the 8 handles this element represents.
    pub side: ResizeHandleSide,
    /// Canvas-space center of the handle.
    pub position: (f32, f32),
    /// Glyph string (single char).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

/// Build the 8-handle set for a single selected node. Returns an
/// empty vector when `node_size` has any non-finite or non-positive
/// component — those nodes can't host a meaningful resize gesture
/// (the verifier flags the underlying state already).
pub fn build_node_resize_handles(
    node_id: &str,
    node_pos: Vec2,
    node_size: Vec2,
) -> Vec<NodeResizeHandleElement> {
    if !node_size.x.is_finite() || !node_size.y.is_finite() || node_size.x <= 0.0 || node_size.y <= 0.0 {
        return Vec::new();
    }

    let (x, y) = (node_pos.x, node_pos.y);
    let (w, h) = (node_size.x, node_size.y);
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let right = x + w;
    let bottom = y + h;

    let positions = [
        (ResizeHandleSide::NW, (x, y)),
        (ResizeHandleSide::N, (cx, y)),
        (ResizeHandleSide::NE, (right, y)),
        (ResizeHandleSide::E, (right, cy)),
        (ResizeHandleSide::SE, (right, bottom)),
        (ResizeHandleSide::S, (cx, bottom)),
        (ResizeHandleSide::SW, (x, bottom)),
        (ResizeHandleSide::W, (x, cy)),
    ];

    positions
        .into_iter()
        .map(|(side, position)| NodeResizeHandleElement {
            node_id: node_id.to_string(),
            side,
            position,
            glyph: SECTION_RESIZE_HANDLE_GLYPH.to_string(),
            color: SELECTED_EDGE_COLOR.to_string(),
            font_size_pt: SECTION_RESIZE_HANDLE_FONT_SIZE_PT,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Some`-sized nodes get exactly 8 handles, one per side, at
    /// the AABB's corners and edge midpoints.
    #[test]
    fn build_emits_eight_handles_at_corners_and_edge_mids() {
        let handles = build_node_resize_handles("0", Vec2::new(10.0, 20.0), Vec2::new(100.0, 40.0));
        assert_eq!(handles.len(), 8);
        let by_side: std::collections::HashMap<ResizeHandleSide, (f32, f32)> =
            handles.iter().map(|h| (h.side, h.position)).collect();
        assert_eq!(by_side[&ResizeHandleSide::NW], (10.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::NE], (110.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::SW], (10.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::SE], (110.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::N], (60.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::S], (60.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::W], (10.0, 40.0));
        assert_eq!(by_side[&ResizeHandleSide::E], (110.0, 40.0));
    }

    /// Non-finite size → no handles.
    #[test]
    fn build_returns_empty_for_non_finite_size() {
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(f32::NAN, 10.0));
        assert!(handles.is_empty());
    }

    /// Non-positive size → no handles.
    #[test]
    fn build_returns_empty_for_non_positive_size() {
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(0.0, 10.0));
        assert!(handles.is_empty());
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(10.0, -5.0));
        assert!(handles.is_empty());
    }
}
