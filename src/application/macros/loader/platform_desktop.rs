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

use std::path::PathBuf;

use super::Macro;

/// Resolve the user's macros.json path: prefer
/// `$XDG_CONFIG_HOME/mandala/macros.json`, fall back to
/// `~/.config/mandala/macros.json`. Returns `None` when neither
/// `XDG_CONFIG_HOME` nor `HOME` is set (a degenerate environment).
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
