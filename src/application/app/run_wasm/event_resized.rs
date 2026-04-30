// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::Resized` arm. Forwards the new physical size to
//! the renderer as a `SetSurfaceSize` decree. WASM has no
//! color-picker overlay rebuild path (color-picker is native-only),
//! so the body is a single decree dispatch.

#![cfg(target_arch = "wasm32")]

use winit::dpi::PhysicalSize;

use crate::application::common::RenderDecree;

impl super::WasmApp {
    pub(super) fn handle_resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(renderer) = self.renderer.borrow_mut().as_mut() {
            renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));
        }
    }
}
