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
//! behavior.
//!
//! Undo: each successful canvas edit pushes a single
//! `UndoAction::CanvasSnapshot` so undo restores every canvas
//! field in one step (theme variables, palettes, defaults — all
//! captured together by design).

use baumhard::mindmap::border::resolve_border_style;
use baumhard::mindmap::model::GlyphBorderConfig;

use super::border::{
    custom_preset_hint, edits_has_glyph_field, positional_subverb_to_edits, prepend_line, stage_kv,
    BorderSurface, KEYS as BORDER_KEYS,
};
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
/// Subverbs both canvas subjects match *ahead* of the
/// positional-vs-kv discriminator, so they stay on offer at a
/// kv-form slot: `canvas border color=#fff show` prints the
/// readout and `canvas border color=#fff preview commit`
/// terminates a preview. The per-field positional subverbs the
/// discriminator does gate are
/// [`super::border::POSITIONAL_SUBVERBS`] — the same seven the
/// per-node `border` verb surfaces, offered here only when the
/// slot is positional.
pub const UNGATED_SUBVERBS: &[&str] = &["show", "reset", "preview"];
/// Modifier under `section-frame` (followed by show|reset|kv).
pub const SECTION_FRAME_MODIFIERS: &[&str] = &["focused"];

pub const COMMAND: Command = Command {
    name: "canvas",
    aliases: &[],
    summary: "Edit map-wide canvas defaults (border, section frame)",
    usage:
        "canvas border show|reset \
         | canvas border preset <name> | canvas border color <value> | canvas border padding <px> \
         | canvas border palette <name> [field=<...>] | canvas border font <family> [size=<pt>] \
         | canvas border side <which> <pattern|reset> | canvas border corner <which> <glyph|reset> \
         | canvas border <key>=<value> … \
         | canvas section-frame [focused] show|reset|<key>=<value> … \
         | canvas border preview <kv>=… | canvas border preview commit|cancel \
         | canvas section-frame [focused] preview <kv>=… | canvas section-frame [focused] preview commit|cancel",
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
    //
    // Every name is lowercased before it is matched, because
    // `execute_canvas` and both of its subject arms dispatch on
    // `to_ascii_lowercase` — `canvas Border Preset <TAB>` reaches
    // the arm that `canvas Border Preset heavy` will run.
    let lower = |s: Option<&str>| s.map(str::to_ascii_lowercase);
    let subject = lower(state.positional(0));
    let subject = subject.as_deref();
    // The `focused` modifier shifts the whole subverb tree one
    // positional right. Every slot below is expressed relative to
    // `verb_at`, the positional index the subverb occupies, so the
    // shift is handled once instead of being re-derived per arm.
    // Re-deriving it is what left `canvas border side top <TAB>`
    // and its two `section-frame` twins on the kv-key catch-all
    // while the per-node `border side top <TAB>` answered: each
    // arm was written for one depth and the next depth had none.
    let focused =
        subject == Some("section-frame") && lower(state.positional(1)).as_deref() == Some("focused");
    let verb_at = if focused { 2 } else { 1 };
    // The same discriminator `execute_border_subject` applies: a
    // kv at or ahead of the subverb slot means the line is kv
    // form, so the positional vocabulary is not what runs. It
    // gates the lookahead below *and* both slots that emit the
    // vocabulary — gating only the lookahead is how `canvas
    // color=#fff border <TAB>` came to offer ten subverbs of
    // which that line rejects seven.
    let positional_form = super::border::subverb_slot_is_positional(state.arg_tokens(), verb_at);
    let verb = lower(positional_form.then(|| state.positional(verb_at)).flatten());
    let verb = verb.as_deref();
    match &state.context {
        // First positional after `canvas`: offer the subjects.
        CompletionContext::Token { index: 0 } => prefix_filter(VERBS, state.partial),
        // Second positional, branched on subject:
        //   - after `border`: show/reset/preview + kv keys
        //   - after `section-frame`: `focused`, show/reset/preview, kv keys
        //
        // Sits ahead of the `verb_at` arms because when the user is
        // still *typing* `focused` the cursor is at this slot while
        // `focused` already reads as present one token ahead.
        CompletionContext::Token { index: 1 } => match subject {
            Some("border") => {
                subverb_completions(state.partial, /* modifiers */ false, positional_form)
            }
            Some("section-frame") => subverb_completions(state.partial, true, positional_form),
            _ => Vec::new(),
        },
        // The subverb slot itself, reachable here only past the
        // `focused` modifier (without it the arm above answers).
        CompletionContext::Token { index } if *index == verb_at => {
            subverb_completions(state.partial, false, positional_form)
        }
        // The subverb's first argument: `preview`'s commit/cancel
        // pair, or the per-field vocabulary of `preset` / `color` /
        // `palette` / `font` / `side` / `corner`.
        CompletionContext::Token { index } if *index == verb_at + 1 => match verb {
            Some("preview") => {
                let mut out = super::border::preview_subverb_completions(state.partial);
                out.extend(kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint));
                out
            }
            other => canvas_value_completions(other, state.partial, ctx),
        },
        // The subverb's second argument — the pattern after `side
        // <which>`, the glyph after `corner <which>`. Same slot,
        // same vocabulary as the per-node verb's.
        CompletionContext::Token { index } if *index == verb_at + 2 => {
            super::border::second_positional_completions(verb, state.partial, state.positional(verb_at + 1))
        }
        // Anything past a subverb's arguments is kv-form.
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

/// The subverb slot's rows: `show` / `reset` / `preview` and —
/// when `positional_form` says the slot is one the verb will read
/// positionally — the per-field positional subverbs; then the kv
/// keys that are the other way to say the same thing, and, at the
/// one slot where it is still ahead of the user, the `focused`
/// modifier.
///
/// `focused` is ungated with the readout subverbs and for the same
/// reason: `execute_section_frame_subject` reads it off
/// `positional(1)` before the discriminator runs, so `canvas
/// section-frame color=#fff focused show` still reaches the
/// focused slot's readout.
fn subverb_completions(partial: &str, with_modifiers: bool, positional_form: bool) -> Vec<Completion> {
    let mut out = if with_modifiers {
        prefix_filter(SECTION_FRAME_MODIFIERS, partial)
    } else {
        Vec::new()
    };
    out.extend(prefix_filter(UNGATED_SUBVERBS, partial));
    if positional_form {
        out.extend(prefix_filter(super::border::POSITIONAL_SUBVERBS, partial));
    }
    out.extend(kv_key_completions_with_hints(BORDER_KEYS, partial, kv_hint));
    out
}

/// Per-field positional-value completion for `canvas border <verb>
/// <TAB>` and `canvas section-frame [focused] <verb> <TAB>`.
/// Routes through the same per-node `border` vocabularies so canvas
/// users see the same preset / palette / font / side / corner rows
/// the per-node verb surfaces.
///
/// `show` deliberately offers nothing: `execute_show_border` and
/// `execute_show_section_frame` take no arguments, so the
/// `side=` / `verbose` rows the per-node `border show <TAB>`
/// offers would name flags this surface silently drops.
fn canvas_value_completions(verb: Option<&str>, partial: &str, ctx: &ConsoleContext) -> Vec<Completion> {
    match verb {
        Some("preset") | Some("color") | Some("palette") | Some("font") => {
            super::border::kv_value_completions(verb.unwrap_or_default(), partial, ctx)
        }
        Some("side") => prefix_filter(super::border::SIDE_VALUES, partial),
        Some("corner") => prefix_filter(super::border::CORNER_VALUES, partial),
        // `padding` takes a number — no candidate vocabulary.
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
    //positional subverbs mirror the per-node `border`
    // verb's grammar so users can `canvas border preset heavy` /
    // `canvas border color #fff` / etc. The kv form
    // `canvas border preset=heavy` still works (alias for
    // keybinds, per).
    if let Some(verb) = args.positional(1) {
        // Same discriminator the per-node `border …` verb applies:
        // a kv ahead of the subverb slot means the line is kv form
        // and the later positional is an unquoted multi-word value,
        // not a subverb. Without it `canvas border color=#fff
        // preset heavy` dispatched positionally and dropped
        // `color=#fff` on the floor without a word.
        let positional_form = super::border::subverb_slot_is_positional(args.tokens(), 1);
        match verb.to_ascii_lowercase().as_str() {
            "show" => return execute_show_border(eff),
            "reset" => return apply_canvas_edits(eff, BorderSurface::CanvasDefault, clear_edits()),
            "preview" => return execute_canvas_border_preview(args, eff),
            other if !other.contains('=') && positional_form => {
                return apply_positional(
                    other,
                    args,
                    /* verb_pos */ 1,
                    BorderSurface::CanvasDefault,
                    eff,
                );
            }
            other if !other.contains('=') => {
                return ExecResult::err(super::border::unquoted_multiword_hint(
                    BorderSurface::CanvasDefault.label(),
                    verb,
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
    apply_canvas_edits(eff, BorderSurface::CanvasDefault, edits)
}

/// Positional-subverb entry point for both canvas subjects. The
/// grammar itself is parsed by the surface-agnostic
/// [`super::border::positional_subverb_to_edits`] — the same
/// parser the per-node `border …` verb uses, so a canvas slot and
/// a node can no longer drift on preset cycling, `reset` glyph
/// resolution, or the non-custom-preset gate.
fn apply_positional(
    verb: &str,
    args: &Args,
    verb_pos: usize,
    surface: BorderSurface,
    eff: &mut ConsoleEffects,
) -> ExecResult {
    let staged = match positional_subverb_to_edits(verb, args, verb_pos, surface, eff.document) {
        Ok(Some(staged)) => staged,
        Ok(None) => return ExecResult::err(unknown_canvas_subverb_message(surface, verb)),
        Err(msg) => return ExecResult::err(msg),
    };
    let outcome = apply_canvas_edits(eff, surface, staged.edits);
    match staged.cycled_to {
        Some(target) => prepend_line(
            outcome,
            format!("{} preset \u{2192} '{}' (cycle)", surface.label(), target),
        ),
        None => outcome,
    }
}

fn unknown_canvas_subverb_message(surface: BorderSurface, verb: &str) -> String {
    format!(
        "{}: unknown subverb '{}'; use 'show', 'reset', 'preview', \
         'preset', 'color', 'padding', 'palette', 'font', 'side', 'corner', or kv form",
        surface.label(),
        verb
    )
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
    let surface = if focused {
        BorderSurface::CanvasSectionFrameFocused
    } else {
        BorderSurface::CanvasSectionFrame
    };

    if let Some(verb) = args.positional(verb_pos) {
        // See `execute_border_subject` — same discriminator, one
        // slot further right when the `focused` modifier is in play.
        let positional_form = super::border::subverb_slot_is_positional(args.tokens(), verb_pos);
        match verb.to_ascii_lowercase().as_str() {
            "show" => return execute_show_section_frame(eff, focused),
            "reset" => return apply_canvas_edits(eff, surface, clear_edits()),
            "preview" => return execute_canvas_section_frame_preview(args, eff, focused),
            other if !other.contains('=') && positional_form => {
                return apply_positional(other, args, verb_pos, surface, eff);
            }
            other if !other.contains('=') => {
                return ExecResult::err(super::border::unquoted_multiword_hint(surface.label(), verb));
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
    apply_canvas_edits(eff, surface, edits)
}

fn clear_edits() -> BorderConfigEdits {
    BorderConfigEdits {
        clear: true,
        ..BorderConfigEdits::default()
    }
}

/// `canvas border preview …` — stage / commit / cancel a
/// preview that targets `Canvas.default_border`. The preview
/// applies map-wide to every framed node without a per-node
/// override; commit writes through the same setter the
/// committing `canvas border …` path uses
/// (`set_canvas_default_border`).
fn execute_canvas_border_preview(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    use crate::application::document::BorderPreviewTarget;
    super::border::dispatch_border_preview(
        args,
        eff,
        "canvas border preview",
        /* subverb_pos */ 2,
        |_sel| Ok(BorderPreviewTarget::CanvasDefault),
    )
}

/// `canvas section-frame [focused] preview …` — stage / commit /
/// cancel a preview that targets one of the two canvas
/// section-frame slots
/// (`default_section_frame_border` or `default_focused_section_frame_border`
/// per the `focused` arg).
fn execute_canvas_section_frame_preview(args: &Args, eff: &mut ConsoleEffects, focused: bool) -> ExecResult {
    use crate::application::document::BorderPreviewTarget;
    let label: &'static str = if focused {
        "canvas section-frame focused preview"
    } else {
        "canvas section-frame preview"
    };
    let target = if focused {
        BorderPreviewTarget::CanvasSectionFrameFocused
    } else {
        BorderPreviewTarget::CanvasSectionFrame
    };
    // Subverb position depends on the `focused` modifier:
    // `canvas section-frame preview …`        → positional(2)
    // `canvas section-frame focused preview …`→ positional(3)
    let subverb_pos = if focused { 3 } else { 2 };
    super::border::dispatch_border_preview(args, eff, label, subverb_pos, move |_sel| Ok(target.clone()))
}

/// Write a staged edit bundle to the canvas slot `surface` names
/// and render the outcome. One function for all three slots — the
/// only difference is which document setter runs.
fn apply_canvas_edits(
    eff: &mut ConsoleEffects,
    surface: BorderSurface,
    edits: BorderConfigEdits,
) -> ExecResult {
    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let outcome: BorderEditOutcome = match surface {
        BorderSurface::CanvasDefault => eff.document.set_canvas_default_border(edits),
        BorderSurface::CanvasSectionFrame => eff
            .document
            .set_canvas_default_section_frame_border_config(false, edits),
        BorderSurface::CanvasSectionFrameFocused => eff
            .document
            .set_canvas_default_section_frame_border_config(true, edits),
        // The per-node surface is the `border …` verb's business;
        // routing it here would silently write the map-wide slot.
        // Defensive per CODE_CONVENTIONS §9 — interactive paths
        // degrade rather than panic.
        BorderSurface::Selection => {
            log::error!("canvas: per-node BorderSurface reached the canvas writer");
            return ExecResult::err("internal: canvas edit routed to the per-node surface");
        }
    };
    finish(outcome, surface.label(), bare_custom)
}

fn finish(outcome: BorderEditOutcome, label: &str, bare_custom: bool) -> ExecResult {
    // A refused glyph is an error, not a "no change": the setter
    // declined it because the loader would reject the saved file,
    // and reporting success here is exactly how the user ends up
    // with a map they cannot reopen. Same posture as the per-node
    // `border` verb's `apply_edits`.
    if !outcome.rejected.is_empty() {
        return ExecResult::Err(format!("{}: {}", label, outcome.rejected.join("; ")));
    }
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
        format_resolved_with_source(
            "canvas border",
            "canvas default",
            resolved.font_name.as_deref(),
            resolved.font_size_pt,
            &resolved.color,
            cfg,
        )
    } else {
        vec!["canvas border: (no map-wide default — every framed node falls back to its per-node `style.frame_color` / hardcoded glyph defaults)".into()]
    };
    ExecResult::lines(lines)
}

fn execute_show_section_frame(eff: &mut ConsoleEffects, focused: bool) -> ExecResult {
    let map = &eff.document.mindmap;
    let label = if focused {
        "canvas section-frame focused"
    } else {
        "canvas section-frame"
    };
    // Cascade matches `resolve_section_frame_border`: focused
    // frames fall through to the unfocused canvas slot before
    // hitting the hardcoded floor.
    let (cfg, source) = if focused {
        match (
            map.canvas.default_focused_section_frame_border.as_ref(),
            map.canvas.default_section_frame_border.as_ref(),
        ) {
            (Some(c), _) => (Some(c), "focused canvas default"),
            (None, Some(c)) => (Some(c), "unfocused canvas default (focused fallback)"),
            (None, None) => (None, "hardcoded heavy floor"),
        }
    } else {
        match map.canvas.default_section_frame_border.as_ref() {
            Some(c) => (Some(c), "unfocused canvas default"),
            None => (None, "hardcoded light floor"),
        }
    };
    let lines = if let Some(cfg) = cfg {
        let resolved = resolve_border_style(Some(cfg), None, "#00E5FF");
        format_resolved_with_source(
            label,
            source,
            resolved.font_name.as_deref(),
            resolved.font_size_pt,
            &resolved.color,
            cfg,
        )
    } else {
        vec![format!(
            "{}: (no map-wide default — falls back to {})",
            label, source
        )]
    };
    ExecResult::lines(lines)
}

/// Tail of `execute_show_*` — produce the multi-line readout
/// describing the resolved style. Now includes per-side patterns
/// and per-corner glyphs (the prior shape omitted both, which
/// hid the very fields users author when they pass
/// `top="###(*)###"` etc. — flagged as M4 / M1 in two prior
/// reviews). `source` labels the cascade level the resolved
/// style came from (e.g. "focused canvas default", "unfocused
/// canvas default (focused fallback)", "hardcoded light floor").
fn format_resolved_with_source(
    label: &str,
    source: &str,
    font: Option<&str>,
    size_pt: f32,
    color: &str,
    cfg: &GlyphBorderConfig,
) -> Vec<String> {
    let mut lines = vec![
        format!("{}:", label),
        format!("  source:    {}", source),
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
    ];
    // Per-side / per-corner readout. Only meaningful when the
    // preset is `custom` (other presets ignore `glyphs`); for
    // the named presets we surface "(preset default)" so the
    // reader knows the preset's defaults are in play.
    if let Some(g) = cfg.glyphs.as_ref() {
        lines.push(format!("  top:       {}", g.top));
        lines.push(format!("  bottom:    {}", g.bottom));
        lines.push(format!("  left:      {}", g.left));
        lines.push(format!("  right:     {}", g.right));
        lines.push(format!(
            "  corners:   tl={}  tr={}  bl={}  br={}",
            g.top_left, g.top_right, g.bottom_left, g.bottom_right
        ));
    } else {
        lines.push(format!("  glyphs:    (preset '{}' defaults)", cfg.preset));
    }
    lines
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

    /// The `focused` modifier shifts `canvas section-frame`'s whole
    /// subverb tree one positional to the right. Completion did not
    /// follow it, so past `focused` the popup offered kv keys only —
    /// hiding `show`, `reset`, `preview` and every per-field subverb
    /// that `execute_canvas` accepts in exactly that position.
    #[test]
    fn test_canvas_focused_section_frame_completion_offers_subverbs() {
        let doc = load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let line = "canvas section-frame focused ";
        let rows: Vec<String> = crate::application::console::completion::complete(line, line.len(), &ctx)
            .into_iter()
            .map(|c| c.text)
            .collect();
        let offered = super::UNGATED_SUBVERBS
            .iter()
            .chain(super::super::border::POSITIONAL_SUBVERBS);
        for expected in offered {
            assert!(
                rows.iter().any(|r| r == expected),
                "`{line}<TAB>` should offer '{expected}'; got {rows:?}"
            );
        }
        // The kv keys stay — `canvas section-frame focused
        // preset=heavy` is a real form.
        assert!(rows.iter().any(|r| r == "preset="));
    }

    /// The popup rows for `line` with the cursor at its end.
    fn popup(line: &str, doc: &crate::application::document::MindMapDocument) -> Vec<String> {
        let ctx = crate::application::console::ConsoleContext::from_document(doc);
        crate::application::console::completion::complete(line, line.len(), &ctx)
            .into_iter()
            .map(|c| c.text)
            .collect()
    }

    /// One positional deeper than the `focused` gap above: the
    /// value slot *after* a `side` / `corner` which-arg. Every arm
    /// stopped at the first value slot, so `canvas border side top
    /// <TAB>` fell to the kv-key catch-all and answered `preset=`
    /// … `br=` while `border side top <TAB>` answered `reset` —
    /// even though `canvas border side top reset` has been pinned
    /// as working since the oracle landed.
    #[test]
    fn test_canvas_side_and_corner_second_value_slot_offers_reset() {
        let doc = load_test_doc();
        for line in [
            "canvas border side top ",
            "canvas border corner tl ",
            "canvas section-frame side top ",
            "canvas section-frame corner tl ",
            "canvas section-frame focused side top ",
            "canvas section-frame focused corner tl ",
        ] {
            assert_eq!(
                popup(line, &doc),
                vec!["reset"],
                "`{line}<TAB>` must offer the same row `border side top <TAB>` does"
            );
        }
        // The which-arg slot itself keeps its five words, at both
        // depths.
        assert_eq!(
            popup("canvas border side ", &doc),
            super::super::border::SIDE_VALUES.to_vec()
        );
        assert_eq!(
            popup("canvas section-frame focused corner ", &doc),
            super::super::border::CORNER_VALUES.to_vec()
        );
    }

    /// A kv ahead of the subverb slot puts the line in kv form —
    /// the discriminator the per-node `border …` verb has always
    /// applied, so an unquoted `palette=My Palette` is not read as
    /// a `Palette` subverb. The canvas surfaces had no such gate:
    /// they dispatched positionally and dropped the kv in silence.
    #[test]
    fn test_canvas_kv_before_subverb_is_kv_form_not_a_positional() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(
            run("canvas border color=#ffffff preset heavy", &mut doc),
            "unexpected positional",
        );
        assert_exec_err_contains(
            run(
                "canvas section-frame focused color=#ffffff preset heavy",
                &mut doc,
            ),
            "unexpected positional",
        );
        // Nothing was written — the rejection is total, not partial.
        assert!(doc.mindmap.canvas.default_border.is_none());
        assert!(doc.mindmap.canvas.default_focused_section_frame_border.is_none());
        // The popup agrees with the verb: no positional vocabulary
        // at a slot the verb reads as kv form.
        assert!(popup("canvas border color=#ffffff side ", &doc).is_empty());
    }

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

    /// `canvas section-frame` (unfocused branch) must auto-
    /// promote preset to `"custom"` when a side or corner
    /// glyph is set — different setter from per-node /
    /// per-section so it needs its own pin.
    #[test]
    fn canvas_section_frame_top_pattern_auto_promotes_preset_to_custom() {
        let mut doc = load_test_doc();
        let result = run("canvas section-frame preset=heavy top=\"###(*)###\"", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc.mindmap.canvas.default_section_frame_border.as_ref().unwrap();
        assert_eq!(cfg.preset, "custom");
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated by side edit");
        assert_eq!(glyphs.top, "###(*)###");
        // The focused variant must NOT be touched.
        assert!(
            doc.mindmap.canvas.default_focused_section_frame_border.is_none(),
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
    /// surface both in the readout.
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

    /// Subverbs accept mixed-case input.
    #[test]
    fn canvas_subverb_dispatch_is_case_insensitive() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas Border preset=heavy", &mut doc));
        assert!(doc.mindmap.canvas.default_border.is_some());
        assert_exec_ok(run("canvas Section-Frame Focused preset=light", &mut doc));
        assert!(doc.mindmap.canvas.default_focused_section_frame_border.is_some());
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
        assert!(!doc.dirty, "no-op canvas border reset must not flip `dirty`");
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

    /// `canvas border preview preset=heavy` stages a preview
    /// against `Canvas.default_border` without writing the model.
    #[test]
    fn canvas_border_preview_targets_canvas_default() {
        let mut doc = load_test_doc();
        assert!(doc.mindmap.canvas.default_border.is_none());
        let result = run("canvas border preview preset=heavy", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert!(doc.border_preview.is_some(), "preview slot populated");
        match &doc.border_preview.as_ref().unwrap().target {
            crate::application::document::BorderPreviewTarget::CanvasDefault => {}
            other => panic!("expected CanvasDefault target, got {:?}", other),
        }
        assert!(
            doc.mindmap.canvas.default_border.is_none(),
            "preview must not write to the model"
        );
    }

    /// `canvas section-frame focused preview preset=double`
    /// targets the focused canvas slot only — commits write to
    /// `default_focused_section_frame_border` and leave the
    /// unfocused variant untouched.
    #[test]
    fn canvas_section_frame_focused_preview_does_not_touch_unfocused_default() {
        let mut doc = load_test_doc();
        assert_exec_ok(run(
            "canvas section-frame focused preview preset=double",
            &mut doc,
        ));
        let preview = doc.border_preview.as_ref().expect("preview slot populated");
        match &preview.target {
            crate::application::document::BorderPreviewTarget::CanvasSectionFrameFocused => {}
            other => panic!("expected CanvasSectionFrameFocused target, got {:?}", other),
        }
        // Commit and verify the focused canvas slot is the only
        // one written.
        let result = run("canvas section-frame focused preview commit", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert_eq!(
            doc.mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .unwrap()
                .preset,
            "double"
        );
        assert!(
            doc.mindmap.canvas.default_section_frame_border.is_none(),
            "unfocused canvas slot must remain untouched"
        );
    }

    /// `canvas border preview cancel` discards without writing.
    #[test]
    fn canvas_border_preview_cancel_clears_without_writing() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preview preset=heavy", &mut doc));
        assert!(doc.border_preview.is_some());
        assert_exec_ok(run("canvas border preview cancel", &mut doc));
        assert!(doc.border_preview.is_none());
        assert!(doc.mindmap.canvas.default_border.is_none());
    }
}

#[cfg(test)]
mod positional_tests {
    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::document::tests_common::load_test_doc;

    ///B6.10: `canvas border preset NAME` writes through
    /// to canvas.default_border.
    #[test]
    fn canvas_border_preset_positional_writes_through() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset heavy", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.as_str()),
            Some("heavy")
        );
    }

    #[test]
    fn canvas_border_color_positional_writes_through() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border color #112233", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_border
                .as_ref()
                .and_then(|c| c.color.as_deref()),
            Some("#112233")
        );
    }

    #[test]
    fn canvas_border_padding_positional_writes_through() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border padding 9", &mut doc));
        assert_eq!(
            doc.mindmap.canvas.default_border.as_ref().map(|c| c.padding),
            Some(9.0)
        );
    }

    #[test]
    fn canvas_border_unknown_subverb_rejects_with_full_hint() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(
            run("canvas border frobnicate", &mut doc),
            "use 'show', 'reset', 'preview', \
             'preset', 'color', 'padding', 'palette', 'font', 'side', 'corner'",
        );
    }

    #[test]
    fn canvas_section_frame_preset_positional_writes_through() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame preset double", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.as_str()),
            Some("double")
        );
    }

    #[test]
    fn canvas_section_frame_focused_color_positional_writes_through() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame focused color #abcdef", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .and_then(|c| c.color.as_deref()),
            Some("#abcdef")
        );
    }
}

#[cfg(test)]
mod blocker_pins {
    //! Regression pins for the canvas-side blockers from the
    //! Batch-6 opus review:
    //!
    //! 1. `canvas border|section-frame side|corner reset` used
    //!    to hardcode "light" preset glyphs. Now resolves the
    //!    target slot's actual preset.
    //! 2. `canvas border|section-frame side|corner WHICH PATTERN`
    //!    silently auto-promoted the slot's preset to "custom".
    //!    Now errors with the same `run \`<verb> preset custom\`
    //!    first` hint the per-node `border` verb uses.

    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::document::tests_common::load_test_doc;

    #[test]
    fn canvas_border_side_reset_uses_actual_preset_not_hardcoded_light() {
        let mut doc = load_test_doc();
        // Set canvas default to heavy first; then put it into
        // custom (so we can write per-side glyphs); then write
        // a top override; then reset top.
        assert_exec_ok(run("canvas border preset heavy", &mut doc));
        assert_exec_ok(run("canvas border preset custom", &mut doc));
        assert_exec_ok(run("canvas border side top \"###\"", &mut doc));
        // Now flip back to heavy and reset the top side. Reset
        // should write heavy's `━`, not light's `─`.
        assert_exec_ok(run("canvas border preset heavy", &mut doc));
        assert_exec_ok(run("canvas border side top reset", &mut doc));
        let top = doc
            .mindmap
            .canvas
            .default_border
            .as_ref()
            .and_then(|c| c.glyphs.as_ref())
            .map(|g| g.top.clone())
            .expect("custom + side write must populate glyphs");
        assert_eq!(top, "━", "reset must use heavy's default top glyph, not light's");
    }

    #[test]
    fn canvas_border_side_on_non_custom_preset_errors() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset heavy", &mut doc));
        assert_exec_err_contains(
            run("canvas border side top \"=##=\"", &mut doc),
            "run `canvas border preset custom` first",
        );
    }

    #[test]
    fn canvas_border_corner_on_non_custom_preset_errors() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset rounded", &mut doc));
        assert_exec_err_contains(
            run("canvas border corner tl +", &mut doc),
            "run `canvas border preset custom` first",
        );
    }

    #[test]
    fn canvas_section_frame_focused_side_on_non_custom_preset_errors() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame focused preset double", &mut doc));
        assert_exec_err_contains(
            run("canvas section-frame focused side top \"###\"", &mut doc),
            "run `canvas section-frame focused preset custom` first",
        );
    }

    #[test]
    fn canvas_section_frame_corner_reset_uses_actual_preset() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame preset double", &mut doc));
        assert_exec_ok(run("canvas section-frame preset custom", &mut doc));
        assert_exec_ok(run("canvas section-frame corner tl +", &mut doc));
        assert_exec_ok(run("canvas section-frame preset double", &mut doc));
        assert_exec_ok(run("canvas section-frame corner tl reset", &mut doc));
        let tl = doc
            .mindmap
            .canvas
            .default_section_frame_border
            .as_ref()
            .and_then(|c| c.glyphs.as_ref())
            .map(|g| g.top_left.clone())
            .expect("custom + corner write must populate glyphs");
        assert_eq!(tl, "╔", "reset must use double's tl glyph, not light's ┌");
    }
}

