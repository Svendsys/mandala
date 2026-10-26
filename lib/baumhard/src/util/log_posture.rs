// SPDX-License-Identifier: MPL-2.0

//! CODE_CONVENTIONS §9's two log-content rules, held against the tree
//! instead of written down and trusted (issue #110):
//!
//! 1. **Every shipped `warn!` / `error!` names its area.** The
//!    message is a string literal of the form `"<area>: …"`, where the
//!    area is one or more lowercase words joined by `::`, a space or
//!    a hyphen — `"macros: …"`, `"font::attrs: …"`, `"console
//!    history: …"`, `"type-to-edit: …"`. A function name
//!    (`"apply_custom_mutation: …"`) passes this reader, because
//!    telling a function from a subsystem is a judgment the reader
//!    cannot make; a CamelCase `Action` name, a `{placeholder}` where
//!    the area should be, and a message with no prefix at all do not.
//!    A message that is not a literal — `warn!(msg)`,
//!    `warn!(format!(…))` — is reported too: the reader cannot see its
//!    prefix, and neither can the next person.
//! 2. **No shipped `warn!` / `error!` formats a path raw.** A
//!    `.display()` or `.to_string_lossy()` inside the invocation puts
//!    `/home/<user>/…` into a line a release build keeps; the site
//!    goes through [`crate::util::log::tilde_path`] instead. A path
//!    that reaches the message some other way — `{path:?}`, or a
//!    string composed elsewhere — is outside what a lexical reader
//!    can see, and the sites that do that are routed through
//!    [`crate::util::log::collapse_home_in`] by hand.
//!
//! The first rule was introduced by PR #104 and violated by its own
//! passthrough sites in the same change, which is the case for a
//! reader over a list: a rule that is not held is not a rule a new
//! reader can tell from a wish. The derived set is what #110's
//! acceptance criterion asks for in place of a hand-kept list of
//! conforming sites (`lib/baumhard/CONVENTIONS.md` §B4).
//!
//! **What counts as shipped** is `unwrap_posture`'s answer, reused:
//! [`production_rust_sources`] for the file list and
//! [`production_source`] for the text, so a `#[cfg(test)]` body's
//! `warn!("capture-test: …")` is never read and a wasm32-only arm
//! always is — both targets ship the channel (§4).
//!
//! **An invocation is read as tokens, not as a word.** Comments come
//! off first and every string literal is blanked before the search,
//! so `warn!` inside a doc comment or inside another message is not
//! an invocation; `my_warn!` is excluded by the identifier byte
//! before it and `#[warn(…)]` by the missing `!`. The message is the
//! first top-level argument after any `key = value;` prefix, read
//! back out of the unblanked text, so `log::warn!(code = X; "area: …")`
//! is classified by its literal and not by its code.
//!
//! Test-only and native-only, for the same reasons
//! [`crate::util::source_scan`] is.

use crate::util::rust_source::{blank_string_literals, matching_delimiter, string_literals, strip_comments};
use crate::util::source_scan::{production_source, relative_to_repo};
use crate::util::unwrap_posture::production_rust_sources;
use std::path::{Path, PathBuf};

/// The two macro names whose lines a release build keeps.
const LEVELS: &[&str] = &["warn", "error"];

/// Method calls that put a path into a message as bytes.
const RAW_PATH_FORMATTERS: &[&str] = &[".display()", ".to_string_lossy()"];

/// One `warn!` / `error!` invocation as the reader saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invocation {
    /// 1-based line of the macro name.
    pub(crate) line: usize,
    /// The invocation's text with every string literal blanked —
    /// what the raw-path rule is read from.
    pub(crate) blanked: String,
    /// The message literal's contents, or `None` when the message
    /// argument is not a string literal.
    pub(crate) message: Option<String>,
}

