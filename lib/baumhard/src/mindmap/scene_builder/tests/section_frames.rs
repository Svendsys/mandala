// SPDX-License-Identifier: MPL-2.0

//! Section-frame emission rules. Pin the gating contract that
//! Plan §3.5 / §4.3 promises: frames appear only in NodeEdit on
//! a multi-section node, only on the active node, and one frame
//! tracks each section's effective AABB.

use std::collections::HashMap;

use super::fixtures::*;
use crate::mindmap::model::{MindSection, Position, Size};
use crate::mindmap::scene_builder::{build_section_frames, SectionFrameElement};

fn three_section_node() -> crate::mindmap::model::MindNode {
    let mut node = synthetic_node("active", 100.0, 200.0, 300.0, 90.0, true);
    // Three stacked sections — each 30 px tall, offset top → bottom.
    node.sections = vec![
        section("alpha", 0.0, 0.0, 300.0, 30.0),
        section("beta", 0.0, 30.0, 300.0, 30.0),
        section("gamma", 0.0, 60.0, 300.0, 30.0),
    ];
    node
}

fn section(text: &str, off_x: f64, off_y: f64, w: f64, h: f64) -> MindSection {
    let mut s = MindSection::new_default(text.into(), vec![]);
    s.offset = Position { x: off_x, y: off_y };
    s.size = Some(Size { width: w, height: h });
    s
}

fn other_node() -> crate::mindmap::model::MindNode {
    synthetic_node("other", 600.0, 200.0, 200.0, 90.0, true)
}

#[test]
fn test_section_frames_default_mode_emits_none() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), None, None);
    assert!(frames.is_empty(), "no NodeEdit target → no frames");
}

#[test]
fn test_section_frames_node_edit_on_multi_section_emits_per_section() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("active"), None);
    assert_eq!(frames.len(), 3, "one frame per section");
    // Frames are emitted in section order.
    assert_eq!(frames[0].section_idx, 0);
    assert_eq!(frames[1].section_idx, 1);
    assert_eq!(frames[2].section_idx, 2);
    // All carry the active node id.
    for f in &frames {
        assert_eq!(f.node_id, "active");
    }
}

#[test]
fn test_section_frames_inactive_node_emits_no_frames() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("active"), None);
    // Only sections of "active" appear; "other" never gets frames.
    assert!(frames.iter().all(|f| f.node_id == "active"));
}

#[test]
fn test_section_frames_single_section_node_skips_frames() {
    let mut node = synthetic_node("solo", 0.0, 0.0, 200.0, 50.0, true);
    node.sections = vec![section("only", 0.0, 0.0, 200.0, 50.0)];
    let map = synthetic_map(vec![node], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("solo"), None);
    assert!(
        frames.is_empty(),
        "single-section nodes skip frames (would duplicate the border)"
    );
}

#[test]
fn test_section_frames_missing_active_node_emits_no_frames() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    // Stale NodeEdit target after a custom mutation deletion.
    let frames = build_section_frames(&map, &HashMap::new(), Some("nonexistent"), None);
    assert!(frames.is_empty(), "missing active node → no frames");
}

#[test]
fn test_section_frames_track_section_aabb() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("active"), None);

    // Section 0 lives at node.position + section.offset = (100, 200).
    let f0 = &frames[0];
    assert!((f0.position.0 - 100.0).abs() < 1e-3, "x = {}", f0.position.0);
    assert!((f0.position.1 - 200.0).abs() < 1e-3, "y = {}", f0.position.1);
    assert!((f0.size.0 - 300.0).abs() < 1e-3, "w = {}", f0.size.0);
    assert!((f0.size.1 - 30.0).abs() < 1e-3, "h = {}", f0.size.1);

    // Section 1 sits below section 0 (offset.y = 30 → y = 230).
    let f1 = &frames[1];
    assert!((f1.position.1 - 230.0).abs() < 1e-3, "y = {}", f1.position.1);
}

#[test]
fn test_section_frames_focused_section_marks_only_matching_idx() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
    );
    assert_eq!(frames.len(), 3);
    assert!(!frames[0].focused);
    assert!(frames[1].focused, "section 1 must be marked focused");
    assert!(!frames[2].focused);
}

/// Focused section pointing at a different node than the active
/// one (selection drift between editor open and rebuild) is
/// silently ignored — every frame stays unfocused.
#[test]
fn test_section_frames_focused_section_owner_mismatch_marks_none() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("other", 0)),
    );
    assert!(frames.iter().all(|f: &SectionFrameElement| !f.focused));
}

#[test]
fn test_section_frames_skip_zero_size_section() {
    let mut node = synthetic_node("active", 0.0, 0.0, 200.0, 200.0, true);
    node.sections = vec![
        section("ok", 0.0, 0.0, 200.0, 100.0),
        // Degenerate zero-height — skipped from frame emission to
        // mirror the `TextElement` skip rule.
        {
            let mut s = MindSection::new_default("bad".into(), vec![]);
            s.offset = Position { x: 0.0, y: 100.0 };
            s.size = Some(Size { width: 200.0, height: 0.0 });
            s
        },
    ];
    let map = synthetic_map(vec![node], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("active"), None);
    assert_eq!(frames.len(), 1, "degenerate section is skipped");
    assert_eq!(frames[0].section_idx, 0);
}

#[test]
fn test_section_frames_uses_selected_edge_color() {
    use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), Some("active"), None);
    for f in &frames {
        assert_eq!(f.color, SELECTION_HIGHLIGHT_HEX);
    }
}
