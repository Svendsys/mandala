// SPDX-License-Identifier: MPL-2.0

//! Inherited state, as the walker carries it: a mutation receives its
//! target's **parent's** state, every element hands its children
//! `inherit(parent's state)` computed **after** its own mutations, and
//! the seed is the state of the walk root's parent. [`TestElement`]
//! makes the state observable — each element hands its children
//! `parent + value`, and [`TestMutation::SetToInherited`] writes what a
//! target received into its value.
//!
//! The expected values are sums of fixture literals worked out by
//! hand; each test keeps an element whose state the walk must not
//! disturb.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::tests::test_element::{add, add_mutator, value, TestElement, TestMutation};
use crate::engine::tree::{MutatorTree, Tree};

/// The seed every test walks with, chosen apart from every fixture
/// value so a sum that dropped it could not land on the expected one.
const SEED: i64 = 1000;

#[test]
fn test_inherited_state_reaches_a_grandchild() {
    do_inherited_state_reaches_a_grandchild();
}

/// `SetToInherited` on a grandchild sets it to seed + grandparent +
/// parent: the root (the grandparent, 1) hands its child `SEED + 1`,
/// the child (the parent, 10) hands the grandchild `SEED + 11`.
pub fn do_inherited_state_reaches_a_grandchild() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let parent = add(&mut tree, root, TestElement::new(0, 10));
    let grandchild = add(&mut tree, parent, TestElement::new(0, 100));

    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let m_parent = add_mutator(&mut mt, mroot, Mutator::new_void(0));
    add_mutator(&mut mt, m_parent, Mutator::new(TestMutation::SetToInherited, 0));

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, grandchild), 1011);
    assert_eq!(value(&tree, parent), 10, "the Void parent is unchanged");
}

#[test]
fn test_inherited_state_is_read_after_the_ancestors_own_mutations() {
    do_inherited_state_is_read_after_the_ancestors_own_mutations();
}

/// Mutating the parent in the same walk is reflected in what the child
/// receives: the parent's `Add(5)` runs before the parent hands its
/// state down, so the grandchild sees `SEED + 1 + 15`, not
/// `SEED + 1 + 10`. And the root's own mutation receives the seed
/// itself: `SetToInherited` at the root makes it `SEED`, which then
/// flows down in place of the root's original 1.
pub fn do_inherited_state_is_read_after_the_ancestors_own_mutations() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let parent = add(&mut tree, root, TestElement::new(0, 10));
    let grandchild = add(&mut tree, parent, TestElement::new(0, 100));

    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let m_parent = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(5), 0));
    add_mutator(&mut mt, m_parent, Mutator::new(TestMutation::SetToInherited, 0));
    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, parent), 15);
    assert_eq!(value(&tree, grandchild), 1016);

    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let child = add(&mut tree, root, TestElement::new(0, 10));
    let mut mt = MutatorTree::new_with(Mutator::new(TestMutation::SetToInherited, 0));
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::SetToInherited, 0));
    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, tree.root), SEED, "the root receives the seed");
    assert_eq!(
        value(&tree, child),
        2 * SEED,
        "and hands down seed + its new value"
    );
}

#[test]
fn test_inherited_state_leaves_a_sibling_subtree_alone() {
    do_inherited_state_leaves_a_sibling_subtree_alone();
}

/// The negative control: mutating one child changes what *its*
/// subtree inherits and nothing else. The sibling's child receives
/// `SEED + 1 + 20` — its own parent's value — not the mutated
/// sibling's.
pub fn do_inherited_state_leaves_a_sibling_subtree_alone() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let mutated = add(&mut tree, root, TestElement::new(1, 10));
    let under_mutated = add(&mut tree, mutated, TestElement::new(0, 0));
    let sibling = add(&mut tree, root, TestElement::new(2, 20));
    let under_sibling = add(&mut tree, sibling, TestElement::new(0, 0));

    let mut mt = MutatorTree::new();
    let mroot = mt.root;
    let m_mutated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(5), 1));
    add_mutator(&mut mt, m_mutated, Mutator::new(TestMutation::SetToInherited, 0));
    let m_sibling = add_mutator(&mut mt, mroot, Mutator::new_void(2));
    add_mutator(&mut mt, m_sibling, Mutator::new(TestMutation::SetToInherited, 0));

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, under_mutated), 1016);
    assert_eq!(
        value(&tree, under_sibling),
        1021,
        "the sibling's mutation is not inherited here"
    );
    assert_eq!(value(&tree, sibling), 20);
}

#[test]
fn test_inherited_state_threads_through_repeat_while() {
    do_inherited_state_threads_through_repeat_while();
}

/// `RepeatWhile` hands every descendant it reaches its own parent's
/// state, read after that parent's mutation in the same loop. With
/// `SetToInherited` repeated down a chain under a root of value 1:
/// `a` receives `SEED + 1` and becomes it, hands down
/// `(SEED + 1) + (SEED + 1)`, and so on — each value doubles the one
/// above.
pub fn do_inherited_state_threads_through_repeat_while() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let a = add(&mut tree, root, TestElement::new(0, 7));
    let b = add(&mut tree, a, TestElement::new(0, 7));
    let c = add(&mut tree, b, TestElement::new(0, 7));

    let mut mt = MutatorTree::new_with(Mutator::Instruction {
        channel: 0,
        instruction: Instruction::RepeatWhile(Predicate::always_true()),
        mutation: None,
    });
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::SetToInherited, 0));

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, a), 1001);
    assert_eq!(value(&tree, b), 2002);
    assert_eq!(value(&tree, c), 4004);
    assert_eq!(
        value(&tree, tree.root),
        1,
        "the instruction carries no payload of its own"
    );
}

#[test]
fn test_inherited_state_threads_through_map_children() {
    do_inherited_state_threads_through_map_children();
}

/// `MapChildren` hands every paired child the instruction target's own
/// state, and a channel-aligned mutator below a paired child receives
/// that child's state in turn.
pub fn do_inherited_state_threads_through_map_children() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let first = add(&mut tree, root, TestElement::new(4, 10));
    let below_first = add(&mut tree, first, TestElement::new(0, 0));
    let second = add(&mut tree, root, TestElement::new(9, 20));

    let mut mt = MutatorTree::new_with(Mutator::new_instruction(Instruction::MapChildren));
    let mroot = mt.root;
    let m_first = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(3), 0));
    add_mutator(&mut mt, m_first, Mutator::new(TestMutation::SetToInherited, 0));
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::SetToInherited, 0));

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, first), 13);
    assert_eq!(value(&tree, below_first), 1014, "SEED + root 1 + first 13");
    assert_eq!(value(&tree, second), 1001, "SEED + root 1");
}

#[test]
fn test_inherited_state_reaches_the_terminator() {
    do_inherited_state_reaches_the_terminator();
}

/// Where a `RepeatWhile` predicate fails, the default terminator
/// resumes the walk at that target with that target's parent's state —
/// the same state the loop would have handed it.
pub fn do_inherited_state_reaches_the_terminator() {
    use crate::engine::predicate::Comparator;
    use crate::engine::tests::test_element::TestField;
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let passes = add(&mut tree, root, TestElement::new(0, 10));
    let fails = add(&mut tree, passes, TestElement::new(0, 500));

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
    let repeated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), 0));
    add_mutator(&mut mt, repeated, Mutator::new(TestMutation::SetToInherited, 0));

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, passes), 11);
    assert_eq!(value(&tree, fails), 1012, "SEED + root 1 + passes 11");
}
