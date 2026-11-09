// SPDX-License-Identifier: MPL-2.0

//! Camera apply_* helpers — zoom step / reset / fit, the pan nudges,
//! center-on-selection and jump-to-root. A camera change touches no tree
//! and calls no projector (§12.4): it is a renderer decree, drawn at the
//! next frame. Where an arm reads where something is drawn, it asks the
//! projector (`Projector::node_rect`, `Projector::curve`), never a glyph
//! layer.

use crate::application::common::RenderDecree;
use crate::application::document::selection::{child_nodes, kind_of};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;
use baumhard::document::element::DocKind;

use super::ViewContext;

/// Which way a zoom step goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ZoomDir {
    In,
    Out,
}

/// Zoom one step about the cursor: `ZoomIn` / `ZoomOut`.
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

/// Back to zoom 1 about the viewport's center: `ZoomReset`.
pub(in crate::application::app) fn apply_zoom_reset(renderer: &mut Renderer) {
    let zoom = renderer.camera_zoom().max(f32::EPSILON);
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: renderer.surface_width() as f32 * 0.5,
        screen_y: renderer.surface_height() as f32 * 0.5,
        factor: 1.0f32 / zoom,
    });
}

/// Fit everything the map draws in the viewport: `ZoomFit`.
pub(in crate::application::app) fn apply_zoom_fit(renderer: &mut Renderer) {
    renderer.fit_camera_to_content();
}

/// Which way a pan nudge goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PanDir {
    North,
    South,
    East,
    West,
}

/// Pan one step: `PanCamera*`.
pub(in crate::application::app) fn apply_pan_camera(dir: PanDir, renderer: &mut Renderer) {
    const PAN_STEP_PX: f32 = 50.0;
    let (dx, dy) = match dir {
        PanDir::North => (0.0, -PAN_STEP_PX),
        PanDir::South => (0.0, PAN_STEP_PX),
        PanDir::East => (-PAN_STEP_PX, 0.0),
        PanDir::West => (PAN_STEP_PX, 0.0),
    };
    renderer.process_decree(RenderDecree::CameraPan(dx, dy));
}

/// The canvas point the selection centers on: the mean of each selected
/// node's drawn center and each selected path's midpoint, as the
/// projector holds them. `None` when nothing selected is drawn.
pub(in crate::application::app) fn selection_center(
    doc: &MindMapDocument,
    view: &MapView,
) -> Option<glam::Vec2> {
    let projector = view.projector();
    let mut sum = glam::Vec2::ZERO;
    let mut count = 0u32;
    for id in doc.selection.iter() {
        let at = match kind_of(doc.doc(), id) {
            Some(DocKind::Node) => projector.node_rect(id).map(|rect| rect.center()),
            Some(DocKind::Connector | DocKind::Link) => projector.curve(id).map(|curve| curve.point(0.5)),
            _ => None,
        };
        if let Some(at) = at {
            sum += at;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f32)
}

/// Center the camera on the selection: `CenterOnSelection`.
pub(in crate::application::app) fn apply_center_on_selection(
    doc: &MindMapDocument,
    view: &MapView,
    renderer: &mut Renderer,
) {
    if let Some(center) = selection_center(doc, view) {
        renderer.set_camera_center(center);
    }
}

/// Select the first top-level node and center the camera on it:
/// `JumpToRoot`. Nothing for a map with no node drawn.
pub(in crate::application::app) fn apply_jump_to_root(vc: &mut ViewContext<'_>) {
    let doc = vc.document.doc();
    let Some(root) = child_nodes(doc, doc.root()).next() else {
        return;
    };
    let Some(center) = vc.view.projector().node_rect(root).map(|rect| rect.center()) else {
        return;
    };
    vc.document.selection.select_only(root);
    vc.renderer.set_camera_center(center);
    vc.sync();
}
