// SPDX-License-Identifier: MPL-2.0

//! Section-frame tree builder: emits one per-section Void parent
//! and four `GlyphArea` runs (top, bottom, left, right) per
//! [`SectionFrameElement`]. The shape mirrors `tree_builder/border.rs`
//! — same four-side decomposition, same per-frame Void parent for
//! stable channels — but uses simpler box-drawing glyphs (no
//! palette cycling, no preset selection).
//!
//! Stable identity = `(node_id, section_idx)` lexicographic. Per-
//! frame Void parent's channel is the 1-based sorted index so
//! distinct frames never collide across rebuilds.

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::scene_builder::SectionFrameElement;
use crate::util::color;

/// Font size (in points) for section-frame glyphs. Smaller than
/// the node border (which uses ~14pt) so the section subdivisions
/// read as a finer-grained subdivision rather than competing with
/// the node frame.
pub const SECTION_FRAME_FONT_SIZE_PT: f32 = 10.0;

/// Approximate fraction of the font size that a frame glyph
/// occupies horizontally. Mirrors `BORDER_APPROX_CHAR_WIDTH_FRAC`
/// — the box-drawing chars share the same width:height ratio.
const SECTION_FRAME_APPROX_CHAR_WIDTH_FRAC: f32 = 0.6;

/// Compute a stable structural-signature seed for a section-frame
/// element list. Hashed by `AppScene::set_canvas_signature` to
/// short-circuit redundant rebuilds. Identity = ordered list of
/// `(node_id, section_idx, focused)` triples (the focused flag is
/// part of the signature so a focus toggle triggers a rebuild).
pub fn section_frame_identity_sequence(elements: &[SectionFrameElement]) -> Vec<(String, usize, bool)> {
    elements
        .iter()
        .map(|e| (e.node_id.clone(), e.section_idx, e.focused))
        .collect()
}

/// Build a `Tree<GfxElement, GfxMutator>` from a slice of
/// [`SectionFrameElement`]s. Each element produces a per-section
/// Void parent (channel = 1-based index) plus four GlyphArea runs
/// (top, bottom, left, right) with box-drawing glyphs in
/// `SELECTED_EDGE_COLOR`.
///
/// Empty input → empty tree (one void root, no children). The
/// caller (`scene_rebuild`) gates this against
/// `InteractionMode::NodeEdit` so non-NodeEdit frames produce a
/// trivial tree the §B2 dispatch can short-circuit.
pub fn build_section_frame_tree(
    elements: &[SectionFrameElement],
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for (idx, frame) in elements.iter().enumerate() {
        let parent_channel = idx + 1;
        let parent_id = tree.arena.new_node(GfxElement::new_void_with_id(
            parent_channel,
            unique_id,
        ));
        tree.root.append(parent_id, &mut tree.arena);
        unique_id += 1;

        append_frame_runs(&mut tree, parent_id, frame, &mut unique_id);
    }
    tree
}

