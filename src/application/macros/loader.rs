// SPDX-License-Identifier: MPL-2.0

//! Macro loaders for the four-tier registry.
//!
//! The format reference is in `format/macros.md`. Each tier has a
//! pinned `MacroSource`; the registry picks the highest-tier macro
//! by id when collisions happen. Today only App and User tiers are
//! wired — Map / Inline (`MindMap::macros` / `MindNode::inline_macros`)
//! are deferred per `TODO.md`.
//!
//! Resilience: app-bundle parses with `expect()` (a malformed bundle
//! is a startup-time bug, not a user input error). User-tier
//! parsing failures log `warn!` and fall through to an empty slice
//! so the application boots even if the user file is broken.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use super::Macro;

/// Application-bundle JSON, embedded at compile time. Parsed by
/// [`load_app_macros`]. Empty array today; future shipped macros
/// land here.
const APP_MACROS_JSON: &str = include_str!("../../../assets/macros/application.json");

/// Load the application-bundle macros. Tier: `MacroSource::App`,
/// assigned at the call site in `run_native_init::build`.
///
/// Parses with `expect()` — a malformed bundle is a build-time bug.
/// `format/macros.md` documents the format; the file MUST be a
/// top-level JSON array of macro objects.
pub fn load_app_macros() -> Vec<Macro> {
    let trimmed = APP_MACROS_JSON.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<Macro>>(trimmed)
        .expect("malformed assets/macros/application.json — bundle is invalid")
}

/// Resolve the user's macros.json path on native: prefer
/// `$XDG_CONFIG_HOME/mandala/macros.json`, fall back to
/// `~/.config/mandala/macros.json`.
fn user_macros_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mandala").join("macros.json"));
        }
    }
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty())?;
    Some(PathBuf::from(home).join(".config").join("mandala").join("macros.json"))
}

/// Load the user-layer macros. Tier: `MacroSource::User`, assigned
/// at the call site in `run_native_init::build`.
///
/// Returns an empty `Vec` when the file is absent or malformed;
/// failures log a warning so users notice but the app still boots.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled `assets/macros/application.json` parses cleanly.
    /// Catches malformed-asset regressions at test time rather than
    /// startup `expect()` panics.
    #[test]
    fn app_bundle_parses() {
        let _ = load_app_macros();
    }
}
