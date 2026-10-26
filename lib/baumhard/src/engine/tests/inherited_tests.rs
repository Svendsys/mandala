// SPDX-License-Identifier: MPL-2.0

//! Inherited state, as the walker carries it: a mutation receives its
//! target's **parent's** state, every element hands its children
//! `inherit(parent's state)` computed **after** its own mutations, and
//! the seed is the state of the walk root's parent. [`TestElement`]
//! makes the state observable — each element hands its children
//! `parent + value`, and [`TestMutation::SetToInherited`] writes what a
//! target received into its value. [`SpatialTestElement`] does the
//! same for the `SpatialDescend` arm, which folds a hit's state down a
//! path rather than descending to it.
//!
//! The expected values are sums of fixture literals worked out by
//! hand; each test keeps an element whose state the walk must not
//! disturb.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::tests::test_element::{
    add, add_mutator, plain, value, SpatialTestElement, TestElement, TestMutation,
};
use crate::engine::tree::{MutatorTree, Placement, Tree};
use crate::engine::walker::walk_tree;
use crate::util::ordered_vec2::OrderedVec2;
use indextree::NodeId;

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
    let m_parent = add_mutator(&mut mt, mroot, Mutator::new_void(plain(0)));
    add_mutator(
        &mut mt,
        m_parent,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );

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
    let m_parent = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(5), plain(0)));
    add_mutator(
        &mut mt,
        m_parent,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );
    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, parent), 15);
    assert_eq!(value(&tree, grandchild), 1016);

    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 1));
    let root = tree.root;
    let child = add(&mut tree, root, TestElement::new(0, 10));
    let mut mt = MutatorTree::new_with(Mutator::new(TestMutation::SetToInherited, plain(0)));
    let mroot = mt.root;
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );
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
    let m_mutated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(5), plain(1)));
    add_mutator(
        &mut mt,
        m_mutated,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );
    let m_sibling = add_mutator(&mut mt, mroot, Mutator::new_void(plain(2)));
    add_mutator(
        &mut mt,
        m_sibling,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );

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
        key: plain(0),
        instruction: Instruction::RepeatWhile(Predicate::always_true()),
        mutation: None,
    });
    let mroot = mt.root;
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );

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

    let mut mt = MutatorTree::new_with(Mutator::new_instruction(Instruction::MapChildren, plain(0)));
    let mroot = mt.root;
    let m_first = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(3), plain(0)));
    add_mutator(
        &mut mt,
        m_first,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );

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
        key: plain(0),
        instruction: Instruction::RepeatWhile(predicate),
        mutation: None,
    });
    let mroot = mt.root;
    let repeated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), plain(0)));
    add_mutator(
        &mut mt,
        repeated,
        Mutator::new(TestMutation::SetToInherited, plain(0)),
    );

    mt.apply_to(&mut tree, &SEED);
    assert_eq!(value(&tree, passes), 11);
    assert_eq!(value(&tree, fails), 1012, "SEED + root 1 + passes 11");
}

#[test]
fn test_inherited_state_reaches_a_spatial_descend_hit() {
    do_inherited_state_reaches_a_spatial_descend_hit();
}

/// `SpatialDescend` reaches its hit without descending to it level by
/// level, so the walker folds the hit's state down the path instead:
/// the hit receives seed + every element from the walk target down to
/// the hit's parent, and nothing of its own.
///
/// The target root (1) holds a parent box (10, 100×100 at the origin)
/// holding a grandchild box (100, 20×20 at (10, 10)), beside a sibling
/// box (7) far from the point. The point (15, 15) is in the parent and
/// the grandchild; the grandchild is smaller and wins, so
/// `SetToInherited` makes it `SEED + 1 + 10`. Folding the hit itself in
/// would give 1111; starting below the target, 1010; skipping the fold,
/// `SEED`.
///
/// An `Instruction` also applies its mutation to its own target, under
/// the rule of the walk's entry. `walk_tree` pairs the two roots as any
/// pair is paired, by key, and the target here is on another channel,
/// so the hit is the only element written. `apply_to` anchors at the
/// root, where only kinds are compared: the target becomes `SEED`
/// first, and the fold reads that, so the hit gets `SEED + SEED + 10`.
pub fn do_inherited_state_reaches_a_spatial_descend_hit() {
    fn add_spatial(
        tree: &mut Tree<SpatialTestElement>,
        parent: NodeId,
        element: SpatialTestElement,
    ) -> NodeId {
        tree.insert_child(parent, element, Placement::Last).unwrap()
    }
    fn value_of(tree: &Tree<SpatialTestElement>, id: NodeId) -> i64 {
        tree.arena()
            .get(id)
            .expect("fixture id belongs to the tree")
            .get()
            .value
    }
    fn fixture(root_channel: usize) -> (Tree<SpatialTestElement>, [NodeId; 3]) {
        let mut tree = Tree::new_non_indexed_with(SpatialTestElement::unboxed(root_channel, 1));
        let root = tree.root;
        let parent = add_spatial(
            &mut tree,
            root,
            SpatialTestElement::boxed(0, 10, (0.0, 0.0, 100.0, 100.0)),
        );
        let grandchild = add_spatial(
            &mut tree,
            parent,
            SpatialTestElement::boxed(0, 100, (10.0, 10.0, 20.0, 20.0)),
        );
        let sibling = add_spatial(
            &mut tree,
            root,
            SpatialTestElement::boxed(0, 7, (200.0, 200.0, 50.0, 50.0)),
        );
        (tree, [parent, grandchild, sibling])
    }
    let mt: MutatorTree<SpatialTestElement, TestMutation> = MutatorTree::new_with(Mutator::Instruction {
        key: 0.into(),
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(15.0, 15.0)),
        mutation: Some(TestMutation::SetToInherited),
    });

    let (mut tree, [parent, grandchild, sibling]) = fixture(5);
    walk_tree(&mut tree, &mt, &SEED);
    assert_eq!(value_of(&tree, grandchild), 1011, "SEED + target 1 + parent 10");
    assert_eq!(
        value_of(&tree, tree.root),
        1,
        "an off-channel target is not written by a pair walk"
    );
    assert_eq!(
        value_of(&tree, parent),
        10,
        "the larger box loses to the grandchild"
    );
    assert_eq!(
        value_of(&tree, sibling),
        7,
        "a box away from the point is not hit"
    );

    let (mut tree, [parent, grandchild, sibling]) = fixture(5);
    mt.apply_to(&mut tree, &SEED);
    assert_eq!(
        value_of(&tree, tree.root),
        1000,
        "the anchored target takes SEED whatever its channel"
    );
    assert_eq!(
        value_of(&tree, grandchild),
        2010,
        "SEED + the target's new 1000 + parent 10"
    );
    assert_eq!(value_of(&tree, parent), 10);
    assert_eq!(value_of(&tree, sibling), 7);
}
