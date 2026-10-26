// SPDX-License-Identifier: MPL-2.0

//! The sorted-children invariant over [`TestElement`]: every parent's
//! children are in ascending key order, equal keys in the order they
//! were placed in. Each test exercises one API that can change a row —
//! [`Tree::insert_child`] at every [`Placement`], [`Tree::update`] that
//! changes a key, [`Tree::remove_subtree`], and a walk whose mutation
//! changes a channel — and asserts [`Tree::check_sorted`] after it,
//! beside the exact row the API should leave. The check itself is
//! shown failing on a row built out of order behind the API's back.
//!
//! Rows are read as the children's values in sibling order
//! ([`row`]); every expected row is a literal.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use crate::engine::mutator::Mutator;
use crate::engine::tests::test_element::{
    add, add_mutator, plain, row, value, TestElement, TestKind, TestMutation,
};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use crate::engine::walker::walk_tree;
use std::cell::Cell;

/// An empty tree whose root is `(Plain, 0)` with value 0.
fn empty() -> Tree<TestElement> {
    Tree::new_non_indexed_with(TestElement::new(0, 0))
}

// ── insert_child ──────────────────────────────────────────────────

#[test]
fn test_insert_child_places_within_the_run_of_its_key() {
    do_insert_child_places_within_the_run_of_its_key();
}

/// The key picks the run, the placement picks the spot in it. `Last`
/// ends the run and `First` starts it — including a run that does not
/// exist yet, which opens between the keys around it — and
/// `Before` / `After` sit next to the named equal-key sibling. Keys
/// order kind first: `(Plain, 0) < (Plain, 1) < (Marked, 0)`.
pub fn do_insert_child_places_within_the_run_of_its_key() {
    let mut tree = empty();
    let root = tree.root;
    let insert = |tree: &mut Tree<TestElement>, element: TestElement, at: Placement| {
        let id = tree.insert_child(root, element, at).unwrap();
        assert_eq!(tree.check_sorted(), Ok(()), "sorted after inserting at {at:?}");
        id
    };
    let p1 = insert(&mut tree, TestElement::new(1, 1), Placement::Last);
    insert(&mut tree, TestElement::marked(0, 2), Placement::Last);
    insert(&mut tree, TestElement::new(0, 3), Placement::Last);
    assert_eq!(
        row(&tree, root),
        vec![3, 1, 2],
        "Last lands by key, not at the end"
    );
    insert(&mut tree, TestElement::new(1, 4), Placement::Last);
    insert(&mut tree, TestElement::new(1, 5), Placement::First);
    insert(&mut tree, TestElement::new(1, 6), Placement::Before(p1));
    insert(&mut tree, TestElement::new(1, 7), Placement::After(p1));
    assert_eq!(row(&tree, root), vec![3, 5, 6, 1, 7, 4, 2]);
    insert(&mut tree, TestElement::new(2, 8), Placement::First);
    insert(&mut tree, TestElement::marked(1, 9), Placement::First);
    assert_eq!(
        row(&tree, root),
        vec![3, 5, 6, 1, 7, 4, 8, 2, 9],
        "a new run opens between the keys around it, whichever end is asked for"
    );
}

#[test]
fn test_insert_child_refuses_a_placement_across_keys() {
    do_insert_child_refuses_a_placement_across_keys();
}

