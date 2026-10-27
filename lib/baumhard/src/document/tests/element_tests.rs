// SPDX-License-Identifier: MPL-2.0

//! The document as an engine instantiation: the kind order that lays
//! out every row, the routing it gives the walker, and the root rule a
//! document mutator meets.

use crate::document::element::{DocElement, DocKind};
use crate::document::mutation::inherited::inherited_seed;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp, SectionOp};
use crate::document::tests::test_support::{example, DocumentBuilder};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tree::{Placement, Tree};
use indextree::NodeId;

/// The kinds of `parent`'s children, in sibling order.
fn row(tree: &Tree<DocElement>, parent: NodeId) -> Vec<DocKind> {
    parent
        .children(tree.arena())
        .map(|c| tree.arena()[c].get().kind())
        .collect()
}

#[test]
fn test_document_kind_order_is_the_declared_order() {
    do_document_kind_order_is_the_declared_order();
}

/// `DocKind`'s derived `Ord` is #235's declaration order, which is
/// what lays out every row.
pub fn do_document_kind_order_is_the_declared_order() {
    use DocKind::*;
    let declared = [
        Map, Theme, Palette, Defaults, Section, Span, Frame, Ring, Connector, CapStart, CapEnd, Label, Link,
        PortalEnd, Node,
    ];
    for pair in declared.windows(2) {
        assert!(pair[0] < pair[1], "{:?} must sort before {:?}", pair[0], pair[1]);
    }
    assert_eq!(DocKind::ALL, declared);
    assert_eq!(DocKind::default(), Map);
}

#[test]
fn test_document_rows_are_laid_out_by_kind() {
    do_document_rows_are_laid_out_by_kind();
}

/// Children added in any order sit in kind order: a node's sections,
/// frame, connector, links and child nodes; a path's spans before its
/// caps and label; the map's theme, palettes and defaults before its
/// nodes.
pub fn do_document_rows_are_laid_out_by_kind() {
    let (builder, ids) = example();
    let tree = builder.tree();
    tree.check_sorted().unwrap();
    use DocKind::*;
    assert_eq!(
        row(tree, builder.root()),
        vec![Theme, Palette, Defaults, Node, Node]
    );
    assert_eq!(row(tree, ids.a), vec![Section, Section, Frame, Link, Link, Node]);
    assert_eq!(row(tree, ids.b), vec![Section, Connector, Node]);
    assert_eq!(row(tree, ids.b_connector), vec![Span, CapStart, CapEnd, Label]);
    assert_eq!(row(tree, ids.portal_source), vec![Span, Label]);

    // A section added after a child node lands among the sections, and
    // a span added after a label lands before it.
    let (mut builder, ids) = example();
    let section = builder.section(ids.b, 1);
    assert_eq!(
        row(builder.tree(), ids.b),
        vec![Section, Section, Connector, Node]
    );
    assert_eq!(ids.b.children(builder.tree().arena()).nth(1), Some(section));
    let span_after = builder.span(ids.portal_source, "added after the label");
    assert_eq!(row(builder.tree(), ids.portal_source), vec![Span, Span, Label]);
    assert_eq!(
        ids.portal_source.children(builder.tree().arena()).nth(1),
        Some(span_after)
    );
}

/// The opacity of the node at `id`.
fn opacity(tree: &Tree<DocElement>, id: NodeId) -> f32 {
    tree.arena()[id].get().as_node().unwrap().opacity
}

#[test]
fn test_document_walk_routes_by_kind() {
    do_document_walk_routes_by_kind();
}

