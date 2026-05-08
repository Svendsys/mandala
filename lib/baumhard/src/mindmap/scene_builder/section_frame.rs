// SPDX-License-Identifier: MPL-2.0

//! Per-section frame pass for `InteractionMode::NodeEdit`. Emits
//! one [`SectionFrameElement`] per section of the active node so
//! the renderer can draw a glyph rectangle around each section —
//! the visual cue telling the user "these are the per-section
//! subdivisions you can pick from."
//!
//! The frame style is resolved through the same
//! [`crate::mindmap::border::resolve_section_frame_border`]
//! cascade that backs every other border in the system. Authors
//! who want a per-section frame style write to
//! `MindSection.frame_border`; map-wide defaults live on
//! `Canvas.default_section_frame_border` (and the focused
//! variant). When neither is set, a thin / heavy floor preset
//! flows through the same resolver so the returned `BorderStyle`
//! has the same shape every other border consumer sees.
//!
//! Skipped entirely in Default mode and for single-section active
//! nodes (where the frame would duplicate the border, and the
//! single-section short-circuit bypasses NodeEdit anyway). The
//! caller (`build_scene_with_cache`) gates emission on
//! `node_edit_for == Some(active)`.

use std::collections::HashMap;

use super::node_pass::section_aabb;
use super::{SectionFrameElement, SELECTED_EDGE_COLOR};
use crate::mindmap::border::{resolve_palette_cycle, resolve_section_frame_border};
use crate::mindmap::model::MindMap;
use crate::util::color::{hex_to_rgba_safe, resolve_var};

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
/// Each emitted element carries a fully-resolved [`BorderStyle`]
/// plus a `palette_cycle`. The matching
/// `(active_node, focused_section_idx)` section emits
/// `focused = true`; the resolver flips to the focused-variant
/// cascade for that one element so its style can differ from
/// its unfocused siblings.
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

    // The active-affordance signal (cyan SELECTED_EDGE_COLOR) sits
    // at the bottom of the cascade — authors who set
    // `frame_border.color` on their override fully replace it,
    // which is the desired shape for "make my borders tell a
    // story." Authors who want the cyan default just leave color
    // unset on their config.
    let frame_color_resolved = resolve_var(SELECTED_EDGE_COLOR, &map.canvas.theme_variables);

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
        let focused = focused_idx == Some(section_idx);
        let border_style =
            resolve_section_frame_border(section, &map.canvas, focused, frame_color_resolved);
        let fallback_rgba = hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
        let palette_cycle =
            resolve_palette_cycle(&map.palettes, &border_style, fallback_rgba);
        out.push(SectionFrameElement {
            node_id: active_id.to_string(),
            section_idx,
            position: (sx, sy),
            size: (sw, sh),
            border_style,
            palette_cycle,
            focused,
        });
    }
    out
}
