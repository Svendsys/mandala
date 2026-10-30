// SPDX-License-Identifier: MPL-2.0

//! [`Tree::contains`] against a brute-force climb, through every way a
//! tree's membership changes.
//!
//! `contains` reads the tree's set of detached nodes, which the engine
//! writes where a subtree leaves or re-enters the tree: a `Remove` or a
//! `Merge` detaching it, a journal's revert, replay or interpolated write
//! (from its touches, `Journal::mark`), and a discard freeing it. Every
//! refusal in the engine trusts that set, so these tests hold it to the
//! one answer that needs no set at all — climbing from the node to the
//! top of whatever it hangs in — for every node the arena holds and
//! every id ever seen, freed ones included, after every call and every
//! step of a journal's life (V1-A5 #282, review of #286).
//!
//! The lifecycle is the one the document's writer runs (ruling R7 on
//! #238): entries of one to three journals — a transaction's base calls
//! and its stage, the stage replaced a few times — undone newest first
//! and redone oldest first; a redo branch discarded when a new entry
//! comes; the oldest entry trimmed while applied; whole builders
//! abandoned; calls that fail and unwind their builder; interpolated
//! writes at random `t` and either [`Snap`](crate::engine::interpolate::Snap), then a replay; and at the
//! end everything undone or discarded, with nothing leaked and nothing
//! left marked.
//!
//! The two sweeps are plain tests, not `do_*` bodies — each is a
//! seeded sweep whose enumeration is its runtime
//! (lib/baumhard/CONVENTIONS.md §B8, the third shape) — in a
//! `cfg(test)` module of their own; the two single scenarios beside
//! them are benchmark-reachable bodies.

use crate::engine::anchor::Anchors;
use crate::engine::journal::{Journal, JournalBuilder};
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tests::test_element::{add, TestElement, TestKind, TestMutation};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use indextree::NodeId;

/// The reviewer's example: J1 inserts S under P, J2 removes P. Undoing
/// J1 before J2 would detach S, and undoing J2 would then mark S back
/// in — its touches name S as removed with P — though S hangs under
/// nothing. So J1's revert is refused while J2 is applied, nothing
/// changes, and in order both undo and redo exactly.
pub fn do_a_journal_is_not_reverted_under_a_later_one() {
    let mut tree: Tree<TestElement> = Tree::new_non_indexed();
    let root = tree.root;
    let p = add(&mut tree, root, TestElement::new(0, 1));
    let insert: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 2)),
            at: Placement::Last,
        },
        BranchKey::new(TestKind::Plain, 0),
    ));
    let remove: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Remove,
        BranchKey::new(TestKind::Plain, 0),
    ));

    let mut first = JournalBuilder::new();
    insert
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![p]), |_, _| 0, &mut first)
        .unwrap();
    let j1 = first.close(&tree);
    let s = tree.arena()[p].first_child().expect("S is under P");
    let mut second = JournalBuilder::new();
    remove
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![p]), |_, _| 0, &mut second)
        .unwrap();
    let j2 = second.close(&tree);
    assert!(
        !tree.contains(p) && !tree.contains(s),
        "precondition: P and S are out"
    );

    assert_eq!(j1.revert(&mut tree), Err(TreeError::OutOfOrder), "J1 under J2");
    assert!(!tree.contains(s), "nothing changed");
    assert_eq!(
        tree.arena()[p].first_child(),
        Some(s),
        "S is still under the detached P"
    );

    j2.revert(&mut tree).unwrap();
    j1.revert(&mut tree).unwrap();
    assert!(tree.contains(p) && !tree.contains(s), "in order: P back, S out");
    assert_eq!(j2.replay(&mut tree), Err(TreeError::OutOfOrder), "J2 before J1");
    j1.replay(&mut tree).unwrap();
    j2.replay(&mut tree).unwrap();
    assert!(!tree.contains(p) && !tree.contains(s), "redone");
    j1.discard(&mut tree);
    j2.discard(&mut tree);
}

#[test]
fn test_a_journal_is_not_reverted_under_a_later_one() {
    do_a_journal_is_not_reverted_under_a_later_one();
}

