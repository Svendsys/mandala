// SPDX-License-Identifier: MPL-2.0

//! One-way migration from the legacy miMind-derived `.mindmap.json`
//! format to the current one. Submodules perform one transform each
//! (IDs, enums, palettes, cleanup); pipeline order is fixed in
//! `convert_legacy`.

mod cleanup;
mod enums;
mod ids;
mod palettes;
mod portals;
mod sections;

pub use portals::convert_portals;
pub use sections::convert_sections;

use serde_json::Value;
use std::path::Path;

/// Mutable handle to `root.nodes`. Returns `None` when missing or
/// wrong-typed — passes treat those as already-clean.
fn nodes_obj_mut(root: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    root.get_mut("nodes").and_then(|v| v.as_object_mut())
}

/// Mutable handle to `root.edges`.
fn edges_arr_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    root.get_mut("edges").and_then(|v| v.as_array_mut())
}

/// Mutable handle to legacy `root.portals` (rejected by the
/// current-format loader; only legacy passes touch it).
fn portals_arr_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    root.get_mut("portals").and_then(|v| v.as_array_mut())
}

/// Read a legacy `.mindmap.json`, convert it to the current format, and
/// write the result to `output_path`.
pub fn convert_legacy(input_path: &Path, output_path: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;

    let mut root: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    // Order matters: IDs first so the rest can rewrite references;
    // enums before palettes so `theme_id` is gone before the palette
    // hoist; cleanup before the section fold so any legacy `text` /
    // `text_runs` survives the cleanup pass and gets folded into a
    // `sections[]` array at the end. Sections last so an
    // already-cleaned tree converges on the post-section shape.
    let nodes = root
        .get("nodes")
        .and_then(|v| v.as_object())
        .ok_or("missing or invalid \"nodes\" object")?;
    let id_map = ids::assign_dewey_ids(nodes);
    ids::rewrite_ids(&mut root, &id_map);
    enums::convert_enums(&mut root);
    palettes::hoist_palettes(&mut root);
    cleanup::cleanup_nodes(&mut root);
    if let Some(nodes) = nodes_obj_mut(&mut root) {
        for (_id, node) in nodes.iter_mut() {
            sections::migrate_one_node(node);
        }
    }

    let json = serde_json::to_string_pretty(&root).map_err(|e| format!("failed to serialize: {e}"))?;

    std::fs::write(output_path, &json)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    eprintln!(
        "converted {} nodes, {} edges",
        id_map.len(),
        root.get("edges")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );

    Ok(())
}
