// SPDX-License-Identifier: MPL-2.0

//! `mode` — query and change the active high-level interaction mode.
//!
//! Verbs:
//! - `mode show` — print the current mode.
//! - `mode default` — exit any active mode and return to `Default`.
//!   Equivalent to pressing the `CancelMode` keybind (Esc by default).
//! - `mode resize` — enter Resize mode targeting the current selection.
//!   Equivalent to `Action::EnterResizeMode` (`r` by default).
//!
//! Reparent and Connect modes are reachable via their own console
//! verbs (currently absent — they're keybind-only on `Ctrl+P` /
//! `Ctrl+D`). Adding `mode reparent` / `mode connect` here is a
//! later batch alongside the rest of the mode-verb surface; this
//! batch lands `mode show / default / resize` only — the slice the
//! resize-UX overhaul absolutely needs.
//!
//! See `SECTIONS_BORDERS_RESIZE_PLAN.md` §3.9 for the full target
//! grammar (`mode node-edit`, `mode section-edit`, etc.) which lands
//! in subsequent batches.

use super::Command;
use crate::application::app::{InteractionMode, ResizeTarget};
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ConsoleSideEffect, ExecResult};
use crate::application::document::SelectionState;

const VERBS: &[&str] = &["show", "default", "resize"];

pub const COMMAND: Command = Command {
    name: "mode",
    aliases: &[],
    summary: "Query or change the active interaction mode (Default / Resize / ...)",
    usage: "mode show | mode default | mode resize",
    tags: &["mode", "resize", "interaction", "default", "exit"],
    applicable: always,
    complete: complete_mode,
    execute: execute_mode,
};

fn complete_mode(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        _ => Vec::new(),
    }
}

fn execute_mode(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    match args.positional(0) {
        Some("show") => {
            // The console verb has no direct handle on the active
            // `InteractionMode` (it's not on `ConsoleEffects`).
            // Future batches that wire a status bar will surface
            // the mode there; for now point users at the
            // documented keybind exits.
            ExecResult::ok_msg(
                "mode: status display landing in Batch 3 (NodeEdit visuals); \
                 use `mode default` or Esc to exit any active mode",
            )
        }
        Some("default") => {
            eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default));
            eff.close_console = true;
            ExecResult::ok_msg("mode: returning to Default")
        }
        Some("resize") => match resolve_resize_target(&eff.document.selection, eff.document) {
            Ok(target) => {
                eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                    target,
                }));
                eff.close_console = true;
                ExecResult::ok_msg("entering resize mode")
            }
            Err(msg) => ExecResult::err(msg),
        },
        Some(other) => ExecResult::err(format!(
            "mode: unknown subverb '{}'; use 'show', 'default', or 'resize'",
            other
        )),
        None => ExecResult::err("usage: mode show | mode default | mode resize"),
    }
}

