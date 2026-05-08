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

use super::border::{stage_kv, KEYS as BORDER_KEYS};
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
        "canvas", "default", "border", "section-frame", "frame", "preset", "glyph", "palette", "padding",
    ],
    applicable: always,
    complete: complete_canvas,
    execute: execute_canvas,
};

fn complete_canvas(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        // tokens[0] = "canvas"; offer the two top-level subverbs.
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        // tokens[1] depends on tokens[0]:
        //   - after "border": offer show/reset + kv keys
        //   - after "section-frame": offer "focused", show/reset, kv keys
        CompletionContext::Token { index: 1 } => match state.tokens.first().map(String::as_str) {
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
        // tokens[>=2] is always inside the kv-form path.
        CompletionContext::Token { .. } => kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint),
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

pub fn execute_canvas(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let subject = match args.positional(0) {
        Some(v) => v,
        None => {
            return ExecResult::err(
                "usage: canvas border|section-frame [focused] show|reset|<key>=<value> …",
            );
        }
    };
    match subject {
        "border" => execute_border_subject(args, eff),
        "section-frame" => execute_section_frame_subject(args, eff),
        other => ExecResult::err(format!(
            "canvas: unknown subverb '{}'; use 'border' or 'section-frame'",
            other
        )),
    }
}

fn execute_border_subject(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // tokens[1] is either show/reset or the first kv. Skip the
    // `subject` positional; everything else mirrors the
    // per-node `border` verb's kv-form path.
    if let Some(verb) = args.positional(1) {
        match verb {
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
        return ExecResult::err(
            "usage: canvas border show|reset|<key>=<value> …",
        );
    }
    apply_border_edits(eff, edits)
}

fn execute_section_frame_subject(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // tokens[1] may be "focused" (modifier) or the first subverb /
    // kv. Determine which by matching exactly.
    let focused = matches!(args.positional(1), Some("focused"));
    let verb_pos = if focused { 2 } else { 1 };

    if let Some(verb) = args.positional(verb_pos) {
        match verb {
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
        return ExecResult::err(
            "usage: canvas section-frame [focused] show|reset|<key>=<value> …",
        );
    }
    apply_section_frame_edits(eff, focused, edits)
}

fn clear_edits() -> BorderConfigEdits {
    let mut e = BorderConfigEdits::default();
    e.clear = true;
    e
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
        format_resolved("canvas border", resolved.font_name.as_deref(), resolved.font_size_pt, &resolved.color, cfg)
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
        format_resolved(label, resolved.font_name.as_deref(), resolved.font_size_pt, &resolved.color, cfg)
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

fn custom_preset_hint(label: &str) -> String {
    format!(
        "hint: 'custom' is the preset that lets you author per-side / per-corner glyphs. \
         Combine it with any of: top=… bottom=… left=… right=… tl=… tr=… bl=… br=…  \
         e.g. `{} preset=custom top=\"###(*)###\" tl=\"+\" tr=\"+\" bl=\"+\" br=\"+\"`. \
         See `format/border-patterns.md` for the side-pattern grammar.",
        label
    )
}

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
            doc.mindmap
                .canvas
                .default_focused_section_frame_border
                .is_none(),
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
            doc.mindmap
                .canvas
                .default_section_frame_border
                .is_none(),
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
            ExecResult::Lines(ls) => ls
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
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
            ExecResult::Lines(ls) => ls
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            other => panic!("expected lines, got {:?}", other),
        };
        assert!(blob.contains("double"), "show must report preset: {}", blob);
        assert!(blob.contains("#ff00cc"), "show must report color: {}", blob);
    }
}
