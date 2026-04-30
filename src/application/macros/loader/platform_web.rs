// SPDX-License-Identifier: MPL-2.0

//! Web user-tier macro loader: URL `?macros=<urlencoded-json>`
//! query param > `localStorage` under the `mandala_macros` key >
//! empty. Mirrors `keybinds::platform_web` and
//! `document::mutations_loader::platform_web` so a future session
//! wiring up persistent web-side authoring has a consistent shape
//! to extend.
//!
//! Never panics: missing or invalid sources are logged with `warn!`
//! and the next layer is tried.
//!
//! Query-param parsing, `localStorage` access, and the size-cap
//! guard are delegated to
//! [`crate::application::user_config`] — shared across the three
//! user-tier loaders.

use log::warn;

use super::Macro;
use crate::application::user_config::{
    payload_within_cap,
    web_storage::{read_local_storage, read_query_param},
};

/// Load user macros on WASM with layered fallback. Same shape
/// (and same name) as the native sibling so the platform-routed
/// `pub use` at `super::load_user_macros` resolves on both targets.
pub fn load_user_macros() -> Vec<Macro> {
    if let Some(json) = read_query_param("macros") {
        if payload_within_cap("macros", "query param", json.len()) {
            match super::parse_user_macros_json(&json) {
                Ok(v) => {
                    log::info!("macros: loaded {} user macro(s) from URL query param", v.len());
                    return v;
                }
                Err(e) => warn!("macros: query param parse failed: {}", e),
            }
        }
    }
    if let Some(json) = read_local_storage("mandala_macros") {
        if payload_within_cap("macros", "localStorage value", json.len()) {
            match super::parse_user_macros_json(&json) {
                Ok(v) => {
                    log::info!("macros: loaded {} user macro(s) from localStorage", v.len());
                    return v;
                }
                Err(e) => warn!("macros: localStorage parse failed: {}", e),
            }
        }
    }
    Vec::new()
}
