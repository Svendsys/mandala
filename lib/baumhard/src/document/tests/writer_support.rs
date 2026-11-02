// SPDX-License-Identifier: MPL-2.0

//! What the writer's tests share: requests built in one line, the saved
//! text a document compares by, and the oracle every step of every
//! writer test is held to — the indexes the writer keeps, computed again
//! from scratch, and the document still savable.

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind};
use crate::document::format::{check_savable, save};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::tests::test_support::{example, Example};
use crate::document::writer::{Derived, EditRequest};
use crate::engine::anchor::Anchors;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use glam::Vec2;
use rustc_hash::FxHashSet;

/// The test builder's example map, built.
pub(crate) fn built() -> (Document, Example) {
    let (builder, ids) = example();
    (builder.build().unwrap(), ids)
}

/// The document as a `.mandala.json`: what two states are compared by.
pub(crate) fn saved(doc: &Document) -> String {
    save(doc).unwrap()
}

/// A mutator of one payload on `kind`, at the root.
pub(crate) fn single(kind: DocKind, mutation: DocMutation) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)))
}

/// The one-step request of `mutator` at `anchors`.
pub(crate) fn request(mutator: DocMutatorTree, anchors: Vec<NodeId>, label: &str) -> EditRequest {
    EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(anchors)))],
        label,
    )
}

/// The request that puts every node of `nodes` at `at`.
pub(crate) fn place(nodes: Vec<NodeId>, at: Vec2) -> EditRequest {
    request(
        single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(at))),
        nodes,
        "move",
    )
}

/// The request that sets the opacity of every node of `nodes`.
pub(crate) fn fade(nodes: Vec<NodeId>, opacity: f32) -> EditRequest {
    request(
        single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(opacity))),
        nodes,
        "fade",
    )
}

/// Where the node at `id` is.
pub(crate) fn position(doc: &Document, id: NodeId) -> Option<Vec2> {
    doc.get(id)
        .and_then(DocElement::as_node)
        .map(|node| node.position)
}

/// The ident of the node at `id`.
pub(crate) fn ident(doc: &Document, id: NodeId) -> Option<NodeIdent> {
    doc.get(id).and_then(DocElement::as_node).map(|node| node.ident)
}

/// How many arena slots of the document are live, in the tree or held
/// out of it by a journal: a slot a discard frees stops counting.
pub(crate) fn live_slots(doc: &Document) -> usize {
    doc.tree().arena().live_count()
}

/// The children of `id` of `kind`, in sibling order.
pub(crate) fn children_of(doc: &Document, id: NodeId, kind: DocKind) -> Vec<NodeId> {
    doc.children(id)
        .filter(|&c| doc.get(c).map(DocElement::kind) == Some(kind))
        .collect()
}

/// Every `(target, link)` pair of an incoming-link index.
fn pairs(derived: &Derived) -> FxHashSet<(NodeId, NodeId)> {
    derived
        .incoming
        .iter()
        .flat_map(|(&to, links)| links.iter().map(move |&link| (to, link)))
        .collect()
}

/// Hold `doc` to what the writer must keep true after every change,
/// each computed again from the tree, which shares no code with the
/// writer's upkeep: the links into each node and the budget
/// ([`Derived::scan`]); every node in the tree carrying an assigned
/// ident that the ident index leads back to, and the index holding
/// nothing else; the rows sorted; and the document savable
/// ([`check_savable`]), so that what loaded can still be written and
/// load again as itself.
pub(crate) fn assert_consistent(doc: &Document, at: &str) {
    let scan = Derived::scan(doc.tree(), doc.definitions());
    assert_eq!(
        pairs(doc.derived()),
        pairs(&scan),
        "{at}: the incoming-link index is the tree's"
    );
    assert!(
        doc.derived().incoming.values().all(|links| !links.is_empty()),
        "{at}: no node keeps an empty entry"
    );
    assert_eq!(
        doc.derived().budget,
        scan.budget,
        "{at}: the budget is the tree's"
    );
    let mut nodes = 0;
    for id in doc.tree().descendants() {
        if let Some(node) = doc.get(id).and_then(DocElement::as_node) {
            nodes += 1;
            assert_ne!(node.ident, NodeIdent::UNASSIGNED, "{at}: node {id} has an ident");
            assert_eq!(
                doc.node_by_ident(node.ident),
                Some(id),
                "{at}: the ident index leads to node {id}"
            );
        }
    }
    assert_eq!(
        doc.ident_count(),
        nodes,
        "{at}: the ident index holds the tree's nodes alone"
    );
    doc.tree().check_sorted().unwrap();
    check_savable(doc).unwrap_or_else(|error| panic!("{at}: the document stays savable: {error}"));
}
