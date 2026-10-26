// SPDX-License-Identifier: MPL-2.0

//! Legacy markers: the shapes the `convert --legacy` passes exist to
//! migrate. A map carrying none of them is already current, and the
//! id pass leaves its ids alone (see [`super::ids::assign_dewey_ids`]).
//!
//! The list mirrors the passes, one shape per thing a pass rewrites:
//!
//! - ids: a node's `index`
//! - portals: a top-level `portals` key
//! - enums: `style.shape_type`, an integer `layout.type` or
//!   `layout.direction`, an integer edge `line_style`, `anchor_from`
//!   or `anchor_to`
//! - palettes: `groups`, `theme_id` or `variant` on a `color_schema`
//! - sections: a node's `text` or `text_runs`
//!
//! Why every one and not just `index`: miMind's heap-pointer ids
//! (`"348068464"`) are runs of digits, which is a well-formed Dewey
//! root, so a legacy export without `index` would otherwise look
//! current by its ids alone and keep them — and a legacy child keyed
//! `"17"` under a root `"348068464"` is not a tree `maptool verify`
//! accepts.

use serde_json::Value;

/// The first legacy marker in `root`, as a short description of where
/// it is, or `None` when the map carries none.
///
/// Cost: one pass over the nodes and one over the edges, a few
/// `Value::get`s each.
pub fn first_legacy_marker(root: &Value) -> Option<String> {
    if root.get("portals").is_some() {
        return Some("top-level `portals`".to_string());
    }
    let is_int = |v: Option<&Value>| v.is_some_and(Value::is_i64) || v.is_some_and(Value::is_u64);
    if let Some(nodes) = root.get("nodes").and_then(Value::as_object) {
        for (id, node) in nodes {
            for key in ["index", "text", "text_runs"] {
                if node.get(key).is_some() {
                    return Some(format!("node {id:?} `{key}`"));
                }
            }
            if node.get("style").and_then(|s| s.get("shape_type")).is_some() {
                return Some(format!("node {id:?} `style.shape_type`"));
            }
            if let Some(layout) = node.get("layout") {
                for key in ["type", "direction"] {
                    if is_int(layout.get(key)) {
                        return Some(format!("node {id:?} integer `layout.{key}`"));
                    }
                }
            }
            if let Some(schema) = node.get("color_schema") {
                for key in ["groups", "theme_id", "variant"] {
                    if schema.get(key).is_some() {
                        return Some(format!("node {id:?} `color_schema.{key}`"));
                    }
                }
            }
        }
    }
    if let Some(edges) = root.get("edges").and_then(Value::as_array) {
        for (i, edge) in edges.iter().enumerate() {
            for key in ["line_style", "anchor_from", "anchor_to"] {
                if is_int(edge.get(key)) {
                    return Some(format!("edge {i} integer `{key}`"));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::first_legacy_marker;
    use baumhard::util::doc_fixtures::repo_path;
    use serde_json::{json, Value};

    fn testament() -> Value {
        let text = std::fs::read_to_string(repo_path("maps/testament.mindmap.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The app-saved map carries no marker, so the rows below each
    /// plant exactly the one they name.
    #[test]
    fn test_testament_carries_no_legacy_marker() {
        assert_eq!(first_legacy_marker(&testament()), None);
    }

    /// One row per shape a `convert --legacy` pass migrates. Each is
    /// planted alone into testament and must be found.
    #[test]
    fn test_every_legacy_shape_is_a_marker() {
        type Plant = fn(&mut Value);
        let rows: [(&str, Plant); 13] = [
            ("portals", |m| m["portals"] = json!([])),
            ("index", |m| m["nodes"]["0"]["index"] = json!(0)),
            ("text", |m| m["nodes"]["0"]["text"] = json!("t")),
            ("text_runs", |m| m["nodes"]["0"]["text_runs"] = json!([])),
            ("shape_type", |m| {
                m["nodes"]["0"]["style"]["shape_type"] = json!(1)
            }),
            ("layout.type", |m| m["nodes"]["0"]["layout"]["type"] = json!(1)),
            ("layout.direction", |m| {
                m["nodes"]["0"]["layout"]["direction"] = json!(2)
            }),
            ("groups", |m| {
                m["nodes"]["0"]["color_schema"]["groups"] = json!([])
            }),
            ("theme_id", |m| {
                m["nodes"]["0"]["color_schema"]["theme_id"] = json!(7)
            }),
            ("variant", |m| {
                m["nodes"]["0"]["color_schema"]["variant"] = json!(3)
            }),
            ("line_style", |m| m["edges"][0]["line_style"] = json!(1)),
            ("anchor_from", |m| m["edges"][0]["anchor_from"] = json!(2)),
            ("anchor_to", |m| m["edges"][0]["anchor_to"] = json!(3)),
        ];
        for (name, plant) in rows {
            let mut map = testament();
            plant(&mut map);
            let found = first_legacy_marker(&map);
            assert!(
                found.as_deref().is_some_and(|f| f.contains(name)),
                "{name} must be a legacy marker, found {found:?}"
            );
        }
    }
}
