// SPDX-License-Identifier: MPL-2.0

//! Camera methods on `Renderer`: framing (`set_camera_center`,
//! `fit_camera_to_content`) and the screen-space ↔ canvas-space
//! mapping every input handler converts through.
//!
//! Hit-testing does **not** live here: the projector answers it
//! (`view::MapView::hit`), from the projection map, and the renderer
//! owns no spatial index.

use baumhard::gfx_structs::run::GlyphRunData;
use glam::Vec2;

use super::Renderer;

/// The margin [`Renderer::fit_camera_to_content`] keeps around the
/// content, as a fraction of the viewport on each side. A capture framed
/// with `fit` uses the same one, so it frames what the load-time fit
/// frames.
pub(crate) const FIT_PADDING_FRACTION: f32 = 0.05;

/// The canvas-space box `(min, max)` around every run in `runs`: what is
/// drawn, frames and connectors included. `None` when there is no run.
/// The extent [`Renderer::fit_camera_to_content`] fits, and the one a
/// `fit`-framed capture frames.
///
/// O(runs), no allocation.
pub(crate) fn content_bounds<'r>(runs: impl IntoIterator<Item = &'r GlyphRunData>) -> Option<(Vec2, Vec2)> {
    let mut bounds: Option<(Vec2, Vec2)> = None;
    for run in runs {
        let (min, size) = run.bounds();
        if !min.is_finite() || !size.is_finite() {
            continue;
        }
        let max = min + size;
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

    /// Fit the camera to show everything the map draws
    /// ([`content_bounds`] over the glyph layers' runs), with
    /// [`FIT_PADDING_FRACTION`] of the viewport held back on each side.
    /// A map that draws nothing leaves the camera alone. A pan and a zoom
    /// touch no tree (§12.4), so nothing follows the fit.
    pub fn fit_camera_to_content(&mut self) {
        if let Some((min, max)) = self.content_bounds() {
            self.camera
                .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                    min,
                    max,
                    padding_fraction: FIT_PADDING_FRACTION,
                });
        }
    }

    /// The canvas box of everything the map draws ([`content_bounds`]).
    pub(crate) fn content_bounds(&self) -> Option<(Vec2, Vec2)> {
        content_bounds(self.layer_renderer.runs())
    }

    /// Where a surface point lies on the canvas under the current
    /// camera.
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