/// Mirror the `Action::EnterResizeMode` resolution rules — defined
/// once in `cross_dispatch::lifecycle::apply_enter_resize_mode` and
/// re-implemented here against `&MindMapDocument` rather than
/// `RebuildContext` because the console verb runs before the
/// dispatcher. Returns a typed error string the user sees in the
/// console scrollback.
fn resolve_resize_target(
    selection: &SelectionState,
    doc: &crate::application::document::MindMapDocument,
) -> Result<ResizeTarget, String> {
    match selection {
        SelectionState::Single(id) => Ok(ResizeTarget::Node(id.clone())),
        SelectionState::Section(s) | SelectionState::SectionRange { sel: s, .. } => {
            let section_size = doc
                .mindmap
                .nodes
                .get(&s.node_id)
                .and_then(|n| n.sections.get(s.section_idx))
                .and_then(|sec| sec.size);
            if section_size.is_none() {
                return Err(format!(
                    "mode resize: section {}[{}] is fill-parent (size=None) — no AABB to stretch. \
                     Pin a size first via `section resize <w> <h>`",
                    s.node_id, s.section_idx
                ));
            }
            Ok(ResizeTarget::Section {
                node_id: s.node_id.clone(),
                section_idx: s.section_idx,
            })
        }
        SelectionState::None => {
            Err("mode resize: no selection; click a node or section first".into())
        }
        SelectionState::Multi(_) | SelectionState::MultiSection(_) => {
            Err("mode resize: multi-target selection — single-target only".into())
        }
        SelectionState::Edge(_)
        | SelectionState::EdgeLabel(_)
        | SelectionState::PortalLabel(_)
        | SelectionState::PortalText(_) => {
            Err("mode resize: edge / label / portal selection — not resizable".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::parse;
    use crate::application::console::parser::ParseResult;
    use crate::application::console::Args;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::SectionSel;

    /// Parse `line` and run the `mode` verb body against `doc`,
    /// returning the result and the side-effect that the dispatcher
    /// would consume. Internal callers pass the doc separately
    /// rather than borrowing it inside the helper because
    /// `ConsoleEffects` borrows the doc for its lifetime; capturing
    /// the side effect requires extracting it before the borrow
    /// ends.
    fn run_mode(
        line: &str,
        doc: &mut crate::application::document::MindMapDocument,
    ) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens = match parse(line) {
            ParseResult::Ok { cmd: _, args } => args,
            ParseResult::Empty => panic!("empty input: {:?}", line),
            ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
        };
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_mode(&Args::new(&tokens), &mut eff);
        let close = eff.close_console;
        let side = eff.side_effect.take();
        (result, side, close)
    }

    fn assert_err_contains(r: &ExecResult, needle: &str) {
        match r {
            ExecResult::Err(msg) => assert!(msg.contains(needle), "msg = {}", msg),
            _ => panic!("expected Err containing {:?}, got non-Err", needle),
        }
    }

    #[test]
    fn mode_default_emits_set_default_side_effect() {
        let mut doc = load_test_doc();
        let (result, side, close) = run_mode("mode default", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        assert!(matches!(
            side,
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default))
        ));
        assert!(close);
    }

    #[test]
    fn mode_resize_with_single_node_selection_targets_node() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().expect("test doc has nodes").clone();
        doc.selection = SelectionState::Single(id.clone());
        let (result, side, _close) = run_mode("mode resize", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        match side {
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                target: ResizeTarget::Node(target_id),
            })) => assert_eq!(target_id, id),
            _ => panic!("expected SetInteractionMode(Resize::Node)"),
        }
    }

    #[test]
    fn mode_resize_with_no_selection_errors() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        let (result, side, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "no selection");
        // No mode flip on error.
        assert!(side.is_none());
    }

    #[test]
    fn mode_resize_with_multi_selection_errors() {
        let mut doc = load_test_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        assert_eq!(ids.len(), 2, "test doc must have at least 2 nodes");
        doc.selection = SelectionState::Multi(ids);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "single-target only");
    }

    #[test]
    fn mode_resize_with_fill_parent_section_errors() {
        let mut doc = load_test_doc();
        let nid = doc.mindmap.nodes.keys().next().expect("nodes").clone();
        // Force section 0 to fill-parent (size = None).
        if let Some(node) = doc.mindmap.nodes.get_mut(&nid) {
            if let Some(s) = node.sections.first_mut() {
                s.size = None;
            }
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: nid,
            section_idx: 0,
        });
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "fill-parent");
    }

    #[test]
    fn mode_unknown_subverb_errors() {
        let mut doc = load_test_doc();
        let (result, _, _) = run_mode("mode wibble", &mut doc);
        assert_err_contains(&result, "unknown subverb");
        if let ExecResult::Err(msg) = &result {
            assert!(msg.contains("'wibble'"), "msg = {}", msg);
        }
    }

    #[test]
    fn mode_no_subverb_emits_usage() {
        let mut doc = load_test_doc();
        let (result, _, _) = run_mode("mode", &mut doc);
        assert_err_contains(&result, "usage:");
    }
}
