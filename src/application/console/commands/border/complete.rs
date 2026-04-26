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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::completion::CompletionContext;
    use crate::application::document::MindMapDocument;

    fn fixture_doc() -> MindMapDocument {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        MindMapDocument::load(&path).expect("testament map loads")
    }

    fn state<'a>(
        partial: &'a str,
        ctx: CompletionContext,
        tokens_owned: &'a [String],
    ) -> CompletionState<'a> {
        CompletionState {
            tokens: tokens_owned,
            cursor_token: 0,
            partial,
            context: ctx,
        }
    }

    /// `border <TAB>` at token 0 surfaces every verb (on / off /
    /// show / reset) plus every kv key — the user gets a
    /// scannable menu of every operation the verb supports.
    #[test]
    fn complete_token_zero_offers_verbs_and_kv_keys() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state("", CompletionContext::Token { index: 0 }, &tokens);
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.display.as_str()).collect();
        // Verbs.
        for v in &["on", "off", "show", "reset"] {
            assert!(
                labels.iter().any(|l| l == v),
                "expected verb '{}' in completions: {:?}",
                v, labels
            );
        }
        // A handful of kv keys.
        for k in &["preset=", "font=", "size=", "color=", "palette="] {
            assert!(
                labels.iter().any(|l| l == k),
                "expected kv key '{}' in completions: {:?}",
                k, labels
            );
        }
    }

    /// `border preset=<TAB>` lists every preset name. Static
    /// vocabulary; doesn't need the document.
    #[test]
    fn complete_preset_value_lists_five_presets() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue { key: "preset".to_string() },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        for p in &["light", "heavy", "double", "rounded", "custom"] {
            assert!(
                labels.iter().any(|l| l == p),
                "expected preset '{}' in completions: {:?}",
                p, labels
            );
        }
    }

    /// `border palette=<TAB>` lists every palette in the
    /// document plus the `off` sentinel for clearing. Dynamic —
    /// reaches into `doc.mindmap.palettes`.
    #[test]
    fn complete_palette_value_lists_doc_palettes_and_off() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue { key: "palette".to_string() },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        // Off sentinel must always appear.
        assert!(
            labels.iter().any(|l| l == &"off"),
            "expected 'off' sentinel: {:?}",
            labels
        );
        // Every palette key in the doc must appear.
        for name in doc.mindmap.palettes.keys() {
            assert!(
                labels.iter().any(|l| l == &name.as_str()),
                "expected palette '{}' in completions: {:?}",
                name, labels
            );
        }
    }

    /// `border field=<TAB>` lists the four `ColorGroup` channels.
    #[test]
    fn complete_field_value_lists_four_channels() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue { key: "field".to_string() },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        for f in &["frame", "background", "text", "title"] {
            assert!(
                labels.iter().any(|l| l == f),
                "expected field '{}' in completions: {:?}",
                f, labels
            );
        }
    }

    /// `border font=<TAB>` reuses the font-family completer:
    /// every popup row carries `font_family = Some(<name>)` so
    /// the renderer shapes the candidate label in that face.
    /// Mirrors `font.rs::tests::completion_after_set_returns_loaded_families_in_their_face`.
    #[test]
    fn complete_font_value_rows_carry_family_tag() {
        baumhard::font::fonts::init();
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue { key: "font".to_string() },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        assert!(!out.is_empty(), "loaded fonts list must not be empty");
        for c in &out {
            assert_eq!(
                c.font_family.as_deref(),
                Some(c.text.as_str()),
                "every font-completion row must tag its display family"
            );
        }
    }
}
