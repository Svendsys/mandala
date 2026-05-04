// SPDX-License-Identifier: MPL-2.0

//! Throttled interaction for the section drag gesture — drag-to-
//! move a single section's `offset` relative to its owning node.
//! Mirrors `MovingNodeInteraction`'s shape; the only differences
//! are: targets one section (not a node-set), uses
//! `apply_section_drag_delta_and_collect_patches` (tree-only,
//! per-frame), and commits via `set_section_offset` at release.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::document::apply_section_drag_delta_and_collect_patches;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::scene_rebuild::flush_canvas_scene_buffers;
use super::{DrainContext, ThrottledInteraction};

/// Drag-to-move state for one section of one node. Per-frame
/// drains apply the accumulated `pending_delta` to the section's
/// tree subtree (text and structural children) and patch the
/// renderer's buffers in place; the model is unchanged until
/// release-commit, where `set_section_offset` writes the final
/// offset and pushes a single `EditNodeStyle` undo entry.
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
            // Section drag doesn't move the owning node's container,
            // so connections / borders / portals don't need a
            // scene-cache rebuild — those are anchored to the
            // node's `position`, which is unchanged. Section text
            // buffer position is patched in place above.
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
