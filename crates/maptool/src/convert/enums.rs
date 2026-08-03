// SPDX-License-Identifier: MPL-2.0

//! Rewrite miMind's numeric enum codes (layout type/direction, node
//! shape, line style, anchors) as the current format's named strings.
//! Out-of-range integers fall back to the safest default.

use serde_json::Value;

/// miMind's `shape_type` ordinals as the canonical `style.shape`
/// spellings this converter writes, **indexed by the ordinal**, so the
/// table is the mapping rather than a copy of one.
///
/// Every entry must appear in
/// [`baumhard::gfx_structs::shape::KNOWN_SHAPES`], which is the only
/// list either the runtime's classifier or `maptool verify` consults.
/// This is a restatement of that vocabulary and it was previously
/// unpinned — drop or rename a `KNOWN_SHAPES` entry and the converter
/// would keep emitting the old spelling, so every converted node would
/// warn on load *and* `maptool verify` would reject the file this tool
/// had just produced. `legacy_shape_ordinals_are_canonical_spellings`
/// is what makes that a failing test instead of a shipped map.
///
/// An array rather than a slice so index 0 — the fallback below — is
/// non-empty at compile time.
const LEGACY_SHAPE_ORDINALS: [&str; 6] = [
    "rectangle",         // 0
    "rounded_rectangle", // 1
    "ellipse",           // 2
    "diamond",           // 3
    "parallelogram",     // 4
    "hexagon",           // 5
];

pub fn convert_enums(root: &mut Value) {
    convert_node_enums(root);
    convert_edge_enums(root);
}

fn convert_node_enums(root: &mut Value) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };
    for node in nodes.values_mut() {
        convert_layout(node);
        convert_shape(node);
    }
}

fn convert_layout(node: &mut Value) {
    let layout = match node.get_mut("layout").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };

    if let Some(val) = layout.get("type").and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "map",
            1 => "tree",
            2 => "outline",
            _ => "map",
        };
        layout.insert("type".to_string(), Value::String(name.into()));
    }

    if let Some(val) = layout.get("direction").and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "auto",
            1 => "up",
            2 => "down",
            3 => "left",
            4 => "right",
            6 => "balanced",
            _ => "auto",
        };
        layout.insert("direction".to_string(), Value::String(name.into()));
    }
}

fn convert_shape(node: &mut Value) {
    let style = match node.get_mut("style").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return,
    };

    if let Some(val) = style.remove("shape_type") {
        // An ordinal miMind never defined — or a non-integer — falls
        // back to ordinal 0, which is `rectangle`: the same "safest
        // default" the layout and anchor conversions above use, and
        // the shape the runtime resolves to for anything it cannot
        // draw.
        let name = val
            .as_i64()
            .and_then(|ordinal| usize::try_from(ordinal).ok())
            .and_then(|ordinal| LEGACY_SHAPE_ORDINALS.get(ordinal).copied())
            .unwrap_or(LEGACY_SHAPE_ORDINALS[0]);
        style.insert("shape".to_string(), Value::String(name.into()));
    }
}

fn convert_edge_enums(root: &mut Value) {
    let Some(edges) = super::edges_arr_mut(root) else { return };
    for edge in edges.iter_mut() {
        let obj = match edge.as_object_mut() {
            Some(o) => o,
            None => continue,
        };

        if let Some(val) = obj.get("line_style").and_then(|v| v.as_i64()) {
            let name = match val {
                0 => "solid",
                1 => "dashed",
                _ => "solid",
            };
            obj.insert("line_style".to_string(), Value::String(name.into()));
        }

        convert_anchor(obj, "anchor_from");
        convert_anchor(obj, "anchor_to");
    }
}

