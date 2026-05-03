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
    let map = CACHED_TESTAMENT_MAP
        .get_or_init(|| loader::load_from_file(&test_map_path()).expect("testament map parses"));
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
pub(in crate::application) fn first_testament_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .keys()
        .next()
        .cloned()
        .expect("testament map has nodes")
}

/// Pick the first `n` node ids from the testament map. Used by
/// multi-selection fanout tests that want a `Multi(ids)`
/// selection of arbitrary cardinality without picking specific
/// ids. Panics if the map has fewer than `n` nodes — the testament
/// fixture is large enough that any reasonable `n` succeeds.
pub(in crate::application) fn first_n_testament_node_ids(doc: &MindMapDocument, n: usize) -> Vec<String> {
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(n).cloned().collect();
    assert!(
        ids.len() == n,
        "testament map has {} nodes; needed {}",
        ids.len(),
        n
    );
    ids
}

/// Pick two distinct node ids in `(a, b)` form — the shape
/// portal-edge / cross-link tests want when they need a
/// from-and-to pair without caring which specific nodes those are.
pub(in crate::application) fn two_testament_node_ids(doc: &MindMapDocument) -> (String, String) {
    let mut iter = doc.mindmap.nodes.keys();
    let a = iter.next().expect("testament map has at least one node").clone();
    let b = iter.next().expect("testament map has at least two nodes").clone();
    (a, b)
}

/// Build a fresh blank `MindMapDocument` carrying a single
/// orphan node `"0"` at the origin. Lighter than
/// [`load_test_doc`] for tests that just need a writable doc
/// with at least one selectable node — no testament JSON
/// parse, no font-system contention. Lifted from byte-
/// identical helpers that previously sat inline in
/// `zoom_bounds.rs` and `edges.rs` test modules.
pub(in crate::application) fn doc_with_one_orphan_node() -> MindMapDocument {
    use std::collections::{HashMap, HashSet};
    let mut doc = MindMapDocument {
        mindmap: MindMap::new_blank("t"),
        file_path: None,
        dirty: false,
        selection: super::SelectionState::None,
        undo_stack: Vec::new(),
        mutation_registry: HashMap::new(),
        mutation_sources: HashMap::new(),
        mutation_handlers: HashMap::new(),
        active_toggles: HashSet::new(),
        label_edit_preview: None,
        portal_text_edit_preview: None,
        color_picker_preview: None,
        active_animations: Vec::new(),
    };
    let node = super::defaults::default_orphan_node("0", glam::Vec2::ZERO);
    doc.mindmap.nodes.insert("0".to_string(), node);
    doc
}

/// Build a fresh `MindMapDocument` carrying two orphan nodes
/// (`"0"` and `"1"`) plus a default parent_child edge between
/// them. Returns the doc paired with an [`super::EdgeRef`]
/// pointing at the edge so callers don't have to reconstruct
/// the triple. The shape every drag / undo / mutate-edge unit
/// test wanted.
pub(in crate::application) fn doc_with_one_edge() -> (MindMapDocument, super::EdgeRef) {
    let mut doc = doc_with_one_orphan_node();
    doc.mindmap.nodes.insert(
        "1".to_string(),
        super::defaults::default_orphan_node("1", glam::Vec2::ZERO),
    );
    let edge = super::defaults::default_parent_child_edge("0", "1");
    let er = super::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.mindmap.edges.push(edge);
    (doc, er)
}

/// Materialise `node_id` into a two-section node with one pinned
/// text run per section. Sets the node's `style.text_color` to
/// `text_color_default` so the cascade source the section colour
/// setter consults (and the colour picker reads) resolves against
/// a known anchor. Each section's single run carries the colour
/// at the matching index in `section_run_colors`; both share
/// `font` and `size_pt`. The pre-existing first section's `text`
/// field is preserved (only its runs are replaced).
///
/// Used across the Tier 2A section-routing tests that previously
/// re-implemented this scaffold in four near-identical inline
/// copies (commands/color, commands/font, console/tests/
/// wheel_dispatch, color_picker/tests/targets).
pub(in crate::application) fn make_two_section_node_with_pinned_runs(
    doc: &mut MindMapDocument,
    node_id: &str,
    text_color_default: &str,
    section_run_colors: [&str; 2],
    font: &str,
    size_pt: u32,
) {
    use baumhard::mindmap::model::{MindSection, TextRun};
    let node = doc.mindmap.nodes.get_mut(node_id).expect("node id exists in doc");
    node.sections
        .push(MindSection::new_default("second".into(), Vec::new()));
    node.style.text_color = text_color_default.into();
    for (i, section) in node.sections.iter_mut().enumerate() {
        section.text_runs.clear();
        section.text_runs.push(TextRun {
            start: 0,
            end: section.text.chars().count().max(1),
            bold: false,
            italic: false,
            underline: false,
            font: font.into(),
            size_pt,
            color: section_run_colors[i].into(),
            hyperlink: None,
        });
    }
}

