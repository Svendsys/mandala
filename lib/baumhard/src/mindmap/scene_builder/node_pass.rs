// SPDX-License-Identifier: MPL-2.0

//! Per-node pass — emits `TextElement`s + `BorderElement`s and
//! computes the per-node clip AABBs (`node_aabbs`) in a single
//! iteration over visible nodes. Kept as one pass rather than split
//! into peer `text.rs` + `border.rs` modules because:
//!
//! - the `TextElement`, `BorderElement`, and AABB all derive from
//!   the same `(pos, size, offset, frame_color)` tuple
//! - the clip AABB's expansion-for-border math (see `clip_pos` /
//!   `clip_size` below) is the same `BorderStyle::default_with_color`
//!   resolution the border element uses
//!
//! Splitting them would force either a two-pass walk over
//! `map.nodes.values()` (perf regression on the hot drag path) or
//! an imbalanced `layout.rs` that returns three vectors — defeating
//! the role-per-file goal.

use std::collections::HashMap;

use glam::Vec2;

use crate::mindmap::border::resolve_border_style;
use crate::mindmap::model::{MindMap, TextRun};
use crate::util::color::resolve_var;

use super::{BorderElement, TextElement};

/// Walk every visible node and emit its text element + border
/// element + clip AABB. Returns the three collections in a tuple —
/// the connection pass downstream consumes `node_aabbs` for its
/// clip filter, so this walk must complete before connections start.
///
/// Hidden-by-fold nodes are skipped entirely. Empty-text nodes skip
/// the `TextElement` push but still contribute an AABB. Frameless
/// nodes skip the `BorderElement` push and use a raw-rect AABB
/// (no border-expansion) so connection glyphs can run right up to
/// the node edge.
pub(super) fn build_node_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> (Vec<TextElement>, Vec<BorderElement>, Vec<(Vec2, Vec2)>) {
    let vars = &map.canvas.theme_variables;
    let mut text_elements = Vec::new();
    let mut border_elements = Vec::new();
    let mut node_aabbs: Vec<(Vec2, Vec2)> = Vec::new();

    for node in map.nodes.values() {
        if map.is_hidden_by_fold(node) {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos = node.pos_vec2();
        let size = node.size_vec2();
        let pos_x = pos.x + ox;
        let pos_y = pos.y + oy;
        let size_x = size.x;
        let size_y = size.y;

        // Resolve the frame color through theme variables once — used for
        // both the clip AABB sizing and the border element below.
        let frame_color = resolve_var(&node.style.frame_color, vars);

        // Resolve the per-node border config once when the frame is
        // visible, then reuse for clip-AABB math, palette-cycle
        // resolution, and the emitted `BorderElement`. The cascade
        // walks `node.style.border` → `canvas.default_border` →
        // hardcoded defaults; doing this twice per visible node was a
        // hot-path regression — the resolver also reparses each side
        // pattern, so the cost compounds.
        let resolved_border = if node.style.show_frame {
            Some(resolve_border_style(
                node.style.border.as_ref(),
                map.canvas.default_border.as_ref(),
                frame_color,
            ))
        } else {
            None
        };

        // Clip AABB: when a node has a visible frame, the rendered border
        // extends beyond the raw node rect by roughly one border
        // `font_size` vertically and one `approx_char_width` horizontally.
        // Expand the clip box to match so connection glyphs don't land
        // inside the visible frame area (see renderer::rebuild_border_buffers
        // for the matching layout math).
        let (clip_pos, clip_size) = if let Some(border_style) = &resolved_border {
            let bf = border_style.font_size_pt;
            let bcw = bf * crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
            (
                Vec2::new(pos_x - bcw, pos_y - bf),
                Vec2::new(size_x + 2.0 * bcw, size_y + 2.0 * bf),
            )
        } else {
            (Vec2::new(pos_x, pos_y), Vec2::new(size_x, size_y))
        };
        node_aabbs.push((clip_pos, clip_size));

        // Text element (skip empty text nodes). Resolve each text run's
        // color through theme variables so the renderer downstream never
        // sees a `var(--name)` literal.
        if !node.text.is_empty() {
            let resolved_runs: Vec<TextRun> = node
                .text_runs
                .iter()
                .map(|run| {
                    let mut r = run.clone();
                    r.color = resolve_var(&run.color, vars).to_string();
                    r
                })
                .collect();
            text_elements.push(TextElement {
                node_id: node.id.clone(),
                text: node.text.clone(),
                text_runs: resolved_runs,
                position: (pos_x, pos_y),
                size: (size_x, size_y),
            });
        }

        // Border element — inherits the owning node's zoom window
        // so the frame never outlives its node at any zoom level.
        // Reuses the `resolved_border` populated above so the
        // resolver runs at most once per visible framed node.
        if let Some(border_style) = resolved_border {
            let fallback_rgba =
                crate::util::color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
            let palette_cycle =
                crate::mindmap::border::resolve_palette_cycle(&map.palettes, &border_style, fallback_rgba);
            border_elements.push(BorderElement {
                node_id: node.id.clone(),
                border_style,
                node_position: (pos_x, pos_y),
                node_size: (size_x, size_y),
                zoom_visibility: node.zoom_window(),
                palette_cycle,
            });
        }
    }

    (text_elements, border_elements, node_aabbs)
}