fn convert_anchor(obj: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(val) = obj.get(field).and_then(|v| v.as_i64()) {
        let name = match val {
            0 => "auto",
            1 => "top",
            2 => "right",
            3 => "bottom",
            4 => "left",
            _ => "auto",
        };
        obj.insert(field.to_string(), Value::String(name.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Imported here rather than at module scope: nothing outside the
    // tests needs it, and the doc links above spell the full path.
    use baumhard::gfx_structs::shape::{NodeShape, ShapeSpelling, KNOWN_SHAPES};

    /// **The converter's shape vocabulary, pinned to the one list.**
    ///
    /// `LEGACY_SHAPE_ORDINALS` restates spellings that
    /// `KNOWN_SHAPES` owns. Unpinned, a rename over there leaves
    /// `convert --legacy` writing a spelling the runtime warns about
    /// and `maptool verify` — which checks against `KNOWN_SHAPES` —
    /// rejects, so this tool would be producing files its own
    /// validator fails.
    ///
    /// Checked with the same case-insensitive compare `verify` uses,
    /// because that is the question that actually matters: would the
    /// file this converter writes pass?
    #[test]
    fn legacy_shape_ordinals_are_canonical_spellings() {
        assert!(
            !KNOWN_SHAPES.is_empty(),
            "KNOWN_SHAPES is empty — this test would pass vacuously"
        );
        for emitted in LEGACY_SHAPE_ORDINALS {
            assert!(
                KNOWN_SHAPES
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(emitted)),
                "convert --legacy emits {emitted:?}, which is not in \
                 KNOWN_SHAPES — maptool verify would reject the map this \
                 tool just wrote"
            );
            assert_ne!(
                ShapeSpelling::classify(emitted),
                ShapeSpelling::Unrecognized,
                "convert --legacy emits {emitted:?}, which the runtime \
                 classifier calls unknown — every converted node carrying \
                 it would warn on load (issue #118)"
            );
        }
    }

    /// The out-of-range fallback is the shape the runtime falls back
    /// to as well. Derived from `NodeShape::default` rather than
    /// spelled out, so the two cannot drift apart quietly.
    #[test]
    fn out_of_range_shape_ordinal_falls_back_to_the_runtime_default() {
        let mut node = serde_json::json!({ "style": { "shape_type": 99 } });
        convert_shape(&mut node);
        let emitted = node["style"]["shape"]
            .as_str()
            .expect("convert_shape must write a string");
        assert_eq!(emitted, LEGACY_SHAPE_ORDINALS[0]);
        assert_eq!(
            ShapeSpelling::classify(emitted),
            ShapeSpelling::Rendered(NodeShape::default()),
            "the converter's fallback spelling must be the one the \
             renderer's default variant claims"
        );
    }

    /// `convert_shape` really does index the table by the ordinal —
    /// every defined ordinal round-trips to its declared spelling.
    ///
    /// This proves the wiring, **not** the numbering: the expectation
    /// comes out of `LEGACY_SHAPE_ORDINALS` itself, so renumbering the
    /// table renumbers both sides. `format/migration.md` is the
    /// independent source, and
    /// `legacy_shape_ordinals_match_the_published_table` below is what
    /// checks against it.
    #[test]
    fn every_ordinal_converts_to_its_declared_spelling() {
        for (ordinal, expected) in LEGACY_SHAPE_ORDINALS.iter().enumerate() {
            let mut node = serde_json::json!({ "style": { "shape_type": ordinal } });
            convert_shape(&mut node);
            assert_eq!(
                node["style"]["shape"].as_str(),
                Some(*expected),
                "miMind shape_type {ordinal} must convert to {expected:?}"
            );
        }
    }

    /// **The numbering, held to the published contract.**
    ///
    /// Turning the old `match val.as_i64()` — where each ordinal was
    /// written beside its spelling — into an array indexed by the
    /// ordinal bought the `KNOWN_SHAPES` pin above and introduced a
    /// hazard the `match` did not have: a row inserted in the middle
    /// renumbers every row below it, silently changing what a decade
    /// of old files convert to. So the mapping is published as a table
    /// in `format/migration.md` and read back here.
    #[test]
    fn legacy_shape_ordinals_match_the_published_table() {
        let published = published_shape_ordinals();
        assert_eq!(
            published.len(),
            LEGACY_SHAPE_ORDINALS.len(),
            "format/migration.md publishes {} shape_type ordinals, the \
             converter defines {}",
            published.len(),
            LEGACY_SHAPE_ORDINALS.len()
        );
        for (ordinal, spelling) in &published {
            assert_eq!(
                LEGACY_SHAPE_ORDINALS.get(*ordinal).copied(),
                Some(spelling.as_str()),
                "format/migration.md publishes shape_type {ordinal} as \
                 {spelling:?}; the converter would write {:?}",
                LEGACY_SHAPE_ORDINALS.get(*ordinal)
            );
        }
    }

    /// The `| ordinal | `spelling` |` rows of the `shape_type` table
    /// in `format/migration.md`.
    ///
    /// Panics if the section or the table is gone rather than
    /// returning an empty vector — the caller's length assertion would
    /// catch that too, but a pointed message beats `0 != 6`.
    fn published_shape_ordinals() -> Vec<(usize, String)> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../format/migration.md");
        let doc = std::fs::read_to_string(path).expect("format/migration.md must be readable");
        let section = doc
            .split_once("### miMind `shape_type` ordinals")
            .expect("format/migration.md must still publish the shape_type ordinals")
            .1;
        let section = section.split("\n## ").next().unwrap_or(section);

        let mut out = Vec::new();
        for line in section.lines() {
            let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
            if cells.len() != 2 {
                continue;
            }
            let Ok(ordinal) = cells[0].trim().parse::<usize>() else {
                continue;
            };
            out.push((ordinal, cells[1].trim().trim_matches('`').to_string()));
        }
        assert!(
            !out.is_empty(),
            "the shape_type section of format/migration.md has no ordinal rows"
        );
        out
    }
}