/// Why an invocation is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// The message argument is not a string literal.
    NotALiteral,
    /// The literal does not start with `"<area>: "`.
    NoAreaPrefix,
    /// The invocation formats a path with `.display()` or
    /// `.to_string_lossy()`.
    RawPath,
}

impl Fault {
    fn remedy(self) -> &'static str {
        match self {
            Fault::NotALiteral => "the message must be a string literal (CODE_CONVENTIONS §9 prefix idiom)",
            Fault::NoAreaPrefix => "the message must start with `\"<area>: \"` (CODE_CONVENTIONS §9)",
            Fault::RawPath => "format the path through `util::log::tilde_path` (CODE_CONVENTIONS §9)",
        }
    }
}

/// The area a conforming message names, or `None`.
///
/// The area is one or more words of `[a-z0-9_]`, the first starting
/// with a letter, joined by `::`, a single space or a hyphen, and
/// followed by `: ` (or a bare `:` ending the literal).
pub(crate) fn area_prefix(literal: &str) -> Option<&str> {
    let bytes = literal.as_bytes();
    let mut i = 0usize;
    loop {
        // One word.
        let word_start = i;
        if !bytes.get(i).is_some_and(u8::is_ascii_lowercase)
            && !(i > 0 && bytes.get(i).is_some_and(u8::is_ascii_digit))
        {
            return None;
        }
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        {
            i += 1;
        }
        debug_assert!(i > word_start);
        // What follows it.
        match (bytes.get(i), bytes.get(i + 1)) {
            (Some(b':'), Some(b':')) => i += 2,
            (Some(b':'), Some(b' ')) | (Some(b':'), None) => return Some(&literal[..i]),
            (Some(b' '), Some(next)) | (Some(b'-'), Some(next)) if next.is_ascii_lowercase() => i += 1,
            _ => return None,
        }
    }
}

/// Every `warn!` / `error!` invocation in `code`, which must already
/// be comment-free (`strip_comments`) and shipped-only
/// (`production_source`).
///
/// Cost: one literal-blanking pass, one search per level name, and
/// one delimiter walk per invocation.
pub(crate) fn log_invocations(code: &str) -> Vec<Invocation> {
    let blanked = blank_string_literals(code);
    let bytes = blanked.as_bytes();
    let mut found = Vec::new();
    for level in LEVELS {
        let mut from = 0usize;
        while let Some(at) = blanked[from..].find(level) {
            let start = from + at;
            let end = start + level.len();
            from = end;
            if start > 0 && is_ident_byte(bytes[start - 1]) {
                continue;
            }
            if bytes.get(end) != Some(&b'!') {
                continue;
            }
            let after_bang = blanked[end + 1..].trim_start();
            if !after_bang.starts_with('(') {
                continue;
            }
            let open = blanked.len() - after_bang.len();
            let Some(close) = matching_delimiter(&blanked, open, b'(', b')') else {
                // An unterminated invocation is a file that does not
                // compile; report it as one with no message rather
                // than skip it.
                found.push(Invocation {
                    line: line_of(code, start),
                    blanked: blanked[start..].to_string(),
                    message: None,
                });
                continue;
            };
            let args = &blanked[open + 1..close];
            let message_from = message_start(args);
            let message_blanked = args[message_from..].trim_start();
            let message = if is_literal_start(message_blanked) {
                string_literals(&code[open + 1 + message_from..close])
                    .first()
                    .map(|lit| (*lit).to_string())
            } else {
                None
            };
            found.push(Invocation {
                line: line_of(code, start),
                blanked: blanked[start..=close].to_string(),
                message,
            });
        }
    }
    found.sort_by_key(|inv| inv.line);
    found
}

/// Offset into `args` (an invocation's blanked argument text) where
/// the message starts: after a `key = value;` prefix when one is
/// present at depth zero before the first top-level comma, else `0`.
fn message_start(args: &str) -> usize {
    let mut depth = 0usize;
    for (i, byte) in args.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => return i + 1,
            b',' if depth == 0 => return 0,
            _ => {}
        }
    }
    0
}

