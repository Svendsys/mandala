// SPDX-License-Identifier: MPL-2.0

//! Selection-changing apply_* helpers — `select_all` / `deselect_all`
//! / `invert` / `select_parent` / `select_child` / `select_sibling`.
//! Each pairs a pure-doc inner (`*_in`) function that returns
//! whether the selection actually changed with an outer wrapper
//! that triggers `rebuild_after_selection_change` on a real
//! change. The selection-only rebuild path skips clearing the
//! connection-sample cache because edge geometry hasn't shifted —
//! a meaningful saving on dense maps where every nav keystroke
//! would otherwise force a thousand-edge re-sample.

use crate::application::document::{MindMapDocument, SelectionState};

use super::RebuildContext;

// ── Selection ───────────────────────────────────────────────────
//
// Each Action arm is split into a pure-doc inner function (returns
// `bool` for "did the selection change"; cross-platform; unit-tested
// at the bottom of this module) and an outer `apply_*` wrapper that
// triggers the scene rebuild only when the inner function reports a
// change. Per `TEST_CONVENTIONS.md §T8`, renderer-touching outers
// are verified manually; the pure inners carry the test surface.

/// Set the document's selection to every visible node — hidden-by-
/// fold descendants are excluded so a follow-up `DeleteSelection`
/// can't silently nuke subtrees the user can't see. Returns `false`
/// only when the document has no visible nodes (empty doc); does
/// NOT detect "selection was already the same" (would require
/// `SelectionState: PartialEq`, which the enum doesn't derive
/// today). Matches the pre-Track-A unconditional-rebuild
/// behaviour for non-empty docs.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_all_in(doc: &mut MindMapDocument) -> bool {
    let all_ids: Vec<String> = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
        .map(|n| n.id.clone())
        .collect();
    if all_ids.is_empty() {
        return false;
    }
    doc.selection = SelectionState::from_ids(all_ids);
    true
}

pub(in crate::application::app) fn apply_select_all(rc: &mut RebuildContext<'_>) {
    if select_all_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Clear the selection. Returns `false` (no rebuild needed) when
/// nothing was selected.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn deselect_all_in(doc: &mut MindMapDocument) -> bool {
    if matches!(doc.selection, SelectionState::None) {
        return false;
    }
    doc.selection = SelectionState::None;
    true
}

pub(in crate::application::app) fn apply_deselect_all(rc: &mut RebuildContext<'_>) {
    if deselect_all_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Invert the current node selection. Edge / EdgeLabel / Portal*
/// selections are preserved (their `selected_ids()` is empty, so
/// inverting would collapse to "select every visible node" —
/// unintuitive). Hidden-by-fold nodes are filtered for the same
/// reason as `select_all_in`. Returns `true` when the selection
/// changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn invert_selection_in(doc: &mut MindMapDocument) -> bool {
    let invertable = matches!(
        doc.selection,
        SelectionState::None
            | SelectionState::Single(_)
            | SelectionState::Multi(_)
    );
    if !invertable {
        return false;
    }
    let selected: std::collections::HashSet<String> = doc
        .selection
        .selected_ids()
        .into_iter()
        .map(String::from)
        .collect();
    let inverted: Vec<String> = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !selected.contains(&n.id) && !doc.mindmap.is_hidden_by_fold(n))
        .map(|n| n.id.clone())
        .collect();
    doc.selection = SelectionState::from_ids(inverted);
    true
}

pub(in crate::application::app) fn apply_invert_selection(rc: &mut RebuildContext<'_>) {
    if invert_selection_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Walk one step up the hierarchy from a single-node selection.
/// No-op when the selection isn't a single node or the node has
/// no parent. Returns `true` when the selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_parent_in(doc: &mut MindMapDocument) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(parent_id) = doc
        .mindmap
        .nodes
        .get(&nid)
        .and_then(|n| n.parent_id.clone())
    else {
        return false;
    };
    doc.selection = SelectionState::Single(parent_id);
    true
}

pub(in crate::application::app) fn apply_select_parent(rc: &mut RebuildContext<'_>) {
    if select_parent_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Step into the first visible child (id-sorted) of the selected
/// single node. Folded children are skipped — keyboard navigation
/// shouldn't jump into a subtree the user can't see; mirrors the
/// fold-aware click hit-test policy. Returns `true` when the
/// selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_child_in(doc: &mut MindMapDocument) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(child_id) = doc
        .mindmap
        .children_of(&nid)
        .into_iter()
        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
        .map(|c| c.id.clone())
    else {
        return false;
    };
    doc.selection = SelectionState::Single(child_id);
    true
}

pub(in crate::application::app) fn apply_select_child(rc: &mut RebuildContext<'_>) {
    if select_child_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Step to the next or previous visible sibling of the selected
/// single node. `forward = true` walks toward the next sibling;
/// `false` walks back. No-op when the selection isn't a single
/// node, or when no visible neighbour exists in the requested
/// direction. Returns `true` when the selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_sibling_in(
    doc: &mut MindMapDocument,
    forward: bool,
) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(target) = sibling_id(&doc.mindmap, &nid, forward) else {
        return false;
    };
    doc.selection = SelectionState::Single(target);
    true
}

pub(in crate::application::app) fn apply_select_sibling(
    forward: bool,
    rc: &mut RebuildContext<'_>,
) {
    if select_sibling_in(rc.document, forward) {
        rc.rebuild_after_selection_change();
    }
}

/// Find the next or previous visible sibling of `nid` under the
/// same parent (or among root nodes when `nid` is a root). Skips
/// folded entries so keyboard navigation matches the fold-aware
/// click hit-test. Returns `None` when `nid` has no visible
/// neighbour in the requested direction.
fn sibling_id(
    map: &baumhard::mindmap::model::MindMap,
    nid: &str,
    forward: bool,
) -> Option<String> {
    let parent_id = map.nodes.get(nid).and_then(|n| n.parent_id.clone());
    let siblings: Vec<(String, bool)> = match parent_id {
        Some(pid) => map
            .children_of(&pid)
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
        None => map
            .root_nodes()
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
    };
    let idx = siblings.iter().position(|(id, _)| id == nid)?;
    if forward {
        siblings
            .iter()
            .skip(idx + 1)
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    } else {
        siblings
            .iter()
            .take(idx)
            .rev()
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    }
}

#[cfg(test)]
mod tests {
    //! Pure-doc-mutation tests for the selection helpers. The
    //! renderer-touching `apply_*` wrappers are out of scope per
    //! `TEST_CONVENTIONS.md §T8` (no live wgpu in unit tests); the
    //! tests below cover the cross-platform inner functions
    //! (`select_all_in`, `deselect_all_in`, etc.) that carry the
    //! actual logic. A regression in any of these silently changes
    //! WASM and native behaviour identically — the type-checker
    //! won't catch it.

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
        assert!(
            descendant_count > 0,
            "test fixture root must have descendants",
        );
        doc.mindmap
            .nodes
            .get_mut(&root_id)
            .unwrap()
            .folded = true;
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
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("fixture has edges")
            .clone();
        let er = crate::application::document::EdgeRef::new(
            &edge.from_id,
            &edge.to_id,
            &edge.edge_type,
        );
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
        let parent_id = doc.mindmap.nodes[&child_id]
            .parent_id
            .clone()
            .unwrap();
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

}
