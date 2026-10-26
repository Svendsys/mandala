// SPDX-License-Identifier: MPL-2.0

//! Four-source mutation loader: application bundle, user file, map,
//! inline-on-node — in ascending precedence.
//!
//! **Source-of-truth note:** the precedence order (App < User < Map
//! < Inline) is [`crate::application::source_tier::SourceTier`],
//! shared with the macro registry and pinned by that module's tests.
//! `format/mutations.md` documents the same ladder under "Where
//! mutations come from" for authors;
//! [`crate::application::document::MindMapDocument::build_mutation_registry_with_app_and_user`]
//! walks it.
//!
//! This module owns the outer loader (app + user slices produced in
//! `load_app_and_user`), the app-bundle parser ([`builtin`]), and the
//! platform-split user-file readers (`platform_desktop` /
//! `platform_web`). The two readers are cfg-gated to their target
//! and cargo doc would warn under "unresolved link" for the inactive
//! one if they were intra-doc links.
//!
//! Resilience posture (§9): every layer is best-effort. Failures log
//! `warn!` and fall through to the next source; the app never crashes
//! on a bad user file. The application bundle, by contrast, is a
//! build-time invariant and is parsed with `expect()` — a malformed
//! bundle is a startup-time bug, not a user input error.

pub mod builtin;

#[cfg(not(target_arch = "wasm32"))]
pub mod platform_desktop;
#[cfg(target_arch = "wasm32")]
pub mod platform_web;

use baumhard::format::json::Value;
use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::salvage::{parse_entries, parse_object};

/// Load the two slices the registry builder expects — application
/// mutations (bundled with the binary) and user mutations (from the
/// local config file on native; from `localStorage` on WASM).
///
/// Map and inline mutations are read from the `MindMapDocument` itself
/// at merge time, not here.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_app_and_user(
    explicit_user_path: Option<&std::path::Path>,
) -> (Vec<CustomMutation>, Vec<CustomMutation>) {
    let app = builtin::load_app();
    let user = platform_desktop::load_user(explicit_user_path);
    (app, user)
}

/// WASM companion — reads the user slice from `localStorage` instead
/// of the filesystem (never from a URL query param — see
/// `platform_web::load_user`).
#[cfg(target_arch = "wasm32")]
pub fn load_app_and_user() -> (Vec<CustomMutation>, Vec<CustomMutation>) {
    let app = builtin::load_app();
    let user = platform_web::load_user();
    (app, user)
}

/// Parse a JSON string carrying a top-level `{"mutations": [...]}`
/// shape, **strictly**: one malformed mutation fails the whole parse.
/// That is the right answer for the application bundle, a build-time
/// artifact whose every entry must load (`builtin::load_app`
/// `expect`s it), and for test fixtures. A user's file goes through
/// [`parse_user_mutations_json`] instead, which skips a broken entry
/// rather than losing the file.
pub fn parse_mutations_json(source: &str) -> Result<Vec<CustomMutation>, String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(default)]
        mutations: Vec<CustomMutation>,
    }
    baumhard::format::json::parse::<Envelope>(source).map(|e| e.mutations)
}