/// `Before` / `After` naming a sibling with a different key is
/// [`TreeError::PlacementCrossesKey`] — a different channel, or the
/// same channel of a different kind — and inserts nothing. The control
/// places an equal key next to the same sibling. A sibling under
/// another parent is [`TreeError::NotAChild`]; a parent no longer in
/// the tree is [`TreeError::StaleNode`].
pub fn do_insert_child_refuses_a_placement_across_keys() {
    let mut tree = empty();
    let root = tree.root;
    let p1 = add(&mut tree, root, TestElement::new(1, 1));
    let m0 = add(&mut tree, root, TestElement::marked(0, 2));

    assert_eq!(
        tree.insert_child(root, TestElement::new(0, 9), Placement::Before(p1)),
        Err(TreeError::PlacementCrossesKey),
        "another channel"
    );
    assert_eq!(
        tree.insert_child(root, TestElement::marked(1, 9), Placement::After(p1)),
        Err(TreeError::PlacementCrossesKey),
        "the same channel of another kind"
    );
    assert_eq!(
        tree.insert_child(root, TestElement::new(0, 9), Placement::After(m0)),
        Err(TreeError::PlacementCrossesKey),
        "the same channel of another kind, the other way round"
    );
    assert_eq!(row(&tree, root), vec![1, 2], "a refused insert inserts nothing");

    tree.insert_child(root, TestElement::new(1, 3), Placement::Before(p1))
        .expect("the control: an equal key may sit next to it");
    assert_eq!(row(&tree, root), vec![3, 1, 2]);

    let below = add(&mut tree, p1, TestElement::new(1, 4));
    assert_eq!(
        tree.insert_child(root, TestElement::new(1, 9), Placement::After(below)),
        Err(TreeError::NotAChild {
            parent: root,
            node: below
        })
    );
    tree.remove_subtree(below).unwrap();
    assert_eq!(
        tree.insert_child(below, TestElement::new(0, 9), Placement::Last),
        Err(TreeError::StaleNode(below))
    );
    assert_eq!(row(&tree, root), vec![3, 1, 2]);
    assert_eq!(tree.check_sorted(), Ok(()));
}

// ── update ────────────────────────────────────────────────────────

#[test]
fn test_update_moves_an_element_whose_key_changed() {
    do_update_moves_an_element_whose_key_changed();
}

/// `update` returns what its closure returns. When the closure changes
/// the key, the element moves to the end of the run of its new key —
/// forward past keys it now outranks, backward past keys that now
/// outrank it, across kinds too — keeping its id; when it does not,
/// the element stays where it is.
pub fn do_update_moves_an_element_whose_key_changed() {
    let mut tree = empty();
    let root = tree.root;
    let a = add(&mut tree, root, TestElement::new(0, 1));
    add(&mut tree, root, TestElement::new(1, 2));
    let c = add(&mut tree, root, TestElement::new(1, 3));
    let d = add(&mut tree, root, TestElement::new(2, 4));
    let e = add(&mut tree, root, TestElement::marked(0, 5));
    assert_eq!(row(&tree, root), vec![1, 2, 3, 4, 5]);

    tree.update(a, |el| el.channel = 1).unwrap();
    assert_eq!(
        row(&tree, root),
        vec![2, 3, 1, 4, 5],
        "forward, to the end of its new run"
    );
    assert_eq!(tree.check_sorted(), Ok(()));

    tree.update(d, |el| el.channel = 0).unwrap();
    assert_eq!(
        row(&tree, root),
        vec![4, 2, 3, 1, 5],
        "backward, before every key above it"
    );
    assert_eq!(tree.check_sorted(), Ok(()));

    tree.update(e, |el| {
        el.kind = TestKind::Plain;
        el.channel = 1;
    })
    .unwrap();
    assert_eq!(
        row(&tree, root),
        vec![4, 2, 3, 1, 5],
        "Marked 0 to Plain 1 ends the Plain 1 run"
    );
    tree.update(e, |el| el.channel = 0).unwrap();
    assert_eq!(row(&tree, root), vec![4, 5, 2, 3, 1]);
    assert_eq!(tree.check_sorted(), Ok(()));

    let returned = tree.update(c, |el| {
        el.value += 10;
        el.value
    });
    assert_eq!(returned, Ok(13));
    assert_eq!(row(&tree, root), vec![4, 5, 2, 13, 1], "the same key stays put");
}

#[test]
fn test_update_of_a_stale_node_calls_nothing() {
    do_update_of_a_stale_node_calls_nothing();
}

