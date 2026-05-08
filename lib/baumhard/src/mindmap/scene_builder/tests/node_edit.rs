// SPDX-License-Identifier: MPL-2.0

//! NodeEdit-mode visual indicators emitted by the scene builder.
//! Today: inactive-node dimming — when one node is the active
//! NodeEdit target, every other visible node renders chrome + text
//! at half alpha.
//!
//! Section frames + status-bar overlay land in adjacent passes (B.1
//! / B.3) and have their own tests when those passes ship.

use super::fixtures::*;
use crate::mindmap::model::MindSection;
use crate::mindmap::scene_builder::node_pass::INACTIVE_NODE_ALPHA_MULTIPLIER;
use crate::mindmap::scene_builder::{
    build_scene_with_offsets_selection_and_overrides, SceneSelectionContext,
};
use std::collections::HashMap;

/// Parse an `#RRGGBB` / `#RRGGBBAA` hex into the alpha channel as a
/// float in `[0.0, 1.0]`. Lifted into a helper so the assertions
/// below stay short and read like prose.
fn alpha_of(hex: &str) -> f32 {
    let stripped = hex.trim_start_matches('#');
    if stripped.len() == 8 {
        let bytes = u8::from_str_radix(&stripped[6..8], 16).expect("hex digits");
        bytes as f32 / 255.0
    } else {
        // No alpha byte → opaque.
        1.0
    }
}

fn ctx_with_node_edit_for<'a>(active: &'a str) -> SceneSelectionContext<'a> {
    SceneSelectionContext {
        node_edit_for: Some(active),
        ..SceneSelectionContext::default()
    }
}

fn node_with_text(id: &str, x: f64, y: f64, text: &str) -> crate::mindmap::model::MindNode {
    let mut node = synthetic_node(id, x, y, 200.0, 200.0, true);
    node.sections = vec![MindSection::new_default(text.into(), vec![])];
    // Pin a known opaque text color so the half-alpha is unambiguous.
    let grapheme_count = text.chars().count();
    node.sections[0].text_runs = vec![crate::mindmap::model::TextRun {
        start: 0,
        end: grapheme_count,
        bold: false,
        italic: false,
        underline: false,
        font: String::new(),
        size_pt: 14,
        color: "#ffffff".into(),
        hyperlink: None,
    }];
    // Pin the frame color (the resolved border color cascades from
    // `style.frame_color` when no `border.color` override is set) so
    // the assertion has a stable starting alpha to halve.
    node.style.frame_color = "#ff8800".into();
    node
}

#[test]
fn test_node_edit_dim_off_renders_full_alpha() {
    let map = synthetic_map(
        vec![node_with_text("a", 0.0, 0.0, "alpha"), node_with_text("b", 400.0, 0.0, "beta")],
        vec![],
    );
    let scene = build_scene_with_offsets_selection_and_overrides(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        1.0,
    );
    for el in &scene.text_elements {
        for run in &el.text_runs {
            assert!(
                (alpha_of(&run.color) - 1.0).abs() < 1e-3,
                "default mode: {} run #{} alpha = {} (expected 1.0)",
                el.node_id,
                el.section_idx,
                alpha_of(&run.color),
            );
        }
    }
    for b in &scene.border_elements {
        assert!(
            (alpha_of(&b.border_style.color) - 1.0).abs() < 1e-3,
            "default mode: {} border alpha = {} (expected 1.0)",
            b.node_id,
            alpha_of(&b.border_style.color),
        );
    }
}

#[test]
fn test_node_edit_active_node_keeps_full_alpha() {
    let map = synthetic_map(
        vec![node_with_text("a", 0.0, 0.0, "alpha"), node_with_text("b", 400.0, 0.0, "beta")],
        vec![],
    );
    let scene = build_scene_with_offsets_selection_and_overrides(
        &map,
        &HashMap::new(),
        ctx_with_node_edit_for("a"),
        None,
        None,
        1.0,
    );
    let active_text: Vec<_> = scene.text_elements.iter().filter(|e| e.node_id == "a").collect();
    assert!(!active_text.is_empty(), "active node 'a' must emit text");
    for el in &active_text {
        for run in &el.text_runs {
            assert!(
                (alpha_of(&run.color) - 1.0).abs() < 1e-3,
                "active node 'a' run alpha must stay 1.0; got {}",
                alpha_of(&run.color),
            );
        }
    }
    let active_border = scene.border_elements.iter().find(|b| b.node_id == "a");
    if let Some(b) = active_border {
        assert!(
            (alpha_of(&b.border_style.color) - 1.0).abs() < 1e-3,
            "active node border alpha must stay 1.0; got {}",
            alpha_of(&b.border_style.color),
        );
    }
}

#[test]
fn test_node_edit_inactive_node_dims_to_half_alpha() {
    let map = synthetic_map(
        vec![node_with_text("a", 0.0, 0.0, "alpha"), node_with_text("b", 400.0, 0.0, "beta")],
        vec![],
    );
    let scene = build_scene_with_offsets_selection_and_overrides(
        &map,
        &HashMap::new(),
        ctx_with_node_edit_for("a"),
        None,
        None,
        1.0,
    );
    let inactive_text: Vec<_> = scene.text_elements.iter().filter(|e| e.node_id == "b").collect();
    assert!(!inactive_text.is_empty(), "inactive node 'b' must emit text");
    for el in &inactive_text {
        for run in &el.text_runs {
            assert!(
                (alpha_of(&run.color) - INACTIVE_NODE_ALPHA_MULTIPLIER).abs() < 1e-2,
                "inactive node 'b' run alpha must be {} (got {})",
                INACTIVE_NODE_ALPHA_MULTIPLIER,
                alpha_of(&run.color),
            );
        }
    }
    let inactive_border = scene.border_elements.iter().find(|b| b.node_id == "b");
    if let Some(b) = inactive_border {
        assert!(
            (alpha_of(&b.border_style.color) - INACTIVE_NODE_ALPHA_MULTIPLIER).abs() < 1e-2,
            "inactive node 'b' border alpha must be {} (got {})",
            INACTIVE_NODE_ALPHA_MULTIPLIER,
            alpha_of(&b.border_style.color),
        );
    }
}
