// SPDX-License-Identifier: MPL-2.0

//! The `cargo` commands the reference docs publish, held against the
//! workspace they claim to drive.
//!
//! A command written into a doc is a claim — *run this and it does
//! what the sentence around it says* — and nothing here checked
//! those claims. One had been false since it was written:
//! `CLAUDE.md`'s "Common tasks" and `TEST_CONVENTIONS.md` §T11 both
//! told a reader to narrow a mandala test run with `-p mandala`
//! plus `--lib`, which exits 101 with `no library targets found in
//! package 'mandala'` — that member is a binary crate. The flag is
//! correct on its neighbor, since baumhard *is* a library, and had
//! been copied across (#148).
//!
//! That defect is mechanical, so it is checked mechanically. Two
//! questions the real manifests can answer:
//!
//! - every `-p <name>` in an audited section names a workspace
//!   member;
//! - every `--lib` sits on a member that actually has a library
//!   target.
//!
//! **What this reads.** Every way those two sections can publish a
//! command: an inline code span opened with any number of backticks,
//! and every line of a fenced block, whether or not the fence
//! carries a language tag. The tagged fence is called out because it
//! was the one gap that made the reader *inconsistent* rather than
//! narrow — the tag glued itself to the front of the command and the
//! `cargo` filter then dropped it, so a fence opened with a `bash`
//! tag hid a command that the same fence untagged did not.
//!
//! **What this does not reach**, said plainly so the silence is not
//! read as coverage. It runs nothing: that a documented command is
//! well-formed against the manifests is a weaker claim than that it
//! succeeds, and the strong one belongs to a person at a terminal.
//! It judges only spans carrying an explicit `-p` / `--package`,
//! because a command without one selects whatever package the
//! working directory holds and a code span does not carry a working
//! directory. It reads a command that is *marked up* as one: a
//! `cargo` invocation typed into running prose with no backticks
//! around it is prose to this reader, as is one hidden inside a
//! quoted string. Inside a fence it takes one line as one command,
//! so a shell continuation (`\` at end of line) reads as two. And
//! it reads two named sections rather than every Markdown file, for
//! the reason [`crate::util::doc_fixtures`] exists at all: a
//! whole-file scan stays green after the thing it claims to pin has
//! moved somewhere else entirely.
//!
//! Test-only and native-only, for the same reasons
//! `crate::util::manifests` — whose member list this reuses rather
//! than restating — is.

use crate::util::doc_fixtures::{repo_path, section_text};
use crate::util::manifests::member_manifests;
use std::collections::BTreeMap;
use std::path::Path;

/// The doc sections that publish runnable commands, as
/// `(repo-relative file, exact heading line)`.
///
/// Adding a section here is how a new set of published commands
/// joins the checked set. [`section_text`] panics when a heading is
/// gone, so a renamed section stops the run rather than quietly
/// removing itself.
pub(crate) const AUDITED_SECTIONS: [(&str, &str); 2] = [
    ("CLAUDE.md", "## Common tasks"),
    ("TEST_CONVENTIONS.md", "## §T11 Running the suite"),
];

/// One `cargo` invocation quoted in an audited doc section.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DocCommand {
    /// Repo-relative file the span was read from, for the message.
    pub(crate) doc: String,
    /// The span, whitespace-normalized — code spans wrap across
    /// source lines and a reader should see one line.
    pub(crate) command: String,
    /// The argument of `-p` / `--package`, when the span carries
    /// one. `None` means the selection depends on the working
    /// directory, which this module deliberately does not guess.
    pub(crate) package: Option<String>,
    /// Whether cargo's own arguments — those before a bare `--` —
    /// include `--lib`.
    pub(crate) wants_lib: bool,
}

/// Every `cargo` command quoted in [`AUDITED_SECTIONS`], parsed.
///
/// Cost: one read of each audited doc plus a linear scan of the
/// named section. Paid once per test that asks.
pub(crate) fn documented_cargo_commands() -> Vec<DocCommand> {
    let mut out = Vec::new();
    for (doc, heading) in AUDITED_SECTIONS {
        for command in cargo_spans(&section_text(&repo_path(doc), heading), doc) {
            let package = package_selected(&command, doc);
            let wants_lib = cargo_arguments(&command).any(|token| token == "--lib");
            out.push(DocCommand {
                doc: doc.to_string(),
                command,
                package,
                wants_lib,
            });
        }
    }
    out
}

