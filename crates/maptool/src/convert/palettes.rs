// SPDX-License-Identifier: MPL-2.0

//! Hoist per-node color_schema groups into a top-level `palettes`
//! map; rewrite each node to reference its palette by key. `theme_id`
//! is dropped; `variant != 2` is folded into the key (`coral-v3`).

use baumhard::mindmap::model::dewey_cmp;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub fn hoist_palettes(root: &mut Value) {
    let defined = defined_palettes(root);
    let palettes = collect_palettes(root, &defined);
    // Every name a schema may end up pointing at: the input's own
    // table plus what this pass hoists. `palette_key` needs both, so a
    // groups-less schema follows its subtree's hoisted fold.
    let targets: HashSet<String> = defined.iter().chain(palettes.keys()).cloned().collect();
    inject_palettes(root, &palettes);
    simplify_node_schemas(root, &defined, &targets);
}

/// The names the input's own `palettes` table defines, before any
/// hoisting adds to it. A legacy map has no such table, so this is
/// empty for one and only names already-current palettes.
fn defined_palettes(root: &Value) -> HashSet<String> {
    root.get("palettes")
        .and_then(Value::as_object)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

/// Scan all nodes for level-0 color_schema entries with non-empty groups.
/// Build a palette map keyed by palette name. When variant differs, fold
/// it into the name (e.g. "coral", "coral-alt").
fn collect_palettes(root: &Value, defined: &HashSet<String>) -> HashMap<String, Value> {
    let mut palettes: HashMap<String, Value> = HashMap::new();

    let nodes = match root.get("nodes").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => return palettes,
    };

    // Outline order, so "first" in first-writer-wins means the node a
    // reader meets first. `nodes` is a `BTreeMap` and iterates its keys
    // as strings, which put `x.10` ahead of `x.2` (#211 review).
    let mut ordered: Vec<(&String, &Value)> = nodes.iter().collect();
    ordered.sort_by(|(a, _), (b, _)| dewey_cmp(a, b));

    for (_, node) in ordered {
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

        // First-writer-wins on a name collision, in outline order, as
        // `format/migration.md` §"Known limitations" documents. One
        // hash lookup via `entry` rather than the `contains_key` +
        // `insert` pair this used to do (clippy::map_entry).
        let palette_name = palette_key(schema, defined, defined);
        palettes
            .entry(palette_name)
            .or_insert_with(|| json!({ "groups": groups }));
    }

    palettes
}

