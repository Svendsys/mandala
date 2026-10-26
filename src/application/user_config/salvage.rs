// SPDX-License-Identifier: MPL-2.0

//! Warn-and-skip parsing: one malformed value costs that value and
//! nothing else.
//!
//! A hand-edited config fails one piece at a time — a stale value
//! shape after a schema change, a number where a list belongs, a
//! macro with a misspelled step kind. Parsed in one serde pass, any of
//! those fails the whole document, and
//! [`super::layered::load_layered`] answers a failed parse by
//! skipping the layer: one bad value used to cost the user every good
//! one in the same file. These are the primitives the parsers stand on
//! instead, so the damage stays with the piece that is broken.
//!
//! **Where the boundary sits.** Two failures, two answers:
//!
//! - The *container* is unusable — the text is not JSON, or it is
//!   JSON of the wrong outer shape (a list where the config is an
//!   object, a string where the macros file is a list). There is
//!   nothing to salvage, so the parser returns `Err` and the layer
//!   fails whole, exactly as before: the driver logs it and walks on.
//!   [`parse_object`] and [`parse_array`] are that check.
//! - The container is fine and one *piece* inside it is not — a
//!   recognized key's value ([`take_field`]) or one entry of a list
//!   ([`parse_entry`] / [`parse_entries`]). That piece is warned
//!   about, naming the key or the entry's index and id, and dropped;
//!   the parse returns `Ok` with everything else.
//!
//! **Why the parsers own this and the driver does not.** The driver
//! sees an opaque payload and a `Result`, and what counts as a "piece"
//! is a fact about each config's shape — a field of the keybinds
//! object, an entry of the macros list, an entry of the mutations
//! envelope's list. Teaching the driver that would make it parse every
//! config's schema; keeping it to `Result<T, String>` keeps its one
//! job (precedence across layers), and `Err` keeps its one meaning
//! (this layer has nothing usable). What would otherwise be written
//! three times is written here once.
//!
//! **A repeated key is the third way a file goes stale**, beside an
//! unknown key and a malformed value, and it gets the same posture:
//! [`parse_object`] and [`parse_array`] warn once per repeat, naming
//! its path, and the **last** occurrence is the one used — what a
//! plain JSON parse keeps, and what a user who appended an override
//! at the bottom of the file meant. A one-pass typed parse refused the
//! whole file instead; a `Value` on its own would keep the last one
//! silently.
//!
//! Every function here is cross-platform: the desktop and browser
//! loaders call the same parsers, so both targets degrade the same way
//! (CODE_CONVENTIONS §4).

use baumhard::format::json::{parse_reporting_duplicates, parse_value, Map, Value};
use serde::de::DeserializeOwned;

/// A parsed JSON object, keyed by field name: what [`parse_object`]
/// returns and [`take_field`] moves fields out of.
pub type JsonObject = Map<String, Value>;

/// Parse `source` as a JSON object, or say why it is not one.
///
/// `Err` is the whole-layer failure: text that is not JSON, or JSON
/// whose outer value is anything but an object. The message names
/// which, so the log line tells a syntax error from a wrong shape. A
/// key repeated anywhere in the document is warned about under
/// `label`, by path, and its last occurrence kept.
///
/// Costs one parse into an owned `Map`; the values are moved out of it
/// later by [`take_field`] rather than cloned.
pub fn parse_object(label: &str, source: &str) -> Result<JsonObject, String> {
    match parse_warning_duplicates(label, source)? {
        Value::Object(object) => Ok(object),
        other => Err(format!("expected a JSON object, found {}", json_kind(&other))),
    }
}

/// Parse `source` as a JSON array, or say why it is not one. The array
/// peer of [`parse_object`], with the same whole-layer meaning for
/// `Err` and the same report of repeated keys inside the entries.
pub fn parse_array(label: &str, source: &str) -> Result<Vec<Value>, String> {
    match parse_warning_duplicates(label, source)? {
        Value::Array(values) => Ok(values),
        other => Err(format!("expected a JSON array, found {}", json_kind(&other))),
    }
}

