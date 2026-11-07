// SPDX-License-Identifier: MPL-2.0

//! The load-failure placard's one judgment: whether a loader message
//! already names the map it failed on (#107).
//!
//! The placard (`MindMapDocument::load_failure_placard`) and the startup
//! log line (`app::startup_load`) both put the source in front of the
//! loader's message — unless the message already names it, so a
//! mistyped path does not read `could not load '/nope/x.mandala.json':
//! /nope/x.mandala.json: could not read: …`. One predicate, so the two
//! surfaces cannot disagree about what "already names it" means.

/// Whether `message` names `source` as a path of its own.
///
/// **A substring is not a name, and the difference is user-visible.**
/// `mandala map` on a malformed file named `map` produces a parse
/// message that may contain `map` inside a longer word. A plain
/// `contains` answered yes, so the placard dropped its source line and
/// the log line dropped its prefix, and neither surface said which file
/// had failed.
///
/// So the occurrence has to be *bounded*: the characters on either side
/// of it must be ones that cannot continue a path. Alphanumerics
/// continue a name (`map` inside `mindmap`), and `_ - . / \ ~` continue
/// a path (`map` inside `map.mandala.json`, or inside `maps/map` — a
/// different file from `map`, and one the message therefore does not
/// name). Everything else — whitespace, `:`, quotes, the ends of the
/// string — bounds it. A Windows `C:\maps\x` still works: `:` bounds,
/// and the drive letter is inside the source rather than beside it.
///
/// **The residual, stated exactly.** A source whose whole name is a
/// word of the loader's own prose is still read as named, and its
/// source line is still dropped. No purely textual predicate can do
/// better: at that point the message really does contain the source,
/// bounded. Erring the other way costs a duplicated path on a screen
/// that already failed; erring this way costs the filename entirely, so
/// the bound is set where the realistic collisions are and the exotic
/// one is written down.
///
/// An empty `source` is never named: `"".contains("")` is true, and a
/// placard built from a blank source must still say what it has.
///
/// Cost: one substring search per occurrence of `source` in `message`,
/// and two character lookups per occurrence.
pub fn message_names_source(source: &str, message: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(offset) = message[from..].find(source) {
        let at = from + offset;
        let before = message[..at].chars().next_back();
        let after = message[at + source.len()..].chars().next();
        if !before.is_some_and(continues_a_path) && !after.is_some_and(continues_a_path) {
            return true;
        }
        // Past the whole occurrence, so an *overlapping* one is
        // deliberately not reached: it starts inside the previous one,
        // so the character before it is a character of `source` itself,
        // and reaching it could only turn a rejection into a false
        // match — the expensive direction, which drops the filename.
        from = at + source.len();
    }
    false
}

/// Whether `c` could be part of the same path as the character next to
/// it — the test [`message_names_source`] applies to the two characters
/// bounding a candidate occurrence.
fn continues_a_path(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\' | '~')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path the read error names whole.
    const SOURCE: &str = "/home/user/maps/broken.mandala.json";

    /// A message that names no file, with `map` inside a longer word.
    const PARSE_MESSAGE: &str = "expected a mindmap object at line 1 column 2";

    /// Every row is a spelling of the same question, and the three the
    /// predicate has to get right are a short source, a source that is a
    /// substring of a longer word, and a source that legitimately
    /// appears as its own path component.
    #[test]
    fn test_message_names_source_bounds_the_occurrence() {
        let read_error =
            "/home/user/maps/broken.mandala.json: could not read: No such file or directory (os error 2)";
        let cases: &[(&str, &str, bool, &str)] = &[
            (SOURCE, read_error, true, "the read error names the whole path"),
            (
                "map",
                PARSE_MESSAGE,
                false,
                "a substring of a longer word is not a name",
            ),
            (
                "map",
                "map: could not read: No such file or directory (os error 2)",
                true,
                "a short source bounded by the start and a colon is named",
            ),
            (
                "map",
                "maps/map.mandala.json: could not read: No such file or directory (os error 2)",
                false,
                "`maps/map.mandala.json` is a different file from `map`",
            ),
            // The separator as the *only* thing saying so: drop `/` from
            // the bounding set and every other row stays green while this
            // one flips.
            (
                "map",
                "maps/map: could not read: No such file or directory (os error 2)",
                false,
                "`maps/map` is a different file from `map`, and only the `/` says so",
            ),
            (
                "broken.json",
                "broken.json.bak: could not read: No such file or directory (os error 2)",
                false,
                "a prefix of a longer filename is not that filename",
            ),
            (
                "./map",
                PARSE_MESSAGE,
                false,
                "the parse message names no file at all",
            ),
            (
                "./map",
                "./map: could not read: No such file or directory (os error 2)",
                true,
                "the read error names the relative path as given",
            ),
            (
                "broken.json",
                "cannot open broken.json",
                true,
                "the end of the message bounds",
            ),
            (
                r"C:\maps\x.mandala.json",
                r"C:\maps\x.mandala.json: could not read: No such file or directory",
                true,
                "a Windows path is bounded by the start and the colon after it",
            ),
            // Synthetic, because only a source that overlaps itself can
            // produce one; a scan that advanced by one character would
            // answer `true` here and drop the filename.
            (
                "a a",
                "za a a",
                false,
                "an overlapping occurrence is not a second mention, it is the same one",
            ),
            ("", PARSE_MESSAGE, false, "an empty source is never named"),
            (
                "",
                "",
                false,
                "an empty source is never named, even by an empty message",
            ),
            // The residual, asserted rather than merely described.
            (
                "mindmap",
                PARSE_MESSAGE,
                true,
                "documented residual — a source named exactly like a word of the message",
            ),
        ];
        for (source, message, expected, why) in cases {
            assert_eq!(
                message_names_source(source, message),
                *expected,
                "message_names_source({source:?}, {message:?}) — {why}"
            );
        }
    }
}
