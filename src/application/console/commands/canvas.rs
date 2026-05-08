// SPDX-License-Identifier: MPL-2.0

//! `canvas …` — map-wide default editing.
//!
//! Sets the canvas-level fallbacks every node / section uses when
//! it has no per-node / per-section override. Subverbs:
//!
//! - `canvas border show|reset|<key>=<value> …` — writes
//!   `Canvas.default_border`. The map-wide default border every
//!   framed node falls back to.
//! - `canvas section-frame show|reset|<key>=<value> …` — writes
//!   `Canvas.default_section_frame_border`. The map-wide default
//!   for the cyan rectangle around an unfocused section in
//!   NodeEdit mode.
//! - `canvas section-frame focused show|reset|<key>=<value> …` —
//!   writes `Canvas.default_focused_section_frame_border`. The
//!   map-wide default for the focused section's frame.
//!
//! All three accept the same kv vocabulary the per-node `border …`
//! and per-section `section frame …` verbs use (preset, font,
//! size, color, palette, field, padding, top, bottom, left,
//! right, tl, tr, bl, br). Auto-promotion of preset to "custom"
//! on side / corner edits matches the per-node / per-section
//! behaviour.
//!
//! Undo: each successful canvas edit pushes a single
//! `UndoAction::CanvasSnapshot` so undo restores every canvas
//! field in one step (theme variables, palettes, defaults — all
//! captured together by design).

use baumhard::mindmap::border::resolve_border_style;
use baumhard::mindmap::model::GlyphBorderConfig;

use super::border::{custom_preset_hint, edits_has_glyph_field, stage_kv, KEYS as BORDER_KEYS};
use super::Command;
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{BorderConfigEdits, BorderEditOutcome, OptionEdit};

/// Subverbs surfaced as token-0 completions.
pub const VERBS: &[&str] = &["border", "section-frame"];
/// Subverbs surfaced under `border` / `section-frame`.
pub const SUBVERBS: &[&str] = &["show", "reset"];
/// Modifier under `section-frame` (followed by show|reset|kv).
pub const SECTION_FRAME_MODIFIERS: &[&str] = &["focused"];

pub const COMMAND: Command = Command {
    name: "canvas",
    aliases: &[],
    summary: "Edit map-wide canvas defaults (border, section frame)",
    usage:
        "canvas border show|reset|<key>=<value> … | canvas section-frame [focused] show|reset|<key>=<value> …",
    tags: &[
        "canvas",
        "default",
        "border",
        "section-frame",
        "frame",
        "preset",
        "glyph",
        "palette",
        "padding",
    ],
    applicable: always,
    complete: complete_canvas,
    execute: execute_canvas,
};

fn complete_canvas(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    // `state.tokens[0]` is the command name ("canvas"); the first
    // subject (`border` / `section-frame`) lives at index 1. The
    // engine's `Token { index: 0 }` counts past the command, so it
    // represents the first positional after `canvas`.
    let subject = state.tokens.get(1).map(String::as_str);
    match &state.context {
        // First positional after `canvas`: offer the subjects.
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        // Second positional, branched on subject:
        //   - after `border`: show/reset + kv keys
        //   - after `section-frame`: `focused`, show/reset, kv keys
        CompletionContext::Token { index: 1 } => match subject {
            Some("border") => {
                let mut out = prefix_filter(SUBVERBS, state.partial);
                out.extend(kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint));
                out
            }
            Some("section-frame") => {
                let mut out = prefix_filter(SECTION_FRAME_MODIFIERS, state.partial);
                out.extend(prefix_filter(SUBVERBS, state.partial));
                out.extend(kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint));
                out
            }
            _ => Vec::new(),
        },
        // Anything past index 1 is always kv-form.
        CompletionContext::Token { .. } => kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint),
        // Per-key value completions (preset/palette/font/color/field)
        // mirror the top-level `border …` popup vocabulary so the
        // popup is identical regardless of which border surface the
        // user is editing.
        CompletionContext::KvValue { key } => {
            super::border::kv_value_completions(key.as_str(), state.partial, ctx)
        }
        _ => Vec::new(),
    }
}

/// Per-key hint table — delegates to the shared
/// [`super::border::kv_hint`] so `border …`, `section frame …`, and
/// `canvas …` surface identical hints.
fn kv_hint(key: &str) -> Option<&'static str> {
    super::border::kv_hint(key)
}

