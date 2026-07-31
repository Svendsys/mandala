// SPDX-License-Identifier: MPL-2.0

//! Migrate the legacy top-level `portals` array to edges with
//! `display_mode = "portal"`. Field map:
//!   `endpoint_a`/`endpoint_b` → `from_id`/`to_id`
//!   `glyph`                   → `glyph_connection.body`
//!   `color`                   → `edge.color`
//!   `font`/`font_size_pt`     → `glyph_connection.{font,font_size_pt}`
//! `label` is dropped (post-refactor portals identify by edge tuple).
//!
//! The transform itself is [`fold_portals_into_edges`], which the
//! `--portals` verb and the `--legacy` pipeline both call. A legacy
//! miMind map can carry portals, so folding them is part of the one
//! legacy hop `format/migration.md` promises — not a separate
//! follow-up the user has to know to run.

use serde_json::{json, Value};
use std::path::Path;

/// Fold every entry of the legacy top-level `portals[]` array into a
/// portal-mode edge appended to `edges[]`, removing the `portals` key.
/// Returns the number of portals folded.
///
/// Idempotent: a tree with no `portals` key (already migrated) folds
/// zero portals and is left byte-identical. A `portals` value that is
/// present but not an array is dropped as unreadable rather than
/// failing the conversion — the current loader rejects the key in any
/// shape, so leaving it behind would keep the map unloadable.
///
/// Errors only on a root that is not a JSON object, or an `edges`
/// key that is not an array — both of which make the file unusable
/// as a mindmap regardless of portals.
pub(super) fn fold_portals_into_edges(root: &mut Value) -> Result<usize, String> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "map root must be a JSON object".to_string())?;
    let portals_array = match obj.remove("portals") {
        Some(Value::Array(a)) => a,
        // No portals field, or an unexpected shape: nothing to fold.
        Some(_) | None => return Ok(0),
    };

    let folded = portals_array.len();
    let edges = obj
        .entry("edges")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("map `edges` field is not an array")?;

    for portal in portals_array {
        let obj = match portal {
            Value::Object(o) => o,
            _ => continue,
        };
        let endpoint_a = obj
            .get("endpoint_a")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let endpoint_b = obj
            .get("endpoint_b")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // An empty `glyph` string would render as a zero-width marker
        // that's impossible to click. Treat `""` the same as missing
        // and fall back to the default marker glyph — a legacy file
        // carrying an empty glyph is almost certainly a bug in
        // whatever tool wrote it.
        let glyph = obj
            .get("glyph")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("\u{25C8}") // ◈ default
            .to_string();
        let color = obj
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("#aa88cc")
            .to_string();
        let font_size_pt = obj.get("font_size_pt").and_then(|v| v.as_f64()).unwrap_or(16.0);
        let font = obj.get("font").cloned().unwrap_or(Value::Null);

        let mut glyph_connection = serde_json::Map::new();
        glyph_connection.insert("body".into(), Value::String(glyph));
        glyph_connection.insert(
            "font_size_pt".into(),
            Value::Number(
                serde_json::Number::from_f64(font_size_pt).unwrap_or_else(|| serde_json::Number::from(16)),
            ),
        );
        if !font.is_null() {
            glyph_connection.insert("font".into(), font);
        }

        let edge = json!({
            "from_id": endpoint_a,
            "to_id": endpoint_b,
            "type": "cross_link",
            "color": color,
            "width": 3,
            "line_style": "solid",
            "visible": true,
            "label": Value::Null,
            "anchor_from": "auto",
            "anchor_to": "auto",
            "control_points": Value::Array(Vec::new()),
            "glyph_connection": Value::Object(glyph_connection),
            "display_mode": "portal",
        });
        edges.push(edge);
    }

    Ok(folded)
}