/// A mutator addressed to `(Node, 0)` under the map reaches both root
/// nodes on channel 0 and nothing of another kind on channel 0 — the
/// theme, the palette, the defaults. Its root is `MutatorTree::new()`'s
/// `Void`, of the default kind, `Map`, which is the document root's
/// kind, so `apply_to` from the root walks at all.
pub fn do_document_walk_routes_by_kind() {
    let (builder, ids) = example();
    let mut tree = builder.tree().clone();
    let before = tree.clone();
    let mut mutators = DocMutatorTree::new();
    let root = mutators.root;
    mutators
        .insert_child(
            root,
            Mutator::new(
                DocMutation::Node(NodeOp::SetOpacity(0.25)),
                BranchKey::new(DocKind::Node, 0),
            ),
            Placement::Last,
        )
        .unwrap();
    let seed = inherited_seed(&tree, tree.root());
    mutators.apply_to(&mut tree, &seed);
    assert_eq!(opacity(&tree, ids.a), 0.25);
    assert_eq!(opacity(&tree, ids.d), 0.25);
    assert_eq!(opacity(&tree, ids.b), 1.0, "a child node is a row below");
    for id in tree.root().children(tree.arena()) {
        if tree.arena()[id].get().kind() != DocKind::Node {
            assert_eq!(tree.arena()[id].get(), before.arena()[id].get());
        }
    }

    // Control: the same mutator under a root of another kind reaches
    // nothing, because a kind mismatch at the root prunes the walk.
    let mut tree = builder.tree().clone();
    let mut mutators = DocMutatorTree::new_with(Mutator::new_void(BranchKey::new(DocKind::Node, 0)));
    let root = mutators.root;
    mutators
        .insert_child(
            root,
            Mutator::new(
                DocMutation::Node(NodeOp::SetOpacity(0.25)),
                BranchKey::new(DocKind::Node, 0),
            ),
            Placement::Last,
        )
        .unwrap();
    let seed = inherited_seed(&tree, tree.root());
    mutators.apply_to(&mut tree, &seed);
    assert_eq!(opacity(&tree, ids.a), 1.0);
}

#[test]
fn test_document_anchored_walk_reaches_the_selection() {
    do_document_anchored_walk_reaches_the_selection();
}

/// A node-rooted mutator anchored at a selection applies at the
/// selected nodes and nowhere else; a section in the selection is not a
/// node, so it is not an anchor.
pub fn do_document_anchored_walk_reaches_the_selection() {
    let (builder, ids) = example();
    let mut tree = builder.tree().clone();
    let mutators = DocMutatorTree::new_with(Mutator::new(
        DocMutation::Node(NodeOp::SetOpacity(0.5)),
        BranchKey::new(DocKind::Node, 0),
    ));
    mutators
        .apply_at(
            &mut tree,
            &Anchors::Nodes(vec![ids.b, ids.d, ids.a_title]),
            inherited_seed,
        )
        .unwrap();
    assert_eq!(opacity(&tree, ids.b), 0.5);
    assert_eq!(opacity(&tree, ids.d), 0.5);
    assert_eq!(opacity(&tree, ids.a), 1.0);
    assert_eq!(opacity(&tree, ids.c), 1.0);
    assert_eq!(tree.arena()[ids.a_title].get().channel(), 0);
}

#[test]
fn test_document_walk_moves_a_rekeyed_element() {
    do_document_walk_moves_a_rekeyed_element();
}

/// A section whose channel a walk changes moves to its place in the
/// row once the walk ends: the sorted-children invariant holds for the
/// document as for any element type.
pub fn do_document_walk_moves_a_rekeyed_element() {
    let (builder, ids) = example();
    let mut tree = builder.tree().clone();
    let mutators = DocMutatorTree::new_with(Mutator::new(
        DocMutation::Section(SectionOp::SetChannel(5)),
        BranchKey::new(DocKind::Section, 0),
    ));
    mutators
        .apply_at(&mut tree, &Anchors::Nodes(vec![ids.a_title]), inherited_seed)
        .unwrap();
    tree.check_sorted().unwrap();
    let sections: Vec<NodeId> = ids.a.children(tree.arena()).take(2).collect();
    assert_eq!(sections, vec![ids.a_body, ids.a_title]);
}

#[test]
fn test_document_element_keys() {
    do_document_element_keys();
}

/// An element's key: its kind, and its stored channel for a node, a
/// section and a ring, its end for a portal end, and 0 for the rest;
/// the void element is an unnamed map.
pub fn do_document_element_keys() {
    let (mut builder, ids) = example();
    let section = builder.section(ids.d, 7);
    builder.edit_node(ids.d, |n| n.channel = 3);
    let tree = builder.tree();
    let key = |id: NodeId| tree.arena()[id].get().key();
    assert_eq!(key(section), BranchKey::new(DocKind::Section, 7));
    assert_eq!(key(ids.d), BranchKey::new(DocKind::Node, 3));
    assert_eq!(key(ids.portal_target), BranchKey::new(DocKind::PortalEnd, 1));
    assert_eq!(key(ids.portal_source), BranchKey::new(DocKind::PortalEnd, 0));
    assert_eq!(key(ids.b_label), BranchKey::new(DocKind::Label, 0));
    assert_eq!(DocElement::void().key(), BranchKey::new(DocKind::Map, 0));
    const { assert!(!DocElement::SPATIAL) };
    let _ = DocumentBuilder::new();
}
