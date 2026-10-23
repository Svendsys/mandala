// SPDX-License-Identifier: MPL-2.0

//! Camera methods on `Renderer`: framing (`set_camera_center`,
//! `fit_camera_to_tree`) and the screen-space ↔ canvas-space
//! mapping every input handler converts through.
//!
//! Hit-testing does **not** live here. Canvas content is
//! hit-tested against the very trees that produced the pixels,
//! through
//! [`AppScene`](crate::application::scene_host::AppScene) and
//! Baumhard's per-tree BVH; the renderer owns no spatial index.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::Renderer;

/// The margin [`Renderer::fit_camera_to_tree`] keeps around the content,
/// as a fraction of the viewport on each side. A capture framed with
/// `fit` uses the same one, so it frames what the load-time fit frames.
pub(crate) const FIT_PADDING_FRACTION: f32 = 0.05;

/// The canvas-space box `(min, max)` around every glyph area in `tree`:
/// each area's `position` and `render_bounds`. `None` when the tree has
/// no glyph area. The extent [`Renderer::fit_camera_to_tree`] fits, and
/// the one a `fit`-framed capture frames.
///
/// O(tree elements), no allocation.
pub(crate) fn content_bounds(tree: &Tree<GfxElement, GfxMutator>) -> Option<(Vec2, Vec2)> {
    let mut bounds: Option<(Vec2, Vec2)> = None;
    for descendant_id in tree.root().descendants(&tree.arena) {
        let Some(area) = tree.arena.get(descendant_id).and_then(|n| n.get().glyph_area()) else {
            continue;
        };
        let min = Vec2::new(area.position.x.0, area.position.y.0);
        let max = min + Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0);
        bounds = Some(match bounds {
            None => (min, max),
            Some((lo, hi)) => (lo.min(min), hi.max(max)),
        });
    }
    bounds
}

impl Renderer {
    /// Pan the camera so `target` (canvas coordinates) is centered
    /// on the viewport at the current zoom. Used by the portal
    /// double-click handler to jump to the other side of a portal
    /// edge. Pure pan — no dirty flag raised; the shader transform
    /// plus render-time `visible_at` handle the new view.
    pub fn set_camera_center(&mut self, target: Vec2) {
        self.camera
            .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::SetPosition {
                canvas_pos: target,
            });
    }

    /// Fit the camera to show a Baumhard tree's content
    /// ([`content_bounds`]), with [`FIT_PADDING_FRACTION`] of the
    /// viewport held back on each side. A tree with no glyph areas
    /// leaves the camera alone.
    pub fn fit_camera_to_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        if let Some((min, max)) = content_bounds(tree) {
            self.camera
                .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                    min,
                    max,
                    padding_fraction: FIT_PADDING_FRACTION,
                });
            // The fit typically changes both pan and zoom. The load
            // paths follow it with a full connection rebuild against
            // the new zoom; the `ZoomFit` action
            // (`cross_dispatch::apply_zoom_fit`) does not, and relies
            // on this flag for the scene-cache flush + rebuild on the
            // next frame instead of leaving stale samples behind.
            self.connection_geometry_dirty = true;
        }
    }

    pub fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2 {
        self.camera.screen_to_canvas(Vec2::new(screen_x, screen_y))
    }

    /// The inverse of [`Self::screen_to_canvas`]: where a canvas
    /// point lands on the surface under the current camera.
    // Native-only consumer: the IPC command target places a synthetic
    // pointer given in canvas space.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn canvas_to_screen(&self, canvas: Vec2) -> Vec2 {
        self.camera.canvas_to_screen(canvas)
    }

    /// Size of one screen pixel in canvas units — used to convert
    /// screen-space tolerances (e.g. click tolerance) to canvas-space
    /// distances that stay visually consistent across zoom.
    // Native-driver-only: screen-space hit tolerances are converted
    // to canvas units only by the native mouse handlers.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn canvas_per_pixel(&self) -> f32 {
        if self.camera.pixels_per_unit() > f32::EPSILON {
            1.0 / self.camera.pixels_per_unit()
        } else {
            1.0
        }
    }
}
