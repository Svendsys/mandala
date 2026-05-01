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
        SelectionState::None | SelectionState::Single(_) | SelectionState::Multi(_)
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
    let Some(parent_id) = doc.mindmap.nodes.get(&nid).and_then(|n| n.parent_id.clone()) else {
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
pub(in crate::application::app) fn select_sibling_in(doc: &mut MindMapDocument, forward: bool) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(target) = sibling_id(&doc.mindmap, &nid, forward) else {
        return false;
    };
    doc.selection = SelectionState::Single(target);
    true
}

pub(in crate::application::app) fn apply_select_sibling(forward: bool, rc: &mut RebuildContext<'_>) {
    if select_sibling_in(rc.document, forward) {
        rc.rebuild_after_selection_change();
    }
}

/// Find the next or previous visible sibling of `nid` under the
/// same parent (or among root nodes when `nid` is a root). Skips
/// folded entries so keyboard navigation matches the fold-aware
/// click hit-test. Returns `None` when `nid` has no visible
/// neighbour in the requested direction.
fn sibling_id(map: &baumhard::mindmap::model::MindMap, nid: &str, forward: bool) -> Option<String> {
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

mod tests;