/// Read `input_path`, convert any `portals[]` entries into
/// portal-mode edges appended to `edges[]`, and write the result to
/// `output_path`. In-place migrations (input == output) are fine:
/// the read completes before the write begins, and the write uses
/// a temp-file + rename so a kill mid-write leaves the original
/// intact rather than truncated.
pub fn convert_portals(input_path: &Path, output_path: &Path) -> Result<(), String> {
    super::transform_map_file(input_path, output_path, |root| {
        let folded = fold_portals_into_edges(root)?;
        Ok(format!("converted {folded} portal(s) to portal-mode edges"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn no_portals_field_is_noop() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t","canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},"nodes":{{}},"edges":[]}}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        assert!(out.get("portals").is_none());
    }

    #[test]
    fn empty_glyph_falls_back_to_default_marker() {
        // A legacy portal with `glyph: ""` would migrate to an edge
        // with an empty `glyph_connection.body`, rendering as a
        // zero-width marker that's impossible to interact with.
        // The converter substitutes the default marker instead.
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"","color":"#ff0","font_size_pt":16.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();
        let body = &out.get("edges").unwrap().as_array().unwrap()[0]["glyph_connection"]["body"];
        assert_eq!(body.as_str().unwrap(), "\u{25C8}");
    }

    // Atomicity (no tmp leftover after successful rename) is tested
    // canonically in `baumhard::mindmap::loader::tests` against the
    // shared `write_atomic` helper that this migration now uses.

    #[test]
    fn legacy_portal_becomes_portal_mode_edge() {
        let mut src = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            src,
            r##"{{"version":"1.0","name":"t",
              "canvas":{{"background_color":"#000","default_border":null,"default_connection":null,"theme_variables":{{}},"theme_variants":{{}}}},
              "nodes":{{}},"edges":[],
              "portals":[{{"endpoint_a":"0","endpoint_b":"1","label":"A","glyph":"⬢","color":"#ff00aa","font_size_pt":20.0}}]
            }}"##
        )
        .unwrap();
        let dst = tempfile::NamedTempFile::new().unwrap();
        convert_portals(src.path(), dst.path()).unwrap();
        let out: Value = serde_json::from_str(&std::fs::read_to_string(dst.path()).unwrap()).unwrap();

        // portals field must be gone.
        assert!(out.get("portals").is_none());
        // edges must have one entry, display_mode=portal, body=⬢.
        let edges = out.get("edges").unwrap().as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["display_mode"], "portal");
        assert_eq!(edges[0]["from_id"], "0");
        assert_eq!(edges[0]["to_id"], "1");
        assert_eq!(edges[0]["color"], "#ff00aa");
        assert_eq!(edges[0]["glyph_connection"]["body"], "⬢");
        assert_eq!(edges[0]["glyph_connection"]["font_size_pt"], 20.0);
    }

    /// `format/migration.md` §"The fold, exactly" prints both halves
    /// of this transform as literal JSON. A doc example that drifts
    /// from the converter is worse than no example — a reader hand-
    /// migrating a map by that block would produce a file the loader
    /// rejects. Both string literals below are the verbatim text of
    /// those two fenced blocks; the assertion is that the converter
    /// turns the first into the second.
    #[test]
    fn documented_fold_matches_converter_output() {
        const DOC_LEGACY_PORTAL: &str = r##"{
  "endpoint_a": "0.0",
  "endpoint_b": "0.1",
  "label": "Cross-reference",
  "glyph": "⬢",
  "color": "#ff00aa",
  "font": "LiberationSans",
  "font_size_pt": 18.0
}"##;
        const DOC_PORTAL_EDGE: &str = r##"{
  "from_id": "0.0",
  "to_id": "0.1",
  "type": "cross_link",
  "color": "#ff00aa",
  "width": 3,
  "line_style": "solid",
  "visible": true,
  "label": null,
  "anchor_from": "auto",
  "anchor_to": "auto",
  "control_points": [],
  "glyph_connection": {
    "body": "⬢",
    "font": "LiberationSans",
    "font_size_pt": 18.0
  },
  "display_mode": "portal"
}"##;

        let legacy_portal: Value = serde_json::from_str(DOC_LEGACY_PORTAL)
            .expect("the documented legacy portal block must be valid JSON");
        let mut root = json!({ "edges": [], "portals": [legacy_portal] });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 1);

        let expected: Value = serde_json::from_str(DOC_PORTAL_EDGE)
            .expect("the documented portal-edge block must be valid JSON");
        let produced = &root["edges"].as_array().unwrap()[0];
        assert_eq!(
            produced, &expected,
            "converter output drifted from format/migration.md's documented fold"
        );

        // ...and the documented shape is one the typed model accepts,
        // so a reader who copies the block by hand gets a loadable map.
        serde_json::from_str::<baumhard::mindmap::model::MindEdge>(DOC_PORTAL_EDGE)
            .expect("the documented portal edge must deserialize as a MindEdge");
    }

    /// A `portals` key holding something other than an array is
    /// dropped rather than carried through: the loader rejects the
    /// key in every shape, so preserving it would leave the output
    /// unloadable for no gain.
    #[test]
    fn non_array_portals_key_is_dropped() {
        let mut root = json!({ "edges": [], "portals": "not an array" });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 0);
        assert!(root.get("portals").is_none());
    }

    /// A legacy map with no `edges` key at all still gets its portals:
    /// the fold creates the array rather than silently dropping them.
    #[test]
    fn fold_creates_edges_array_when_absent() {
        let mut root = json!({
            "portals": [{ "endpoint_a": "0", "endpoint_b": "1" }]
        });
        assert_eq!(fold_portals_into_edges(&mut root).unwrap(), 1);
        let edges = root["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["display_mode"], "portal");
        // Defaults documented in format/migration.md.
        assert_eq!(edges[0]["glyph_connection"]["body"], "\u{25C8}");
        assert_eq!(edges[0]["color"], "#aa88cc");
        assert_eq!(edges[0]["glyph_connection"]["font_size_pt"], 16.0);
        assert!(edges[0]["glyph_connection"].get("font").is_none());
    }
}
