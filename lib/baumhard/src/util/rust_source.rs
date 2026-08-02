// SPDX-License-Identifier: MPL-2.0

//! Read this workspace's own Rust source for tests that pin a
//! *source-level* property — one no runtime assertion can see.
//!
//! Two properties are like that in practice, and both are load-bearing:
//! a statement whose only observable effect goes to a process-global
//! logger the suite does not install, and a branch that only compiles
//! on `wasm32` and so is never linked into `cargo test`
//! (`TEST_CONVENTIONS.md` §T9). A test for either has to read the
//! file.
//!
//! ## Why a module instead of `read_to_string` at the call site
//!
//! A naive `source.contains(needle)` is satisfied by **any** byte run
//! in the file, and the two cheapest places for one to hide are the
//! two a person deleting code actually writes:
//!
//! - **A comment.** Gutting a function and leaving
//!   `// this used to be log::error!("startup: {}", message);` keeps
//!   the scan green while the behavior is gone. That is not a
//!   contrived mutation — it is the ordinary shape of a removal.
//! - **The test's own body, or a later `#[cfg(test)]` module.** The
//!   needle a test asserts on is spelled out inside that test, so a
//!   whole-file scan of the file the test lives in matches itself.
//!   Appending a test module to a *different* file does the same to a
//!   scan of that file.
//!
//! [`production_code`](crate::util::rust_source::production_code)
//! closes both: it replaces every comment with a space
//! ([`strip_comments`](crate::util::rust_source::strip_comments)) and
//! drops everything from the first `#[cfg(test)]` module onward
//! ([`above_test_modules`](crate::util::rust_source::above_test_modules)),
//! so what comes back is the code that ships.
//! [`braced_block_after`](crate::util::rust_source::braced_block_after)
//! narrows further, to one item's body, for the pins that are about
//! one statement.
//!
//! ## What is still out of reach
//!
//! A **string literal** in production code holding the needle
//! verbatim satisfies the scan, because
//! [`strip_comments`](crate::util::rust_source::strip_comments)
//! leaves literals alone — it has to, since the needles here contain
//! string literals themselves
//! (`log::error!("startup: {}", message)`). That is the residual,
//! and it is a deliberate act rather than the ordinary shape of a
//! mistake: scoping a pin to one function body with
//! [`braced_block_after`](crate::util::rust_source::braced_block_after)
//! means hiding a needle requires planting a raw string inside the
//! very function under test.
//!
//! Native-only in effect — it reads files off the filesystem, and
//! nothing in a shipped build parses its own source. It is not
//! `#[cfg(test)]`-gated for the same reason
//! [`crate::util::doc_fixtures`] is not: the callers live in the
//! `mandala` crate, and a `cfg(test)` item in `baumhard` is invisible
//! to them.
//!
//! No criterion entry for
//! [`production_code`](crate::util::rust_source::production_code), on
//! the same precedent as [`crate::util::doc_fixtures`]: it is file
//! I/O on a cold test path, and a benchmark of it would measure the
//! page cache. The three pure scanners do carry bench entries
//! (`lib/baumhard/CONVENTIONS.md` §B7):
//! [`strip_comments`](crate::util::rust_source::strip_comments),
//! [`above_test_modules`](crate::util::rust_source::above_test_modules)
//! and
//! [`braced_block_after`](crate::util::rust_source::braced_block_after).

use crate::util::doc_fixtures::repo_path;

/// The attribute [`above_test_modules`] looks for. Spelled as a
/// `concat!` so this file's own text does not contain the literal it
/// searches for: `production_code("lib/baumhard/src/util/rust_source.rs")`
/// would otherwise truncate itself at this constant.
///
/// Searched **after** comments are stripped, so the many prose
/// mentions of the attribute in files like this one do not truncate
/// their own module at the first paragraph that names it. An
/// occurrence inside a *string literal* still truncates early, which
/// is the safe direction — a pin then scans less code and fails
/// rather than passing on text it should not have seen.
const TEST_ATTRIBUTE: &str = concat!("#[cfg(", "test)]");