/// A journal that closes empty — every call it recorded undone by a
/// later one of the same builder — has no arrangement of its own: it
/// leaves the tree at the version the journal before it ended at. So
/// dropping it, kept in the history or not, never blocks that earlier
/// journal's undo; before closing reset the version, a discarded empty
/// journal stranded the tree at a version no other journal held, and
/// every undo past it was refused as out of order.
pub fn do_an_empty_journal_leaves_the_history_restorable() {
    let mut tree: Tree<TestElement> = Tree::new_non_indexed();
    let root = tree.root;
    let p = add(&mut tree, root, TestElement::new(0, 1));
    let insert: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 2)),
            at: Placement::Last,
        },
        BranchKey::new(TestKind::Plain, 0),
    ));
    let mut first = JournalBuilder::new();
    insert
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![p]), |_, _| 0, &mut first)
        .unwrap();
    let j0 = first.close(&tree);
    let s = tree.arena()[p].first_child().expect("S is under P");

    // A builder whose two updates cancel out: it ran calls, and closes
    // empty.
    let mut noop = JournalBuilder::new();
    tree.update_journaled(p, &mut noop, |e| e.value += 1).unwrap();
    tree.update_journaled(p, &mut noop, |e| e.value -= 1).unwrap();
    let empty = noop.close(&tree);
    assert!(empty.is_empty(), "precondition: the journal closed empty");
    empty.discard(&mut tree);
    j0.revert(&mut tree).unwrap();
    assert!(
        tree.contains(p) && !tree.contains(s),
        "J0 undone past the dropped empty journal"
    );
    j0.replay(&mut tree).unwrap();

    // Kept between two journals and dropped while the later one is
    // applied: undoing the later one, then J0, is still in order.
    let mut noop = JournalBuilder::new();
    tree.update_journaled(p, &mut noop, |e| e.value += 1).unwrap();
    tree.update_journaled(p, &mut noop, |e| e.value -= 1).unwrap();
    let empty = noop.close(&tree);
    let mut third = JournalBuilder::new();
    tree.update_journaled(s, &mut third, |e| e.value += 5).unwrap();
    let j2 = third.close(&tree);
    empty.discard(&mut tree);
    j2.revert(&mut tree).unwrap();
    j0.revert(&mut tree).unwrap();
    assert!(tree.contains(p) && !tree.contains(s), "both undone");
    j0.discard(&mut tree);
    j2.discard(&mut tree);
}

#[test]
fn test_an_empty_journal_leaves_the_history_restorable() {
    do_an_empty_journal_leaves_the_history_restorable();
}

/// A tree with P and Q under the root, and J1, which inserted S under
/// P: the start of each unjournaled-change case below.
fn with_s_inserted_under_p() -> (Tree<TestElement>, Journal<TestElement>, [NodeId; 3]) {
    let mut tree: Tree<TestElement> = Tree::new_non_indexed();
    let root = tree.root;
    let p = add(&mut tree, root, TestElement::new(0, 1));
    let q = add(&mut tree, root, TestElement::new(0, 2));
    let insert: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 3)),
            at: Placement::Last,
        },
        BranchKey::new(TestKind::Plain, 0),
    ));
    let mut builder = JournalBuilder::new();
    insert
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![p]), |_, _| 0, &mut builder)
        .unwrap();
    let j1 = builder.close(&tree);
    let s = tree.arena()[p].first_child().expect("S is under P");
    (tree, j1, [p, q, s])
}

/// An unjournaled structural change ends the history before it (review
/// round 3 on #286). J1 inserts S under P; then, with no journal:
///
/// - `insert_child(S, X)`. Were J1's revert accepted, it would detach S
///   with X under it, while its touches — which never named X — left X
///   marked in the tree: `contains(X)` true for a node under a detached
///   subtree, and `insert_child(X, …)` accepted under it. It is refused,
///   and nothing changes.
/// - `remove_subtree(S)`: J1 names S, so it is refused as stale
///   first. And, apart, `remove_subtree(Q)` of a node J1 never touched,
///   which leaves every node J1 names where J1 left it: J1 is refused
///   for its order, not for a node it happens to find missing.
/// - An unjournaled `update` of P, which leaves every node's parent and
///   membership as they were, does not end the history: J1 still
///   reverts.
pub fn do_an_unjournaled_structural_change_ends_the_history() {
    let (mut tree, j1, [_, _, s]) = with_s_inserted_under_p();
    let x = tree
        .insert_child(s, TestElement::new(0, 4), Placement::Last)
        .unwrap();
    assert_eq!(
        j1.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "J1 after insert_child"
    );
    assert!(tree.contains(s) && tree.contains(x), "nothing changed");
    assert_eq!(tree.arena()[x].parent(), Some(s), "X is still under S");
    assert_eq!(j1.replay(&mut tree), Err(TreeError::OutOfOrder), "nor replayed");
    j1.discard(&mut tree);

    let (mut tree, j1, [_, _, s]) = with_s_inserted_under_p();
    tree.remove_subtree(s).unwrap();
    assert_eq!(
        j1.revert(&mut tree),
        Err(TreeError::StaleNode(s)),
        "J1 after removing S"
    );
    j1.discard(&mut tree);

    let (mut tree, j1, [_, q, s]) = with_s_inserted_under_p();
    tree.remove_subtree(q).unwrap();
    assert_eq!(
        j1.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "J1 after removing Q"
    );
    assert!(tree.contains(s), "nothing changed");
    j1.discard(&mut tree);

    let (mut tree, j1, [p, _, s]) = with_s_inserted_under_p();
    tree.update(p, |e| e.value += 1).unwrap();
    j1.revert(&mut tree).unwrap();
    assert!(!tree.contains(s), "J1 reverts after an update");
    j1.discard(&mut tree);
}

