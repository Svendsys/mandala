// SPDX-License-Identifier: MPL-2.0

//! Configurable keybindings.
//!
//! Hardcoded defaults overlaid with JSON: desktop reads
//! `--keybinds <path>` or `$XDG_CONFIG_HOME/mandala/keybinds.json`;
//! WASM reads `?keybinds=<json>` or `localStorage["mandala_keybinds"]`.
//! Failures log and skip the layer — the app never crashes for a bad
//! keybinds file. Partial configs work via serde's `default`
//! attribute.

mod action;
mod bind;
mod config;
mod context;
mod resolved;

#[cfg(not(target_arch = "wasm32"))]
mod platform_desktop;
#[cfg(target_arch = "wasm32")]
mod platform_web;

#[cfg(test)]
mod tests;

pub use action::Action;
// Public surface; the lint can't see in-crate test consumers without --tests.
#[allow(unused_imports)]
pub use bind::{key_to_name, normalize_key_name, KeyBind};
pub use config::KeybindConfig;
pub use context::InputContext;
pub use resolved::ResolvedKeybinds;

