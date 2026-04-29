// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `Action` arm bodies.
//!
//! Each function here implements one or more `Action::*` variant
//! bodies in a form callable from BOTH the native dispatcher
//! ([`super::dispatch::dispatch_action`]) and the WASM dispatcher
//! ([`super::run_wasm`]). The split exists because the two
//! dispatchers carry different context types — native has 21 fields
//! including console / picker / app_mode / modifiers; WASM has 9
//! fields, a strict subset. Arms whose bodies touch only the
//! shared subset live here; native-only arms stay in
//! [`super::dispatch`].
//!
//! This is the partial-Track-C path documented in
//! [`WASM_CONVERGENCE.md`]: incrementally lift arm bodies as they
//! turn out to need only cross-platform state, without waiting for
//! a full context-type unification. Each migration removes
//! duplication and the "keep in sync" maintenance tax that mirror
//! arms (Path A1) carry.
//!
//! Helpers take a [`RebuildContext`] when they need the rebuild
//! plumbing, or just `&mut Renderer` for renderer-only operations.
//! Both dispatchers construct the right shape at the call site.

use crate::application::common::RenderDecree;
use crate::application::document::{MindMapDocument, SelectionState};
use crate::application::keybinds::Action;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::scene_rebuild::rebuild_all;

/// Borrowed bundle of the shared rebuild plumbing — the minimum
/// surface every cross-platform mutating Action arm needs.
/// Constructed at the call site from whichever larger context
/// (`InputHandlerContext` on native, `WasmInputState` on WASM)
/// the dispatcher inherits.
pub(in crate::application::app) struct RebuildContext<'a> {
    pub document: &'a mut MindMapDocument,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub app_scene: &'a mut AppScene,
    pub renderer: &'a mut Renderer,
    pub scene_cache: &'a mut SceneConnectionCache,
}

impl<'a> RebuildContext<'a> {
    /// Trigger the same scene-rebuild path the native dispatcher
    /// uses after a document mutation. Clears the connection
    /// sample cache and rebuilds tree + app-scene + renderer
    /// buffers.
    pub fn rebuild_after_doc_change(&mut self) {
        self.scene_cache.clear();
        rebuild_all(
            self.document,
            self.mindmap_tree,
            self.app_scene,
            self.renderer,
            self.scene_cache,
        );
    }
}

// ── Camera / zoom ───────────────────────────────────────────────

/// Step zoom toward `(screen_x, screen_y)` (typically the cursor).
/// The factor mirrors the legacy hardcoded wheel handler (1.1×) so
/// wheel-bound `ZoomIn`/`ZoomOut` behave identically across targets.
pub(in crate::application::app) fn apply_zoom_step(
    action: &Action,
    cursor_pos: (f64, f64),
    renderer: &mut Renderer,
) {
    let factor = match action {
        Action::ZoomIn => 1.1f32,
        Action::ZoomOut => 1.0f32 / 1.1f32,
        _ => return,
    };
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: cursor_pos.0 as f32,
        screen_y: cursor_pos.1 as f32,
        factor,
    });
}

/// Reset zoom to 1.0 anchored at the screen centre (NOT the
/// cursor). A cursor-anchored zoom emits a `CameraZoom` decree
/// whose canvas-position formula shifts the camera when the focus
/// is off-centre — so a Ctrl+0 with the cursor in a corner would
/// scoot the view by 200+ px instead of cleanly resetting in
/// place. Computing the factor inverse against current zoom keeps
/// the multiplicative path; using screen-centre as the focus
/// cancels the position shift algebraically.
pub(in crate::application::app) fn apply_zoom_reset(renderer: &mut Renderer) {
    let zoom = renderer.camera_zoom().max(f32::EPSILON);
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: renderer.surface_width() as f32 * 0.5,
        screen_y: renderer.surface_height() as f32 * 0.5,
        factor: 1.0f32 / zoom,
    });
}

/// Fit the viewport to the loaded tree's bounds. No-op when no
/// tree has been built yet.
pub(in crate::application::app) fn apply_zoom_fit(
    mindmap_tree: &Option<MindMapTree>,
    renderer: &mut Renderer,
) {
    if let Some(tree) = mindmap_tree.as_ref() {
        renderer.fit_camera_to_tree(&tree.tree);
    }
}

/// Keyboard nudge — fixed step in screen pixels, then converted
/// to a `CameraPan` decree like the LeftDrag path emits per cursor
/// move. Step size matches a coarse but perceptible nudge.
pub(in crate::application::app) fn apply_pan_camera(
    action: &Action,
    renderer: &mut Renderer,
) {
    const PAN_STEP_PX: f32 = 50.0;
    let (dx, dy) = match action {
        Action::PanCameraNorth => (0.0, -PAN_STEP_PX),
        Action::PanCameraSouth => (0.0, PAN_STEP_PX),
        Action::PanCameraEast => (-PAN_STEP_PX, 0.0),
        Action::PanCameraWest => (PAN_STEP_PX, 0.0),
        _ => return,
    };
    renderer.process_decree(RenderDecree::CameraPan(dx, dy));
}

/// Centre the camera on the centroid of the currently-selected
/// nodes. No-op when nothing is selected (or only an edge /
/// portal-marker selection, which carries no point centroid).
pub(in crate::application::app) fn apply_center_on_selection(
    document: &MindMapDocument,
    renderer: &mut Renderer,
) {
    let ids: Vec<&str> = document.selection.selected_ids();
    if ids.is_empty() {
        return;
    }
    let mut sum = glam::Vec2::ZERO;
    let mut count = 0u32;
    for id in &ids {
        if let Some(node) = document.mindmap.nodes.get(*id) {
            sum += glam::Vec2::new(
                node.position.x as f32 + node.size.width as f32 * 0.5,
                node.position.y as f32 + node.size.height as f32 * 0.5,
            );
            count += 1;
        }
    }
    if count > 0 {
        renderer.set_camera_center(sum / count as f32);
    }
}

/// Select the document's first root node (id-sorted) and centre
/// on it. No-op when the document is empty.
pub(in crate::application::app) fn apply_jump_to_root(rc: &mut RebuildContext<'_>) {
    let target = rc.document.mindmap.root_nodes().first().map(|n| {
        (
            n.id.clone(),
            glam::Vec2::new(
                n.position.x as f32 + n.size.width as f32 * 0.5,
                n.position.y as f32 + n.size.height as f32 * 0.5,
            ),
        )
    });
    if let Some((id, centre)) = target {
        rc.document.selection = SelectionState::Single(id);
        rc.renderer.set_camera_center(centre);
        rc.rebuild_after_doc_change();
    }
}
