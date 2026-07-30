// SPDX-License-Identifier: MPL-2.0

//! Desktop user-file plumbing for custom mutations: explicit CLI
//! path first, then `$XDG_CONFIG_HOME/mandala/mutations.json` (or
//! the `$HOME/.config` fallback), then nothing. Not compiled on
//! WASM.
//!
//! The read, the size cap, and the fallback walk itself all belong
//! to [`crate::application::user_config`]; this module only names
//! the layers and the parser. The keybinds and macros desktop
//! loaders name theirs the same way against the same driver.

use std::path::Path;

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::{load_layered, read_capped, xdg::xdg_mandala_path, ConfigLayer};

/// Load user mutations, with layered fallback: explicit CLI path >
/// `$XDG_CONFIG_HOME/mandala/mutations.json` >
/// `$HOME/.config/mandala/mutations.json` > empty. Never fails —
/// missing or invalid files are logged and the next layer is tried.
pub fn load_user(explicit_path: Option<&Path>) -> Vec<CustomMutation> {
    let default_path = xdg_mandala_path("mutations.json");
    let explicit_source = display_or_empty(explicit_path);
    let default_source = display_or_empty(default_path.as_deref());

    let explicit = || explicit_path.map(read_capped);
    // Only the XDG layer checks `exists()` first: an absent default
    // config is the normal case and must stay silent, whereas an
    // explicit `--mutations <path>` that does not resolve is a user
    // error worth a warning.
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
    match load_layered("mutations", layers, super::parse_mutations_json) {
        Some((v, source)) => {
            log::info!("loaded {} user mutations from {}", v.len(), source);
            v
        }
        None => Vec::new(),
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
    fn test_load_user_missing_file_returns_empty_vec() {
        let p = Path::new("/nonexistent/path/mutations.json");
        let v = load_user(Some(p));
        assert!(v.is_empty());
    }

    #[test]
    fn test_load_user_malformed_file_returns_empty_vec() {
        let tmp = std::env::temp_dir().join("mandala_test_bad_mutations.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_user_oversized_file_is_rejected() {
        let tmp = std::env::temp_dir().join("mandala_test_oversized_mutations.json");
        // Write a 2 MiB file — twice the 1 MiB cap. Content is
        // irrelevant; the rejection happens before serde runs.
        let blob = vec![b' '; MAX_USER_PAYLOAD_BYTES * 2];
        std::fs::write(&tmp, &blob).unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty(), "oversized file must produce an empty result");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_user_valid_file_loads_mutations() {
        let tmp = std::env::temp_dir().join("mandala_test_good_mutations.json");
        let src = r#"{
            "mutations": [{
                "id": "user-mut",
                "name": "User Mutation",
                "mutator": {"Macro": {"channel": 0, "mutations": {"Literal": []}}},
                "target_scope": "SelfOnly"
            }]
        }"#;
        std::fs::write(&tmp, src).unwrap();
        let v = load_user(Some(&tmp));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "user-mut");
        let _ = std::fs::remove_file(&tmp);
    }
}
