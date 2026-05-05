// SPDX-License-Identifier: MPL-2.0

//! `section move <dx> <dy>` and `section resize <w> <h>` — per-
//! section position and size verbs targeting either the selection's
//! section (when the selection is `SelectionState::Section`) or an
//! explicit `section=K` kv (when the selection is a single node).
//!
//! Validation messages mirror `crates/maptool/src/verify/sections.rs`
//! so a verb-rejected move and a `verify` violation read identically.
//! `section resize none` flips a section's `size` back to `None`
//! (fill-parent) — the only console-side path to that state today.

use super::Command;
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SectionSel, SelectionState};

pub const KEYS: &[&str] = &["section"];
pub const VERBS: &[&str] = &["move", "resize"];

pub const COMMAND: Command = Command {
    name: "section",
    aliases: &[],
    summary: "Move or resize a section relative to its owning node",
    usage:
        "section move <dx> <dy> [section=<idx>] | section resize <w> <h> [section=<idx>] | section resize none [section=<idx>]",
    tags: &["section", "move", "resize", "offset", "size"],
    applicable: always,
    complete: complete_section,
    execute: execute_section,
};

fn complete_section(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        CompletionContext::Token { index: 1 } => match state.tokens.first().map(String::as_str) {
            Some("resize") => prefix_filter(&["none"], state.partial),
            _ => Vec::new(),
        },
        CompletionContext::Token { .. } => kv_key_completions_with_hints(KEYS, state.partial, kv_hint),
        CompletionContext::KvValue { key } if key == "section" => Vec::new(),
        _ => Vec::new(),
    }
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "section" => Some("target section index inside a multi-section node"),
        _ => None,
    }
}

fn execute_section(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let verb = match args.positional(0) {
        Some(v) => v,
        None => {
            return ExecResult::err(
                "usage: section move <dx> <dy> | section resize <w> <h> | section resize none",
            )
        }
    };
    let target_idx = match resolve_section_idx(args, &eff.document.selection) {
        Ok(idx) => idx,
        Err(msg) => return ExecResult::err(msg),
    };
    let node_id = match resolve_node_id(&eff.document.selection) {
        Ok(id) => id,
        Err(msg) => return ExecResult::err(msg),
    };
    // Verify the index resolves before delegating — explicit
    // `section=99` should error, not silently return "no change"
    // (indistinguishable from a successful idempotent set).
    let section_count = eff
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);
    if target_idx >= section_count {
        return ExecResult::err(format!("section[{}] not found on node '{}'", target_idx, node_id));
    }
    match verb {
        "move" => execute_move(args, eff.document, &node_id, target_idx),
        "resize" => execute_resize(args, eff.document, &node_id, target_idx),
        other => ExecResult::err(format!("section: unknown subverb '{}'", other)),
    }
}

/// Resolve `(node_id, section_idx)` from the current selection +
/// optional `section=K` kv. A `Section` selection supplies both;
/// a `Single` selection requires the kv (no implicit default —
/// authors who want section 0 specifically should say so).
fn resolve_section_idx(args: &Args, selection: &SelectionState) -> Result<usize, String> {
    let kv_idx = parse_section_kv(args)?;
    match (selection, kv_idx) {
        (_, Some(idx)) => Ok(idx),
        (SelectionState::Section(SectionSel { section_idx, .. }), None) => Ok(*section_idx),
        (SelectionState::SectionRange { sel: SectionSel { section_idx, .. }, .. }, None) => {
            Ok(*section_idx)
        }
        (SelectionState::Single(_), None) => {
            Err("section: select a specific section (multi-section node) or pass section=<idx>".into())
        }
        // Section move / resize is single-target by design (each
        // gesture writes one section's offset / size). Fan-out
        // across a MultiSection would imply each section moves
        // by the same delta — semantically valid for `move` but
        // ambiguous for `resize` (different starting sizes
        // produce different post-resize shapes per section). For
        // both, surface a clearer error than the generic
        // "requires a node or section" pre-N3 message.
        (SelectionState::MultiSection(_), None) => Err(
            "section: multi-section selection — single-target only; pass section=<idx> or click one section first".into(),
        ),
        _ => Err("section: requires a node or section selection".into()),
    }
}

fn resolve_node_id(selection: &SelectionState) -> Result<String, String> {
    if let Some(id) = selection.primary_node_id() {
        return Ok(id.to_string());
    }
    if matches!(selection, SelectionState::MultiSection(_)) {
        return Err(
            "section: multi-section selection — single-target only; pass section=<idx> or click one section first".into(),
        );
    }
    Err("section: requires a node or section selection".into())
}

fn parse_section_kv(args: &Args) -> Result<Option<usize>, String> {
    for (k, v) in args.kvs() {
        if k == "section" {
            return super::range_kv::parse_section_kv("section", v).map(Some);
        }
    }
    Ok(None)
}

