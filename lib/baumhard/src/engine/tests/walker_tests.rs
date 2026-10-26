// SPDX-License-Identifier: MPL-2.0

//! The walker matrix, run against [`TestElement`] — channel alignment,
//! broadcast to equal channels, `Void` gates, nested `Macro`s,
//! `RepeatWhile` with a filtering predicate (and its terminator),
//! `MapChildren` — plus the cache-invalidation contract. Every
//! expected value is a literal worked out from the fixture, not
//! recomputed through the code under test; every test leaves an
//! element the walk must *not* reach and asserts it untouched.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::tests::test_element::{
    add, add_mutator, value, TestElement, TestField, TestKind, TestMutation, TestMutator,
};
use crate::engine::tree::{MutatorTree, Tree};
use crate::engine::walker::walk_tree;
use indextree::NodeId;

/// A root (channel 0, value 0) with one child per `(channel, value)`.
fn flat_tree(children: &[(usize, i64)]) -> (Tree<TestElement>, Vec<NodeId>) {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let ids = children
        .iter()
        .map(|&(channel, v)| add(&mut tree, root, TestElement::new(channel, v)))
        .collect();
    (tree, ids)
}

/// A mutator tree rooted at a channel-0 `Void` with `children`.
fn mutators(children: Vec<TestMutator>) -> (MutatorTree<TestElement, TestMutation>, Vec<NodeId>) {
    let mut mt = MutatorTree::new();
    let root = mt.root;
    let ids = children
        .into_iter()
        .map(|m| add_mutator(&mut mt, root, m))
        .collect();
    (mt, ids)
}

// ── channel alignment ─────────────────────────────────────────────

#[test]
fn test_engine_walk_aligns_mutators_by_channel() {
    do_engine_walk_aligns_mutators_by_channel();
}

/// A mutator child reaches only the target children on its own
/// channel; a mutator on a channel no target carries reaches nothing.
/// Arena order is not channel order on either side.
pub fn do_engine_walk_aligns_mutators_by_channel() {
    let (mut tree, t) = flat_tree(&[(3, 30), (1, 10), (2, 20)]);
    let (mt, _) = mutators(vec![
        Mutator::new(TestMutation::Add(5), 2),
        Mutator::new(TestMutation::Add(500), 9),
        Mutator::new(TestMutation::Add(1), 1),
    ]);
    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, t[0]), 30, "channel 3 has no mutator");
    assert_eq!(value(&tree, t[1]), 11);
    assert_eq!(value(&tree, t[2]), 25);
    assert_eq!(value(&tree, tree.root), 0, "the Void root applies nothing");

    // The root pair is not merged — it is handed to the walk as given —
    // so the channel check there is the only thing standing between a
    // mutator and a target on another channel.
    let (mut tree, t) = flat_tree(&[(0, 10)]);
    let off_channel = MutatorTree::new_with(Mutator::new(TestMutation::Add(7), 7));
    walk_tree(&mut tree, &off_channel, &0);
    assert_eq!(
        value(&tree, tree.root),
        0,
        "a channel-7 root mutator misses a channel-0 root"
    );
    assert_eq!(value(&tree, t[0]), 10);
    let on_channel = MutatorTree::new_with(Mutator::new(TestMutation::Add(7), 0));
    walk_tree(&mut tree, &on_channel, &0);
    assert_eq!(
        value(&tree, tree.root),
        7,
        "the same mutator on channel 0 reaches it"
    );
}

// ── broadcast ─────────────────────────────────────────────────────

#[test]
fn test_engine_walk_broadcasts_to_equal_channels() {
    do_engine_walk_broadcasts_to_equal_channels();
}

/// One mutator reaches every target sharing its channel. The merge
/// consumes those targets as it goes, so a *second* mutator on the
/// same channel finds none left and reaches nothing — pinned here as
/// the walk does it, because this issue keeps the merge's semantics
/// exactly (routing keys replace it in a later sub-issue).
pub fn do_engine_walk_broadcasts_to_equal_channels() {
    let (mut tree, t) = flat_tree(&[(1, 10), (2, 20), (1, 30)]);
    let (mt, _) = mutators(vec![
        Mutator::new(TestMutation::Add(5), 1),
        Mutator::new(TestMutation::Add(100), 1),
    ]);
    walk_tree(&mut tree, &mt, &0);
    assert_eq!(
        value(&tree, t[0]),
        15,
        "the first channel-1 mutator reaches both targets"
    );
    assert_eq!(value(&tree, t[2]), 35);
    assert_eq!(value(&tree, t[1]), 20, "channel 2 is not broadcast to");
}

// ── Void gates ────────────────────────────────────────────────────

#[test]
fn test_engine_walk_void_gates_descent_without_applying() {
    do_engine_walk_void_gates_descent_without_applying();
}

