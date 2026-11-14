// SPDX-License-Identifier: MPL-2.0

//! `grep`: every line of span text a regex matches, wherever a map holds
//! spans — sections, frame rings, connectors and line links with their
//! caps and labels, and portal ends with theirs.
//!
//! Each element's spans are concatenated into its text, which is
//! matched line by line. A hit prints as `#<id> <element>: <line>`, the
//! element named as a load error's path names it below its node
//! (`sections[0]`, `frame.rings[1]`, `links[2].ends[0].label`). Hits come
//! in the file's order: nodes in pre-order, and within a node in
//! [`text_owners`]'s order.

use std::fmt::Write as _;

use baumhard::document::ids::Document;
use regex::Regex;

use crate::map::{ident, text_into, text_owners, Nodes};

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
            for line in text.lines().filter(|line| regex.is_match(line)) {
                let _ = writeln!(out, "#{id} {place}: {line}");
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

    /// One hit per kind of element that holds spans, each named by its
    /// path: a section's second line, a ring, a connector, its cap and
    /// label, a line link, a portal end and a portal end's label. A line
    /// that does not match is not printed.
    ///
    /// Control: dropping `path_owners`' cap and label calls loses the
    /// `connector.cap_end` and `connector.label` lines. (Numbering links
    /// only among line links is not seen here, where the line link comes
    /// first; the CLI test on the portals fixture is its control.)
    #[test]
    fn test_grep_finds_text_in_every_element_that_holds_spans() {
        let doc = format::load(
            r#"{"format": 1, "name": "m", "nodes": [
              {"id": 1, "position": [0, 0],
               "sections": [{"spans": [{"text": "no\nx sec"}]}],
               "frame": {"rings": [{"spans": [{"text": "x ring"}]}]},
               "links": [{"to": 2, "kind": "line", "spans": [{"text": "x line"}]},
                         {"to": 2, "kind": "portal", "ends": [{}, {"spans": [{"text": "x end"}], "label": {"spans": [{"text": "x end label"}]}}]}]},
              {"id": 2, "parent": 1, "position": [0, 0],
               "connector": {"spans": [{"text": "x con"}], "cap_end": {"spans": [{"text": "x cap"}]}, "label": {"spans": [{"text": "x", "weight": "bold"}, {"text": " label"}]}}}
            ]}"#,
        )
        .unwrap();
        let (out, hits) = grep(&doc, &Regex::new("^x").unwrap());
        assert_eq!(
            out,
            "#1 sections[0]: x sec\n\
             #1 frame.rings[0]: x ring\n\
             #1 links[0]: x line\n\
             #1 links[1].ends[1]: x end\n\
             #1 links[1].ends[1].label: x end label\n\
             #2 connector: x con\n\
             #2 connector.cap_end: x cap\n\
             #2 connector.label: x label\n"
        );
        assert_eq!(hits, 8);
    }
}