/// Whether blanked argument text begins with a string literal: a
/// plain `"`, a byte string, or a raw string with any number of
/// hashes.
fn is_literal_start(text: &str) -> bool {
    let text = text.strip_prefix('b').unwrap_or(text);
    if text.starts_with('"') {
        return true;
    }
    let Some(raw) = text.strip_prefix('r') else {
        return false;
    };
    raw.trim_start_matches('#').starts_with('"')
}

fn line_of(code: &str, offset: usize) -> usize {
    code[..offset].matches('\n').count() + 1
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The faults one invocation carries, in a fixed order.
pub(crate) fn faults_of(invocation: &Invocation) -> Vec<Fault> {
    let mut faults = Vec::new();
    match &invocation.message {
        None => faults.push(Fault::NotALiteral),
        Some(literal) if area_prefix(literal).is_none() => faults.push(Fault::NoAreaPrefix),
        Some(_) => {}
    }
    if RAW_PATH_FORMATTERS
        .iter()
        .any(|call| invocation.blanked.contains(call))
    {
        faults.push(Fault::RawPath);
    }
    faults
}

/// What the reader found in one file's shipped text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    /// How many invocations were read, faulted or not — the floor a
    /// clean report is measured against.
    pub(crate) invocations: usize,
    /// One line per fault, `"<file>:<line>: <remedy>: <head>"`.
    pub(crate) offenders: Vec<String>,
}

impl Report {
    fn absorb(&mut self, other: Report) {
        self.invocations += other.invocations;
        self.offenders.extend(other.offenders);
    }
}

/// Read `text` as the shipped source of `file` and report it.
///
/// `file` decides which `cfg(test)` shapes `production_source`
/// excises and how the offender is named; it need not exist on
/// disk, which is what lets a test hand this a planted snippet.
pub(crate) fn report_for(file: &Path, text: &str) -> Report {
    let code = strip_comments(&production_source(file, text));
    let name = relative_to_repo(file);
    let mut report = Report::default();
    for invocation in log_invocations(&code) {
        report.invocations += 1;
        let head: String = invocation
            .message
            .as_deref()
            .unwrap_or_else(|| invocation.blanked.lines().next().unwrap_or(""))
            .chars()
            .take(60)
            .collect();
        for fault in faults_of(&invocation) {
            report.offenders.push(format!(
                "{name}:{}: {}: {head:?}",
                invocation.line,
                fault.remedy()
            ));
        }
    }
    report
}