/// The code in `text` that holds a `cargo` invocation,
/// whitespace-normalized: every inline code span, plus every line of
/// every fenced block, in document order.
///
/// A fenced block is taken line by line and its opening marker —
/// info string and all — is dropped. Reading the fence body as prose
/// instead is what used to make a language-tagged fence invisible:
/// the tag ended up glued to the front of the command, and the
/// `cargo` filter below dropped the result. A doc that publishes a
/// command in a fence is publishing a command, and a tag is a
/// syntax-highlighting hint, not a claim that the line is unrunnable.
///
/// An opening fence that is never closed, and a code span whose
/// backtick run is never matched by a run of the same length, both
/// panic naming `doc`: either one swallows the rest of the section
/// into a single span, turning a section full of commands into a
/// section holding none. That posture is `crate::util::manifests`':
/// a shape the reader cannot handle stops the run instead of
/// deleting itself from the checked set.
pub(crate) fn cargo_spans(text: &str, doc: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut prose = String::new();
    let mut fence: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        match &fence {
            Some(marker) => {
                if is_fence_close(trimmed, marker) {
                    fence = None;
                } else {
                    spans.push(normalize(line));
                }
            }
            None if trimmed.starts_with("```") => {
                spans.extend(inline_spans(&prose, doc));
                prose.clear();
                fence = Some(trimmed.chars().take_while(|c| *c == '`').collect());
            }
            None => {
                prose.push('\n');
                prose.push_str(line);
            }
        }
    }
    assert!(
        fence.is_none(),
        "{doc}: a fenced code block is opened and never closed, so everything below \
         it reads as code and this reader can no longer tell a published command \
         from prose"
    );
    spans.extend(inline_spans(&prose, doc));
    spans.retain(|span| span == "cargo" || span.starts_with("cargo "));
    spans
}

/// Whether `trimmed` closes a fence opened with `marker` — a run of
/// at least as many backticks as opened it, and nothing else on the
/// line, which is what CommonMark asks of a closing fence.
fn is_fence_close(trimmed: &str, marker: &str) -> bool {
    let run = trimmed.chars().take_while(|c| *c == '`').count();
    run >= marker.len() && trimmed.trim_end().len() == run
}

/// `text` with every run of whitespace — line breaks included —
/// collapsed to one space. A code span wraps across source lines and
/// a reader should see one line.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The inline code spans in `text`, normalized, in document order.
///
/// A span opens on a run of backticks and closes on the next run of
/// exactly the same length, so the two-backtick form Markdown offers
/// for a command that itself contains a backtick is read rather than
/// silently skipped. An opener with no matching closer panics: it is
/// far more often a typo that hid a command than a literal backtick
/// somebody meant.
fn inline_spans(text: &str, doc: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let open = backtick_run(bytes, at);
        let Some(close) = closing_run(bytes, at + open, open) else {
            panic!(
                "{doc}: a code span opened with {open} backtick(s) is never closed by a run \
                 of {open} — its backticks do not pair up, so this reader would swallow the \
                 rest of the section into one span. It opens at: {:?}",
                normalize(&text[at..bytes.len().min(at + 60)])
            );
        };
        out.push(normalize(&text[at + open..close]));
        at = close + open;
    }
    out
}

/// The length of the run of backticks starting at `at`.
fn backtick_run(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|byte| **byte == b'`').count()
}

/// The byte offset of the next run of exactly `open` backticks at or
/// after `from`, or `None` when the text holds no such run.
fn closing_run(bytes: &[u8], from: usize, open: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let run = backtick_run(bytes, at);
        if run == open {
            return Some(at);
        }
        at += run;
    }
    None
}

/// Cargo's own arguments in `command` — every token up to a bare
/// `--`, which hands the rest to the built binary or the invoked
/// tool. `cargo run -- --lib` passes `--lib` to mandala; it does not
/// ask cargo for a library target.
pub(crate) fn cargo_arguments(command: &str) -> impl Iterator<Item = &str> {
    command.split(' ').take_while(|token| *token != "--")
}

/// The package `command` selects with `-p` / `--package`, or `None`
/// when it names none.
///
/// A selection flag with nothing after it panics naming `doc`: the
/// span is not a command anyone can run, which is the class of
/// defect this module exists for rather than an input to tolerate.
pub(crate) fn package_selected(command: &str, doc: &str) -> Option<String> {
    let mut tokens = cargo_arguments(command);
    while let Some(token) = tokens.next() {
        if let Some(name) = token.strip_prefix("--package=") {
            return Some(name.to_string());
        }
        if token == "-p" || token == "--package" {
            return Some(
                tokens
                    .next()
                    .unwrap_or_else(|| panic!("{doc}: `{command}` ends on {token} with no package after it"))
                    .to_string(),
            );
        }
    }
    None
}

