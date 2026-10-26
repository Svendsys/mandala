// SPDX-License-Identifier: MPL-2.0

//! **No comment in the workspace's Rust cites a work plan by an
//! unnamed section, rule, batch, risk or review-finding number.**
//! CLAUDE.md asks that comments not reference the current task, and
//! `aa73bcd0` applied that to this class once: it stripped 55 such
//! citations from production code. A doc sweep (#52) later found 66
//! more written after it, so the class is held here rather than left
//! to the next sweep.
//!
//! **What is read.** Comment prose only (`//`, `///`, `//!` and block
//! comments, through [`comment_text`]), so string literals and code
//! are never read. Consecutive comment lines are joined into one run,
//! on a single space, before anything is matched: a citation an
//! author wrapped between its word and its number is the same
//! citation, and hand-reflowed comments wrap exactly there. A hit is
//! reported at the line its keyword starts on.
//!
//! **What is flagged** in a run:
//!
//! - the word `plan` (any case, possessive or not) followed by a
//!   section sign;
//! - `plan rule` (any case) followed by a number;
//! - `Risk #` (any case) followed by a number;
//! - a review-finding tag, `review` then `T` or `TIER` and a number,
//!   in any case;
//! - capitalized `Batch` followed by a number, directly or after one
//!   space or hyphen, where it is mid-sentence or carries a `pre-`
//!   prefix. A sentence that merely starts with the word is prose
//!   about batches, so the word at the start of a run, or after a
//!   `.`, `!`, `?` or `:`, is left alone.
//!
//! Each one names a place in a document the reader is not told the
//! name of, and says nothing about the code once the plan is closed.
//! The fix is to write the behavior the citation stood for.
//!
//! **What is deliberately not held.** A pointer that names its
//! document (`work_plans/LLM_IPC.md` and a `§D5`, or
//! `work_plans/WASM_CONVERGENCE.md`) is the form CLAUDE.md itself
//! uses, and a reader can follow it. Neither is lowercase prose
//! (`batch` as in GPU batching), nor the "Pre-fix" history `aa73bcd0`
//! left to case-by-case judgment. Letter-and-dot batch numbers (`B`,
//! a digit, a dot, a digit) are not held either: no rule for them
//! told a plan batch from a second baumhard convention in a `§B`
//! citation list or a version string, and a hard gate's false
//! positive trains people to allowlist.

use crate::util::rust_source::comment_text;
use crate::util::source_scan::{relative_to_repo, workspace_rust_sources};

/// The comments allowed to write a flagged form, each anchored to
/// the text of the one site it exempts: `(file, needle)`, where a
/// hit is allowed only when it starts inside that needle. The one
/// entry is the regression test pinning that a user-facing border
/// error no longer surfaces a plan citation, whose doc quotes the
/// string it forbids.
const ALLOWED: &[(&str, &str)] = &[(
    "src/application/console/commands/border/tests.rs",
    "no longer surfaces \"Plan §5.4 #3\"",
)];

/// One flagged citation: the 1-based line its keyword starts on, the
/// form it matched, and which [`ALLOWED`] needle (of the ones passed
/// in) it lies inside, if any.
#[derive(Debug, PartialEq)]
struct Hit {
    line: usize,
    form: &'static str,
    allowed_by: Option<usize>,
}

/// Every flagged citation in `text`'s comments. `text` is a whole
/// Rust file; `allowed` are the needles that exempt a hit starting
/// inside them.
fn plan_references_in(text: &str, allowed: &[&str]) -> Vec<Hit> {
    let prose = comment_text(text);
    let mut hits = Vec::new();
    let mut run = String::new();
    // Byte offset in `run` where each joined line starts, and its line.
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for (offset, line) in prose.lines().chain(std::iter::once("")).enumerate() {
        let line = line.trim();
        if line.is_empty() {
            hits.extend(hits_in_run(&run, &starts, allowed));
            run.clear();
            starts.clear();
            continue;
        }
        if !run.is_empty() {
            run.push(' ');
        }
        starts.push((run.len(), offset + 1));
        run.push_str(line);
    }
    hits
}

