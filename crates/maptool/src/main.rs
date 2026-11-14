// SPDX-License-Identifier: MPL-2.0

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails every `./test.sh`
// run; the lint fails only `./test.sh --lint`, a hard gate since
// clippy's baseline reached zero, but one you have to ask for. Two
// mechanisms rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! `maptool` — the command-line tool for `.mandala.json` files, a client
//! of the same [`Document`] the app edits.
//!
//! Every verb reads its map with
//! [`file::read`](baumhard::document::format::file::read), the app's own
//! read: a map the app would refuse, `maptool` refuses with the same
//! words. `show`, `grep`, `export` and `verify` only read the document.
//! `apply` changes it the one way a document changes — an
//! [`EditRequest`](baumhard::document::writer::EditRequest) handed to
//! [`Document::apply`] — and writes it with [`Document::save_json`] and
//! [`file::write`](baumhard::document::format::file::write).
//!
//! Exit status: `0` when the verb did its work, `1` when it could not (a
//! map that does not load, no match, a command that failed, a write that
//! failed), `2` for a command line it cannot run.

use std::io::Write as _;
use std::process::ExitCode;

use baumhard::document::format::file;
use baumhard::document::ids::Document;
use regex::{Regex, RegexBuilder};

mod apply;
mod export;
mod grep;
mod map;
mod show;
mod verify;

/// What `maptool --help` prints, and what follows a usage error.
const USAGE: &str = "\
Usage: maptool <command> <map.mandala.json> [args...]

Every command reads the map the way the app does; a map the app would
refuse is refused with the loader's message, naming its JSON path.

Commands:
  show <map>                    Print the node tree, one line per node,
                                indented by depth: '#<id>', the node's
                                section text (sections separated by ' │ ',
                                a line break shown as '⏎'), and markers
                                '[frame]', '[link → #<id>]' and
                                '[portal → #<id>]'.
  grep <map> <pattern> [-i]     Print every line of text the regex
                                <pattern> matches, in sections, frame
                                rings, connectors and line links (their
                                spans, caps and labels) and portal ends
                                (and their labels), as
                                '#<id> <element>: <line>', the element
                                named by its path below the node
                                ('sections[0]', 'links[1].ends[0]').
                                -i matches case-insensitively. No match
                                exits 1.
  export <map> [out.md]         Write the node tree as Markdown: the first
                                line of each node's section text is a
                                heading as deep as the node, its other
                                lines plain text under it; a node with no
                                text passes its level to its children.
                                To stdout, or to <out.md>.
  verify <map>                  Load the map: a refusal is printed with
                                its JSON path and exits 1. Then print a
                                warning for what loads but is probably a
                                mistake: a verb nothing refers to, a
                                node saved hidden that nothing can
                                reveal, a visible frame with no rings,
                                and a portal whose ends both have no
                                content. Warnings alone exit 0.
  apply <map> <pattern> [-i] [--dry-run] -- <cmd> [args...]
                                For each section with a line the regex
                                <pattern> matches, pipe its text to <cmd>
                                and replace its spans with one span of
                                the command's output (one trailing line
                                break removed), all as one edit. Prints
                                the changed map to stdout; -i writes it
                                back to <map> instead, through a
                                temporary file and a rename; --dry-run
                                prints the change as a unified diff and
                                writes nothing (the commands still run).
                                No matching section exits 1, and so does
                                a command that fails, before anything is
                                written.";

/// Why a run did not succeed: the exit status and the message for it.
#[derive(Debug)]
enum CliError {
    /// The command line cannot be run: exit `2`, with the usage.
    Usage(String),
    /// The verb could not do its work: exit `1`.
    Failed(String),
}