#[test]
fn test_an_unjournaled_structural_change_ends_the_history() {
    do_an_unjournaled_structural_change_ends_the_history();
}

/// A one-node `Insert` under whatever node it is anchored at.
fn insert_one() -> MutatorTree<TestElement, TestMutation> {
    MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 5)),
            at: Placement::Last,
        },
        BranchKey::new(TestKind::Plain, 0),
    ))
}

/// A change made between two calls of one builder ends the history
/// before it too (review round 4 on #286). The builder's later call
/// must not put the tree back at the stamp its first call left: that
/// would erase the change, and the journal the change ended would
/// match again. Each case starts from J0, which inserted S under P; X
/// ends up under S, so a revert of J0 that were accepted would detach
/// S with X under it and leave X marked in the tree.
///
/// - **One builder, an unjournaled edit between its calls**, this body (the
///   reviewer's probe): B updates Q; `insert_child(S, X)`; B updates Q
///   again. B's own journal reverts and replays; J0 is refused.
/// - **Two builders, interleaved**: A updates Q; B inserts X under S,
///   journaled; A updates Q again. Neither A's revert nor B's brings J0
///   back.
/// - **An empty builder closed after another's change**: A's one call
///   changes nothing; B inserts X under S and closes; A closes empty.
///   Closing A must not wind the tree back to where A started, under
///   B's change. (This one is conservative: once B is undone too, J0
///   stays refused, since A and B interleaved.)
pub fn do_an_edit_between_one_builders_calls_ends_the_history() {
    let (mut tree, j0, [_, q, s]) = with_s_inserted_under_p();
    let mut b = JournalBuilder::new();
    tree.update_journaled(q, &mut b, |e| e.value += 1).unwrap();
    let x = tree
        .insert_child(s, TestElement::new(0, 4), Placement::Last)
        .unwrap();
    tree.update_journaled(q, &mut b, |e| e.value += 1).unwrap();
    let jb = b.close(&tree);
    jb.revert(&mut tree).unwrap();
    assert_eq!(
        j0.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "J0 under the edit between B's calls"
    );
    assert!(tree.contains(s) && tree.contains(x), "nothing changed");
    assert_eq!(tree.arena()[x].parent(), Some(s), "X is still under S");
    jb.replay(&mut tree).unwrap();
    jb.revert(&mut tree).unwrap();
    jb.discard(&mut tree);
    j0.discard(&mut tree);
}

/// The second case of [`do_an_edit_between_one_builders_calls_ends_the_history`]:
/// two builders, interleaved.
pub fn do_interleaved_builders_end_the_history() {
    let (mut tree, j0, [_, q, s]) = with_s_inserted_under_p();
    let mut a = JournalBuilder::new();
    let mut b = JournalBuilder::new();
    tree.update_journaled(q, &mut a, |e| e.value += 1).unwrap();
    insert_one()
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![s]), |_, _| 0, &mut b)
        .unwrap();
    let x = tree.arena()[s].first_child().expect("X is under S");
    tree.update_journaled(q, &mut a, |e| e.value += 1).unwrap();
    let ja = a.close(&tree);
    let jb = b.close(&tree);
    ja.revert(&mut tree).unwrap();
    assert_eq!(
        j0.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "J0 after A, under B"
    );
    assert_eq!(
        jb.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "B under A's later call"
    );
    assert!(tree.contains(s) && tree.contains(x), "nothing changed");
    assert_eq!(tree.arena()[x].parent(), Some(s), "X is still under S");
    ja.discard(&mut tree);
    jb.discard(&mut tree);
    j0.discard(&mut tree);
}

