// SPDX-License-Identifier: MPL-2.0

//! Macro loaders for the registry's two tiers.
//!
//! The format reference is in `format/macros.md`. Each tier has a
//! pinned `SourceTier`; the registry picks the higher-tier macro by id
//! when collisions happen. Both tiers ship on both native and WASM:
//! - **App** — `assets/macros/application.json`, embedded with
//!   `include_str!`. Cross-platform.
//! - **User** — `~/.config/mandala/macros.json` on native;
//!   `localStorage` under `mandala_macros` on WASM (never a URL
//!   query param — see `platform_web`). Routed via the
//!   `platform_desktop` / `platform_web` sibling modules. Both
//!   are cfg-gated to their target so intra-doc links would
//!   trip "unresolved link" warnings on the inactive one.
//!
//! A map carries no macros: its verbs replace them (design §11.3,
//! V1-D3 #252), so there is no document-derived tier to refresh when
//! a map is loaded.
//!
//! Resilience: app-bundle parses with `expect()` (a malformed bundle
//! is a startup-time bug, not a user input error). Other tiers'
//! parse failures log `warn!` and fall through to an empty slice
//! / skipped entry so the application boots even if user input is
//! broken.

#[cfg(not(target_arch = "wasm32"))]
pub mod platform_desktop;
#[cfg(target_arch = "wasm32")]
pub mod platform_web;

#[cfg(not(target_arch = "wasm32"))]
pub use platform_desktop::load_user_macros;
#[cfg(target_arch = "wasm32")]
pub use platform_web::load_user_macros;

use super::Macro;
use crate::application::source_tier::SourceTier;
use crate::application::user_config::salvage::{parse_array, parse_entries};

/// Application-bundle JSON, embedded at compile time. Parsed by
/// [`load_app_macros`]. Empty array today; future shipped macros
/// land here.
const APP_MACROS_JSON: &str = include_str!("../../../../assets/macros/application.json");

/// Load the application-bundle macros. Tier: `SourceTier::App`,
/// assigned at the call site in `run_native_init::build` and the
/// WASM init block.
///
/// Parses with `expect()` — a malformed bundle is a build-time bug.
/// `format/macros.md` documents the format; the file MUST be a
/// top-level JSON array of macro objects.
pub fn load_app_macros() -> Vec<Macro> {
    let trimmed = APP_MACROS_JSON.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    baumhard::format::json::parse::<Vec<Macro>>(trimmed)
        .expect("malformed assets/macros/application.json — bundle is invalid")
}

/// Parse a user-tier `macros.json` payload: a JSON array of macro
/// objects. Used by both targets' user-tier loaders (the native
/// `platform_desktop` after the capped read; the WASM `platform_web`
/// after the `localStorage` fetch).
///
/// **Per entry, not per file.** Text that is not a JSON array is the
/// whole-layer failure (`Err`, which the shared driver logs before
/// the user tier comes up empty); inside the array, a macro that
/// does not parse is warned about by index and id and skipped, and
/// every other macro loads. A macro is all-or-nothing — one with a
/// step dropped would run a different program under the same id —
/// so the entry, not the step, is the unit. A skipped user macro that
/// was overriding an App-tier macro of the same id leaves the App
/// macro in effect, which is the entry-level reading of "keeps its
/// default". An empty or all-whitespace file is an empty tier.
pub fn parse_user_macros_json(source: &str) -> Result<Vec<Macro>, String> {
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    let values = parse_array("macros", source).map_err(|e| format!("malformed user macros JSON: {e}"))?;
    Ok(parse_entries("macros", "user", &values))
}

/// Build the load-time macro registry: the App and User tiers from
/// the platform loaders.
///
/// This is the whole of what both runtimes do at startup, and both
/// used to do it in their own copy — same loop, same counters, same
/// `log::info!` format string. The logging shape is load-bearing:
/// cross-target log triage compares browser-console output against
/// desktop stderr, so a divergence in the message is a divergence in
/// the tooling.
///
/// Tier precedence is App < User, enforced by insert order plus the
/// registry's last-writer-wins semantics: a user macro shadows an App
/// macro of the same id. `format/macros.md` carries the threat model.
pub fn build_macro_registry() -> super::MacroRegistry {
    let mut macros = super::MacroRegistry::new();
    let mut app_count = 0usize;
    for m in load_app_macros() {
        macros.insert(m, SourceTier::App);
        app_count += 1;
    }
    let mut user_count = 0usize;
    for m in load_user_macros() {
        macros.insert(m, SourceTier::User);
        user_count += 1;
    }
    if app_count > 0 || user_count > 0 {
        log::info!(
            "loaded {} macro(s): {} app-tier, {} user-tier",
            macros.len(),
            app_count,
            user_count
        );
    }
    macros
}

