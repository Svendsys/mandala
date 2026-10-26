// SPDX-License-Identifier: MPL-2.0

//! Hoist per-node color_schema groups into a top-level `palettes`
//! map; rewrite each node to reference its palette by key. `theme_id`
//! is dropped; `variant != 2` is folded into the key (`coral-v3`).

use serde_json::{json, Value};
use std::collections::HashMap;

pub fn hoist_palettes(root: &mut Value) {
    let palettes = collect_palettes(root);
    inject_palettes(root, &palettes);
    simplify_node_schemas(root);
}

/// Scan all nodes for level-0 color_schema entries with non-empty groups.
/// Build a palette map keyed by palette name. When variant differs, fold
/// it into the name (e.g. "coral", "coral-alt").
fn collect_palettes(root: &Value) -> HashMap<String, Value> {
    let mut palettes: HashMap<String, Value> = HashMap::new();

    let nodes = match root.get("nodes").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => return palettes,
    };

    for node in nodes.values() {
        let schema = match node.get("color_schema") {
            Some(Value::Object(obj)) => obj,
            _ => continue,
        };

        let level = schema.get("level").and_then(|v| v.as_i64()).unwrap_or(-1);
        if level != 0 {
            continue;
        }

        let groups = match schema.get("groups").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr.clone(),
            _ => continue,
        };

        // First-writer-wins on a name collision, as
        // `format/migration.md` §"Known limitations" documents. One
        // hash lookup via `entry` rather than the `contains_key` +
        // `insert` pair this used to do (clippy::map_entry).
        let palette_name = palette_key(schema);
        palettes
            .entry(palette_name)
            .or_insert_with(|| json!({ "groups": groups }));
    }

    palettes
}

/// Derive a palette key from a color_schema object. Uses the palette
/// name, with variant folded in for non-standard variants.
fn palette_key(schema: &serde_json::Map<String, Value>) -> String {
    let base = schema
        .get("palette")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let variant = schema.get("variant").and_then(|v| v.as_i64()).unwrap_or(2);
    if variant == 2 {
        base.to_string()
    } else {
        format!("{}-v{}", base, variant)
    }
}

/// Add the hoisted palettes to `root.palettes`, keeping every palette
/// already there.
///
/// A merge, not a replacement. A map that already has a `palettes`
/// table keeps all of it: a key already defined wins over a hoisted
/// one of the same name, the same first-writer-wins rule
/// `collect_palettes` applies between nodes, because a defined palette
/// is the authored one. This used to overwrite the table with the
/// hoisted set, so one stray `color_schema.groups` on a current map
/// replaced its palettes with the single one hoisted from it.
fn inject_palettes(root: &mut Value, palettes: &HashMap<String, Value>) {
    if palettes.is_empty() {
        return;
    }
    let Some(obj) = root.as_object_mut() else { return };
    let table = obj
        .entry("palettes")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(table) = table.as_object_mut() else { return };
    for (key, palette) in palettes {
        table.entry(key.clone()).or_insert_with(|| palette.clone());
    }
}

/// Read a legacy `level` as the non-negative depth the current model
/// requires (`ColorSchema.level: usize`).
///
/// The legacy field was a signed integer and the model's was too, so
/// nothing upstream ever had to mean it. A negative depth is not a
/// depth — it is corruption or a hand edit — and emitting one now
/// produces a file `maptool convert` itself cannot load back. Floor it
/// at 0, which is the schema root, and say so: the conversion still
/// succeeds and the author learns which node to look at. A missing or
/// non-integer field defaults to 0 silently, as it always has.
fn nonnegative_level(raw: Option<&Value>, node_id: &str) -> u64 {
    match raw.and_then(Value::as_i64) {
        Some(level) if level < 0 => {
            eprintln!(
                "convert: node {node_id} carries a negative color_schema.level ({level}); \
                 flooring to 0 — depth below the schema root has no meaning"
            );
            0
        }
        Some(level) => level as u64,
        None => 0,
    }
}

