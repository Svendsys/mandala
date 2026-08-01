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
//! generalizes that idea so every doc pin shares one reader:
//! [`documented_json_block`] for a fenced example,
//! [`section_text`] for the prose of a named section. Both are
//! section-scoped, which is the property a whole-file `contains()`
//! lacks — it keeps passing after the thing it claims to pin has
//! moved somewhere else entirely.
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
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../format")).join(file_name)
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
    documented_block(path, heading, nth, "json")
}

/// Return the `nth` (0-based) **untagged** fenced block (a bare
/// ```` ``` ```` with no language after it) under the heading
/// `heading` in the file at `path`.
///
/// The blocks this reads are the ones a spec uses to publish program
/// *output* rather than input: an error message, a CLI transcript.
/// Those are as much a contract as a JSON example and drift the same
/// way — a published error that no longer matches the one the code
/// emits is a spec that lies. Tagged blocks (```` ```json ````,
/// ```` ```rust ````) are skipped entirely, so the index counts only
/// untagged ones and adding a JSON example above cannot repoint a pin.
///
/// Same rules as [`documented_json_block`] otherwise: `heading` is
/// matched as a whole line including its `#` markers, the search
/// stops at the next heading of the same or shallower depth, and
/// every failure panics naming the file and the heading.
///
/// A caller comparing this against a runtime message should normalize
/// whitespace on both sides — a doc wraps its fences to the column
/// limit and the emitted message does not.
///
/// Cost: one full read of the Markdown file plus a single line scan
/// — O(file_size), paid once per test that pins a block.
pub fn documented_plain_block(path: &Path, heading: &str, nth: usize) -> String {
    documented_block(path, heading, nth, "")
}

/// Collapse every run of whitespace to a single space and trim, so a
/// wrapped doc block can be compared against an unwrapped runtime
/// string for equality.
///
/// A `format/` doc hard-wraps its fences to the column limit; the
/// message the code emits is one long line. Comparing the two
/// verbatim would pin the test against the doc's *line breaks* —
/// a typesetting decision — and force a re-flow every time the
/// message changes length. Everything that carries meaning (wording,
/// key order, punctuation, which keys are listed) still has to match
/// exactly.
///
/// Cost: one allocation sized to the input.
pub fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Shared fence scanner behind [`documented_json_block`] and
/// [`documented_plain_block`]. `info` is the language tag a block
/// must carry to be counted (`""` for untagged blocks); blocks with
/// any other tag are read past and never counted, so the two indexes
/// are independent of each other.
fn documented_block(path: &Path, heading: &str, nth: usize, info: &str) -> String {
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
    // `Some((language tag, body))` while inside a fence.
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in lines {
        let trimmed = line.trim_end();
        if current.is_none() {
            // A heading at the same or a shallower level ends the
            // section; anything past it belongs to a different
            // concept. Only checked outside a fence, so a `# comment`
            // inside a block cannot end the search.
            if let Some(depth) = heading_depth_of(trimmed) {
                if depth <= heading_depth {
                    break;
                }
            }
            if let Some(tag) = trimmed.strip_prefix("```") {
                current = Some((tag.trim().to_string(), Vec::new()));
            }
            continue;
        }
        if trimmed == "```" {
            let (tag, body) = current.take().expect("in-block by construction");
            if tag == info {
                if seen == nth {
                    return body.join("\n").trim().to_string();
                }
                seen += 1;
            }
        } else if let Some((_, body)) = &mut current {
            body.push(line);
        }
    }

    let label = if info.is_empty() { "untagged" } else { info };
    panic!(
        "{} §{heading} publishes {seen} {label} block(s); block {nth} was requested. \
         Update the doc or the test — they are meant to move together.",
        path.display()
    );
}

