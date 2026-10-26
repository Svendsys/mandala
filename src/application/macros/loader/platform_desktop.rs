// SPDX-License-Identifier: MPL-2.0

//! Native user-tier macro loader. Reads
//! `$XDG_CONFIG_HOME/mandala/macros.json` (fallback
//! `~/.config/mandala/macros.json`) and parses through the shared
//! [`super::parse_user_macros_json`].
//!
//! The read, the size cap, the layer construction, and the fallback
//! walk all belong to [`crate::application::user_config`]; this
//! module only names the file and the parser. The keybinds and
//! mutations desktop loaders name theirs the same way against the
//! same wrapper, which is also where the shared resilience posture
//! lives: the app boots with an empty user tier when the file is
//! absent or not a JSON array, skips (and names) a malformed macro
//! inside a usable file, and warns on every failure so the user
//! notices.

use super::Macro;
use crate::application::user_config::load_desktop_layered;

/// Load the user-layer macros. Tier: `SourceTier::User`, assigned
/// at the call site in `run_native_init::build`.
///
/// Macros have no CLI override, so the explicit layer is `None` and
/// the XDG path is the only candidate — the one place this loader
/// differs from its keybinds / mutations peers.
///
/// Returns an empty `Vec` when the file is absent, oversized, or not a
/// JSON array, and every macro but the malformed ones when some
/// entries do not parse; failures log a warning so users notice but
/// the app still boots. Files larger than `MAX_USER_PAYLOAD_BYTES` are
/// rejected before `read_to_string` runs.
pub fn load_user_macros() -> Vec<Macro> {
    match load_desktop_layered("macros", "macros.json", None, super::parse_user_macros_json) {
        Some((v, source)) => {
            if !v.is_empty() {
                log::info!("macros: loaded {} user macro(s) from {}", v.len(), source);
            }
            v
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use baumhard::util::test_temp::TempDir;

    use crate::application::user_config::test_env::with_env;

    /// The desktop loader parses `macros.json` entry by entry: a
    /// malformed macro beside a good one costs only itself. Held here
    /// rather than only on `parse_user_macros_json`, because a loader
    /// handed a whole-file parser would pass every parser test.
    #[test]
    fn test_load_user_macros_skips_a_malformed_macro_and_keeps_the_rest() {
        let home = TempDir::new("xdg-macros-mixed");
        let dir = home.join("mandala");
        std::fs::create_dir_all(&dir).unwrap();
        let root = home.path().display().to_string();
        let write = |source: &str| std::fs::write(dir.join("macros.json"), source).unwrap();

        with_env(Some(&root), None, || {
            write(
                r#"[
                    {"id": "broken", "steps": [{"kind": "Acton", "action": "Undo"}]},
                    {"id": "fine", "steps": [{"kind": "Action", "action": "Undo"}]}
                ]"#,
            );
            let ids: Vec<String> = load_user_macros().into_iter().map(|m| m.id).collect();
            assert_eq!(ids, vec!["fine".to_string()]);

            // Control: a file that is not an array at all is the
            // whole-layer failure, so the tier comes up empty — the
            // `fine` above was salvaged, not merely tolerated.
            write(r#"{"id": "fine", "steps": [{"kind": "Action", "action": "Undo"}]}"#);
            assert!(load_user_macros().is_empty());
        });
    }
}
