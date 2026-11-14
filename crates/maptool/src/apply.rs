// SPDX-License-Identifier: MPL-2.0

//! `apply`: rewrite the text of every section a regex matches by piping
//! it through a command.
//!
//! A section is a target when **a line** of its text — its spans
//! concatenated — matches the pattern, as `grep` matches; a section with
//! no text is one empty line. Its text goes to the command's stdin, and
//! the command's stdout is its new text, compared on the input's own
//! terms: a command such as `tr` or `sed` ends its output with a line
//! break the input may not have had, so one trailing line break is
//! removed from the output **only when the input did not end in one**,
//! and an identity command (`cat`) changes nothing.
//!
//! Every target whose text changed has its spans replaced by one span of
//! the new text (none at all when the new text is empty), by
//! [`replace_spans`] — and all of them together are **one
//! [`EditRequest`]**, applied by [`Document::apply`]. Nothing else
//! writes to the document.
//!
//! The new span takes the style of the section's first span, so a
//! rewritten title keeps its size, color and weight; a section whose
//! spans were styled differently keeps only the first one's. A span's
//! `link` is not carried: it belonged to the text it was on, which the
//! command has replaced.
//!
//! The commands all run before the request is built, one at a time and
//! each waited for **without a time limit**, as `sed` and `xargs` wait;
//! the first that fails ends the run with nothing written. What a
//! command writes on stderr reaches the user's stderr: after it exits
//! when it succeeds, and inside the failure's message when it fails.

use std::io::Write as _;
use std::process::{Command, Stdio};

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, SpanData, TextStyle};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::writer::{ChangeSource, EditRequest};
use regex::Regex;

use crate::map::{lines, sections, text_into, Nodes};

/// A section the pattern matched.
pub struct Target {
    /// The section.
    pub section: NodeId,
    /// Its text when it was matched.
    pub text: String,
    /// The style of its first span, which the new span takes.
    pub style: TextStyle,
}

