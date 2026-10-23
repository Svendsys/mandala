// SPDX-License-Identifier: MPL-2.0

//! The default, human-and-LLM-readable output: compact, greppable,
//! uncolored. `--json` bypasses all of this and prints the reply as
//! the server wrote it.
//!
//! **One line per leaf, path first.** A result becomes `path: value`
//! lines — `counts.nodes: 252`, `nodes[0].node_id: "1.2"` — so any
//! fact is one `grep` away and carries its own context, the property
//! that makes a nested dump greppable at all. Scalars print as JSON, so
//! the string `"1"` and the number `1` stay distinguishable. A result
//! with nothing to say prints `ok`.
//!
//! Two verbs print differently because their result is text, not
//! data: `act.console` prints the console's own output lines, and
//! `describe` prints a command table (`Render` on the verb).

use serde_json::Value;

use crate::verbs::Render;

/// The default-mode text for a successful `result`: zero or more
/// newline-terminated lines.
pub fn result_text(render: Render, result: &Value) -> String {
    let mut text = String::new();
    match render {
        Render::Describe => describe(result, &mut text),
        Render::Lines(key) => lines(key, result, &mut text),
        Render::Flat => flat(result, &mut text),
    }
    text
}

/// A result printed as `path: value` lines, each path prefixed with
/// `prefix` (empty for none): how `--wait-settled` shows the wait's
/// result under `wait.`.
pub fn prefixed(prefix: &str, result: &Value) -> String {
    let mut text = String::new();
    leaves(prefix, result, &mut text);
    text
}

fn flat(result: &Value, out: &mut String) {
    if result.as_object().is_some_and(|o| o.is_empty()) {
        out.push_str("ok\n");
        return;
    }
    leaves("", result, out);
}

fn lines(key: &str, result: &Value, out: &mut String) {
    let Some(output) = result.get(key).and_then(Value::as_array) else {
        return flat(result, out);
    };
    let mut rest = result.clone();
    if let Some(object) = rest.as_object_mut() {
        object.remove(key);
    }
    for line in output {
        match line.as_str() {
            Some(text) => out.push_str(text),
            None => out.push_str(&line.to_string()),
        }
        out.push('\n');
    }
    if rest.as_object().is_some_and(|o| !o.is_empty()) {
        leaves("", &rest, out);
    }
}

/// Append `path: value` for every leaf under `value`.
fn leaves(path: &str, value: &Value, out: &mut String) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                leaves(&join(path, key), child, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                leaves(&format!("{path}[{index}]"), child, out);
            }
        }
        Value::Object(_) => line(path, "{}", out),
        Value::Array(_) => line(path, "[]", out),
        scalar => line(path, &scalar.to_string(), out),
    }
}

fn line(path: &str, value: &str, out: &mut String) {
    if !path.is_empty() {
        out.push_str(path);
        out.push_str(": ");
    }
    out.push_str(value);
    out.push('\n');
}

/// `path.key`, or `path["key"]` for a key that is not a plain
/// identifier, so a path can always be read back unambiguously.
fn join(path: &str, key: &str) -> String {
    let plain = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match (plain, path.is_empty()) {
        (true, true) => key.to_string(),
        (true, false) => format!("{path}.{key}"),
        (false, _) => format!("{path}[{}]", Value::String(key.to_string())),
    }
}