/// Parse `source` into a `Value`, warning once per repeated key.
fn parse_warning_duplicates(label: &str, source: &str) -> Result<Value, String> {
    let (value, duplicates) = parse_reporting_duplicates(source)?;
    for path in duplicates {
        log::warn!("user config: {label} key '{path}' appears more than once; the last occurrence is used");
    }
    Ok(value)
}

/// Move `key` out of `object` and deserialize it as `T`.
///
/// - Absent: `None`, silently — an absent key means "keep the
///   default", which is what a partial config is for.
/// - Present and well formed: `Some(value)`.
/// - Present and malformed: `None`, after a `warn!` that names the
///   key, says its default stands, and carries serde's reason. The
///   caller keeps the field's default, exactly as if the key were
///   absent.
///
/// The key is removed either way, so whatever is left in `object`
/// afterwards is what no field claimed.
pub fn take_field<T: DeserializeOwned>(label: &str, object: &mut JsonObject, key: &str) -> Option<T> {
    let value = object.remove(key)?;
    match parse_value::<T>(&value) {
        Ok(parsed) => Some(parsed),
        Err(e) => {
            log::warn!("user config: {label} '{key}' has a malformed value and keeps its default: {e}");
            None
        }
    }
}

/// Deserialize entry `index` of a list as `T`, or warn and skip it.
///
/// The warning names the entry by index and, when it carries one, by
/// its `id` — macros and mutations are addressed by id, so
/// `entry [3] (id=save-and-quit)` is what a user can find in the file.
/// `tier` says which list it came from (`"user"`, `"Map-tier"`, ...),
/// since the same id can live in several.
///
/// Deserializes straight out of the borrowed `value`; nothing is
/// cloned beyond what `T` keeps.
pub fn parse_entry<T: DeserializeOwned>(label: &str, tier: &str, index: usize, value: &Value) -> Option<T> {
    match parse_value::<T>(value) {
        Ok(parsed) => Some(parsed),
        Err(e) => {
            let id = value.get("id").and_then(Value::as_str).unwrap_or("<no id>");
            log::warn!(
                "user config: {label} {tier} entry [{index}] (id={id}) failed to parse: {e} (skipping)"
            );
            None
        }
    }
}

/// [`parse_entry`] over a whole list, keeping the entries that parse,
/// in order. An entry is all-or-nothing: a macro or a mutation missing
/// one of its steps is a different program, not a smaller one.
pub fn parse_entries<T: DeserializeOwned>(label: &str, tier: &str, values: &[Value]) -> Vec<T> {
    values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| parse_entry(label, tier, index, value))
        .collect()
}