/// Parse a user-tier `mutations.json` payload — the same
/// `{"mutations": [...]}` envelope as [`parse_mutations_json`] — one
/// entry at a time. Used by both targets' user loaders.
///
/// **Per entry, not per file.** Text that is not a JSON object, or a
/// `mutations` value that is not an array, is the whole-layer failure:
/// there is no list to salvage, so this returns `Err` and the shared
/// driver logs it. Inside the list, a mutation that does not parse is
/// warned about by index and id and skipped, and every other mutation
/// loads. A mutation is all-or-nothing — one with a command dropped
/// is a different edit under the same id — so the entry is the unit.
/// A skipped user mutation that was overriding an App-tier mutation of
/// the same id leaves the App one in effect. A missing `mutations`
/// key is an empty list, as in the strict parser.
///
/// The envelope's only key is `mutations`; any other (a `"mutation"` or
/// `"Mutations"` typo, most likely, which would otherwise load as an
/// empty tier) is warned about and ignored, as `keybinds.json` treats
/// a key it does not recognize. Keys starting with `_` are comments.
pub fn parse_user_mutations_json(source: &str) -> Result<Vec<CustomMutation>, String> {
    let mut envelope = parse_object("mutations", source)?;
    let mutations = envelope.remove("mutations");
    for key in envelope.keys().filter(|key| !key.starts_with('_')) {
        log::warn!("mutations: unrecognized key '{key}' in the user mutations file — ignored; only `mutations` is read");
    }
    match mutations {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(parse_entries("mutations", "user", &values)),
        Some(_) => Err("expected `mutations` to be a JSON array".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_envelope_yields_empty_vec() {
        let v = parse_mutations_json(r#"{"mutations": []}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_missing_mutations_key_treated_as_empty() {
        let v = parse_mutations_json(r#"{}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_malformed_json_reports_error() {
        let err = parse_mutations_json("{ not json").unwrap_err();
        assert!(!err.is_empty());
    }

    /// A mutation as the user file carries it, with `id` and a body
    /// that either parses or does not.
    fn mutation_json(id: &str, target_scope: &str) -> String {
        format!(
            r#"{{"id": "{id}", "name": "{id}",
                "mutator": {{"Macro": {{"kind": null, "channel": 0, "mutations": {{"Literal": []}}}}}},
                "target_scope": "{target_scope}"}}"#
        )
    }

    /// One malformed mutation in a user's file costs that mutation,
    /// not the file (#129); the bundle's strict parser still refuses
    /// the same text, so a broken bundle entry cannot slip through.
    #[test]
    fn test_parse_user_mutations_json_skips_a_malformed_entry_and_keeps_the_rest() {
        baumhard::util::test_logger::install();
        let source = format!(
            r#"{{"mutations": [{}, {}, {}]}}"#,
            mutation_json("keep-1", "SelfOnly"),
            mutation_json("bad-scope-81f0", "NoSuchScope"),
            mutation_json("keep-2", "SelfOnly"),
        );
        let (parsed, captured) = baumhard::util::log::capture::capture(|| parse_user_mutations_json(&source));
        let ids: Vec<String> = parsed
            .expect("a usable envelope")
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["keep-1".to_string(), "keep-2".to_string()]);
        let warning = captured
            .records
            .iter()
            .find(|r| r.message.contains("bad-scope-81f0"))
            .unwrap_or_else(|| panic!("the skipped mutation must be named: {:?}", captured.records));
        assert!(
            warning
                .message
                .starts_with("user config: mutations user entry [1] (id=bad-scope-81f0) failed to parse: "),
            "{}",
            warning.message
        );
        assert!(
            warning.message.contains("NoSuchScope"),
            "serde's reason: {}",
            warning.message
        );

        // The strict parser the bundle uses still fails whole on it.
        assert!(parse_mutations_json(&source).is_err());
        // Control: with the entry fixed, both parsers take all three,
        // so every failure above is the bad scope's doing.
        let fixed = source.replace("NoSuchScope", "SelfOnly");
        assert_eq!(parse_user_mutations_json(&fixed).unwrap().len(), 3);
        assert_eq!(parse_mutations_json(&fixed).unwrap().len(), 3);
    }

    /// A key beside `mutations` in the envelope is reported, not
    /// silently dropped: a `"mutation"` typo would otherwise load as an
    /// empty tier with nothing said.
    #[test]
    fn test_parse_user_mutations_json_reports_an_unrecognized_envelope_key() {
        baumhard::util::test_logger::install();
        let source = format!(
            r#"{{"mutation": [{}], "_note": "a comment"}}"#,
            mutation_json("typo-ed", "SelfOnly")
        );
        let (parsed, captured) = baumhard::util::log::capture::capture(|| parse_user_mutations_json(&source));
        assert!(parsed.unwrap().is_empty(), "only `mutations` is read");
        let messages: Vec<&str> = captured.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            messages,
            vec!["mutations: unrecognized key 'mutation' in the user mutations file — ignored; only `mutations` is read"],
            "one warning, and none for the `_`-prefixed comment"
        );
        // Control: spelled right, the entry loads and nothing is said.
        let fixed = source.replace(r#""mutation":"#, r#""mutations":"#);
        let (parsed, captured) = baumhard::util::log::capture::capture(|| parse_user_mutations_json(&fixed));
        assert_eq!(parsed.unwrap().len(), 1);
        assert!(captured.records.is_empty(), "{:?}", captured.records);
    }

    /// The envelope itself is the boundary: not an object, or a
    /// `mutations` that is not a list, leaves nothing to salvage.
    #[test]
    fn test_parse_user_mutations_json_fails_whole_only_on_an_unusable_envelope() {
        assert_eq!(parse_user_mutations_json(r#"{}"#).unwrap().len(), 0);
        assert_eq!(
            parse_user_mutations_json(r#"[]"#).unwrap_err(),
            "expected a JSON object, found an array"
        );
        assert_eq!(
            parse_user_mutations_json(r#"{"mutations": {"id": "x"}}"#).unwrap_err(),
            "expected `mutations` to be a JSON array"
        );
        assert!(parse_user_mutations_json("{ not json").is_err());
    }

    #[test]
    fn parse_single_mutation_with_new_shape() {
        let src = r#"{
            "mutations": [{
                "id": "hi",
                "name": "Hi",
                "mutator": {"Macro": {"kind": null, "channel": 0, "mutations": {"Literal": []}}},
                "target_scope": "SelfOnly"
            }]
        }"#;
        let v = parse_mutations_json(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "hi");
    }
}
