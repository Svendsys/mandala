// SPDX-License-Identifier: MPL-2.0

//! `export`: the node tree as Markdown, from section text alone.
//!
//! A node's text is its sections' text, joined by line breaks. Its first
//! line becomes a heading whose level is the node's depth in the tree
//! (`#`, `##`, `###`, …), its other lines plain text under it. A node
//! with no text (or only whitespace) is transparent: it writes nothing,
//! and its children take its level. Styles, frames, paths and links are
//! left out.
//!
//! Markdown defines headings `#` to `######`; a deeper tree keeps adding
//! `#`, which most renderers show as plain text — the export is a
//! text-and-shape dump, not a styled document.

use baumhard::arena::NodeId;
use baumhard::document::ids::Document;

use crate::map::{push_children, sections, text_into};

/// `doc` as Markdown.
///
/// **Iterative**: a chain of nodes as deep as the format allows (it sets
/// no bound on depth) is walked in constant stack. Pending nodes are
/// queued in reverse so the first sibling is written first.
///
/// Cost: one walk of the nodes and their sections; the output, one
/// reused text buffer and the pending queue.
pub fn markdown(doc: &Document) -> String {
    let mut out = String::new();
    let mut text = String::new();
    let mut section = String::new();
    let mut pending: Vec<(NodeId, usize)> = Vec::new();
    push_children(doc, doc.root(), 1, &mut pending);
    while let Some((node, level)) = pending.pop() {
        text.clear();
        for (n, id) in sections(doc, node).enumerate() {
            if n > 0 {
                text.push('\n');
            }
            text_into(doc, id, &mut section);
            text.push_str(&section);
        }
        if text.trim().is_empty() {
            push_children(doc, node, level, &mut pending);
            continue;
        }
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("");
        for _ in 0..level {
            out.push('#');
        }
        out.push(' ');
        out.push_str(first);
        out.push('\n');
        for rest in lines {
            out.push_str(rest);
            out.push('\n');
        }
        out.push('\n');
        push_children(doc, node, level + 1, &mut pending);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format;

    /// Headings follow depth, extra lines and later sections are plain
    /// text under the heading, and a textless node is transparent: its
    /// child is written at the textless node's own level.
    ///
    /// Control: pushing a textless node's children one level deeper
    /// fails the comparison (`### grandchild` instead of
    /// `## grandchild`).
    #[test]
    fn test_export_levels_lines_and_transparent_nodes() {
        let doc = format::load(
            r#"{"format": 1, "name": "m", "nodes": [
              {"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "Root\nmore"}]}, {"channel": 1, "spans": [{"text": "second"}]}]},
              {"id": 2, "parent": 1, "position": [0, 0], "sections": [{"spans": [{"text": "  "}]}]},
              {"id": 3, "parent": 2, "position": [0, 0], "sections": [{"spans": [{"text": "grandchild"}]}]},
              {"id": 4, "parent": 1, "position": [0, 0], "sections": [{"spans": [{"text": "child"}]}]},
              {"id": 5, "position": [0, 0], "sections": [{"spans": [{"text": "Other"}]}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            markdown(&doc),
            "# Root\nmore\nsecond\n\n## grandchild\n\n## child\n\n# Other\n\n"
        );
    }
}
