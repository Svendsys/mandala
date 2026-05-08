// SPDX-License-Identifier: MPL-2.0

//! Per-section frame pass for `InteractionMode::NodeEdit`. Emits
//! one [`SectionFrameElement`] per section of the active node so
//! the renderer can draw a thin cyan rectangle around each section
//! — the visual cue telling the user "these are the
//! per-section subdivisions you can pick from."
//!
//! Skipped entirely in Default mode and for single-section active
//! nodes (where the frame would just duplicate the border, and the
//! single-section short-circuit bypasses NodeEdit anyway). The
//! caller (`build_scene_with_cache`) gates emission on
//! `node_edit_for == Some(active)`.

use std::collections::HashMap;

use super::node_pass::section_aabb;
use super::{SectionFrameElement, SELECTED_EDGE_COLOR};
use crate::mindmap::model::MindMap;

/// Emit one [`SectionFrameElement`] per section of `active_node`.
/// Returns an empty vector for:
/// - `active_node = None` (Default mode — no frames anywhere).
/// - The named node missing from `map.nodes` (stale NodeEdit
///   target after a custom-mutation deletion).
/// - The named node hidden by fold (the frame would otherwise
///   render under collapsed chrome).
/// - The named node having `sections.len() <= 1` (frame would
///   duplicate the border; the single-section short-circuit
///   bypasses NodeEdit anyway).
/// - Any section with non-finite or non-positive size /
///   non-finite offset — same skip rules `node_pass` applies to
///   `TextElement` emission, so frames track the same set of
///   "renderable" sections.
///
/// The matching `(active_node, focused_section_idx)` section
/// emits `focused = true`; every other emitted element gets
/// `focused = false`. The renderer uses this flag to draw the
/// focused frame at a thicker stroke (Plan §4.4).
pub fn build_section_frames(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    active_node: Option<&str>,
    focused_section: Option<(&str, usize)>,
) -> Vec<SectionFrameElement> {
    let Some(active_id) = active_node else {
        return Vec::new();
    };
    let Some(node) = map.nodes.get(active_id) else {
        return Vec::new();
    };
    if map.is_hidden_by_fold(node) {
        return Vec::new();
    }
    if node.sections.len() <= 1 {
        return Vec::new();
    }

    let (ox, oy) = offsets.get(active_id).copied().unwrap_or((0.0, 0.0));
    let pos = node.pos_vec2();
    let size = node.size_vec2();
    let pos_x = pos.x + ox;
    let pos_y = pos.y + oy;
    let size_x = size.x;
    let size_y = size.y;

    let focused_idx = focused_section
        .filter(|(id, _)| *id == active_id)
        .map(|(_, idx)| idx);

    let mut out: Vec<SectionFrameElement> = Vec::with_capacity(node.sections.len());
    for (section_idx, section) in node.sections.iter().enumerate() {
        if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
            continue;
        }
        if let Some(sz) = section.size.as_ref() {
            if !sz.width.is_finite()
                || !sz.height.is_finite()
                || sz.width <= 0.0
                || sz.height <= 0.0
            {
                continue;
            }
        }
        let ((sx, sy), (sw, sh)) = section_aabb(section, pos_x, pos_y, size_x, size_y);
        out.push(SectionFrameElement {
            node_id: active_id.to_string(),
            section_idx,
            position: (sx, sy),
            size: (sw, sh),
            color: SELECTED_EDGE_COLOR.to_string(),
            focused: focused_idx == Some(section_idx),
        });
    }
    out
}
