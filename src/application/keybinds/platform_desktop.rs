// SPDX-License-Identifier: MPL-2.0

//! Desktop config-source plumbing for keybinds: explicit CLI path
//! first, then `$XDG_CONFIG_HOME/mandala/keybinds.json` (or the
//! `$HOME/.config` fallback), then the hardcoded defaults. Not
//! compiled on WASM.
//!
//! The read, the size cap, the layer construction, and the fallback
//! walk all belong to [`crate::application::user_config`]; this
//! module only names the file and the parser. The mutations and
//! macros desktop loaders name theirs the same way against the same
//! wrapper.

use std::path::Path;

use super::config::KeybindConfig;
use crate::application::user_config::load_desktop_layered;

impl KeybindConfig {
    /// Load a config on desktop, with layered fallback: the explicit
    /// CLI path first, then the default user-config path, then the
    /// hardcoded defaults. Never fails — missing or invalid files are
    /// logged and the next layer is tried.
    pub fn load_for_desktop(explicit_path: Option<&Path>) -> Self {
        match load_desktop_layered("keybinds", "keybinds.json", explicit_path, Self::from_json) {
            Some((cfg, source)) => {
                log::info!("loaded keybinds from {}", source);
                cfg
            }
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use baumhard::util::test_temp::TempDir;

    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;

    #[test]
    fn test_load_for_desktop_missing_explicit_path_falls_back_to_defaults() {
        let cfg = KeybindConfig::load_for_desktop(Some(Path::new("/nonexistent/keybinds.json")));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_oversized_file_falls_back_to_defaults() {
        let scratch = TempDir::new("oversized-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, vec![b' '; MAX_USER_PAYLOAD_BYTES + 1]).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_malformed_file_falls_back_to_defaults() {
        let scratch = TempDir::new("bad-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_reads_the_shipped_template_end_to_end() {
        // The template is what a user copies to
        // `~/.config/mandala/keybinds.json`, so the path it has to
        // survive is this one: read from disk, through the size cap
        // and the layered fallback, parsed, resolved. Its `exit_mode`
        // entry spent a release spelled `cancel_mode` — a key the
        // schema had renamed — and every layer here accepted it in
        // silence (issue #32).
        use crate::application::keybinds::{Action, InputContext};

        // Read off the disk, not out of `include_str!`: a key the
        // schema does not have would otherwise be invisible here,
        // because the field it failed to fill falls back to a default
        // that happens to carry the same combo.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/default_keybinds.json");
        let on_disk = std::fs::read_to_string(&path).unwrap();

        // Provenance first, because it is the thing this test is
        // named for. The two assertions at the bottom — the file's
        // `exit_mode` value, and Escape resolving to `ExitMode` —
        // both pass through the built-in `exit_mode` default, which
        // carries the same `Escape`, so on their own they cannot
        // fail on a renamed key; the schema check further down used
        // to be the only assertion here that noticed, and it is a
        // duplicate of
        // `test_shipped_keybinds_template_has_no_unrecognized_keys`
        // with a different source. So the file's own value is proven
        // live: a copy of the real file with `exit_mode` swapped for
        // a combo no default carries, driven through the same loader
        // off disk. A rename puts the swap under a key the schema
        // ignores, the default `Escape` survives untouched, and both
        // halves fail.
        let scratch = TempDir::new("template-exit-mode-provenance");
        let probe_path = scratch.join("keybinds.json");
        let mut template: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&on_disk).unwrap();
        let shipped = template.insert("exit_mode".to_string(), serde_json::json!(["Ctrl+Alt+F9"]));
        assert_eq!(
            shipped,
            Some(serde_json::json!(["Escape"])),
            "the file this loader reads no longer binds Escape under `exit_mode`",
        );
        std::fs::write(&probe_path, serde_json::Value::Object(template).to_string()).unwrap();
        let probed = KeybindConfig::load_for_desktop(Some(probe_path.as_path())).resolve();
        assert_eq!(
            probed.action_for_context(InputContext::Document, "f9", true, false, true),
            Some(Action::ExitMode),
            "the file's `exit_mode` list is not what the loader read",
        );
        assert_eq!(
            probed.action_for_context(InputContext::Document, "escape", false, false, false),
            None,
            "Escape still exits the mode after the file moved `exit_mode` off it, so the binding \
             this test is about comes from the built-in default rather than from disk",
        );

        assert_eq!(
            KeybindConfig::unknown_top_level_keys(&on_disk),
            Vec::<String>::new(),
            "the file this loader reads names a key the schema does not have",
        );

        let cfg = KeybindConfig::load_for_desktop(Some(&path));
        assert_eq!(cfg.exit_mode, vec!["Escape".to_string()]);
        assert_eq!(
            cfg.resolve()
                .action_for_context(InputContext::Document, "escape", false, false, false),
            Some(Action::ExitMode),
        );
    }

    #[test]
    fn test_load_for_desktop_valid_explicit_file_wins() {
        let scratch = TempDir::new("good-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, r#"{"undo": ["Ctrl+Alt+U"]}"#).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(
            cfg.undo,
            vec!["Ctrl+Alt+U".to_string()],
            "the explicit file must win over the defaults"
        );
    }
}
