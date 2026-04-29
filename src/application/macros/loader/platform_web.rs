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

use log::warn;

use super::Macro;

/// Upper bound on a user-macros payload on WASM, in bytes. Real
/// macro files are tiny (a kilobyte at most for typical author
/// workflows); a multi-MB blob is almost certainly accidental or
/// hostile. Rationale matches `mutations_loader::platform_web`.
const MAX_USER_STRING_BYTES: usize = 1 << 20;

/// Load user macros on WASM with layered fallback. Same shape
/// (and same name) as the native sibling so the platform-routed
/// `pub use` at `super::load_user_macros` resolves on both targets.
pub fn load_user_macros() -> Vec<Macro> {
    if let Some(json) = read_from_query() {
        if json.len() > MAX_USER_STRING_BYTES {
            warn!(
                "macros query param exceeds size cap ({} bytes > {} max); skipping",
                json.len(),
                MAX_USER_STRING_BYTES
            );
        } else {
            match super::parse_user_macros_json(&json) {
                Ok(v) => {
                    log::info!("macros: loaded {} user macro(s) from URL query param", v.len());
                    return v;
                }
                Err(e) => warn!("macros: query param parse failed: {}", e),
            }
        }
    }
    if let Some(json) = read_from_local_storage() {
        if json.len() > MAX_USER_STRING_BYTES {
            warn!(
                "macros localStorage value exceeds size cap ({} bytes > {} max); skipping",
                json.len(),
                MAX_USER_STRING_BYTES
            );
        } else {
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

fn read_from_query() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("macros=") {
            let decoded = js_sys::decode_uri_component(val).ok()?;
            return decoded.as_string();
        }
    }
    None
}

fn read_from_local_storage() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item("mandala_macros").ok()?
}