/// Read `<repo>/<relative>` and return the part of it that ships:
/// [`strip_comments`], then [`above_test_modules`].
///
/// This is the reader every source-level pin in the workspace should
/// use. The two transforms are not tidiness — see the module docs
/// for the mutations each one kills. They run in that order because
/// prose mentions `#[cfg(test)]` far more often than code declares a
/// module with it.
///
/// Line breaks survive stripping, so a line number quoted in a
/// failure message still points at the right line of the original
/// file.
///
/// Panics — never degrades — when the file cannot be read, naming
/// the path: the caller is a test whose entire purpose is to notice
/// that the file moved.
///
/// Cost: one file read plus one pass over its bytes, paid once per
/// test that pins a source property.
pub fn production_code(relative: &str) -> String {
    let path = repo_path(relative);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
    above_test_modules(&strip_comments(&source)).to_string()
}

/// `code` up to its first `#[cfg(test)]` **module**, or all of it
/// when there is none.
///
/// The anchor is the module, not the attribute, and the difference is
/// load-bearing: `src/application/renderer/mod.rs` opens with a
/// `#[cfg(test)] use …` five hundred lines above the code a pin cares
/// about, and truncating there would hand every caller an empty
/// string — a scan that cannot fail, which is worse than no scan.
///
/// A visibility in front of the `mod` is accepted, since a
/// benchmark-reusable test tree is declared `pub mod tests;`
/// (`lib/baumhard/CONVENTIONS.md` §B8).
///
/// **What this deliberately does not remove:** a `#[cfg(test)]` on a
/// smaller item — a `use`, a helper `fn`, an `impl`. Those are code
/// that exists rather than prose that does not, so a needle inside
/// one is a needle in a real function, and the narrower pins built
/// on [`braced_block_after`] do not reach them anyway.
///
/// `code` should already be comment-free; a mention of the attribute
/// in prose would otherwise truncate a file at its own module docs.
///
/// Cost: one substring search plus a few bytes of lookahead per hit.
pub fn above_test_modules(code: &str) -> &str {
    let mut from = 0usize;
    while let Some(offset) = code[from..].find(TEST_ATTRIBUTE) {
        let at = from + offset;
        let after = &code[at + TEST_ATTRIBUTE.len()..];
        if declares_a_module(after) {
            return &code[..at];
        }
        from = at + TEST_ATTRIBUTE.len();
    }
    code
}

/// Whether `after` — the text immediately following a `#[cfg(test)]`
/// — opens a module declaration, allowing a leading visibility.
fn declares_a_module(after: &str) -> bool {
    let rest = after.trim_start();
    let rest = match rest.strip_prefix("pub") {
        // `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`.
        Some(tail) => match tail.strip_prefix('(') {
            Some(paren) => match paren.find(')') {
                Some(close) => paren[close + 1..].trim_start(),
                None => return false,
            },
            None => tail.trim_start(),
        },
        None => rest,
    };
    rest.strip_prefix("mod")
        .is_some_and(|tail| tail.starts_with(char::is_whitespace))
}

/// Replace every Rust comment in `src` with a single space, leaving
/// string, byte-string, raw-string and character literals exactly as
/// they are.
///
/// Literals are preserved on purpose: the things worth pinning in
/// this workspace *are* string literals — a log line's `"<area>: "`
/// prefix, a `startup_load::adopt(` call. A stripper that blanked
/// them would leave nothing to assert on.
///
/// What it understands, because Rust source here uses all of it:
///
/// - `//` line comments, including `///` and `//!` doc comments,
///   which are comments for this purpose — a doc comment is the
///   *most* likely place for a stale needle to survive;
/// - `/* */` block comments, **nested**, which Rust allows;
/// - `"..."` and `b"..."` with backslash escapes;
/// - `r"..."`, `r#"..."#`, `br##"..."##` at any hash count, where a
///   backslash escapes nothing and only the matching `"#…#` ends it;
/// - `'x'` character literals, distinguished from a `'a` lifetime by
///   looking for the closing quote rather than by guessing.
///
/// Newlines inside a comment are emitted, so the output has the same
/// line count as the input and a byte offset still lands on the same
/// line.
///
/// Cost: one `char_indices` pass over `src` and one `String` of at
/// most its length. Never slices at a non-boundary — every cut comes
/// from `char_indices`, so a comment full of emoji is as safe as one
/// full of ASCII.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("//") {
            // Line comment: consume to the newline, which is kept.
            let end = rest.find('\n').map_or(src.len(), |n| i + n);
            out.push(' ');
            i = end;
            continue;
        }
        if rest.starts_with("/*") {
            i = skip_block_comment(src, i, &mut out);
            continue;
        }
        if let Some(next) = raw_string_end(src, i) {
            out.push_str(&src[i..next]);
            i = next;
            continue;
        }
        if bytes[i] == b'"' {
            let next = quoted_end(src, i, '"');
            out.push_str(&src[i..next]);
            i = next;
            continue;
        }
        if bytes[i] == b'\'' {
            if let Some(next) = char_literal_end(src, i) {
                out.push_str(&src[i..next]);
                i = next;
                continue;
            }
            // A lifetime (`'a`, `'static`). Ordinary code.
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Consume the nested block comment starting at `from` (which points
/// at `/*`), pushing one space plus every newline it spanned into
/// `out`. Returns the byte offset just past the comment, or the end
/// of `src` for an unterminated one.
fn skip_block_comment(src: &str, from: usize, out: &mut String) -> usize {
    out.push(' ');
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("/*") {
            depth += 1;
            i += 2;
            continue;
        }
        if rest.starts_with("*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
            continue;
        }
        let ch = rest.chars().next().expect("in-bounds by the loop guard");
        if ch == '\n' {
            out.push('\n');
        }
        i += ch.len_utf8();
    }
    src.len()
}

/// Byte offset just past the `delim`-quoted literal starting at
/// `from`, honoring backslash escapes. An unterminated literal
/// yields the end of `src`.
fn quoted_end(src: &str, from: usize, delim: char) -> usize {
    let mut chars = src[from..].char_indices();
    chars.next(); // the opening delimiter
    let mut escaped = false;
    for (offset, ch) in chars {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            c if c == delim => return from + offset + ch.len_utf8(),
            _ => {}
        }
    }
    src.len()
}

