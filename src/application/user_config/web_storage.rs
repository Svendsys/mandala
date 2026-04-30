// SPDX-License-Identifier: MPL-2.0

//! WASM-side `?<name>=` query-param + `localStorage` access. The
//! three user-tier JSON loaders (keybinds, mutations, macros) all
//! reach for the same shape: pull the named string from the URL,
//! fall back to a localStorage key. Same shape; same machinery.

/// Read a URL query parameter by name, percent-decoding the value.
/// Returns `None` if `web_sys::window()`/location is missing, the
/// query string is empty, the param is absent, or the decode fails.
/// Recognises `?name=value` and `&name=value` (interior).
///
/// Allocates the formatted prefix and the decoded `String` on the
/// success path. O(n) over the query-string length.
pub fn read_query_param(name: &str) -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let trimmed = search.trim_start_matches('?');
    let prefix = format!("{}=", name);
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix(prefix.as_str()) {
            return js_sys::decode_uri_component(val).ok()?.as_string();
        }
    }
    None
}

/// Read a value from the browser's `localStorage` by key. Returns
/// `None` if the storage API is unavailable (cookies disabled,
/// sandboxed iframe, etc.) or the key is unset.
///
/// O(1); one round-trip into the JS storage backend.
pub fn read_local_storage(key: &str) -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item(key).ok()?
}
