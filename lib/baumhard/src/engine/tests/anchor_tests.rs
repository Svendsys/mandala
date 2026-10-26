// SPDX-License-Identifier: MPL-2.0

//! Anchored walks over [`TestElement`]:
//! [`MutatorTree::apply_at`] starts at each anchor instead of the root.
//! Anchors resolve to concrete nodes ([`Anchors::ParentOf`],
//! [`Anchors::SiblingsOf`]), nested anchors are dropped when the
//! mutator reaches below its root, the root applies by kind alone,
//! every seed is taken before any anchor is walked, and a stale anchor
//! applies nothing.
//!
//! [`TestElement`]'s inherited state makes seeds observable: each
//! element hands its children `parent + value`, so the inherited state
//! of a node's parent is the seed plus the values of all its ancestors
//! (`ancestors_sum`), which [`TestMutation::SetToInherited`] writes
//! into the node. Every expected value is a literal.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use crate::engine::anchor::Anchors;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::tests::test_element::{
    add, add_mutator, marked, plain, row, value, TestElement, TestMutation, TestMutator,
};
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use crate::engine::walker::walk_tree;
use indextree::NodeId;
use std::cell::Cell;

/// The seed the anchored walks start from, apart from every fixture
/// value.
const SEED: i64 = 1000;

/// The inherited state of `id`'s parent as `seed_for` must give it:
/// [`SEED`] plus the value of every strict ancestor of `id`. Computed
/// by climbing the arena, not by the walker.
fn ancestors_sum(tree: &Tree<TestElement>, id: NodeId) -> i64 {
    SEED + id
        .ancestors(tree.arena())
        .skip(1)
        .map(|a| value(tree, a))
        .sum::<i64>()
}

/// A mutator tree that is `root` alone.
fn only(root: TestMutator) -> MutatorTree<TestElement, TestMutation> {
    MutatorTree::new_with(root)
}

/// A root (value 0) holding `parent` (10), holding `child` (100).
fn chain() -> (Tree<TestElement>, NodeId, NodeId) {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let parent = add(&mut tree, root, TestElement::new(0, 10));
    let child = add(&mut tree, parent, TestElement::new(0, 100));
    (tree, parent, child)
}

// ── nested anchors ────────────────────────────────────────────────

#[test]
fn test_apply_at_drops_anchors_nested_in_another() {
    do_apply_at_drops_anchors_nested_in_another();
}

/// With `[child, parent]` as anchors, a mutator that reaches below its
/// root — `Add(1)` at the anchor and `Add(1)` on its children — reaches
/// the child once, through the parent's walk: the child's own anchor is
/// dropped. Listing the child first makes no difference; anchors walk
/// in tree order. The control is a mutator that touches only its root:
/// it keeps both anchors, and each gets its `Add(1)` exactly once.
pub fn do_apply_at_drops_anchors_nested_in_another() {
    let (mut tree, parent, child) = chain();
    let mut subtree = only(Mutator::new(TestMutation::Add(1), plain(0)));
    let mroot = subtree.root;
    add_mutator(&mut subtree, mroot, Mutator::new(TestMutation::Add(1), plain(0)));
    assert!(
        subtree.reaches_below_root(),
        "precondition: the mutator reaches below its root"
    );

    subtree
        .apply_at(&mut tree, &Anchors::Nodes(vec![child, parent]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, parent), 11);
    assert_eq!(value(&tree, child), 101, "the child is reached exactly once");

    let (mut tree, parent, child) = chain();
    let root_only = only(Mutator::new(TestMutation::Add(1), plain(0)));
    assert!(
        !root_only.reaches_below_root(),
        "precondition: the control touches only its root"
    );
    root_only
        .apply_at(&mut tree, &Anchors::Nodes(vec![child, parent]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, parent), 11);
    assert_eq!(value(&tree, child), 101, "the control: the nested anchor is kept");
}

#[test]
fn test_apply_at_walks_a_repeated_anchor_once() {
    do_apply_at_walks_a_repeated_anchor_once();
}

/// An anchor named twice is one anchor, even for a mutator that keeps
/// nested anchors.
pub fn do_apply_at_walks_a_repeated_anchor_once() {
    let (mut tree, parent, child) = chain();
    only(Mutator::new(TestMutation::Add(1), plain(0)))
        .apply_at(&mut tree, &Anchors::Nodes(vec![child, child]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, child), 101);
    assert_eq!(value(&tree, parent), 10);
}

