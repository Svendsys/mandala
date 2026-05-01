// SPDX-License-Identifier: MPL-2.0

//! Pure-doc-mutation tests for the selection bucket's inner
//! `select_*_in` helpers — `select_all_in` / `deselect_all_in` /
//! `invert_selection_in` / `select_parent_in` / `select_child_in` /
//! `select_sibling_in`. The renderer-touching `apply_*` wrappers
//! are out of scope per `TEST_CONVENTIONS.md §T8` (no live wgpu
//! in unit tests); the tests below cover the cross-platform inner
//! functions that carry the actual logic. A regression in any of
//! these silently changes WASM and native behaviour identically —
//! the type-checker won't catch it.

#![cfg(test)]

use super::*;
use crate::application::document::tests_common::load_test_doc;

fn first_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .keys()
        .next()
        .expect("test fixture has nodes")
        .clone()
}

fn first_root_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .root_nodes()
        .first()
        .expect("test fixture has at least one root")
        .id
        .clone()
}

#[test]
fn select_all_in_picks_every_visible_node() {
    let mut doc = load_test_doc();
    let visible_count = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
        .count();
    assert!(visible_count > 0, "fixture has visible nodes");
    let changed = select_all_in(&mut doc);
    assert!(changed);
    let selected = doc.selection.selected_ids();
    assert_eq!(selected.len(), visible_count);
}

#[test]
fn select_all_in_excludes_folded_descendants() {
    let mut doc = load_test_doc();
    // Pick a non-leaf root and fold it.
    let root_id = first_root_id(&doc);
    let descendant_count = doc.mindmap.all_descendants(&root_id).len();
    assert!(descendant_count > 0, "test fixture root must have descendants",);
    doc.mindmap.nodes.get_mut(&root_id).unwrap().folded = true;
    let total_visible_before_fold = doc.mindmap.nodes.len();
    let _ = select_all_in(&mut doc);
    let selected = doc.selection.selected_ids();
    // The folded root itself is still visible; only its
    // descendants are hidden.
    assert!(selected.iter().any(|id| *id == root_id));
    assert_eq!(selected.len(), total_visible_before_fold - descendant_count);
}

#[test]
fn select_all_in_returns_false_when_no_visible_nodes() {
    let mut doc = MindMapDocument::new_blank(None);
    // Empty document → no visible nodes → no-op + false.
    assert!(!select_all_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn deselect_all_in_clears_selection() {
    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    assert!(deselect_all_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn deselect_all_in_returns_false_when_already_none() {
    let mut doc = load_test_doc();
    // Default selection is None.
    assert!(!deselect_all_in(&mut doc));
}

#[test]
fn invert_selection_in_skips_edge_selection() {
    let mut doc = load_test_doc();
    let edge = doc.mindmap.edges.first().expect("fixture has edges").clone();
    let er = crate::application::document::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.selection = SelectionState::Edge(er.clone());
    // Edge selections are NOT invertable — the helper preserves
    // them (selecting "every visible node" via inversion would
    // be unintuitive).
    assert!(!invert_selection_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::Edge(_)));
}

#[test]
fn invert_selection_in_inverts_node_selection() {
    let mut doc = load_test_doc();
    let pivot = first_node_id(&doc);
    doc.selection = SelectionState::Single(pivot.clone());
    assert!(invert_selection_in(&mut doc));
    // Pivot is no longer in the selection.
    assert!(!doc.selection.selected_ids().iter().any(|id| **id == pivot));
    // Every other visible node IS in the selection.
    let expected = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| n.id != pivot && !doc.mindmap.is_hidden_by_fold(n))
        .count();
    assert_eq!(doc.selection.selected_ids().len(), expected);
}

#[test]
fn select_parent_in_walks_up_one_level() {
    let mut doc = load_test_doc();
    // Pick a non-root node to start.
    let child_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .expect("fixture has a non-root node")
        .id
        .clone();
    let parent_id = doc.mindmap.nodes[&child_id].parent_id.clone().unwrap();
    doc.selection = SelectionState::Single(child_id);
    assert!(select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &parent_id
    ));
}

#[test]
fn select_parent_in_no_op_at_root() {
    let mut doc = load_test_doc();
    let root_id = first_root_id(&doc);
    doc.selection = SelectionState::Single(root_id.clone());
    // Roots have no parent — no-op + false.
    assert!(!select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &root_id
    ));
}

#[test]
fn select_parent_in_no_op_for_multi_selection() {
    let mut doc = load_test_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    doc.selection = SelectionState::from_ids(ids);
    assert!(!select_parent_in(&mut doc));
}

#[test]
fn select_child_in_steps_into_first_visible_child() {
    let mut doc = load_test_doc();
    let parent_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| !doc.mindmap.children_of(&n.id).is_empty())
        .expect("fixture has a parent node")
        .id
        .clone();
    let expected_child = doc
        .mindmap
        .children_of(&parent_id)
        .into_iter()
        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
        .expect("at least one visible child")
        .id
        .clone();
    doc.selection = SelectionState::Single(parent_id);
    assert!(select_child_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &expected_child
    ));
}

#[test]
fn select_child_in_no_op_for_leaf() {
    let mut doc = load_test_doc();
    // Find a node with no children.
    let leaf_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| doc.mindmap.children_of(&n.id).is_empty())
        .expect("fixture has a leaf")
        .id
        .clone();
    doc.selection = SelectionState::Single(leaf_id);
    assert!(!select_child_in(&mut doc));
}

#[test]
fn select_sibling_in_walks_visible_neighbour() {
    let mut doc = load_test_doc();
    // Find a node with at least one sibling.
    let (start_id, _next_id) = doc
        .mindmap
        .nodes
        .values()
        .filter_map(|n| {
            let parent = n.parent_id.as_ref()?;
            let siblings = doc.mindmap.children_of(parent);
            if siblings.len() < 2 {
                return None;
            }
            let idx = siblings.iter().position(|s| s.id == n.id)?;
            let next = siblings.get(idx + 1)?.id.clone();
            Some((n.id.clone(), next))
        })
        .next()
        .expect("fixture has at least one node with a next sibling");
    doc.selection = SelectionState::Single(start_id);
    assert!(select_sibling_in(&mut doc, true));
    // Walking back returns to the previous sibling.
    assert!(select_sibling_in(&mut doc, false));
}
