// SPDX-License-Identifier: MPL-2.0

//! `node resize <w> <h>` — sets a node's `size` atomically.
//!
//! Validation messages mirror the document-side `set_node_size`
//! setter: finite + strictly positive components, astronomical-typo
//! guard (>100× the prior dimension on the same axis).
//! Position stays unchanged — this verb writes size only; the
//! drag gesture's release path uses `set_node_aabb` for the
//! atomic `(position, size)` write.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const VERBS: &[&str] = &["resize"];

pub const COMMAND: Command = Command {
    name: "node",
    aliases: &[],
    summary: "Resize the selected node atomically",
    usage: "node resize <w> <h>",
    tags: &["node", "resize", "size"],
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
        None => return ExecResult::err("usage: node resize <w> <h>"),
    };
    match verb {
        "resize" => execute_resize(args, eff),
        other => ExecResult::err(format!("node: unknown subverb '{}'", other)),
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
}
