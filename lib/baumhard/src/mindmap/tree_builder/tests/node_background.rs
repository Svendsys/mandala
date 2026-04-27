// SPDX-License-Identifier: MPL-2.0

//! Tree-builder node-background tests — hex, empty, transparent, theme var, malformed, three-digit.

use super::super::*;
use super::fixtures::*;

#[test]
fn test_background_color_opaque_hex_populates_field() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("n").unwrap().style.background_color = "#ff8800".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([255, 136, 0, 255]));
}

#[test]
fn test_background_color_empty_string_becomes_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("n").unwrap().style.background_color = "".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

#[test]
fn test_background_color_fully_transparent_becomes_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    // `#00000000` is the conventional "no fill" opt-out.
    map.nodes.get_mut("n").unwrap().style.background_color = "#00000000".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

#[test]
fn test_background_color_resolves_theme_variable() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    map.canvas
        .theme_variables
        .insert("--panel".into(), "#112233".into());
    map.nodes.get_mut("n").unwrap().style.background_color = "var(--panel)".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([17, 34, 51, 255]));
}

#[test]
fn test_background_color_malformed_hex_degrades_to_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    // `hex_to_rgba_safe` degrades unknown/bad strings to the
    // fallback we passed in — `[0,0,0,0]` for background — which
    // then trips the transparent-alpha sentinel below and becomes
    // `None`. Keeps a typo from crashing the render.
    map.nodes.get_mut("n").unwrap().style.background_color = "not-a-color".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

/// Framed nodes get `background_padding` set to the border's
/// outward extension (one `font_size` vertically, one
/// `approx_char_width` horizontally) so the renderer's fill rect
/// draws under the border glyphs that sit outside the node's text
/// rect. Without this, the border would render against the canvas
/// backdrop instead of the node's fill colour.
#[test]
fn test_framed_node_carries_background_padding_for_border() {
    use crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
    let map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    // `synthetic_node` defaults `show_frame: true`.
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    let expected_x = 14.0 * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let expected_y = 14.0;
    assert!(
        (area.background_padding.x() - expected_x).abs() < 0.01,
        "background_padding.x = {} but expected {}",
        area.background_padding.x(),
        expected_x
    );
    assert!(
        (area.background_padding.y() - expected_y).abs() < 0.01,
        "background_padding.y = {} but expected {}",
        area.background_padding.y(),
        expected_y
    );
}

/// Frameless nodes (and non-rectangle shapes) leave
/// `background_padding` at zero — the historical behaviour. Only
/// rectangle nodes with `show_frame = true` extend the fill rect.
#[test]
fn test_frameless_node_keeps_zero_background_padding() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("n").unwrap().style.show_frame = false;
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_padding.x(), 0.0);
    assert_eq!(area.background_padding.y(), 0.0);
}

#[test]
fn test_background_color_three_digit_hex_works() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    // `#000` is the default in all the synthetic nodes above, and
    // it's opaque black — verify the builder treats it as a real
    // fill (not transparent) so the renderer draws the rect. A
    // future refactor that mis-parses short hex values would
    // regress this.
    map.nodes.get_mut("n").unwrap().style.background_color = "#000".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([0, 0, 0, 255]));
}
