// SPDX-License-Identifier: MPL-2.0

//! Input-event value types: keyboard keys, modifiers, mouse
//! buttons, scroll deltas, key/button up-down state.
//!
//! Today every type is a `pub use` of the corresponding
//! `winit::*` type. A future backend swap (SDL, custom WASM
//! event-loop, …) replaces these aliases — every inward caller
//! that imports through this module stays put.

pub use winit::event::ElementState;
/// Mouse-button payload — consumed by native event handlers and the
/// color-picker click flow. WASM's bootstrap (`run_wasm/mod.rs`)
/// imports directly from winit, so this re-export is gated to native.
#[cfg(not(target_arch = "wasm32"))]
pub use winit::event::MouseButton;
/// Touch-phase payload — consumed by the native touch handler and
/// by the IPC `input.touch` command, which delivers it to the same
/// handler. WASM's touch path translates browser events itself.
#[cfg(not(target_arch = "wasm32"))]
pub use winit::event::TouchPhase;
pub use winit::keyboard::Key;
pub use winit::keyboard::ModifiersState as Modifiers;
/// Named-key payload (`NamedKey::Enter` / `Tab` / etc.). Production
/// key handlers pattern-match on `Key::Named(..)` directly; the
/// names are built by the modal-editor test fixtures and by the IPC
/// `input.key` / `input.text` commands, which synthesize the payload
/// winit would deliver.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub use winit::keyboard::NamedKey;

/// Inline-string payload for `Key::Character(...)` — winit reuses
/// `smol_str::SmolStr` here, exposed under the platform name so
/// modal-editor tests can synthesise character payloads without
/// importing the smol-str crate by name. Production input handlers
/// receive the `SmolStr` already wrapped inside a `Key::Character(...)`
/// from winit; the one production builder is the IPC `input.key` /
/// `input.text` pair, which wraps a typed grapheme the way winit
/// would.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub use winit::keyboard::SmolStr;

/// Wheel/trackpad scroll-delta payload. Both targets route through
/// this seam: the shared `wheel_lines` helper in `app/mod.rs` names
/// the delta by this alias, and each target's wheel handler passes
/// its driver's value straight in. Previously WASM-only, when native
/// still decomposed the delta itself inside its driver dispatcher.
pub use winit::event::MouseScrollDelta;