fn execute_move(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    let dx = match parse_positional_f64(args, 1, "dx") {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let dy = match parse_positional_f64(args, 2, "dy") {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let (current_x, current_y) = match doc
        .mindmap
        .nodes
        .get(node_id)
        .and_then(|n| n.sections.get(idx))
        .map(|s| (s.offset.x, s.offset.y))
    {
        Some(p) => p,
        None => return ExecResult::err(format!("section[{}] not found on node '{}'", idx, node_id)),
    };
    match doc.set_section_offset(node_id, idx, current_x + dx, current_y + dy) {
        Ok(true) => ExecResult::ok_msg(format!("section[{}] moved", idx)),
        Ok(false) => ExecResult::ok_msg("section: no change"),
        Err(msg) => ExecResult::err(msg),
    }
}

fn execute_resize(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    if args.positional(1).map(str::to_ascii_lowercase).as_deref() == Some("none") {
        return match doc.set_section_size(node_id, idx, None) {
            Ok(true) => ExecResult::ok_msg(format!("section[{}] size cleared (fill parent)", idx)),
            Ok(false) => ExecResult::ok_msg("section: no change"),
            Err(msg) => ExecResult::err(msg),
        };
    }
    let w = match parse_positional_f64(args, 1, "w") {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let h = match parse_positional_f64(args, 2, "h") {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let new_size = baumhard::mindmap::model::Size { width: w, height: h };
    match doc.set_section_size(node_id, idx, Some(new_size)) {
        Ok(true) => ExecResult::ok_msg(format!("section[{}] resized", idx)),
        Ok(false) => ExecResult::ok_msg("section: no change"),
        Err(msg) => ExecResult::err(msg),
    }
}

fn parse_positional_f64(args: &Args, index: usize, name: &str) -> Result<f64, String> {
    let raw = args
        .positional(index)
        .ok_or_else(|| format!("section: missing positional <{}>", name))?;
    raw.parse::<f64>()
        .map_err(|_| format!("section: <{}>='{}' is not a number", name, raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};

    #[test]
    fn section_move_writes_offset_when_section_selection_supplies_idx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section move 5 7", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 15.0);
        assert_eq!(s.offset.y, 17.0);
    }

    #[test]
    fn section_move_kv_overrides_selection_idx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section move 3 4 section=1", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 13.0);
        assert_eq!(s.offset.y, 14.0);
    }

    #[test]
    fn section_move_rejects_when_single_selection_lacks_section_kv() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("section move 3 4", &mut doc), "select a specific section");
    }

    #[test]
    fn section_move_rejects_aabb_overflow_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // section[1] starts at offset (10,10) size 50×30; node is
        // 200×100. Moving by (200,0) puts right edge at 260 > 200.
        assert_exec_err_contains(
            run("section move 200 0", &mut doc),
            "extends past node right edge",
        );
    }

    #[test]
    fn section_move_rejects_negative_offset_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // Move (-50, 0) from offset (10,10) → -40, would-be negative.
        assert_exec_err_contains(
            run("section move -50 0", &mut doc),
            "section[1].offset.x is negative",
        );
    }

    #[test]
    fn section_move_rejects_unparseable_dx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section move not-a-number 0", &mut doc), "not a number");
    }

    #[test]
    fn section_move_no_change_returns_ok_msg() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section move 0 0", &mut doc);
        assert!(matches!(result, ExecResult::Ok(_)));
    }

    #[test]
    fn section_move_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section move 7 3", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 17.0);
        assert_eq!(s.offset.y, 13.0);
        assert!(doc.undo());
        let restored = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(restored.offset.x, 10.0, "undo restores prior offset");
        assert_eq!(restored.offset.y, 10.0);
    }

    /// Out-of-range `section=K` errors at the verb layer rather
    /// than silently returning "no change" — pre-fix the setter's
    /// `Ok(false)` for unknown sections was indistinguishable
    /// from a successful idempotent set.
    #[test]
    fn section_move_out_of_range_section_kv_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("section move 1 1 section=99", &mut doc), "not found on node");
    }

    #[test]
    fn section_resize_writes_size() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section resize 80 40", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.size.as_ref().unwrap().width, 80.0);
        assert_eq!(s.size.as_ref().unwrap().height, 40.0);
    }

    #[test]
    fn section_resize_none_clears_size() {
        let (mut doc, id) = pinned_two_section_node();
        // The fixture pins section[1] at offset (10, 10) with
        // an explicit size; `section resize none` flatten-to-
        // fill-parent is only legal at offset (0, 0) post the
        // effective-size fix, so reset offset first.
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section resize none", &mut doc));
        assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size.is_none());
    }

    #[test]
    fn section_resize_rejects_overflow_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // Offset (10,10) + width 250 = 260 > node.size.width 200.
        assert_exec_err_contains(
            run("section resize 250 30", &mut doc),
            "extends past node right edge",
        );
    }

    #[test]
    fn section_resize_rejects_zero_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section resize 0 30", &mut doc), "is not positive");
    }

    #[test]
    fn section_resize_rejects_astronomical_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // node.size.width=200, 100× = 20000. 25000 trips the typo guard.
        assert_exec_err_contains(
            run("section resize 25000 30", &mut doc),
            "over 100× the node's width",
        );
    }

    #[test]
    fn section_resize_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let before = doc.mindmap.nodes.get(&id).unwrap().sections[1].size.clone();
        assert_exec_ok(run("section resize 80 40", &mut doc));
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&id).unwrap().sections[1].size.clone();
        assert_eq!(restored, before, "undo restores prior size");
    }

    #[test]
    fn section_unknown_subverb_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frobnicate 1 2", &mut doc), "unknown subverb");
    }

    #[test]
    fn section_no_selection_errors() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        assert_exec_err_contains(
            run("section move 1 1", &mut doc),
            "requires a node or section selection",
        );
    }
}