/// A `Void` that matches a target's channel applies nothing to it but
/// carries its children down to the target's children; a `Void` whose
/// channel matches no target carries its children nowhere.
pub fn do_engine_walk_void_gates_descent_without_applying() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let gate = add(&mut tree, root, TestElement::new(1, 10));
    let behind_gate = add(&mut tree, gate, TestElement::new(0, 100));
    let closed = add(&mut tree, root, TestElement::new(2, 20));
    let behind_closed = add(&mut tree, closed, TestElement::new(0, 200));

    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let open_void = add_mutator(&mut mt, mroot, Mutator::new_void(1));
    add_mutator(&mut mt, open_void, Mutator::new(TestMutation::Add(1), 0));
    let shut_void = add_mutator(&mut mt, mroot, Mutator::new_void(7));
    add_mutator(&mut mt, shut_void, Mutator::new(TestMutation::Add(1), 0));

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, gate), 10, "a Void applies nothing to its own target");
    assert_eq!(value(&tree, behind_gate), 101, "but carries its children down");
    assert_eq!(value(&tree, closed), 20);
    assert_eq!(
        value(&tree, behind_closed),
        200,
        "a Void on channel 7 opens nothing"
    );
}

// ── nested Macros ─────────────────────────────────────────────────

#[test]
fn test_engine_walk_nested_macros_apply_in_order() {
    do_engine_walk_nested_macros_apply_in_order();
}

/// A `Macro` applies its whole list in order to its target, and a
/// `Macro` below it does the same one level down. `Set` then `Add`
/// gives a different answer from `Add` then `Set`, so the order is
/// pinned, not just the count.
pub fn do_engine_walk_nested_macros_apply_in_order() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let outer = add(&mut tree, root, TestElement::new(1, 10));
    let inner = add(&mut tree, outer, TestElement::new(0, 100));
    let sibling = add(&mut tree, outer, TestElement::new(4, 400));

    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let m_outer = add_mutator(
        &mut mt,
        mroot,
        Mutator::new_macro(vec![TestMutation::Add(1), TestMutation::Add(2)], 1),
    );
    add_mutator(
        &mut mt,
        m_outer,
        Mutator::new_macro(vec![TestMutation::Set(7), TestMutation::Add(1)], 0),
    );

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, outer), 13);
    assert_eq!(value(&tree, inner), 8, "Set(7) then Add(1), in that order");
    assert_eq!(
        value(&tree, sibling),
        400,
        "the inner Macro is on channel 0, not 4"
    );
}

// ── RepeatWhile ───────────────────────────────────────────────────

/// A chain under the root, all on channel 0, with the given values;
/// returns the chain's ids top-down.
fn chain(values: &[i64]) -> (Tree<TestElement>, Vec<NodeId>) {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let mut parent = tree.root;
    let mut ids = Vec::new();
    for &v in values {
        parent = add(&mut tree, parent, TestElement::new(0, v));
        ids.push(parent);
    }
    (tree, ids)
}

#[test]
fn test_engine_repeat_while_stops_where_the_predicate_fails() {
    do_engine_repeat_while_stops_where_the_predicate_fails();
}

/// `RepeatWhile(value < 100)` applies its child to every descendant
/// down to the first one that fails the predicate, and to nothing
/// below it — the branch terminates even though a deeper descendant
/// would pass again.
pub fn do_engine_repeat_while_stops_where_the_predicate_fails() {
    let (mut tree, c) = chain(&[1, 2, 500, 3]);
    let predicate = Predicate::<TestElement> {
        fields: vec![(TestField::Value(100), Comparator::less())],
        always_match: false,
    };
    let mut mt = MutatorTree::new_with(Mutator::Instruction {
        channel: 0,
        instruction: Instruction::RepeatWhile(predicate),
        mutation: None,
    });
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1000), 0));

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, c[0]), 1001);
    assert_eq!(value(&tree, c[1]), 1002);
    assert_eq!(value(&tree, c[2]), 500, "the predicate fails here");
    assert_eq!(value(&tree, c[3]), 3, "and nothing below the failure is reached");
}

#[test]
fn test_engine_repeat_while_filters_on_kind() {
    do_engine_repeat_while_filters_on_kind();
}

/// The predicate is the element type's own vocabulary: filtering on
/// [`TestKind`] stops at the first `Marked` element of each branch,
/// and a sibling branch is judged on its own.
pub fn do_engine_repeat_while_filters_on_kind() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let plain = add(&mut tree, root, TestElement::new(0, 1));
    let plain_below = add(&mut tree, plain, TestElement::new(0, 2));
    let marked = add(&mut tree, root, TestElement::marked(0, 3));
    let below_marked = add(&mut tree, marked, TestElement::new(0, 4));

    let predicate = Predicate::<TestElement> {
        fields: vec![(TestField::Kind(TestKind::Plain), Comparator::equals())],
        always_match: false,
    };
    let mut mt = MutatorTree::new_with(Mutator::Instruction {
        channel: 0,
        instruction: Instruction::RepeatWhile(predicate),
        mutation: None,
    });
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(10), 0));

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, plain), 11);
    assert_eq!(value(&tree, plain_below), 12);
    assert_eq!(value(&tree, marked), 3, "Marked fails the predicate");
    assert_eq!(value(&tree, below_marked), 4);
}

