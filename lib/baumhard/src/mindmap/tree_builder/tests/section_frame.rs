// SPDX-License-Identifier: MPL-2.0

//! Section-frame tree-builder invariants: per-section Void
//! parents, four-side runs, focus glyph swap, identity-sequence
//! stability for the §B2 dispatch.

use crate::mindmap::scene_builder::SectionFrameElement;
use crate::mindmap::tree_builder::{build_section_frame_tree, section_frame_identity_sequence};

fn frame(node_id: &str, idx: usize, focused: bool) -> SectionFrameElement {
    SectionFrameElement {
        node_id: node_id.to_string(),
        section_idx: idx,
        position: (100.0, 200.0 + idx as f32 * 30.0),
        size: (300.0, 30.0),
        color: "#00E5FF".to_string(),
        focused,
    }
}

#[test]
fn test_section_frame_tree_empty_input_yields_empty_tree() {
    let tree = build_section_frame_tree(&[]);
    // Only the void root — no children.
    assert_eq!(tree.root.children(&tree.arena).count(), 0);
}

#[test]
fn test_section_frame_tree_one_void_parent_per_frame() {
    let frames = [frame("n", 0, false), frame("n", 1, false), frame("n", 2, false)];
    let tree = build_section_frame_tree(&frames);
    let parents: Vec<_> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 3, "one Void parent per frame");
}

#[test]
fn test_section_frame_tree_four_runs_per_frame() {
    let frames = [frame("n", 0, false)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let runs: Vec<_> = parent.children(&tree.arena).collect();
    assert_eq!(runs.len(), 4, "top + bottom + left + right = 4 runs");
}

#[test]
fn test_section_frame_tree_focused_uses_heavy_glyphs() {
    let frames = [frame("n", 0, true)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    // Top run = first child by channel order. Heavy box-drawing
    // chars use ┏ (U+250F) for the top-left corner.
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert!(
        area.text.starts_with('\u{250F}'),
        "focused frame must use heavy ┏ corner, got {:?}",
        area.text
    );
}

#[test]
fn test_section_frame_tree_unfocused_uses_thin_glyphs() {
    let frames = [frame("n", 0, false)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    assert!(
        area.text.starts_with('\u{250C}'),
        "unfocused frame must use thin ┌ corner, got {:?}",
        area.text
    );
}

#[test]
fn test_section_frame_identity_sequence_includes_focus_flag() {
    let unfocused = [frame("n", 0, false)];
    let focused = [frame("n", 0, true)];
    let s_unfocused = section_frame_identity_sequence(&unfocused);
    let s_focused = section_frame_identity_sequence(&focused);
    assert_ne!(
        s_unfocused, s_focused,
        "focus toggle must change the structural signature so the dispatch rebuilds the glyphs"
    );
}

#[test]
fn test_section_frame_identity_sequence_stable_for_unchanged_inputs() {
    let frames = [frame("n", 0, false), frame("n", 1, true)];
    assert_eq!(
        section_frame_identity_sequence(&frames),
        section_frame_identity_sequence(&frames),
        "identity is a pure function of inputs"
    );
}
