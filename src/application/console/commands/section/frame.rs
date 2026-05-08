// SPDX-License-Identifier: MPL-2.0

//! `section frame …` — configure a section's per-section frame
//! border style.
//!
//! Mirrors the [`crate::application::console::commands::border`]
//! kv vocabulary (`preset`, `font`, `size`, `color`, `palette`,
//! `field`, `padding`, `top`, `bottom`, `left`, `right`, `tl`,
//! `tr`, `bl`, `br`) but writes to
//! [`baumhard::mindmap::model::MindSection::frame_border`] instead
//! of `MindNode.style.border`. Same parsing, same auto-promotion
//! behaviour (any `top=` / corner edit promotes `preset` to
//! `"custom"`), same per-side pattern grammar.
//!
//! Section frames don't carry a visibility flag — they're drawn
//! whenever the owning node is in `InteractionMode::NodeEdit`, so
//! there's no `section frame on` / `off`. Subverbs are:
//!
//! - `section frame show` — readout of the resolved frame style.
//! - `section frame reset` — drop the per-section override (falls
//!   back to `Canvas.default_section_frame_border` and then to the
//!   hardcoded floor).
//! - kv form: any combination of the keys above.
//!
//! The target section is resolved from the current selection
//! (`SelectionState::Section` / `SectionRange`) or via an explicit
//! `section=K` kv when the selection is `Single(node_id)`.

use baumhard::mindmap::border::resolve_section_frame_border;

use crate::application::console::commands::border::{
    nodes_in_selection as border_nodes_in_selection, stage_kv, KEYS as BORDER_KEYS,
};
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{BorderConfigEdits, BorderEditOutcome, OptionEdit, SectionSel, SelectionState};

/// Subverbs surfaced as token-2 completions after `section frame`.
pub const VERBS: &[&str] = &["show", "reset"];

pub fn complete_section_frame(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        // tokens[0] = "section", tokens[1] = "frame", tokens[2..] is
        // the same shape as a top-level `border …` invocation.
        CompletionContext::Token { index: 2 } => {
            let mut out = prefix_filter(VERBS, state.partial);
            out.extend(kv_key_completions_with_hints(
                BORDER_KEYS,
                state.partial,
                kv_hint,
            ));
            out
        }
        CompletionContext::Token { index: i } if *i > 2 => {
            kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint)
        }
        _ => Vec::new(),
    }
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "preset" => Some("light | heavy | double | rounded | custom"),
        "font" => Some("font family for border glyphs (use `font list` for names)"),
        "size" => Some("border glyph size in points"),
        "color" => Some("#hex, var(--name), preset, or 'reset'"),
        "palette" => Some("palette name to cycle per-glyph colours, or 'off'"),
        "field" => Some("frame | background | text | title"),
        "padding" => Some("border-to-content padding in pixels"),
        "top" | "bottom" | "left" | "right" => Some("side pattern: `prefix(fill)suffix` or atomic"),
        "tl" | "tr" | "bl" | "br" => Some("single corner glyph (escapes apply)"),
        _ => None,
    }
}

/// Entry point dispatched from `section/mod.rs::execute_section`
/// when the user typed `section frame …`. Args still includes the
/// `frame` token at positional(0) (the parent dispatcher only
/// consumed `section`); we read positional(1) to peek at the
/// optional `show` / `reset` subverb.
pub fn execute_section_frame(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if let Some(verb) = args.positional(1) {
        match verb {
            "show" => return execute_show(args, eff),
            "reset" => return apply_reset(args, eff),
            other if !other.contains('=') => {
                if args.kvs().next().is_some() {
                    return ExecResult::err(format!(
                        "section frame: unexpected positional '{}' alongside a kv pair — \
                         did you mean to quote a multi-word value? \
                         e.g. `section frame palette=\"{}\"`",
                        other, other
                    ));
                }
                return ExecResult::err(format!(
                    "section frame: unknown subverb '{}'; use 'show', 'reset', or kv form",
                    other
                ));
            }
            _ => {}
        }
    }

    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        if k == "section" {
            // The `section=K` kv targets the section to write to;
            // it's not a border field. Skip it on the staging
            // pass — the resolver below consumes it separately.
            continue;
        }
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err(
            "usage: section frame show|reset | section frame <key>=<value> … [section=<idx>]",
        );
    }
    apply_edits(args, eff, edits)
}

fn apply_reset(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let mut edits = BorderConfigEdits::default();
    edits.clear = true;
    apply_edits(args, eff, edits)
}

