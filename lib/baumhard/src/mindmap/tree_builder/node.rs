// SPDX-License-Identifier: MPL-2.0

//! Node-tree helpers: convert a `MindNode` to a `GlyphArea`
//! (background-color resolution + text-run → `ColorFontRegions`
//! projection) and the recursive child-insertion walker that
//! `build_mindmap_tree` drives into the arena.

use std::collections::HashMap;

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::border::{
    resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC,
};
use crate::mindmap::model::{MindMap, MindNode};
use crate::util::color;
use crate::util::ordered_vec2::OrderedVec2;

/// Converts a MindNode's data into a Baumhard GlyphArea. Text-run colors
/// are resolved through the map's theme variables before being converted
/// to RGBA; unknown references and malformed hex fall back to transparent
/// black rather than panicking so a theme typo can't crash the render.
///
/// `canvas_default_border` is threaded through so the area can stamp the
/// resolved border's outward padding onto its `background_padding` —
/// without that, the node's background fill ends at the text rect and
/// the border glyphs (which sit one cell outside the rect) draw against
/// the canvas backdrop instead of the node's fill colour.
pub(super) fn mindnode_to_glyph_area(
    node: &MindNode,
    vars: &HashMap<String, String>,
    canvas_default_border: Option<&crate::mindmap::model::GlyphBorderConfig>,
) -> GlyphArea {
    let scale = node
        .text_runs
        .first()
        .map(|r| r.size_pt as f32)
        .unwrap_or(14.0);
    let line_height = scale * 1.2;
    let position = Vec2::new(node.position.x as f32, node.position.y as f32);
    let bounds = Vec2::new(node.size.width as f32, node.size.height as f32);

    let mut area = GlyphArea::new_with_str(&node.text, scale, line_height, position, bounds);

    // Background-fill padding: the border's four glyph runs sit
    // outside the text rect by one `font_size` vertically and one
    // `approx_char_width` horizontally on each side (see
    // `tree_builder::border::append_border_sub_tree`). Stamp the
    // same outward expansion onto the area's `background_padding`
    // so the renderer's fill rect covers the border area too. The
    // padding is computed from the resolved border style (per-node
    // override → canvas default → hardcoded preset defaults), and
    // is `Vec2::ZERO` when the frame is hidden or the shape isn't
    // a rectangle (the only shape borders attach to today).
    if node.style.show_frame
        && NodeShape::from_style_string(&node.style.shape) == NodeShape::Rectangle
    {
        let frame_color_resolved = color::resolve_var(&node.style.frame_color, vars);
        let border_style = resolve_border_style(
            node.style.border.as_ref(),
            canvas_default_border,
            frame_color_resolved,
        );
        let approx_char_width =
            border_style.font_size_pt * BORDER_APPROX_CHAR_WIDTH_FRAC;
        area.background_padding = OrderedVec2::new_f32(
            approx_char_width,
            border_style.font_size_pt,
        );
    }

    // Resolve the node's background color through theme variables and
    // pack it as u8 RGBA onto the tree element. The renderer's rect
    // pipeline reads it back out during `rebuild_buffers_from_tree`
    // and emits a solid quad behind the text glyphs.
    //
    // `None` means "no fill" — the canvas background shows through.
    // Both an empty string and a fully-transparent alpha ("#00000000"
    // / "#0000") map to `None`. Bad hex degrades to `None` as well,
    // so a theme typo leaves the node transparent rather than
    // painting it opaque black.
    area.background_color = {
        let raw = &node.style.background_color;
        if raw.is_empty() {
            None
        } else {
            let resolved = color::resolve_var(raw, vars);
            // Sentinel alpha = 0 means "parse failed" here because
            // the fallback is fully transparent. Authors can also
            // opt out with an explicit `#00000000` / `#0000`, which
            // lands on the same sentinel for free.
            let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 0.0]);
            if rgba[3] <= 0.0 {
                None
            } else {
                Some(color::convert_f32_to_u8(&rgba))
            }
        }
    };

    // Resolve the format-level `style.shape` string into a
    // `NodeShape`. Unknown / empty values fall back to Rectangle
    // (same "survive a typo" posture as `background_color` above).
    // The renderer's rect pipeline and the BVH hit test both read
    // this single field, so setting it here is enough to change
    // both visuals and input together.
    area.shape = NodeShape::from_style_string(&node.style.shape);

    // Stamp the node's optional zoom window onto the area. Default
    // (both `None`) leaves the area unbounded — the renderer's
    // final cull skips no-ops for it. Border areas inherit this
    // same window via `MindNode::zoom_window`, so a node that
    // disappears at high zoom takes its glyph frame with it.
    area.zoom_visibility = node.zoom_window();

    // Convert text runs to ColorFontRegions. The data-model
    // `TextRun.font` is a family-name string; resolve it through the
    // font table so the per-region attrs builder
    // (`baumhard::font::attrs::attrs_list_from_regions`) can pin the
    // chosen face. Empty / unknown family resolves to `None`, and
    // the attrs builder falls back to monospace with a warning.
    let mut regions = ColorFontRegions::new_empty();
    for run in &node.text_runs {
        let resolved = color::resolve_var(&run.color, vars);
        let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0]);
        let font = if run.font.is_empty() {
            None
        } else {
            crate::font::fonts::app_font_by_family(&run.font)
        };
        regions.submit_region(ColorFontRegion::new(
            Range::new(run.start, run.end),
            font,
            Some(rgba),
        ));
    }
    area.regions = regions;

    area
}

pub(super) fn build_children_recursive(
    map: &MindMap,
    parent_mind_id: &str,
    parent_node_id: NodeId,
    tree: &mut Tree<GfxElement, GfxMutator>,
    node_map: &mut HashMap<String, NodeId>,
    id_counter: &mut usize,
) {
    let vars = &map.canvas.theme_variables;
    let canvas_default_border = map.canvas.default_border.as_ref();
    let children = map.children_of(parent_mind_id);
    for child in &children {
        if map.is_hidden_by_fold(child) {
            continue;
        }
        let area = mindnode_to_glyph_area(child, vars, canvas_default_border);
        let element = GfxElement::new_area_non_indexed_with_id(area, child.channel, *id_counter);
        *id_counter += 1;

        let child_node_id = tree.arena.new_node(element);
        parent_node_id.append(child_node_id, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        build_children_recursive(map, &child.id, child_node_id, tree, node_map, id_counter);
    }
}
