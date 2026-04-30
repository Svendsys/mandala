// SPDX-License-Identifier: MPL-2.0

//! Desktop user-file plumbing: filesystem-based user mutation loading
//! with `$XDG_CONFIG_HOME` / `$HOME/.config` fallback, mirroring the
//! shape of `keybinds::platform_desktop`. Not compiled on WASM.
//!
//! Path resolution and the size-cap constant are delegated to
//! [`crate::application::user_config`] — the same plumbing used by
//! the keybinds and macros loaders.

use log::warn;
use std::path::Path;

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::{xdg::xdg_mandala_path, MAX_USER_PAYLOAD_BYTES};

/// Load user mutations, with layered fallback: explicit CLI path >
/// `$XDG_CONFIG_HOME/mandala/mutations.json` >
/// `$HOME/.config/mandala/mutations.json` > empty. Never fails —
/// missing or invalid files are logged and the next layer is tried.
pub fn load_user(explicit_path: Option<&Path>) -> Vec<CustomMutation> {
    if let Some(p) = explicit_path {
        match read_and_parse(p) {
            Ok(v) => {
                log::info!("loaded {} user mutations from {}", v.len(), p.display());
                return v;
            }
            Err(e) => warn!("mutations load failed for explicit path: {}", e),
        }
    }
    if let Some(default_path) = xdg_mandala_path("mutations.json") {
        if default_path.exists() {
            match read_and_parse(&default_path) {
                Ok(v) => {
                    log::info!(
                        "loaded {} user mutations from {}",
                        v.len(),
                        default_path.display()
                    );
                    return v;
                }
                Err(e) => warn!("mutations load failed for default path: {}", e),
            }
        }
    }
    Vec::new()
}

fn read_and_parse(path: &Path) -> Result<Vec<CustomMutation>, String> {
    // Reject oversized files before reading — `read_to_string`
    // would otherwise allocate a String the size of the entire
    // file and hand it to serde. The cap is shared with the web
    // loader and the other user-tier loaders via
    // `user_config::MAX_USER_PAYLOAD_BYTES`.
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
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    super::parse_mutations_json(&src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_empty_vec() {
        let p = Path::new("/nonexistent/path/mutations.json");
        let v = load_user(Some(p));
        assert!(v.is_empty());
    }

    #[test]
    fn malformed_file_returns_empty_vec_and_warns() {
        let tmp = std::env::temp_dir().join("mandala_test_bad_mutations.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn oversized_file_is_rejected() {
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
    fn valid_file_loads_mutations() {
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
