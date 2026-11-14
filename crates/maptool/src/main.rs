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
//! [`file::read`], the app's own
//! read: a map the app would refuse, `maptool` refuses with the same
//! words. `show`, `grep`, `export` and `verify` only read the document.
//! `apply` changes it the one way a document changes — an
//! [`EditRequest`](baumhard::document::writer::EditRequest) handed to
//! [`Document::apply`] — and writes it with [`Document::save_json`] and
//! [`file::write`].
//!
//! Exit status: `0` when the verb did its work, `1` when it could not (a
//! map that does not load, no match, a command that failed, a write that
//! failed), `2` for a command line it cannot run.

use std::io::Write as _;
use std::process::ExitCode;

use std::path::Path;

use baumhard::document::format::file;
use baumhard::document::ids::Document;
use baumhard::util::guarded_file::{publish, real_file, shown_path, PublishRefusal};
use baumhard::util::log::{collapse_home_in, tilde_path};
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
                                <pattern> matches, as
                                '#<id> <element>: <line>'. Searched:
                                every string the map draws as glyphs —
                                the spans of sections, frame rings,
                                connectors and line links (and their
                                caps and labels) and portal ends (and
                                their labels); the 'content' a
                                connector, line link or portal end sets
                                of its own; a 'once' fit's 'filler'; a
                                frame's or ring's 'sides' ornament; and
                                a ring's 'vertical' and 'corners'. Not
                                searched: a span's 'url' link, and what
                                the map's defaults set. The element is
                                named by its path below the node as the
                                saved map spells it ('sections[0]',
                                'links[1].ends[0]', 'connector.content',
                                'frame.rings[0].corners[1]'). An empty
                                section is one empty line. -i matches
                                case-insensitively. No match exits 1.
  export <map> [out.md]         Write the node tree as Markdown: the first
                                line of each node's section text is a
                                heading as deep as the node, its other
                                lines plain text under it; a node with no
                                text passes its level to its children.
                                To stdout, or to <out.md>, which may not
                                be a map ('.mandala.json'), and is
                                written through a temporary file and a
                                rename; a symlink is written through to
                                its file.
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
                                <pattern> matches (an empty section is
                                one empty line), pipe its text to <cmd>
                                and replace its spans with one span of
                                the command's output in the first span's
                                style (not its link), all as one edit.
                                One trailing line break is removed from
                                the output only when the text did not
                                end in one, so 'cat' changes nothing.
                                Here -i is the write flag: to match
                                case-insensitively, start the pattern
                                with (?i). Prints the changed map to
                                stdout; -i writes it back to <map>
                                instead, through a temporary file and a
                                rename (a map behind a symlink is saved
                                through it); --dry-run prints, as a unified
                                diff, what -i would change in the file
                                on disk, and writes nothing (the
                                commands still run). Each command runs
                                in turn and is waited for with no time
                                limit; what it writes on stderr is
                                passed on. No matching section exits 1,
                                and so does a command that fails, before
                                anything is written.";

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
            out.push_str(&tilde_path(Path::new(map)));
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
    let found: [&str; N] = found
        .try_into()
        .map_err(|_| CliError::Usage(format!("{verb}: expected {N} argument(s), found {count}")))?;
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
    // The output is resolved first — a path named on maptool's own
    // command line is written through a symlink to its real file, as a
    // map's save is — and the "not a map" rule is held to the file
    // written as well as to the name typed (amendment E1.1), so a link
    // cannot carry the Markdown over a map.
    let real = match out {
        None => None,
        Some(path) => Some(real_file(Path::new(path)).map_err(|refusal| failed("export", refusal))?),
    };
    if let (Some(path), Some(real)) = (out, &real) {
        if file::has_map_extension(path) {
            return Err(CliError::Failed(format!(
                "export: {}: names a map; the Markdown goes to a file of its own",
                tilde_path(Path::new(path))
            )));
        }
        if file::has_map_extension(&real.to_string_lossy()) {
            return Err(CliError::Failed(format!(
                "export: {}: links to `{}`, a map; the Markdown goes to a file of its own",
                tilde_path(Path::new(path)),
                collapse_home_in(&shown_path(real))
            )));
        }
    }
    let markdown = export::markdown(&read(map)?);
    match real {
        None => Ok(markdown),
        Some(real) => publish(&real, true, |out| {
            out.write_all(markdown.as_bytes()).map_err(|e| e.to_string())
        })
        .map(|_| String::new())
        .map_err(|refusal| failed("export", refusal)),
    }
}

/// A refusal of the guarded writer as `verb`'s failure, home folded.
fn failed(verb: &str, refusal: PublishRefusal) -> CliError {
    let (PublishRefusal::Invalid(message) | PublishRefusal::Internal(message)) = refusal;
    CliError::Failed(collapse_home_in(&format!("{verb}: {message}")))
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
    let bytes = file::read_bytes(map).map_err(CliError::Failed)?;
    let mut doc = file::load(map, &bytes).map_err(CliError::Failed)?;
    let targets = apply::targets(&doc, &regex);
    if targets.is_empty() {
        return Err(CliError::Failed(format!("apply: no section matches {pattern:?}")));
    }
    let matched = targets.len();
    let changed = apply::run_commands(targets, cmd, cmd_args).map_err(CliError::Failed)?;
    apply::rewrite(&mut doc, &changed, &format!("maptool apply: {cmd}")).map_err(CliError::Failed)?;
    let after = doc
        .save_json()
        .map_err(|e| CliError::Failed(format!("apply: the map cannot be saved: {e}")))?;
    eprintln!("apply: {} of {matched} matched section(s) changed", changed.len());
    if dry_run {
        // What `-i` would do to the file on disk: nothing when nothing
        // changed, else replace its bytes — UTF-8, since they loaded —
        // with `after`.
        if changed.is_empty() {
            return Ok(String::new());
        }
        let before = String::from_utf8_lossy(&bytes);
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
