// SPDX-License-Identifier: MPL-2.0

//! `grep`: every line of text a regex matches, in every string a map
//! draws as glyphs (amendment E1.1 on #253, §4 and its second and third
//! addenda). The list is not kept here: it is whatever [`walk`] visits,
//! and `walk` names every string field of every element it visits, so a
//! field the format gains fails to compile there until it is searched or
//! listed as not drawn. Today that is:
//!
//! - the spans of sections, frame rings, connectors and line links with
//!   their caps and labels, and portal ends with theirs;
//! - the `content` a connector, a line link or a portal end sets of its
//!   own, which is what it draws when it has no spans;
//! - a `once` fit's `filler`, on a frame, a ring, a connector or a line
//!   link;
//! - a frame's or a ring's `sides` ornament, and a ring's `vertical` and
//!   `corners` texts.
//!
//! Not drawn, so not searched: a span's `url` link. A string the map's
//! defaults give is not the element's own and is not searched either.
//!
//! An element's spans are concatenated into its text, which is matched
//! line by line, as each string is. A hit prints as
//! `#<id> <element>: <line>`, the element named as the saved map spells
//! its path below the node (`sections[0]`, `links[2].ends[0].label`,
//! `connector.content`, `frame.rings[0].fit.once.filler`,
//! `frame.rings[0].corners[1]`), in the saved map's order. One spelling
//! differs from a hand-written file's: a `sides` ornament with no color
//! is named `.sides` even when the file wrote it as an object, since the
//! saver writes it bare and the document keeps no record of the file's
//! spelling.

use std::fmt::Write as _;

use baumhard::document::ids::Document;
use regex::Regex;

use crate::map::{ident, lines, text_into, walk, Found, Nodes};

