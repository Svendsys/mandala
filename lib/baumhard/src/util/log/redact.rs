// SPDX-License-Identifier: MPL-2.0

//! What a `warn!` / `error!` line may carry, now that release builds
//! keep it (CODE_CONVENTIONS §9; issue #110).
//!
//! `release_max_level_warn` made the two channels user-facing: a
//! line is pasted into a bug report, or read off the browser console
//! on wasm32. Two kinds of content that were harmless while release
//! builds discarded everything are not harmless there:
//!
//! - **Document text.** A console line is something the user typed,
//!   and a macro step's `ConsoleLine` is something they authored in
//!   `macros.json`; either can be a paragraph. [`excerpt`] keeps
//!   enough of it to recognize the line — [`EXCERPT_WIDTH`] display
//!   cells, cut on a grapheme boundary (`lib/baumhard/CONVENTIONS.md`
//!   §B3) — and marks the cut.
//! - **Absolute paths.** `/home/<user>/…` discloses the account name
//!   and the directory layout. [`tilde_path`] and [`collapse_home_in`]
//!   keep the path, which is the useful part of most such lines, and
//!   fold the home directory to `~`.
//!
//! Both are plain functions a call site reaches for; nothing here
//! wraps the `log` macros, for the reason the parent module gives.
//! The rule that every `warn!` / `error!` site *does* reach for them
//! where a path is formatted is held by `util::log_posture`, which
//! reads the sources rather than trusting this paragraph.
//!
//! Cross-platform: on wasm32 there is no home directory to find
//! (`std::env` answers nothing), so the path helpers return the path
//! as given, and the excerpt is the same on both targets (§4).

use crate::util::grapheme_chad::truncate_to_display_width;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

/// Display cells of document text a log line keeps — the "roughly
/// 40 graphemes" #110 asked for, in the unit
/// [`truncate_to_display_width`] measures.
pub const EXCERPT_WIDTH: usize = 40;

/// Display cells a *diagnosis* keeps — the console's own error for a
/// failed line, which since #136 echoes the offending token as the
/// user typed it, so a quoted paragraph would otherwise come back
/// whole in the tail of the record that excerpted it in the head.
/// Wider than [`EXCERPT_WIDTH`] because the diagnosis is the part a
/// bug report is about.
pub const FAILURE_WIDTH: usize = 120;

/// Appended to an excerpt that was cut, so a reader can tell a short
/// line from the head of a long one.
pub const EXCERPT_MARK: &str = "…";

/// The part of `text` a log line may carry: one line, at most
/// [`EXCERPT_WIDTH`] display cells, cut on a grapheme boundary and
/// marked with [`EXCERPT_MARK`] when anything was dropped.
///
/// Every control character — `\n`, `\r`, the C0 range an escape
/// sequence starts with, U+0085 — and the Unicode line and paragraph
/// separators U+2028 / U+2029 become one space first, so a value
/// cannot make one log record read as two, or as none, on any
/// terminal that writes it raw (`env_logger` does). A `\r\n` pair
/// folds to a single space.
///
/// Cost: one pass over `text` and one `String` of at most its length
/// plus the mark.
pub fn excerpt(text: &str) -> String {
    excerpt_to(text, EXCERPT_WIDTH)
}

/// [`excerpt`] at a caller-chosen width — [`FAILURE_WIDTH`] for a
/// diagnosis, [`EXCERPT_WIDTH`] for authored text.
pub fn excerpt_to(text: &str, width: usize) -> String {
    let one_line: String = text
        .replace("\r\n", " ")
        .chars()
        .map(|c| if breaks_a_record(c) { ' ' } else { c })
        .collect();
    let kept = truncate_to_display_width(&one_line, width);
    if kept.len() == one_line.len() {
        one_line
    } else {
        format!("{kept}{EXCERPT_MARK}")
    }
}

