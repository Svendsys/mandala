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

    // Background-fill padding: extend the fill outward to land on
    // the *visible glyph stroke* of the surrounding border runs,
    // not on the runs' buffer-cell edges. The previous pass used
    // cell-edge values, but cosmic-text positions text inside the
    // cell — the visible `─` and `│` strokes sit roughly at the
    // centre of the em-square (`0.5·fs` from the cell top for the
    // horizontals, `0.5·acw` from the cell left for the verticals),
    // not at the cell boundary. Cell-edge padding therefore
    // overshot the visible line by 0.5·fs ≈ 7px on top/bottom and
    // 0.5·acw ≈ 4px on left/right.
    //
    // Layout reference (matching `tree_builder::border::append_border_sub_tree`):
    //   top run buffer top:    ny - fs + corner_overlap
    //   bottom run buffer top: ny + nh - corner_overlap
    //   left column cell left:  nx - approx_char_width
    //   right column cell left: right_corner_x
    //   right_corner_x = nx + char_count·acw - 2·acw
    //                    where char_count = ceil(nw/acw + 2).max(3)
    //
    // Visible stroke centres (assuming `─` and `│` are at em
    // centre, the convention for box-drawing in monospace faces):
    //   top:    (ny - fs + corner_overlap) + 0.5·fs = ny - (0.5·fs - corner_overlap)
    //   bottom: (ny + nh - corner_overlap) + 0.5·fs = ny + nh + (0.5·fs - corner_overlap)
    //   left:   (nx - acw) + 0.5·acw = nx - 0.5·acw
    //   right:  right_corner_x + 0.5·acw
    //         = nx + char_count·acw - 2·acw + 0.5·acw
    //         = (nx + nw) + (char_count·acw - 1.5·acw - nw)
    //
    // Outward extents from the text rect (this is `pad_*`):
    //   pad_top    = 0.5·fs - corner_overlap
    //   pad_bottom = 0.5·fs - corner_overlap   (same, by symmetry of the math)
    //   pad_left   = 0.5·acw
    //   pad_right  = char_count·acw - 1.5·acw - nw
    //
    // pad_right is per-node — when `nw mod acw != 0`, `char_count`
    // rounds up by one and the right column's outer edge can sit
    // up to `acw` further from `nx + nw` than the left column's
    // does from `nx`. Symmetric `pad_right = pad_left` was wrong:
    // it under-padded by 0..acw px depending on `nw`.
    //
    // The 0.5·fs and 0.5·acw multipliers approximate the per-glyph
    // position of `─` / `│` within the em-square. They're correct
    // for LiberationSans-style monospace box-drawing; per-face
    // calibration lives on `BORDER_CORNER_OVERLAP_FRAC` /
    // `BORDER_APPROX_CHAR_WIDTH_FRAC` in `border.rs`.
    //
    // `EdgePadding::ZERO` when the frame is hidden or the shape
    // isn't a rectangle (the only shape borders attach to today).
    if node.style.show_frame
        && NodeShape::from_style_string(&node.style.shape) == NodeShape::Rectangle
    {
        let frame_color_resolved = color::resolve_var(&node.style.frame_color, vars);
        let border_style = resolve_border_style(
            node.style.border.as_ref(),
            canvas_default_border,
            frame_color_resolved,
        );
        let fs = border_style.font_size_pt;
        let acw = fs * BORDER_APPROX_CHAR_WIDTH_FRAC;
        let corner_overlap = fs * crate::mindmap::border::BORDER_CORNER_OVERLAP_FRAC;
        let nw = node.size.width as f32;
        // Mirror the char_count formula in `append_border_sub_tree`
        // so `pad_right` and the actual right-column placement stay
        // in lock-step. `.max(3.0)` covers the degenerate
        // `nw < acw` case the same way the layout does.
        let char_count = ((nw / acw) + 2.0).ceil().max(3.0);
        let pad_top_bottom = 0.5 * fs - corner_overlap;
        let pad_left = 0.5 * acw;
        let pad_right = char_count * acw - 1.5 * acw - nw;
        area.background_padding = crate::gfx_structs::area::EdgePadding::new(
            /* top    */ pad_top_bottom,
            /* right  */ pad_right,
            /* bottom */ pad_top_bottom,
            /* left   */ pad_left,
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
