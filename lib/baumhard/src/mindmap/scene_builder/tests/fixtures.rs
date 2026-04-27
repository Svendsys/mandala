// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures for the scene_builder tests. Bodies live in the
//! shared `mindmap::test_helpers` module — this file thins over
//! that canonical surface with the per-call-site signatures the
//! scene_builder tests grew used to (parameterised size +
//! `show_frame`, no parent linkage).

use crate::mindmap::model::{MindEdge, MindMap, MindNode};

pub(super) use crate::mindmap::test_helpers::{
    synthetic_edge, synthetic_portal_edge, testament_map_path as test_map_path,
};

pub(super) fn synthetic_node(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode {
    crate::mindmap::test_helpers::synthetic_node_full(id, None, x, y, w, h, show_frame)
}

pub(super) fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    crate::mindmap::test_helpers::synthetic_map(nodes_vec, edges)
}

pub(super) fn themed_node(id: &str, bg: &str, frame: &str, text: &str) -> MindNode {
    let mut n = synthetic_node(id, 0.0, 0.0, 40.0, 40.0, true);
    n.style.background_color = bg.to_string();
    n.style.frame_color = frame.to_string();
    n.style.text_color = text.to_string();
    n
}

pub(super) fn two_node_edge_map() -> MindMap {
    synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    )
}