/// Every line `regex` matches, one `#<id> <element>: <line>\n` each, and
/// how many there are.
///
/// Cost: one walk of the nodes and every element that holds spans, one
/// match per line of text; the output and one reused text buffer.
pub fn grep(doc: &Document, regex: &Regex) -> (String, usize) {
    let mut out = String::new();
    let mut hits = 0;
    let mut buffer = String::new();
    for (node, _) in Nodes::new(doc) {
        let id = ident(doc, node);
        walk(doc, node, |place, found| {
            let (field, text) = match found {
                Found::Spans(owner) => {
                    text_into(doc, owner, &mut buffer);
                    (None, buffer.as_str())
                }
                Found::Field(field, text) => (Some(field), text),
            };
            for line in lines(text).filter(|line| regex.is_match(line)) {
                match field {
                    None => {
                        let _ = writeln!(out, "#{id} {place}: {line}");
                    }
                    Some(field) => {
                        let _ = writeln!(out, "#{id} {place}{field}: {line}");
                    }
                }
                hits += 1;
            }
        });
    }
    (out, hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format;

    /// One hit per place that holds text, each named by its path, in the
    /// saved file's key order: a section's second line, a ring, a line
    /// link's filler, `content`, spans, caps and label, a portal end's
    /// `content`, spans and label, and a connector's filler, `content`,
    /// spans, caps and label. A line that does not match is not printed,
    /// and neither is a default's `content` or filler.
    ///
    /// Control: misspelling one place (the connector's start cap),
    /// dropping the `content` search, or dropping the filler search
    /// fails the comparison.
    #[test]
    fn test_grep_finds_text_in_every_element_that_holds_spans() {
        let doc = format::load(
            r#"{"format": 1, "name": "m",
              "defaults": {"connector": {"content": "x default", "fit": {"once": {"filler": "x default filler"}}}},
              "nodes": [
              {"id": 1, "position": [0, 0],
               "sections": [{"spans": [{"text": "no\nx sec"}]}],
               "frame": {"rings": [{"spans": [{"text": "x ring"}]}]},
               "links": [{"to": 2, "kind": "line", "fit": {"once": {"filler": "x line filler"}}, "content": "x line content", "spans": [{"text": "x line"}],
                          "cap_start": {"spans": [{"text": "x link start"}]}, "cap_end": {"spans": [{"text": "x link end"}]},
                          "label": {"spans": [{"text": "x link label"}]}},
                         {"to": 2, "kind": "portal", "ends": [{"content": "x marker"}, {"spans": [{"text": "x end"}], "label": {"spans": [{"text": "x end label"}]}}]}]},
              {"id": 2, "parent": 1, "position": [0, 0],
               "connector": {"fit": {"once": {"filler": "x con filler"}}, "content": "x con content", "spans": [{"text": "x con"}], "cap_start": {"spans": [{"text": "x start"}]},
                             "cap_end": {"spans": [{"text": "x cap"}]}, "label": {"spans": [{"text": "x", "weight": "bold"}, {"text": " label"}]}}}
            ]}"#,
        )
        .unwrap();
        let (out, hits) = grep(&doc, &Regex::new("^x").unwrap());
        assert_eq!(
            out,
            "#1 sections[0]: x sec\n\
             #1 frame.rings[0]: x ring\n\
             #1 links[0].fit.once.filler: x line filler\n\
             #1 links[0].content: x line content\n\
             #1 links[0]: x line\n\
             #1 links[0].cap_start: x link start\n\
             #1 links[0].cap_end: x link end\n\
             #1 links[0].label: x link label\n\
             #1 links[1].ends[0].content: x marker\n\
             #1 links[1].ends[1]: x end\n\
             #1 links[1].ends[1].label: x end label\n\
             #2 connector.fit.once.filler: x con filler\n\
             #2 connector.content: x con content\n\
             #2 connector: x con\n\
             #2 connector.cap_start: x start\n\
             #2 connector.cap_end: x cap\n\
             #2 connector.label: x label\n"
        );
        assert_eq!(hits, 17);
    }

    /// An empty section is one empty line: `^$` finds it, and finds no
    /// section that has text.
    ///
    /// Control: matching `str::lines` alone (no line for an empty text)
    /// finds nothing.
    #[test]
    fn test_grep_finds_an_empty_section_as_one_empty_line() {
        let doc = format::load(
            r#"{"format": 1, "name": "m", "nodes": [
              {"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "a"}]}, {"channel": 1}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            grep(&doc, &Regex::new("^$").unwrap()),
            ("#1 sections[1]: \n".to_string(), 1)
        );
    }

    /// A frame's own strings are searched and named by their paths: the
    /// frame's filler and its `sides` as a bare string, then a ring's
    /// filler, its colored `sides` by its `content`, its spans, its
    /// `vertical` and each matching corner. A default's `sides` is not
    /// the frame's own and is not searched.
    ///
    /// Control: dropping any one of the searches (filler, sides,
    /// vertical, corners) fails the comparison.
    #[test]
    fn test_grep_finds_the_frames_ornaments() {
        let doc = format::load(
            r##"{"format": 1, "name": "m",
              "defaults": {"frame": {"sides": "x default side"}},
              "nodes": [
              {"id": 3, "position": [0, 0],
               "frame": {"fit": {"once": {"filler": "x frame filler"}}, "sides": "x frame side", "rings": [
                 {"spans": [{"text": "ring"}]},
                 {"channel": 1, "fit": {"once": {"filler": "x ring filler"}}, "sides": {"content": "x ring side", "color": "#ffffff"},
                  "vertical": "x vert", "corners": ["x c0", "no", "x c2", "no"]}]}}
            ]}"##,
        )
        .unwrap();
        let (out, hits) = grep(&doc, &Regex::new("^x").unwrap());
        assert_eq!(
            out,
            "#3 frame.fit.once.filler: x frame filler\n\
             #3 frame.sides: x frame side\n\
             #3 frame.rings[1].fit.once.filler: x ring filler\n\
             #3 frame.rings[1].sides.content: x ring side\n\
             #3 frame.rings[1].vertical: x vert\n\
             #3 frame.rings[1].corners[0]: x c0\n\
             #3 frame.rings[1].corners[2]: x c2\n"
        );
        assert_eq!(hits, 7);
    }

    /// An uncolored `sides` written by hand as an object is named as the
    /// saved map spells it — the bare `frame.sides` — since the saver
    /// writes an ornament with no color as the bare string and the
    /// document keeps no record of the file's spelling.
    ///
    /// Control: naming by the presence of `content` (every `On` as
    /// `.sides.content`) fails it.
    #[test]
    fn test_grep_names_an_uncolored_object_sides_as_saved() {
        let doc = format::load(
            r#"{"format": 1, "name": "m", "nodes": [
              {"id": 1, "position": [0, 0], "frame": {"sides": {"content": "xs"}}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            grep(&doc, &Regex::new("xs").unwrap()),
            ("#1 frame.sides: xs\n".to_string(), 1)
        );
        let saved = format::save(&doc).unwrap();
        assert!(
            saved.contains(r#""frame": {"sides": "xs"}"#),
            "precondition: the saver writes the bare form: {saved}"
        );
    }
}
