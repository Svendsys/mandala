// SPDX-License-Identifier: MPL-2.0

//! Window-shape value types: cursor icon, surface size, cursor
//! position. The actual `Window` handle the renderer needs lives
//! at the bootstrap layer (winit-typed) — only the value-shaped
//! companions live here.
//!
//! `#[allow(unused_imports)]`: `PhysicalSize` is only consumed by
//! WASM-gated callers; the export stays unconditional so the
//! platform-shape API is complete on every target.

#![allow(unused_imports)]

pub use winit::dpi::PhysicalPosition;
pub use winit::dpi::PhysicalSize;
pub use winit::window::CursorIcon;
