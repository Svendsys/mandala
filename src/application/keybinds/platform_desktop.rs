// SPDX-License-Identifier: MPL-2.0

//! Desktop config-source plumbing for keybinds: explicit CLI path
//! first, then `$XDG_CONFIG_HOME/mandala/keybinds.json` (or the
//! `$HOME/.config` fallback), then the hardcoded defaults. Not
//! compiled on WASM.
//!
//! The read, the size cap, and the fallback walk itself all belong
//! to [`crate::application::user_config`]; this module only names
//! the layers and the parser. The mutations and macros desktop
//! loaders name theirs the same way against the same driver.

use std::path::Path;

use super::config::KeybindConfig;
use crate::application::user_config::{load_layered, read_capped, xdg::xdg_mandala_path, ConfigLayer};

impl KeybindConfig {
    /// Load a config on desktop, with layered fallback: explicit CLI path
    /// > default user-config path > hardcoded defaults. Never fails —
    /// missing or invalid files are logged and the next layer is tried.
    pub fn load_for_desktop(explicit_path: Option<&Path>) -> Self {
        let default_path = xdg_mandala_path("keybinds.json");
        let explicit_source = display_or_empty(explicit_path);
        let default_source = display_or_empty(default_path.as_deref());

        let explicit = || explicit_path.map(read_capped);
        // Only the XDG layer checks `exists()` first: an absent
        // default config is the normal case and must stay silent,
        // whereas an explicit `--keybinds <path>` that does not
        // resolve is a user error worth a warning.
        let default = || default_path.as_deref().filter(|p| p.exists()).map(read_capped);

        let layers = [
            ConfigLayer {
                source: &explicit_source,
                fetch: &explicit,
            },
            ConfigLayer {
                source: &default_source,
                fetch: &default,
            },
        ];
        match load_layered("keybinds", layers, Self::from_json) {
            Some((cfg, source)) => {
                log::info!("loaded keybinds from {}", source);
                cfg
            }
            None => Self::default(),
        }
    }
}

/// Render a path for the log line, or the empty string when the
/// layer has no path at all — in which case its `fetch` returns
/// `None` and the name is never printed.
fn display_or_empty(path: Option<&Path>) -> String {
    path.map(|p| p.display().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;

    #[test]
    fn test_load_for_desktop_missing_explicit_path_falls_back_to_defaults() {
        let cfg = KeybindConfig::load_for_desktop(Some(Path::new("/nonexistent/keybinds.json")));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_oversized_file_falls_back_to_defaults() {
        let tmp = std::env::temp_dir().join("mandala_test_oversized_keybinds.json");
        std::fs::write(&tmp, vec![b' '; MAX_USER_PAYLOAD_BYTES + 1]).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_for_desktop_malformed_file_falls_back_to_defaults() {
        let tmp = std::env::temp_dir().join("mandala_test_bad_keybinds.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_for_desktop_valid_explicit_file_wins() {
        let tmp = std::env::temp_dir().join("mandala_test_good_keybinds.json");
        std::fs::write(&tmp, r#"{"undo": ["Ctrl+Alt+U"]}"#).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(
            cfg.undo,
            vec!["Ctrl+Alt+U".to_string()],
            "the explicit file must win over the defaults"
        );
        let _ = std::fs::remove_file(&tmp);
    }
}