fn main() -> ExitCode {
    // The loader logs what it degrades, as it does in the app; without a
    // logger those lines would go nowhere.
    baumhard::util::log::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    let result = run(&args).and_then(|out| {
        stdout
            .write_all(out.as_bytes())
            .map_err(|e| CliError::Failed(format!("maptool: could not write the output: {e}")))
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprintln!("{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Failed(message)) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

/// Run the command line `args` (without the program name) and return
/// what it prints on stdout. Anything it says on stderr besides an error
/// — `apply`'s count — it prints itself.
fn run(args: &[String]) -> Result<String, CliError> {
    let Some(verb) = args.first() else {
        return Err(CliError::Usage("maptool: missing command".into()));
    };
    let rest = &args[1..];
    match verb.as_str() {
        "show" => {
            let [map] = positional::<1>("show", rest, &[])?.0;
            Ok(show::show(&read(map)?))
        }
        "grep" => {
            let ([map, pattern], flags) = positional::<2>("grep", rest, &["-i"])?;
            let regex = build_regex("grep", pattern, flags.contains(&"-i"))?;
            let (out, hits) = grep::grep(&read(map)?, &regex);
            if hits == 0 {
                return Err(CliError::Failed(format!("grep: no matches for {pattern:?}")));
            }
            Ok(out)
        }
        "export" => export(rest),
        "verify" => {
            let [map] = positional::<1>("verify", rest, &[])?.0;
            let doc = read(map)?;
            let warnings = verify::warnings(&doc);
            let mut out = String::new();
            for warning in &warnings {
                out.push_str("warning: ");
                out.push_str(warning);
                out.push('\n');
            }
            out.push_str(map);
            match warnings.len() {
                0 => out.push_str(": valid\n"),
                n => out.push_str(&format!(": valid, {n} warning(s)\n")),
            }
            Ok(out)
        }
        "apply" => apply(rest),
        "-h" | "--help" | "help" => Ok(format!("{USAGE}\n")),
        other => Err(CliError::Usage(format!("maptool: unknown command: {other}"))),
    }
}

/// Read the map at `path` as the app does.
fn read(path: &str) -> Result<Document, CliError> {
    file::read(path).map_err(CliError::Failed)
}

/// Split `args` into exactly `N` positional arguments and the flags out
/// of `allowed` among them, which may stand anywhere. Any other argument
/// starting with `--`, and too few or too many positional ones, is a
/// usage error naming `verb`; one starting with a single `-` is
/// positional, so a pattern may begin with one.
fn positional<'a, const N: usize>(
    verb: &str,
    args: &'a [String],
    allowed: &[&str],
) -> Result<([&'a str; N], Vec<&'a str>), CliError> {
    let mut flags = Vec::new();
    let mut found = Vec::new();
    for arg in args {
        if allowed.contains(&arg.as_str()) {
            flags.push(arg.as_str());
        } else if arg.starts_with("--") {
            return Err(CliError::Usage(format!("{verb}: unknown flag: {arg}")));
        } else {
            found.push(arg.as_str());
        }
    }
    let count = found.len();
    let found: [&str; N] = found.try_into().map_err(|_| {
        CliError::Usage(format!("{verb}: expected {N} argument(s), found {count}"))
    })?;
    Ok((found, flags))
}

/// Compile `pattern`, naming `verb` in a refusal.
fn build_regex(verb: &str, pattern: &str, case_insensitive: bool) -> Result<Regex, CliError> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| CliError::Usage(format!("{verb}: invalid regex {pattern:?}: {e}")))
}

/// `export <map> [out.md]`.
fn export(args: &[String]) -> Result<String, CliError> {
    let (map, out) = match args {
        [map] => (map, None),
        [map, out] => (map, Some(out)),
        _ => {
            return Err(CliError::Usage(format!(
                "export: expected <map> and an optional <out.md>, found {} argument(s)",
                args.len()
            )))
        }
    };
    let markdown = export::markdown(&read(map)?);
    match out {
        None => Ok(markdown),
        Some(path) => std::fs::write(path, markdown)
            .map(|()| String::new())
            .map_err(|e| {
                CliError::Failed(format!(
                    "export: {}: could not write: {e}",
                    baumhard::util::log::tilde_path(std::path::Path::new(path))
                ))
            }),
    }
}

/// `apply <map> <pattern> [-i] [--dry-run] -- <cmd> [args...]`.
fn apply(args: &[String]) -> Result<String, CliError> {
    let Some(separator) = args.iter().position(|arg| arg == "--") else {
        return Err(CliError::Usage("apply: missing `--` before the command".into()));
    };
    let ([map, pattern], flags) = positional::<2>("apply", &args[..separator], &["-i", "--dry-run"])?;
    let in_place = flags.contains(&"-i");
    let dry_run = flags.contains(&"--dry-run");
    if in_place && dry_run {
        return Err(CliError::Usage(
            "apply: -i writes the map and --dry-run writes nothing: give one".into(),
        ));
    }
    let Some((cmd, cmd_args)) = args[separator + 1..].split_first() else {
        return Err(CliError::Usage("apply: missing the command after `--`".into()));
    };
    let regex = build_regex("apply", pattern, false)?;
    let mut doc = read(map)?;
    let targets = apply::targets(&doc, &regex);
    if targets.is_empty() {
        return Err(CliError::Failed(format!("apply: no section matches {pattern:?}")));
    }
    let matched = targets.len();
    let changed = apply::run_commands(targets, cmd, cmd_args).map_err(CliError::Failed)?;
    let saved = |doc: &mut Document| {
        doc.save_json()
            .map_err(|e| CliError::Failed(format!("apply: the map cannot be saved: {e}")))
    };
    let before = saved(&mut doc)?;
    apply::rewrite(&mut doc, &changed, &format!("maptool apply: {cmd}")).map_err(CliError::Failed)?;
    let after = saved(&mut doc)?;
    eprintln!("apply: {} of {matched} matched section(s) changed", changed.len());
    if dry_run {
        Ok(apply::diff(map, &before, &after))
    } else if in_place {
        if !changed.is_empty() {
            file::write(map, &after).map_err(CliError::Failed)?;
        }
        Ok(String::new())
    } else {
        Ok(after)
    }
}
