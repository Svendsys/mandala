// SPDX-License-Identifier: MPL-2.0

//! What is selected, and the moves through the document tree the
//! keyboard makes from it (design §5.2: selection holds `NodeId`s).
//!
//! A [`Selection`] is a set of document elements in the order they were
//! selected: nodes, and the paths between them — a node's connector, a
//! link. It names elements by their arena `NodeId`, never by a string,
//! and nothing here reads a glyph layer: a click is resolved by the
//! projector's hit test (`Projector::hit`) and lands here as the element
//! it names ([`selectable`]), and the keyboard's moves follow the
//! document's own tree.
//!
//! The selection is app state, not document state: it is never saved
//! and never in the history. An undo that removes a selected element
//! leaves an id that no longer reads anything, which
//! [`Selection::retain_live`] drops.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::projection::Hit;

/// The selected elements, oldest first, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    items: Vec<NodeId>,
}

impl Selection {
    /// Nothing selected. No allocation.
    pub fn new() -> Self {
        Selection::default()
    }

    /// The selected elements, oldest first. No allocation.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.items.iter().copied()
    }

    /// Whether `id` is selected. O(the selection).
    pub fn contains(&self, id: NodeId) -> bool {
        self.items.contains(&id)
    }

    /// The element selected first, if any. O(1).
    pub fn first(&self) -> Option<NodeId> {
        self.items.first().copied()
    }

    /// The selected nodes of `doc`, oldest first: the elements that are
    /// nodes, the paths left out. No allocation.
    pub fn nodes<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = NodeId> + 'a {
        self.items
            .iter()
            .copied()
            .filter(move |&id| kind_of(doc, id) == Some(DocKind::Node))
    }

    /// Select `id` alone. Returns whether the selection changed.
    pub fn select_only(&mut self, id: NodeId) -> bool {
        if self.items.len() == 1 && self.items[0] == id {
            return false;
        }
        self.items.clear();
        self.items.push(id);
        true
    }

    /// Add `id` when it is not selected, take it out when it is: what a
    /// Shift-click does. O(the selection).
    pub fn toggle(&mut self, id: NodeId) {
        match self.items.iter().position(|&s| s == id) {
            Some(at) => {
                self.items.remove(at);
            }
            None => self.items.push(id),
        }
    }

    /// Select exactly `ids`, in their order, each once. Returns whether
    /// the selection changed. O(ids × the selection).
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) -> bool {
        let mut next: Vec<NodeId> = Vec::new();
        for id in ids {
            if !next.contains(&id) {
                next.push(id);
            }
        }
        if next == self.items {
            return false;
        }
        self.items = next;
        true
    }

    /// Select nothing. Returns whether anything was selected.
    pub fn clear(&mut self) -> bool {
        let had = !self.items.is_empty();
        self.items.clear();
        had
    }

    /// Drop every element `doc` no longer holds — what an undo or a redo
    /// that removed it leaves behind. Returns whether any was dropped.
    /// O(the selection).
    pub fn retain_live(&mut self, doc: &Document) -> bool {
        let before = self.items.len();
        self.items.retain(|&id| doc.get(id).is_some());
        self.items.len() != before
    }
}

/// The kind of `id` in `doc`, or `None` for an id `doc` does not hold.
/// O(1).
pub fn kind_of(doc: &Document, id: NodeId) -> Option<DocKind> {
    doc.get(id).map(|element| element.kind())
}

/// The element a click on `hit` selects: the path a glyph of a
/// connector, a link, a cap or a label belongs to — a portal end's
/// link — and otherwise the node. Sections, frames and rings select
/// their node: editing inside a node is V1-C8's (#249) NodeEdit mode.
/// O(the depth from the element hit to its path, at most three).
pub fn selectable(doc: &Document, hit: &Hit) -> NodeId {
    let mut at = Some(hit.element);
    while let Some(id) = at {
        match kind_of(doc, id) {
            Some(DocKind::Connector | DocKind::Link) => return id,
            Some(DocKind::Node) | None => break,
            Some(_) => at = doc.parent(id),
        }
    }
    hit.node
}

/// The node `node` hangs from, when it is not a top-level node. O(1).
pub fn parent_node(doc: &Document, node: NodeId) -> Option<NodeId> {
    doc.parent(node)
        .filter(|&parent| kind_of(doc, parent) == Some(DocKind::Node))
}

/// The child nodes of `parent` — a node, or the map's root for the
/// top-level nodes — in sibling order. No allocation.
pub fn child_nodes(doc: &Document, parent: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(parent)
        .filter(move |&child| kind_of(doc, child) == Some(DocKind::Node))
}

/// The first child node of `node`, in sibling order. O(its children).
pub fn first_child_node(doc: &Document, node: NodeId) -> Option<NodeId> {
    child_nodes(doc, node).next()
}

/// The sibling node after (`forward`) or before `node` among the nodes
/// that share its parent — the top-level nodes for a top-level node —
/// wrapping at either end. `None` when it has no sibling. O(its
/// siblings).
pub fn sibling_node(doc: &Document, node: NodeId, forward: bool) -> Option<NodeId> {
    let parent = doc.parent(node)?;
    let row: Vec<NodeId> = child_nodes(doc, parent).collect();
    if row.len() < 2 {
        return None;
    }
    let at = row.iter().position(|&n| n == node)?;
    let next = if forward {
        (at + 1) % row.len()
    } else {
        (at + row.len() - 1) % row.len()
    };
    Some(row[next])
}

/// Every node of `doc`, in pre-order. One walk of the document, iterative;
/// no allocation beyond the walk's own.
pub fn every_node(doc: &Document) -> impl Iterator<Item = NodeId> + '_ {
    let tree = doc.tree();
    tree.root()
        .descendants(tree.arena())
        .filter(move |&id| kind_of(doc, id) == Some(DocKind::Node))
}
