// SPDX-License-Identifier: MPL-2.0

//! `grep`: every line of text a regex matches, wherever a map holds
//! text — the spans of sections, frame rings, connectors and line links
//! with their caps and labels, and portal ends with theirs; and the
//! `content` a connector, a line link or a portal end sets of its own,
//! which is what it draws when it has no spans (amendment E1.1 on #253).
//!
//! An element's spans are concatenated into its text, which is matched
//! line by line, as its own `content` is. A hit prints as
//! `#<id> <element>: <line>`, the element named as a load error's path
//! names it below its node (`sections[0]`, `frame.rings[1]`,
//! `links[2].ends[0].label`, `connector.content`). Hits come in the
//! file's order: nodes in pre-order, and within a node in
//! [`text_owners`]'s order, an element's `content` right after its
//! spans. A defaults' or the built-in `content` is not the element's own
//! and is not searched.

use std::fmt::Write as _;

use baumhard::document::ids::Document;
use regex::Regex;

use crate::map::{ident, lines, own_content, text_into, text_owners, Nodes};

/// Every line `regex` matches, one `#<id> <element>: <line>\n` each, and
/// how many there are.
///
/// Cost: one walk of the nodes and every element that holds spans, one
/// match per line of text; the output and one reused text buffer.
pub fn grep(doc: &Document, regex: &Regex) -> (String, usize) {
    let mut out = String::new();
    let mut hits = 0;
    let mut text = String::new();
    for (node, _) in Nodes::new(doc) {
        let id = ident(doc, node);
        text_owners(doc, node, |place, owner| {
            text_into(doc, owner, &mut text);
            for line in lines(&text).filter(|line| regex.is_match(line)) {
                let _ = writeln!(out, "#{id} {place}: {line}");
                hits += 1;
            }
            if let Some(content) = own_content(doc, owner) {
                for line in lines(content).filter(|line| regex.is_match(line)) {
                    let _ = writeln!(out, "#{id} {place}.content: {line}");
                    hits += 1;
                }
            }
        });
    }
    (out, hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format;

    /// One hit per place that holds text, each named by its path: a
    /// section's second line, a ring, a line link's spans, caps, label
    /// and own `content`, a portal end's spans, label and `content`, and
    /// a connector's spans, caps, label and `content`. A line that does
    /// not match is not printed, and neither is a default's `content`.
    ///
    /// Control: renaming one spelling (`cap_start` as `cap_begin`), or
    /// dropping the `content` search, fails the comparison.
    #[test]
    fn test_grep_finds_text_in_every_element_that_holds_spans() {
        let doc = format::load(
            r#"{"format": 1, "name": "m",
              "defaults": {"connector": {"content": "x default"}},
              "nodes": [
              {"id": 1, "position": [0, 0],
               "sections": [{"spans": [{"text": "no\nx sec"}]}],
               "frame": {"rings": [{"spans": [{"text": "x ring"}]}]},
               "links": [{"to": 2, "kind": "line", "content": "x line content", "spans": [{"text": "x line"}],
                          "cap_start": {"spans": [{"text": "x link start"}]}, "cap_end": {"spans": [{"text": "x link end"}]},
                          "label": {"spans": [{"text": "x link label"}]}},
                         {"to": 2, "kind": "portal", "ends": [{"content": "x marker"}, {"spans": [{"text": "x end"}], "label": {"spans": [{"text": "x end label"}]}}]}]},
              {"id": 2, "parent": 1, "position": [0, 0],
               "connector": {"content": "x con content", "spans": [{"text": "x con"}], "cap_start": {"spans": [{"text": "x start"}]},
                             "cap_end": {"spans": [{"text": "x cap"}]}, "label": {"spans": [{"text": "x", "weight": "bold"}, {"text": " label"}]}}}
            ]}"#,
        )
        .unwrap();
        let (out, hits) = grep(&doc, &Regex::new("^x").unwrap());
        assert_eq!(
            out,
            "#1 sections[0]: x sec\n\
             #1 frame.rings[0]: x ring\n\
             #1 links[0]: x line\n\
             #1 links[0].content: x line content\n\
             #1 links[0].cap_start: x link start\n\
             #1 links[0].cap_end: x link end\n\
             #1 links[0].label: x link label\n\
             #1 links[1].ends[0].content: x marker\n\
             #1 links[1].ends[1]: x end\n\
             #1 links[1].ends[1].label: x end label\n\
             #2 connector: x con\n\
             #2 connector.content: x con content\n\
             #2 connector.cap_start: x start\n\
             #2 connector.cap_end: x cap\n\
             #2 connector.label: x label\n"
        );
        assert_eq!(hits, 15);
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
}
