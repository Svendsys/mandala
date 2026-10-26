// SPDX-License-Identifier: MPL-2.0

//! Web user-tier macro loader: `localStorage` under the
//! `mandala_macros` key, then empty. Not compiled on native.
//!
//! **No query-param layer, deliberately.** Macros are the one user
//! config that executes: `MacroStep::ConsoleLine` runs an arbitrary
//! console verb, and the `User` tier this loader assigns is what
//! `SourceTier::allows_console_line` passes. A `?macros=` payload
//! comes from whoever composed the URL, so honoring it made a link
//! into arbitrary editor-command execution against the recipient's
//! open document — while `?map=` on the same URL was correctly
//! treated as untrusted. See `user_config::web_storage::load_web_storage_only`.
//!
//! The storage read, the size cap, and the fallback walk all
//! belong to `crate::application::user_config::web_storage` — the
//! keybinds and mutations web loaders name their layers against the
//! same driver, so the three cannot drift. Never panics: a missing
//! payload, or one that is not a JSON array, is logged by the driver
//! and the registry gets no user macros; a malformed macro inside a
//! usable payload is skipped on its own by the shared
//! `parse_user_macros_json`, exactly as on desktop.

use super::Macro;
use crate::application::user_config::web_storage::load_web_storage_only;

/// Load user macros on WASM from `localStorage`. Same shape
/// (and same name) as the native sibling so the platform-routed
/// `pub use` at `super::load_user_macros` resolves on both targets.
pub fn load_user_macros() -> Vec<Macro> {
    match load_web_storage_only("macros", "mandala_macros", super::parse_user_macros_json) {
        Some((v, source)) => {
            if !v.is_empty() {
                log::info!("macros: loaded {} user macro(s) from {}", v.len(), source);
            }
            v
        }
        None => Vec::new(),
    }
}
