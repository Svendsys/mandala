// SPDX-License-Identifier: MPL-2.0

//! A document as deep as memory allows (design D30, §20 rule 11;
//! V1-A5, #282): a single chain of [`DEEP`] nodes survives every walk
//! the engine and the document make over it — a full walk, a
//! `RepeatWhile` descent to the leaf, an anchored walk at the leaf, a
//! journaled edit there reverted and replayed, a move of the whole
//! chain under a new parent, `validate_subtree` and an insert of a
//! subtree as deep, and the inherited state and the style resolution
//! at the leaf — on a default test thread, and through the wasm32 node
//! probe (`tests/wasm_font_lock.rs`, TEST_CONVENTIONS §T9) on wasm32's
//! default stack in a debug build.
//!
//! Every edit goes through `Document::run_steps`, the path the
//! writer runs a request through, and is held to
//! [`validate_applied`]; a revert and a replay run on a copy of the
//! tree, as the composite builders' tests do.

use crate::document::element::ConnectorData;
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::inherited::inherited_seed;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp, SpanOp, TextOp};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::mutation::validate::validate_applied;
use crate::document::resolve::{resolve_text, Input};
use crate::document::shape::ShapeError;
use crate::document::tests::test_support::{node_data, path_data, DocumentBuilder};
use crate::document::values::FontSize;
use crate::engine::anchor::Anchors;
use crate::engine::journal::{Journal, JournalBuilder};
use crate::engine::key::BranchKey;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::{Placement, Tree};
use indextree::NodeId;

/// How deep the chains go: §20 rule 11's chain.
pub const DEEP: usize = 100_000;

/// A document holding a chain of [`DEEP`] nodes below a root node, with
/// the deepest content at the leaf.
struct Chain {
    doc: Document,
    /// The root node the chain hangs from.
    holder: NodeId,
    /// A second root node, with nothing below it.
    other: NodeId,
    /// The chain, from the top down; each node has its connector.
    nodes: Vec<NodeId>,
    /// The span in the leaf's section.
    leaf_span: NodeId,
}

impl Chain {
    fn leaf(&self) -> NodeId {
        self.nodes[DEEP - 1]
    }
}

/// Build the [`Chain`]: the leaf sets its text to 20 pt and holds a
/// section with a span that sets nothing. Built by appending, so
/// building it climbs nothing.
fn chain() -> Chain {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let holder = builder.node(root);
    let other = builder.node(root);
    let mut at = holder;
    let mut nodes = Vec::with_capacity(DEEP);
    for _ in 0..DEEP {
        at = builder.node(at);
        nodes.push(at);
    }
    builder.edit_node(at, |node| node.style.text.size = Some(FontSize::Pt(20.0)));
    let section = builder.section(at, 0);
    let leaf_span = builder.span(section, "the deepest text");
    Chain {
        doc: builder.build().unwrap(),
        holder,
        other,
        nodes,
        leaf_span,
    }
}

/// Run `walks` as one request on `doc` and hold it to
/// [`validate_applied`]; returns the closed journal.
fn run(
    doc: &mut Document,
    walks: Vec<(DocMutatorTree, Anchors)>,
) -> Result<Journal<DocElement>, MutationError> {
    let steps = walks
        .into_iter()
        .map(|(mutator, anchors)| DocStep::fixed(DocWalk::new(mutator, anchors)))
        .collect();
    let builder = doc.run_steps(steps, JournalBuilder::new())?;
    let journal = builder.close(doc.tree());
    validate_applied(doc.tree(), &journal)?;
    Ok(journal)
}

/// A mutator of one payload on `kind`, at the root.
fn at_root(kind: DocKind, mutation: DocMutation) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)))
}

/// Whether the node at `id` is hidden.
fn hidden(doc: &Document, id: NodeId) -> bool {
    doc.get(id)
        .and_then(DocElement::as_node)
        .is_some_and(|node| node.hidden)
}

/// The text size the element at `id` sets itself.
fn own_size(doc: &Document, id: NodeId) -> Option<FontSize> {
    doc.get(id)
        .and_then(DocElement::text_style)
        .and_then(|style| style.size)
}

/// Revert `journal` on a copy of `doc`'s tree and check `before` holds
/// there, then replay it and check `after` does.
fn revert_and_replay(
    doc: &Document,
    journal: &Journal<DocElement>,
    before: impl Fn(&Tree<DocElement>) -> bool,
    after: impl Fn(&Tree<DocElement>) -> bool,
) {
    let mut tree = doc.tree().clone();
    journal.revert(&mut tree).unwrap();
    assert!(before(&tree), "the revert restores the document");
    journal.replay(&mut tree).unwrap();
    assert!(after(&tree), "the replay restores the edit");
}

