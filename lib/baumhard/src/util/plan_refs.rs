// SPDX-License-Identifier: MPL-2.0

//! **No comment in the workspace's Rust cites a work plan by an
//! unnamed section, batch, risk or review-finding number.** CLAUDE.md
//! asks that comments not reference the current task, and `aa73bcd0`
//! applied that to this class once: it stripped 55 such citations
//! from production code. A doc sweep later found 66 more written
//! after it, so the class is held here rather than left to the next
//! sweep.
//!
//! **What is flagged**, in comment prose only (`//`, `///`, `//!` and
//! block comments, read through [`comment_text`], so string literals
//! and code are never read):
//!
//! - the word `Plan` followed by a space and a section sign;
//! - `Batch` followed by a digit, directly or after one space or
//!   hyphen, capitalized and at a word start (after a hyphenated
//!   `pre-` prefix too);
//! - a `B`-letter batch number with a dot (`B6`, a dot, `4`) at a
//!   word start that is not a section sign, so baumhard's own
//!   `§B`-numbered conventions are not caught;
//! - `Risk #` followed by a digit;
//! - a review-finding tag: `review T` or `review TIER` followed by a
//!   digit.
//!
//! Each one names a place in a document the reader is not told the
//! name of, and says nothing about the code once the plan is closed.
//! The fix is to write the behavior the citation stood for.
//!
//! **What is deliberately not held.** A pointer that names its
//! document — `work_plans/LLM_IPC.md` with a `§D5`, or
//! `work_plans/WASM_CONVERGENCE.md` — is the form CLAUDE.md itself
//! uses, and a reader can follow it. Neither is lowercase prose
//! (`batch` as in GPU batching), nor "Pre-fix" history, which
//! `aa73bcd0` left to case-by-case judgment.

use crate::util::rust_source::comment_text;
use crate::util::source_scan::{relative_to_repo, workspace_rust_sources};

/// The one comment allowed to write a flagged form: the regression
/// test pinning that a user-facing border error no longer surfaces a
/// plan citation, whose doc quotes the string it forbids.
const ALLOWED: &[(&str, &str)] = &[("src/application/console/commands/border/tests.rs", "Plan §")];

/// Every flagged citation in `text`'s comments, as `(1-based line,
/// the matched form)`. `text` is a whole Rust file.
fn plan_references_in(text: &str) -> Vec<(usize, &'static str)> {
    let prose = comment_text(text);
    let mut out = Vec::new();
    for (offset, line) in prose.lines().enumerate() {
        for (form, found) in [
            ("Plan §", line.contains("Plan §")),
            ("Batch N", has_batch_number(line)),
            ("Bn.n", has_lettered_batch(line)),
            ("Risk #N", has_digit_after(line, "Risk #")),
            (
                "review TN",
                has_digit_after(line, "review T") || has_digit_after(line, "review TIER"),
            ),
        ] {
            if found {
                out.push((offset + 1, form));
            }
        }
    }
    out
}

/// True when `c` may sit right before a flagged word: nothing, or a
/// character that is not part of a word.
fn starts_word(before: Option<char>) -> bool {
    before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// `needle` immediately followed by an ASCII digit.
fn has_digit_after(line: &str, needle: &str) -> bool {
    line.match_indices(needle)
        .any(|(at, _)| line[at + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// `Batch` at a word start, then a digit, optionally after one space
/// or hyphen.
fn has_batch_number(line: &str) -> bool {
    line.match_indices("Batch").any(|(at, word)| {
        let rest = &line[at + word.len()..];
        let rest = rest.strip_prefix([' ', '-']).unwrap_or(rest);
        starts_word(line[..at].chars().next_back()) && rest.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// `B`, digits, a dot and a digit, at a word start not preceded by a
/// section sign.
fn has_lettered_batch(line: &str) -> bool {
    line.match_indices('B').any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let digits: String = line[at + 1..].chars().take_while(char::is_ascii_digit).collect();
        let rest = &line[at + 1 + digits.len()..];
        before != Some('§')
            && starts_word(before)
            && !digits.is_empty()
            && rest.starts_with('.')
            && rest[1..].starts_with(|c: char| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **No Rust comment in the workspace cites a work plan by an
    /// unnamed section, batch, risk or review-finding number** — the
    /// class `aa73bcd0` stripped and the #52 sweep stripped again.
    /// Write the behavior the citation stood for instead; a pointer
    /// that names its document (`work_plans/X.md`) is not held.
    #[test]
    fn test_no_comment_cites_a_work_plan_by_an_unnamed_number() {
        let mut violations = Vec::new();
        let mut allowed_seen = vec![false; ALLOWED.len()];
        let files = workspace_rust_sources();
        assert!(
            files.len() > 100,
            "the walk found {} Rust file(s); the workspace has far more, so this \
             scan is no longer reading what it claims to check",
            files.len()
        );
        for file in files {
            let text = std::fs::read_to_string(&file).expect("workspace source is readable");
            let relative = relative_to_repo(&file);
            for (line, form) in plan_references_in(&text) {
                match ALLOWED.iter().position(|&(f, a)| f == relative && a == form) {
                    Some(at) => allowed_seen[at] = true,
                    None => violations.push(format!("{relative}:{line}: {form}")),
                }
            }
        }
        assert!(
            violations.is_empty(),
            "{} comment(s) cite a work plan by an unnamed number. Describe the \
             behavior instead:\n  {}",
            violations.len(),
            violations.join("\n  ")
        );
        for (at, seen) in allowed_seen.iter().enumerate() {
            assert!(
                seen,
                "allowlist entry {:?} matches nothing any more; delete it",
                ALLOWED[at]
            );
        }
    }

    /// Negative control: every flagged form, written as a comment,
    /// is found, on its own line.
    #[test]
    fn test_plan_reference_scan_flags_every_form() {
        let source = "/// Wired in Batch 3.\n\
                      // pre-Batch-2 gate\n\
                      //! shipped in B6.4\n\
                      /* Risk #1 */\n\
                      // opus review T2 pin\n\
                      // review TIER2\n\
                      fn x() {} // Plan §4.5\n";
        assert_eq!(
            plan_references_in(source),
            vec![
                (1, "Batch N"),
                (2, "Batch N"),
                (3, "Bn.n"),
                (4, "Risk #N"),
                (5, "review TN"),
                (6, "review TN"),
                (7, "Plan §"),
            ]
        );
    }

    /// Control: what reads like a citation but is not one — lowercase
    /// prose, a named document pointer, a baumhard `§B` convention, a
    /// word that merely contains the letters, and a string literal —
    /// is left alone.
    #[test]
    fn test_plan_reference_scan_leaves_prose_and_literals_alone() {
        let source = "// GPU batch 3 draws in one pass\n\
                      // see `work_plans/LLM_IPC.md` §D5\n\
                      /// per baumhard CONVENTIONS §B3.1\n\
                      // the Batches table; a SUB2.1 id\n\
                      let s = \"Plan §4.5 and Batch 3\";\n";
        assert_eq!(plan_references_in(source), Vec::new());
    }
}
