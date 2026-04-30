// SPDX-License-Identifier: MPL-2.0

//! Web user-source plumbing: URL `?mutations=` query param +
//! `localStorage` fallback. Not compiled on native. Mirrors
//! `keybinds::platform_web` so a future session wiring up web-side
//! write-back has a consistent shape to extend.
//!
//! Query-param parsing, `localStorage` access, and the size-cap
//! guard are delegated to
//! [`crate::application::user_config`] — shared across the three
//! user-tier loaders.

use log::warn;

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::{
    payload_within_cap,
    web_storage::{read_local_storage, read_query_param},
};

/// Load user mutations on WASM, with layered fallback: URL
/// `?mutations=<json>` query param > `localStorage` under the
/// `mandala_mutations` key > empty. Never fails — missing or invalid
/// sources are logged and the next layer is tried.
pub fn load_user() -> Vec<CustomMutation> {
    if let Some(json) = read_query_param("mutations") {
        if payload_within_cap("mutations", "query param", json.len()) {
            match super::parse_mutations_json(&json) {
                Ok(v) => {
                    log::info!("loaded {} user mutations from URL query param", v.len());
                    return v;
                }
                Err(e) => warn!("mutations query param parse failed: {}", e),
            }
        }
    }
    if let Some(json) = read_local_storage("mandala_mutations") {
        if payload_within_cap("mutations", "localStorage value", json.len()) {
            match super::parse_mutations_json(&json) {
                Ok(v) => {
                    log::info!("loaded {} user mutations from localStorage", v.len());
                    return v;
                }
                Err(e) => warn!("mutations localStorage parse failed: {}", e),
            }
        }
    }
    Vec::new()
}
