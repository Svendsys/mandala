// SPDX-License-Identifier: MPL-2.0

//! `mode` — query and change the active high-level interaction mode.
//!
//! Verbs:
//! - `mode default` — exit any active mode and return to `Default`.
//!   Equivalent to pressing the `ExitMode` keybind (Esc by default).
//! - `mode resize` — enter Resize mode targeting the current selection.
//!   Equivalent to `Action::EnterResizeMode` (`r` by default).
//!
//! `mode show` is intentionally absent until Batch 3 plumbs the
//! active mode through `ConsoleEffects` (it has no clean surface to
//! read mode from today, so a stub would be a half-feature per
//! CODE_CONVENTIONS §5). Reparent / Connect transitions remain
//! keybind-only (`Ctrl+P` / `Ctrl+D`); their console-verb surface
//! lands alongside the mode-verb expansion in a later batch.
//!
//! See `SECTIONS_BORDERS_RESIZE_PLAN.md` §3.9 for the full target
//! grammar (`mode node-edit`, `mode section-edit`, `mode reparent`,
//! `mode connect`, `mode show`) that lands in subsequent batches.

use super::Command;
use crate::application::app::{resolve_resize_target, InteractionMode, ResizeTargetError};
#[cfg(test)]
use crate::application::app::ResizeTarget;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ConsoleSideEffect, ExecResult};

const VERBS: &[&str] = &["default", "resize"];

pub const COMMAND: Command = Command {
    name: "mode",
    aliases: &[],
    summary: "Change the active interaction mode (default / resize)",
    usage: "mode default | mode resize",
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
        Some("default") => {
            eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default));
            eff.close_console = true;
            ExecResult::ok_msg("mode: returning to Default")
        }
        Some("resize") => match resolve_resize_target(&eff.document.selection, &eff.document.mindmap) {
            Ok(target) => {
                eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                    target,
                }));
                eff.close_console = true;
                ExecResult::ok_msg("entering resize mode")
            }
            Err(e) => ExecResult::err(format_resize_error(&e)),
        },
        Some(other) => ExecResult::err(format!(
            "mode: unknown subverb '{}'; use 'default' or 'resize'",
            other
        )),
        None => ExecResult::err("usage: mode default | mode resize"),
    }
}

/// Format a [`ResizeTargetError`] (returned by the shared
/// [`crate::application::app::resolve_resize_target`] resolver) as a
/// user-facing console message prefixed with the verb name. The
/// underlying error type is the same one the dispatcher arm
/// (`apply_enter_resize_mode`) consumes, with `log::warn!` lines
/// rather than console output — keeping the resolver one source of
/// truth while letting each consumer phrase its own user surface.
fn format_resize_error(e: &ResizeTargetError) -> String {
    match e {
        ResizeTargetError::NoSelection => {
            "mode resize: no selection; click a node or section first".into()
        }
        ResizeTargetError::MultiTarget => {
            "mode resize: multi-target selection — single-target only".into()
        }
        ResizeTargetError::SectionFillParent { node_id, section_idx } => format!(
            "mode resize: section {}[{}] is fill-parent (size=None) — no AABB to stretch. \
             Pin a size first via `section resize <w> <h>`",
            node_id, section_idx,
        ),
        ResizeTargetError::EdgeOrPortal => {
            "mode resize: edge / label / portal selection — not resizable".into()
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
    use crate::application::document::{
        EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState,
    };
    use baumhard::mindmap::scene_cache::EdgeKey;

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
