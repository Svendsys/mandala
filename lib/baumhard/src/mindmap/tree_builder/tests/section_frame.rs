// SPDX-License-Identifier: MPL-2.0

//! Section-frame tree-builder invariants: per-section Void
//! parents, four-side runs through `border_run_specs`, focus
//! style swap, identity-sequence stability for the §B2 dispatch.
//!
//! Pins the contract that section frames flow through the same
//! `BorderStyle` machinery node borders use: any preset, any
//! `SidePattern`, any per-corner glyph, any palette cycle a
//! creative-toolkit author can configure ends up in the section
//! frame's GlyphArea text without a separate code path.

use crate::mindmap::border::{resolve_border_style, BorderStyle};
use crate::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};
use crate::mindmap::scene_builder::SectionFrameElement;
use crate::mindmap::tree_builder::{build_section_frame_tree, section_frame_identity_sequence};

fn floor_style(focused: bool) -> BorderStyle {
    let cfg = GlyphBorderConfig {
        preset: if focused { "heavy".into() } else { "light".into() },
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    resolve_border_style(Some(&cfg), None, "#00E5FF")
}

fn frame(node_id: &str, idx: usize, focused: bool) -> SectionFrameElement {
    SectionFrameElement {
        node_id: node_id.to_string(),
        section_idx: idx,
        position: (100.0, 200.0 + idx as f32 * 30.0),
        size: (300.0, 30.0),
        border_style: floor_style(focused),
        palette_cycle: Vec::new(),
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
fn test_section_frame_tree_focused_uses_heavy_preset_top_corner() {
    let frames = [frame("n", 0, true)];
    let tree = build_section_frame_tree(&frames);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    // Top run = first child by channel order. Heavy preset uses
    // ┏ (U+250F) at the top-left corner.
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
        "focused frame's heavy preset must start with ┏, got {:?}",
        area.text
    );
}

#[test]
fn test_section_frame_tree_unfocused_uses_light_preset_top_corner() {
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
        "unfocused frame's light preset must start with ┌, got {:?}",
        area.text
    );
}

/// Custom-preset frame with author-supplied side / corner glyphs
/// renders the author's chars, **not** the preset defaults. Pins
/// the creative-toolkit contract: any glyph the user types ends
/// up on screen.
#[test]
fn test_section_frame_tree_custom_preset_renders_author_glyphs() {
    let cfg = GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "A".into(),
            bottom: "B".into(),
            left: "L".into(),
            right: "R".into(),
            top_left: "+".into(),
            top_right: "*".into(),
            bottom_left: "*".into(),
            bottom_right: "+".into(),
        }),
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    let style = resolve_border_style(Some(&cfg), None, "#00E5FF");
    let elem = SectionFrameElement {
        node_id: "n".into(),
        section_idx: 0,
        position: (100.0, 200.0),
        size: (300.0, 30.0),
        border_style: style,
        palette_cycle: Vec::new(),
        focused: false,
    };
    let tree = build_section_frame_tree(&[elem]);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    // The top run starts with the author's `top_left` corner '+'
    // and ends with `top_right` '*'.
    assert!(
        area.text.starts_with('+'),
        "custom-preset top must start with author's tl '+', got {:?}",
        area.text
    );
    assert!(
        area.text.ends_with('*'),
        "custom-preset top must end with author's tr '*', got {:?}",
        area.text
    );
    // The fill between corners uses the author's 'A' glyph.
    assert!(area.text.contains('A'), "custom-preset top fill must include 'A', got {:?}", area.text);
}

/// Section frame side patterns flow through `border_run_specs`,
/// which understands `SidePattern::PrefixFillSuffix`. Pin that a
/// `top="###(*)###"` pattern renders with the prefix + repeating
/// fill + suffix structure.
#[test]
fn test_section_frame_tree_supports_prefix_fill_suffix_pattern() {
    let cfg = GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "###(*)###".into(),
            bottom: "─".into(),
            left: "│".into(),
            right: "│".into(),
            top_left: "┌".into(),
            top_right: "┐".into(),
            bottom_left: "└".into(),
            bottom_right: "┘".into(),
        }),
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    };
    let style = resolve_border_style(Some(&cfg), None, "#00E5FF");
    let elem = SectionFrameElement {
        node_id: "n".into(),
        section_idx: 0,
        position: (0.0, 0.0),
        size: (400.0, 30.0),
        border_style: style,
        palette_cycle: Vec::new(),
        focused: false,
    };
    let tree = build_section_frame_tree(&[elem]);
    let parent = tree.root.children(&tree.arena).next().expect("one parent");
    let top_run_id = parent.children(&tree.arena).next().expect("top run");
    let area = tree
        .arena
        .get(top_run_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("GlyphArea");
    // The top text should contain at least one `*` (the fill)
    // between two `###` static segments.
    assert!(area.text.contains("###"), "top must contain prefix '###', got {:?}", area.text);
    assert!(area.text.contains('*'), "top must contain fill '*', got {:?}", area.text);
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