/// Layout the four-side glyph runs for one section frame and
/// append them under `parent_id`. Same shape as
/// `tree_builder::border::append_border_run` but with simpler
/// uniform regions (no palette cycling) and frame-specific
/// box-drawing glyph sets.
fn append_frame_runs(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    frame: &SectionFrameElement,
    unique_id: &mut usize,
) {
    let glyphs = if frame.focused {
        FrameGlyphs::FOCUSED
    } else {
        FrameGlyphs::DEFAULT
    };

    let font_size = SECTION_FRAME_FONT_SIZE_PT;
    let approx_char_width = font_size * SECTION_FRAME_APPROX_CHAR_WIDTH_FRAC;
    let (pos_x, pos_y) = frame.position;
    let (size_x, size_y) = frame.size;
    let char_count = ((size_x / approx_char_width) + 2.0).ceil().max(3.0) as usize;
    let row_count = (size_y / font_size).ceil().max(1.0) as usize;
    let color_rgba = color::hex_to_rgba_safe(&frame.color, [1.0, 1.0, 1.0, 1.0]);

    // Top row: `glyphs.top_left + glyphs.horizontal × (char_count - 2) + glyphs.top_right`.
    let top_text = build_horizontal(glyphs.top_left, glyphs.horizontal, glyphs.top_right, char_count);
    let bottom_text = build_horizontal(glyphs.bottom_left, glyphs.horizontal, glyphs.bottom_right, char_count);
    let left_text = build_vertical(glyphs.vertical, row_count);
    let right_text = build_vertical(glyphs.vertical, row_count);

    let top_y = pos_y - font_size * 0.5;
    let bottom_y = pos_y + size_y - font_size * 0.5;
    let h_width = (char_count as f32 + 1.0) * approx_char_width;
    let v_width = approx_char_width * 2.0;

    // Channels 1..=4 inside each per-section subtree. The Void
    // parent above already disambiguates across sections.
    append_run(tree, parent_id, 1, *unique_id, &top_text, font_size, (pos_x - approx_char_width, top_y), (h_width, font_size), color_rgba);
    *unique_id += 1;
    append_run(tree, parent_id, 2, *unique_id, &bottom_text, font_size, (pos_x - approx_char_width, bottom_y), (h_width, font_size), color_rgba);
    *unique_id += 1;
    append_run(tree, parent_id, 3, *unique_id, &left_text, font_size, (pos_x - approx_char_width * 0.5, pos_y), (v_width, size_y), color_rgba);
    *unique_id += 1;
    append_run(tree, parent_id, 4, *unique_id, &right_text, font_size, (pos_x + size_x - approx_char_width * 0.5, pos_y), (v_width, size_y), color_rgba);
    *unique_id += 1;
}

/// Glyph set for one frame style. `DEFAULT` is thin (single-line
/// box-drawing); `FOCUSED` is heavy (the section currently inside
/// the inline text editor).
#[derive(Clone, Copy)]
struct FrameGlyphs {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

impl FrameGlyphs {
    /// Thin single-line box-drawing chars for unfocused sections.
    const DEFAULT: Self = Self {
        top_left: '\u{250C}',     // ┌
        top_right: '\u{2510}',    // ┐
        bottom_left: '\u{2514}',  // └
        bottom_right: '\u{2518}', // ┘
        horizontal: '\u{2500}',   // ─
        vertical: '\u{2502}',     // │
    };

    /// Heavy box-drawing chars for the focused section (the one
    /// inside the open text editor). Plan §4.4 — thicker stroke /
    /// 100% alpha than unfocused frames in the same NodeEdit mode.
    const FOCUSED: Self = Self {
        top_left: '\u{250F}',     // ┏
        top_right: '\u{2513}',    // ┓
        bottom_left: '\u{2517}',  // ┗
        bottom_right: '\u{251B}', // ┛
        horizontal: '\u{2501}',   // ━
        vertical: '\u{2503}',     // ┃
    };
}

fn build_horizontal(left: char, fill: char, right: char, char_count: usize) -> String {
    let mut s = String::with_capacity(char_count * 4);
    s.push(left);
    for _ in 1..char_count.saturating_sub(1) {
        s.push(fill);
    }
    s.push(right);
    s
}

fn build_vertical(glyph: char, row_count: usize) -> String {
    let mut s = String::with_capacity(row_count * 5);
    for i in 0..row_count {
        if i > 0 {
            s.push('\n');
        }
        s.push(glyph);
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn append_run(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    channel: usize,
    unique_id: usize,
    text: &str,
    font_size: f32,
    position: (f32, f32),
    bounds: (f32, f32),
    color_rgba: [f32; 4],
) {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        font_size,
        Vec2::new(position.0, position.1),
        Vec2::new(bounds.0, bounds.1),
    );

    // Single uniform color region across the whole run — no palette
    // cycling. Cluster count = grapheme count of the text (newlines
    // are folded into one cluster per visible glyph by the counter).
    let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(text);
    let mut regions = ColorFontRegions::new_empty();
    if cluster_count > 0 {
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(color_rgba),
        ));
    }
    area.regions = regions;

    let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
    let node = tree.arena.new_node(element);
    parent_id.append(node, &mut tree.arena);
}