#[cfg(test)]
mod cycle_pin {
    use crate::application::console::tests::fixtures::{assert_exec_ok, run};
    use crate::application::document::tests_common::load_test_doc;

    /// `canvas border preset cycle` advances the canvas-default
    /// preset by one, wrapping. Pin parity with the per-node
    /// `border preset cycle` shipped in B6.4.
    #[test]
    fn canvas_border_preset_cycle_advances_canvas_default() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset light", &mut doc));
        assert_exec_ok(run("canvas border preset cycle", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.as_str()),
            Some("heavy")
        );
    }

    /// Cycle wraps from the last entry (`custom`) back to the
    /// first (`light`).
    #[test]
    fn canvas_section_frame_preset_cycle_wraps() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame preset custom", &mut doc));
        assert_exec_ok(run("canvas section-frame preset cycle", &mut doc));
        assert_eq!(
            doc.mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.as_str()),
            Some("light")
        );
    }
}

#[cfg(test)]
mod completion_pins {
    use super::*;
    use crate::application::console::completion::CompletionContext;

    fn fixture_doc() -> crate::application::document::MindMapDocument {
        crate::application::document::tests_common::load_test_doc()
    }

    fn at_token<'a>(index: usize, partial: &'a str, tokens: &'a [String]) -> CompletionState<'a> {
        CompletionState {
            tokens,
            partial,
            context: CompletionContext::Token { index },
        }
    }

