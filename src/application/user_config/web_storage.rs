// SPDX-License-Identifier: MPL-2.0

//! WASM-side `localStorage` access for the user-tier JSON configs.
//!
//! The three browser loaders (`keybinds::platform_web`,
//! `macros::loader::platform_web`,
//! `document::mutations_loader::platform_web`) read their config from
//! one place, `localStorage`, through [`load_web_storage_only`] — the
//! browser peer of `desktop::load_desktop_layered`, running on the
//! same [`super::layered::load_layered`] driver so the size cap and
//! the "absent is silent, broken is logged" policy are the native
//! ones. The function's doc carries the reason there is no URL layer
//! above it: a `?<name>=<json>` parameter belongs to whoever composed
//! the link, not to the user, and these configs run at the trusted
//! `User` tier.

use super::{load_layered, ConfigLayer};

/// Name the `localStorage` layer answers to in log lines.
const LOCAL_STORAGE_SOURCE: &str = "localStorage";

/// The browser's user-tier chain: `localStorage[storage_key]`, then
/// nothing. Returns the parsed value plus the name of the layer that
/// produced it, for the caller's success log; `None` means nothing
/// usable was stored, and any failure has already been logged by the
/// shared driver, so the caller only has to substitute its defaults.
///
/// Costs: one `localStorage` round-trip.
///
/// **A URL query param is not the user's own configuration.** The
/// trusted [`SourceTier::User`](crate::application::source_tier::SourceTier::User)
/// is justified by "the user owns the file" — true of an XDG config
/// on native, and true of `localStorage`, which only the user's own
/// browser session writes. It is *not* true of `?name=<json>`, which
/// is owned by whoever composed the link.
///
/// That distinction did not exist, and on the web the same URL carried
/// both halves of the trust boundary: `?map=` supplied the document at
/// the untrusted `Map` tier while `?macros=` supplied macros at the
/// fully trusted `User` tier — where
/// [`SourceTier::allows_console_line`](crate::application::source_tier::SourceTier::allows_console_line)
/// permits `MacroStep::ConsoleLine`, an arbitrary console verb, and
/// `allows_action` permits every destructive `Action`. One transport,
/// two trust levels, decided by nothing but which parameter name the
/// sender chose.
///
/// So no user-tier config reads the URL, and no URL layer exists to
/// opt into: the argument is strongest for macros, which run console
/// verbs, and holds unchanged for keybinds and mutations, which reach
/// the same `User` tier (each loader states its own case). The blast
/// radius
/// was bounded by the browser sandbox — no filesystem, no process —
/// but "anyone who can send you a link can run editor commands against
/// your open document" is not a boundary worth keeping.
pub fn load_web_storage_only<T>(
    label: &str,
    storage_key: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<(T, &'static str)> {
    let storage = || read_local_storage(storage_key).map(Ok);
    load_layered(
        label,
        [ConfigLayer {
            source: LOCAL_STORAGE_SOURCE,
            fetch: &storage,
        }],
        parse,
    )
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