/// The JSON kind of `value`, as the shape errors above name it.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::util::log::capture::capture;

    #[test]
    fn test_parse_object_tells_a_wrong_shape_from_text_that_is_not_json() {
        assert!(parse_object("salvage-test", r#"{"a": 1}"#).is_ok());
        let shape = parse_object("salvage-test", "[1, 2]").unwrap_err();
        assert_eq!(shape, "expected a JSON object, found an array");
        // Control: text that is not JSON fails in serde, before any
        // shape is known, so the message is serde's and not the shape
        // one — the two failures stay distinguishable in the log.
        let syntax = parse_object("salvage-test", "{ not json").unwrap_err();
        assert!(!syntax.contains("expected a JSON object"), "{syntax}");
    }

    #[test]
    fn test_parse_array_tells_a_wrong_shape_from_text_that_is_not_json() {
        assert_eq!(parse_array("salvage-test", "[]").unwrap(), Vec::<Value>::new());
        let shape = parse_array("salvage-test", r#"{"id": "x"}"#).unwrap_err();
        assert_eq!(shape, "expected a JSON array, found an object");
        let syntax = parse_array("salvage-test", "[ 1,").unwrap_err();
        assert!(!syntax.contains("expected a JSON array"), "{syntax}");
    }

    /// A repeated key is warned about by path, once per repeat, and the
    /// last occurrence wins — at the top level and inside an entry.
    #[test]
    fn test_a_repeated_key_is_named_and_the_last_occurrence_wins() {
        baumhard::util::test_logger::install();
        let (object, captured) = capture(|| {
            parse_object(
                "salvage-test",
                r#"{"dup-4b0e": ["first"], "other": 1, "dup-4b0e": ["last"]}"#,
            )
        });
        assert_eq!(object.unwrap()["dup-4b0e"], serde_json::json!(["last"]));
        let messages: Vec<&str> = captured.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            messages,
            vec!["user config: salvage-test key 'dup-4b0e' appears more than once; the last occurrence is used"]
        );

        let (values, captured) =
            capture(|| parse_array("salvage-test", r#"[{"id": "a"}, {"id": "b", "id": "c"}]"#));
        assert_eq!(values.unwrap()[1]["id"], "c");
        let messages: Vec<&str> = captured.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            messages,
            vec![
                "user config: salvage-test key '[1].id' appears more than once; the last occurrence is used"
            ]
        );

        // Control: the same documents without the repeat warn nothing,
        // so the lines above are the repeat's doing.
        let (_, captured) = capture(|| parse_object("salvage-test", r#"{"dup-4b0e": ["last"], "other": 1}"#));
        assert!(captured.records.is_empty(), "{:?}", captured.records);
    }

    #[test]
    fn test_take_field_degrades_a_malformed_value_and_names_its_key() {
        baumhard::util::test_logger::install();
        let mut object =
            parse_object("salvage-test", r#"{"good": ["a"], "bad-7d1f": 5, "rest": true}"#).unwrap();

        let (good, captured) = capture(|| take_field::<Vec<String>>("salvage-test", &mut object, "good"));
        assert_eq!(good, Some(vec!["a".to_string()]));
        assert!(
            captured.records.is_empty(),
            "a good value is not a mistake: {:?}",
            captured.records
        );

        let (bad, captured) = capture(|| take_field::<Vec<String>>("salvage-test", &mut object, "bad-7d1f"));
        assert_eq!(bad, None);
        let warning = captured
            .records
            .iter()
            .find(|r| r.message.contains("'bad-7d1f'"))
            .unwrap_or_else(|| panic!("a malformed value must be named: {:?}", captured.records));
        assert!(
            warning.message.starts_with("user config: salvage-test "),
            "{}",
            warning.message
        );
        assert!(
            warning.message.contains("keeps its default"),
            "{}",
            warning.message
        );
        assert!(
            warning.message.contains("expected a sequence"),
            "serde's reason: {}",
            warning.message
        );

        // Absent is silent, and both taken keys are gone while the
        // unclaimed one stays for the caller's unknown-key report.
        let (absent, captured) = capture(|| take_field::<Vec<String>>("salvage-test", &mut object, "nope"));
        assert_eq!(absent, None);
        assert!(captured.records.is_empty(), "{:?}", captured.records);
        assert_eq!(object.keys().collect::<Vec<_>>(), vec!["rest"]);
    }

    #[test]
    fn test_parse_entries_keeps_the_good_entries_in_order_and_names_the_bad_one() {
        baumhard::util::test_logger::install();
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Entry {
            id: String,
            n: u32,
        }
        let values = parse_array(
            "salvage-test",
            r#"[
                {"id": "first", "n": 1},
                {"id": "broken-3c9a", "n": "one"},
                {"n": "no id either"},
                {"id": "last", "n": 3}
            ]"#,
        )
        .unwrap();
        let (entries, captured) = capture(|| parse_entries::<Entry>("salvage-test", "user", &values));
        assert_eq!(
            entries,
            vec![
                Entry {
                    id: "first".into(),
                    n: 1
                },
                Entry {
                    id: "last".into(),
                    n: 3
                }
            ]
        );
        let messages: Vec<&str> = captured.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "one warning per skipped entry: {messages:?}");
        assert!(
            messages[0]
                .starts_with("user config: salvage-test user entry [1] (id=broken-3c9a) failed to parse: "),
            "{}",
            messages[0]
        );
        assert!(
            messages[1].starts_with("user config: salvage-test user entry [2] (id=<no id>) "),
            "{}",
            messages[1]
        );
    }
}
