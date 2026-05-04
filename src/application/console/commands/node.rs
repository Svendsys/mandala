// SPDX-License-Identifier: MPL-2.0

//! `node resize <w> <h>` and `node fit` — node-size verbs.
//!
//! Validation messages mirror the document-side `set_node_size`
//! and `fit_node_to_content` setters: finite + strictly positive
//! components and an absolute astronomical-typo ceiling
//! (`MAX_NODE_AXIS = 1_000_000`). Position stays unchanged for
//! `resize`; the drag gesture's release path uses `set_node_aabb`
//! for the atomic `(position, size)` write. `fit` reaches a
//! state the ambient `grow_*` passes can't (shrink to text floor).

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const VERBS: &[&str] = &["resize", "fit"];

pub const COMMAND: Command = Command {
    name: "node",
    aliases: &[],
    summary: "Resize the selected node atomically, or fit it to its content",
    usage: "node resize <w> <h> | node fit",
    tags: &["node", "resize", "size", "fit", "shrink", "content"],
    applicable: always,
    complete: complete_node,
    execute: execute_node,
};

fn complete_node(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        _ => Vec::new(),
    }
}

fn execute_node(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let verb = match args.positional(0) {
        Some(v) => v,
        None => return ExecResult::err("usage: node resize <w> <h> | node fit"),
    };
    match verb {
        "resize" => execute_resize(args, eff),
        "fit" => execute_fit(eff),
        other => ExecResult::err(format!("node: unknown subverb '{}'", other)),
    }
}

fn execute_fit(eff: &mut ConsoleEffects) -> ExecResult {
    let node_id = match &eff.document.selection {
        SelectionState::Single(id) => id.clone(),
        SelectionState::Section(s) => s.node_id.clone(),
        _ => {
            return ExecResult::err("node fit: requires a single-node or section selection");
        }
    };
    match eff.document.fit_node_to_content(&node_id) {
        Ok(true) => ExecResult::ok_msg(format!("node '{}' fitted to content", node_id)),
        Ok(false) => ExecResult::ok_msg("node fit: already at floor".to_string()),
        Err(msg) => ExecResult::err(msg),
    }
}

fn execute_resize(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let node_id = match &eff.document.selection {
        SelectionState::Single(id) => id.clone(),
        // A `Section` selection lifts to its owning node — the
        // verb operates on the *node*, not the section. Keeps
        // the typing path frictionless when the user has a
        // section selected and wants to resize the parent node.
        SelectionState::Section(s) => s.node_id.clone(),
        _ => {
            return ExecResult::err("node resize: requires a single-node or section selection");
        }
    };
    let w = match args.positional(1).and_then(|s| s.parse::<f64>().ok()) {
        Some(v) => v,
        None => return ExecResult::err("node resize: <w> must be a number"),
    };
    let h = match args.positional(2).and_then(|s| s.parse::<f64>().ok()) {
        Some(v) => v,
        None => return ExecResult::err("node resize: <h> must be a number"),
    };
    let new_size = baumhard::mindmap::model::Size {
        width: w,
        height: h,
    };
    match eff.document.set_node_size(&node_id, new_size) {
        Ok(true) => ExecResult::ok_msg(format!("node '{}' resized to {}×{}", node_id, w, h)),
        Ok(false) => ExecResult::ok_msg("node resize: no change".to_string()),
        Err(msg) => ExecResult::err(msg),
    }
}

#[cfg(test)]
mod tests {
    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};
    use crate::application::document::SelectionState;

    #[test]
    fn node_resize_single_writes_through_set_node_size() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // Use a target large enough to fit testament text so the
        // floor-respect pass doesn't rewrite the requested size.
        assert_exec_ok(run("node resize 800 400", &mut doc));
        let n = &doc.mindmap.nodes[&id];
        assert_eq!(n.size.width, 800.0);
        assert_eq!(n.size.height, 400.0);
    }

    #[test]
    fn node_resize_rejects_non_positive() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("node resize 0 50", &mut doc), "is not positive");
        assert_exec_err_contains(run("node resize -5 50", &mut doc), "is not positive");
    }

    #[test]
    fn node_resize_rejects_astronomical_typo() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id);
        // Absolute ceiling at 1_000_000 — values past it trip
        // the typo guard. Independent of the prior-size baseline.
        assert_exec_err_contains(
            run("node resize 2000000 50", &mut doc),
            "exceeds the",
        );
    }

    #[test]
    fn node_resize_with_section_selection_uses_owning_node() {
        use crate::application::document::SectionSel;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Simulate a section selection on this node — the verb
        // should resize the node, not the section. Use a target
        // large enough to fit testament text floor.
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        assert_exec_ok(run("node resize 800 400", &mut doc));
        let n = &doc.mindmap.nodes[&id];
        assert_eq!(n.size.width, 800.0);
        assert_eq!(n.size.height, 400.0);
    }

    #[test]
    fn node_resize_requires_node_or_section_selection() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        assert_exec_err_contains(run("node resize 50 50", &mut doc), "single-node or section");
    }

    /// `node fit` shrinks an over-sized node to its
    /// measured-text floor — the path that lets the user
    /// recover from a manual resize that pinned the node larger
    /// than its content.
    #[test]
    fn node_fit_shrinks_oversized_node() {
        use baumhard::mindmap::model::Size;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Pre-grow the node to be obviously over-sized — far
        // bigger than any text floor would compute.
        doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
            width: 5000.0,
            height: 5000.0,
        };
        doc.undo_stack.clear();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("node fit", &mut doc));
        let after = doc.mindmap.nodes[&id].size;
        assert!(
            after.width < 5000.0 && after.height < 5000.0,
            "fit-to-content should shrink to floor, got {}×{}",
            after.width,
            after.height
        );
        // Undo restores the over-sized state.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.nodes[&id].size.width, 5000.0);
    }

    #[test]
    fn node_fit_no_op_when_already_at_floor() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // First call lands at the floor.
        assert_exec_ok(run("node fit", &mut doc));
        let undo_after_first = doc.undo_stack.len();
        // Second call is a no-op.
        assert_exec_ok(run("node fit", &mut doc));
        assert_eq!(
            doc.undo_stack.len(),
            undo_after_first,
            "second fit-to-content must not push another undo entry"
        );
    }

    #[test]
    fn node_fit_with_section_selection_uses_owning_node() {
        use crate::application::document::SectionSel;
        use baumhard::mindmap::model::Size;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
            width: 5000.0,
            height: 5000.0,
        };
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        assert_exec_ok(run("node fit", &mut doc));
        let after = doc.mindmap.nodes[&id].size;
        assert!(after.width < 5000.0);
    }

    #[test]
    fn node_fit_requires_node_or_section_selection() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        assert_exec_err_contains(run("node fit", &mut doc), "single-node or section");
    }
}
