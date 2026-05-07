// SPDX-License-Identifier: MPL-2.0

//! Window-shape value types: cursor icon, surface size, cursor
//! position. The actual `Window` handle the renderer needs lives
//! at the bootstrap layer (winit-typed) — only the value-shaped
//! companions live here.

pub use winit::dpi::PhysicalPosition;
pub use winit::window::CursorIcon;

/// Surface-size payload — only the WASM resize handler routes
/// through this seam; native handles `WindowEvent::Resized` at
/// its driver dispatcher.
#[cfg(target_arch = "wasm32")]
pub use winit::dpi::PhysicalSize;
