// SPDX-License-Identifier: MPL-2.0

//! Section-frame tree builder: emits one per-section Void parent
//! and four `GlyphArea` runs (top, bottom, left, right) per
//! [`SectionFrameElement`]. The four-side run geometry comes from
//! [`crate::mindmap::border::border_run_specs`] — the **same** path
//! node borders use — so any preset, any per-side `SidePattern`,
//! any per-corner glyph, any palette cycle that node borders
//! support flows through section frames identically.
//!
//! Stable identity = the order [`SectionFrameElement`]s are
//! emitted in. Per-frame Void parent's channel is the 1-based
//! sorted index so distinct frames never collide across rebuilds.

use indextree::NodeId;

use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::scene_builder::SectionFrameElement;
use crate::util::color::hex_to_rgba_safe;

/// Compute a stable structural-signature seed for a section-frame
/// element list. Hashed by `AppScene::set_canvas_signature` to
/// short-circuit redundant rebuilds.
///
/// Identity captures every **input** to the rendered glyph runs:
/// id triple (node_id, section_idx, focused), the resolved
/// `BorderStyle` axes (preset corners + 4 side patterns + color +
/// font + font_size + palette + palette_field), the position +
/// bounds (so a node move while in NodeEdit re-registers the
/// frames), and the resolved palette cycle (so an authored palette
/// edit triggers a rebuild). Hashing inputs — not the rendered
/// output — is both correct (no missed shifts that happen to
/// preserve cluster_count) and cheap (zero allocations on the
/// hot path; pre-fix the function ran `border_run_specs` four
/// times per frame on every NodeEdit rebuild for the side
/// strings, which were then thrown away after the hash compare).
///
/// Combined with the `InPlaceMutator` early-return in
/// `update_section_frame_tree`, the completeness of this signature
/// is load-bearing: a missed delta means the dispatch declares
/// "no work needed" and the screen keeps stale glyphs.
pub fn section_frame_identity_sequence(elements: &[SectionFrameElement]) -> Vec<SectionFrameIdentity> {
    elements
        .iter()
        .map(|e| {
            let bs = &e.border_style;
            SectionFrameIdentity {
                node_id: e.node_id.clone(),
                section_idx: e.section_idx,
                focused: e.focused,
                position_bits: (e.position.0.to_bits(), e.position.1.to_bits()),
                size_bits: (e.size.0.to_bits(), e.size.1.to_bits()),
                color: bs.color.clone(),
                font_name: bs.font_name.clone(),
                font_size_pt_bits: bs.font_size_pt.to_bits(),
                color_palette: bs.color_palette.clone(),
                palette_field: bs.palette_field,
                corners: bs.corners.clone(),
                side_patterns: bs.side_patterns.clone(),
                palette_cycle_bits: e
                    .palette_cycle
                    .iter()
                    .map(|c| [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()])
                    .collect(),
            }
        })
        .collect()
}

/// One row of the structural signature returned by
/// [`section_frame_identity_sequence`]. Hashable so the
/// `AppScene` dispatch can compare against the last frame's
/// signature without re-walking the elements.
///
/// Float fields ride as their `to_bits()` `u32` form so the struct
/// can derive `Hash`/`Eq` directly (NaN equality is irrelevant —
/// the only NaN that survives upstream is a parse error, which the
/// scene-builder gate filters out).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SectionFrameIdentity {
    pub node_id: String,
    pub section_idx: usize,
    pub focused: bool,
    pub position_bits: (u32, u32),
    pub size_bits: (u32, u32),
    pub color: String,
    pub font_name: Option<String>,
    pub font_size_pt_bits: u32,
    pub color_palette: Option<String>,
    pub palette_field: crate::mindmap::border::PaletteField,
    pub corners: crate::mindmap::border::BorderCorners,
    pub side_patterns: crate::mindmap::border::SidePatternQuad,
    pub palette_cycle_bits: Vec<[u32; 4]>,
}

/// Build a `Tree<GfxElement, GfxMutator>` from a slice of
/// [`SectionFrameElement`]s. Each element's resolved
/// [`crate::mindmap::border::BorderStyle`] is fed to
/// [`crate::mindmap::border::border_run_specs`] for the four-side
/// run geometry; the runs are appended via the same
/// `append_border_run` helper node borders use, so palette
/// cycling, multi-cluster fills, custom corners, and any future
/// border feature lights up on section frames automatically.
///
/// Empty input → empty tree (one void root, no children). The
/// caller (`scene_rebuild`) gates this against
/// `InteractionMode::NodeEdit` so non-NodeEdit rebuilds produce
/// a trivial tree.
pub fn build_section_frame_tree(elements: &[SectionFrameElement]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for (idx, frame) in elements.iter().enumerate() {
        let parent_channel = idx + 1;
        let parent_id = tree
            .arena
            .new_node(GfxElement::new_void_with_id(parent_channel, unique_id));
        tree.root.append(parent_id, &mut tree.arena);
        unique_id += 1;

        append_frame_runs(&mut tree, parent_id, frame, &mut unique_id);
    }
    tree
}

/// Layout the four-side glyph runs for one section frame and
/// append them under `parent_id`. Delegates to
/// [`crate::mindmap::border::border_run_specs`] for the run
/// geometry and to
/// [`super::border::append_border_run`] for the per-run
/// `GlyphArea` construction. Section frames inherit every layout
/// detail node borders have — corner overlap, character-width
/// approximation, palette-offset sweep around the rectangle —
/// for free.
fn append_frame_runs(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    frame: &SectionFrameElement,
    unique_id: &mut usize,
) {
    let specs = crate::mindmap::border::border_run_specs(&frame.border_style, frame.position, frame.size);
    let color_rgba = hex_to_rgba_safe(&frame.border_style.color, [1.0, 1.0, 1.0, 1.0]);
    // Frames inherit the active node's zoom window implicitly —
    // they only render while the active node is visible (the
    // outer dispatch gates emission on `is_hidden_by_fold`), so a
    // permissive zoom window is the right default. A future
    // refinement could pin the frame's window to the node's, but
    // that's redundant with the emission gate today.
    let zoom_visibility = crate::gfx_structs::zoom_visibility::ZoomVisibility::default();
    for spec in &specs {
        super::border::append_border_run(
            tree,
            parent_id,
            spec.channel,
            *unique_id,
            &spec.text,
            spec.font_size_pt,
            spec.position,
            spec.bounds,
            color_rgba,
            zoom_visibility,
            &frame.palette_cycle,
            spec.palette_offset,
            spec.cluster_count,
        );
        *unique_id += 1;
    }
}
