// SPDX-License-Identifier: MPL-2.0

//! The shared `section=N` / `range=A..B` targeting vocabulary —
//! both halves of it.
//!
//! `range=A..B` lifts the Rust-style half-open form into
//! `(usize, usize)` grapheme indices that the range-aware document
//! setters (`set_section_text_color_range`, `_font_size_range`,
//! `_font_family_range`) consume directly; `section=N` lifts a
//! non-negative integer.
//!
//! Alongside the parsers sits [`section_idx_completions`], the
//! popup those same verbs offer on the value side of `section=`.
//! It lives here for the same reason the parsers do: `color`,
//! `font` and `section` all speak this one kv, and a copy per verb
//! is how `font section=<TAB>` came to offer point sizes — the
//! vocabulary of a *different* key on the same verb — while
//! `color section=<TAB>` offered color presets. Neither was wrong
//! about its own verb; both were reading a list that had nothing
//! to do with the key under the cursor.

use crate::application::console::completion::Completion;
use crate::application::console::ConsoleContext;

/// Parse a `range=A..B` kv value into `(start, end)` grapheme
/// indices. Accepts the Rust-style `usize..usize` half-open
/// form. Rejects empty halves, non-numeric components, and
/// `start >= end` (an empty or inverted range is a usage error
/// — the verb path lifts this to an `ExecResult::err`).
pub(super) fn parse_range_kv(value: &str) -> Result<(usize, usize), String> {
    let (start_str, end_str) = match value.split_once("..") {
        Some(pair) => pair,
        None => return Err("expected `start..end` (e.g. `range=2..7`)".to_string()),
    };
    if start_str.is_empty() || end_str.is_empty() {
        return Err("both halves of `start..end` must be non-empty".to_string());
    }
    let start: usize = start_str
        .parse()
        .map_err(|_| format!("`{}` is not a non-negative integer", start_str))?;
    let end: usize = end_str
        .parse()
        .map_err(|_| format!("`{}` is not a non-negative integer", end_str))?;
    if start >= end {
        return Err(format!(
            "empty / inverted range — `{}..{}` requires start < end",
            start, end
        ));
    }
    Ok((start, end))
}

/// Parse a `section=N` kv value into a non-negative integer.
/// Used by `color` / `font` / `section` verbs. Verb name is
/// prepended to the error message so the caller doesn't have to
/// re-format. Returns `Err` on negative or non-numeric input.
pub(super) fn parse_section_kv(verb: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{}: section='{}' is not a non-negative integer", verb, value))
}

/// The popup for `section=<TAB>`: one row per section on the
/// selection's primary node, each hinted with a short preview of
/// that section's text so the user can tell which is which.
///
/// Grapheme-aware truncation via `take_graphemes` — a preview cut
/// at 20 *bytes* would slice a multi-codepoint emoji in half.
pub(super) fn section_idx_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    use baumhard::util::grapheme_chad::take_graphemes;
    let Some(primary_id) = ctx.document.selection.primary_node_id() else {
        return Vec::new();
    };
    let Some(node) = ctx.document.mindmap.nodes.get(primary_id) else {
        return Vec::new();
    };
    node.sections
        .iter()
        .enumerate()
        .filter(|(idx, _)| idx.to_string().starts_with(partial))
        .map(|(idx, section)| {
            // One walk, no prefix allocation — `take_graphemes`
            // returns the borrowed prefix and the overflow flag
            // together. Empty sections render `(empty)` so the row
            // isn't a bare bullet.
            let (preview, overflow) = take_graphemes(&section.text, 20);
            let hint = if preview.is_empty() {
                "(empty)".to_string()
            } else if overflow {
                format!("\"{}…\"", preview)
            } else {
                format!("\"{}\"", preview)
            };
            Completion {
                text: idx.to_string(),
                display: idx.to_string(),
                hint: Some(hint),
                font_family: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_range_kv;

    #[test]
    fn test_parse_range_kv_happy_path() {
        assert_eq!(parse_range_kv("2..7"), Ok((2, 7)));
        assert_eq!(parse_range_kv("0..1"), Ok((0, 1)));
        assert_eq!(parse_range_kv("100..1000"), Ok((100, 1000)));
    }

    #[test]
    fn test_parse_range_kv_missing_separator() {
        assert!(parse_range_kv("27").is_err());
        assert!(parse_range_kv("").is_err());
    }

    #[test]
    fn test_parse_range_kv_empty_halves() {
        assert!(parse_range_kv("..7").is_err());
        assert!(parse_range_kv("2..").is_err());
        assert!(parse_range_kv("..").is_err());
    }

    #[test]
    fn test_parse_range_kv_non_numeric() {
        assert!(parse_range_kv("foo..bar").is_err());
        assert!(parse_range_kv("2..bar").is_err());
        assert!(parse_range_kv("foo..7").is_err());
    }

    #[test]
    fn test_parse_range_kv_inverted_or_empty() {
        // `start >= end` rejected — empty or inverted range is
        // a usage error rather than a silent no-op.
        assert!(parse_range_kv("5..5").is_err());
        assert!(parse_range_kv("7..3").is_err());
    }

    #[test]
    fn test_parse_range_kv_negative_rejected() {
        // `usize::parse` rejects negative integers — surface as
        // a clear error message rather than silent overflow.
        assert!(parse_range_kv("-1..5").is_err());
    }
}
