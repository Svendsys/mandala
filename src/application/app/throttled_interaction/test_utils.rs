use std::time::Duration;
use baumhard::mindmap::model::MindEdge;
use crate::application::frame_throttle::MutationFrequencyThrottle;

/// Push the throttle's average over-budget until `n > 1`. Returns
/// the final drain divisor for assertion plumbing.
pub fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
    for _ in 0..80 {
        if t.should_drain() {
            t.record_work_duration(Duration::from_micros(50_000));
        }
    }
    t.current_n()
}

/// Construct a minimally-valid `MindEdge` for the tests. The drag
/// state only references the snapshot for its pre-drag identity
/// bookkeeping; no field inside it is under test here.
pub fn fixture_edge() -> MindEdge {
    MindEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
        edge_type: "parent_child".to_string(),
        color: "#888888".to_string(),
        width: 4,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}