/// Derive a palette key from a color_schema object. Uses the palette
/// name, with variant folded in for non-standard variants.
///
/// The fold happens once, and is decided per `(palette, variant)` pair
/// rather than per node, so a legacy subtree keys as one:
///
/// - A `palette` that already ends in `-v{variant}` is taken to have
///   been folded on an earlier pass and is the key as it stands.
/// - Otherwise the folded name is the key, unless the schema has no
///   `groups` of its own, its base name is one `defined` (the input's
///   own table) holds, and the folded name is not a palette at all —
///   neither defined nor hoisted in this pass (`targets`). That case
///   is a stray `variant` on an already-current node.
///
/// History (#211 review): folding unconditionally turned a current
/// map's `coral-v3` + stray `variant: 3` into `coral-v3-v3`, which no
/// table defines. A guard on the defined base alone then bound a
/// legacy `autumn` + `variant: 3` schema to the table's `autumn` and
/// dropped its groups; narrowing that to groups-less schemas still
/// split a legacy subtree, its root folding to `autumn-v3` while its
/// groups-less children stayed on `autumn`. Asking whether the folded
/// name exists is what keeps the subtree together.
fn palette_key(
    schema: &serde_json::Map<String, Value>,
    defined: &HashSet<String>,
    targets: &HashSet<String>,
) -> String {
    let base = schema
        .get("palette")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let variant = schema.get("variant").and_then(|v| v.as_i64()).unwrap_or(2);
    let suffix = format!("-v{variant}");
    if variant == 2 || base.ends_with(&suffix) {
        return base.to_string();
    }
    let folded = format!("{base}{suffix}");
    let has_groups = schema
        .get("groups")
        .and_then(Value::as_array)
        .is_some_and(|groups| !groups.is_empty());
    if has_groups || targets.contains(&folded) || !defined.contains(base) {
        folded
    } else {
        base.to_string()
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
fn simplify_node_schemas(root: &mut Value, defined: &HashSet<String>, targets: &HashSet<String>) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };

    for (node_id, node) in nodes.iter_mut() {
        let Some(Value::Object(schema)) = node.get_mut("color_schema") else {
            continue;
        };

        let key = palette_key(schema, defined, targets);
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

    /// A stray `variant` on a current map does not fold a second time.
    /// The loader keeps it as an unknown key, so an app-saved map can
    /// carry one; folding `coral-v3` again gave `coral-v3-v3`, which
    /// no table defines. Both guards are checked: a name the table
    /// already defines on a schema with nothing to hoist whose folded
    /// name is no palette, and a name that already ends in the suffix.
    /// A schema that does carry its own `groups` still folds even when
    /// its base name is defined, and a groups-less schema follows the
    /// fold when its folded name is hoisted from another node.
    #[test]
    fn test_hoist_palettes_folds_a_variant_once() {
        // Defined in the table, no groups to hoist; the suffix would
        // not match `-v5`.
        let mut root = legacy_map_with_level(json!(0));
        root["palettes"] = json!({ "coral-v3": { "groups": [] } });
        let schema = root["nodes"]["0"]["color_schema"].as_object_mut().unwrap();
        schema.insert("palette".into(), json!("coral-v3"));
        schema.insert("variant".into(), json!(5));
        schema.remove("groups");
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral-v3");

        // Defined in the table, but the schema has its own groups: it
        // has not been hoisted, so it folds and its groups are kept.
        let mut root = legacy_map_with_level(json!(0));
        root["palettes"] = json!({ "coral": { "groups": [{ "background": "#defined" }] } });
        root["nodes"]["0"]["color_schema"]["variant"] = json!(3);
        let own_groups = root["nodes"]["0"]["color_schema"]["groups"].clone();
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral-v3");
        assert_eq!(root["palettes"]["coral-v3"]["groups"], own_groups);
        assert_eq!(root["palettes"]["coral"]["groups"][0]["background"], "#defined");

        // `groups: []` is no groups: with nothing hoisting the folded
        // name, a stray `variant` on a defined base keeps the base,
        // since `coral-v3` would name no palette at all.
        let mut root = legacy_map_with_level(json!(0));
        root["palettes"] = json!({ "coral": { "groups": [{ "background": "#defined" }] } });
        root["nodes"]["0"]["color_schema"]["variant"] = json!(3);
        root["nodes"]["0"]["color_schema"]["groups"] = json!([]);
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral");
        assert!(
            root["palettes"].get("coral-v3").is_none(),
            "nothing is hoisted from empty groups"
        );

        // A legacy subtree under a defined base name: the root carries
        // the groups, its child the same palette and variant with
        // `groups: []`. The child follows its subtree's fold to the
        // hoisted `coral-v3` rather than staying on the table's
        // `coral`, because the folded name is being hoisted.
        let mut root = legacy_map_with_level(json!(0));
        root["palettes"] = json!({ "coral": { "groups": [{ "background": "#defined" }] } });
        root["nodes"]["0"]["color_schema"]["variant"] = json!(3);
        root["nodes"]["0.0"] = json!({
            "parent_id": "0",
            "color_schema": { "palette": "coral", "variant": 3, "level": 1, "groups": [] }
        });
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral-v3");
        assert_eq!(
            root["nodes"]["0.0"]["color_schema"]["palette"], "coral-v3",
            "a groups-less child must follow its subtree's hoisted fold"
        );

        // Not in any table, but already folded with this variant.
        let mut root = legacy_map_with_level(json!(0));
        let schema = root["nodes"]["0"]["color_schema"].as_object_mut().unwrap();
        schema.insert("palette".into(), json!("coral-v3"));
        schema.insert("variant".into(), json!(3));
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral-v3");

        // The legacy fold itself still happens.
        let mut root = legacy_map_with_level(json!(0));
        root["nodes"]["0"]["color_schema"]["variant"] = json!(3);
        hoist_palettes(&mut root);
        assert_eq!(root["nodes"]["0"]["color_schema"]["palette"], "coral-v3");
    }

    /// On a palette-name collision the first node in outline order
    /// wins, not the first in the map's string order: `0.2` before
    /// `0.10`.
    #[test]
    fn test_hoist_palettes_collision_goes_to_the_first_node_in_outline_order() {
        let mut root = legacy_map_with_level(json!(0));
        let nodes = root["nodes"].as_object_mut().unwrap();
        nodes.remove("0");
        for (id, color) in [("0.2", "#222222"), ("0.10", "#101010")] {
            nodes.insert(
                id.into(),
                json!({ "color_schema": {
                    "palette": "autumn", "variant": 3, "level": 0,
                    "groups": [{ "background": color }]
                } }),
            );
        }
        assert!(
            "0.10" < "0.2",
            "the fixture relies on string order putting 0.10 first"
        );
        hoist_palettes(&mut root);
        assert_eq!(
            root["palettes"]["autumn-v3"]["groups"][0]["background"],
            "#222222"
        );
    }
}
