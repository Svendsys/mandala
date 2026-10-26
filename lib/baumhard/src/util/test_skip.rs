// SPDX-License-Identifier: MPL-2.0

//! A `SKIPPED:` note that reaches the terminal from a passing test.
//!
//! Some tests in this workspace guard something the machine may not
//! be able to run — a wasm32 probe under `node`
//! (`lib/baumhard/tests/wasm_font_lock.rs`), the real app on a GPU
//! adapter (`mandala`'s `app::gpu_harness`). Where the machine
//! cannot, the test passes having proved nothing, and the only thing
//! that keeps that from reading as a green result is the note it
//! prints. libtest swallows that note: a passing test's `eprintln!`
//! goes to a per-test capture buffer that is shown only when the
//! test fails or `--nocapture` is given, so under `./test.sh` the
//! note never reached anyone.
//! [`note_skipped`](crate::util::test_skip::note_skipped) writes it
//! through [`std::io::Stderr`] directly, which the capture does not
//! intercept (the capture hooks the `print!`/`eprintln!` macros'
//! path, not the `Write` impl on the handle), so the line shows in
//! the run's output and `test.sh` can count `^SKIPPED:` lines into
//! its summary.
//!
//! Native-only, like `test_temp` beside it: the browser build has no
//! libtest and no stderr to write past. Not `cfg(test)`-gated, for
//! the reason `test_temp` is not — its callers live in `mandala` and
//! in an integration test, both compiled outside this crate's own
//! `cfg(test)`.

use std::io::Write;

/// Prefix of every note, and what `test.sh` greps for.
pub const PREFIX: &str = "SKIPPED: ";

/// Write `SKIPPED: {what}` and a newline to the process's stderr,
/// past libtest's per-test capture, so the note shows in a plain
/// `cargo test` run. One write under the stderr lock; the note is
/// formatted once. A failed write is ignored — a note that cannot be
/// written has no better channel to complain through.
pub fn note_skipped(what: &str) {
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    let _ = write_note(&mut stderr, what);
    let _ = stderr.flush();
}

/// The note's text, written to `sink`: the one line [`note_skipped`]
/// sends to stderr, separable so its shape can be asserted on a
/// buffer.
pub fn write_note(sink: &mut dyn Write, what: &str) -> std::io::Result<()> {
    writeln!(sink, "{PREFIX}{what}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The note is one line that starts with the prefix `test.sh`
    /// counts — so a note whose text changed shape would stop being
    /// counted here before it stopped being counted there.
    #[test]
    fn test_note_is_one_prefixed_line() {
        let mut sink = Vec::new();
        write_note(&mut sink, "no adapter here").expect("a Vec never fails to grow");
        let text = String::from_utf8(sink).expect("the note is UTF-8");
        assert_eq!(text, "SKIPPED: no adapter here\n");
        assert_eq!(
            text.lines().filter(|line| line.starts_with("SKIPPED:")).count(),
            1,
            "exactly one countable line"
        );
    }

    /// Control for the count above: a note whose reason spans lines
    /// is still counted once, because only its first line carries
    /// the prefix — the count is of notes, not of their lines.
    #[test]
    fn test_a_multiline_reason_is_still_one_note() {
        let mut sink = Vec::new();
        write_note(&mut sink, "first line\nsecond line").expect("a Vec never fails to grow");
        let text = String::from_utf8(sink).expect("the note is UTF-8");
        assert_eq!(text.lines().count(), 2, "precondition: the reason wrapped");
        assert_eq!(
            text.lines().filter(|line| line.starts_with("SKIPPED:")).count(),
            1
        );
    }
}
