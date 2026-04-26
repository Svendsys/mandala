// SPDX-License-Identifier: MPL-2.0

//! `border` contextual completion. Mirrors `font.rs`'s structure:
//! token 0 surfaces verbs + kv keys; `KvValue { key }` matches a
//! per-key vocabulary (presets, palette names, font families).

use crate::application::console::completion::{
    prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::ConsoleContext;

pub fn complete_border(
    state: &CompletionState,
    ctx: &ConsoleContext,
) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => verb_or_key(state.partial),
        CompletionContext::Token { .. } => key_completions(state.partial),
        CompletionContext::KvValue { key } => match key.as_str() {
            "preset" => prefix_filter(super::PRESETS, state.partial),
            "field" => prefix_filter(super::FIELDS, state.partial),
            "color" => prefix_filter(super::COLOR_PRESETS, state.partial),
            "palette" => palette_value_completions(state.partial, ctx),
            "font" => font_family_completions(state.partial),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn verb_or_key(partial: &str) -> Vec<Completion> {
    let mut out: Vec<Completion> = Vec::new();
    for v in super::VERBS {
        if v.starts_with(partial) {
            out.push(Completion {
                text: v.to_string(),
                display: v.to_string(),
                hint: Some(verb_hint(v).to_string()),
                font_family: None,
            });
        }
    }
    out.extend(key_completions(partial));
    out
}

fn key_completions(partial: &str) -> Vec<Completion> {
    super::KEYS
        .iter()
        .filter(|k| k.starts_with(partial))
        .map(|k| Completion {
            text: format!("{}=", k),
            display: format!("{}=", k),
            hint: Some(key_hint(k).to_string()),
            font_family: None,
        })
        .collect()
}

fn verb_hint(v: &str) -> &'static str {
    match v {
        "on" => "show the border",
        "off" => "hide the border",
        "show" => "print the resolved config",
        "reset" => "drop the per-node override",
        _ => "",
    }
}

fn key_hint(k: &str) -> &'static str {
    match k {
        "preset" => "light | heavy | double | rounded | custom",
        "font" => "font family for border glyphs (use `font list` for names)",
        "size" => "border glyph size in points",
        "color" => "#hex, var(--name), preset, or 'reset'",
        "palette" => "palette name to cycle per-glyph colours, or 'off'",
        "field" => "frame | background | text | title",
        "padding" => "border-to-content padding in pixels",
        "top" | "bottom" | "left" | "right" => {
            "side pattern: `prefix(fill)suffix` or atomic"
        }
        "tl" | "tr" | "bl" | "br" => "single corner glyph (escapes apply)",
        _ => "",
    }
}

fn palette_value_completions(
    partial: &str,
    ctx: &ConsoleContext,
) -> Vec<Completion> {
    let mut names: Vec<&str> = ctx
        .document
        .mindmap
        .palettes
        .keys()
        .map(String::as_str)
        .collect();
    names.sort();
    let mut out: Vec<Completion> = names
        .into_iter()
        .filter(|n| n.starts_with(partial))
        .map(|n| Completion {
            text: n.to_string(),
            display: n.to_string(),
            hint: None,
            font_family: None,
        })
        .collect();
    if "off".starts_with(partial) {
        out.push(Completion {
            text: "off".to_string(),
            display: "off".to_string(),
            hint: Some("clear palette cycling".to_string()),
            font_family: None,
        });
    }
    out
}

fn font_family_completions(partial: &str) -> Vec<Completion> {
    let lower = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families_iter()
        .filter(|f| f.to_ascii_lowercase().starts_with(&lower))
        .map(|family| Completion {
            text: family.to_string(),
            display: family.to_string(),
            hint: None,
            font_family: Some(family.to_string()),
        })
        .collect()
}