#[test]
fn test_reaches_below_root_reads_the_shape() {
    do_reaches_below_root_reads_the_shape();
}

/// A mutator reaches below its root when its root is an `Instruction`
/// or anything below the root is not a `Void`; a root over nothing but
/// `Void`s does not.
pub fn do_reaches_below_root_reads_the_shape() {
    let mut voids = only(Mutator::new(TestMutation::Add(1), plain(0)));
    let mroot = voids.root;
    let v = add_mutator(&mut voids, mroot, Mutator::new_void(plain(0)));
    add_mutator(&mut voids, v, Mutator::new_void(plain(3)));
    assert!(!voids.reaches_below_root(), "Voids below the root change nothing");

    let mut deep = voids.clone();
    add_mutator(&mut deep, v, Mutator::new(TestMutation::Add(1), plain(1)));
    assert!(deep.reaches_below_root(), "a payload two levels down");

    let instruction = only(Mutator::new_instruction(
        Instruction::RepeatWhile(Predicate::always_true()),
        plain(0),
    ));
    assert!(instruction.reaches_below_root(), "an Instruction root");
}

// ── the root at an anchor ─────────────────────────────────────────

#[test]
fn test_apply_at_matches_the_root_by_kind_alone() {
    do_apply_at_matches_the_root_by_kind_alone();
}

/// At an anchor, the mutator's root applies when the kinds match,
/// whatever the channels — here a `(Marked, 5)` root on a
/// `(Marked, 0)` anchor — and the walk continues below. An anchor of
/// another kind is skipped whole: a `(Plain, 0)` root reaches neither
/// the `Marked` anchor nor the `Plain` child below it.
pub fn do_apply_at_matches_the_root_by_kind_alone() {
    let fixture = || {
        let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
        let root = tree.root;
        let anchor = add(&mut tree, root, TestElement::marked(0, 1));
        let below = add(&mut tree, anchor, TestElement::new(0, 2));
        (tree, anchor, below)
    };
    let with_root = |root: TestMutator| {
        let mut mt = only(root);
        let mroot = mt.root;
        add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(100), plain(0)));
        mt
    };

    let (mut tree, anchor, below) = fixture();
    with_root(Mutator::new(TestMutation::Add(10), marked(5)))
        .apply_at(&mut tree, &Anchors::Nodes(vec![anchor]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, anchor), 11, "the channel is not compared");
    assert_eq!(value(&tree, below), 102, "and the walk continues below");

    let (mut tree, anchor, below) = fixture();
    with_root(Mutator::new(TestMutation::Add(10), plain(0)))
        .apply_at(&mut tree, &Anchors::Nodes(vec![anchor]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, anchor), 1, "another kind is not reached");
    assert_eq!(value(&tree, below), 2, "nor is anything below it");
}

#[test]
fn test_apply_to_anchors_at_the_root() {
    do_apply_to_anchors_at_the_root();
}

/// `apply_to` is `apply_at` anchored at the tree's root: a mutator root
/// on channel 3 applies to a channel-0 root of its kind. A walk from
/// the two roots as a pair (`walk_tree`) compares the whole key, and
/// does not.
pub fn do_apply_to_anchors_at_the_root() {
    let mt = only(Mutator::new(TestMutation::Add(1), plain(3)));
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 5));
    mt.apply_to(&mut tree, &0);
    assert_eq!(value(&tree, tree.root), 6);

    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 5));
    walk_tree(&mut tree, &mt, &0);
    assert_eq!(
        value(&tree, tree.root),
        5,
        "the control: a pair walk compares channels"
    );
}

// ── relative anchors ──────────────────────────────────────────────

/// A root (0) with `a` (Plain 0, 1) — holding `a1` (Plain 0, 10), `a2`
/// (Plain 1, 20) and `am` (Marked 0, 30) — and `b` (Plain 1, 2),
/// holding its only child `b1` (Plain 0, 40).
struct Family {
    tree: Tree<TestElement>,
    a: NodeId,
    a1: NodeId,
    a2: NodeId,
    am: NodeId,
    b: NodeId,
    b1: NodeId,
}

