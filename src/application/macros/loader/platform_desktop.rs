// SPDX-License-Identifier: MPL-2.0

//! Native user-tier macro loader. Reads
//! `$XDG_CONFIG_HOME/mandala/macros.json` (fallback
//! `~/.config/mandala/macros.json`) and parses through the shared
//! [`super::parse_user_macros_json`].
//!
//! Mirrors `keybinds/platform_desktop.rs` and
//! `document/mutations_loader/platform_desktop.rs` — same
//! resilience posture: the app boots with an empty user tier when
//! the file is absent or malformed, and warns on parse failure so
//! the user notices.
//!
//! Path resolution is delegated to
//! [`crate::application::user_config::xdg::xdg_mandala_path`] —
//! shared with the keybinds and mutations loaders.

use super::Macro;
use crate::application::user_config::xdg::xdg_mandala_path;

/// Load the user-layer macros. Tier: `MacroSource::User`, assigned
/// at the call site in `run_native_init::build`.
///
/// Returns an empty `Vec` when the file is absent or malformed;
/// failures log a warning so users notice but the app still boots.
pub fn load_user_macros() -> Vec<Macro> {
    let path = match xdg_mandala_path("macros.json") {
        Some(p) => p,
        None => {
            log::debug!("macros: no HOME / XDG_CONFIG_HOME; user macro file disabled");
            return Vec::new();
        }
    };
    if !path.exists() {
        return Vec::new();
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("macros: failed to read {}: {}", path.display(), e);
            return Vec::new();
        }
    };
    match super::parse_user_macros_json(&text) {
        Ok(v) => {
            if !v.is_empty() {
                log::info!(
                    "macros: loaded {} user macro(s) from {}",
                    v.len(),
                    path.display(),
                );
            }
            v
        }
        Err(e) => {
            log::warn!("macros: {} ({})", e, path.display());
            Vec::new()
        }
    }
}
