// SPDX-License-Identifier: MPL-2.0

//! Color-picker overlay helpers split out of `Renderer`. Wrappers in
//! `Renderer` call these free fns and then
//! `rebuild_overlay_scene_buffers`. The backdrop rect is returned to
//! the wrapper rather than relocated here, so `Renderer` keeps it for
//! the rect-pipeline pass.

use crate::application::color_picker::{ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::color_picker_overlay;
use crate::application::scene_host::{AppScene, OverlayRole};

/// Register the picker overlay tree (build-or-unregister depending
/// on whether geometry+layout are provided) and return the backdrop
/// rect the rect pipeline should paint underneath the glyphs.
///
/// Caller (a thin `Renderer` wrapper) owns:
/// - assigning the returned `Option<(f32,f32,f32,f32)>` to
///   `self.color_picker_backdrop`
/// - calling `self.rebuild_overlay_scene_buffers(app_scene)`
///   afterwards to re-shape the cosmic-text buffers for the new
///   tree.
pub(super) fn prepare_overlay_for_rebuild(
    app_scene: &mut AppScene,
    geometry_and_layout: Option<(&ColorPickerOverlayGeometry, &ColorPickerLayout)>,
) -> Option<(f32, f32, f32, f32)> {
    let Some((geometry, layout)) = geometry_and_layout else {
        app_scene.unregister_overlay(OverlayRole::ColorPicker);
        return None;
    };
    let build = color_picker_overlay::build(geometry, layout);
    app_scene.register_overlay(OverlayRole::ColorPicker, build.tree, glam::Vec2::ZERO);
    build.backdrop
}

/// §B2 mutation path — apply the **layout-phase** delta to the
/// picker overlay tree without rebuilding the arena. Use only
/// when something the layout depends on actually changed (viewport
/// resize, RMB size_scale drag, drag-move repositioning the
/// wheel). Per-frame hover/HSV/chip updates should call
/// [`apply_dynamic_mutator`] instead — same arena, slimmer
/// per-cell delta.
///
/// Caller reshapes buffers afterwards via
/// `Renderer::rebuild_overlay_scene_buffers`.
pub(super) fn apply_layout_mutator(
    app_scene: &mut AppScene,
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) {
    let mutator = color_picker_overlay::build_mutator(geometry, layout);
    app_scene.apply_overlay_mutator(OverlayRole::ColorPicker, &mutator);
}

/// §B2 mutation path — apply the **dynamic-phase** delta to the
/// picker overlay tree. Only per-frame fields (color regions,
/// hover scale, hex text) are written; layout-phase fields stay
/// as the previous layout-mutator wrote them. The per-frame hot
/// path for hover / HSV / chip-focus updates.
pub(super) fn apply_dynamic_mutator(
    app_scene: &mut AppScene,
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) {
    let mutator = color_picker_overlay::build_dynamic_mutator(geometry, layout);
    app_scene.apply_overlay_mutator(OverlayRole::ColorPicker, &mutator);
}
