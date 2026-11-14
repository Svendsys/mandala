// SPDX-License-Identifier: MPL-2.0

//! `show`: the node tree, one line per node, indented by depth.
//!
//! A line is `#<id>`, the node's section text — each section's spans
//! concatenated, sections separated by ` │ `, a line break shown as `⏎`
//! so a node stays on one line — and a marker for its frame, each line
//! link and each portal: `[frame]`, `[link → #4]`, `[portal → #6]`.

use std::fmt::Write as _;

use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::document::values::LinkKind;

use crate::map::{ident, is, link, sections, text_into, Nodes};

/// What separates two sections of one node.
pub const SECTION_SEPARATOR: &str = " │ ";

/// What a line break inside a section is shown as.
pub const LINE_BREAK: char = '⏎';

/// The tree of `doc`, one line per node, each ending in `\n`.
///
/// Cost: one walk of the nodes and their own elements; the output and
/// one reused text buffer.
pub fn show(doc: &Document) -> String {
    let mut out = String::new();
    let mut text = String::new();
    for (node, depth) in Nodes::new(doc) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        let _ = write!(out, "#{}", ident(doc, node));
        let start = out.len();
        let mut any_text = false;
        for (n, section) in sections(doc, node).enumerate() {
            if n > 0 {
                out.push_str(SECTION_SEPARATOR);
            } else {
                out.push(' ');
            }
            text_into(doc, section, &mut text);
            any_text |= !text.is_empty();
            for c in text.chars() {
                out.push(if c == '\n' { LINE_BREAK } else { c });
            }
        }
        if !any_text {
            out.truncate(start);
        }
        for id in doc.children(node) {
            if is(doc, id, DocKind::Frame) {
                out.push_str(" [frame]");
            } else if let Some(link) = link(doc, id) {
                let kind = match link.kind {
                    LinkKind::Line => "link",
                    LinkKind::Portal => "portal",
                };
                let _ = write!(out, " [{kind} → #{}]", ident(doc, link.to));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format;

    /// A node with two sections, one holding two spans and a line break,
    /// a frame, a line link and a portal, above a child with no text: each
    /// part of a line is spelled as the module says.
    ///
    /// Control: printing sections with the separator before the first
    /// (` │ a`) or joining spans with a space fails the first line; not
    /// truncating a textless node's leading space fails the second.
    #[test]
    fn test_show_spells_text_separators_breaks_and_markers() {
        let doc = format::load(
            r#"{"format": 1, "name": "m", "nodes": [
              {"id": 7, "position": [0, 0],
               "sections": [{"spans": [{"text": "one"}, {"text": " two\nthree"}]}, {"channel": 1, "spans": [{"text": "four"}]}],
               "frame": {},
               "links": [{"to": 9, "kind": "line"}, {"to": 9, "kind": "portal"}]},
              {"id": 9, "parent": 7, "position": [0, 0], "sections": [{}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            show(&doc),
            "#7 one two⏎three │ four [frame] [link → #9] [portal → #9]\n  #9\n"
        );
    }
}
