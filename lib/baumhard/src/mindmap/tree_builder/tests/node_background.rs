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

/// Framed nodes get `background_padding` set per-edge so the
/// fill rect extends out to where the *visible glyph stroke* of
/// each border run actually lands — not to the buffer-cell edge,
/// which sits ~`0.5·fs` (top/bottom) and ~`0.5·acw` (left/right)
/// further out. The four formulas:
///   pad_top    = 0.5·fs - corner_overlap
///   pad_bottom = 0.5·fs - corner_overlap                  (same)
///   pad_left   = 0.5·acw
///   pad_right  = char_count·acw - 1.5·acw - nw
/// pad_right is per-node because `char_count = ceil(nw/acw + 2)`
/// rounds up by one whenever `nw mod acw != 0`, shifting the
/// right column further outside than the left.
/// Mirrors the layout math in `tree_builder::border::append_border_sub_tree`.
#[test]
fn test_framed_node_carries_visible_stroke_background_padding() {
    use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};
    let map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    // `synthetic_node` defaults `show_frame: true`, size 80×40.
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    let fs = 14.0_f32;
    let acw = fs * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let overlap = fs * BORDER_CORNER_OVERLAP_FRAC;
    let nw = 80.0_f32;
    let char_count = ((nw / acw) + 2.0).ceil().max(3.0);
    let expected_top = 0.5 * fs - overlap;
    let expected_bottom = expected_top;
    let expected_left = 0.5 * acw;
    let expected_right = char_count * acw - 1.5 * acw - nw;
    let pad = area.background_padding;
    assert!(
        (pad.top() - expected_top).abs() < 0.01,
        "pad.top = {} but expected {}",
        pad.top(),
        expected_top
    );
    assert!(
        (pad.bottom() - expected_bottom).abs() < 0.01,
        "pad.bottom = {} but expected {}",
        pad.bottom(),
        expected_bottom
    );
    assert!(
        (pad.left() - expected_left).abs() < 0.01,
        "pad.left = {} but expected {}",
        pad.left(),
        expected_left
    );
    assert!(
        (pad.right() - expected_right).abs() < 0.01,
        "pad.right = {} but expected {}",
        pad.right(),
        expected_right
    );
    // Sanity: pad_top == pad_bottom (the visible `─` strokes sit
    // at the same em-centre offset in both runs, so the asymmetry
    // of the buffer cells doesn't carry to the visible-stroke
    // padding). And pad_right > pad_left for non-acw-aligned widths
    // — at nw=80 with acw=8.4, char_count=12 forces the right
    // column out by `(12·8.4 - 1.5·8.4 - 80) = 8.2` vs the left's
    // `0.5·8.4 = 4.2`. A regression to symmetric-with-left would
    // trip on this.
    assert!(
        (pad.top() - pad.bottom()).abs() < 0.001,
        "pad.top should equal pad.bottom by symmetry; got {}, {}",
        pad.top(),
        pad.bottom()
    );
    assert!(
        pad.right() > pad.left(),
        "pad.right ({}) should exceed pad.left ({}) when nw is not a multiple of acw",
        pad.right(),
        pad.left()
    );
}

/// `pad_right` tracks `nw` per-node — for a node whose width is
/// an exact integer multiple of `approx_char_width`, the right
/// column lines up with the left and the two pads should match
/// (both `0.5·acw`). This test makes the per-node dependence
/// explicit so a future refactor that reverts to a constant
/// `pad_right = approx_char_width` regresses visibly.
#[test]
fn test_pad_right_equals_pad_left_when_nw_is_acw_aligned() {
    use crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0.0, 0.0)],
        vec![],
    );
    let fs = 14.0_f32;
    let acw = fs * BORDER_APPROX_CHAR_WIDTH_FRAC;
    // 10 × acw = 84.0 px exactly — char_count = ceil(10 + 2) = 12,
    // pad_right = 12·acw - 1.5·acw - 10·acw = 0.5·acw = pad_left.
    let aligned_w = 10.0 * acw;
    map.nodes.get_mut("n").unwrap().size.width = aligned_w as f64;
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    let pad = area.background_padding;
    let expected = 0.5 * acw;
    assert!(
        (pad.left() - expected).abs() < 0.01 && (pad.right() - expected).abs() < 0.01,
        "acw-aligned width should give pad_left == pad_right == 0.5·acw; got left={}, right={}, expected={}",
        pad.left(),
        pad.right(),
        expected
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
    assert!(area.background_padding.is_zero());
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
