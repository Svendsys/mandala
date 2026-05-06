// SPDX-License-Identifier: MPL-2.0

//! Input-event value types: keyboard keys, modifiers, mouse
//! buttons, scroll deltas, key/button up-down state.
//!
//! Today every type is a `pub use` of the corresponding
//! `winit::*` type. A future backend swap (SDL, custom WASM
//! event-loop, …) replaces these aliases — every inward caller
//! that imports through this module stays put.
//!
//! `#[allow(unused_imports)]`: some types (`MouseScrollDelta`,
//! `SmolStr`) are only consumed by WASM-gated callers; the
//! exports stay unconditional so the platform-shape API is
//! complete on every target.

#![allow(unused_imports)]

pub use winit::event::ElementState;
pub use winit::event::MouseButton;
pub use winit::event::MouseScrollDelta;
pub use winit::keyboard::Key;
pub use winit::keyboard::ModifiersState as Modifiers;
pub use winit::keyboard::NamedKey;
/// Inline-string payload for `Key::Character(...)` — winit reuses
/// `smol_str::SmolStr` here, exposed under the platform name so
/// modal editors don't import the smol-str crate by name.
pub use winit::keyboard::SmolStr;
