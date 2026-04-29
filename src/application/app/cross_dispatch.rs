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

// ── FPS overlay ─────────────────────────────────────────────────

/// Toggle the FPS overlay between `Snapshot` and `Off`. Mirrors
/// `fps on` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps(renderer: &mut Renderer) {
    use crate::application::common::FpsDisplayMode;
    let next = match renderer.fps_display_mode() {
        FpsDisplayMode::Snapshot => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Snapshot,
    };
    renderer.set_fps_display(next);
}

/// Toggle the FPS overlay between `Debug` and `Off`. Mirrors
/// `fps debug` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps_debug(renderer: &mut Renderer) {
    use crate::application::common::FpsDisplayMode;
    let next = match renderer.fps_display_mode() {
        FpsDisplayMode::Debug => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Debug,
    };
    renderer.set_fps_display(next);
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

// ── Selection ───────────────────────────────────────────────────

/// Select every visible node — hidden-by-fold descendants are
/// excluded so a follow-up `DeleteSelection` can't silently nuke
/// subtrees the user can't see. Mirrors the click hit-test's
/// fold-aware policy.
pub(in crate::application::app) fn apply_select_all(rc: &mut RebuildContext<'_>) {
    let all_ids: Vec<String> = rc
        .document
        .mindmap
        .nodes
        .values()
        .filter(|n| !rc.document.mindmap.is_hidden_by_fold(n))
        .map(|n| n.id.clone())
        .collect();
    rc.document.selection = SelectionState::from_ids(all_ids);
    rc.rebuild_after_doc_change();
}

/// Clear the selection. No-op when nothing was selected.
pub(in crate::application::app) fn apply_deselect_all(rc: &mut RebuildContext<'_>) {
    if !matches!(rc.document.selection, SelectionState::None) {
        rc.document.selection = SelectionState::None;
        rc.rebuild_after_doc_change();
    }
}

/// Invert the current node selection. Edge / EdgeLabel / Portal*
/// selections are preserved (their `selected_ids()` is empty, so
/// inverting would collapse to "select every visible node" —
/// unintuitive). Hidden-by-fold nodes are filtered for the same
/// reason as `apply_select_all`.
pub(in crate::application::app) fn apply_invert_selection(rc: &mut RebuildContext<'_>) {
    let invertable = matches!(
        rc.document.selection,
        SelectionState::None
            | SelectionState::Single(_)
            | SelectionState::Multi(_)
    );
    if !invertable {
        return;
    }
    let selected: std::collections::HashSet<String> = rc
        .document
        .selection
        .selected_ids()
        .into_iter()
        .map(String::from)
        .collect();
    let inverted: Vec<String> = rc
        .document
        .mindmap
        .nodes
        .values()
        .filter(|n| {
            !selected.contains(&n.id) && !rc.document.mindmap.is_hidden_by_fold(n)
        })
        .map(|n| n.id.clone())
        .collect();
    rc.document.selection = SelectionState::from_ids(inverted);
    rc.rebuild_after_doc_change();
}

/// Walk one step up the hierarchy from a single-node selection.
/// Multi / edge / unselected: no-op.
pub(in crate::application::app) fn apply_select_parent(rc: &mut RebuildContext<'_>) {
    if let SelectionState::Single(nid) = rc.document.selection.clone() {
        if let Some(parent_id) = rc
            .document
            .mindmap
            .nodes
            .get(&nid)
            .and_then(|n| n.parent_id.clone())
        {
            rc.document.selection = SelectionState::Single(parent_id);
            rc.rebuild_after_doc_change();
        }
    }
}

/// Step into the first visible child (id-sorted) of the selected
/// single node. Folded children are skipped — keyboard navigation
/// shouldn't jump into a subtree the user can't see; mirrors the
/// fold-aware click hit-test policy.
pub(in crate::application::app) fn apply_select_child(rc: &mut RebuildContext<'_>) {
    if let SelectionState::Single(nid) = rc.document.selection.clone() {
        let first_child = rc
            .document
            .mindmap
            .children_of(&nid)
            .into_iter()
            .find(|c| !rc.document.mindmap.is_hidden_by_fold(c))
            .map(|c| c.id.clone());
        if let Some(child_id) = first_child {
            rc.document.selection = SelectionState::Single(child_id);
            rc.rebuild_after_doc_change();
        }
    }
}

/// Step to the next or previous visible sibling of the selected
/// single node. `forward = true` walks toward the next sibling;
/// `false` walks back. No-op when the selection isn't a single
/// node, or when no visible neighbour exists in the requested
/// direction.
pub(in crate::application::app) fn apply_select_sibling(
    forward: bool,
    rc: &mut RebuildContext<'_>,
) {
    if let SelectionState::Single(nid) = rc.document.selection.clone() {
        if let Some(target) = sibling_id(&rc.document.mindmap, &nid, forward) {
            rc.document.selection = SelectionState::Single(target);
            rc.rebuild_after_doc_change();
        }
    }
}

/// Find the next or previous visible sibling of `nid` under the
/// same parent (or among root nodes when `nid` is a root). Skips
/// folded entries so keyboard navigation matches the fold-aware
/// click hit-test. Returns `None` when `nid` has no visible
/// neighbour in the requested direction.
fn sibling_id(
    map: &baumhard::mindmap::model::MindMap,
    nid: &str,
    forward: bool,
) -> Option<String> {
    let parent_id = map.nodes.get(nid).and_then(|n| n.parent_id.clone());
    let siblings: Vec<(String, bool)> = match parent_id {
        Some(pid) => map
            .children_of(&pid)
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
        None => map
            .root_nodes()
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
    };
    let idx = siblings.iter().position(|(id, _)| id == nid)?;
    if forward {
        siblings
            .iter()
            .skip(idx + 1)
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    } else {
        siblings
            .iter()
            .take(idx)
            .rev()
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
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
