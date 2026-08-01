// SPDX-License-Identifier: MPL-2.0

//! Read published examples out of the `format/` specs so tests pin
//! code against the *documentation*, not against a copy of it.
//!
//! The failure this exists to prevent: a test that restates a doc's
//! JSON as a string literal pins the code against that literal. Edit
//! the doc and the literal keeps agreeing with the code while the
//! spec drifts away from both — the test passes, the reader gets a
//! shape that no longer loads. `format/` is a normative spec; a
//! documented wire shape that does not parse has, in this repo,
//! already taken whole-document loading down.
//!
//! The precedent is
//! `gfx_structs::tests::area_tests::documented_rotate_example`, which
//! reads its inline-code example straight out of
//! `format/mutations.md` for exactly this reason. This module
//! generalizes the fenced-block half of that idea so every doc pin
//! shares one reader.
//!
//! Everything here panics loudly rather than degrading: a silent
//! fallback when the heading or the block has moved would defeat the
//! entire point.
//!
//! Native-only — the specs live on the filesystem, and wasm32 has no
//! filesystem to read them from. Cross-platform wire shapes are
//! pinned once on native (`TEST_CONVENTIONS.md` §T9).

use std::path::{Path, PathBuf};

/// Absolute path to `<repo>/format/<file_name>`.
///
/// Resolved from baumhard's own `CARGO_MANIFEST_DIR`, which `env!`
/// expands when *this crate* compiles — so the answer is
/// `<repo>/format/...` no matter which crate's test calls it, and
/// callers do not each hand-roll a `../..` hop of their own depth.
///
/// Cost: one `PathBuf` allocation. No I/O; the file is not opened
/// and its existence is not checked here — [`documented_json_block`]
/// reports that with a better message.
pub fn format_doc_path(file_name: &str) -> PathBuf {
    repo_doc_path("format").join(file_name)
}

/// Absolute path to `<repo>/<relative>`, for the reference docs that
/// live outside `format/` — `CONCEPTS.md`, `CODE_CONVENTIONS.md`,
/// `TEST_CONVENTIONS.md`. Same `CARGO_MANIFEST_DIR` anchor as
/// [`format_doc_path`], which is a thin wrapper over this.
///
/// Cost: one `PathBuf` allocation. No I/O.
pub fn repo_doc_path(relative: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(relative)
}

/// Return the `nth` (0-based) fenced ```` ```json ```` block that
/// appears after the Markdown heading line `heading` in the file at
/// `path`, with the fences stripped and surrounding whitespace
/// trimmed.
///
/// `heading` is matched as a whole line after trimming, so pass it
/// exactly as written including its `#` markers (e.g.
/// `"### Portal-mode edges"`). The search stops at the next heading
/// of the *same or shallower* depth, so a block belonging to a later
/// section can never be picked up by accident.
///
/// Panics — never returns a fallback — when the file is unreadable,
/// the heading is absent, or the section holds fewer than `nth + 1`
/// JSON blocks. Each message names the file and the heading, because
/// the caller is a test whose whole purpose is to notice that the
/// doc moved.
///
/// Cost: one full read of the Markdown file plus a single line scan
/// — O(file_size), paid once per test that pins a block.
pub fn documented_json_block(path: &Path, heading: &str, nth: usize) -> String {
    let doc =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));

    let heading_depth = heading_depth_of(heading).unwrap_or_else(|| {
        panic!("{heading:?} is not a Markdown heading — pass it with its leading '#' markers")
    });

    let mut lines = doc.lines();
    lines
        .find(|line| line.trim_end() == heading)
        .unwrap_or_else(|| panic!("{} no longer publishes the heading {heading:?}", path.display()));

    let mut seen = 0usize;
    let mut current: Option<Vec<&str>> = None;
    for line in lines {
        // A heading at the same or a shallower level ends the
        // section; anything past it belongs to a different concept.
        if current.is_none() {
            if let Some(depth) = heading_depth_of(line.trim_end()) {
                if depth <= heading_depth {
                    break;
                }
            }
        }

        match (&mut current, line.trim_end()) {
            (None, "```json") => current = Some(Vec::new()),
            (Some(_), "```") => {
                let block = current.take().expect("in-block by construction");
                if seen == nth {
                    return block.join("\n").trim().to_string();
                }
                seen += 1;
            }
            (Some(body), _) => body.push(line),
            (None, _) => {}
        }
    }

    panic!(
        "{} §{heading} publishes {seen} json block(s); block {nth} was requested. \
         Update the doc or the test — they are meant to move together.",
        path.display()
    );
}