    /// `canvas border <TAB>` surfaces every positional subverb,
    /// not just show/reset/preview.
    #[test]
    fn canvas_border_token1_lists_all_positional_subverbs() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["canvas".to_string(), "border".to_string()];
        let s = at_token(1, "", &tokens);
        let labels: Vec<String> = complete_canvas(&s, &ctx).into_iter().map(|c| c.display).collect();
        for v in &[
            "show", "reset", "preview", "preset", "color", "padding", "palette", "font", "side", "corner",
        ] {
            assert!(
                labels.iter().any(|l| l == v),
                "canvas border completion missing '{}': {:?}",
                v,
                labels
            );
        }
    }

    #[test]
    fn canvas_border_preset_value_completion() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["canvas".to_string(), "border".to_string(), "preset".to_string()];
        let s = at_token(2, "", &tokens);
        let labels: Vec<String> = complete_canvas(&s, &ctx).into_iter().map(|c| c.display).collect();
        for v in &["light", "heavy", "double", "rounded", "custom", "cycle"] {
            assert!(
                labels.iter().any(|l| l == v),
                "canvas border preset value completion missing '{}': {:?}",
                v,
                labels
            );
        }
    }

    #[test]
    fn canvas_border_side_value_completion() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["canvas".to_string(), "border".to_string(), "side".to_string()];
        let s = at_token(2, "", &tokens);
        let labels: Vec<String> = complete_canvas(&s, &ctx).into_iter().map(|c| c.display).collect();
        for v in &["top", "bottom", "left", "right", "all"] {
            assert!(
                labels.iter().any(|l| l == v),
                "canvas border side value completion missing '{}': {:?}",
                v,
                labels
            );
        }
    }

    #[test]
    fn canvas_section_frame_focused_preset_value_completion() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec![
            "canvas".to_string(),
            "section-frame".to_string(),
            "focused".to_string(),
            "preset".to_string(),
        ];
        let s = at_token(3, "", &tokens);
        let labels: Vec<String> = complete_canvas(&s, &ctx).into_iter().map(|c| c.display).collect();
        assert!(labels.iter().any(|l| l == "heavy"));
        assert!(labels.iter().any(|l| l == "cycle"));
    }
}

