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
//! holds, never what they hold.

use super::ConsoleContext;
use crate::application::document::selection::kind_of;
use baumhard::document::element::DocKind;

pub fn always(_: &ConsoleContext) -> bool {
    true
}

/// Exactly one node is selected: the `section` verb's target, whose
/// subverbs each resolve to one section of one node. Paths selected
/// beside it do not count.
pub fn node_or_section_selected_single_node(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    let mut nodes = ctx.document.selection.nodes(doc);
    nodes.next().is_some() && nodes.next().is_none()
}

/// A path — a connector or a link — is selected: the target of the
/// path verbs (`anchor`, `body`, `cap`, `spacing`, `label`).
pub fn path_selected(ctx: &ConsoleContext) -> bool {
    let doc = ctx.document.doc();
    ctx.document
        .selection
        .iter()
        .any(|id| matches!(kind_of(doc, id), Some(DocKind::Connector | DocKind::Link)))
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
        let steps =
            baumhard::document::mutation::compose::create_child(doc.doc(), parent, glam::Vec2::new(0.0, 80.0))
                .expect("a child composes");
        doc.apply_for_test(baumhard::document::writer::EditRequest {
            steps,
            label: "child".to_string(),
        })
        .expect("the child applies");
        doc
    }

    /// `section` wants exactly one node: none and two are refused, one
    /// is admitted.
    #[test]
    fn test_the_section_predicate_admits_exactly_one_node() {
        let mut doc = two_nodes();
        let nodes: Vec<_> = every_node(doc.doc()).collect();
        assert_eq!(nodes.len(), 2);
        assert!(!node_or_section_selected_single_node(&ConsoleContext::from_document(&doc)));
        doc.selection.select_only(nodes[0]);
        assert!(node_or_section_selected_single_node(&ConsoleContext::from_document(&doc)));
        doc.selection.set(nodes.iter().copied());
        assert!(!node_or_section_selected_single_node(&ConsoleContext::from_document(&doc)));
    }

    /// The path verbs want a path: a node alone is not one, the child's
    /// connector is.
    #[test]
    fn test_the_path_predicate_admits_a_connector_and_not_a_node() {
        let mut doc = two_nodes();
        let child = every_node(doc.doc()).nth(1).expect("the child");
        let connector = doc
            .doc()
            .children(child)
            .find(|&c| kind_of(doc.doc(), c) == Some(DocKind::Connector))
            .expect("a child has a connector");
        doc.selection.select_only(child);
        assert!(!path_selected(&ConsoleContext::from_document(&doc)));
        doc.selection.select_only(connector);
        assert!(path_selected(&ConsoleContext::from_document(&doc)));
    }
}
