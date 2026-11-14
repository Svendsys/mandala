// SPDX-License-Identifier: MPL-2.0

//! `apply`: rewrite the text of every section a regex matches by piping
//! it through a command.
//!
//! Each section whose text has a line the pattern matches is a target.
//! Its text — its spans concatenated — goes to the command's stdin, and
//! the command's stdout, less one trailing line break, is its new text.
//! Every target whose text changed has its spans replaced by one span of
//! the new text, with no style of its own (none at all when the new text
//! is empty), by [`replace_spans`] — and all of them together are **one
//! [`EditRequest`]**, applied by [`Document::apply`]. Nothing else
//! writes to the document.
//!
//! The commands all run before the request is built, and the first that
//! fails ends the run with nothing written.

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use baumhard::arena::NodeId;
use baumhard::document::element::SpanData;
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::writer::{ChangeSource, EditRequest};
use regex::Regex;

use crate::map::{sections, text_into, Nodes};

/// A section the pattern matched.
pub struct Target {
    /// The section.
    pub section: NodeId,
    /// Its text when it was matched.
    pub text: String,
}

/// Every section of `doc` with a line `regex` matches, in the file's
/// order. Cost: one walk of the nodes and their sections; one `String`
/// per target.
pub fn targets(doc: &Document, regex: &Regex) -> Vec<Target> {
    let mut out = Vec::new();
    let mut text = String::new();
    for (node, _) in Nodes::new(doc) {
        for section in sections(doc, node) {
            text_into(doc, section, &mut text);
            if text.lines().any(|line| regex.is_match(line)) {
                out.push(Target {
                    section,
                    text: text.clone(),
                });
            }
        }
    }
    out
}

/// Run `cmd` over each target's text, in order, and return the targets
/// whose text it changed, each with its new text. The first command that
/// fails is the error, and the commands after it do not run.
pub fn run_commands(targets: Vec<Target>, cmd: &str, args: &[String]) -> Result<Vec<Target>, String> {
    let mut changed = Vec::new();
    for target in targets {
        let text = run_pipe(cmd, args, &target.text)?;
        if text != target.text {
            changed.push(Target {
                section: target.section,
                text,
            });
        }
    }
    Ok(changed)
}

/// Replace the spans of each changed section with one span of its new
/// text, as one [`EditRequest`] applied through [`Document::apply`]. A
/// list with nothing in it changes nothing.
pub fn rewrite(doc: &mut Document, changed: &[Target], label: &str) -> Result<(), String> {
    let mut steps = Vec::new();
    for target in changed {
        let spans: Vec<SpanData> = if target.text.is_empty() {
            Vec::new()
        } else {
            vec![SpanData {
                text: target.text.clone(),
                ..SpanData::default()
            }]
        };
        steps.extend(replace_spans(doc, &[target.section], &spans).map_err(|e| format!("apply: {e}"))?);
    }
    doc.apply(EditRequest::new(steps, label), ChangeSource::Console)
        .map(|_| ())
        .map_err(|e| format!("apply: the document refused the change: {e}"))
}

/// The lines that differ between `before` and `after`, as a unified diff
/// with no context lines (`diff -U0`), headed `--- a/<name>` and
/// `+++ b/<name>`; empty when the two are equal.
///
/// The two are saves of one document before and after `apply`, which
/// changes only span text inside a `sections` value — one line of its
/// node's block, since every key of a node is one line in the saved
/// layout (D31). So the lines correspond one to one, and each changed
/// line is a hunk of its own; runs of adjacent changed lines are one
/// hunk. Should the two texts differ in length, what follows the shorter
/// one's end is a last hunk, so the diff is still exact.
pub fn diff(name: &str, before: &str, after: &str) -> String {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let common = old.len().min(new.len());
    let mut hunks: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < common {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < common && old[i] != new[i] {
            i += 1;
        }
        hunks.push((start, i - start, start, i - start));
    }
    if old.len() != new.len() {
        hunks.push((common, old.len() - common, common, new.len() - common));
    }
    let mut out = String::new();
    if hunks.is_empty() {
        return out;
    }
    let _ = writeln!(out, "--- a/{name}\n+++ b/{name}");
    for (old_start, old_count, new_start, new_count) in hunks {
        let _ = writeln!(
            out,
            "@@ -{} +{} @@",
            range(old_start, old_count),
            range(new_start, new_count)
        );
        for line in &old[old_start..old_start + old_count] {
            let _ = writeln!(out, "-{line}");
        }
        for line in &new[new_start..new_start + new_count] {
            let _ = writeln!(out, "+{line}");
        }
    }
    out
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
/// its output), and return its stdout less one trailing `\n` or `\r\n`.
/// A failed spawn, a non-zero exit (with the command's stderr), output
/// that is not UTF-8 or a failed write to its stdin is the error; a
/// command that exits without reading all of its stdin is not one, so
/// its own status is what is reported.
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
    let mut out =
        String::from_utf8(output.stdout).map_err(|e| format!("apply: `{cmd}` wrote output that is not UTF-8: {e}"))?;
    if out.ends_with('\n') {
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
    /// numbered from 1; equal texts give nothing.
    ///
    /// Control: numbering from 0 prints `@@ -1 +1 @@` for the second
    /// line; ending a hunk after one line splits the pair in two.
    #[test]
    fn test_apply_diff_is_a_zero_context_unified_diff() {
        let before = "a\nb\nc\nd\ne\n";
        let after = "a\nB\nc\nD\nE\n";
        assert_eq!(
            diff("m.mandala.json", before, after),
            "--- a/m.mandala.json\n+++ b/m.mandala.json\n@@ -2 +2 @@\n-b\n+B\n@@ -4,2 +4,2 @@\n-d\n-e\n+D\n+E\n"
        );
        assert_eq!(diff("m", before, before), "");
    }

    /// Texts of different lengths end with a hunk for the extra lines,
    /// numbered as `diff -U0` numbers an insertion: after line 2.
    ///
    /// Control: dropping the length check loses the last hunk, and the
    /// diff comes back empty.
    #[test]
    fn test_apply_diff_reports_lines_past_the_shorter_end() {
        assert_eq!(diff("m", "a\nb\n", "a\nb\nc\n"), "--- a/m\n+++ b/m\n@@ -2,0 +3 @@\n+c\n");
    }
}