pub fn execute_canvas(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let subject = match args.positional(0) {
        Some(v) => v,
        None => {
            return ExecResult::err(
                "usage: canvas border|section-frame [focused] show|reset|<key>=<value> …",
            );
        }
    };
    // Subject and subverb names are accepted case-insensitively
    // throughout the console — matches the policy at
    // `border/execute.rs:308` (preset names) and `section/mod.rs`
    // (the `none` literal). Picking lowercase here means downstream
    // exact-match arms work without extra ceremony.
    match subject.to_ascii_lowercase().as_str() {
        "border" => execute_border_subject(args, eff),
        "section-frame" => execute_section_frame_subject(args, eff),
        _ => ExecResult::err(format!(
            "canvas: unknown subverb '{}'; use 'border' or 'section-frame'",
            subject
        )),
    }
}

fn execute_border_subject(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // tokens[1] is either show/reset or the first kv. Skip the
    // `subject` positional; everything else mirrors the
    // per-node `border` verb's kv-form path. Case-insensitive
    // for parity with the rest of the verb.
    if let Some(verb) = args.positional(1) {
        match verb.to_ascii_lowercase().as_str() {
            "show" => return execute_show_border(eff),
            "reset" => return apply_border_edits(eff, clear_edits()),
            other if !other.contains('=') => {
                return ExecResult::err(format!(
                    "canvas border: unknown subverb '{}'; use 'show', 'reset', or kv form",
                    other
                ));
            }
            _ => {}
        }
    }

    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err("usage: canvas border show|reset|<key>=<value> …");
    }
    apply_border_edits(eff, edits)
}

fn execute_section_frame_subject(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // tokens[1] may be the `focused` modifier or the first subverb /
    // kv. Match case-insensitively so the user's casing tolerance
    // is uniform across the verb.
    let focused = args
        .positional(1)
        .map(|t| t.eq_ignore_ascii_case("focused"))
        .unwrap_or(false);
    let verb_pos = if focused { 2 } else { 1 };

    if let Some(verb) = args.positional(verb_pos) {
        match verb.to_ascii_lowercase().as_str() {
            "show" => return execute_show_section_frame(eff, focused),
            "reset" => return apply_section_frame_edits(eff, focused, clear_edits()),
            other if !other.contains('=') => {
                return ExecResult::err(format!(
                    "canvas section-frame{}: unknown subverb '{}'; use 'show', 'reset', or kv form",
                    if focused { " focused" } else { "" },
                    other
                ));
            }
            _ => {}
        }
    }

    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err("usage: canvas section-frame [focused] show|reset|<key>=<value> …");
    }
    apply_section_frame_edits(eff, focused, edits)
}

fn clear_edits() -> BorderConfigEdits {
    BorderConfigEdits {
        clear: true,
        ..BorderConfigEdits::default()
    }
}

fn apply_border_edits(eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let outcome: BorderEditOutcome = eff.document.set_canvas_default_border_config(edits);
    finish(outcome, "canvas border", bare_custom)
}

fn apply_section_frame_edits(
    eff: &mut ConsoleEffects,
    focused: bool,
    edits: BorderConfigEdits,
) -> ExecResult {
    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let outcome: BorderEditOutcome = eff
        .document
        .set_canvas_default_section_frame_border_config(focused, edits);
    let label = if focused {
        "canvas section-frame focused"
    } else {
        "canvas section-frame"
    };
    finish(outcome, label, bare_custom)
}

