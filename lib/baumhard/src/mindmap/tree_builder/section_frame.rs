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
/// Identity captures `(node_id, section_idx, focused, color,
/// per-side rendered text)` — running each side's `SidePattern`
/// through `border_run_specs` and hashing the resulting glyph
/// strings. That catches every change a creative-toolkit author
/// can make: preset, pattern, corner, color, focus toggle. The
/// rendered-text path is the cheapest stable hash given that
/// `BorderStyle` itself doesn't derive `Hash` (it carries
/// runtime-resolved glyph sets and palette enums).
pub fn section_frame_identity_sequence(
    elements: &[SectionFrameElement],
) -> Vec<SectionFrameIdentity> {
    elements
        .iter()
        .map(|e| {
            let specs = crate::mindmap::border::border_run_specs(
                &e.border_style,
                e.position,
                e.size,
            );
            SectionFrameIdentity {
                node_id: e.node_id.clone(),
                section_idx: e.section_idx,
                focused: e.focused,
                color: e.border_style.color.clone(),
                top: specs[0].text.clone(),
                bottom: specs[1].text.clone(),
                left: specs[2].text.clone(),
                right: specs[3].text.clone(),
            }
        })
        .collect()
}

/// One row of the structural signature returned by
/// [`section_frame_identity_sequence`]. Hashable so the
/// `AppScene` dispatch can compare against the last frame's
/// signature without re-walking the elements.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SectionFrameIdentity {
    pub node_id: String,
    pub section_idx: usize,
    pub focused: bool,
    pub color: String,
    pub top: String,
    pub bottom: String,
    pub left: String,
    pub right: String,
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
    let specs = crate::mindmap::border::border_run_specs(
        &frame.border_style,
        frame.position,
        frame.size,
    );
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