/// The third case of [`do_an_edit_between_one_builders_calls_ends_the_history`]:
/// an empty builder closed after another's change.
pub fn do_an_empty_builder_closed_after_another_keeps_its_change() {
    let (mut tree, j0, [_, q, s]) = with_s_inserted_under_p();
    let mut a = JournalBuilder::new();
    let mut b = JournalBuilder::new();
    tree.update_journaled(q, &mut a, |e| e.value += 0).unwrap();
    insert_one()
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![s]), |_, _| 0, &mut b)
        .unwrap();
    let x = tree.arena()[s].first_child().expect("X is under S");
    let jb = b.close(&tree);
    let ja = a.close(&tree);
    assert!(ja.is_empty(), "precondition: A closed empty");
    ja.discard(&mut tree);
    assert_eq!(
        j0.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "J0 under B, past the empty A"
    );
    assert!(tree.contains(s) && tree.contains(x), "nothing changed");
    jb.revert(&mut tree).unwrap();
    assert!(!tree.contains(x), "B undone");
    assert_eq!(
        j0.revert(&mut tree),
        Err(TreeError::OutOfOrder),
        "conservative: A and B interleaved"
    );
    jb.discard(&mut tree);
    j0.discard(&mut tree);
}

#[test]
fn test_an_edit_between_one_builders_calls_ends_the_history() {
    do_an_edit_between_one_builders_calls_ends_the_history();
}

#[test]
fn test_interleaved_builders_end_the_history() {
    do_interleaved_builders_end_the_history();
}

#[test]
fn test_an_empty_builder_closed_after_another_keeps_its_change() {
    do_an_empty_builder_closed_after_another_keeps_its_change();
}

/// The seeded sweeps, and the helpers only they use. `cfg(test)`
/// because the enumeration is their runtime, not the primitive
/// (lib/baumhard/CONVENTIONS.md §B8, the third shape): plain
/// `#[test]` fns, with nothing for the library build to compile.
#[cfg(test)]
mod sweeps {
    use crate::engine::anchor::Anchors;
    use crate::engine::element::TreeElement;
    use crate::engine::interpolate::Snap;
    use crate::engine::journal::{Journal, JournalBuilder};
    use crate::engine::key::BranchKey;
    use crate::engine::mutation::Mutation;
    use crate::engine::mutator::{Instruction, Mutator};
    use crate::engine::predicate::{Comparator, Predicate};
    use crate::engine::structure::{StructOp, Subtree};
    use crate::engine::tests::journal_fixtures::{state_above, structural_scenario, StructuralScenario};
    use crate::engine::tests::test_element::{TestElement, TestField, TestKind, TestMutation};
    use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
    use indextree::NodeId;
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

    /// How many seeds each sweep runs.
    const SEEDS: u64 = 400;

    /// `contains` against a climb, for every node the arena holds and every
    /// id in `seen` — which collects them, so a freed id goes on being asked
    /// and must answer `false`. Returns how many were checked.
    fn check_contains(context: &str, tree: &Tree<TestElement>, seen: &mut Vec<NodeId>) -> usize {
        let arena = tree.arena();
        seen.extend(
            arena
                .iter()
                .filter(|n| !n.is_removed())
                .filter_map(|n| arena.get_node_id(n)),
        );
        seen.sort();
        seen.dedup();
        for &id in seen.iter() {
            // The oracle: in the arena, and the root at the top of its
            // climb. A climb is fine here: this is the answer, not the code.
            let climbed = arena.get(id).is_some_and(|n| !n.is_removed())
                && id.ancestors(arena).last() == Some(tree.root);
            assert_eq!(tree.contains(id), climbed, "{context}: contains({id:?})");
        }
        seen.len()
    }

    /// A random key, mostly `Plain`.
    fn key(rng: &mut StdRng) -> BranchKey<TestKind> {
        let kind = if rng.random_bool(0.8) {
            TestKind::Plain
        } else {
            TestKind::Marked
        };
        BranchKey::new(kind, rng.random_range(0..3))
    }