fn family() -> Family {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let a = add(&mut tree, root, TestElement::new(0, 1));
    let a1 = add(&mut tree, a, TestElement::new(0, 10));
    let a2 = add(&mut tree, a, TestElement::new(1, 20));
    let am = add(&mut tree, a, TestElement::marked(0, 30));
    let b = add(&mut tree, root, TestElement::new(1, 2));
    let b1 = add(&mut tree, b, TestElement::new(0, 40));
    Family {
        tree,
        a,
        a1,
        a2,
        am,
        b,
        b1,
    }
}

/// `Add(1000)` at each anchor and nothing below — so the values say
/// exactly which nodes the anchors resolved to.
fn mark_anchors() -> MutatorTree<TestElement, TestMutation> {
    only(Mutator::new(TestMutation::Add(1000), plain(0)))
}

#[test]
fn test_parent_of_resolves_to_the_distinct_parents() {
    do_parent_of_resolves_to_the_distinct_parents();
}

/// `ParentOf` resolves to the distinct parents: two children of `a`
/// and the child of `b` make `a` and `b`, each once. The root has no
/// parent and contributes nothing.
pub fn do_parent_of_resolves_to_the_distinct_parents() {
    let mut f = family();
    let root = f.tree.root;
    mark_anchors()
        .apply_at(
            &mut f.tree,
            &Anchors::ParentOf(vec![f.a1, f.a2, f.b1]),
            ancestors_sum,
        )
        .unwrap();
    assert_eq!(value(&f.tree, f.a), 1001);
    assert_eq!(value(&f.tree, f.b), 1002);
    assert_eq!(value(&f.tree, root), 0);
    assert_eq!(
        row(&f.tree, f.a),
        vec![10, 20, 30],
        "the given nodes are not anchors"
    );

    let mut f = family();
    mark_anchors()
        .apply_at(&mut f.tree, &Anchors::ParentOf(vec![root]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&f.tree, root), 0, "the root has no parent");
    assert_eq!(row(&f.tree, root), vec![1, 2]);
}

#[test]
fn test_siblings_of_resolves_to_same_kind_siblings() {
    do_siblings_of_resolves_to_same_kind_siblings();
}

/// `SiblingsOf` resolves to each node's siblings of its own kind,
/// excluding the node: `a1`'s are `a2` but not the `Marked` `am`, and
/// the `Marked` `am` has none — its `Plain` siblings, which the `Plain`
/// mutator would reach, are not its kind. An only child (`b1`) and the
/// root have none either.
pub fn do_siblings_of_resolves_to_same_kind_siblings() {
    let mut f = family();
    mark_anchors()
        .apply_at(&mut f.tree, &Anchors::SiblingsOf(vec![f.a1]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&f.tree, f.a2), 1020);
    assert_eq!(value(&f.tree, f.a1), 10, "the node itself is excluded");
    assert_eq!(value(&f.tree, f.am), 30, "a sibling of another kind is excluded");

    let mut f = family();
    mark_anchors()
        .apply_at(&mut f.tree, &Anchors::SiblingsOf(vec![f.am]), ancestors_sum)
        .unwrap();
    assert_eq!(
        row(&f.tree, f.a),
        vec![10, 20, 30],
        "the Marked node's siblings of another kind are not anchors"
    );

    let mut f = family();
    let root = f.tree.root;
    mark_anchors()
        .apply_at(&mut f.tree, &Anchors::SiblingsOf(vec![f.b1, root]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&f.tree, f.b1), 40, "an only child has no siblings");
    assert_eq!(row(&f.tree, root), vec![1, 2], "and neither has the root");
    assert_eq!(value(&f.tree, root), 0);
}

// ── kind before nesting ───────────────────────────────────────────

#[test]
fn test_apply_at_decides_nesting_among_anchors_of_the_root_kind() {
    do_apply_at_decides_nesting_among_anchors_of_the_root_kind();
}

