// SPDX-License-Identifier: MPL-2.0

//! Camera / zoom apply_* helpers — keyboard-driven zoom step,
//! reset, fit-to-tree, pan nudges, centre-on-selection, and
//! jump-to-root. The first four are renderer-only (no document
//! mutation, no rebuild). `apply_center_on_selection` reads the
//! document but doesn't mutate. `apply_jump_to_root` is the lone
//! arm that touches both selection and camera, so it routes
//! through the selection-rebuild envelope.

use crate::application::common::RenderDecree;
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::selection::jump_to_root_in;
use super::RebuildContext;

/// Direction of a single keyboard / wheel zoom step. Typed so
/// callers don't have to pass `&Action` and the helper doesn't
/// have to re-match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ZoomDir {
    In,
    Out,
}

/// Step zoom toward `(screen_x, screen_y)` (typically the cursor).
/// The factor mirrors the legacy hardcoded wheel handler (1.1×) so
/// wheel-bound `ZoomIn`/`ZoomOut` behave identically across targets.
pub(in crate::application::app) fn apply_zoom_step(
    dir: ZoomDir,
    cursor_pos: (f64, f64),
    renderer: &mut Renderer,
) {
    let factor = match dir {
        ZoomDir::In => 1.1f32,
        ZoomDir::Out => 1.0f32 / 1.1f32,
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

/// Direction of a single keyboard pan nudge. Typed so callers
/// don't have to pass `&Action` and the helper doesn't have to
/// re-match it. Geographic compass names mirror the
/// `Action::PanCamera*` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PanDir {
    North,
    South,
    East,
    West,
}

/// Keyboard nudge — fixed step in screen pixels, then converted
/// to a `CameraPan` decree like the LeftDrag path emits per cursor
/// move. Step size matches a coarse but perceptible nudge.
pub(in crate::application::app) fn apply_pan_camera(
    dir: PanDir,
    renderer: &mut Renderer,
) {
    const PAN_STEP_PX: f32 = 50.0;
    let (dx, dy) = match dir {
        PanDir::North => (0.0, -PAN_STEP_PX),
        PanDir::South => (0.0, PAN_STEP_PX),
        PanDir::East => (-PAN_STEP_PX, 0.0),
        PanDir::West => (PAN_STEP_PX, 0.0),
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
            sum += node.center_vec2();
            count += 1;
        }
    }
    if count > 0 {
        renderer.set_camera_center(sum / count as f32);
    }
}

/// Select the document's first root node and centre the camera on
/// it. No-op when the document is empty.
pub(in crate::application::app) fn apply_jump_to_root(rc: &mut RebuildContext<'_>) {
    if let Some(centre) = jump_to_root_in(rc.document) {
        rc.renderer.set_camera_center(centre);
        rc.rebuild_after_selection_change();
    }
}