    /// A random element.
    fn element(rng: &mut StdRng) -> TestElement {
        let key = key(rng);
        TestElement {
            kind: key.kind,
            channel: key.channel,
            value: rng.random_range(-3..=6),
        }
    }

    /// A random mutation.
    fn mutation(rng: &mut StdRng) -> TestMutation {
        match rng.random_range(0..9) {
            0..=2 => TestMutation::Add(rng.random_range(-3..=3)),
            3 => TestMutation::Set(rng.random_range(-5..=5)),
            4 | 5 => TestMutation::SetToInherited,
            _ => TestMutation::SetChannel(rng.random_range(0..3)),
        }
    }

    /// A random predicate, sometimes always true.
    fn predicate(rng: &mut StdRng) -> Predicate<TestElement> {
        if rng.random_bool(0.2) {
            return Predicate::always_true();
        }
        let bound = rng.random_range(-3..=6);
        let comparator = match rng.random_range(0..4) {
            0 => Comparator::less(),
            1 => Comparator::greater(),
            2 => Comparator::not_equals(),
            _ => Comparator::greater_or_equal(),
        };
        let mut predicate = Predicate::new();
        predicate.fields.push((TestField::Value(bound), comparator));
        predicate
    }

    /// A random structural command: a remove, a split, a merge, a move to
    /// one of `nodes`, or an insert of a small subtree.
    fn op(rng: &mut StdRng, nodes: &[NodeId]) -> StructOp<TestElement> {
        let place = |rng: &mut StdRng| {
            if rng.random_bool(0.5) {
                Placement::Last
            } else {
                Placement::First
            }
        };
        match rng.random_range(0..5) {
            0 => StructOp::Remove,
            1 => StructOp::Split {
                at: rng.random_range(1..4),
            },
            2 => StructOp::Merge,
            3 => StructOp::Move {
                to: nodes[rng.random_range(0..nodes.len())],
                at: place(rng),
            },
            _ => {
                let mut subtree = Subtree::leaf(element(rng));
                for _ in 0..rng.random_range(0..3) {
                    let mut child = Subtree::leaf(element(rng));
                    if rng.random_bool(0.3) {
                        child.children.push(Subtree::leaf(element(rng)));
                    }
                    subtree.children.push(child);
                }
                StructOp::Insert {
                    subtree,
                    at: place(rng),
                }
            }
        }
    }

    /// One random journaled call on `builder`: a structural scenario's next
    /// call, a removal (removals nest across calls and journals), a
    /// `RepeatWhile` carrying a command at many levels, an update, or
    /// several commands in one walk. Some fail, and unwind the builder.
    fn random_call(
        rng: &mut StdRng,
        scenario: &mut StructuralScenario,
        tree: &mut Tree<TestElement>,
        builder: &mut JournalBuilder<TestElement>,
    ) -> Result<(), TreeError> {
        let nodes: Vec<NodeId> = tree.descendants().collect();
        let pick = |rng: &mut StdRng| nodes[rng.random_range(0..nodes.len())];
        match rng.random_range(0..10) {
            0..=5 => {
                let (call, _) = scenario.next_call(tree);
                call.run(tree, builder)
            }
            6 => {
                let node = pick(rng);
                let key = tree.arena()[node].get().key();
                MutatorTree::<TestElement, TestMutation>::new_with(Mutator::new_structure(
                    StructOp::Remove,
                    key,
                ))
                .apply_at_journaled(tree, &Anchors::Nodes(vec![node]), state_above, builder)
            }
            7 => {
                let node = pick(rng);
                let key = tree.arena()[node].get().key();
                let mut mutators: MutatorTree<TestElement, TestMutation> = MutatorTree::new_with(
                    Mutator::new_instruction(Instruction::RepeatWhile(predicate(rng)), key),
                );
                let root = mutators.root;
                let command = op(rng, &nodes);
                let child_key = self::key(rng);
                mutators
                    .insert_child(root, Mutator::new_structure(command, child_key), Placement::Last)
                    .unwrap();
                mutators.apply_at_journaled(tree, &Anchors::Nodes(vec![node]), state_above, builder)
            }
            8 => {
                let node = pick(rng);
                let change = mutation(rng);
                tree.update_journaled(node, builder, |e| change.apply_to(e, &0))
                    .map(|_| ())
            }
            _ => {
                let node = pick(rng);
                let key = tree.arena()[node].get().key();
                let mut mutators: MutatorTree<TestElement, TestMutation> =
                    MutatorTree::new_with(Mutator::new_void(key));
                let root = mutators.root;
                for _ in 0..rng.random_range(1..=4) {
                    let command = op(rng, &nodes);
                    let child_key = self::key(rng);
                    mutators
                        .insert_child(root, Mutator::new_structure(command, child_key), Placement::Last)
                        .unwrap();
                }
                mutators.apply_at_journaled(tree, &Anchors::Nodes(vec![node]), state_above, builder)
            }
        }
    }