/// [`report_for`] over every file, read from disk.
///
/// Cost: one read and one `syn` parse per file (the test-gate
/// resolution), plus the passes [`log_invocations`] names.
pub(crate) fn report_for_files(files: &[PathBuf]) -> Report {
    let mut total = Report::default();
    for file in files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        total.absorb(report_for(file, &text));
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::unwrap_posture::SHIPPED_ROOTS;

    /// A shipped file's worth of invocations with every shape the
    /// reader must get right, and the line each offender sits on.
    /// Planted, not borrowed from the tree, so the control does not
    /// move when the tree is cleaned.
    const CONTROL: &str = r#"
use log::{error, warn};
fn f(path: &std::path::Path, msg: &str) {
    warn!("area: fine");                                   // 4  ok
    log::error!("font::attrs: fine {msg}");                // 5  ok
    log::warn!(code = SOME_CODE; "console history: fine"); // 6  ok
    warn!(
        "type-to-edit: fine across\
         a line break"
    );                                                     // 7  ok
    warn!("Width must be positive");                       // 11 no prefix
    error!("SetFont: invalid");                            // 12 no prefix (CamelCase)
    warn!("{}: load failed", msg);                         // 13 no prefix (placeholder)
    warn!(msg);                                            // 14 not a literal
    warn!("capture: could not evict {}", path.display());  // 15 raw path
    let s = "log::warn!(not an invocation)";
    my_warn!("not an invocation either");
    #[warn(clippy::all)]
    fn g() {}
}
"#;

    /// **Every shipped `warn!` / `error!` carries an area prefix and
    /// formats no path raw** — CODE_CONVENTIONS §9, enforced (#110).
    ///
    /// The input that makes it fail is a `warn!("oops")`, a
    /// `warn!(msg)` or a `warn!("x: {}", p.display())` written in any
    /// shipped file. Three assertions stand in front of the offender
    /// list so its emptiness is evidence: the shipped roots are all
    /// walked, the reader found a floor's worth of invocations in the
    /// tree (a reader that stopped recognizing the macros would
    /// report a clean tree forever), and the same reader over the
    /// planted control reports exactly the five planted faults on
    /// their lines and nothing else.
    #[test]
    fn test_every_shipped_log_line_names_its_area_and_formats_no_path_raw() {
        let files = production_rust_sources();
        for root in SHIPPED_ROOTS {
            assert!(
                files.iter().any(|f| relative_to_repo(f).starts_with(root)),
                "the walk found no shipped source under `{root}`, so a bare `warn!` written \
                 in it would pass in silence"
            );
        }

        let control = report_for(Path::new("log_posture_control.rs"), CONTROL);
        assert_eq!(
            control.invocations, 9,
            "the control plants nine invocations: {control:#?}"
        );
        let expected = [
            (11, Fault::NoAreaPrefix),
            (12, Fault::NoAreaPrefix),
            (13, Fault::NoAreaPrefix),
            (14, Fault::NotALiteral),
            (15, Fault::RawPath),
        ];
        assert_eq!(
            control.offenders.len(),
            expected.len(),
            "the control plants exactly these faults: {control:#?}"
        );
        for ((line, fault), reported) in expected.iter().zip(&control.offenders) {
            let prefix = format!("log_posture_control.rs:{line}: {}", fault.remedy());
            assert!(
                reported.starts_with(&prefix),
                "expected an offender starting {prefix:?}, got {reported:?}"
            );
        }

        let report = report_for_files(&files);
        // Floor well under the roughly 170 shipped invocations the
        // workspace holds today: a reader that dropped whole files or
        // stopped matching lands here before it can hollow out the
        // check.
        assert!(
            report.invocations >= 120,
            "the reader found only {} shipped `warn!` / `error!` invocations where roughly \
             170 exist; it has lost most of the surface it checks",
            report.invocations
        );
        assert!(
            report.offenders.is_empty(),
            "{} shipped log line(s) break CODE_CONVENTIONS §9's content rules:\n  {}",
            report.offenders.len(),
            report.offenders.join("\n  ")
        );
    }

    /// The prefix reader accepts the idiom's spellings and rejects
    /// each neighbor by the thing that makes it not an area: an
    /// uppercase start, a placeholder, a quote or brace after the
    /// first word, a missing colon, a missing space after it.
    #[test]
    fn test_area_prefix_accepts_the_idiom_and_rejects_its_neighbors() {
        let accepted = [
            ("macros: x", "macros"),
            ("font::attrs: x", "font::attrs"),
            ("console history: x", "console history"),
            ("type-to-edit: x", "type-to-edit"),
            ("section move fan-out: x", "section move fan-out"),
            ("gfx_structs::area: x", "gfx_structs::area"),
            ("save:", "save"),
            ("ipc2: x", "ipc2"),
        ];
        for (literal, area) in accepted {
            assert_eq!(area_prefix(literal), Some(area), "{literal:?}");
        }
        let rejected = [
            "Width has to be positive",
            "SetFont{{slot}}: invalid",
            "WASM: canvas",
            "{}: {} load failed",
            "{label}: x",
            "mutation '{}': x",
            "font {}: x",
            "color verb on section {} of node",
            "area:x",
            "area x",
            "",
            ": x",
            "::x: y",
        ];
        for literal in rejected {
            assert_eq!(area_prefix(literal), None, "{literal:?}");
        }
    }

    /// Invocations are found through the `log::` path, a bare
    /// import, a `key = value;` prefix and a line break between the
    /// name and the parenthesis — and not inside a comment, inside
    /// another literal, behind a longer identifier, or in the
    /// `#[warn(…)]` attribute. The message read back is the literal's
    /// contents, not the code's.
    #[test]
    fn test_invocations_are_found_through_every_spelling_and_not_in_lookalikes() {
        let code = strip_comments(
            r#"
            // warn!("in a comment")
            log::warn!("a: one");
            error!("b: two");
            log::warn!(code = "not-the-message"; "c: three");
            warn!
            ("d: four");
            let s = "warn!(\"in a literal\")";
            my_warn!("e: five");
            #[warn(clippy::all)]
            fn g() {}
            "#,
        );
        let found = log_invocations(&code);
        let messages: Vec<Option<&str>> = found.iter().map(|i| i.message.as_deref()).collect();
        assert_eq!(
            messages,
            vec![Some("a: one"), Some("b: two"), Some("c: three"), Some("d: four")]
        );
        assert_eq!(found[0].line, 3);
        assert_eq!(found[3].line, 6, "the line reported is the macro name's");
    }

    /// A message that is not a literal is `None` — a binding, a
    /// `format!`, a `concat!` — while a literal is read whole across
    /// a line continuation and through a raw string.
    #[test]
    fn test_a_message_that_is_not_a_literal_is_read_as_none() {
        let code = strip_comments(
            r##"
            warn!(msg);
            warn!(format!("a: {}", 1));
            warn!(concat!("a", ": b"));
            warn!("a: split \
                   across");
            warn!(r#"a: raw "quoted""#);
            warn!(b"a: bytes");
            "##,
        );
        let messages: Vec<Option<String>> = log_invocations(&code).into_iter().map(|i| i.message).collect();
        assert_eq!(
            messages,
            vec![
                None,
                None,
                None,
                Some("a: split \\\n                   across".to_string()),
                Some("a: raw \"quoted\"".to_string()),
                Some("a: bytes".to_string()),
            ]
        );
    }

    /// `.display()` inside an invocation is a fault; the same call on
    /// the line before, or spelled inside the message literal, is not.
    #[test]
    fn test_a_raw_path_formatter_counts_only_inside_the_invocation() {
        let code = strip_comments(
            r#"
            let shown = path.display();
            warn!("a: {shown}");
            warn!("a: see .display() in the docs");
            warn!("a: {}", path.display());
            error!("a: {}", path.to_string_lossy());
            warn!("a: {}", tilde_path(path));
            "#,
        );
        let faults: Vec<Vec<Fault>> = log_invocations(&code).iter().map(faults_of).collect();
        assert_eq!(
            faults,
            vec![vec![], vec![], vec![Fault::RawPath], vec![Fault::RawPath], vec![]]
        );
    }

    /// A `#[cfg(test)]` module's invocations are not shipped and are
    /// not read; the same text with the gate removed is. This is the
    /// control that the excision reaches this reader at all.
    #[test]
    fn test_test_gated_invocations_are_not_read() {
        let gated = "fn f() { log::warn!(\"a: fine\"); }\n#[cfg(test)]\nmod t { fn g() { log::warn!(\"no prefix\"); } }\n";
        let report = report_for(Path::new("log_posture_gate.rs"), gated);
        assert_eq!(
            (report.invocations, report.offenders.len()),
            (1, 0),
            "{report:#?}"
        );

        let ungated = gated.replace("#[cfg(test)]\n", "");
        let report = report_for(Path::new("log_posture_gate.rs"), &ungated);
        assert_eq!(
            (report.invocations, report.offenders.len()),
            (2, 1),
            "{report:#?}"
        );
    }
}
