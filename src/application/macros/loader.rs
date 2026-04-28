// SPDX-License-Identifier: MPL-2.0

//! User-file macro loader.
//!
//! Native-only. Reads `~/.config/mandala/macros.json` (or the path
//! pointed at by `$XDG_CONFIG_HOME/mandala/macros.json`) into a
//! `Vec<Macro>`. Failures log `warn!` and return an empty slice — the
//! application boots even when the user macro file is malformed,
//! same resilience posture as the mutation loader.
//!
//! App-bundle macros (parallel to `assets/mutations/application.json`)
//! and inline-on-map macros (parallel to `MindMap::custom_mutations`)
//! are not yet implemented; this loader covers the user layer only.
//! The registry hands out one merged slice; future layers append in
//! ascending precedence so an inline macro overrides a user macro
//! overrides an app macro by id.
//!
//! See `format/macros.md` for the on-disk format (TODO: not yet written).

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use super::Macro;

/// Resolve the user's macros.json path on native: prefer
/// `$XDG_CONFIG_HOME/mandala/macros.json`, fall back to
/// `~/.config/mandala/macros.json`.
fn user_macros_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mandala").join("macros.json"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("mandala").join("macros.json"))
}

/// Load the user-layer macros. Returns an empty `Vec` when the file
/// is absent or malformed; failures log a warning so users notice but
/// the app still boots.
pub fn load_user_macros() -> Vec<Macro> {
    let path = match user_macros_path() {
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
    match serde_json::from_str::<Vec<Macro>>(&text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("macros: failed to parse {}: {}", path.display(), e);
            Vec::new()
        }
    }
}
