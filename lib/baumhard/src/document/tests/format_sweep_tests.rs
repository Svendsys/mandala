// SPDX-License-Identifier: MPL-2.0

//! Every strict-loader error names its JSON path (design §16), tested
//! by sweeping every value of every fixture rather than by the cases
//! someone thought of: each location is replaced, in turn, by values of
//! the wrong type, and the loader's error must name that location.

use crate::document::format::{load, LoadErrorKind};
use crate::document::tests::format_support::FIXTURES;
use serde_json::{json, Value};

/// One step into a JSON value.
#[derive(Clone, Debug)]
enum Step {
    Key(String),
    Index(usize),
}

/// `steps` as the loader writes a path.
fn render(steps: &[Step]) -> String {
    let mut out = String::new();
    for step in steps {
        match step {
            Step::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Step::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

/// Every location in `value` below the root, with the value there.
fn locations(value: &Value, here: &mut Vec<Step>, out: &mut Vec<(Vec<Step>, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                here.push(Step::Key(key.clone()));
                out.push((here.clone(), inner.clone()));
                locations(inner, here, out);
                here.pop();
            }
        }
        Value::Array(items) => {
            for (index, inner) in items.iter().enumerate() {
                here.push(Step::Index(index));
                out.push((here.clone(), inner.clone()));
                locations(inner, here, out);
                here.pop();
            }
        }
        _ => {}
    }
}

/// `value` with the value at `steps` replaced.
fn replaced(value: &Value, steps: &[Step], with: Value) -> Value {
    let mut copy = value.clone();
    let mut at = &mut copy;
    for step in steps {
        at = match step {
            Step::Key(key) => &mut at[key.as_str()],
            Step::Index(index) => &mut at[*index],
        };
    }
    *at = with;
    copy
}

/// Whether `path` is `location` or something inside it.
fn within(path: &str, location: &str) -> bool {
    location.is_empty()
        || path == location
        || path
            .strip_prefix(location)
            .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
}

/// The kinds a wrong type or an unknown key gives: the ones about the
/// value itself, which is why they must name it. A reference that
/// points nowhere (a verb that the replaced `id` no longer defines) is
/// an error about *another* value and names that.
fn is_local(kind: &LoadErrorKind) -> bool {
    matches!(
        kind,
        LoadErrorKind::Invalid { .. }
            | LoadErrorKind::UnknownKey { .. }
            | LoadErrorKind::MissingKey { .. }
            | LoadErrorKind::DuplicateKey { .. }
            | LoadErrorKind::Syntax { .. }
    )
}

#[test]
fn test_every_wrong_type_at_every_location_names_that_location() {
    let mut swept = 0;
    let mut loaded_by_every_probe = Vec::new();
    for (name, text) in FIXTURES {
        let value: Value = serde_json::from_str(text).unwrap();
        let mut here = Vec::new();
        let mut all = Vec::new();
        locations(&value, &mut here, &mut all);
        for (steps, original) in all {
            let location = render(&steps);
            let mut probes = vec![
                Value::Null,
                json!(true),
                json!("?"),
                json!(1.5),
                json!([]),
                json!({}),
            ];
            // An object written as the array of its own values: the
            // positional form serde would otherwise accept.
            if let Value::Object(map) = &original {
                probes.push(Value::Array(map.values().cloned().collect()));
            }
            let mut refused_locally = false;
            for probe in probes {
                let json = replaced(&value, &steps, probe.clone()).to_string();
                let Err(error) = load(&json) else { continue };
                if !is_local(&error.kind) {
                    continue;
                }
                refused_locally = true;
                assert!(
                    within(&error.path, &location),
                    "{name}: {probe} at `{location}` gave an error elsewhere: {error:?}"
                );
            }
            if !refused_locally {
                loaded_by_every_probe.push(format!("{name}: {location}"));
            }
            swept += 1;
        }
    }
    assert!(swept > 800, "precondition: {swept} locations were swept");
    assert!(
        loaded_by_every_probe.is_empty(),
        "every value of the format has a type, so some wrong type must be refused at it; \
         these accepted all of null, a boolean, a string, a number, an array and an \
         object:\n  {}",
        loaded_by_every_probe.join("\n  ")
    );
}

#[test]
fn test_an_unknown_key_in_every_object_names_that_object() {
    let mut swept = 0;
    for (name, text) in FIXTURES {
        let value: Value = serde_json::from_str(text).unwrap();
        let mut here = Vec::new();
        let mut all = vec![(Vec::new(), value.clone())];
        locations(&value, &mut here, &mut all);
        for (steps, original) in all {
            let Value::Object(_) = original else { continue };
            let location = render(&steps);
            let mut with_key = original.clone();
            with_key["zzz"] = json!(0);
            let json = replaced(&value, &steps, with_key).to_string();
            let error = load(&json).expect_err("an object with a key the schema lacks must not load");
            // An unknown key, usually. serde_json reports a second key
            // in a one-key enum object (`link`) as a syntax error from
            // inside it; the path is the object's all the same.
            assert!(
                matches!(
                    error.kind,
                    LoadErrorKind::UnknownKey { .. }
                        | LoadErrorKind::Invalid { .. }
                        | LoadErrorKind::Syntax { .. }
                ),
                "{name}: a stray key at `{location}` gave {error:?}"
            );
            assert!(
                within(&error.path, &location),
                "{name}: a stray key at `{location}` gave an error elsewhere: {error:?}"
            );
            swept += 1;
        }
    }
    assert!(swept > 250, "precondition: {swept} objects were swept");
}