/// Every workspace member, mapped to whether it has a library
/// target.
///
/// Read from the manifests [`member_manifests`] resolves out of
/// `[workspace] members`, so a fifth crate joins the checked set the
/// day it is added. A member has a library target when its manifest
/// declares `[lib]` or cargo would auto-discover `src/lib.rs` beside
/// it — the two ways one comes to exist.
///
/// Cost: one read of each member manifest plus one `stat` per
/// member.
pub(crate) fn library_targets() -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for manifest in member_manifests() {
        let text = std::fs::read_to_string(repo_path(&manifest))
            .unwrap_or_else(|e| panic!("{manifest} must be readable: {e}"));
        out.insert(package_name(&text, &manifest), has_lib_target(&manifest, &text));
    }
    out
}

/// The `[package] name` of a manifest, section-scoped.
///
/// Section-scoped because `name` is not unique in a manifest:
/// `lib/baumhard/Cargo.toml` carries a second one under `[[bench]]`,
/// and a whole-file search for the first `name = ` would answer
/// correctly there only by accident of ordering.
fn package_name(text: &str, manifest: &str) -> String {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("name") else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };
        let name = value.trim().trim_matches('"').trim();
        assert!(
            !name.is_empty() && name != "true",
            "{manifest}: `[package] name` reads {name:?} — an inherited or empty name \
             is a shape this reader cannot resolve to a package"
        );
        return name.to_string();
    }
    panic!("{manifest}: no `[package] name` — the doc scan cannot name what it checks")
}

/// Whether the member at `manifest` has a library target: a declared
/// `[lib]` table, or a `src/lib.rs` cargo would auto-discover.
fn has_lib_target(manifest: &str, text: &str) -> bool {
    if text.lines().any(|line| line.trim() == "[lib]") {
        return true;
    }
    let dir = Path::new(manifest).parent().unwrap_or_else(|| Path::new(""));
    repo_path(&dir.to_string_lossy())
        .join("src")
        .join("lib.rs")
        .is_file()
}

/// Every documented command that asks cargo for a library target of
/// a package that has none, rendered `"<doc>: <command>"`.
///
/// A `--lib` on a package outside `targets` is *not* reported here —
/// [`unknown_package_selections`] owns that failure, and reporting
/// it twice would say one defect in two voices.
pub(crate) fn lib_flag_violations(commands: &[DocCommand], targets: &BTreeMap<String, bool>) -> Vec<String> {
    commands
        .iter()
        .filter(|command| command.wants_lib)
        .filter(|command| {
            command
                .package
                .as_ref()
                .and_then(|name| targets.get(name))
                .is_some_and(|has_lib| !has_lib)
        })
        .map(|command| format!("{}: `{}`", command.doc, command.command))
        .collect()
}

