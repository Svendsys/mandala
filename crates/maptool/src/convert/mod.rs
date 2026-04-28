// SPDX-License-Identifier: MPL-2.0

//! One-way migration from the miMind-derived legacy `.mindmap.json`
//! format to the current one.
//!
//! The current format's loader has no runtime compatibility shim for
//! legacy files: `portals[]` is rejected outright by the loader and
//! every other legacy-shaped field (opaque integer IDs, enum codes,
//! inlined palettes, `index`) trips serde's own type mismatch on
//! parse. Either way, an unmigrated file does not load. This module
//! is how a user crosses the one-way door: each submodule performs
//! one orthogonal transform (IDs, enums, palettes, cleanup) and the
//! whole pipeline runs in a fixed order so later passes can assume
//! the earlier ones have already landed.

mod cleanup;
mod enums;
mod ids;
mod palettes;
mod portals;

pub use portals::convert_portals;

use serde_json::Value;
use std::path::Path;

/// Drill into `root.nodes` as a mutable JSON object, returning
/// `None` when the field is missing or wrong-typed (the
/// "silently no-op" posture the per-pass cleanups rely on —
/// they treat absent/wrong-typed sections as already-clean).
/// Single source of truth for the prelude every convert sub-pass
/// previously hand-rolled.
fn nodes_obj_mut(root: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    root.get_mut("nodes").and_then(|v| v.as_object_mut())
}

/// Drill into `root.edges` as a mutable JSON array. Sibling of
/// [`nodes_obj_mut`] for the legacy edge-array shape.
fn edges_arr_mut(root: &mut Value) -> Option<&mut Vec<Value>> {
    root.get_mut("edges").and_then(|v| v.as_array_mut())
}

/// Drill into `root.portals` as a mutable JSON array. Used only
/// by the legacy-format passes — current-format files reject
/// `portals[]` at the loader.
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

    // 1. Assign Dewey-decimal IDs and rewrite all references.
    let nodes = root
        .get("nodes")
        .and_then(|v| v.as_object())
        .ok_or("missing or invalid \"nodes\" object")?;
    let id_map = ids::assign_dewey_ids(nodes);
    ids::rewrite_ids(&mut root, &id_map);

    // 2. Convert integer enums to named strings.
    enums::convert_enums(&mut root);

    // 3. Hoist color schemas into top-level palettes.
    palettes::hoist_palettes(&mut root);

    // 4. Drop index, add channel.
    cleanup::cleanup_nodes(&mut root);

    // Write output with sorted keys for deterministic output.
    let json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize: {e}"))?;

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