fn finish(outcome: BorderEditOutcome, label: &str, bare_custom: bool) -> ExecResult {
    if !outcome.changed {
        if bare_custom {
            return ExecResult::lines(vec![
                format!("{}: preset=custom set; no glyph fields were given", label),
                custom_preset_hint(label),
            ]);
        }
        return ExecResult::ok_msg(format!("{}: no change", label));
    }
    let mut lines: Vec<String> = vec![format!("{} updated", label)];
    if outcome.preset_auto_promoted {
        if let Some(name) = outcome.requested_preset.as_deref() {
            lines.push(format!(
                "note: preset='{}' auto-promoted to 'custom' \
                 (a side or corner glyph was set; non-custom presets \
                 ignore the per-canvas glyph override)",
                name
            ));
        }
    }
    if bare_custom {
        lines.push(custom_preset_hint(label));
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
}

fn execute_show_border(eff: &mut ConsoleEffects) -> ExecResult {
    let map = &eff.document.mindmap;
    let cfg: Option<&GlyphBorderConfig> = map.canvas.default_border.as_ref();
    let lines = if let Some(cfg) = cfg {
        let resolved = resolve_border_style(Some(cfg), None, "#cccace");
        format_resolved(
            "canvas border",
            resolved.font_name.as_deref(),
            resolved.font_size_pt,
            &resolved.color,
            cfg,
        )
    } else {
        vec!["canvas border: (no map-wide default — falls back to the hardcoded floor)".into()]
    };
    ExecResult::lines(lines)
}

fn execute_show_section_frame(eff: &mut ConsoleEffects, focused: bool) -> ExecResult {
    let map = &eff.document.mindmap;
    let cfg = if focused {
        map.canvas.default_focused_section_frame_border.as_ref()
    } else {
        map.canvas.default_section_frame_border.as_ref()
    };
    let label = if focused {
        "canvas section-frame focused"
    } else {
        "canvas section-frame"
    };
    let lines = if let Some(cfg) = cfg {
        let resolved = resolve_border_style(Some(cfg), None, "#00E5FF");
        format_resolved(
            label,
            resolved.font_name.as_deref(),
            resolved.font_size_pt,
            &resolved.color,
            cfg,
        )
    } else {
        vec![format!(
            "{}: (no map-wide default — falls back to the hardcoded floor)",
            label
        )]
    };
    ExecResult::lines(lines)
}

fn format_resolved(
    label: &str,
    font: Option<&str>,
    size_pt: f32,
    color: &str,
    cfg: &GlyphBorderConfig,
) -> Vec<String> {
    vec![
        format!("{}:", label),
        format!("  preset:    {}", cfg.preset),
        format!("  font:      {}", font.unwrap_or("(default)")),
        format!("  size:      {} pt", size_pt),
        format!("  color:     {}", color),
        format!("  padding:   {}", cfg.padding),
        format!(
            "  palette:   {}",
            cfg.color_palette
                .as_deref()
                .map(|n| {
                    let field = cfg.color_palette_field.as_deref().unwrap_or("frame");
                    format!("{} (field={})", n, field)
                })
                .unwrap_or_else(|| "(none)".into())
        ),
    ]
}

// `edits_has_glyph_field` and `custom_preset_hint` are imported
// from `super::border` (re-exported in `border/mod.rs`) — the
// canvas / section-frame / per-node verbs all share the same
// helpers per CODE_CONVENTIONS.md §5.

#[cfg(test)]
mod tests {
    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::load_test_doc;

    #[test]
    fn canvas_border_preset_writes_canvas_default() {
        let mut doc = load_test_doc();
        assert!(doc.mindmap.canvas.default_border.is_none());
        assert_exec_ok(run("canvas border preset=heavy", &mut doc));
        let cfg = doc
            .mindmap
            .canvas
            .default_border
            .as_ref()
            .expect("default_border populated");
        assert_eq!(cfg.preset, "heavy");
    }

    #[test]
    fn canvas_section_frame_preset_writes_unfocused_default() {
        let mut doc = load_test_doc();
        assert!(doc.mindmap.canvas.default_section_frame_border.is_none());
        assert_exec_ok(run("canvas section-frame preset=double", &mut doc));
        let cfg = doc
            .mindmap
            .canvas
            .default_section_frame_border
            .as_ref()
            .expect("default_section_frame_border populated");
        assert_eq!(cfg.preset, "double");
        assert!(
            doc.mindmap.canvas.default_focused_section_frame_border.is_none(),
            "focused variant must not be touched"
        );
    }

    #[test]
    fn canvas_section_frame_focused_writes_focused_default_only() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame focused preset=heavy", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .expect("focused default populated")
                .preset,
            "heavy"
        );
        assert!(
            doc.mindmap.canvas.default_section_frame_border.is_none(),
            "unfocused variant must not be touched"
        );
    }

    #[test]
    fn canvas_border_top_pattern_auto_promotes_preset_to_custom() {
        let mut doc = load_test_doc();
        let result = run("canvas border preset=heavy top=\"###(*)###\"", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc.mindmap.canvas.default_border.as_ref().unwrap();
        assert_eq!(cfg.preset, "custom");
        // The glyph payload must have landed too — checking only
        // the preset would let a regression that drops the glyph
        // edit slip through.
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated by side edit");
        assert_eq!(glyphs.top, "###(*)###");
    }

    /// `canvas section-frame` (unfocused branch) must auto-promote
    /// preset to `"custom"` when a side or corner glyph is set.
    /// Pre-fix only the per-node and per-section paths were
    /// covered; the canvas section-frame paths are different
    /// setters writing different model slots.
    #[test]
    fn canvas_section_frame_top_pattern_auto_promotes_preset_to_custom() {
        let mut doc = load_test_doc();
        let result = run(
            "canvas section-frame preset=heavy top=\"###(*)###\"",
            &mut doc,
        );
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc
            .mindmap
            .canvas
            .default_section_frame_border
            .as_ref()
            .unwrap();
        assert_eq!(cfg.preset, "custom");
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated by side edit");
        assert_eq!(glyphs.top, "###(*)###");
        // The focused variant must NOT be touched.
        assert!(
            doc.mindmap
                .canvas
                .default_focused_section_frame_border
                .is_none(),
            "focused canvas default must be untouched"
        );
    }

    /// Same auto-promotion contract for the focused canvas
    /// section-frame branch.
    #[test]
    fn canvas_section_frame_focused_top_pattern_auto_promotes_preset_to_custom() {
        let mut doc = load_test_doc();
        let result = run(
            "canvas section-frame focused preset=heavy top=\"+=##=+\"",
            &mut doc,
        );
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc
            .mindmap
            .canvas
            .default_focused_section_frame_border
            .as_ref()
            .unwrap();
        assert_eq!(cfg.preset, "custom");
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated");
        assert_eq!(glyphs.top, "+=##=+");
        assert!(
            doc.mindmap.canvas.default_section_frame_border.is_none(),
            "unfocused canvas default must be untouched"
        );
    }

    /// `canvas border show` after setting palette + field must
    /// surface both in the readout — pre-fix only the preset/color
    /// pair was asserted; a regression that dropped palette from
    /// `format_resolved` would have shipped silently.
    #[test]
    fn canvas_border_show_reports_palette_and_field() {
        let mut doc = load_test_doc();
        // Use a palette that exists in the testament fixture.
        assert_exec_ok(run(
            "canvas border preset=light palette=rainbow field=frame",
            &mut doc,
        ));
        let cfg = doc.mindmap.canvas.default_border.as_ref().unwrap();
        assert_eq!(cfg.color_palette.as_deref(), Some("rainbow"));
        assert_eq!(cfg.color_palette_field.as_deref(), Some("frame"));
        let result = run("canvas border show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected lines, got {:?}", other),
        };
        assert!(
            blob.contains("rainbow"),
            "show must report palette name: {}",
            blob
        );
        assert!(
            blob.contains("field=frame"),
            "show must report palette field: {}",
            blob
        );
    }

    /// Subverbs accept mixed-case input. Pre-fix `Focused` and
    /// `Border` were exact-matched and a casing typo errored as
    /// "unknown subverb".
    #[test]
    fn canvas_subverb_dispatch_is_case_insensitive() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas Border preset=heavy", &mut doc));
        assert!(doc.mindmap.canvas.default_border.is_some());
        assert_exec_ok(run("canvas Section-Frame Focused preset=light", &mut doc));
        assert!(doc
            .mindmap
            .canvas
            .default_focused_section_frame_border
            .is_some());
    }

    /// `canvas border reset` against an already-empty default is
    /// a no-op and must not push undo entries or flip `dirty`.
    #[test]
    fn canvas_border_reset_when_already_empty_is_noop() {
        let mut doc = load_test_doc();
        let undo_depth = doc.undo_stack.len();
        doc.dirty = false;
        let result = run("canvas border reset", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert_eq!(
            doc.undo_stack.len(),
            undo_depth,
            "no-op canvas border reset must not push undo entries"
        );
        assert!(
            !doc.dirty,
            "no-op canvas border reset must not flip `dirty`"
        );
    }

    #[test]
    fn canvas_border_reset_clears_default() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset=heavy", &mut doc));
        assert!(doc.mindmap.canvas.default_border.is_some());
        assert_exec_ok(run("canvas border reset", &mut doc));
        assert!(
            doc.mindmap.canvas.default_border.is_none(),
            "canvas border reset must clear the map-wide default"
        );
    }

    #[test]
    fn canvas_round_trips_through_undo() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset=heavy color=#ff8800", &mut doc));
        assert!(doc.undo());
        assert!(
            doc.mindmap.canvas.default_border.is_none(),
            "undo restores the absent prior canvas default"
        );
    }

    #[test]
    fn canvas_unknown_subverb_errors() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(run("canvas frobnicate preset=heavy", &mut doc), "unknown subverb");
    }

    #[test]
    fn canvas_no_args_errors_with_usage() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(run("canvas", &mut doc), "usage:");
    }

    #[test]
    fn canvas_border_show_reports_default_or_floor() {
        let mut doc = load_test_doc();
        // With no canvas default set, show says so.
        let result = run("canvas border show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected lines, got {:?}", other),
        };
        assert!(
            blob.contains("hardcoded floor") || blob.contains("no map-wide default"),
            "show with no default should say so: {}",
            blob
        );

        // After setting a default, show prints its fields.
        assert_exec_ok(run("canvas border preset=double color=#ff00cc", &mut doc));
        let result = run("canvas border show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected lines, got {:?}", other),
        };
        assert!(blob.contains("double"), "show must report preset: {}", blob);
        assert!(blob.contains("#ff00cc"), "show must report color: {}", blob);
    }
}
