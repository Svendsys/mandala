// SPDX-License-Identifier: MPL-2.0

//! The user's settings: `settings.json` on native
//! (`$XDG_CONFIG_HOME/mandala/settings.json`, or the `$HOME/.config`
//! fallback), `localStorage["mandala_settings"]` in the browser.
//!
//! One key so far, `history_limit`: the most entries a document's undo
//! history holds (D18). The document is held to it from the moment it
//! is installed (`document::MindMapDocument::new`), so the limit is the
//! same whichever way a map arrives — at startup, through `open` or
//! `new`, or from the browser's `?map=`.
//!
//! The file degrades the way every user-tier config does
//! ([`super::salvage`]): text that is not a JSON object skips the
//! layer and the defaults stand; a recognized key with a malformed
//! value is warned about by name and keeps its default; an
//! unrecognized key is warned about and dropped. Keys starting with
//! `_` are comments and are not reported.

use baumhard::document::writer::DEFAULT_HISTORY_LIMIT;

use super::salvage::{parse_object, take_field};

/// The `settings.json` file name, and the label its log lines carry.
const LABEL: &str = "settings";

/// The settings, each at its default unless the user's file says
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The most entries a document's undo history holds. Default
    /// [`DEFAULT_HISTORY_LIMIT`].
    pub history_limit: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }
}

/// The keys [`parse`] recognizes, in the order a report lists them.
const KNOWN_KEYS: &[&str] = &["history_limit"];

/// Parse a settings payload, one key at a time.
///
/// `Err` only when the text is not a JSON object: there is nothing to
/// salvage, and the loader keeps the defaults. Inside an object, a
/// malformed value keeps its default with a warning naming the key,
/// and a key this file does not recognize is warned about and dropped.
///
/// Cost: one parse; a warning per offending key.
pub fn parse(json: &str) -> Result<Settings, String> {
    let mut object = parse_object(LABEL, json).map_err(|e| format!("parse settings JSON: {e}"))?;
    let defaults = Settings::default();
    let settings = Settings {
        history_limit: take_field(LABEL, &mut object, "history_limit").unwrap_or(defaults.history_limit),
    };
    let mut unknown: Vec<String> = object
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| !key.starts_with('_'))
        .collect();
    unknown.sort();
    for key in unknown {
        log::warn!(
            "settings: unrecognized key '{key}' — ignored. The keys this file recognizes are: {}; keys \
             starting with '_' are treated as comments.",
            KNOWN_KEYS.join(", "),
        );
    }
    Ok(settings)
}

/// Load the user's settings: the XDG `settings.json` on native, never
/// failing — an absent or unusable file leaves the defaults.
///
/// Cost: one `stat` and, when the file exists, one capped read and
/// one parse.
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Settings {
    match super::load_desktop_layered(LABEL, "settings.json", None, parse) {
        Some((settings, source)) => {
            log::info!("loaded settings from {source}");
            settings
        }
        None => Settings::default(),
    }
}

/// Load the user's settings: `localStorage["mandala_settings"]` in the
/// browser — no URL layer, for the reason
/// `web_storage::load_web_storage_only` gives — never failing.
///
/// Cost: one `localStorage` round-trip and, when the key is set, one
/// parse.
#[cfg(target_arch = "wasm32")]
pub fn load() -> Settings {
    match super::web_storage::load_web_storage_only(LABEL, "mandala_settings", parse) {
        Some((settings, source)) => {
            log::info!("loaded settings from {source}");
            settings
        }
        None => Settings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::util::log::capture::capture;

    /// The value the file names is the value the settings carry, and an
    /// empty object leaves the default.
    #[test]
    fn test_settings_history_limit_is_read_and_defaults_when_absent() {
        assert_eq!(parse(r#"{"history_limit": 7}"#).unwrap().history_limit, 7);
        // Control: the same parse without the key keeps the default,
        // so the 7 above came from the file and not from the default.
        assert_eq!(parse("{}").unwrap().history_limit, DEFAULT_HISTORY_LIMIT);
        assert_ne!(DEFAULT_HISTORY_LIMIT, 7);
    }

    /// An unrecognized key is reported by name and dropped; the keys
    /// around it still apply.
    #[test]
    fn test_settings_an_unknown_key_is_reported_and_dropped() {
        baumhard::util::test_logger::install();
        let (settings, captured) = capture(|| parse(r#"{"history_limit": 3, "histroy_lmit-9c1f": 4}"#));
        assert_eq!(settings.unwrap().history_limit, 3);
        let named: Vec<String> = captured
            .records
            .iter()
            .map(|r| r.message.as_str())
            .filter(|m| m.contains("histroy_lmit-9c1f"))
            .map(str::to_string)
            .collect();
        assert_eq!(named.len(), 1, "one report for the one unknown key: {named:?}");
        assert!(
            named[0].starts_with("settings: unrecognized key 'histroy_lmit-9c1f' — ignored."),
            "{named:?}"
        );
    }

    /// A comment key is not reported: JSON has no comment syntax.
    #[test]
    fn test_settings_a_comment_key_is_not_reported() {
        baumhard::util::test_logger::install();
        let (_, captured) = capture(|| parse(r#"{"_comment-77aa": "hello"}"#));
        assert!(!captured
            .records
            .iter()
            .map(|r| r.message.as_str())
            .any(|m| m.contains("_comment-77aa")));
    }

    /// A malformed value keeps the default, says so naming the key, and
    /// costs nothing else.
    #[test]
    fn test_settings_a_malformed_value_keeps_its_default() {
        baumhard::util::test_logger::install();
        let (settings, captured) = capture(|| parse(r#"{"history_limit": "many"}"#));
        assert_eq!(settings.unwrap().history_limit, DEFAULT_HISTORY_LIMIT);
        assert!(captured
            .records
            .iter()
            .map(|r| r.message.as_str())
            .any(|m| m.contains("'history_limit' has a malformed value")));
    }

    /// Text that is not an object is the one whole-file failure.
    #[test]
    fn test_settings_a_non_object_is_refused_whole() {
        assert!(parse("[1]").is_err());
        assert!(parse("{ not json").is_err());
    }
}
