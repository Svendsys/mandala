// SPDX-License-Identifier: MPL-2.0

//! Desktop user-file plumbing for custom mutations: explicit CLI
//! path first, then `$XDG_CONFIG_HOME/mandala/mutations.json` (or
//! the `$HOME/.config` fallback), then nothing. Not compiled on
//! WASM.
//!
//! The read, the size cap, the layer construction, and the fallback
//! walk all belong to [`crate::application::user_config`]; this
//! module only names the file and the parser. The keybinds and
//! macros desktop loaders name theirs the same way against the same
//! wrapper.

use std::path::Path;

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::load_desktop_layered;

/// Load user mutations, with layered fallback: the explicit CLI
/// path first, then `$XDG_CONFIG_HOME/mandala/mutations.json` (or
/// `$HOME/.config/mandala/mutations.json`), then empty. Never fails
/// — a missing or unusable file is logged and the next layer is
/// tried; a malformed mutation inside a usable file is skipped on its
/// own (see [`super::parse_user_mutations_json`]).
pub fn load_user(explicit_path: Option<&Path>) -> Vec<CustomMutation> {
    match load_desktop_layered(
        "mutations",
        "mutations.json",
        explicit_path,
        super::parse_user_mutations_json,
    ) {
        Some((v, source)) => {
            log::info!("loaded {} user mutations from {}", v.len(), source);
            v
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use baumhard::util::test_temp::TempDir;

    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;

    #[test]
    fn test_load_user_missing_file_returns_empty_vec() {
        let p = Path::new("/nonexistent/path/mutations.json");
        let v = load_user(Some(p));
        assert!(v.is_empty());
    }

    #[test]
    fn test_load_user_malformed_file_returns_empty_vec() {
        let scratch = TempDir::new("bad-mutations");
        let tmp = scratch.join("mutations.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty());
    }

    #[test]
    fn test_load_user_oversized_file_is_rejected() {
        let scratch = TempDir::new("oversized-mutations");
        let tmp = scratch.join("mutations.json");
        // Write a 2 MiB file — twice the 1 MiB cap. Content is
        // irrelevant; the rejection happens before serde runs.
        let blob = vec![b' '; MAX_USER_PAYLOAD_BYTES * 2];
        std::fs::write(&tmp, &blob).unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty(), "oversized file must produce an empty result");
    }

    #[test]
    fn test_load_user_valid_file_loads_mutations() {
        let scratch = TempDir::new("good-mutations");
        let tmp = scratch.join("mutations.json");
        let src = r#"{
            "mutations": [{
                "id": "user-mut",
                "name": "User Mutation",
                "mutator": {"Macro": {"kind": null, "channel": 0, "mutations": {"Literal": []}}},
                "target_scope": "SelfOnly"
            }]
        }"#;
        std::fs::write(&tmp, src).unwrap();
        let v = load_user(Some(&tmp));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "user-mut");
    }

    /// The desktop loader parses the user file entry by entry: a bad
    /// mutation beside a good one costs only itself. Held here, not
    /// only on the parser, because a loader handed the strict bundle
    /// parser would pass every parser test and drop the whole file.
    #[test]
    fn test_load_user_skips_a_malformed_entry_and_keeps_the_rest() {
        let scratch = TempDir::new("mixed-mutations");
        let tmp = scratch.join("mutations.json");
        let entry = |id: &str, scope: &str| {
            format!(
                r#"{{"id": "{id}", "name": "{id}",
                    "mutator": {{"Macro": {{"kind": null, "channel": 0, "mutations": {{"Literal": []}}}}}},
                    "target_scope": "{scope}"}}"#
            )
        };
        let src = format!(
            r#"{{"mutations": [{}, {}]}}"#,
            entry("bad", "NoSuchScope"),
            entry("good", "SelfOnly")
        );
        std::fs::write(&tmp, src).unwrap();
        let ids: Vec<String> = load_user(Some(&tmp)).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["good".to_string()]);
    }
}
