// SPDX-License-Identifier: MPL-2.0

//! Selection apply_* helpers: what a click or a tap selects, and the
//! selection Actions (`SelectAll`, `DeselectAll`, `InvertSelection`,
//! `SelectParent`, `SelectChild`, `SelectNextSibling`,
//! `SelectPrevSibling`).
//!
//! The keyboard's moves follow the document's tree
//! (`document::selection`'s `parent_node`, `first_child_node`,
//! `sibling_node`); "every node" means every node drawn, which is what
//! the projector holds a rectangle for. Each `*_in` function is the pure
//! half, over the document and the view, and returns whether the
//! selection changed; its `apply_*` sibling syncs the view when it did.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;

use crate::application::app::ClickHit;
use crate::application::document::selection::{
    every_node, first_child_node, kind_of, parent_node, sibling_node, Selection,
};
use crate::application::document::MindMapDocument;
use crate::application::view::MapView;

use super::ViewContext;

/// Update `selection` for a click on `hit`: a click on an element selects
/// it alone, a Shift-click adds it or takes it out; a click on empty
/// canvas selects nothing, and a Shift-click there changes nothing.
/// Returns whether the selection changed.
pub(in crate::application::app) fn click_selection(
    selection: &mut Selection,
    hit: ClickHit,
    shift: bool,
) -> bool {
    match (hit, shift) {
        (ClickHit::Element(id), false) => selection.select_only(id),
        (ClickHit::Element(id), true) => {
            selection.toggle(id);
            true
        }
        (ClickHit::Empty, false) => selection.clear(),
        (ClickHit::Empty, true) => false,
    }
}

/// The node a keyboard move starts from: the first selected node, or the
/// node a selected path belongs to.
fn anchor_node(doc: &MindMapDocument) -> Option<NodeId> {
    let first = doc.selection.first()?;
    match kind_of(doc.doc(), first)? {
        DocKind::Node => Some(first),
        _ => doc.doc().owning_node(first),
    }
}

/// Every drawn node, in document order.
fn drawn_nodes<'a>(doc: &'a MindMapDocument, view: &'a MapView) -> impl Iterator<Item = NodeId> + 'a {
    every_node(doc.doc()).filter(move |&node| view.projector().node_rect(node).is_some())
}

/// Select every drawn node. O(the document).
pub(in crate::application::app) fn select_all_in(doc: &mut MindMapDocument, view: &MapView) -> bool {
    let all: Vec<NodeId> = drawn_nodes(doc, view).collect();
    doc.selection.set(all)
}

/// Select nothing.
pub(in crate::application::app) fn deselect_all_in(doc: &mut MindMapDocument) -> bool {
    doc.selection.clear()
}

/// Select every drawn node that is not selected, and nothing else.
/// O(the document × the selection).
pub(in crate::application::app) fn invert_selection_in(doc: &mut MindMapDocument, view: &MapView) -> bool {
    let inverted: Vec<NodeId> = drawn_nodes(doc, view)
        .filter(|&node| !doc.selection.contains(node))
        .collect();
    doc.selection.set(inverted)
}

/// Select the parent of the anchor node; nothing for a top-level node.
pub(in crate::application::app) fn select_parent_in(doc: &mut MindMapDocument) -> bool {
    match anchor_node(doc).and_then(|node| parent_node(doc.doc(), node)) {
        Some(parent) => doc.selection.select_only(parent),
        None => false,
    }
}

/// Select the anchor node's first child node; nothing for a leaf.
pub(in crate::application::app) fn select_child_in(doc: &mut MindMapDocument) -> bool {
    match anchor_node(doc).and_then(|node| first_child_node(doc.doc(), node)) {
        Some(child) => doc.selection.select_only(child),
        None => false,
    }
}

/// Select the anchor node's next (`forward`) or previous sibling node,
/// wrapping; nothing for an only child.
pub(in crate::application::app) fn select_sibling_in(doc: &mut MindMapDocument, forward: bool) -> bool {
    match anchor_node(doc).and_then(|node| sibling_node(doc.doc(), node, forward)) {
        Some(sibling) => doc.selection.select_only(sibling),
        None => false,
    }
}

/// Which selection Action an arm runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum SelectionMove {
    All,
    None,
    Invert,
    Parent,
    Child,
    NextSibling,
    PrevSibling,
}

