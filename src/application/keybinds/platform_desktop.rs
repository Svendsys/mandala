// SPDX-License-Identifier: MPL-2.0

//! Desktop config-source plumbing: file-based `KeybindConfig` loading
//! with the `$XDG_CONFIG_HOME` / `$HOME/.config` fallback, plus the
//! layered `load_for_desktop` driver. Not compiled on WASM.
//!
//! Path resolution is delegated to
//! [`crate::application::user_config::xdg::xdg_mandala_path`] —
//! shared with the mutations and macros loaders so all three pick
//! up `mandala/<file>.json` from the same XDG namespace.

use log::warn;
use std::path::Path;

use super::config::KeybindConfig;
use crate::application::user_config::{xdg::xdg_mandala_path, MAX_USER_PAYLOAD_BYTES};

impl KeybindConfig {
    /// Load a config from a file on disk. Desktop-only; WASM users load
    /// via `load_from_web`. Failures return an error string the caller can
    /// log. Files larger than `MAX_USER_PAYLOAD_BYTES` are rejected before
    /// `read_to_string` runs — same posture as the mutations desktop
    /// loader, since a multi-MB keybinds file is almost certainly the
    /// wrong file or hostile content.
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_USER_PAYLOAD_BYTES as u64 => {
                return Err(format!(
                    "{} exceeds size cap ({} bytes > {} max); refusing to load",
                    path.display(),
                    meta.len(),
                    MAX_USER_PAYLOAD_BYTES,
                ));
            }
            Ok(_) => {}
            Err(e) => return Err(format!("stat {}: {}", path.display(), e)),
        }
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        Self::from_json(&json)
    }

    /// Load a config on desktop, with layered fallback: explicit CLI path
    /// > default user-config path > hardcoded defaults. Never fails —
    /// missing or invalid files are logged and the next layer is tried.
    pub fn load_for_desktop(explicit_path: Option<&Path>) -> Self {
        if let Some(p) = explicit_path {
            match Self::load_from_file(p) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from {}", p.display());
                    return cfg;
                }
                Err(e) => warn!("keybinds load failed for explicit path: {}", e),
            }
        }
        if let Some(default_path) = xdg_mandala_path("keybinds.json") {
            if default_path.exists() {
                match Self::load_from_file(&default_path) {
                    Ok(cfg) => {
                        log::info!("loaded keybinds from {}", default_path.display());
                        return cfg;
                    }
                    Err(e) => warn!("keybinds load failed for default path: {}", e),
                }
            }
        }
        Self::default()
    }
}