    /// What a sweep did, so a test can show it did each thing at all.
    #[derive(Default, Debug)]
    struct Done {
        checks: usize,
        calls: usize,
        failed_calls: usize,
        abandons: usize,
        interpolations: usize,
        stage_replacements: usize,
        undos: usize,
        redos: usize,
        trims: usize,
        inserted: usize,
        removed: usize,
    }

    /// Count the touches of `journal` that put nodes into or out of the
    /// tree, so the sweep can show it exercised both.
    fn count_touches(journal: &Journal<TestElement>, done: &mut Done) {
        use crate::engine::journal::Touch;
        for (_, touch) in journal.touched() {
            match touch {
                Touch::Inserted => done.inserted += 1,
                Touch::Removed => done.removed += 1,
                Touch::Field | Touch::Moved => {}
            }
        }
    }

    #[test]
    fn test_contains_matches_a_climb_through_the_journal_lifecycle() {
        let mut done = Done::default();
        for seed in 0..SEEDS {
            let mut scenario = structural_scenario(seed);
            let mut tree = scenario.tree.clone();
            let mut rng = StdRng::seed_from_u64(seed ^ 0xA5A5);
            let mut seen = Vec::new();
            // Each entry is a list of journals, oldest first; `applied`
            // entries are in the tree, the rest are the redo branch.
            let mut history: Vec<Vec<Journal<TestElement>>> = Vec::new();
            let mut applied = 0usize;
            done.checks += check_contains(&format!("seed {seed} start"), &tree, &mut seen);
            for step in 0..rng.random_range(4..=16) {
                let context = format!("seed {seed} step {step}");
                let roll = rng.random_range(0..100);
                if roll < 50 {
                    let mut entry = Vec::new();
                    for _part in 0..rng.random_range(1..=3) {
                        let mut stages = rng.random_range(1..=3);
                        loop {
                            let mut builder = JournalBuilder::new();
                            for _ in 0..rng.random_range(1..=6) {
                                match random_call(&mut rng, &mut scenario, &mut tree, &mut builder) {
                                    Ok(()) => done.calls += 1,
                                    Err(_) => done.failed_calls += 1,
                                }
                                done.checks += check_contains(&format!("{context} call"), &tree, &mut seen);
                            }
                            if rng.random_bool(0.15) {
                                builder.abandon(&mut tree);
                                done.abandons += 1;
                                done.checks +=
                                    check_contains(&format!("{context} abandon"), &tree, &mut seen);
                                break;
                            }
                            let journal = builder.close(&tree);
                            count_touches(&journal, &mut done);
                            if rng.random_bool(0.3) {
                                let t: f32 = rng.random_range(0.0..1.0);
                                let snap = if rng.random_bool(0.5) {
                                    Snap::Start
                                } else {
                                    Snap::End
                                };
                                journal.write_interpolated(&mut tree, t, snap).unwrap();
                                done.checks +=
                                    check_contains(&format!("{context} interpolate t={t}"), &tree, &mut seen);
                                journal.replay(&mut tree).unwrap();
                                done.checks += check_contains(&format!("{context} replay"), &tree, &mut seen);
                                done.interpolations += 1;
                            }
                            stages -= 1;
                            if stages > 0 {
                                journal.revert(&mut tree).unwrap();
                                done.checks +=
                                    check_contains(&format!("{context} stage revert"), &tree, &mut seen);
                                journal.discard(&mut tree);
                                done.checks +=
                                    check_contains(&format!("{context} stage discard"), &tree, &mut seen);
                                done.stage_replacements += 1;
                                continue;
                            }
                            if journal.is_empty() {
                                journal.discard(&mut tree);
                                done.checks +=
                                    check_contains(&format!("{context} empty discard"), &tree, &mut seen);
                            } else {
                                entry.push(journal);
                            }
                            break;
                        }
                    }
                    if entry.is_empty() {
                        continue;
                    }
                    let mut branch: Vec<Vec<Journal<TestElement>>> = history.drain(applied..).collect();
                    while !branch.is_empty() {
                        let k = rng.random_range(0..branch.len());
                        for journal in branch.remove(k) {
                            journal.discard(&mut tree);
                            done.checks +=
                                check_contains(&format!("{context} redo branch"), &tree, &mut seen);
                        }
                    }
                    history.push(entry);
                    applied += 1;
                } else if roll < 72 {
                    if applied > 0 {
                        for journal in history[applied - 1].iter().rev() {
                            journal
                                .revert(&mut tree)
                                .unwrap_or_else(|e| panic!("{context}: undo: {e}"));
                            done.checks += check_contains(&format!("{context} undo"), &tree, &mut seen);
                        }
                        applied -= 1;
                        done.undos += 1;
                    }
                } else if roll < 92 {
                    if applied < history.len() {
                        for journal in &history[applied] {
                            journal
                                .replay(&mut tree)
                                .unwrap_or_else(|e| panic!("{context}: redo: {e}"));
                            done.checks += check_contains(&format!("{context} redo"), &tree, &mut seen);
                        }
                        applied += 1;
                        done.redos += 1;
                    }
                } else if applied > 0 {
                    for journal in history.remove(0) {
                        journal.discard(&mut tree);
                        done.checks += check_contains(&format!("{context} trim"), &tree, &mut seen);
                    }
                    applied -= 1;
                    done.trims += 1;
                }
                assert_eq!(tree.check_sorted(), Ok(()), "{context}");
            }
            for _ in 0..rng.random_range(0..=applied) {
                for journal in history[applied - 1].iter().rev() {
                    journal.revert(&mut tree).unwrap();
                    done.checks += check_contains(&format!("seed {seed} final undo"), &tree, &mut seen);
                }
                applied -= 1;
            }
            for entry in history.drain(..) {
                for journal in entry {
                    journal.discard(&mut tree);
                    done.checks += check_contains(&format!("seed {seed} final discard"), &tree, &mut seen);
                }
            }
            let live = tree.arena().iter().filter(|n| !n.is_removed()).count();
            assert_eq!(live, tree.descendants().count(), "seed {seed}: a node leaked");
            assert!(
                tree.detached.is_empty(),
                "seed {seed}: freed nodes are still marked"
            );
        }
        // Preconditions: the sweep did each thing it claims to.
        for (what, count) in [
            ("calls", done.calls),
            ("failed calls", done.failed_calls),
            ("abandons", done.abandons),
            ("interpolations", done.interpolations),
            ("stage replacements", done.stage_replacements),
            ("undos", done.undos),
            ("redos", done.redos),
            ("trims", done.trims),
            ("inserted touches", done.inserted),
            ("removed touches", done.removed),
        ] {
            assert!(count > 0, "precondition: the sweep made {what}: {done:?}");
        }
    }

