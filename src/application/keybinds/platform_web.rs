// SPDX-License-Identifier: MPL-2.0

//! Web (WASM) config-source plumbing: URL `?keybinds=` query param +
//! `localStorage` fallback. Not compiled on native.
//!
//! Query-param parsing and `localStorage` access are delegated to
//! [`crate::application::user_config::web_storage`] — shared with
//! the mutations and macros loaders so all three reach for the same
//! window/storage shape.

use log::warn;

use super::config::KeybindConfig;
use crate::application::user_config::web_storage::{read_local_storage, read_query_param};

impl KeybindConfig {
    /// Load a config on WASM, with layered fallback: URL `?keybinds=<json>`
    /// query param (inline JSON, URL-encoded) > `localStorage` value under
    /// the `mandala_keybinds` key > hardcoded defaults.
    pub fn load_for_web() -> Self {
        if let Some(json) = read_query_param("keybinds") {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from URL query param");
                    return cfg;
                }
                Err(e) => warn!("keybinds query param parse failed: {}", e),
            }
        }
        if let Some(json) = read_local_storage("mandala_keybinds") {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from localStorage");
                    return cfg;
                }
                Err(e) => warn!("keybinds localStorage parse failed: {}", e),
            }
        }
        Self::default()
    }
}