/// Every documented `-p <name>` naming something that is not a
/// workspace member, rendered `"<doc>: <command>"`.
pub(crate) fn unknown_package_selections(
    commands: &[DocCommand],
    targets: &BTreeMap<String, bool>,
) -> Vec<String> {
    commands
        .iter()
        .filter(|command| {
            command
                .package
                .as_ref()
                .is_some_and(|name| !targets.contains_key(name))
        })
        .map(|command| format!("{}: `{}`", command.doc, command.command))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a command the way [`documented_cargo_commands`] does,
    /// so a control exercises the same parse the repo test does
    /// rather than a hand-set struct that cannot disagree with it.
    fn parse(span: &str) -> DocCommand {
        let spans = cargo_spans(&format!("prose `{span}` prose"), "fixture.md");
        assert_eq!(spans.len(), 1, "the fixture must yield exactly one span");
        let command = spans.into_iter().next().unwrap_or_default();
        let package = package_selected(&command, "fixture.md");
        let wants_lib = cargo_arguments(&command).any(|token| token == "--lib");
        DocCommand {
            doc: "fixture.md".to_string(),
            command,
            package,
            wants_lib,
        }
    }

    fn workspace_fixture() -> BTreeMap<String, bool> {
        BTreeMap::from([("alib".to_string(), true), ("abin".to_string(), false)])
    }

    /// **No documented command asks for a library target of a
    /// package that has none** — #148's defect, which lived in two
    /// documents at once because the flag was copied off a neighbor
    /// that legitimately carries it.
    ///
    /// The fix for a failure is to drop `--lib`: on a binary crate
    /// it narrows nothing, it exits 101 before a single test runs.
    #[test]
    fn test_no_documented_command_targets_a_lib_the_package_lacks() {
        let commands = documented_cargo_commands();
        let targets = library_targets();
        assert_scan_is_looking(&commands, &targets);
        let violations = lib_flag_violations(&commands, &targets);
        assert!(
            violations.is_empty(),
            "{} documented command(s) ask cargo for a library target of a package \
             that has none. Drop `--lib` — on a binary crate it does not narrow the \
             run, it fails it:\n  {}",
            violations.len(),
            violations.join("\n  ")
        );
    }

    /// **Every documented `-p <name>` names a workspace member.**
    /// The sibling failure to the one above and the reason
    /// [`lib_flag_violations`] stays quiet about an unknown package:
    /// a doc that outlived a crate rename is a different repair than
    /// a doc that asked for the wrong target.
    #[test]
    fn test_every_documented_package_selection_names_a_workspace_member() {
        let commands = documented_cargo_commands();
        let targets = library_targets();
        assert_scan_is_looking(&commands, &targets);
        let unknown = unknown_package_selections(&commands, &targets);
        assert!(
            unknown.is_empty(),
            "{} documented command(s) select a package this workspace does not have. \
             The members are {:?}:\n  {}",
            unknown.len(),
            targets.keys().collect::<Vec<_>>(),
            unknown.join("\n  ")
        );
    }

    /// The preconditions both repo tests rest on. Each one is a way
    /// the checks above could report "nothing is wrong" while having
    /// stopped looking — a section rewritten to quote no commands,
    /// a member reader that answers the same way for everyone, a
    /// `--lib` that no longer appears anywhere to be judged.
    fn assert_scan_is_looking(commands: &[DocCommand], targets: &BTreeMap<String, bool>) {
        for (doc, heading) in AUDITED_SECTIONS {
            let found = commands.iter().filter(|command| command.doc == doc).count();
            assert!(
                found >= 3,
                "{doc}'s {heading:?} yields {found} cargo command(s); it published \
                 several, so the scan is no longer reading what it claims to check"
            );
        }
        assert!(
            commands.iter().any(|command| command.wants_lib),
            "no documented command carries `--lib` any more, so the check that a \
             `--lib` names a library target has nothing left to judge"
        );
        assert!(
            targets.values().any(|has_lib| *has_lib) && targets.values().any(|has_lib| !has_lib),
            "the member reader classified every one of {} member(s) the same way, so \
             it cannot tell a library from a binary: {targets:?}",
            targets.len()
        );
    }

    /// Positive control: the judgment fires on a binary crate and
    /// stays quiet on a library one. Without this, a
    /// [`lib_flag_violations`] that returned an empty vector
    /// unconditionally would read exactly like a clean repository.
    #[test]
    fn test_a_lib_flag_on_a_binary_crate_is_reported() {
        let targets = workspace_fixture();
        let offending = vec![parse("cargo test -p abin --lib pattern")];
        assert_eq!(
            lib_flag_violations(&offending, &targets),
            vec!["fixture.md: `cargo test -p abin --lib pattern`"]
        );
        let fine = vec![
            parse("cargo test -p alib --lib pattern"),
            parse("cargo test -p abin pattern"),
        ];
        assert!(lib_flag_violations(&fine, &targets).is_empty());
    }

    /// The control that disables the mechanism on the exercised
    /// path: truncating at the bare `--` is what separates cargo's
    /// arguments from the invoked program's. Read the whole span and
    /// `cargo run -p abin -- --lib` becomes a false report against a
    /// crate that is behaving correctly.
    #[test]
    fn test_a_flag_after_the_argument_separator_is_not_cargos() {
        let after_separator = parse("cargo run -p abin -- --lib map.json");
        assert!(!after_separator.wants_lib);
        assert!(lib_flag_violations(&[after_separator], &workspace_fixture()).is_empty());
        assert!(parse("cargo test -p abin --lib").wants_lib);
    }

    /// A package selection is read from cargo's flags in either
    /// spelling, and only from them.
    #[test]
    fn test_package_selection_is_read_in_every_spelling_cargo_accepts() {
        assert_eq!(parse("cargo test -p alib").package.as_deref(), Some("alib"));
        assert_eq!(
            parse("cargo test --package alib").package.as_deref(),
            Some("alib")
        );
        assert_eq!(
            parse("cargo test --package=alib").package.as_deref(),
            Some("alib")
        );
        assert_eq!(parse("cargo test --workspace").package, None);
        assert_eq!(parse("cargo run -- -p alib").package, None);
    }

    /// Only `cargo` spans are collected, and a span that wraps
    /// across source lines — which the audited sections do write —
    /// is normalized to one line rather than dropped.
    #[test]
    fn test_the_span_reader_collects_cargo_invocations_and_nothing_else() {
        let text = "run `./test.sh --lint`, or `cargo test\n  -p alib --lib name`, not `--lib`";
        assert_eq!(
            cargo_spans(text, "fixture.md"),
            vec!["cargo test -p alib --lib name".to_string()]
        );
    }

    /// A backtick run with no matching run stops the scan. The rest
    /// of the section would otherwise read as one enormous span,
    /// turning a section full of commands into a section holding
    /// none — the silent shape this module refuses.
    #[test]
    #[should_panic(expected = "do not pair up")]
    fn test_an_unmatched_backtick_run_stops_the_scan() {
        cargo_spans("a `cargo test -p alib` and a stray ` tick", "fixture.md");
    }

    /// A command published in a fenced block is published, tag or no
    /// tag. The tagged fence is the case that used to disappear: the
    /// info string was read as the first word of the command, and
    /// the `cargo` filter dropped what came out.
    #[test]
    fn test_a_fenced_block_publishes_commands_tag_or_no_tag() {
        let tagged = "prose\n\n```bash\ncargo test -p abin --lib pattern\n```\n\nmore";
        let untagged = "prose\n\n```\ncargo test -p abin --lib pattern\n```\n\nmore";
        let expected = vec!["cargo test -p abin --lib pattern".to_string()];
        assert_eq!(cargo_spans(tagged, "fixture.md"), expected);
        assert_eq!(cargo_spans(untagged, "fixture.md"), expected);
        assert_eq!(
            cargo_spans(tagged, "fixture.md"),
            cargo_spans(untagged, "fixture.md"),
            "a fence's language tag is a highlighting hint; it cannot decide whether the \
             command inside is checked"
        );
    }

    /// Spans and fences are read in document order, and a fence body
    /// is read as code rather than as prose to scan for backticks —
    /// so a `#` comment or an unbalanced tick inside one cannot
    /// derail the reader.
    #[test]
    fn test_fenced_and_inline_commands_are_read_in_document_order() {
        let text = "`cargo test -p alib` then\n\n```sh\n# a lone ` tick in a comment\ncargo build -p abin\n```\n\nand `cargo doc -p alib`";
        assert_eq!(
            cargo_spans(text, "fixture.md"),
            vec![
                "cargo test -p alib".to_string(),
                "cargo build -p abin".to_string(),
                "cargo doc -p alib".to_string(),
            ]
        );
    }

    /// The two-backtick span Markdown offers for a command that
    /// itself contains a backtick is a span like any other. Read as
    /// a split on single ticks it yielded nothing at all.
    #[test]
    fn test_a_span_delimited_by_two_backticks_is_read() {
        assert_eq!(
            cargo_spans("write ``cargo test -p abin --lib `x` `` here", "fixture.md"),
            vec!["cargo test -p abin --lib `x`".to_string()]
        );
    }

    /// An unclosed fence stops the run: everything below it reads as
    /// code, so the reader can no longer tell a published command
    /// from the prose around it.
    #[test]
    #[should_panic(expected = "opened and never closed")]
    fn test_an_unclosed_fence_stops_the_scan() {
        cargo_spans("prose\n\n```bash\ncargo test -p alib\n\nmore prose", "fixture.md");
    }

    /// The manifests answer both ways for the real workspace, named
    /// member by member: the reader that says "library" for
    /// everything and the reader that says "binary" for everything
    /// both pass every other assertion here.
    #[test]
    fn test_the_member_reader_tells_this_workspace_libraries_from_binaries() {
        let targets = library_targets();
        assert_eq!(targets.get("baumhard"), Some(&true), "baumhard is a library");
        assert_eq!(
            targets.get("mandala_derive"),
            Some(&true),
            "mandala_derive declares `[lib]`"
        );
        assert_eq!(targets.get("mandala"), Some(&false), "mandala is a binary crate");
        assert_eq!(targets.get("maptool"), Some(&false), "maptool is a binary crate");
    }
}
