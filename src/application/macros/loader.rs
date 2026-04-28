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

/// Parse Map-tier macros out of a loaded document's
/// `mindmap.macros: Vec<serde_json::Value>`. Per-entry parse
/// failures log a `warn!` and are skipped — a malformed Map-tier
/// macro doesn't break document loading. Tier assignment
/// (`MacroSource::Map`) happens at the call site in the document-
/// load path.
///
/// Map-tier macros are stored as untyped JSON in baumhard because
/// the typed `Macro` lives in the application crate (its `Action`
/// enum would otherwise force a circular dependency). The JSON
/// shape matches the User / App tiers — see `format/macros.md`.
pub fn parse_map_macros(values: &[serde_json::Value]) -> Vec<Macro> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, v) in values.iter().enumerate() {
        match serde_json::from_value::<Macro>(v.clone()) {
            Ok(m) => out.push(m),
            Err(e) => {
                // Surface the entry's `id` field in the warning when
                // available — the user / map author addresses macros
                // by id, so "entry [3] (id=save-and-quit)" is a much
                // better diagnostic than "entry [3]" alone. The id
                // may be missing entirely on a malformed entry; fall
                // back to a placeholder.
                let id_hint = v
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("<no id>");
                log::warn!(
                    "macros: Map-tier entry [{}] (id={}) failed to parse: {} (skipping)",
                    idx, id_hint, e
                );
            }
        }
    }
    out
}

/// Refresh the registry's `Map` tier from a loaded document. App
/// and User tiers are untouched — those load once at startup and
/// don't depend on which document is open. Called from:
///
/// 1. `run_native_init::build` after the initial document load.
/// 2. `execute_console_line` when `replace_document` swaps the
///    document via `open` / `new`.
/// 3. *(future)* the WASM document-load path when Phase-9
///    convergence lands.
pub fn rebuild_map_macros(
    registry: &mut super::MacroRegistry,
    doc: &crate::application::document::MindMapDocument,
) {
    registry.clear_tier(super::MacroSource::Map);
    let map_macros = parse_map_macros(&doc.mindmap.macros);
    if !map_macros.is_empty() {
        log::info!("macros: loaded {} Map-tier macro(s)", map_macros.len());
    }
    registry.extend_with_tier(map_macros, super::MacroSource::Map);
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
    use serde_json::json;

    /// The bundled `assets/macros/application.json` parses cleanly.
    /// Catches malformed-asset regressions at test time rather than
    /// startup `expect()` panics.
    #[test]
    fn app_bundle_parses() {
        let _ = load_app_macros();
    }

    /// `parse_map_macros` is best-effort: malformed entries log
    /// `warn!` and skip without breaking the rest of the parse.
    /// Locks the resilience contract documented in the rustdoc.
    #[test]
    fn parse_map_macros_skips_malformed_entries() {
        let values = vec![
            json!({
                "id": "valid",
                "steps": [{"kind": "Action", "action": "Undo"}]
            }),
            json!({
                "id": "missing-steps"
                // missing required `steps` field
            }),
            json!({
                "id": "valid-2",
                "steps": [{"kind": "ConsoleLine", "line": "save"}]
            }),
        ];
        let parsed = parse_map_macros(&values);
        assert_eq!(parsed.len(), 2, "malformed middle entry should be skipped");
        assert_eq!(parsed[0].id, "valid");
        assert_eq!(parsed[1].id, "valid-2");
    }

    #[test]
    fn parse_map_macros_empty_input_returns_empty() {
        let parsed = parse_map_macros(&[]);
        assert!(parsed.is_empty());
    }
}