/// If a raw string (`r"`, `r#"`, `br##"`, …) starts at `from`,
/// return the byte offset just past its closing `"#…#`. Otherwise
/// `None`.
///
/// The leading `r` / `br` must not be the tail of a longer
/// identifier — `for r in ...` is not a raw string — so the byte
/// before it is checked.
fn raw_string_end(src: &str, from: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if from > 0 {
        let prev = bytes[from - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    let mut i = from;
    if bytes.get(i) == Some(&b'b') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let hash_start = i;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let hashes = i - hash_start;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    let mut terminator = String::with_capacity(hashes + 1);
    terminator.push('"');
    for _ in 0..hashes {
        terminator.push('#');
    }
    match src[i..].find(&terminator) {
        Some(at) => Some(i + at + terminator.len()),
        None => Some(src.len()),
    }
}

/// If a character literal starts at the `'` at `from`, return the
/// byte offset just past its closing quote. `None` means the `'`
/// opens a lifetime instead.
///
/// The two are told apart by looking for the close, not by guessing
/// from the first character: `'\n'`, `'é'` and `'\u{1F600}'` are
/// literals, `'a` and `'static` are not.
fn char_literal_end(src: &str, from: usize) -> Option<usize> {
    let after = &src[from + 1..];
    let mut chars = after.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' {
        // `''` is not legal Rust; treat it as not-a-literal rather
        // than consuming a quote that may open something real.
        return None;
    }
    if first == '\\' {
        return Some(quoted_end(src, from, '\''));
    }
    match chars.next() {
        Some((offset, '\'')) => Some(from + 1 + offset + 1),
        _ => None,
    }
}

/// The `{ … }` block of the first item in `src` whose text contains
/// `header`, brace-matched, with the outer braces included.
///
/// `src` must already be comment-free — pass the output of
/// [`production_code`] or [`strip_comments`]. Braces inside string
/// and character literals are skipped, so a `"}"` in a message
/// cannot close the block early.
///
/// The point is scope: a pin that reads "this one statement is still
/// here" should fail when the statement leaves *that function*, not
/// merely when it leaves the file. `header` is matched as a
/// substring so a caller passes the signature prefix it cares about
/// (`"fn adopt("`) rather than reproducing the whole signature,
/// which would break on an unrelated parameter rename.
///
/// `None` when `header` does not occur, or when no `{` follows it —
/// both of which a caller should treat as a failure, because the
/// item it meant to pin is gone.
///
/// Cost: one pass over `src` from `header` to the matching brace.
pub fn braced_block_after<'a>(src: &'a str, header: &str) -> Option<&'a str> {
    let at = src.find(header)?;
    let open = at + src[at..].find('{')?;
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < src.len() {
        if let Some(next) = raw_string_end(src, i) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'"' => {
                i = quoted_end(src, i, '"');
                continue;
            }
            b'\'' => {
                if let Some(next) = char_literal_end(src, i) {
                    i = next;
                    continue;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open..=i]);
                }
            }
            _ => {}
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        i += ch.len_utf8();
    }
    None
}
