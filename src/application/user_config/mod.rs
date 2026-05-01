// SPDX-License-Identifier: MPL-2.0

//! Shared loader plumbing for user-tier JSON config files
//! (`keybinds.json`, `mutations.json`, `macros.json`). Each loader
//! has its own native and web platform shim, but the path-resolution,
//! query-param parsing, localStorage access, and payload-size guard
//! are identical across all three; that machinery lives here.
//!
//! Native uses [`xdg::xdg_mandala_path`] to resolve
//! `$XDG_CONFIG_HOME/mandala/<file>.json` (or the `$HOME/.config`
//! fallback); web uses `web_storage::read_query_param` and
//! `web_storage::read_local_storage` for the URL/localStorage
//! lookups (both cfg-gated to wasm32, so plain backticks instead of
//! intra-doc links to keep native cargo doc clean). Both targets
//! share the [`MAX_USER_PAYLOAD_BYTES`] cap and the
//! [`payload_within_cap`] guard so size-cap rejection logs
//! consistently across loaders.

#[cfg(not(target_arch = "wasm32"))]
pub mod xdg;

#[cfg(target_arch = "wasm32")]
pub mod web_storage;

/// Upper bound on a user-tier JSON payload (file or web string),
/// in bytes. Real keybind / mutation / macro files are small —
/// a few KB at the high end. A multi-MB input is almost certainly
/// accidental or hostile, and loading it into memory + running serde
/// over it is wasted work. 1 MiB is generous (~1000x the largest
/// real file we ship).
pub const MAX_USER_PAYLOAD_BYTES: usize = 1 << 20;

/// Verify a user-tier payload is within [`MAX_USER_PAYLOAD_BYTES`].
/// Returns `true` if the payload is safe to parse; `false` and
/// emits a `log::warn!` if oversized so the caller can short-circuit.
/// `label` and `source` are interpolated into the warning so the
/// user sees "macros query param exceeds size cap (...)" rather than
/// a generic message.
///
/// O(1); the call cost is one comparison plus the warning format
/// only on the failing branch.
pub fn payload_within_cap(label: &str, source: &str, byte_len: usize) -> bool {
    if byte_len > MAX_USER_PAYLOAD_BYTES {
        log::warn!(
            "{} {} exceeds size cap ({} bytes > {} max); skipping",
            label,
            source,
            byte_len,
            MAX_USER_PAYLOAD_BYTES,
        );
        false
    } else {
        true
    }
}