/// Simplify each node's color_schema in place: palette becomes the
/// hoisted key, `level` is floored, `starts_at_root` and
/// `connections_colored` default to `true`, and `groups`, `theme_id`
/// and `variant` are dropped.
///
/// Every other key rides through untouched — `overrides` first among
/// them, which is where `format/migration.md` step 4 tells authors to
/// put a per-node color edit, and keys this tool does not know, which
/// the loader would carry too. This used to rebuild the object from
/// the four keys above, so a second pass over a current map deleted
/// the overrides the first pass's documentation recommended.
fn simplify_node_schemas(root: &mut Value) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };

    for (node_id, node) in nodes.iter_mut() {
        let Some(Value::Object(schema)) = node.get_mut("color_schema") else {
            continue;
        };

        let key = palette_key(schema);
        let level = nonnegative_level(schema.get("level"), node_id);
        for legacy in ["groups", "theme_id", "variant"] {
            schema.remove(legacy);
        }
        schema.insert("palette".to_string(), json!(key));
        schema.insert("level".to_string(), json!(level));
        schema.entry("starts_at_root").or_insert(json!(true));
        schema.entry("connections_colored").or_insert(json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::hoist_palettes;
    use serde_json::{json, Value};

    fn legacy_map_with_level(level: Value) -> Value {
        json!({
            "version": "1.0",
            "name": "t",
            "canvas": {"background_color": "#000"},
            "nodes": {
                "0": {
                    "color_schema": {
                        "palette": "coral",
                        "variant": 2,
                        "level": level,
                        "starts_at_root": true,
                        "connections_colored": true,
                        "groups": [
                            {"background": "#a9decb", "frame": "#30b082", "text": "#000000", "title": "#000000"}
                        ]
                    }
                }
            },
            "edges": []
        })
    }

    fn converted_level(level: Value) -> i64 {
        let mut root = legacy_map_with_level(level);
        hoist_palettes(&mut root);
        root["nodes"]["0"]["color_schema"]["level"]
            .as_i64()
            .expect("level must survive as an integer")
    }

    #[test]
    fn test_hoist_palettes_keeps_a_nonnegative_level() {
        assert_eq!(converted_level(json!(0)), 0);
        assert_eq!(converted_level(json!(4)), 4);
    }

    /// `ColorSchema.level` is a `usize` in the model, so a converted
    /// file carrying a negative depth would not load back — and a
    /// converter whose output its own loader refuses is worse than
    /// one that says what it changed.
    #[test]
    fn test_hoist_palettes_floors_a_negative_level_to_the_schema_root() {
        assert_eq!(converted_level(json!(-1)), 0);
        assert_eq!(converted_level(json!(-9000)), 0);
    }

    #[test]
    fn test_hoist_palettes_defaults_a_missing_level_to_zero() {
        let mut root = legacy_map_with_level(json!(3));
        root["nodes"]["0"]["color_schema"]
            .as_object_mut()
            .unwrap()
            .remove("level");
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["level"], json!(0));
    }

    /// Keys the simplifier does not own ride through: `overrides`,
    /// which a rebuild from four keys used to delete (#198), and a
    /// key this tool has never heard of. The three legacy keys it
    /// does own are still dropped.
    #[test]
    fn test_hoist_palettes_carries_keys_it_does_not_own() {
        let mut root = legacy_map_with_level(json!(0));
        let schema = root["nodes"]["0"]["color_schema"].as_object_mut().unwrap();
        schema.insert("overrides".into(), json!({ "background": "#123456" }));
        schema.insert("future_key".into(), json!([1, 2]));
        schema.insert("theme_id".into(), json!(7));
        hoist_palettes(&mut root);
        assert_eq!(
            root["nodes"]["0"]["color_schema"],
            json!({
                "palette": "coral",
                "level": 0,
                "starts_at_root": true,
                "connections_colored": true,
                "overrides": { "background": "#123456" },
                "future_key": [1, 2]
            })
        );
    }

    /// Hoisting merges into an existing `palettes` table: every
    /// palette already there survives, a hoisted palette with a new
    /// name is added, and one whose name is already defined does not
    /// replace the definition (#211 review).
    #[test]
    fn test_hoist_palettes_merges_into_an_existing_table() {
        let mut root = legacy_map_with_level(json!(0));
        root["palettes"] = json!({
            "coral": { "groups": [{ "background": "#defined" }] },
            "other": { "groups": [{ "background": "#other" }] },
        });
        root["nodes"]["1"] = json!({
            "color_schema": {
                "palette": "fresh",
                "level": 0,
                "groups": [{ "background": "#fresh" }]
            }
        });
        hoist_palettes(&mut root);
        assert_eq!(
            root["palettes"],
            json!({
                "coral": { "groups": [{ "background": "#defined" }] },
                "other": { "groups": [{ "background": "#other" }] },
                "fresh": { "groups": [{ "background": "#fresh" }] },
            })
        );
    }
}