#[test]
fn test_document_deep_chain_full_walk() {
    do_document_deep_chain_full_walk();
}

/// A mutator as deep as the chain, from the map down every node, hides
/// the leaf alone: the walk aligns 100,000 levels by key.
pub fn do_document_deep_chain_full_walk() {
    let mut chain = chain();
    let mut mutator = DocMutatorTree::new_with(Mutator::new_void(BranchKey::new(DocKind::Map, 0)));
    let mut at = mutator.root;
    for _ in 0..DEEP {
        at = mutator
            .insert_child(
                at,
                Mutator::new_void(BranchKey::new(DocKind::Node, 0)),
                Placement::Last,
            )
            .unwrap();
    }
    mutator
        .insert_child(
            at,
            Mutator::new(
                DocMutation::Node(NodeOp::SetHidden(true)),
                BranchKey::new(DocKind::Node, 0),
            ),
            Placement::Last,
        )
        .unwrap();
    let root = chain.doc.root();
    run(&mut chain.doc, vec![(mutator, Anchors::Nodes(vec![root]))]).unwrap();
    assert!(hidden(&chain.doc, chain.leaf()), "the leaf is reached");
    assert!(!hidden(&chain.doc, chain.nodes[DEEP - 2]), "and nothing above it");
    assert!(!hidden(&chain.doc, chain.holder));
}

#[test]
fn test_document_deep_chain_repeat_while_to_the_leaf() {
    do_document_deep_chain_repeat_while_to_the_leaf();
}

/// A `RepeatWhile` from the map hides every node, the leaf 100,000
/// levels down included — kind-routed at every level (R2): the
/// connectors, sections and spans beside each node are passed.
pub fn do_document_deep_chain_repeat_while_to_the_leaf() {
    let mut chain = chain();
    let mut mutator = DocMutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(Predicate::always_true()),
        BranchKey::new(DocKind::Map, 0),
    ));
    let root = mutator.root;
    mutator
        .insert_child(
            root,
            Mutator::new(
                DocMutation::Node(NodeOp::SetHidden(true)),
                BranchKey::new(DocKind::Node, 0),
            ),
            Placement::Last,
        )
        .unwrap();
    let map = chain.doc.root();
    run(&mut chain.doc, vec![(mutator, Anchors::Nodes(vec![map]))]).unwrap();
    assert!(hidden(&chain.doc, chain.leaf()), "the leaf is reached");
    assert!(
        chain.nodes.iter().all(|&id| hidden(&chain.doc, id)),
        "every level is"
    );
    assert!(hidden(&chain.doc, chain.holder) && hidden(&chain.doc, chain.other));
}

#[test]
fn test_document_deep_chain_journaled_edit_at_the_leaf() {
    do_document_deep_chain_journaled_edit_at_the_leaf();
}

/// Anchored walks at the leaf: one hides the leaf node, one grows its
/// span, whose own size is unset, from the 20 pt it inherits — the
/// inherited state the seed folds at the leaf. One request, one
/// journal, reverted and replayed exactly.
pub fn do_document_deep_chain_journaled_edit_at_the_leaf() {
    let mut chain = chain();
    let (leaf, span) = (chain.leaf(), chain.leaf_span);
    let journal = run(
        &mut chain.doc,
        vec![
            (
                at_root(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(true))),
                Anchors::Nodes(vec![leaf]),
            ),
            (
                at_root(
                    DocKind::Span,
                    DocMutation::Span(SpanOp::Text(TextOp::GrowSize(2.0))),
                ),
                Anchors::Nodes(vec![span]),
            ),
        ],
    )
    .unwrap();
    assert!(hidden(&chain.doc, leaf));
    assert_eq!(
        own_size(&chain.doc, span),
        Some(FontSize::Pt(22.0)),
        "20 pt inherited, grown by 2"
    );
    let size = |tree: &Tree<DocElement>| {
        tree.get(span)
            .and_then(|n| n.get().text_style())
            .and_then(|s| s.size)
    };
    let is_hidden = |tree: &Tree<DocElement>| {
        tree.get(leaf)
            .and_then(|n| n.get().as_node())
            .is_some_and(|node| node.hidden)
    };
    revert_and_replay(
        &chain.doc,
        &journal,
        |tree| !is_hidden(tree) && size(tree).is_none(),
        |tree| is_hidden(tree) && size(tree) == Some(FontSize::Pt(22.0)),
    );
}

#[test]
fn test_document_deep_chain_moves_whole() {
    do_document_deep_chain_moves_whole();
}