    #[test]
    fn test_restores_out_of_order_are_refused_and_contains_stays_exact() {
        let mut accepted = 0usize;
        let mut refused = 0usize;
        for seed in 0..SEEDS {
            let mut scenario = structural_scenario(seed);
            let mut tree = scenario.tree.clone();
            let mut rng = StdRng::seed_from_u64(seed ^ 0x5A5A);
            let mut seen = Vec::new();
            let mut journals: Vec<Journal<TestElement>> = Vec::new();
            for step in 0..rng.random_range(4..=14) {
                if journals.is_empty() || rng.random_bool(0.4) {
                    let mut builder = JournalBuilder::new();
                    for _ in 0..rng.random_range(1..=5) {
                        let _ = random_call(&mut rng, &mut scenario, &mut tree, &mut builder);
                    }
                    journals.push(builder.close(&tree));
                } else {
                    let k = rng.random_range(0..journals.len());
                    let result = if rng.random_bool(0.5) {
                        journals[k].revert(&mut tree)
                    } else {
                        journals[k].replay(&mut tree)
                    };
                    match result {
                        Ok(()) => accepted += 1,
                        Err(_) => refused += 1,
                    }
                }
                check_contains(&format!("seed {seed} step {step}"), &tree, &mut seen);
            }
            for journal in journals {
                // A probe, not a lifecycle: what each journal holds is
                // freed with the tree.
                std::mem::forget(journal);
            }
        }
        assert!(accepted > 0, "precondition: restores in order are accepted");
        assert!(
            refused > 0,
            "precondition: restores out of order were tried and refused"
        );
    }
}