#[cfg(test)]
mod shared_positional_pins {
    //! P1-28: `canvas …` no longer carries its own copy of the
    //! positional border grammar — it shares
    //! `border::positional_subverb_to_edits` with the per-node
    //! `border …` verb. These pin the behaviors the canvas copy
    //! used to lack, so a regression shows up as a failing test
    //! rather than as a silently re-diverged second parser.

    use crate::application::console::tests::fixtures::{
        assert_exec_err_contains, assert_exec_ok, join_lines, run,
    };
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::load_test_doc;

    /// `canvas border preset cycle` now names the preset it landed
    /// on, exactly as `border preset cycle` does.
    #[test]
    fn canvas_cycle_reports_the_resolved_preset_like_the_node_verb() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas border preset light", &mut doc));
        let blob = match run("canvas border preset cycle", &mut doc) {
            ExecResult::Lines(rows) => join_lines(&rows),
            other => panic!("expected Lines with the cycle header, got {:?}", other),
        };
        assert!(
            blob.contains("→ 'heavy'") && blob.contains("(cycle)"),
            "canvas cycle must name the resolved preset: {}",
            blob
        );
    }

    /// Extra positionals after a single-value subverb are rejected
    /// on the canvas surfaces too — the canvas copy used to drop
    /// them silently.
    #[test]
    fn canvas_positional_rejects_extra_positional() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(
            run("canvas border padding 12 50", &mut doc),
            "unexpected extra positional",
        );
        assert_exec_err_contains(
            run("canvas section-frame focused preset heavy bogus", &mut doc),
            "unexpected extra positional",
        );
    }

    /// A missing value emits the same `usage:` line shape the
    /// per-node verb emits, labeled for the surface the user typed.
    #[test]
    fn canvas_positional_missing_value_emits_usage_for_its_surface() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(
            run("canvas border padding", &mut doc),
            "usage: canvas border padding",
        );
        assert_exec_err_contains(
            run("canvas section-frame focused color", &mut doc),
            "usage: canvas section-frame focused color",
        );
    }

    /// `cycle` is offered as a preset value on the canvas
    /// surfaces, so the unknown-preset error must advertise it —
    /// the canvas copy routed straight to the kv parser, whose
    /// error omitted `| cycle`.
    #[test]
    fn canvas_unknown_preset_error_advertises_cycle() {
        let mut doc = load_test_doc();
        assert_exec_err_contains(run("canvas border preset frobnicate", &mut doc), "| cycle");
    }

    /// The glyph gate names the surface being edited in both the
    /// diagnosis and the suggested fix.
    #[test]
    fn canvas_section_frame_corner_gate_names_its_own_surface() {
        let mut doc = load_test_doc();
        assert_exec_ok(run("canvas section-frame preset double", &mut doc));
        assert_exec_err_contains(
            run("canvas section-frame corner tl +", &mut doc),
            "run `canvas section-frame preset custom` first",
        );
    }
}
