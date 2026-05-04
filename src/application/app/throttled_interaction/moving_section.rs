// SPDX-License-Identifier: MPL-2.0

//! Throttled drag-to-move state for one section's `offset`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::scene_builder::build_section_resize_handles;
use glam::Vec2;

use crate::application::document::apply_section_drag_delta_and_collect_patches;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_section_resize_handle_tree_from_slice};
use super::{DrainContext, ThrottledInteraction};

/// Per-frame drains mutate the section's tree subtree only; the
/// model is unchanged until release-commit, where
/// `set_section_offset` writes the final offset and pushes a
/// single `EditNodeStyle` undo entry. Drag callers that bypass
/// this discipline and call the setter per-frame would explode
/// the undo stack.
pub(in crate::application::app) struct MovingSectionInteraction {
    pub node_id: String,
    pub section_idx: usize,
    /// Section's `offset` at drag start; release-commit adds
    /// `total_delta` to this and writes the result via
    /// `set_section_offset`.
    pub start_offset: (f64, f64),
    /// Accumulated total delta across the entire drag. Used at
    /// release to compute the new offset from `start_offset`.
    pub total_delta: Vec2,
    /// Delta accumulated since the last successful drain. Folded
    /// into the tree and reset to `Vec2::ZERO` in `drain`.
    pub pending_delta: Vec2,
    /// Per-interaction adaptive throttle.
    pub throttle: MutationFrequencyThrottle,
}

impl MovingSectionInteraction {
    pub(in crate::application::app) fn new(
        node_id: String,
        section_idx: usize,
        start_offset: (f64, f64),
    ) -> Self {
        Self {
            node_id,
            section_idx,
            start_offset,
            total_delta: Vec2::ZERO,
            pending_delta: Vec2::ZERO,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl ThrottledInteraction for MovingSectionInteraction {
    fn has_pending(&self) -> bool {
        self.pending_delta != Vec2::ZERO
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            ..
        } = ctx;

        if let Some(tree) = mindmap_tree.as_mut() {
            let mut patches = Vec::new();
            apply_section_drag_delta_and_collect_patches(
                tree,
                &self.node_id,
                self.section_idx,
                self.pending_delta.x,
                self.pending_delta.y,
                &mut patches,
            );
            renderer.patch_drag_positions(&patches);
            // The section was Section-selected at threshold-cross
            // (otherwise no `MovingSection` promotion), so the
            // selection-gated resize handles render at the
            // pre-drag offset. Refresh their positions in lockstep
            // with the section's tree-side movement; without this
            // the 8 handles freeze in place while the section
            // visibly slides under them.
            if let Some(doc) = document.as_ref() {
                if let Some(node) = doc.mindmap.nodes.get(&self.node_id) {
                    if let Some(section) = node.sections.get(self.section_idx) {
                        let canvas_pos = Vec2::new(
                            node.position.x as f32
                                + section.offset.x as f32
                                + self.total_delta.x,
                            node.position.y as f32
                                + section.offset.y as f32
                                + self.total_delta.y,
                        );
                        let canvas_size = section
                            .size
                            .as_ref()
                            .map(|s| Vec2::new(s.width as f32, s.height as f32));
                        let elements = build_section_resize_handles(
                            &self.node_id,
                            self.section_idx,
                            canvas_pos,
                            canvas_size,
                        );
                        update_section_resize_handle_tree_from_slice(&elements, app_scene);
                    }
                }
            }
            // Container/connections/borders/portals untouched —
            // those anchor to `node.position` which the section
            // drag doesn't change.
            flush_canvas_scene_buffers(app_scene, renderer);
        }

        self.pending_delta = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, trait_default_tests_for_throttled_interaction,
    };

    #[test]
    fn test_new_initialises_fields_with_zero_deltas() {
        let i = MovingSectionInteraction::new("0".to_string(), 1, (10.0, 10.0));
        assert_eq!(i.node_id, "0");
        assert_eq!(i.section_idx, 1);
        assert_eq!(i.start_offset, (10.0, 10.0));
        assert_eq!(i.pending_delta, Vec2::ZERO);
        assert_eq!(i.total_delta, Vec2::ZERO);
        assert_eq!(i.throttle.current_n(), 1);
    }

    #[test]
    fn test_has_pending_false_for_zero_delta() {
        let i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        assert!(!i.has_pending());
    }

    #[test]
    fn test_has_pending_true_for_nonzero_delta() {
        let mut i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        i.pending_delta = Vec2::new(3.0, -2.0);
        assert!(i.has_pending());
    }

    #[test]
    fn test_has_pending_true_for_tiny_nonzero_delta() {
        // Strict `!= ZERO` — a sub-pixel accumulator from one
        // high-frequency cursor tick must still count as pending,
        // because the sum across skipped frames is the contract
        // `drive()` relies on.
        let mut i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        i.pending_delta = Vec2::new(1e-6, 0.0);
        assert!(i.has_pending());
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = MovingSectionInteraction::new("n".into(), 1, (5.0, 7.0));
        i.pending_delta = Vec2::new(11.0, 13.0);
        i.total_delta = Vec2::new(17.0, 19.0);
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        // Pending / total / identity survive — reset is throttle-only.
        assert_eq!(i.pending_delta, Vec2::new(11.0, 13.0));
        assert_eq!(i.total_delta, Vec2::new(17.0, 19.0));
        assert_eq!(i.node_id, "n");
        assert_eq!(i.section_idx, 1);
        assert_eq!(i.start_offset, (5.0, 7.0));
    }

    #[test]
    fn test_throttle_accessor_reaches_owned_instance() {
        let mut i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        drive_throttle_over_budget(i.throttle());
        assert!(i.throttle.current_n() > 1);
    }

    trait_default_tests_for_throttled_interaction! {
        build = || MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0)),
        set_pending = |i: &mut MovingSectionInteraction| {
            i.pending_delta = Vec2::new(1.0, 0.0);
        },
    }
}
