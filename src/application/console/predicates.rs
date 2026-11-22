// SPDX-License-Identifier: MPL-2.0

//! Applicability predicates: the "is this command relevant to the
//! current selection" checks.
//!
//! `help` filters the visible command list through them, and
//! completion hides commands whose predicate returns false.
//!
//! They're kept in one place so the predicate vocabulary is scannable
//! — if you're adding a new command and need "a path is selected", the
//! helper already exists here.
//!
//! The vocabulary is deliberately *selection-shaped*: the selection
//! holds nodes and paths (connectors and links,
//! `document::selection`), and each predicate asks which of those it
//! holds — a link's kind included, since a portal link and a line link
//! take different verbs — never what they hold.

use super::ConsoleContext;
use crate::application::document::selection::kind_of;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::values::LinkKind;

pub fn always(_: &ConsoleContext) -> bool {
    true
}

/// A node or a section is selected: what the `section` and `range`
/// verbs reach (`fields::section::targets`). Paths selected beside them
/// do not count.
pub fn node_or_section_selected(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    ctx.document
        .selection
        .iter()
        .any(|id| matches!(kind_of(doc, id), Some(DocKind::Node | DocKind::Section)))
}

/// The kind of link `id` is, or `None` when it is not a link.
fn link_kind(doc: &baumhard::document::ids::Document, id: baumhard::arena::NodeId) -> Option<LinkKind> {
    match doc.get(id) {
        Some(DocElement::Link(link)) => Some(link.kind),
        _ => None,
    }
}

/// A line path — a connector, or a line link — is selected: what `path`
/// and `label` reach. A portal link draws no path, and its labels are
/// its ends' (`crate::application::fields::path`), so it does not count.
pub fn line_path_selected(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    ctx.document
        .selection
        .iter()
        .any(|id| kind_of(doc, id) == Some(DocKind::Connector) || link_kind(doc, id) == Some(LinkKind::Line))
}

/// A link is selected, of either kind: what `link` reaches. A connector
/// does not count, being always a line (D22).
pub fn link_selected(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    ctx.document
        .selection
        .iter()
        .any(|id| kind_of(doc, id) == Some(DocKind::Link))
}

/// A portal link is selected: what `portal` reaches, one of its ends.
pub fn portal_selected(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    ctx.document
        .selection
        .iter()
        .any(|id| link_kind(doc, id) == Some(LinkKind::Portal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::selection::every_node;
    use crate::application::document::MindMapDocument;

    /// A map with two nodes, the second a child of the first, so it has
    /// a connector.
    fn two_nodes() -> MindMapDocument {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let parent = every_node(doc.doc()).next().expect("the root just made");
        let steps = baumhard::document::mutation::compose::create_child(
            doc.doc(),
            parent,
            glam::Vec2::new(0.0, 80.0),
        )
        .expect("a child composes");
        doc.apply_for_test(baumhard::document::writer::EditRequest {
            steps,
            label: "child".to_string(),
        })
        .expect("the child applies");
        doc
    }

    /// `section` wants a node or a section: nothing and a connector
    /// alone are refused; a node, two nodes and a section are admitted.
    #[test]
    fn test_the_section_predicate_admits_nodes_and_sections() {
        let mut doc = two_nodes();
        let nodes: Vec<_> = every_node(doc.doc()).collect();
        assert_eq!(nodes.len(), 2);
        let admits = |doc: &MindMapDocument| node_or_section_selected(&ConsoleContext::from_document(doc));
        assert!(!admits(&doc), "nothing selected");
        doc.selection.select_only(nodes[0]);
        assert!(admits(&doc));
        doc.selection.set(nodes.iter().copied());
        assert!(admits(&doc), "two nodes");
        let section = doc
            .doc()
            .children(nodes[0])
            .find(|&c| kind_of(doc.doc(), c) == Some(DocKind::Section))
            .expect("a node has a section");
        doc.selection.select_only(section);
        assert!(admits(&doc), "a section");
        let connector = doc
            .doc()
            .children(nodes[1])
            .find(|&c| kind_of(doc.doc(), c) == Some(DocKind::Connector))
            .expect("a child has a connector");
        doc.selection.select_only(connector);
        assert!(!admits(&doc), "a connector alone");
    }

    /// The path verbs want a line path: a node alone is not one, the
    /// child's connector is.
    #[test]
    fn test_the_line_path_predicate_admits_a_connector_and_not_a_node() {
        let mut doc = two_nodes();
        let child = every_node(doc.doc()).nth(1).expect("the child");
        let connector = doc
            .doc()
            .children(child)
            .find(|&c| kind_of(doc.doc(), c) == Some(DocKind::Connector))
            .expect("a child has a connector");
        doc.selection.select_only(child);
        assert!(!line_path_selected(&ConsoleContext::from_document(&doc)));
        doc.selection.select_only(connector);
        assert!(line_path_selected(&ConsoleContext::from_document(&doc)));
    }
}
