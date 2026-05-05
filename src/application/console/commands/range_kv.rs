// SPDX-License-Identifier: MPL-2.0

//! Shared `range=A..B` kv parser used by `color` and `font`
//! verbs (and any future range-targeted console verb). Lifts the
//! Rust-style `start..end` half-open form into `(usize, usize)`
//! grapheme indices that the range-aware document setters
//! (`set_section_text_color_range`, `_font_size_range`,
//! `_font_family_range`) consume directly.
//!
//! Lives next to the verb modules rather than inside one of them
//! because both call sites need the same parser — the per-call-
//! site copy would be the kind of duplication CODE_CONVENTIONS §5
//! forbids.

/// Parse a `range=A..B` kv value into `(start, end)` grapheme
/// indices. Accepts the Rust-style `usize..usize` half-open
/// form. Rejects empty halves, non-numeric components, and
/// `start >= end` (an empty or inverted range is a usage error
/// — the verb path lifts this to an `ExecResult::err`).
pub fn parse_range_kv(value: &str) -> Result<(usize, usize), String> {
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