#[test]
fn test_engine_repeat_while_resumes_with_the_terminator() {
    do_engine_repeat_while_resumes_with_the_terminator();
}

/// Where the predicate fails, the default terminator walks the
/// repeated mutator's own children against that target — on matching
/// channels only.
pub fn do_engine_repeat_while_resumes_with_the_terminator() {
    let (mut tree, c) = chain(&[1, 500, 3]);
    let predicate = Predicate::<TestElement> {
        fields: vec![(TestField::Value(100), Comparator::less())],
        always_match: false,
    };
    let mut mt = MutatorTree::new_with(Mutator::Instruction {
        channel: 0,
        instruction: Instruction::RepeatWhile(predicate),
        mutation: None,
    });
    let mroot = mt.root;
    let repeated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1000), 0));
    add_mutator(&mut mt, repeated, Mutator::new(TestMutation::Set(-1), 0));
    add_mutator(&mut mt, repeated, Mutator::new(TestMutation::Set(-2), 5));

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, c[0]), 1001);
    assert_eq!(
        value(&tree, c[1]),
        -1,
        "the channel-0 after-mutation lands where the loop failed"
    );
    assert_eq!(value(&tree, c[2]), 3);
}

// ── MapChildren ───────────────────────────────────────────────────

#[test]
fn test_engine_map_children_pairs_by_position() {
    do_engine_map_children_pairs_by_position();
}

/// `MapChildren` pairs mutator children with target children by
/// sibling position, ignoring both sides' channels; an excess target
/// child is left alone.
pub fn do_engine_map_children_pairs_by_position() {
    let (mut tree, t) = flat_tree(&[(3, 30), (7, 70), (9, 90)]);
    let mut mt = MutatorTree::new_with(Mutator::new_instruction(Instruction::MapChildren));
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), 0));
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(2), 0));

    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, t[0]), 31);
    assert_eq!(value(&tree, t[1]), 72);
    assert_eq!(value(&tree, t[2]), 90, "a third target has no partner");
}

// ── cache invalidation ────────────────────────────────────────────

#[test]
fn test_engine_apply_invalidates_the_cache_per_mutation_and_once_more() {
    do_engine_apply_invalidates_the_cache_per_mutation_and_once_more();
}

/// `MutatorTree::apply_to` invalidates the tree's cache once for every
/// mutator node the walker applies to a target and once when it ends — so a walk that
/// applied nothing still invalidates, exactly once, and the count
/// tells applied mutations from aligned-but-skipped ones.
pub fn do_engine_apply_invalidates_the_cache_per_mutation_and_once_more() {
    let (mut tree, _) = flat_tree(&[(1, 10), (1, 11), (2, 20)]);
    let (mt, _) = mutators(vec![Mutator::new(TestMutation::Add(1), 1)]);
    mt.apply_to(&mut tree, &0);
    assert_eq!(
        tree.cache.invalidations.get(),
        3,
        "two channel-1 targets applied, plus the end of the walk"
    );

    let (mut untouched, _) = flat_tree(&[(2, 20)]);
    mt.apply_to(&mut untouched, &0);
    assert_eq!(
        untouched.cache.invalidations.get(),
        1,
        "nothing applied: only the end of the walk"
    );
}

// ── a wide tree, for the bench ────────────────────────────────────

#[test]
fn test_engine_walk_wide_and_deep_tree() {
    do_engine_walk_wide_and_deep_tree();
}

/// A tree of 16 children with 16 grandchildren each, every element on
/// channel `i % 4`, walked by a mutator that broadcasts to channel 1
/// at both levels and carries inherited state down. The walker's
/// per-node work on an element type with a real `Inherited`; the
/// assertions pin the count of elements reached.
pub fn do_engine_walk_wide_and_deep_tree() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    for i in 0..16 {
        let child = add(&mut tree, root, TestElement::new(i % 4, 1));
        for j in 0..16 {
            add(&mut tree, child, TestElement::new(j % 4, 1));
        }
    }
    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let level_one = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), 1));
    add_mutator(&mut mt, level_one, Mutator::new(TestMutation::SetToInherited, 1));

    walk_tree(&mut tree, &mt, &0);
    // 4 level-one targets on channel 1 → value 2; under each, 4
    // grandchildren on channel 1 receive their parent's state:
    // seed 0 + root 0 + parent 2 = 2.
    let reached_children = tree
        .root
        .children(&tree.arena)
        .filter(|id| value(&tree, *id) == 2)
        .count();
    assert_eq!(reached_children, 4);
    let grandchildren_set: usize = tree
        .root
        .children(&tree.arena)
        .map(|c| c.children(&tree.arena).filter(|g| value(&tree, *g) == 2).count())
        .sum();
    assert_eq!(grandchildren_set, 16);
}
