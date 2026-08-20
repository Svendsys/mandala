// SPDX-License-Identifier: MPL-2.0

//! Web user-source plumbing for custom mutations: `localStorage`
//! under the `mandala_mutations` key, then empty. Not compiled on
//! native.
//!
//! **There is no `?mutations=` layer.** The query-param machinery
//! exists in `crate::application::user_config::web_storage` and this
//! loader deliberately does not reach it — see [`load_user`] below
//! for the trust argument, and the `web_storage` module header for
//! the same note against its two siblings.
//!
//! The storage read, the size cap, and the fallback walk all belong
//! to `crate::application::user_config::web_storage` — the keybinds
//! and macros web loaders name their layers against the same driver,
//! so the three cannot drift.

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::web_storage::load_web_storage_only;

/// Load user mutations on WASM: `localStorage` under the
/// `mandala_mutations` key, then empty. Never fails — a missing or
/// invalid source is logged and the default is used.
///
/// **No `?mutations=` layer**, for the reason given on
/// [`load_web_storage_only`]:
/// a query param is owned by whoever composed the link, so it must not
/// reach `SourceTier::User`. A user-tier custom mutation outranks the
/// map's own by precedence and is not screened by the macro-step
/// gates, which govern `MacroStep`s rather than mutations.
pub fn load_user() -> Vec<CustomMutation> {
    match load_web_storage_only("mutations", "mandala_mutations", super::parse_mutations_json) {
        Some((v, source)) => {
            log::info!("loaded {} user mutations from {}", v.len(), source);
            v
        }
        None => Vec::new(),
    }
}