/// With `[a, b]` as anchors — `a` of kind `Marked`, `b` a `Plain`
/// child of it — a reaching mutator rooted at `Plain` does not apply at
/// `a`, so `a` cannot stand in for `b`: `b` stays an anchor and is
/// walked, its child with it, whichever order the anchors are listed
/// in. Only `b` is seeded. The contrast: with `a` of the root's kind,
/// `b` is dropped and reached once through `a`.
pub fn do_apply_at_decides_nesting_among_anchors_of_the_root_kind() {
    let fixture = |a_element: TestElement| {
        let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
        let root = tree.root;
        let a = add(&mut tree, root, a_element);
        let b = add(&mut tree, a, TestElement::new(0, 10));
        let below = add(&mut tree, b, TestElement::new(0, 100));
        (tree, a, b, below)
    };
    let mut reaching = only(Mutator::new(TestMutation::Add(1), plain(0)));
    let mroot = reaching.root;
    add_mutator(&mut reaching, mroot, Mutator::new(TestMutation::Add(1), plain(0)));

    for listed_b_first in [false, true] {
        let (mut tree, a, b, below) = fixture(TestElement::marked(0, 1));
        let anchors = if listed_b_first { vec![b, a] } else { vec![a, b] };
        let seeded = Cell::new(0);
        let seed_for = |tree: &Tree<TestElement>, id: NodeId| {
            seeded.set(seeded.get() + 1);
            ancestors_sum(tree, id)
        };
        reaching
            .apply_at(&mut tree, &Anchors::Nodes(anchors), seed_for)
            .unwrap();
        assert_eq!(value(&tree, a), 1, "the Marked anchor is not the root's kind");
        assert_eq!(
            value(&tree, b),
            11,
            "b is walked though a lists above it ({listed_b_first})"
        );
        assert_eq!(value(&tree, below), 101, "and the walk continues below b");
        assert_eq!(seeded.get(), 1, "only the anchor walked is seeded");
    }

    let (mut tree, a, b, below) = fixture(TestElement::new(0, 1));
    reaching
        .apply_at(&mut tree, &Anchors::Nodes(vec![a, b]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, a), 2, "the contrast: a of the root's kind is walked");
    assert_eq!(value(&tree, b), 11, "and b is reached once, through a");
    assert_eq!(value(&tree, below), 100);
}

// ── resolution ────────────────────────────────────────────────────

#[test]
fn test_anchors_are_resolved_once_and_walked_in_tree_order() {
    do_anchors_are_resolved_once_and_walked_in_tree_order();
}

/// Anchors are seeded — and walked — in tree order, whatever order they
/// are named in: siblings of different keys by key, siblings of one key
/// by their place in the row, an ancestor before its subtree. A node
/// named by several of the given nodes is one anchor: `SiblingsOf`
/// three siblings names each of them twice, and each is seeded once.
pub fn do_anchors_are_resolved_once_and_walked_in_tree_order() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let high = add(&mut tree, root, TestElement::new(1, 1));
    let first = add(&mut tree, root, TestElement::new(0, 2));
    let second = add(&mut tree, root, TestElement::new(0, 3));
    let inner = add(&mut tree, first, TestElement::new(0, 4));
    assert_eq!(
        row(&tree, root),
        vec![2, 3, 1],
        "precondition: the row is Plain 0 (first), Plain 0 (second), Plain 1 (high)"
    );

    let order = std::cell::RefCell::new(Vec::new());
    let seed_for = |tree: &Tree<TestElement>, id: NodeId| {
        order.borrow_mut().push(id);
        ancestors_sum(tree, id)
    };
    let root_only = only(Mutator::new(TestMutation::Add(1000), plain(0)));
    root_only
        .apply_at(
            &mut tree,
            &Anchors::Nodes(vec![high, inner, second, first, high]),
            seed_for,
        )
        .unwrap();
    assert_eq!(*order.borrow(), vec![first, inner, second, high]);
    assert_eq!(value(&tree, high), 1001, "named twice, walked once");

    order.borrow_mut().clear();
    root_only
        .apply_at(
            &mut tree,
            &Anchors::SiblingsOf(vec![high, first, second]),
            seed_for,
        )
        .unwrap();
    assert_eq!(
        *order.borrow(),
        vec![first, second, high],
        "each sibling once, in tree order"
    );
    assert_eq!(row(&tree, root), vec![2002, 2003, 2001]);
}

// ── seeds ─────────────────────────────────────────────────────────