/// Whether `c`, written raw, could end a log record early or start a
/// second one: a control character or a Unicode line/paragraph
/// separator.
fn breaks_a_record(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// A console line, for a log line: the home directory folded to `~`
/// *and then* excerpted — one call, so the two cannot be half-applied
/// and the cut can never land inside `/home/<user>` and leave the
/// account name in the kept half.
///
/// The three places a console line is logged without the prompt — a
/// file-path `Action`'s line, a macro `ConsoleLine` step, and the
/// browser's no-runtime skip — all go through here, so the record
/// reads the same however the line was reached. On wasm32 there is
/// no home to fold and this is [`excerpt`].
pub fn console_line_excerpt(line: &str) -> String {
    match home_dir() {
        Some(home) => console_line_excerpt_with(line, &home),
        None => excerpt(line),
    }
}

/// [`console_line_excerpt`] with the home directory supplied: the
/// composition as a pure function, which is what the tests drive —
/// with a home long enough that cutting first would land inside it,
/// which no machine's `$HOME` can be relied on to be.
pub fn console_line_excerpt_with(line: &str, home: &Path) -> String {
    excerpt(&collapse_home_in_with(line, home))
}

/// A console failure, for a log line: home-collapsed, then excerpted
/// at [`FAILURE_WIDTH`]. The peer of [`console_line_excerpt`] for the
/// other half of the same record.
pub fn failure_excerpt(failure: &str) -> String {
    match home_dir() {
        Some(home) => failure_excerpt_with(failure, &home),
        None => excerpt_to(failure, FAILURE_WIDTH),
    }
}

/// [`failure_excerpt`] with the home directory supplied, pure.
pub fn failure_excerpt_with(failure: &str, home: &Path) -> String {
    excerpt_to(&collapse_home_in_with(failure, home), FAILURE_WIDTH)
}

/// `path`, for a log line: the home directory folded to `~` when the
/// path lies under it, otherwise the path as `Display` shows it.
///
/// Cost: one environment read and one `String`.
pub fn tilde_path(path: &Path) -> String {
    collapse_home(path, home_dir().as_deref())
}

/// [`tilde_path`] with the home directory supplied, so the rule is a
/// pure function a test can drive without touching the environment.
///
/// The match is by path component, not by prefix bytes: with a home
/// of `/home/mikal`, `/home/mikalx/map.json` is somebody else's
/// directory and is left alone. A `home` that is `None`, empty or the
/// filesystem root collapses nothing.
pub fn collapse_home(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home.and_then(usable_home) else {
        return path.display().to_string();
    };
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{MAIN_SEPARATOR}{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `text`, for a log line, with every occurrence of the home
/// directory that stands as a path folded to `~`.
///
/// For a message that was composed elsewhere and already carries a
/// path — a loader error naming the file it could not read, a
/// console line that names the map to open — where [`tilde_path`]
/// had no chance to run.
///
/// Cost: one environment read and one pass over `text`.
pub fn collapse_home_in(text: &str) -> String {
    match home_dir() {
        Some(home) => collapse_home_in_with(text, &home),
        None => text.to_string(),
    }
}

/// [`collapse_home_in`] with the home directory supplied.
///
/// An occurrence counts as a path only when it is not glued to path
/// characters on either side: `/home/mikal/a.json` and a quoted
/// `'/home/mikal'` fold, while `/home/mikalx/a.json` and
/// `x/home/mikal` do not. A `home` that is empty or the filesystem
/// root folds nothing.
pub fn collapse_home_in_with(text: &str, home: &Path) -> String {
    let Some(home) = usable_home(home) else {
        return text.to_string();
    };
    let needle = home.to_string_lossy();
    let needle = needle.trim_end_matches(['/', '\\']);
    if needle.is_empty() {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut from = 0usize;
    while let Some(at) = text[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let free_before = start == 0 || !is_path_byte(bytes[start - 1]);
        let free_after = end == text.len() || !is_path_byte(bytes[end]) || is_separator(bytes[end]);
        out.push_str(&text[from..start]);
        if free_before && free_after {
            out.push('~');
        } else {
            out.push_str(&text[start..end]);
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// A byte that continues a path or an identifier, so that the home
/// directory's spelling inside a longer name is not a home directory.
fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'~') || is_separator(byte)
}

fn is_separator(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

/// `home` unless it is too short to be anyone's home directory: an
/// empty string or the filesystem root would fold every path.
fn usable_home(home: &Path) -> Option<&Path> {
    let shown = home.to_string_lossy();
    let trimmed = shown.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        None
    } else {
        Some(home)
    }
}

/// The user's home directory as the environment names it: `HOME`,
/// and on Windows `USERPROFILE`. `None` on wasm32, where
/// `std::env` has nothing to read, and wherever neither is set.
fn home_dir() -> Option<PathBuf> {
    let from = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    from("HOME").or_else(|| if cfg!(windows) { from("USERPROFILE") } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short value is carried whole and unmarked; the value one
    /// cell past the width is cut to the width and marked. The
    /// first row is the control for the second: a truncation that
    /// fired on every input would mark the short one too.
    #[test]
    fn test_excerpt_keeps_a_short_value_and_cuts_a_long_one_at_the_width() {
        let exact: String = "a".repeat(EXCERPT_WIDTH);
        assert_eq!(excerpt(&exact), exact, "a value at the width is not cut");
        assert!(!excerpt(&exact).contains(EXCERPT_MARK));

        let long: String = "b".repeat(EXCERPT_WIDTH + 1);
        let cut = excerpt(&long);
        assert_eq!(cut, format!("{}{EXCERPT_MARK}", "b".repeat(EXCERPT_WIDTH)));
    }

    /// Wide and composed clusters are not split. Twenty-one CJK
    /// glyphs are forty-two cells, so twenty are kept and the cut
    /// lands between glyphs. A ZWJ family is one cluster measured at
    /// its base scalar's width — one cell under
    /// `grapheme_display_width`'s CJK-only table — so it is kept whole
    /// when that cell fits and dropped whole, leaving no stray member,
    /// when it does not. The two family rows differ by one `x`, which
    /// is what makes the second the control for the first.
    #[test]
    fn test_excerpt_cuts_on_grapheme_boundaries() {
        let cjk: String = "漢".repeat(21);
        assert_eq!(excerpt(&cjk), format!("{}{EXCERPT_MARK}", "漢".repeat(20)));

        let family = "👨\u{200D}👩\u{200D}👧";
        let over = format!("{}{family}", "x".repeat(EXCERPT_WIDTH));
        let cut = excerpt(&over);
        assert_eq!(cut, format!("{}{EXCERPT_MARK}", "x".repeat(EXCERPT_WIDTH)));
        for member in ['👨', '👩', '👧'] {
            assert!(
                !cut.contains(member),
                "a cluster that does not fit is dropped whole: {cut:?}"
            );
        }

        let fits = format!("{}{family}", "x".repeat(EXCERPT_WIDTH - 1));
        assert_eq!(
            excerpt(&fits),
            fits,
            "a cluster that fits is kept whole and unmarked"
        );
    }

    /// Line breaks become spaces, so an excerpt is always one log
    /// record; a value with no break is the control.
    #[test]
    fn test_excerpt_folds_line_breaks_into_spaces() {
        assert_eq!(excerpt("one\ntwo\r\nthree\rfour"), "one two three four");
        assert_eq!(excerpt("one two"), "one two");
        // The breaks a terminal honors that are not `\n` / `\r`: the
        // Unicode separators, NEL, and the ESC an escape sequence
        // starts with (the rest of the sequence is then ordinary text).
        assert_eq!(excerpt("a\u{2028}b\u{2029}c\u{85}d\x1b[2Je\tf"), "a b c d [2Je f");
    }

    /// A home an ordinary `$HOME` is short of: longer than the
    /// excerpt width on its own, so a cut taken before the fold lands
    /// inside it — the case the composed helpers exist for.
    const LONG_HOME: &str = "/home/firstname.lastname-longcorp/workspaces/2026/q4";

    /// Pure over an injected home, so it holds whatever `$HOME` is on
    /// the machine running it — `/`, a trailing slash, unset, or
    /// longer than the line.
    #[test]
    fn test_console_line_and_failure_excerpts_fold_home_before_they_cut() {
        let short_home = Path::new("/home/mikal");
        let long_home = Path::new(LONG_HOME);
        assert!(
            LONG_HOME.len() > EXCERPT_WIDTH,
            "the long home must not fit in an excerpt on its own, or the control below proves nothing"
        );
        // Exactly `EXCERPT_WIDTH` cells once folded, so the line is
        // kept whole with the account name gone, whichever home it
        // held.
        let tail = "m".repeat(EXCERPT_WIDTH - "open ~/".len());
        for home in [short_home, long_home] {
            let line = format!("open {}/{tail}", home.display());
            assert_eq!(
                console_line_excerpt_with(&line, home),
                format!("open ~/{tail}"),
                "{home:?}"
            );
        }
        // Control, the other order: cutting first lands inside the
        // long home, so the fold that follows finds no home to fold
        // and the head of the account path is what the record keeps.
        let line = format!("open {}/{tail}", long_home.display());
        let cut_first = collapse_home_in_with(&excerpt(&line), long_home);
        assert!(
            cut_first.starts_with("open /home/firstname.lastname"),
            "control: cutting before folding keeps the head of the account path: {cut_first:?}"
        );
        assert_eq!(
            failure_excerpt_with(&format!("could not open '{LONG_HOME}/x': denied"), long_home),
            "could not open '~/x': denied"
        );
        // The failure width is the wider one: a diagnosis of
        // `EXCERPT_WIDTH + 1` cells survives `failure_excerpt` whole
        // and is cut by `excerpt`.
        let diagnosis = "d".repeat(EXCERPT_WIDTH + 1);
        assert_eq!(failure_excerpt_with(&diagnosis, short_home), diagnosis);
        assert_eq!(
            excerpt(&diagnosis),
            format!("{}{EXCERPT_MARK}", "d".repeat(EXCERPT_WIDTH))
        );
        let paragraph = "p".repeat(FAILURE_WIDTH + 1);
        assert_eq!(
            failure_excerpt_with(&paragraph, short_home),
            format!("{}{EXCERPT_MARK}", "p".repeat(FAILURE_WIDTH))
        );
    }

    /// The environment-reading pair is wired to the same composition.
    /// The expectation is derived from what `usable_home` makes of
    /// this machine's `$HOME` — folded when there is a usable one,
    /// untouched when there is none (unset, empty, `/`) — so the test
    /// holds under any home and still goes red if either function
    /// stops reading it.
    #[test]
    fn test_console_line_and_failure_excerpts_read_the_home_the_collapse_helpers_read() {
        let home = home_dir();
        match home.as_deref().and_then(usable_home) {
            Some(home) => {
                let shown = home.to_string_lossy();
                let needle = shown.trim_end_matches(['/', '\\']);
                assert_eq!(console_line_excerpt(&format!("open {needle}/x")), "open ~/x");
                assert_eq!(failure_excerpt(&format!("{needle}/x: denied")), "~/x: denied");
            }
            None => {
                assert_eq!(console_line_excerpt("open /x"), "open /x");
                assert_eq!(failure_excerpt("/x: denied"), "/x: denied");
            }
        }
    }

    /// A path under the home directory folds to `~`, the home itself
    /// folds to a bare `~`, and the three neighbors that are not under
    /// it — a sibling whose name merely starts the same way, a
    /// relative path, and a path with no home to compare against —
    /// come back as `Display` shows them.
    #[test]
    fn test_collapse_home_folds_by_component_not_by_prefix_bytes() {
        let home = Path::new("/home/mikal");
        assert_eq!(
            collapse_home(Path::new("/home/mikal/maps/a.json"), Some(home)),
            "~/maps/a.json"
        );
        assert_eq!(collapse_home(Path::new("/home/mikal"), Some(home)), "~");
        assert_eq!(
            collapse_home(Path::new("/home/mikalx/a.json"), Some(home)),
            "/home/mikalx/a.json",
            "a sibling account whose name starts the same way is not home"
        );
        assert_eq!(collapse_home(Path::new("maps/a.json"), Some(home)), "maps/a.json");
        assert_eq!(
            collapse_home(Path::new("/home/mikal/a.json"), None),
            "/home/mikal/a.json"
        );
    }

    /// An empty home and the filesystem root fold nothing: either
    /// would turn every absolute path into `~/…`.
    #[test]
    fn test_collapse_home_refuses_a_home_that_would_fold_everything() {
        for home in ["", "/"] {
            assert_eq!(
                collapse_home(Path::new("/etc/hosts"), Some(Path::new(home))),
                "/etc/hosts",
                "home {home:?} must fold nothing"
            );
            assert_eq!(
                collapse_home_in_with("read /etc/hosts", Path::new(home)),
                "read /etc/hosts"
            );
        }
    }

    /// Inside composed text the home directory folds wherever it
    /// stands as a path — quoted, mid-sentence, at the end, more than
    /// once, with or without a trailing separator on the home itself —
    /// and not where it is glued to path characters on either side.
    #[test]
    fn test_collapse_home_in_text_folds_paths_and_leaves_lookalikes() {
        let home = Path::new("/home/mikal");
        assert_eq!(
            collapse_home_in_with("could not load '/home/mikal/a.json': no such file", home),
            "could not load '~/a.json': no such file"
        );
        assert_eq!(collapse_home_in_with("wrote /home/mikal", home), "wrote ~");
        assert_eq!(
            collapse_home_in_with("/home/mikal/a and /home/mikal/b", home),
            "~/a and ~/b"
        );
        assert_eq!(
            collapse_home_in_with("could not load '/home/mikal/a.json'", Path::new("/home/mikal/")),
            "could not load '~/a.json'",
            "a trailing separator on the home is not part of the needle"
        );
        assert_eq!(
            collapse_home_in_with("/home/mikalx/a.json", home),
            "/home/mikalx/a.json",
            "a longer account name is left alone"
        );
        assert_eq!(
            collapse_home_in_with("x/home/mikal/a.json", home),
            "x/home/mikal/a.json",
            "the spelling inside a longer path is left alone"
        );
        assert_eq!(collapse_home_in_with("no path here", home), "no path here");
    }
}