/// The flagged citations in one joined run of comment lines.
fn hits_in_run(run: &str, starts: &[(usize, usize)], allowed: &[&str]) -> Vec<Hit> {
    let lower = run.to_lowercase();
    // `to_lowercase` can change byte lengths outside ASCII; only an
    // ASCII-identical run lets the lowercase offsets index `run`.
    let lower = if lower.len() == run.len() {
        lower
    } else {
        run.to_ascii_lowercase()
    };
    let mut found: Vec<(usize, &'static str)> = Vec::new();
    for needle in ["plan §", "plan's §"] {
        found.extend(word_starts(&lower, needle).map(|at| (at, "plan §")));
    }
    for (needle, form) in [
        ("plan rule ", "plan rule N"),
        ("risk #", "Risk #N"),
        ("review t", "review TN"),
        ("review tier", "review TN"),
    ] {
        found.extend(
            word_starts(&lower, needle)
                .filter(|&at| lower[at + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
                .map(|at| (at, form)),
        );
    }
    found.extend(batch_citations(run).map(|at| (at, "Batch N")));
    found.sort();
    found
        .into_iter()
        .map(|(at, form)| Hit {
            line: starts
                .iter()
                .rev()
                .find(|(start, _)| *start <= at)
                .map_or(0, |(_, line)| *line),
            form,
            allowed_by: allowed.iter().position(|needle| {
                run.match_indices(needle)
                    .any(|(from, n)| (from..from + n.len()).contains(&at))
            }),
        })
        .collect()
}

/// Every offset where `needle` starts at a word start in `text`.
fn word_starts<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.match_indices(needle).map(|(at, _)| at).filter(|&at| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

/// Every offset of a capitalized `Batch` + number that is a citation:
/// mid-sentence, or behind a `pre-` prefix.
fn batch_citations(run: &str) -> impl Iterator<Item = usize> + '_ {
    word_starts(run, "Batch").filter(move |&at| {
        let rest = &run[at + "Batch".len()..];
        let rest = rest.strip_prefix([' ', '-']).unwrap_or(rest);
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            return false;
        }
        let before = &run[..at];
        let prefixed = before.to_ascii_lowercase().ends_with("pre-");
        let sentence_start = before
            .trim_end()
            .chars()
            .next_back()
            .is_none_or(|c| matches!(c, '.' | '!' | '?' | ':'));
        prefixed || !sentence_start
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(source: &str) -> Vec<(usize, &'static str)> {
        plan_references_in(source, &[])
            .into_iter()
            .map(|h| (h.line, h.form))
            .collect()
    }

    /// **No Rust comment in the workspace cites a work plan by an
    /// unnamed section, rule, batch, risk or review-finding number**,
    /// the class `aa73bcd0` stripped and the #52 sweep stripped again.
    /// Write the behavior the citation stood for instead; a pointer
    /// that names its document (`work_plans/X.md`) is not held.
    #[test]
    fn test_no_comment_cites_a_work_plan_by_an_unnamed_number() {
        let mut violations = Vec::new();
        let mut allowed_seen = vec![0usize; ALLOWED.len()];
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
            let needles: Vec<&str> = ALLOWED
                .iter()
                .filter(|(f, _)| *f == relative)
                .map(|(_, n)| *n)
                .collect();
            for hit in plan_references_in(&text, &needles) {
                match hit.allowed_by {
                    Some(at) => {
                        let entry = ALLOWED
                            .iter()
                            .position(|(f, n)| *f == relative && *n == needles[at]);
                        allowed_seen[entry.expect("needle came from ALLOWED")] += 1;
                    }
                    None => violations.push(format!("{relative}:{}: {}", hit.line, hit.form)),
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
        for (entry, seen) in ALLOWED.iter().zip(&allowed_seen) {
            assert_eq!(
                *seen, 1,
                "allowlist entry {entry:?} should exempt exactly one citation; it \
                 exempted {seen}. Delete a stale entry; never widen one"
            );
        }
    }

    /// Negative control: every flagged form, written as a comment, is
    /// found, at the line its keyword starts on — including each form
    /// wrapped between its word and its number, the way a hand reflow
    /// leaves it, and the forms the #52 sweep's own first round
    /// wrote back in.
    #[test]
    fn test_plan_reference_scan_flags_every_form() {
        let source = "/// Wired in Batch 3.\n\
                      \n\
                      // the pre-Batch-2 gate\n\
                      \n\
                      /* Risk #1 */\n\
                      \n\
                      // opus review T2 pin\n\
                      \n\
                      // Review TIER2\n\
                      \n\
                      fn x() {} // Plan §4.5\n\
                      \n\
                      // Per rule 4 of the plan's §9.1.3, fan out\n\
                      \n\
                      // Plan rule 3: auto-resolve\n\
                      \n\
                      // Wired in Batch\n\
                      // 3 of the overhaul.\n\
                      \n\
                      /// see Plan\n\
                      /// §4.5 for why\n\
                      \n\
                      // the Risk\n\
                      // #1 case\n";
        assert_eq!(
            forms(source),
            vec![
                (1, "Batch N"),
                (3, "Batch N"),
                (5, "Risk #N"),
                (7, "review TN"),
                (9, "review TN"),
                (11, "plan §"),
                (13, "plan §"),
                (15, "plan rule N"),
                (17, "Batch N"),
                (20, "plan §"),
                (23, "Risk #N"),
            ]
        );
    }

    /// Control: what reads like a citation but is not one is left
    /// alone — lowercase prose, a sentence that starts with "Batch",
    /// a named document pointer, baumhard `§B` convention lists, a
    /// version string, a word that merely contains the letters, the
    /// other uses of "plan" in this tree, and a string literal.
    #[test]
    fn test_plan_reference_scan_leaves_prose_and_literals_alone() {
        let source = "// GPU batch 3 draws in one pass\n\
                      \n\
                      // Batch 2 of the upload flushes first\n\
                      \n\
                      // It flushes. Batch 3 follows.\n\
                      \n\
                      // see `work_plans/LLM_IPC.md` §D5\n\
                      \n\
                      /// per baumhard CONVENTIONS §B3.1\n\
                      \n\
                      /// per CONVENTIONS §B3/B4.2\n\
                      \n\
                      /// see §B3, B4.1 and §B7\n\
                      \n\
                      /// Version B2.0 of the font table\n\
                      \n\
                      // the Batches table; a SUB2.1 id\n\
                      \n\
                      // the capture plan's camera; the plan settled on 3\n\
                      let s = \"Plan §4.5 and Batch 3 and Risk #1\";\n";
        assert_eq!(forms(source), Vec::new());
    }

    /// An allowlist needle exempts the one citation it contains and
    /// nothing else in the same file.
    #[test]
    fn test_plan_reference_allowlist_exempts_only_its_own_site() {
        let source = "/// error message no longer surfaces \"Plan §5.4 #3\"\n\
                      \n\
                      // Per Plan §5.4 rule 2.\n";
        let hits = plan_references_in(source, &["no longer surfaces \"Plan §5.4 #3\""]);
        assert_eq!(
            hits,
            vec![
                Hit {
                    line: 1,
                    form: "plan §",
                    allowed_by: Some(0)
                },
                Hit {
                    line: 3,
                    form: "plan §",
                    allowed_by: None
                },
            ]
        );
    }
}