// `load_user_macros` lives in the cfg-routed sibling modules
// `platform_desktop` and `platform_web`; the platform-routed
// `pub use` at the top of this file picks the right one for the
// current target. Both expose the same `pub fn load_user_macros()
// -> Vec<Macro>` signature. Written as a plain comment because a
// `///` here would attach itself to the test module below.

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled `assets/macros/application.json` parses cleanly.
    /// Catches malformed-asset regressions at test time rather than
    /// startup `expect()` panics.
    #[test]
    fn test_app_bundle_parses() {
        let _ = load_app_macros();
    }

    /// `parse_user_macros_json` is the cross-platform parsing seam
    /// the WASM `platform_web::load_user_macros` and the native
    /// `platform_desktop::load_user_macros` both call. Pin its
    /// contract here so the WASM path (which has no headless test
    /// harness — see `TEST_CONVENTIONS.md §T9`) is at least
    /// indirectly covered.
    #[test]
    fn test_parse_user_macros_json_empty_input_returns_empty() {
        assert!(parse_user_macros_json("").unwrap().is_empty());
        assert!(parse_user_macros_json("   ").unwrap().is_empty());
    }

    #[test]
    fn test_parse_user_macros_json_array_round_trips() {
        let source = r#"[
            {"id": "u1", "steps": [{"kind": "Action", "action": "Undo"}]},
            {"id": "u2", "steps": [{"kind": "ConsoleLine", "line": "save"}]}
        ]"#;
        let parsed = parse_user_macros_json(source).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "u1");
        assert_eq!(parsed[1].id, "u2");
    }

    /// One malformed macro in a user's file costs that macro, not the
    /// file (#129): the entries beside it load, and the warning names
    /// the skipped one by index and id.
    #[test]
    fn test_parse_user_macros_json_skips_a_malformed_macro_and_keeps_the_rest() {
        baumhard::util::test_logger::install();
        let source = r#"[
            {"id": "keep-1", "steps": [{"kind": "Action", "action": "Undo"}]},
            {"id": "typo-5e21", "steps": [{"kind": "Acton", "action": "Undo"}]},
            {"id": "keep-2", "steps": [{"kind": "Action", "action": "ZoomReset"}]}
        ]"#;
        let (parsed, captured) = baumhard::util::log::capture::capture(|| parse_user_macros_json(source));
        let ids: Vec<String> = parsed
            .expect("a usable array")
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["keep-1".to_string(), "keep-2".to_string()]);
        let warning = captured
            .records
            .iter()
            .find(|r| r.message.contains("typo-5e21"))
            .unwrap_or_else(|| panic!("the skipped macro must be named: {:?}", captured.records));
        assert!(
            warning
                .message
                .starts_with("user config: macros user entry [1] (id=typo-5e21) failed to parse: "),
            "{}",
            warning.message
        );
        // Control: the same file with the typo fixed loads all three,
        // so the skip above is the typo's doing.
        let fixed = source.replace("Acton", "Action");
        assert_eq!(parse_user_macros_json(&fixed).unwrap().len(), 3);
    }

    /// A field repeated inside a user macro is named by its path and
    /// the last occurrence used — the macro still loads.
    #[test]
    fn test_parse_user_macros_json_names_a_repeated_field_and_keeps_the_macro() {
        baumhard::util::test_logger::install();
        let source = r#"[
            {"id": "first-id", "steps": [{"kind": "Action", "action": "Undo"}], "id": "last-id"}
        ]"#;
        let (parsed, captured) = baumhard::util::log::capture::capture(|| parse_user_macros_json(source));
        let ids: Vec<String> = parsed.unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["last-id".to_string()]);
        let messages: Vec<&str> = captured.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            messages,
            vec!["user config: macros key '[0].id' appears more than once; the last occurrence is used"]
        );
    }

    /// A user file that is not a JSON array has no entries to salvage,
    /// so it is the whole-layer failure, with the shape named.
    #[test]
    fn test_parse_user_macros_json_rejects_a_file_that_is_not_an_array() {
        let err = parse_user_macros_json(r#"{"id": "u1", "steps": []}"#).unwrap_err();
        assert_eq!(
            err,
            "malformed user macros JSON: expected a JSON array, found an object"
        );
    }

    #[test]
    fn test_parse_user_macros_json_malformed_returns_err_not_panic() {
        // User-tier loader on both targets logs the err and falls
        // back to empty rather than panicking; pin the err-not-panic
        // contract here.
        let result = parse_user_macros_json("definitely { not } json");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("malformed user macros JSON"),
            "expected canonical err prefix, got: {}",
            msg
        );
    }
}