/// `update` on a removed node is [`TreeError::StaleNode`], and its
/// closure is not called. The control updates a live node.
pub fn do_update_of_a_stale_node_calls_nothing() {
    let mut tree = empty();
    let root = tree.root;
    let gone = add(&mut tree, root, TestElement::new(0, 1));
    let kept = add(&mut tree, root, TestElement::new(0, 2));
    tree.remove_subtree(gone).unwrap();
    let called = Cell::new(false);
    assert_eq!(
        tree.update(gone, |_| called.set(true)),
        Err(TreeError::StaleNode(gone))
    );
    assert!(!called.get(), "the closure must not run on a stale node");
    tree.update(kept, |_| called.set(true)).unwrap();
    assert!(called.get(), "the control: a live node runs it");
}

// ── remove_subtree ────────────────────────────────────────────────

#[test]
fn test_remove_subtree_frees_the_subtree_and_keeps_the_row() {
    do_remove_subtree_frees_the_subtree_and_keeps_the_row();
}

/// Removing a child frees it and its descendants — their ids go stale —
/// and leaves its siblings in order. The root cannot be removed, and a
/// node already removed is stale.
pub fn do_remove_subtree_frees_the_subtree_and_keeps_the_row() {
    let mut tree = empty();
    let root = tree.root;
    let a = add(&mut tree, root, TestElement::new(0, 1));
    let a_child = add(&mut tree, a, TestElement::new(0, 10));
    add(&mut tree, root, TestElement::new(1, 2));
    add(&mut tree, root, TestElement::new(2, 3));

    assert_eq!(tree.remove_subtree(a), Ok(()));
    assert!(
        tree.get(a).is_none() && tree.get(a_child).is_none(),
        "the whole subtree is freed"
    );
    assert_eq!(row(&tree, root), vec![2, 3]);
    assert_eq!(tree.check_sorted(), Ok(()));
    assert_eq!(tree.remove_subtree(a), Err(TreeError::StaleNode(a)));
    assert_eq!(tree.remove_subtree(root), Err(TreeError::RemoveRoot));
    assert_eq!(row(&tree, root), vec![2, 3]);
}

// ── a walk that changes keys ──────────────────────────────────────

/// Four `Plain` children on channels 0..=3, values 10, 20, 30, 40.
fn four() -> Tree<TestElement> {
    let mut tree = empty();
    let root = tree.root;
    for (channel, v) in [(0, 10), (1, 20), (2, 30), (3, 40)] {
        add(&mut tree, root, TestElement::new(channel, v));
    }
    tree
}

#[test]
fn test_walk_moves_the_children_whose_key_it_changed() {
    do_walk_moves_the_children_whose_key_it_changed();
}

/// A walk that moves channel 0 to 2 and channel 2 to 0 leaves the row
/// sorted, by moving exactly those two: each goes to the end of the run
/// of its new key among the children that kept theirs, in the order
/// the walk changed them.
pub fn do_walk_moves_the_children_whose_key_it_changed() {
    let mut tree = four();
    let root = tree.root;
    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetChannel(2), plain(0)),
    );
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetChannel(0), plain(2)),
    );
    mt.apply_to(&mut tree, &0);
    assert_eq!(tree.check_sorted(), Ok(()));
    assert_eq!(row(&tree, root), vec![30, 20, 10, 40]);
}

#[test]
fn test_walk_sees_the_row_as_it_was_until_it_ends() {
    do_walk_sees_the_row_as_it_was_until_it_ends();
}