/// Return the body of the Markdown section introduced by the heading
/// line `heading` in the file at `path` — everything after the heading
/// up to the next heading of the *same or shallower* depth, with the
/// heading line itself excluded and surrounding whitespace trimmed.
///
/// This is the prose sibling of [`documented_json_block`]: use it when
/// a test needs to pin what a *named section* says rather than what a
/// fenced block contains. A whole-file `contains()` is not a substitute
/// — it stays green when the paragraph is moved to a different section
/// or the heading it claims to pin is renamed away, which is exactly
/// the silent drift this module exists to refuse.
///
/// `heading` is matched as a whole line after trimming, so pass it
/// exactly as written including its `#` markers (e.g. `"## §9 Error
/// handling"`).
///
/// Fenced blocks are transparent to the section scan: a shell comment
/// (`# note`) inside a ``` fence is content, not a heading, so it
/// cannot truncate the section early.
///
/// Panics — never returns a fallback — when the file is unreadable or
/// the heading is absent. An empty section is returned as `""` rather
/// than panicking; a test that cares asserts on the content.
///
/// Cost: one full read of the Markdown file plus a single line scan —
/// O(file_size), paid once per test that pins a section.
pub fn section_text(path: &Path, heading: &str) -> String {
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
    let mut in_fence = false;
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence {
            if let Some(depth) = heading_depth_of(trimmed) {
                if depth <= heading_depth {
                    break;
                }
            }
        }
        body.push(line);
    }
    body.join("\n").trim().to_string()
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

    const MIXED: &str = "\
# Title

## Output

```json
{ \"a\": 1 }
```

```
first plain
```

```rust
let x = 1;
```

```
second plain
```

## Second

```
elsewhere
```
";

    /// The two indexes are independent: a JSON example added above an
    /// error transcript must not shift which transcript a pin reads.
    #[test]
    fn test_documented_plain_block_counts_only_untagged_fences() {
        let dir = TempDir::new("doc-fixtures-plain");
        let path = write_doc(&dir, MIXED);
        assert_eq!(documented_plain_block(&path, "## Output", 0), "first plain");
        assert_eq!(documented_plain_block(&path, "## Output", 1), "second plain");
        assert_eq!(documented_json_block(&path, "## Output", 0), "{ \"a\": 1 }");
    }

    /// A tagged fence is read past rather than treated as an opener
    /// and a closer, so its body can never be mistaken for a plain
    /// block of its own.
    #[test]
    fn test_documented_plain_block_stops_at_the_next_sibling_heading() {
        let dir = TempDir::new("doc-fixtures-plain-stop");
        let path = write_doc(&dir, MIXED);
        let err = std::panic::catch_unwind(|| documented_plain_block(&path, "## Output", 2))
            .expect_err("block 2 is under ## Second and must not be found");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("publishes 2 untagged block(s)"), "got: {msg}");
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

    /// A section stops at its next sibling heading and keeps its
    /// deeper subsections — the property that makes it a real pin
    /// rather than a whole-file substring search.
    #[test]
    fn test_section_text_stops_at_the_next_sibling_heading() {
        let dir = TempDir::new("doc-fixtures-section");
        let path = write_doc(&dir, SAMPLE);
        let first = section_text(&path, "## First");
        assert!(first.contains("{ \"a\": 1 }"), "got: {first}");
        assert!(
            first.contains("### Nested"),
            "deeper headings stay in; got: {first}"
        );
        assert!(
            !first.contains("{ \"b\": 1 }"),
            "## Second must not leak in; got: {first}"
        );
    }

    /// A `#`-prefixed line inside a fence is content, not a heading,
    /// so it must not truncate the section.
    #[test]
    fn test_section_text_ignores_headings_inside_fences() {
        let dir = TempDir::new("doc-fixtures-section-fence");
        let path = write_doc(
            &dir,
            "## Only\n\n```sh\n# not a heading\n```\n\ntail line\n\n## Next\n\nother\n",
        );
        let body = section_text(&path, "## Only");
        assert!(
            body.contains("tail line"),
            "fence must not end the section; got: {body}"
        );
        assert!(!body.contains("other"), "## Next must not leak in; got: {body}");
    }

    #[test]
    fn test_section_text_panics_when_the_heading_is_gone() {
        let dir = TempDir::new("doc-fixtures-section-missing");
        let path = write_doc(&dir, SAMPLE);
        let err = std::panic::catch_unwind(|| section_text(&path, "## Absent"))
            .expect_err("a missing heading must panic, not fall back");
        let msg = err.downcast_ref::<String>().expect("panic payload is a String");
        assert!(msg.contains("no longer publishes the heading"), "got: {msg}");
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
