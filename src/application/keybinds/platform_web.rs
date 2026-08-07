// SPDX-License-Identifier: MPL-2.0

//! Web (WASM) config-source plumbing for keybinds: URL `?keybinds=`
//! query param > `localStorage` under the `mandala_keybinds` key >
//! hardcoded defaults. Not compiled on native.
//!
//! The query/storage reads, the size cap, and the fallback walk all
//! belong to `crate::application::user_config::web_storage` — the
//! mutations and macros web loaders name their layers against the
//! same driver, so the three cannot drift.

use super::config::KeybindConfig;
use crate::application::user_config::web_storage::load_web_storage_only;

impl KeybindConfig {
    /// Load a config on WASM: `localStorage` under the
    /// `mandala_keybinds` key, then hardcoded defaults. The payload is
    /// bounded by `MAX_USER_PAYLOAD_BYTES` — an oversized blob is
    /// logged and skipped without invoking serde.
    ///
    /// **No `?keybinds=` layer**, for the reason given on
    /// [`load_web_storage_only`]:
    /// a query param is owned by whoever composed the link, not by the
    /// user, so it must not reach `SourceTier::User`. A keybind config
    /// binds keys directly to `Action`s — including the
    /// filesystem-touching `open_document` / `save_document_as` /
    /// `new_document_at`, which are native-only and therefore inert
    /// here, and `custom_mutation_bindings` / macro bindings, whose
    /// own tier gates still apply. So the reachable harm from this
    /// door is smaller than from `?macros=`; the trust argument is
    /// identical, and a policy that held for one config and not its
    /// siblings is how the gap survived being fixed once.
    pub fn load_for_web() -> Self {
        match load_web_storage_only("keybinds", "mandala_keybinds", Self::from_json) {
            Some((cfg, source)) => {
                log::info!("loaded keybinds from {}", source);
                cfg
            }
            None => Self::default(),
        }
    }
}