/// Return the full body text of the Markdown section headed by
/// `heading` in the file at `path` — every line after the heading up
/// to the next heading of the *same or shallower* depth, headings of
/// deeper sections included.
///
/// The prose counterpart of [`documented_json_block`], for the doc
/// surfaces that publish a **vocabulary** rather than a wire shape: a
/// list of the values an enum field accepts, a table with one row per
/// variant. A test can then assert the section names every variant
/// the code defines, so adding a variant fails the suite instead of
/// leaving the spec a row short — the drift a hand-kept list in the
/// test itself would not catch.
///
/// `heading` is matched exactly as written, `#` markers and all, same
/// as [`documented_json_block`]. Panics — never returns a fallback —
/// when the file is unreadable or the heading is absent, because the
/// caller is a test whose whole purpose is to notice the doc moved.
///
/// Cost: one full read of the Markdown file plus a single line scan —
/// O(file_size), paid once per test that pins a section.
pub fn documented_section_text(path: &Path, heading: &str) -> String {
    let doc =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));

    let heading_depth = heading_depth_of(heading).unwrap_or_else(|| {
        panic!("{heading:?} is not a Markdown heading — pass it with its leading '#' markers")
    });

    let mut lines = doc.lines();
    lines
        .find(|line| line.trim_end() == heading)
        .unwrap_or_else(|| panic!("{} no longer publishes the heading {heading:?}", path.display()));

    let mut body: Vec<&str> = Vec::new();
    for line in lines {
        if let Some(depth) = heading_depth_of(line.trim_end()) {
            if depth <= heading_depth {
                break;
            }
        }
        body.push(line);
    }
    body.join("\n").trim().to_string()
}

/// Return the blank-line-delimited block *inside* `heading`'s
/// section that contains the line `marker` — the value list, the
/// table, the "Variants:" sentence — with the block's own line
/// breaks preserved.
///
/// A vocabulary pin run over a whole section is weaker than it
/// looks: prose elsewhere in the section mentions the variants in
/// passing, so deleting a variant from the construct that actually
/// *publishes* it leaves the section still naming it and the pin
/// still green. Deleting the `Descendants` row from the
/// `target_scope` table in `format/mutations.md` is the concrete
/// case — two later
/// paragraphs discuss `Descendants` by name. Narrowing to the
/// publishing block is what makes the deletion fail the suite.
///
/// `marker` is matched as a substring of a line, so pass the
/// distinguishing part of the block's first line (`"Variants:"`,
/// `"| Value |"`). Panics — never falls back — when the heading or
/// the marker is gone, per this module's contract: the caller is a
/// test whose purpose is to notice the doc moved.
///
/// Cost: one [`documented_section_text`] read plus a line scan of
/// the section — O(file_size).
pub fn documented_paragraph(path: &Path, heading: &str, marker: &str) -> String {
    let section = documented_section_text(path, heading);
    let mut block: Vec<&str> = Vec::new();
    let mut found = false;
    for line in section.lines() {
        if line.trim().is_empty() {
            if found {
                return block.join("\n");
            }
            block.clear();
        } else {
            block.push(line);
            found |= line.contains(marker);
        }
    }
    if found {
        return block.join("\n");
    }
    panic!(
        "{} §{heading} no longer holds a block containing {marker:?} — the doc moved, \
         so the pin has to move with it.",
        path.display()
    );
}