/// The walk does not re-sort a row it is still merging over. The
/// channel-0 mutator moves its target to channel 5, and a channel-5
/// mutator follows it in the row: had the target moved at once, it
/// would land after channel 1, ahead of the merge, and the channel-5
/// mutator would reach it a second time. It moves when the walk is
/// over instead, and the channel-5 mutator reaches nothing.
pub fn do_walk_sees_the_row_as_it_was_until_it_ends() {
    let mut tree = empty();
    let root = tree.root;
    let moved = add(&mut tree, root, TestElement::new(0, 0));
    let stays = add(&mut tree, root, TestElement::new(1, 10));
    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetChannel(5), plain(0)),
    );
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(100), plain(5)));
    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, moved), 0, "the moved target is not met again");
    assert_eq!(value(&tree, stays), 10);
    assert_eq!(
        row(&tree, root),
        vec![10, 0],
        "and it is moved once the walk is over"
    );
    assert_eq!(tree.check_sorted(), Ok(()));
}

// ── derived state ─────────────────────────────────────────────────

#[test]
fn test_every_tree_method_that_changes_it_invalidates_the_cache() {
    do_every_tree_method_that_changes_it_invalidates_the_cache();
}

/// `insert_child`, `update` and `remove_subtree` each mark the tree's
/// derived state stale once, so no caller has to; a refused call,
/// which changes nothing, marks nothing.
pub fn do_every_tree_method_that_changes_it_invalidates_the_cache() {
    let mut tree = empty();
    let root = tree.root;
    let count = |tree: &Tree<TestElement>| tree.cache.invalidations.get();

    let before = count(&tree);
    let child = tree
        .insert_child(root, TestElement::new(0, 1), Placement::Last)
        .unwrap();
    assert_eq!(count(&tree) - before, 1, "insert_child");

    let before = count(&tree);
    tree.update(child, |el| el.value = 2).unwrap();
    assert_eq!(count(&tree) - before, 1, "update");

    let before = count(&tree);
    tree.remove_subtree(child).unwrap();
    assert_eq!(count(&tree) - before, 1, "remove_subtree");

    let before = count(&tree);
    assert!(tree.update(child, |el| el.value = 3).is_err());
    assert!(tree.remove_subtree(root).is_err());
    assert!(tree
        .insert_child(child, TestElement::new(0, 4), Placement::Last)
        .is_err());
    assert_eq!(count(&tree) - before, 0, "a refused call changes nothing");
}

// ── the check itself ──────────────────────────────────────────────

#[test]
fn test_check_sorted_finds_a_row_out_of_order() {
    do_check_sorted_finds_a_row_out_of_order();
}

/// [`Tree::check_sorted`] and [`MutatorTree::check_sorted`] name the
/// first child keyed below its previous sibling. No public API can
/// build such a row, so the fixture goes to the arena directly — which
/// only the engine itself can — to show the check is not vacuous; the
/// control is the same row built through `insert_child`.
pub fn do_check_sorted_finds_a_row_out_of_order() {
    let mut tree = empty();
    let root = tree.root;
    let parent = add(&mut tree, root, TestElement::new(0, 1));
    let high = tree.arena.new_node(TestElement::new(2, 2));
    parent.append(high, &mut tree.arena);
    let low = tree.arena.new_node(TestElement::new(1, 3));
    parent.append(low, &mut tree.arena);
    assert_eq!(
        tree.check_sorted(),
        Err(TreeError::Unsorted { parent, child: low })
    );

    let mut mt: MutatorTree<TestElement, TestMutation> = MutatorTree::new();
    let mroot = mt.root;
    let m_high = mt.arena.new_node(Mutator::new_void(plain(2)));
    mroot.append(m_high, &mut mt.arena);
    let m_low = mt.arena.new_node(Mutator::new_void(plain(1)));
    mroot.append(m_low, &mut mt.arena);
    assert_eq!(
        mt.check_sorted(),
        Err(TreeError::Unsorted {
            parent: mroot,
            child: m_low
        })
    );

    let mut sorted = empty();
    let root = sorted.root;
    let parent = add(&mut sorted, root, TestElement::new(0, 1));
    add(&mut sorted, parent, TestElement::new(2, 2));
    add(&mut sorted, parent, TestElement::new(1, 3));
    assert_eq!(
        sorted.check_sorted(),
        Ok(()),
        "the control: the same row, inserted"
    );
}
