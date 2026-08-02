// SPDX-License-Identifier: MPL-2.0

//! A real `log::Log` sink, so a test can assert that a degrade path
//! actually said something.
//!
//! CODE_CONVENTIONS §9 makes `warn!` the visible half of "degrade,
//! log, keep running", which means the log line is part of the
//! behavior rather than decoration — and until this existed nothing
//! could tell a `warn!` that fires from one that was deleted. The
//! loader's unknown-key report is the case that forced it: the keys
//! it preserves are observable through
//! [`MindMap::unknown_keys`](crate::mindmap::model::MindMap::unknown_keys),
//! but "and it told somebody" was untestable, and a mutation deleting
//! the `warn!` left the whole suite green.
//!
//! **This is not a mock** (TEST_CONVENTIONS §T10). It is an ordinary
//! implementation of the `log` crate's own trait, installed the way
//! `env_logger` is installed and receiving exactly what a real run
//! would send it. Nothing about the code under test changes shape to
//! accommodate it.
//!
//! **How it survives parallel tests.** `log::set_logger` is
//! process-global and `cargo test` runs tests concurrently, so the
//! buffer holds lines from whatever else happened to be logging.
//! Rather than serialize the suite behind a lock — which would be a
//! standing tax on every future test — callers search the buffer for
//! a needle they made unique, and simply do not care what else is in
//! it. [`lines_containing`] is that search.
//!
//! Native-only, like the rest of `#[cfg(test)]` support here: the
//! browser build installs `console_log` and has no use for a buffer.

use std::sync::{Mutex, Once, OnceLock};

/// Every line the recording logger has been handed, formatted the way
/// a reader sees it: `"<LEVEL> <message>"`.
fn buffer() -> &'static Mutex<Vec<String>> {
    static BUFFER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(Vec::new()))
}

/// The sink itself. Records everything and prints nothing — a test
/// run should not gain a stderr flood because one test wanted to read
/// a warning.
struct Recorder;

impl log::Log for Recorder {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        // A poisoned buffer means some other test panicked mid-push.
        // Recording is not what that test failed at, so recover the
        // lock rather than turning one failure into many.
        let mut lines = buffer().lock().unwrap_or_else(|e| e.into_inner());
        lines.push(format!("{} {}", record.level(), record.args()));
    }

    fn flush(&self) {}
}

/// Install the recorder, once per process.
///
/// Idempotent and safe to call from every test that wants to read a
/// log line. A second `set_logger` would fail — and does, harmlessly,
/// if something else got there first; the `Once` is what keeps the
/// common case from even trying.
///
/// Cost: one atomic check after the first call.
pub(crate) fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        // `warn!`/`error!` are all a release build keeps
        // (`release_max_level_warn`), but a test build keeps
        // everything, and a test that wants to read a `debug!` should
        // be able to.
        if log::set_logger(&Recorder).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    });
}

/// Every recorded line containing `needle`, oldest first.
///
/// Installs the recorder if it is not already installed, so a caller
/// is one function call away from an assertion.
///
/// **`needle` has to be something only this test could have
/// produced** — a node id, a key name, a path it built. The buffer is
/// shared with every other test running at the same time, and this
/// deliberately does not clear it: clearing would race with those
/// tests instead of merely coexisting with them.
///
/// Cost: O(lines recorded so far) per call, under one mutex
/// acquisition.
pub(crate) fn lines_containing(needle: &str) -> Vec<String> {
    install();
    let lines = buffer().lock().unwrap_or_else(|e| e.into_inner());
    lines
        .iter()
        .filter(|line| line.contains(needle))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder records, and the search finds what this test put
    /// there rather than what a neighbor did.
    #[test]
    fn test_a_warning_reaches_the_buffer() {
        install();
        log::warn!("test_logger: unmistakable-needle-4a91 happened");
        let found = lines_containing("unmistakable-needle-4a91");
        assert_eq!(
            found.len(),
            1,
            "expected exactly one recorded line, got {found:?}"
        );
        assert!(
            found[0].starts_with("WARN "),
            "the level must survive: {}",
            found[0]
        );
    }

    /// A needle nobody logged finds nothing — the control that keeps
    /// the assertion above from passing on an always-true search.
    #[test]
    fn test_an_unlogged_needle_finds_nothing() {
        install();
        assert!(lines_containing("never-logged-needle-0c37").is_empty());
    }
}