/// Run `step` and sync the view when the selection changed. Returns
/// whether it did.
pub(in crate::application::app) fn apply_selection_move(
    step: SelectionMove,
    vc: &mut ViewContext<'_>,
) -> bool {
    let changed = match step {
        SelectionMove::All => select_all_in(vc.document, vc.view),
        SelectionMove::None => deselect_all_in(vc.document),
        SelectionMove::Invert => invert_selection_in(vc.document, vc.view),
        SelectionMove::Parent => select_parent_in(vc.document),
        SelectionMove::Child => select_child_in(vc.document),
        SelectionMove::NextSibling => select_sibling_in(vc.document, true),
        SelectionMove::PrevSibling => select_sibling_in(vc.document, false),
    };
    if changed {
        vc.sync();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::testament;

    /// The tree read the slow way, as an oracle: a node's parent is the
    /// node whose children include it.
    fn oracle_parent(doc: &MindMapDocument, node: NodeId) -> Option<NodeId> {
        every_node(doc.doc()).find(|&candidate| doc.doc().children(candidate).any(|child| child == node))
    }

    /// A node's row: every node, in pre-order, that shares its parent.
    fn oracle_row(doc: &MindMapDocument, node: NodeId) -> Vec<NodeId> {
        let parent = doc.doc().parent(node);
        every_node(doc.doc())
            .filter(|&other| doc.doc().parent(other) == parent)
            .collect()
    }

    /// **The keyboard's moves follow the document's tree**, checked
    /// over every node of the testament map against the tree read the
    /// slow way: `SelectParent` selects the node it hangs from,
    /// `SelectChild` its first child in sibling order, and the sibling
    /// moves the next and previous node of its row, wrapping.
    ///
    /// Controls: a top-level node has no parent and a leaf no child, and
    /// a move that has nowhere to go reports no change and leaves the
    /// selection where it was; the map has both, and nodes with each
    /// move, so no branch of the check is vacuous.
    #[test]
    fn test_the_keyboard_moves_follow_the_document_tree() {
        let mut doc = testament();
        let nodes: Vec<NodeId> = every_node(doc.doc()).collect();
        let (mut parents, mut children, mut rows, mut top_level, mut leaves) = (0, 0, 0, 0, 0);
        for &node in &nodes {
            doc.selection.select_only(node);
            let parent = oracle_parent(&doc, node);
            assert_eq!(select_parent_in(&mut doc), parent.is_some(), "parent of {node:?}");
            match parent {
                Some(parent) => {
                    parents += 1;
                    assert_eq!(doc.selection.first(), Some(parent), "parent of {node:?}");
                }
                None => {
                    top_level += 1;
                    assert_eq!(
                        doc.selection.first(),
                        Some(node),
                        "a top-level node stays selected"
                    );
                }
            }

            doc.selection.select_only(node);
            let child = every_node(doc.doc()).find(|&other| doc.doc().parent(other) == Some(node));
            assert_eq!(select_child_in(&mut doc), child.is_some(), "child of {node:?}");
            match child {
                Some(child) => {
                    children += 1;
                    assert_eq!(doc.selection.first(), Some(child), "first child of {node:?}");
                }
                None => {
                    leaves += 1;
                    assert_eq!(doc.selection.first(), Some(node), "a leaf stays selected");
                }
            }

            let row = oracle_row(&doc, node);
            let at = row
                .iter()
                .position(|&n| n == node)
                .expect("a node is in its own row");
            for (forward, expected) in [
                (true, row[(at + 1) % row.len()]),
                (false, row[(at + row.len() - 1) % row.len()]),
            ] {
                doc.selection.select_only(node);
                let moved = select_sibling_in(&mut doc, forward);
                if row.len() < 2 {
                    assert!(!moved, "an only child has no sibling");
                    assert_eq!(doc.selection.first(), Some(node));
                } else {
                    rows += 1;
                    assert!(moved, "sibling of {node:?}");
                    assert_eq!(doc.selection.first(), Some(expected), "sibling of {node:?}");
                }
            }
        }
        assert!(
            parents > 0 && children > 0 && rows > 0 && top_level > 0 && leaves > 0,
            "control: every branch was exercised (parents {parents}, children {children}, \
             sibling moves {rows}, top-level {top_level}, leaves {leaves})"
        );
    }
}
