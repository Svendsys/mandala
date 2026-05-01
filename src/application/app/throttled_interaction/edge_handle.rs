// SPDX-License-Identifier: MPL-2.0

//! Throttled interaction for the edge grab-handle drag gesture.
//!
//! The user drags one of a selected edge's handles (anchor,
//! midpoint, or control point). Accumulates canvas-space deltas
//! the same way `MovingNode` does; each drain folds the sum into
//! the edge's model state via
//! [`crate::application::app::edge_drag::apply_edge_handle_drag`]
//! and re-emits only the one dirty edge's geometry.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use glam::Vec2;

use crate::application::document::EdgeRef;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::edge_drag::apply_edge_handle_drag;
use super::super::scene_rebuild::{
    flush_canvas_scene_buffers, update_connection_label_tree, update_connection_tree,
    update_edge_handle_tree, update_portal_tree,
};
use super::{DrainContext, ThrottledInteraction};

/// Drag-to-reshape state for one edge's grab-handle.
pub(in crate::application::app) struct EdgeHandleInteraction {
    pub edge_ref: EdgeRef,
    /// Which handle is being dragged. `Midpoint` is only the
    /// initial kind — after the first drain frame inserts a
    /// fresh control point, this mutates in place to
    /// `ControlPoint(0)` so subsequent frames take the CP path.
    pub handle: baumhard::mindmap::scene_builder::EdgeHandleKind,
    /// Full snapshot of the edge at drag start, consumed by the
    /// release path for the `UndoAction::EditEdge` entry and the
    /// no-op skip check.
    pub original: baumhard::mindmap::model::MindEdge,
    /// Canvas-space handle position at drag start. Used to
    /// recompute the handle's new position from an absolute
    /// cursor location, which avoids accumulating drift on
    /// non-control-point handles.
    pub start_handle_pos: Vec2,
    /// Accumulated delta since drag start.
    pub total_delta: Vec2,
    /// Delta accumulated since the last successful drain.
    pub pending_delta: Vec2,
    pub throttle: MutationFrequencyThrottle,
}

impl EdgeHandleInteraction {
    pub(in crate::application::app) fn new(
        edge_ref: EdgeRef,
        handle: baumhard::mindmap::scene_builder::EdgeHandleKind,
        original: baumhard::mindmap::model::MindEdge,
        start_handle_pos: Vec2,
    ) -> Self {
        Self {
            edge_ref,
            handle,
            original,
            start_handle_pos,
            total_delta: Vec2::ZERO,
            pending_delta: Vec2::ZERO,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl ThrottledInteraction for EdgeHandleInteraction {
    fn has_pending(&self) -> bool {
        self.pending_delta != Vec2::ZERO
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            app_scene,
            renderer,
            scene_cache,
            ..
        } = ctx;

        if let Some(doc) = document.as_mut() {
            let new_handle = apply_edge_handle_drag(
                doc,
                &self.edge_ref,
                self.handle,
                self.start_handle_pos,
                self.total_delta,
            );
            self.handle = new_handle;

            let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(&self.edge_ref);
            scene_cache.invalidate_edge(&edge_key);

            let offsets: HashMap<String, (f32, f32)> = HashMap::new();
            let scene = doc.build_scene_with_cache(&offsets, scene_cache, renderer.camera_zoom());
            update_connection_tree(&scene, app_scene);
            update_edge_handle_tree(&scene, app_scene);
            update_connection_label_tree(&scene, app_scene, renderer);
            update_portal_tree(doc, &offsets, app_scene, renderer);
            flush_canvas_scene_buffers(app_scene, renderer);
        }

        self.pending_delta = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, fixture_edge, trait_default_tests_for_throttled_interaction,
    };
    use baumhard::mindmap::scene_builder::EdgeHandleKind;

    fn fixture_interaction() -> EdgeHandleInteraction {
        EdgeHandleInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            EdgeHandleKind::AnchorFrom,
            fixture_edge(),
            Vec2::new(10.0, 20.0),
        )
    }

    #[test]
    fn test_new_initialises_fields_with_zero_deltas() {
        let i = fixture_interaction();
        assert_eq!(i.edge_ref.from_id, "a");
        assert_eq!(i.edge_ref.to_id, "b");
        assert_eq!(i.edge_ref.edge_type, "parent_child");
        assert_eq!(i.handle, EdgeHandleKind::AnchorFrom);
        assert_eq!(i.start_handle_pos, Vec2::new(10.0, 20.0));
        assert_eq!(i.pending_delta, Vec2::ZERO);
        assert_eq!(i.total_delta, Vec2::ZERO);
        assert_eq!(i.throttle.current_n(), 1);
    }

    #[test]
    fn test_has_pending_false_for_zero_delta() {
        let i = fixture_interaction();
        assert!(!i.has_pending());
    }

    #[test]
    fn test_has_pending_true_for_nonzero_delta() {
        let mut i = fixture_interaction();
        i.pending_delta = Vec2::new(0.0, 3.0);
        assert!(i.has_pending());
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = fixture_interaction();
        i.pending_delta = Vec2::new(1.0, 2.0);
        i.total_delta = Vec2::new(4.0, 5.0);
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        assert_eq!(i.pending_delta, Vec2::new(1.0, 2.0));
        assert_eq!(i.total_delta, Vec2::new(4.0, 5.0));
        assert_eq!(i.start_handle_pos, Vec2::new(10.0, 20.0));
        assert_eq!(i.handle, EdgeHandleKind::AnchorFrom);
    }

    trait_default_tests_for_throttled_interaction! {
        build = fixture_interaction,
        set_pending = |i: &mut EdgeHandleInteraction| {
            i.pending_delta = Vec2::new(2.0, 0.0);
        },
    }

    #[test]
    fn test_handle_variant_round_trips_control_point() {
        // Midpoint is only the initial kind — the drag promotes it to
        // ControlPoint(0) on first drain. The constructor accepts any
        // variant; verify a non-trivial one round-trips through `new`.
        let i = EdgeHandleInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            EdgeHandleKind::Midpoint,
            fixture_edge(),
            Vec2::ZERO,
        );
        assert_eq!(i.handle, EdgeHandleKind::Midpoint);
    }
}
