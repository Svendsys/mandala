// SPDX-License-Identifier: MPL-2.0

//! View / overlay-toggle apply_* helpers — zoom-window edits on
//! the selection (`SetZoomWindow`, `ClearZoom`) plus the renderer-
//! side FPS overlay toggles (`ToggleFps`, `ToggleFpsDebug`). Zoom-
//! window edits go through the shared `apply_with_rebuild`
//! envelope; the FPS toggles are renderer-only and rebuild nothing.

use crate::application::renderer::Renderer;

use super::{apply_with_rebuild, RebuildContext};

pub(in crate::application::app) fn apply_set_zoom_window(
    min: crate::application::document::OptionEdit<f32>,
    max: crate::application::document::OptionEdit<f32>,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(doc, min, max)
    });
}

pub(in crate::application::app) fn apply_clear_zoom(rc: &mut RebuildContext<'_>) {
    use crate::application::document::OptionEdit;
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(
            doc,
            OptionEdit::Clear,
            OptionEdit::Clear,
        )
    });
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