/// Pick the first visible edge and return its EdgeRef + a guaranteed
/// on-path sample point. Used by hit-test edge tests.
pub(super) fn pick_test_edge(doc: &MindMapDocument) -> (super::EdgeRef, glam::Vec2) {
    let edge = doc
        .mindmap
        .edges
        .iter()
        .find(|e| e.visible)
        .expect("testament map has visible edges");
    let from = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let to = doc.mindmap.nodes.get(&edge.to_id).unwrap();
    let from_pos = from.pos_vec2();
    let from_size = from.size_vec2();
    let to_pos = to.pos_vec2();
    let to_size = to.size_vec2();
    let path = baumhard::mindmap::connection::build_connection_path(
        from_pos,
        from_size,
        &edge.anchor_from,
        to_pos,
        to_size,
        &edge.anchor_to,
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

/// Builder for a `CustomMutation` carrying a single `NudgeRight`
/// area command. Replaces three near-identical local factories
/// that diverged only in default magnitude (`make_test_mutation`
/// at 10.0, `make_cm` at 1.0, `make_animated_mutation` at 100.0)
/// and a four-position positional helper that grew thin per-call-
/// site wrappers everywhere it landed.
///
/// Two required fields (`id`, `scope`); everything else has a
/// default and a chainable setter. Each setter takes `self` by
/// value and returns `Self`, so call sites read as a fluent chain
/// terminating in `.build()`:
///
/// ```ignore
/// let cm = TestNudgeMutation::new("nudge", TargetScope::SelfOnly)
///     .magnitude(10.0)
///     .build();
/// ```
///
/// Cost: trivial — one struct instantiation + one
/// `CustomMutation` build per `.build()`.
pub(in crate::application) struct TestNudgeMutation {
    id: String,
    scope: baumhard::mindmap::custom_mutation::TargetScope,
    magnitude: f32,
    contexts: Vec<String>,
    description: String,
    timing: Option<baumhard::mindmap::animation::AnimationTiming>,
}

impl TestNudgeMutation {
    /// Start a builder with the required `id` and target scope.
    /// Magnitude defaults to 1.0; everything else empty / `None`.
    pub(in crate::application) fn new(
        id: &str,
        scope: baumhard::mindmap::custom_mutation::TargetScope,
    ) -> Self {
        Self {
            id: id.to_string(),
            scope,
            magnitude: 1.0,
            contexts: Vec::new(),
            description: String::new(),
            timing: None,
        }
    }

    /// Override the `NudgeRight` magnitude in canvas pixels.
    pub(in crate::application) fn magnitude(mut self, magnitude: f32) -> Self {
        self.magnitude = magnitude;
        self
    }

    /// Replace the `contexts` list (default empty).
    pub(in crate::application) fn contexts(mut self, contexts: Vec<String>) -> Self {
        self.contexts = contexts;
        self
    }

    /// Replace the `description` field (default empty).
    pub(in crate::application) fn description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    /// Attach an `AnimationTiming` so the mutation becomes
    /// animated. Default `None` produces an instant mutation.
    pub(in crate::application) fn timing(
        mut self,
        timing: baumhard::mindmap::animation::AnimationTiming,
    ) -> Self {
        self.timing = Some(timing);
        self
    }

    /// Materialise the `CustomMutation`. Consumes the builder.
    pub(in crate::application) fn build(self) -> baumhard::mindmap::custom_mutation::CustomMutation {
        use baumhard::gfx_structs::area::GlyphAreaCommand;
        use baumhard::gfx_structs::mutator::Mutation;
        use baumhard::mindmap::custom_mutation::{CustomMutation, MutationBehavior};
        CustomMutation {
            id: self.id.clone(),
            name: self.id,
            description: self.description,
            contexts: self.contexts,
            mutator: Some(baumhard::mindmap::custom_mutation::scope::self_only(vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(self.magnitude)),
            ])),
            target_scope: self.scope,
            behavior: MutationBehavior::Persistent,
            predicate: None,
            document_actions: vec![],
            timing: self.timing,
        }
    }
}