#[test]
fn test_apply_at_seeds_every_anchor_before_walking_any() {
    do_apply_at_seeds_every_anchor_before_walking_any();
}

/// `SetToInherited` at each anchor writes what `seed_for` gave it — the
/// anchor's parent's state: `SEED` plus its ancestors. With `parent`
/// and `child` both anchors, `parent`'s walk changes `child`'s
/// ancestor before `child` is walked, yet `child` gets the sum over the
/// tree as it was: `SEED + 0 + 10`, not `SEED + 0 + 1000`. `seed_for`
/// runs once per anchor.
pub fn do_apply_at_seeds_every_anchor_before_walking_any() {
    let (mut tree, parent, child) = chain();
    let calls = Cell::new(0);
    let seed_for = |tree: &Tree<TestElement>, id: NodeId| {
        calls.set(calls.get() + 1);
        ancestors_sum(tree, id)
    };
    only(Mutator::new(TestMutation::SetToInherited, plain(0)))
        .apply_at(&mut tree, &Anchors::Nodes(vec![parent, child]), seed_for)
        .unwrap();
    assert_eq!(value(&tree, parent), 1000, "SEED + root 0");
    assert_eq!(
        value(&tree, child),
        1010,
        "SEED + root 0 + the parent's original 10"
    );
    assert_eq!(calls.get(), 2);
}

// ── stale anchors ─────────────────────────────────────────────────

#[test]
fn test_a_stale_anchor_applies_nothing() {
    do_a_stale_anchor_applies_nothing();
}

/// An anchor that is no longer in the tree — removed, and its slot
/// reused by a new node, so only indextree's stamp tells them apart —
/// is [`TreeError::StaleNode`], and nothing is applied, not even at the
/// live anchor listed before it; `seed_for` is not called. Each anchor
/// form refuses it. The control walks the live anchor alone.
pub fn do_a_stale_anchor_applies_nothing() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let live = add(&mut tree, root, TestElement::new(0, 10));
    let gone = add(&mut tree, root, TestElement::new(1, 20));
    tree.remove_subtree(gone).unwrap();
    let reuse = add(&mut tree, root, TestElement::new(1, 30));
    assert!(
        tree.get(gone).is_none() && tree.get(reuse).is_some(),
        "precondition: the old id is stale while its slot lives on"
    );

    let mt = only(Mutator::new(TestMutation::Add(1), plain(0)));
    let seeded = Cell::new(false);
    let seed_for = |tree: &Tree<TestElement>, id: NodeId| {
        seeded.set(true);
        ancestors_sum(tree, id)
    };
    for anchors in [
        Anchors::Nodes(vec![live, gone]),
        Anchors::ParentOf(vec![live, gone]),
        Anchors::SiblingsOf(vec![live, gone]),
    ] {
        assert_eq!(
            mt.apply_at(&mut tree, &anchors, seed_for),
            Err(TreeError::StaleNode(gone)),
            "{anchors:?}"
        );
    }
    assert_eq!(value(&tree, live), 10, "nothing is applied");
    assert_eq!(value(&tree, root), 0);
    assert!(!seeded.get(), "no anchor is seeded");

    mt.apply_at(&mut tree, &Anchors::Nodes(vec![live]), ancestors_sum)
        .unwrap();
    assert_eq!(value(&tree, live), 11, "the control: the live anchor alone");
}

// ── keys changed at an anchor ─────────────────────────────────────

#[test]
fn test_apply_at_moves_an_anchor_whose_key_it_changed() {
    do_apply_at_moves_an_anchor_whose_key_it_changed();
}

/// A root mutation that changes the anchor's channel moves the anchor
/// among its siblings once the walk is over, as any walk does.
pub fn do_apply_at_moves_an_anchor_whose_key_it_changed() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let first = add(&mut tree, root, TestElement::new(0, 1));
    add(&mut tree, root, TestElement::new(1, 2));
    add(&mut tree, root, TestElement::new(2, 3));
    only(Mutator::new(TestMutation::SetChannel(1), plain(0)))
        .apply_at(&mut tree, &Anchors::Nodes(vec![first]), ancestors_sum)
        .unwrap();
    assert_eq!(row(&tree, root), vec![2, 1, 3]);
    assert_eq!(tree.check_sorted(), Ok(()));
}
