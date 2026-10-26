// SPDX-License-Identifier: MPL-2.0

//! Single-source JSON parsing for non-mindmap loaders.
//!
//! Three loaders in the application crate (`keybinds`, `macros`,
//! `widgets/color_picker_widget`) all reach for `serde_json::from_str`
//! directly. Each formats its own error message in a slightly
//! different shape. This module collapses them onto one helper so
//! a future format swap (or a richer error wrapper) is one edit.
//!
//! The mindmap format itself is handled by
//! [`crate::mindmap::loader`] — that module owns the entire on-disk
//! schema and its legacy-detection rules, so it stays separate.
//!
//! Maptool (the CLI) keeps using `serde_json` directly because it
//! works below the typed shape (raw `Value` migrations) — the
//! audit policy carves convenience-only deps out of the wrapper
//! mandate, and that's the case here.

use std::cell::RefCell;
use std::fmt;

use serde::de::{DeserializeOwned, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

/// Re-export so callers don't need a `serde_json` import for the
/// dynamic-JSON case (legacy migration scaffolding, the
/// `Vec<serde_json::Value>` shape on `MindMap.macros`, etc).
pub use serde_json::Value;

/// Re-export of the map a JSON object parses into, for callers that
/// walk an object's fields one at a time (the app's warn-and-skip
/// config parsers) rather than deserializing it in one pass.
pub use serde_json::Map;

/// Parse a JSON string into a typed value, returning a
/// caller-readable error string on failure. Wraps
/// [`serde_json::from_str`].
///
/// Use for any non-mindmap typed loader (keybinds, user macros,
/// embedded widget specs). The mindmap format has its own
/// loader at [`crate::mindmap::loader::load_from_str`] which
/// also handles legacy detection.
#[inline]
pub fn parse<T: DeserializeOwned>(source: &str) -> Result<T, String> {
    serde_json::from_str(source).map_err(|e| e.to_string())
}

/// Deserialize a typed value out of a borrowed `Value`, returning a
/// caller-readable error string.
///
/// Borrows rather than consumes: `serde_json` deserializes straight
/// out of `&Value`, so a caller walking an `&[Value]` (the macro and
/// mutation loaders' per-entry parse) pays for the strings the typed
/// value keeps and nothing else — no clone of the entry first.
#[inline]
pub fn parse_value<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    T::deserialize(value).map_err(|e| e.to_string())
}

/// Parse `source` into a `Value`, also returning the path of every
/// object key that occurs more than once within the same object.
///
/// JSON leaves a repeated key's meaning to the parser, and a `Value`
/// cannot hold both occurrences, so a plain parse keeps the last one
/// and says nothing. A typed `#[derive(Deserialize)]` struct, by
/// contrast, refuses a repeated field outright. A caller that parses
/// field by field from a `Value` — the app's warn-and-skip config
/// loaders — loses that refusal; this keeps the information, so the
/// caller can say which key was repeated. The value holds the **last**
/// occurrence of each key, as a plain parse would.
///
/// Paths are dotted for object keys and bracketed for array indices,
/// relative to the root: `undo`, `[1].steps`, `mutations[0].id`. One
/// entry per extra occurrence, in document order.
///
/// The error strings are `serde_json`'s, from the same deserializer
/// [`parse`] uses, so a syntax error reads the same through either.
/// Recursion depth is bounded by `serde_json`'s own limit.
///
/// Costs one pass over `source`, the `Value` it builds, and one
/// `String` per object or array (its path, which its children's paths
/// extend), plus one per duplicate found.
pub fn parse_reporting_duplicates(source: &str) -> Result<(Value, Vec<String>), String> {
    let duplicates = RefCell::new(Vec::new());
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let value = DuplicateSeed {
        parent: "",
        segment: Segment::Root,
        duplicates: &duplicates,
    }
    .deserialize(&mut deserializer)
    .map_err(|e| e.to_string())?;
    deserializer.end().map_err(|e| e.to_string())?;
    Ok((value, duplicates.into_inner()))
}

/// Where a value sits inside its parent, for
/// [`parse_reporting_duplicates`]'s paths.
#[derive(Clone, Copy)]
enum Segment<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

/// The path of the value at `segment` under `parent`.
fn join_path(parent: &str, segment: Segment<'_>) -> String {
    match segment {
        Segment::Root => String::new(),
        Segment::Key(key) if parent.is_empty() => key.to_string(),
        Segment::Key(key) => format!("{parent}.{key}"),
        Segment::Index(index) => format!("{parent}[{index}]"),
    }
}

/// Deserializes one value into a `Value`, recording repeated object
/// keys at and below it. The seed carries its parent's path and its
/// own segment rather than a built path, so a scalar — most values in
/// a config — never allocates one.
struct DuplicateSeed<'a> {
    parent: &'a str,
    segment: Segment<'a>,
    duplicates: &'a RefCell<Vec<String>>,
}

impl<'de> DeserializeSeed<'de> for DuplicateSeed<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for DuplicateSeed<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        self.deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let path = join_path(self.parent, self.segment);
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element_seed(DuplicateSeed {
            parent: &path,
            segment: Segment::Index(values.len()),
            duplicates: self.duplicates,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let path = join_path(self.parent, self.segment);
        let mut object = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            let value = access.next_value_seed(DuplicateSeed {
                parent: &path,
                segment: Segment::Key(&key),
                duplicates: self.duplicates,
            })?;
            if object.contains_key(&key) {
                self.duplicates
                    .borrow_mut()
                    .push(join_path(&path, Segment::Key(&key)));
            }
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reporting_duplicates_names_every_repeated_key_by_path() {
        let (value, duplicates) = parse_reporting_duplicates(
            r#"{
                "undo": ["a"],
                "undo": ["b"],
                "list": [{"id": "x", "id": "y"}, {"id": "z"}],
                "nested": {"k": 1, "k": 2, "k": 3}
            }"#,
        )
        .unwrap();
        assert_eq!(duplicates, vec!["undo", "list[0].id", "nested.k", "nested.k"]);
        // The value keeps the last occurrence, as a plain parse does.
        assert_eq!(value["undo"], serde_json::json!(["b"]));
        assert_eq!(value["list"][0]["id"], "y");
        assert_eq!(value["nested"]["k"], 3);
        // Control: the same document through a plain parse is the same
        // value, so the report is the only thing this adds.
        assert_eq!(
            parse::<Value>(r#"{"undo": ["a"], "undo": ["b"]}"#).unwrap()["undo"],
            serde_json::json!(["b"])
        );
    }

    #[test]
    fn test_parse_reporting_duplicates_reports_nothing_for_distinct_keys() {
        let source = r#"[{"id": "a", "n": 1.5, "on": true, "x": null}, {"id": "a"}, "id", -3]"#;
        let (value, duplicates) = parse_reporting_duplicates(source).unwrap();
        // The same key in *sibling* objects is not a repeat.
        assert!(duplicates.is_empty(), "{duplicates:?}");
        assert_eq!(value, parse::<Value>(source).unwrap());
    }

    #[test]
    fn test_parse_reporting_duplicates_fails_where_a_plain_parse_fails() {
        for source in ["{ \"a\": ", "[1, 2] trailing", ""] {
            let ours = parse_reporting_duplicates(source).unwrap_err();
            let plain = parse::<Value>(source).unwrap_err();
            assert_eq!(ours, plain, "{source:?}");
        }
    }
}