/// `describe`'s table: a header, then each command's summary,
/// parameters and result fields, and the event catalog.
fn describe(result: &Value, out: &mut String) {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or("?").to_string();
    let commands = result
        .get("commands")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    out.push_str(&format!(
        "{} {} (protocol {}), {} command(s)\n",
        text(result, "app"),
        text(result, "version"),
        result.get("protocol").map_or("?".to_string(), Value::to_string),
        commands.len()
    ));
    for command in commands {
        out.push_str(&format!(
            "{} - {}\n",
            text(command, "name"),
            text(command, "summary")
        ));
        for param in command
            .get("params")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let need = if param.get("required").and_then(Value::as_bool).unwrap_or(false) {
                "required"
            } else {
                "optional"
            };
            out.push_str(&format!(
                "  param {} {} {} - {}\n",
                text(param, "name"),
                text(param, "type"),
                need,
                text(param, "doc")
            ));
        }
        for field in command
            .get("result")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "  returns {} {} - {}\n",
                text(field, "name"),
                text(field, "type"),
                text(field, "doc")
            ));
        }
    }
    for event in result
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        out.push_str(&format!(
            "event {} - {}\n",
            text(event, "name"),
            text(event, "doc")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The lines of `text`, sorted: whether a result's keys come out in
    /// the server's order or alphabetically is `serde_json`'s feature
    /// flag to decide, not a property these tests are about.
    fn sorted_lines(text: &str) -> Vec<&str> {
        let mut lines: Vec<&str> = text.lines().collect();
        lines.sort_unstable();
        lines
    }

    #[test]
    fn test_flat_prints_one_greppable_line_per_leaf_with_json_scalars() {
        let result = json!({"file_path": "m.json", "dirty": false, "counts": {"nodes": 252}, "none": null});
        let text = result_text(Render::Flat, &result);
        assert_eq!(
            sorted_lines(&text),
            [
                "counts.nodes: 252",
                "dirty: false",
                "file_path: \"m.json\"",
                "none: null"
            ]
        );
    }

    #[test]
    fn test_flat_indexes_arrays_and_keeps_string_and_number_apart() {
        let result = json!({"nodes": [{"node_id": "1", "n": 1}, {"node_id": "2", "n": 2}]});
        let text = result_text(Render::Flat, &result);
        assert_eq!(
            sorted_lines(&text),
            [
                "nodes[0].n: 1",
                "nodes[0].node_id: \"1\"",
                "nodes[1].n: 2",
                "nodes[1].node_id: \"2\""
            ]
        );
    }

    #[test]
    fn test_flat_says_ok_for_an_empty_result_and_names_empty_containers() {
        assert_eq!(result_text(Render::Flat, &json!({})), "ok\n");
        assert_eq!(
            result_text(Render::Flat, &json!({"a": [], "b": {}})),
            "a: []\nb: {}\n"
        );
    }

    #[test]
    fn test_a_key_that_is_not_an_identifier_is_bracketed_so_the_path_reads_back() {
        let text = result_text(Render::Flat, &json!({"nodes": {"1.2": {"x": 1}}}));
        assert_eq!(text, "nodes[\"1.2\"].x: 1\n");
    }

    #[test]
    fn test_a_string_with_a_newline_stays_on_one_line() {
        let text = result_text(Render::Flat, &json!({"text": "a\nb"}));
        assert_eq!(text, "text: \"a\\nb\"\n");
    }

    #[test]
    fn test_lines_prints_the_console_output_bare_then_anything_else() {
        let text = result_text(Render::Lines("output"), &json!({"output": ["one", "two words"]}));
        assert_eq!(text, "one\ntwo words\n");
        assert_eq!(result_text(Render::Lines("output"), &json!({"output": []})), "");
        let text = result_text(Render::Lines("output"), &json!({"output": ["x"], "extra": 1}));
        assert_eq!(text, "x\nextra: 1\n");
    }

    #[test]
    fn test_lines_falls_back_to_flat_when_the_result_is_not_what_it_expects() {
        assert_eq!(
            result_text(Render::Lines("output"), &json!({"other": 1})),
            "other: 1\n"
        );
    }

    #[test]
    fn test_prefixed_roots_every_path_under_the_prefix() {
        assert_eq!(
            prefixed("wait", &json!({"elapsed_ms": 31})),
            "wait.elapsed_ms: 31\n"
        );
        assert_eq!(prefixed("wait", &json!({})), "wait: {}\n");
    }

    #[test]
    fn test_describe_prints_a_table_with_params_results_and_events() {
        let result = json!({
            "protocol": 1, "app": "mandala", "version": "0.1.0",
            "commands": [{
                "name": "scene.hit_test", "family": "scene", "summary": "What a click would hit.",
                "params": [{"name": "x", "type": "number", "required": true, "doc": "The x."}],
                "result": [{"name": "node", "type": "object", "doc": "The node rung."}]
            }],
            "events": [{"name": "hello", "doc": "The greeting."}]
        });
        assert_eq!(
            result_text(Render::Describe, &result),
            "mandala 0.1.0 (protocol 1), 1 command(s)\n\
             scene.hit_test - What a click would hit.\n\
             \x20 param x number required - The x.\n\
             \x20 returns node object - The node rung.\n\
             event hello - The greeting.\n"
        );
    }
}