fn apply_edits(args: &Args, eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    let node_ids = match border_nodes_in_selection(&eff.document.selection) {
        Ok(ids) => ids,
        Err(_) => {
            return ExecResult::err(
                "section frame: select a section (or a node + pass section=<idx>) first",
            );
        }
    };
    let kv_idx = match parse_section_kv(args) {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };

    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let mut changed = 0usize;
    let mut auto_promoted: Option<String> = None;

    for node_id in &node_ids {
        let section_idx = match resolve_section_idx_for(&eff.document.selection, node_id, kv_idx) {
            Ok(idx) => idx,
            Err(msg) => return ExecResult::err(msg),
        };
        let count = eff
            .document
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        if section_idx >= count {
            return ExecResult::err(format!(
                "section[{}] not found on node '{}'",
                section_idx, node_id
            ));
        }
        let outcome: BorderEditOutcome =
            eff.document
                .set_section_frame_border_config(node_id, section_idx, edits.clone());
        if outcome.changed {
            changed += 1;
        }
        if outcome.preset_auto_promoted && auto_promoted.is_none() {
            auto_promoted = outcome.requested_preset.clone();
        }
    }

    let mut lines: Vec<String> = Vec::new();
    if changed == 0 {
        if bare_custom {
            lines.push("section frame: preset=custom set; no glyph fields were given".into());
            lines.push(custom_preset_hint());
            return ExecResult::lines(lines);
        }
        return ExecResult::ok_msg("section frame: no change");
    }
    lines.push(format!("section frame applied to {} section(s)", changed));
    if let Some(name) = auto_promoted {
        lines.push(format!(
            "note: preset='{}' auto-promoted to 'custom' \
             (a side or corner glyph was set; non-custom presets \
             ignore the per-section glyph override)",
            name
        ));
    }
    if bare_custom {
        lines.push(custom_preset_hint());
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
}

fn execute_show(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let node_ids = match border_nodes_in_selection(&eff.document.selection) {
        Ok(ids) => ids,
        Err(_) => {
            return ExecResult::err(
                "section frame show: select a section (or a node + pass section=<idx>) first",
            );
        }
    };
    if node_ids.len() != 1 {
        return ExecResult::err(
            "section frame show: single-section target only; pick one section first",
        );
    }
    let node_id = node_ids.into_iter().next().expect("len==1");
    let kv_idx = match parse_section_kv(args) {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let section_idx = match resolve_section_idx_for(&eff.document.selection, &node_id, kv_idx) {
        Ok(idx) => idx,
        Err(msg) => return ExecResult::err(msg),
    };
    let map = &eff.document.mindmap;
    let Some(node) = map.nodes.get(&node_id) else {
        return ExecResult::err(format!("section frame show: node '{}' not found", node_id));
    };
    let Some(section) = node.sections.get(section_idx) else {
        return ExecResult::err(format!(
            "section[{}] not found on node '{}'",
            section_idx, node_id
        ));
    };

    // Resolve through the same cascade the renderer uses so the
    // readout matches what's drawn on screen, even when the
    // section has no override.
    let resolved = resolve_section_frame_border(section, &map.canvas, false, "#00E5FF");
    let source = if section.frame_border.is_some() {
        "per-section override"
    } else if map.canvas.default_section_frame_border.is_some() {
        "canvas default"
    } else {
        "hardcoded floor"
    };
    let lines = vec![
        format!("section frame: node='{}' section={}", node_id, section_idx),
        format!("  source:    {}", source),
        format!("  font:      {}", resolved.font_name.as_deref().unwrap_or("(default)")),
        format!("  size:      {} pt", resolved.font_size_pt),
        format!("  color:     {}", resolved.color),
        format!(
            "  palette:   {}",
            resolved
                .color_palette
                .as_deref()
                .map(|n| format!("{} (field={})", n, resolved.palette_field.as_str()))
                .unwrap_or_else(|| "(none)".into())
        ),
    ];
    ExecResult::lines(lines)
}

fn parse_section_kv(args: &Args) -> Result<Option<usize>, String> {
    for (k, v) in args.kvs() {
        if k == "section" {
            return super::super::range_kv::parse_section_kv("section", v).map(Some);
        }
    }
    Ok(None)
}

fn resolve_section_idx_for(
    sel: &SelectionState,
    node_id: &str,
    kv_idx: Option<usize>,
) -> Result<usize, String> {
    if let Some(idx) = kv_idx {
        return Ok(idx);
    }
    match sel {
        SelectionState::Section(SectionSel { node_id: nid, section_idx }) if nid == node_id => {
            Ok(*section_idx)
        }
        SelectionState::SectionRange { sel: SectionSel { node_id: nid, section_idx }, .. }
            if nid == node_id =>
        {
            Ok(*section_idx)
        }
        _ => Err(format!(
            "section frame: node '{}' has multiple sections — pass section=<idx>",
            node_id
        )),
    }
}

fn edits_has_glyph_field(edits: &BorderConfigEdits) -> bool {
    !matches!(edits.side_top, OptionEdit::Keep)
        || !matches!(edits.side_bottom, OptionEdit::Keep)
        || !matches!(edits.side_left, OptionEdit::Keep)
        || !matches!(edits.side_right, OptionEdit::Keep)
        || !matches!(edits.corner_top_left, OptionEdit::Keep)
        || !matches!(edits.corner_top_right, OptionEdit::Keep)
        || !matches!(edits.corner_bottom_left, OptionEdit::Keep)
        || !matches!(edits.corner_bottom_right, OptionEdit::Keep)
}

fn custom_preset_hint() -> String {
    "hint: 'custom' is the preset that lets you author per-side / per-corner glyphs. \
     Combine it with any of: top=… bottom=… left=… right=… tl=… tr=… bl=… br=…  \
     e.g. `section frame preset=custom top=\"###(*)###\" tl=\"+\" tr=\"+\" bl=\"+\" br=\"+\"`. \
     See `format/border-patterns.md` for the side-pattern grammar."
        .to_string()
}

#[cfg(test)]
mod tests {
    use crate::application::console::tests::fixtures::{
        assert_exec_err_contains, assert_exec_ok, run,
    };
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::pinned_two_section_node;
    use crate::application::document::{SectionSel, SelectionState};

    #[test]
    fn section_frame_preset_writes_section_frame_border() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.preset, "heavy");
    }

    #[test]
    fn section_frame_preset_does_not_touch_node_style_border() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().style.border.is_none(),
            "node-level border must not be created by section frame edits"
        );
    }

    #[test]
    fn section_frame_kv_overrides_selection_index() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section frame preset=double section=0", &mut doc));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[0]
            .frame_border
            .as_ref()
            .expect("section[0].frame_border populated");
        assert_eq!(cfg.preset, "double");
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "section[1] untouched"
        );
    }

    #[test]
    fn section_frame_top_pattern_auto_promotes_preset_to_custom() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // `preset=heavy` plus a side glyph: the staged top forces
        // the preset to "custom" because non-custom presets ignore
        // the per-side override. Verb returns `Lines` because the
        // auto-promotion note rides alongside the success message.
        let result = run("section frame preset=heavy top=\"###(*)###\"", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.preset, "custom");
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated by side edit");
        assert_eq!(glyphs.top, "###(*)###");
    }

    #[test]
    fn section_frame_reset_clears_per_section_override() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=double", &mut doc));
        assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .is_some());
        assert_exec_ok(run("section frame reset", &mut doc));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "reset must drop the per-section override"
        );
    }

    #[test]
    fn section_frame_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy color=#ff8800", &mut doc));
        assert!(doc.undo());
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "undo restores the absent prior frame_border"
        );
    }

    #[test]
    fn section_frame_no_change_does_not_grow_undo_stack() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        let depth_after_first = doc.undo_stack.len();
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        assert_eq!(
            doc.undo_stack.len(),
            depth_after_first,
            "an idempotent edit must not push an undo entry"
        );
    }

    #[test]
    fn section_frame_unknown_key_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frame frob=baz", &mut doc), "unknown key");
    }

    #[test]
    fn section_frame_out_of_range_section_kv_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(
            run("section frame preset=heavy section=99", &mut doc),
            "not found on node",
        );
    }

    #[test]
    fn section_frame_show_reports_resolved_config() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=double color=#cd00cd", &mut doc));
        let result = run("section frame show", &mut doc);
        let lines = match result {
            ExecResult::Lines(ls) => ls,
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        let blob = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(blob.contains("section="), "show must include section index: {}", blob);
        assert!(
            blob.contains("per-section override"),
            "show must label the source: {}",
            blob
        );
        assert!(blob.contains("#cd00cd"), "show must surface the color: {}", blob);
    }

    #[test]
    fn section_frame_bare_preset_custom_emits_glyph_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section frame preset=custom", &mut doc);
        let blob: String = match result {
            ExecResult::Lines(ls) => ls
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            ExecResult::Ok(s) => s,
            other => panic!("expected lines or ok, got {:?}", other),
        };
        assert!(
            blob.contains("preset=custom"),
            "bare preset=custom should mention what was set: {}",
            blob
        );
        assert!(
            blob.contains("top=") || blob.contains("glyph"),
            "bare preset=custom should hint at the glyph fields: {}",
            blob
        );
    }

    #[test]
    fn section_frame_palette_kv_lands_on_section_only() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        assert_exec_ok(run(
            "section frame preset=light palette=rainbow field=frame",
            &mut doc,
        ));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[0]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.color_palette.as_deref(), Some("rainbow"));
        assert_eq!(cfg.color_palette_field.as_deref(), Some("frame"));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "section[1] must not be touched"
        );
    }

    #[test]
    fn section_frame_no_args_errors_with_usage() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frame", &mut doc), "usage:");
    }
}