/// Every section of `doc` with a line `regex` matches, in the file's
/// order. Cost: one walk of the nodes and their sections; one `String`
/// and one style per target.
pub fn targets(doc: &Document, regex: &Regex) -> Vec<Target> {
    let mut out = Vec::new();
    let mut text = String::new();
    for (node, _) in Nodes::new(doc) {
        for section in sections(doc, node) {
            text_into(doc, section, &mut text);
            if lines(&text).any(|line| regex.is_match(line)) {
                let style = doc
                    .children(section)
                    .find_map(|id| match doc.get(id) {
                        Some(DocElement::Span(span)) => Some(span.style.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                out.push(Target {
                    section,
                    text: text.clone(),
                    style,
                });
            }
        }
    }
    out
}

/// Run `cmd` over each target's text, in order, and return the targets
/// whose text it changed, each with its new text. The first command that
/// fails is the error, and the commands after it do not run.
///
/// Cost: one process per target, each waited for in turn.
pub fn run_commands(targets: Vec<Target>, cmd: &str, args: &[String]) -> Result<Vec<Target>, String> {
    let mut changed = Vec::new();
    for target in targets {
        let text = run_pipe(cmd, args, &target.text)?;
        if text != target.text {
            changed.push(Target { text, ..target });
        }
    }
    Ok(changed)
}

/// Replace the spans of each changed section with one span of its new
/// text in its first span's style, as one [`EditRequest`] applied
/// through [`Document::apply`]. A list with nothing in it changes
/// nothing.
///
/// Cost: [`replace_spans`]' steps for each section, applied as one
/// journal.
pub fn rewrite(doc: &mut Document, changed: &[Target], label: &str) -> Result<(), String> {
    let mut steps = Vec::new();
    for target in changed {
        let spans: Vec<SpanData> = if target.text.is_empty() {
            Vec::new()
        } else {
            vec![SpanData {
                text: target.text.clone(),
                style: target.style.clone(),
                link: None,
            }]
        };
        steps.extend(replace_spans(doc, &[target.section], &spans).map_err(|e| format!("apply: {e}"))?);
    }
    // `Console` is a compromise (amendment E1.1 on #253): maptool's
    // command line stands in for a console line; no history, journal or
    // event outlives the process, so the source is never reported. The
    // design's sources (§8) name no tool, and adding one is a design
    // change this sub-issue does not make.
    doc.apply(EditRequest::new(steps, label), ChangeSource::Console)
        .map(|_| ())
        .map_err(|e| format!("apply: the document refused the change: {e}"))
}

/// What `--dry-run` prints: a unified diff, with no context lines
/// (`diff -U0`), from `before` — the map's bytes as they are on disk —
/// to `after`, the text `-i` would write in their place; empty when the
/// two are equal. Both headers name `name`, the map's path as given, so
/// the diff applies with `patch -p0`.
///
/// When the two have as many lines, they correspond one to one — the
/// case of a file in the saver's layout, where `apply` changes only the
/// one `sections` line of each node it rewrites (D31) — and each run of
/// changed lines is a hunk. Otherwise the file is in another layout,
/// `-i` rewrites all of it, and the diff says exactly that: one hunk of
/// every line out and every line in. A last line with no line break is
/// marked as `diff` marks it.
///
/// Cost: one pass over both texts; the output.
pub fn diff(name: &str, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let old: Vec<&str> = before.split_inclusive('\n').collect();
    let new: Vec<&str> = after.split_inclusive('\n').collect();
    let mut out = format!("--- {name}\n+++ {name}\n");
    if old.len() != new.len() {
        hunk(&mut out, &old, 0, &new, 0);
        return out;
    }
    let mut i = 0;
    while i < old.len() {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < old.len() && old[i] != new[i] {
            i += 1;
        }
        hunk(&mut out, &old[start..i], start, &new[start..i], start);
    }
    out
}

/// One hunk: `old`, from 0-based line `old_start`, out; `new`, from
/// `new_start`, in.
fn hunk(out: &mut String, old: &[&str], old_start: usize, new: &[&str], new_start: usize) {
    out.push_str(&format!(
        "@@ -{} +{} @@\n",
        range(old_start, old.len()),
        range(new_start, new.len())
    ));
    for (sign, lines) in [('-', old), ('+', new)] {
        for line in lines {
            out.push(sign);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
}

/// A hunk's range of `count` lines from 0-based `start`, as a unified
/// diff spells it: `l` for one line, `l,n` otherwise, and for no lines
/// the line before them, `l,0`.
fn range(start: usize, count: usize) -> String {
    match count {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        _ => format!("{},{count}", start + 1),
    }
}

/// Spawn `cmd args`, write `input` to its stdin from a thread of its own
/// (so an input larger than the pipe's buffer cannot deadlock against
/// its output), wait for it with no time limit, and return its stdout —
/// less one trailing `\n` or `\r\n` when `input` does not end in `\n`.
/// What it wrote on stderr is passed to this process's stderr when it
/// succeeds.
///
/// A failed spawn, a non-zero exit (with the command's stderr), output
/// that is not UTF-8 or a failed write to its stdin is the error; a
/// command that exits without reading all of its stdin is not one, so
/// its own status is what is reported.
///
/// Cost: one process; its whole stdout and stderr, and a copy of
/// `input` for the writing thread.
fn run_pipe(cmd: &str, args: &[String], input: &str) -> Result<String, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("apply: could not run `{cmd}`: {e}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("apply: `{cmd}`: no stdin to write to"))?;
    let bytes = input.as_bytes().to_vec();
    let name = cmd.to_string();
    // The thread owns `stdin` and drops it as it returns, on every path,
    // which is what lets the command see the end of its input.
    let writer = std::thread::spawn(move || match stdin.write_all(&bytes) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            Err(format!("apply: `{name}`: could not write its input: {e}"))
        }
        _ => Ok(()),
    });
    let output = child
        .wait_with_output()
        .map_err(|e| format!("apply: `{cmd}`: {e}"))?;
    writer
        .join()
        .map_err(|_| format!("apply: `{cmd}`: the thread writing its input panicked"))??;
    if !output.status.success() {
        let status = output
            .status
            .code()
            .map_or_else(|| "a signal".to_string(), |code| code.to_string());
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("apply: `{cmd}` exited with {status}")
        } else {
            format!("apply: `{cmd}` exited with {status}: {stderr}")
        });
    }
    // A stderr this process cannot write to has nowhere else to go.
    let _ = std::io::stderr().write_all(&output.stderr);
    let mut out = String::from_utf8(output.stdout)
        .map_err(|e| format!("apply: `{cmd}` wrote output that is not UTF-8: {e}"))?;
    if !input.ends_with('\n') && out.ends_with('\n') {
        out.pop();
        if out.ends_with('\r') {
            out.pop();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Changed lines are hunks of their own, adjacent ones one hunk,
    /// numbered from 1 and headed by the name as given; equal texts give
    /// nothing.
    ///
    /// Control: ending a hunk after one changed line splits the `d`, `e`
    /// pair in two, and the comparison goes red.
    #[test]
    fn test_apply_diff_is_a_zero_context_unified_diff() {
        let before = "a\nb\nc\nd\ne\n";
        let after = "a\nB\nc\nD\nE\n";
        assert_eq!(
            diff("/tmp/m.mandala.json", before, after),
            "--- /tmp/m.mandala.json\n+++ /tmp/m.mandala.json\n@@ -2 +2 @@\n-b\n+B\n@@ -4,2 +4,2 @@\n-d\n-e\n+D\n+E\n"
        );
        assert_eq!(diff("m", before, before), "");
    }

    /// A text of another shape — one line where the save writes three —
    /// is diffed as it is: the one line goes, the three come.
    ///
    /// Control: dropping the whole-file branch compares the first line
    /// alone and loses the two added lines.
    #[test]
    fn test_apply_diff_reports_a_change_of_shape() {
        assert_eq!(
            diff("m", "{\"a\": 1}\n", "{\n  \"a\": 2\n}\n"),
            "--- m\n+++ m\n@@ -1 +1,3 @@\n-{\"a\": 1}\n+{\n+  \"a\": 2\n+}\n"
        );
    }

    /// One trailing line break is removed from the output only when the
    /// input had none: `cat` returns a text ending in `\n` unchanged,
    /// and `printf` output gains no break it did not write.
    ///
    /// Control: removing the break whatever the input fails the first
    /// assertion (`"foo"` for `"foo\n"`).
    #[cfg(unix)]
    #[test]
    fn test_apply_keeps_the_inputs_trailing_line_break() {
        assert_eq!(run_pipe("cat", &[], "foo\n").unwrap(), "foo\n");
        assert_eq!(run_pipe("cat", &[], "foo").unwrap(), "foo");
        assert_eq!(
            run_pipe("tr", &["a-z".into(), "A-Z".into()], "foo").unwrap(),
            "FOO"
        );
        assert_eq!(
            run_pipe("sh", &["-c".into(), "cat; echo".into()], "foo").unwrap(),
            "foo"
        );
    }
}
