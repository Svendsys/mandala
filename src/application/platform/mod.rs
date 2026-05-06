// SPDX-License-Identifier: MPL-2.0

//! Platform-input and platform-window value types — the inward
//! seam between the winit bootstrap (`app/run_native.rs`,
//! `app/run_wasm/`) and the rest of the application.
//!
//! Inward code (event handlers, modal text/label editors, keybind
//! matchers, the keybind parser) reaches `Key`, `Modifiers`,
//! `MouseButton`, `CursorIcon`, etc. through this module rather
//! than importing `winit::*` directly. The bootstrap files keep
//! their `use winit::application::ApplicationHandler` etc. — they
//! own the event-loop driver, which is fundamentally winit-shaped
//! and would be rewritten end-to-end for any backend swap (SDL,
//! a custom WASM driver, …).
//!
//! Today these are type aliases over `winit::*`. The wall is
//! concentrated here: a future swap rewrites this module (plus
//! the bootstrap), and every inward caller is unchanged.

/// Input-event value types — what comes out of a key/mouse press
/// after the bootstrap unwraps the corresponding winit event.
pub mod input;
/// Window-shape value types — cursor/size/position primitives the
/// renderer surface and the cursor-state code reach for.
pub mod window;