/// Whether `body` names `token` as a **whole word** — at least one
/// occurrence with a non-identifier character (or nothing at all) on
/// each side.
///
/// The companion of [`documented_section_text`] for vocabulary pins,
/// and the reason it exists is that `str::contains` is a substring
/// test while enum variant names nest. `Descendants` is a substring
/// of `SelfAndDescendants`, so a `contains` pin calls the narrow
/// variant published the moment a section names the wide one. That
/// is not hypothetical: with `contains`, `Descendants` could be
/// deleted from all three normative `target_scope` surfaces —
/// `format/schema.md`, `format/mutations.md`, `CONCEPTS.md` — and
/// the suite stayed green.
///
/// An identifier character is `[A-Za-z0-9_]`, so every delimiter the
/// specs actually put around a variant name counts as a boundary:
/// backtick, quote, comma, pipe, space, line end. No surface has to
/// agree with another on how it wraps the name, which is what makes
/// this usable across three docs that each wrap differently
/// (`` `Descendants` `` in two of them, `` `"Descendants"` `` in the
/// third).
///
/// Cost: O(body_len × token_len) worst case, no allocation.
pub fn names_token(body: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    body.match_indices(token).any(|(at, _)| {
        let before = body[..at].chars().next_back();
        let after = body[at + token.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Depth of a Markdown ATX heading (`## x` → 2), or `None` when the
/// line is not a heading. A run of `#` must be followed by a space
/// to count, so a `#[derive(...)]` inside a fenced block cannot be
/// mistaken for one.
fn heading_depth_of(line: &str) -> Option<usize> {
    let depth = line.chars().take_while(|c| *c == '#').count();
    if depth > 0 && line[depth..].starts_with(' ') {
        Some(depth)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_temp::TempDir;

    fn write_doc(dir: &TempDir, body: &str) -> PathBuf {
        let path = dir.join("doc.md");
        std::fs::write(&path, body).expect("write scratch doc");
        path
    }

    const SAMPLE: &str = "\
# Title

## First

```json
{ \"a\": 1 }
```

```json
{ \"a\": 2 }
```

### Nested

```json
{ \"a\": 3 }
```

## Second

```json
{ \"b\": 1 }
```
";

    #[test]
    fn test_documented_json_block_returns_the_requested_block() {
        let dir = TempDir::new("doc-fixtures-nth");
        let path = write_doc(&dir, SAMPLE);
        assert_eq!(documented_json_block(&path, "## First", 0), "{ \"a\": 1 }");
        assert_eq!(documented_json_block(&path, "## First", 1), "{ \"a\": 2 }");
    }

    /// A deeper heading is still inside the section, so its blocks
    /// keep counting — that is what lets a caller name the outer
    /// section and index through everything under it.
    #[test]
    fn test_documented_json_block_descends_into_deeper_headings() {
        let dir = TempDir::new("doc-fixtures-nested");
        let path = write_doc(&dir, SAMPLE);
        assert_eq!(documented_json_block(&path, "## First", 2), "{ \"a\": 3 }");
        assert_eq!(documented_json_block(&path, "### Nested", 0), "{ \"a\": 3 }");
    }

    /// A sibling heading ends the section: `## First` must not reach
    /// into `## Second`, or a doc reshuffle would silently repoint a
    /// pin at someone else's example.
    #[test]
    fn test_documented_json_block_stops_at_the_next_sibling_heading() {
        let dir = TempDir::new("doc-fixtures-stop");
        let path = write_doc(&dir, SAMPLE);
        let err = std::panic::catch_unwind(|| documented_json_block(&path, "## First", 3))
            .expect_err("block 3 is under ## Second and must not be found");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("publishes 3 json block(s)"), "got: {msg}");
    }

    #[test]
    fn test_documented_json_block_panics_when_the_heading_is_gone() {
        let dir = TempDir::new("doc-fixtures-missing");
        let path = write_doc(&dir, SAMPLE);
        let err = std::panic::catch_unwind(|| documented_json_block(&path, "## Absent", 0))
            .expect_err("a missing heading must panic, not fall back");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("no longer publishes the heading"), "got: {msg}");
    }

    #[test]
    fn test_documented_section_text_returns_the_whole_section() {
        let dir = TempDir::new("doc-fixtures-section");
        let path = write_doc(&dir, SAMPLE);
        let body = documented_section_text(&path, "## First");
        assert!(body.contains("{ \"a\": 1 }"), "got: {body}");
        assert!(
            body.contains("{ \"a\": 3 }"),
            "deeper sections belong to it: {body}"
        );
        assert!(
            !body.contains("{ \"b\": 1 }"),
            "must stop at the next sibling heading: {body}"
        );
        assert!(body.contains("### Nested"), "deeper headings are kept: {body}");
    }

    #[test]
    fn test_documented_section_text_panics_when_the_heading_is_gone() {
        let dir = TempDir::new("doc-fixtures-section-missing");
        let path = write_doc(&dir, SAMPLE);
        let err = std::panic::catch_unwind(|| documented_section_text(&path, "## Absent"))
            .expect_err("a missing heading must panic, not fall back");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("no longer publishes the heading"), "got: {msg}");
    }

    const VOCAB: &str = "\
# Title

## Scopes

Intro prose that mentions Descendants in passing.

Variants: `SelfOnly`, `Descendants`, `SelfAndDescendants`.

Trailing prose about Descendants again.
";

    /// The block, not the section: the surrounding prose names
    /// `Descendants` too, and a pin that sees it there cannot notice
    /// the listing losing an entry.
    #[test]
    fn test_documented_paragraph_returns_only_the_marked_block() {
        let dir = TempDir::new("doc-fixtures-paragraph");
        let path = write_doc(&dir, VOCAB);
        let block = documented_paragraph(&path, "## Scopes", "Variants:");
        assert_eq!(
            block,
            "Variants: `SelfOnly`, `Descendants`, `SelfAndDescendants`."
        );
    }

    #[test]
    fn test_documented_paragraph_panics_when_the_marker_is_gone() {
        let dir = TempDir::new("doc-fixtures-paragraph-missing");
        let path = write_doc(&dir, VOCAB);
        let err = std::panic::catch_unwind(|| documented_paragraph(&path, "## Scopes", "Values:"))
            .expect_err("a missing marker must panic, not fall back");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("no longer holds a block containing"), "got: {msg}");
    }

    /// A block that runs to the end of the section has no trailing
    /// blank line to close it.
    #[test]
    fn test_documented_paragraph_handles_a_block_at_the_end_of_a_section() {
        let dir = TempDir::new("doc-fixtures-paragraph-last");
        let path = write_doc(
            &dir,
            "## Scopes\n\nprose\n\n| Value | Meaning |\n|---|---|\n| `A` | a |\n",
        );
        let block = documented_paragraph(&path, "## Scopes", "| Value |");
        assert!(block.starts_with("| Value | Meaning |"), "got: {block}");
        assert!(block.ends_with("| `A` | a |"), "got: {block}");
        assert!(!block.contains("prose"), "got: {block}");
    }

    /// The case [`names_token`] exists for: a substring test cannot
    /// tell "the doc publishes `Descendants`" from "the doc
    /// publishes `SelfAndDescendants`, which happens to contain the
    /// letters". The first assertion is the one `contains` gets
    /// wrong.
    #[test]
    fn test_names_token_rejects_a_name_nested_inside_a_longer_one() {
        let body = "| `SelfAndDescendants` | Anchor + all descendants. |";
        assert!(
            body.contains("Descendants"),
            "precondition: this is exactly what a substring pin sees"
        );
        assert!(!names_token(body, "Descendants"));
        assert!(names_token(body, "SelfAndDescendants"));
    }

    /// Every delimiter the three normative surfaces actually wrap a
    /// variant name in has to read as a boundary, or the pin fails
    /// on a doc that is correct.
    #[test]
    fn test_names_token_accepts_every_delimiter_the_specs_use() {
        for body in [
            "| `Descendants` | All descendants recursively. |",
            "one of `\"SelfOnly\"`, `\"Descendants\"`, `\"Parent\"`.",
            "`Descendants` (not the anchor), `SelfAndDescendants`,",
            "Descendants",
            "wide scopes: Descendants, SelfAndDescendants",
        ] {
            assert!(names_token(body, "Descendants"), "must match in: {body}");
        }
    }

    /// Boundaries are symmetric, and an underscore is an identifier
    /// character — `self_only_scope` does not publish `self_only`.
    #[test]
    fn test_names_token_requires_a_boundary_on_both_sides() {
        assert!(!names_token("Descendantsly", "Descendants"));
        assert!(!names_token("xDescendants", "Descendants"));
        assert!(!names_token("self_only_scope", "self_only"));
        assert!(names_token("self_only scope", "self_only"));
        assert!(!names_token("nothing here", "Descendants"));
        assert!(!names_token("anything", ""), "an empty token names nothing");
    }

    /// The `format/` docs this module exists to read are reachable
    /// from wherever the suite runs.
    #[test]
    fn test_format_doc_path_resolves_to_the_repo_specs() {
        let path = format_doc_path("schema.md");
        assert!(
            path.is_file(),
            "format/schema.md must be readable at {}",
            path.display()
        );
    }
}