/// The whole chain — its top, with its connector — moves under the
/// other root node, kept valid, then reverted and replayed.
pub fn do_document_deep_chain_moves_whole() {
    let mut chain = chain();
    let (top, other, holder) = (chain.nodes[0], chain.other, chain.holder);
    let moving = DocMutatorTree::new_with(Mutator::new_structure(
        StructOp::Move {
            to: other,
            at: Placement::Last,
        },
        BranchKey::new(DocKind::Node, 0),
    ));
    let journal = run(&mut chain.doc, vec![(moving, Anchors::Nodes(vec![top]))]).unwrap();
    assert_eq!(chain.doc.parent(top), Some(other), "moved");
    assert_eq!(
        chain.doc.depth(chain.leaf()),
        DEEP,
        "the leaf sits below the other root"
    );
    revert_and_replay(
        &chain.doc,
        &journal,
        |tree| tree.get(top).and_then(|n| n.parent()) == Some(holder),
        |tree| tree.get(top).and_then(|n| n.parent()) == Some(other),
    );
}

/// A node subtree [`DEEP`] levels deep, each node with its connector
/// and the next node, built from the bottom up so building it recurses
/// on nothing.
pub fn deep_node_subtree() -> Subtree<DocElement> {
    let connector = || Subtree::leaf(DocElement::Connector(ConnectorData { path: path_data() }));
    let mut subtree = Subtree {
        root: DocElement::Node(node_data(0)),
        children: vec![connector()],
    };
    for _ in 1..DEEP {
        subtree = Subtree {
            root: DocElement::Node(node_data(0)),
            children: vec![connector(), subtree],
        };
    }
    subtree
}

#[test]
fn test_document_deep_subtree_validates_and_inserts() {
    do_document_deep_subtree_validates_and_inserts();
}

/// `validate_subtree` takes a subtree 100,000 levels deep, and finds a
/// broken row at its bottom by its full path; the subtree is cloned,
/// compared and dropped on this thread; and inserted below the leaf —
/// 200,000 levels in all — validated, reverted and replayed.
pub fn do_document_deep_subtree_validates_and_inserts() {
    let subtree = deep_node_subtree();
    assert_eq!(Document::validate_subtree(&subtree), Ok(()));
    let mut broken = subtree.clone();
    assert!(broken == subtree, "a clone is equal");
    let mut bottom = &mut broken;
    while bottom.children.len() > 1 {
        bottom = &mut bottom.children[1];
    }
    bottom
        .children
        .push(Subtree::leaf(DocElement::Node(node_data(0))));
    assert!(broken != subtree, "a change at the bottom is a difference");
    match Document::validate_subtree(&broken) {
        Err(ShapeError::SubtreeRow { path, .. }) => {
            assert_eq!(
                path.len(),
                DEEP,
                "the path runs to the node added at the bottom, which lacks its connector"
            );
        }
        other => panic!("expected the bottom row refused, got {other:?}"),
    }
    drop(broken);

    let mut chain = chain();
    let leaf = chain.leaf();
    let insert = DocMutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree,
            at: Placement::Last,
        },
        BranchKey::new(DocKind::Node, 0),
    ));
    let journal = run(&mut chain.doc, vec![(insert, Anchors::Nodes(vec![leaf]))]).unwrap();
    let inserted = chain
        .doc
        .children(leaf)
        .find(|&id| chain.doc.get(id).map(DocElement::kind) == Some(DocKind::Node))
        .expect("the subtree is below the leaf");
    assert_eq!(
        journal.touched().count(),
        2 * DEEP,
        "every inserted node and connector is touched"
    );
    revert_and_replay(
        &chain.doc,
        &journal,
        |tree| tree.get(inserted).and_then(|n| n.parent()).is_none(),
        |tree| tree.get(inserted).and_then(|n| n.parent()) == Some(leaf),
    );
}

#[test]
fn test_document_deep_chain_resolves_at_the_leaf() {
    do_document_deep_chain_resolves_at_the_leaf();
}

/// The inherited state a walk anchored at the leaf's span starts from,
/// and the span's resolved text: 20 pt from the leaf node, folded
/// within the node and never climbing above it; the palette tier, with
/// no binding anywhere, climbs once from the leaf to the map — one
/// read per node on the way, never more.
pub fn do_document_deep_chain_resolves_at_the_leaf() {
    let chain = chain();
    let tree = chain.doc.tree();
    let seed = inherited_seed(tree, chain.leaf_span);
    assert_eq!(seed.text_size_for(DocKind::Span), FontSize::Pt(20.0));
    let mut reads: Vec<Input> = Vec::new();
    let text = resolve_text(tree, chain.leaf_span, &mut reads);
    assert_eq!(text.size, FontSize::Pt(20.0));
    let bindings = reads
        .iter()
        .filter(|input| matches!(input, Input::PaletteBinding(_)))
        .count();
    assert_eq!(
        bindings,
        DEEP + 1,
        "the leaf's binding walk reads each node above it once"
    );
}
