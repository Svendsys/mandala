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
