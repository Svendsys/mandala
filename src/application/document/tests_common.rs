// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures used by the `tests_*` submodules and by every
//! console / document test outside the `document/` tree that
//! needs the testament map. The single-source-of-truth loader
//! (`load_test_doc`) caches one parsed `MindMap` in a process-wide
//! `OnceLock` and clones it per call — this avoids the
//! `FONT_SYSTEM` write-lock contention `MindMapDocument::load`
//! would otherwise trigger N times in a parallel test run (each
//! call hits `finalize` → `grow_node_sizes_to_fit_text` →
//! per-node lock acquisition). The cache itself is harmless for
//! tests that mutate the doc — every caller gets a fresh clone
//! and the cached `MindMap` is untouched.
//!
//! Visibility: `pub(crate)` under `#[cfg(test)]` so callers in
//! `console/commands/*` and other crate-test scopes outside
//! `document/` can re-use the same loader (per `TEST_CONVENTIONS.md`
//! "the project owns one fixture loader, not five").

use std::path::PathBuf;
use std::sync::OnceLock;

use baumhard::mindmap::loader;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::MindMapDocument;

pub(crate) fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("maps/testament.mindmap.json");
    path
}

/// Process-wide cache for the testament `MindMap`. Filled lazily
/// on first call to [`load_test_doc`]; subsequent calls clone
/// from the cache. The clone cost is one walk over the node /
/// edge / palette `HashMap`s — far cheaper than the JSON parse,
/// and *much* cheaper than the per-node FONT_SYSTEM write-lock
/// acquisitions a `MindMapDocument::load` call would otherwise
/// trigger.
static CACHED_TESTAMENT_MAP: OnceLock<MindMap> = OnceLock::new();

/// Load the testament map into a fresh `MindMapDocument` shell.
/// Backed by a `OnceLock` so the JSON parse only happens once
/// per process; subsequent calls clone the cached `MindMap` into
/// a new doc shell via [`MindMapDocument::from_finalized_mindmap`].
///
/// Skips `finalize` (the grow-node-sizes-to-fit-text + border
/// passes) since the testament map's authored sizes already
/// accommodate its text and borders. Tests that explicitly need
/// to exercise `finalize` (e.g. the load-time auto-resize test
/// in `tests_nodes.rs`) build their own synthetic fixture and
/// route through `MindMapDocument::from_json_str`.
pub(crate) fn load_test_doc() -> MindMapDocument {
    let map = CACHED_TESTAMENT_MAP.get_or_init(|| {
        loader::load_from_file(&test_map_path())
            .expect("testament map parses")
    });
    MindMapDocument::from_finalized_mindmap(map.clone(), None)
}

pub(super) fn load_test_tree() -> MindMapTree {
    load_test_doc().build_tree()
}

/// Pick the first node id the testament map's loader exposes
/// from its `nodes` HashMap. Stable within one process because
/// the cached fixture clone is the same `MindMap` every call.
/// Used by tests that just need *some* node to operate on; tests
/// that want the well-known root specifically should index by
/// `"0"` directly so the dependency on the testament shape is
/// visible at the call site.
pub(crate) fn first_testament_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .keys()
        .next()
        .cloned()
        .expect("testament map has nodes")
}

/// Pick the first visible edge and return its EdgeRef + a guaranteed
/// on-path sample point. Used by hit-test edge tests.
pub(super) fn pick_test_edge(doc: &MindMapDocument) -> (super::EdgeRef, glam::Vec2) {
    use glam::Vec2;
    let edge = doc.mindmap.edges.iter()
        .find(|e| e.visible)
        .expect("testament map has visible edges");
    let from = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let to = doc.mindmap.nodes.get(&edge.to_id).unwrap();
    let from_pos = Vec2::new(from.position.x as f32, from.position.y as f32);
    let from_size = Vec2::new(from.size.width as f32, from.size.height as f32);
    let to_pos = Vec2::new(to.position.x as f32, to.position.y as f32);
    let to_size = Vec2::new(to.size.width as f32, to.size.height as f32);
    let path = baumhard::mindmap::connection::build_connection_path(
        from_pos, from_size, &edge.anchor_from,
        to_pos, to_size, &edge.anchor_to,
        &edge.control_points,
    );
    let samples = baumhard::mindmap::connection::sample_path(&path, 4.0);
    let midpoint = samples[samples.len() / 2].position;
    let edge_ref = super::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    (edge_ref, midpoint)
}

/// Grab the first edge from the testament map and return its EdgeRef.
pub(super) fn first_testament_edge_ref(doc: &MindMapDocument) -> super::EdgeRef {
    let e = doc.mindmap.edges.first().expect("testament map has edges");
    super::EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type)
}
