// SPDX-License-Identifier: MPL-2.0

//! Structural commands over [`TestElement`]: each command's errors,
//! with the tree unchanged after every one; the two phases; the
//! journal's exact revert and replay of structure, with the same
//! `NodeId`s; detached subtrees and what [`Journal::discard`] frees;
//! [`Journal::touched`] for whole subtrees; structure snapping in
//! `write_interpolated`; and the cases of R3 (#233, #234) where
//! structural and key moves meet in one builder. The seeded properties
//! that mix every command with key and value changes are in
//! `journal_tests` and `interpolate_tests`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::Snap;
use crate::engine::journal::{children_before, Journal, JournalBuilder, Touch};
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tests::journal_fixtures::{
    run_structural, snapshot, state_above, StructuralRun, StructuralStats, SEEDS,
};
use crate::engine::tests::test_element::{
    add, add_mutator, marked, plain, value, SpatialTestElement, TestElement, TestKind, TestMutation,
};
use crate::engine::tree::{MutatorTree, Placement, Tree, TreeError};
use crate::engine::walker::walk_tree_from;
use indextree::NodeId;
use rustc_hash::{FxHashMap, FxHashSet};

type TestMutatorTree = MutatorTree<TestElement, TestMutation>;

/// A root (value 0) with one `Plain` child per `(channel, value)`, in
/// the order given — each appended at the end of its key's run.
fn flat_tree(children: &[(usize, i64)]) -> (Tree<TestElement>, Vec<NodeId>) {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let ids = children
        .iter()
        .map(|&(channel, v)| add(&mut tree, root, TestElement::new(channel, v)))
        .collect();
    (tree, ids)
}

/// A one-node mutator tree: `op` on a `Plain` root, which applies at
/// every `Plain` anchor.
fn command(op: StructOp<TestElement>) -> TestMutatorTree {
    MutatorTree::new_with(Mutator::new_structure(op, plain(0)))
}

/// `mutators` applied at `anchors`, journaled in `builder`.
fn run(
    tree: &mut Tree<TestElement>,
    mutators: &TestMutatorTree,
    anchors: Vec<NodeId>,
    builder: &mut JournalBuilder<TestElement>,
) -> Result<(), TreeError> {
    mutators.apply_at_journaled(tree, &Anchors::Nodes(anchors), state_above, builder)
}

/// `parent`'s children, in sibling order.
fn row(tree: &Tree<TestElement>, parent: NodeId) -> Vec<NodeId> {
    parent.children(tree.arena()).collect()
}

/// The values of `parent`'s children, in sibling order.
fn row_values(tree: &Tree<TestElement>, parent: NodeId) -> Vec<i64> {
    parent.children(tree.arena()).map(|id| value(tree, id)).collect()
}

/// How many nodes the arena holds, freed slots not counted.
fn live_nodes(tree: &Tree<TestElement>) -> usize {
    tree.arena().iter().filter(|node| !node.is_removed()).count()
}

// ── refusals: each leaves the tree as it was ──────────────────────

#[test]
fn test_structure_move_under_its_own_subtree_is_a_cycle() {
    do_structure_move_under_its_own_subtree_is_a_cycle();
}

/// `a` has a child, which has a child. One walk raises `a`'s children
/// by 5 in its first phase and then moves `a` under its grandchild: the
/// move is a `Cycle`, and the whole walk is undone — the raise included
/// — leaving the builder empty. Moving `a` under itself and moving the
/// root anywhere are cycles too. The control: the same walk moving `a`
/// under its sibling succeeds, raises the child (so the raise is real),
/// and takes `a`'s subtree along.
pub fn do_structure_move_under_its_own_subtree_is_a_cycle() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let (a, b) = (t[0], t[1]);
    let child = add(&mut tree, a, TestElement::new(0, 10));
    let grandchild = add(&mut tree, child, TestElement::new(0, 11));
    let before = snapshot(&tree);
    let raising_move = |to| {
        let mut mutators = command(StructOp::Move {
            to,
            at: Placement::Last,
        });
        let root = mutators.root;
        add_mutator(&mut mutators, root, Mutator::new(TestMutation::Add(5), plain(0)));
        mutators
    };

    let mut builder = JournalBuilder::new();
    let result = run(&mut tree, &raising_move(grandchild), vec![a], &mut builder);
    assert_eq!(
        result,
        Err(TreeError::Cycle {
            node: a,
            to: grandchild
        })
    );
    assert_eq!(
        snapshot(&tree),
        before,
        "the walk's raise is undone with the move"
    );
    assert_eq!(builder.recorded_len(), 0, "and the builder left empty");

    let result = run(&mut tree, &raising_move(a), vec![a], &mut builder);
    assert_eq!(result, Err(TreeError::Cycle { node: a, to: a }));
    let root = tree.root;
    let result = run(&mut tree, &raising_move(b), vec![root], &mut builder);
    assert_eq!(result, Err(TreeError::Cycle { node: root, to: b }));
    assert_eq!(snapshot(&tree), before);

    run(&mut tree, &raising_move(b), vec![a], &mut builder).unwrap();
    assert_eq!(value(&tree, child), 15, "the control's raise applied");
    assert_eq!(tree.arena()[a].parent(), Some(b), "a moved under b");
    assert_eq!(row(&tree, a), vec![child], "with its subtree");
    assert_eq!(row(&tree, child), vec![grandchild]);
    tree.check_sorted().unwrap();
}

#[test]
fn test_structure_a_command_on_a_node_removed_earlier_in_the_walk_fails() {
    do_structure_a_command_on_a_node_removed_earlier_in_the_walk_fails();
}

/// `p` has a child `c`. A `Remove` anchored at both reaches `p` first —
/// the walk is in pre-order — so its second command names a node of a
/// removed subtree: `TargetRemovedEarlier(c)`. The same for a command
/// below the removing one, which reaches `p`'s children, and for a move
/// whose destination an earlier command of the walk removed. Each time
/// the tree is as it was. The controls: each command alone succeeds.
pub fn do_structure_a_command_on_a_node_removed_earlier_in_the_walk_fails() {
    let (mut tree, t) = flat_tree(&[(0, 1), (1, 2)]);
    let (p, y) = (t[0], t[1]);
    let c = add(&mut tree, p, TestElement::new(0, 10));
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();

    let result = run(&mut tree, &command(StructOp::Remove), vec![c, p], &mut builder);
    assert_eq!(result, Err(TreeError::TargetRemovedEarlier(c)));
    assert_eq!(snapshot(&tree), before);

    let mut nested = command(StructOp::Remove);
    let root = nested.root;
    add_mutator(
        &mut nested,
        root,
        Mutator::new_structure(StructOp::Split { at: 4 }, plain(0)),
    );
    let result = run(&mut tree, &nested, vec![p], &mut builder);
    assert_eq!(result, Err(TreeError::TargetRemovedEarlier(c)));
    assert_eq!(snapshot(&tree), before);

    // At the root: remove `p` (channel 0), then move `y` (channel 1),
    // which the merge reaches second, under `p`.
    let mut remove_then_move = MutatorTree::new_with(Mutator::new_void(plain(0)));
    let root = remove_then_move.root;
    add_mutator(
        &mut remove_then_move,
        root,
        Mutator::new_structure(StructOp::Remove, plain(0)),
    );
    add_mutator(
        &mut remove_then_move,
        root,
        Mutator::new_structure(
            StructOp::Move {
                to: p,
                at: Placement::Last,
            },
            plain(1),
        ),
    );
    let tree_root = tree.root;
    let result = run(&mut tree, &remove_then_move, vec![tree_root], &mut builder);
    assert_eq!(result, Err(TreeError::TargetRemovedEarlier(p)));
    assert_eq!(snapshot(&tree), before);

    let mut control = tree.clone();
    run(
        &mut control,
        &command(StructOp::Remove),
        vec![c],
        &mut JournalBuilder::new(),
    )
    .unwrap();
    let mut control = tree.clone();
    run(
        &mut control,
        &command(StructOp::Remove),
        vec![p],
        &mut JournalBuilder::new(),
    )
    .unwrap();
    let mut control = tree.clone();
    let mut move_only = MutatorTree::new_with(Mutator::new_void(plain(0)));
    let root = move_only.root;
    add_mutator(
        &mut move_only,
        root,
        Mutator::new_structure(
            StructOp::Move {
                to: p,
                at: Placement::Last,
            },
            plain(1),
        ),
    );
    run(
        &mut control,
        &move_only,
        vec![tree_root],
        &mut JournalBuilder::new(),
    )
    .unwrap();
    assert_eq!(control.arena()[y].parent(), Some(p));
}

#[test]
fn test_structure_merge_refuses_a_sibling_with_children() {
    do_structure_merge_refuses_a_sibling_with_children();
}

/// Row `a b c` keyed `0 0 1`, and `b` has a child. `a` cannot absorb
/// `b`, which has children; `b` cannot absorb `c`, whose key differs;
/// `c` has no next sibling. Each is `Unmergeable`, naming the target,
/// and changes nothing. The control: once `b`'s child is removed, `a`
/// absorbs `b`.
pub fn do_structure_merge_refuses_a_sibling_with_children() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (1, 30)]);
    let (a, b, c) = (t[0], t[1], t[2]);
    let kid = add(&mut tree, b, TestElement::new(0, 20));
    let root = tree.root;
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    for target in [a, b, c] {
        let result = run(&mut tree, &command(StructOp::Merge), vec![target], &mut builder);
        assert_eq!(result, Err(TreeError::Unmergeable(target)));
        assert_eq!(snapshot(&tree), before);
    }

    run(&mut tree, &command(StructOp::Remove), vec![kid], &mut builder).unwrap();
    run(&mut tree, &command(StructOp::Merge), vec![a], &mut builder).unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 30], "a absorbed b");
    assert_eq!(row(&tree, root), vec![a, c]);
}

#[test]
fn test_structure_refusals_name_what_is_wrong_and_change_nothing() {
    do_structure_refusals_name_what_is_wrong_and_change_nothing();
}

/// The remaining refusals, each against the same tree, which none of
/// them changes: removing the root; splitting the root, or an element
/// that cannot be cut there (at its whole value, at 0, a value of 1);
/// inserting `Before` a sibling of another key, or `After` a node under
/// another parent; moving `Before` the moved node itself, or `After` a
/// sibling of another key; moving under a freed node; and a freed
/// target, refused before the walk.
pub fn do_structure_refusals_name_what_is_wrong_and_change_nothing() {
    let (mut tree, t) = flat_tree(&[(0, 5), (1, 1)]);
    let (a, b) = (t[0], t[1]);
    let nephew = add(&mut tree, b, TestElement::new(0, 7));
    let root = tree.root;
    let gone = add(&mut tree, root, TestElement::new(2, 9));
    tree.remove_subtree(gone).unwrap();
    let before = snapshot(&tree);
    let leaf = |channel| Subtree::leaf(TestElement::new(channel, 100));
    let cases: Vec<(TestMutatorTree, NodeId, TreeError)> = vec![
        (command(StructOp::Remove), root, TreeError::RemoveRoot),
        (
            command(StructOp::Split { at: 2 }),
            root,
            TreeError::Unsplittable(root),
        ),
        (command(StructOp::Split { at: 5 }), a, TreeError::Unsplittable(a)),
        (command(StructOp::Split { at: 0 }), a, TreeError::Unsplittable(a)),
        (command(StructOp::Split { at: 1 }), b, TreeError::Unsplittable(b)),
        (
            command(StructOp::Insert {
                subtree: leaf(0),
                at: Placement::Before(b),
            }),
            root,
            TreeError::PlacementCrossesKey,
        ),
        (
            command(StructOp::Insert {
                subtree: leaf(0),
                at: Placement::After(nephew),
            }),
            root,
            TreeError::NotAChild {
                parent: root,
                node: nephew,
            },
        ),
        (
            command(StructOp::Move {
                to: root,
                at: Placement::Before(a),
            }),
            a,
            TreeError::NotAChild {
                parent: root,
                node: a,
            },
        ),
        (
            command(StructOp::Move {
                to: root,
                at: Placement::After(b),
            }),
            a,
            TreeError::PlacementCrossesKey,
        ),
        (
            command(StructOp::Move {
                to: gone,
                at: Placement::Last,
            }),
            a,
            TreeError::StaleNode(gone),
        ),
        (command(StructOp::Remove), gone, TreeError::StaleNode(gone)),
    ];
    for (mutators, target, error) in cases {
        let mut builder = JournalBuilder::new();
        let result = run(&mut tree, &mutators, vec![target], &mut builder);
        assert_eq!(result, Err(error), "{mutators:?} at {target}");
        assert_eq!(snapshot(&tree), before, "{error} changed nothing");
    }
}

// ── the two phases ────────────────────────────────────────────────

#[test]
fn test_structure_commands_run_after_the_walk() {
    do_structure_commands_run_after_the_walk();
}

/// One walk at the root inserts a channel-0 child and raises every
/// channel-0 child by 1. The raise runs in the walk and the insert after
/// it, so the inserted child keeps the value it was inserted with while
/// the children that were there are raised. A walk that inserted as it
/// went would have reached the new child too.
pub fn do_structure_commands_run_after_the_walk() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let root = tree.root;
    let mut mutators = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 100)),
        at: Placement::Last,
    });
    let mroot = mutators.root;
    add_mutator(&mut mutators, mroot, Mutator::new(TestMutation::Add(1), plain(0)));
    run(&mut tree, &mutators, vec![root], &mut JournalBuilder::new()).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 100]);
    assert_eq!(
        &row(&tree, root)[..2],
        &t[..],
        "the old children keep their ids and places"
    );
}

#[test]
fn test_structure_inserts_land_where_their_placement_says() {
    do_structure_inserts_land_where_their_placement_says();
}

/// Row `a b | c` keyed `0 0 | 1`. Channel-0 inserts at `First`, `Last`,
/// `Before(b)` and `After(a)` land at the start of the channel-0 run, its
/// end, in front of `b` and behind `a`; a channel-1 insert at `First`
/// lands in front of `c`, not at the front of the row. A subtree's
/// children are put in key order whatever order they were given in.
/// Every row stays sorted.
pub fn do_structure_inserts_land_where_their_placement_says() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (1, 3)]);
    let (a, b) = (t[0], t[1]);
    let root = tree.root;
    let insert = |channel, value, at| {
        command(StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(channel, value)),
            at,
        })
    };
    let mut builder = JournalBuilder::new();
    for (mutators, expected) in [
        (insert(0, 10, Placement::First), vec![10, 1, 2, 3]),
        (insert(0, 20, Placement::Last), vec![10, 1, 2, 20, 3]),
        (insert(0, 30, Placement::Before(b)), vec![10, 1, 30, 2, 20, 3]),
        (insert(0, 40, Placement::After(a)), vec![10, 1, 40, 30, 2, 20, 3]),
        (insert(1, 50, Placement::First), vec![10, 1, 40, 30, 2, 20, 50, 3]),
    ] {
        run(&mut tree, &mutators, vec![root], &mut builder).unwrap();
        assert_eq!(row_values(&tree, root), expected);
        tree.check_sorted().unwrap();
    }

    let subtree = Subtree {
        root: TestElement::new(0, 60),
        children: vec![
            Subtree::leaf(TestElement::new(2, 61)),
            Subtree::leaf(TestElement::marked(0, 62)),
            Subtree::leaf(TestElement::new(0, 63)),
            Subtree::leaf(TestElement::new(2, 64)),
        ],
    };
    run(
        &mut tree,
        &command(StructOp::Insert {
            subtree,
            at: Placement::Last,
        }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    let inserted = row(&tree, a)[0];
    assert_eq!(
        row_values(&tree, inserted),
        vec![63, 61, 64, 62],
        "children in key order"
    );
    tree.check_sorted().unwrap();
}

// ── exact revert and replay ───────────────────────────────────────

#[test]
fn test_structure_revert_and_replay_put_back_the_same_nodes() {
    do_structure_revert_and_replay_put_back_the_same_nodes();
}

/// One builder inserts a subtree, removes a node with a child, moves a
/// node under another parent and splits one. `revert` gives back the
/// tree from before, and `replay` the tree from after — the same
/// `NodeId`s in the same places, the removed child and the inserted
/// subtree's nodes included — twice over.
pub fn do_structure_revert_and_replay_put_back_the_same_nodes() {
    let (mut tree, t) = flat_tree(&[(0, 5), (0, 6), (1, 7)]);
    let (a, b, c) = (t[0], t[1], t[2]);
    let kid = add(&mut tree, b, TestElement::new(0, 60));
    let root = tree.root;
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    let subtree = Subtree {
        root: TestElement::new(0, 9),
        children: vec![Subtree::leaf(TestElement::new(0, 90))],
    };
    run(
        &mut tree,
        &command(StructOp::Insert {
            subtree,
            at: Placement::First,
        }),
        vec![c],
        &mut builder,
    )
    .unwrap();
    run(&mut tree, &command(StructOp::Remove), vec![b], &mut builder).unwrap();
    let moved = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Move {
            to: c,
            at: Placement::Last,
        },
        plain(0),
    ));
    run(&mut tree, &moved, vec![a], &mut builder).unwrap();
    run(
        &mut tree,
        &command(StructOp::Split { at: 2 }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    let after = snapshot(&tree);
    assert_eq!(row(&tree, root), vec![c]);
    assert_eq!(row_values(&tree, c), vec![9, 2, 3]);
    assert!(
        !tree.contains(kid) && tree.get(kid).is_some(),
        "the removed child is held, not freed"
    );
    let journal = builder.close(&tree);
    for cycle in 0..2 {
        journal.revert(&mut tree).unwrap();
        assert_eq!(snapshot(&tree), before, "cycle {cycle}: revert");
        assert_eq!(
            row(&tree, b),
            vec![kid],
            "cycle {cycle}: b came back with its child"
        );
        journal.replay(&mut tree).unwrap();
        assert_eq!(snapshot(&tree), after, "cycle {cycle}: replay, with the same ids");
        tree.check_sorted().unwrap();
    }
}

#[test]
fn test_structure_split_then_merge_round_trips_a_value() {
    do_structure_split_then_merge_round_trips_a_value();
}

/// `a` (7, with a child) is split at 3: `a` keeps 3 and its child, and
/// a new childless 4 lands right behind it. Merging `a` again absorbs
/// the 4: `a` holds 7, the second part is out of the tree, and the
/// builder's journal — a split and a merge that cancel — is empty,
/// though it still owns the second part, which discarding frees. A split
/// on its own reverts to 7 and replays to 3 and 4 with the same new id.
pub fn do_structure_split_then_merge_round_trips_a_value() {
    let (mut tree, t) = flat_tree(&[(0, 7), (1, 9)]);
    let a = t[0];
    let kid = add(&mut tree, a, TestElement::new(0, 50));
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    run(
        &mut tree,
        &command(StructOp::Split { at: 3 }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 4, 9]);
    let second = row(&tree, root)[1];
    assert_eq!(row(&tree, root)[0], a, "a keeps the first part");
    assert_eq!(row(&tree, a), vec![kid], "and its children");
    assert!(row(&tree, second).is_empty(), "the second part has none");
    run(&mut tree, &command(StructOp::Merge), vec![a], &mut builder).unwrap();
    assert_eq!(row_values(&tree, root), vec![7, 9]);
    assert_eq!(row(&tree, a), vec![kid]);
    assert!(!tree.contains(second) && tree.get(second).is_some());
    let journal = builder.close(&tree);
    assert!(journal.is_empty(), "the split and the merge cancel");
    journal.discard(&mut tree);
    assert!(
        tree.get(second).is_none(),
        "the empty journal still freed the second part"
    );

    let mut builder = JournalBuilder::new();
    run(
        &mut tree,
        &command(StructOp::Split { at: 2 }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    let after = snapshot(&tree);
    let second = row(&tree, root)[1];
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![7, 9]);
    journal.replay(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), after);
    assert_eq!(row(&tree, root)[1], second, "the same second part");
}

// ── R3: structure and key moves in one builder ────────────────────

#[test]
fn test_structure_insert_then_rekey_reverts_and_replays_exactly() {
    do_structure_insert_then_rekey_reverts_and_replays_exactly();
}

/// Row `A B C` on channel 0. One builder inserts `Z` after `A`, then
/// moves `A` to channel 1: `Z B C A`. Replaying the two as a log of
/// moves would be exact only with the insert first; restoring the
/// arrangements is exact either way. `revert` gives `A B C`, `replay`
/// `Z B C A`, with the same ids.
pub fn do_structure_insert_then_rekey_reverts_and_replays_exactly() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3)]);
    let (a, b, c) = (t[0], t[1], t[2]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    let insert_z = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 26)),
        at: Placement::After(a),
    });
    run(&mut tree, &insert_z, vec![root], &mut builder).unwrap();
    let z = row(&tree, root)[1];
    assert_eq!(row(&tree, root), vec![a, z, b, c]);
    tree.update_journaled(a, &mut builder, |e| e.channel = 1).unwrap();
    assert_eq!(row(&tree, root), vec![z, b, c, a]);
    let journal = builder.close(&tree);
    for _ in 0..2 {
        journal.revert(&mut tree).unwrap();
        assert_eq!(row(&tree, root), vec![a, b, c]);
        assert_eq!(tree.arena()[a].get().channel, 0);
        journal.replay(&mut tree).unwrap();
        assert_eq!(row(&tree, root), vec![z, b, c, a]);
        tree.check_sorted().unwrap();
    }
}

#[test]
fn test_structure_rekey_then_insert_reverts_and_replays_exactly() {
    do_structure_rekey_then_insert_reverts_and_replays_exactly();
}

/// Row `A B C` on channel 0. One builder moves `A` to channel 1 (`B C
/// A`), then inserts `Z` — on channel 1 — after `A`: `B C A Z`.
/// Replaying the two as a log of moves would be exact only with the
/// key move first. `revert` gives `A B C`, `replay` `B C A Z`.
pub fn do_structure_rekey_then_insert_reverts_and_replays_exactly() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3)]);
    let (a, b, c) = (t[0], t[1], t[2]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    tree.update_journaled(a, &mut builder, |e| e.channel = 1).unwrap();
    assert_eq!(row(&tree, root), vec![b, c, a]);
    let insert_z = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(1, 26)),
        at: Placement::After(a),
    });
    run(&mut tree, &insert_z, vec![root], &mut builder).unwrap();
    let z = row(&tree, root)[3];
    assert_eq!(row(&tree, root), vec![b, c, a, z]);
    let journal = builder.close(&tree);
    for _ in 0..2 {
        journal.revert(&mut tree).unwrap();
        assert_eq!(row(&tree, root), vec![a, b, c]);
        journal.replay(&mut tree).unwrap();
        assert_eq!(row(&tree, root), vec![b, c, a, z]);
        tree.check_sorted().unwrap();
    }
}

#[test]
fn test_structure_a_reparented_node_is_restored_against_both_parents() {
    do_structure_a_reparented_node_is_restored_against_both_parents();
}

/// Rows `p: [x y]` and `q: [u v]`. One builder moves `x` to the end of
/// `q`, then removes `u`, then moves `y` in front of `x` in `q`: `p` is
/// empty and `q` is `v y x`. Every position is recorded against the
/// arrangement the builder started from, whichever parent it is in, so
/// `revert` gives back `p: [x y]`, `q: [u v]` and `replay` `q: [v y x]`.
pub fn do_structure_a_reparented_node_is_restored_against_both_parents() {
    let (mut tree, t) = flat_tree(&[(0, 1), (1, 2)]);
    let (p, q) = (t[0], t[1]);
    let x = add(&mut tree, p, TestElement::new(0, 10));
    let y = add(&mut tree, p, TestElement::new(0, 11));
    let u = add(&mut tree, q, TestElement::new(0, 20));
    let v = add(&mut tree, q, TestElement::new(0, 21));
    let before = snapshot(&tree);
    let move_to = |at| command(StructOp::Move { to: q, at });
    let mut builder = JournalBuilder::new();
    run(&mut tree, &move_to(Placement::Last), vec![x], &mut builder).unwrap();
    run(&mut tree, &command(StructOp::Remove), vec![u], &mut builder).unwrap();
    run(&mut tree, &move_to(Placement::Before(x)), vec![y], &mut builder).unwrap();
    assert!(row(&tree, p).is_empty());
    assert_eq!(row(&tree, q), vec![v, y, x]);
    let after = snapshot(&tree);
    let journal = builder.close(&tree);
    for _ in 0..2 {
        journal.revert(&mut tree).unwrap();
        assert_eq!(snapshot(&tree), before);
        journal.replay(&mut tree).unwrap();
        assert_eq!(snapshot(&tree), after);
    }
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(x, Touch::Moved), (u, Touch::Removed), (y, Touch::Moved)]
    );
}

// ── touched ───────────────────────────────────────────────────────

#[test]
fn test_structure_touched_names_whole_subtrees_in_pre_order() {
    do_structure_touched_names_whole_subtrees_in_pre_order();
}

/// One builder inserts a four-node subtree under `a`, removes `b` with
/// its two-node subtree, and moves `c` under `a`. `touched` names the
/// inserted subtree node by node in pre-order, all `Inserted`; the
/// removed one likewise, all `Removed`; and `c` once, `Moved` — its
/// child moved with it and is not named. A split names its target as a
/// `Field` and the new part as `Inserted`; a merge its target as a
/// `Field` and the absorbed node as `Removed`.
pub fn do_structure_touched_names_whole_subtrees_in_pre_order() {
    let (mut tree, t) = flat_tree(&[(0, 5), (0, 2), (1, 3)]);
    let (a, b, c) = (t[0], t[1], t[2]);
    let b1 = add(&mut tree, b, TestElement::new(0, 20));
    let b2 = add(&mut tree, b1, TestElement::new(0, 21));
    let c1 = add(&mut tree, c, TestElement::new(0, 30));
    let subtree = Subtree {
        root: TestElement::new(0, 10),
        children: vec![
            Subtree {
                root: TestElement::new(0, 11),
                children: vec![Subtree::leaf(TestElement::new(0, 12))],
            },
            Subtree::leaf(TestElement::new(1, 13)),
        ],
    };
    let mut builder = JournalBuilder::new();
    run(
        &mut tree,
        &command(StructOp::Insert {
            subtree,
            at: Placement::Last,
        }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    let s = row(&tree, a)[0];
    let s1 = row(&tree, s)[0];
    let s2 = row(&tree, s1)[0];
    let s3 = row(&tree, s)[1];
    run(&mut tree, &command(StructOp::Remove), vec![b], &mut builder).unwrap();
    let move_c = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Move {
            to: a,
            at: Placement::Last,
        },
        plain(1),
    ));
    run(&mut tree, &move_c, vec![c], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![
            (s, Touch::Inserted),
            (s1, Touch::Inserted),
            (s2, Touch::Inserted),
            (s3, Touch::Inserted),
            (b, Touch::Removed),
            (b1, Touch::Removed),
            (b2, Touch::Removed),
            (c, Touch::Moved),
        ]
    );
    assert!(
        journal.touched().all(|(id, _)| id != c1),
        "c's child moved with it"
    );

    let root = tree.root;
    let mut builder = JournalBuilder::new();
    run(
        &mut tree,
        &command(StructOp::Split { at: 1 }),
        vec![a],
        &mut builder,
    )
    .unwrap();
    let second = row(&tree, root)[1];
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(a, Touch::Field), (second, Touch::Inserted)]
    );
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Merge), vec![a], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(a, Touch::Field), (second, Touch::Removed)]
    );
}

#[test]
fn test_structure_touched_reads_each_node_s_own_membership() {
    do_structure_touched_reads_each_node_s_own_membership();
}

/// What a node's touch is depends on where it was before and is after,
/// not on which command reached it. One builder inserts `s` and then
/// moves `x` — in the tree before — into it: `s` is `Inserted`, `x`
/// `Moved` though it now sits in an inserted subtree, and `x`'s child
/// is not named. Another inserts `s` and removes it again: no one saw
/// `s`, the journal names nothing and is empty, yet it owns `s`.
pub fn do_structure_touched_reads_each_node_s_own_membership() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let (a, x) = (t[0], t[1]);
    let x1 = add(&mut tree, x, TestElement::new(0, 20));
    let insert = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 10)),
        at: Placement::Last,
    });
    let mut builder = JournalBuilder::new();
    run(&mut tree, &insert, vec![a], &mut builder).unwrap();
    let s = row(&tree, a)[0];
    let into_s = command(StructOp::Move {
        to: s,
        at: Placement::Last,
    });
    run(&mut tree, &into_s, vec![x], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(s, Touch::Inserted), (x, Touch::Moved)]
    );
    assert!(journal.touched().all(|(id, _)| id != x1));
    journal.revert(&mut tree).unwrap();

    let mut builder = JournalBuilder::new();
    run(&mut tree, &insert, vec![a], &mut builder).unwrap();
    let s = row(&tree, a)[0];
    let raise = MutatorTree::new_with(Mutator::new(TestMutation::Add(5), plain(0)));
    run(&mut tree, &raise, vec![s], &mut builder).unwrap();
    assert_eq!(value(&tree, s), 15, "s was changed while it was in the tree");
    run(&mut tree, &command(StructOp::Remove), vec![s], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(journal.touched().count(), 0, "s was never seen");
    assert!(journal.is_empty());
    assert!(tree.get(s).is_some(), "but s is held");
    journal.discard(&mut tree);
    assert!(tree.get(s).is_none(), "until the journal is discarded");
}

// ── detached subtrees and discard ─────────────────────────────────

#[test]
fn test_structure_discard_stales_a_detached_node_and_keeps_an_attached_one() {
    do_structure_discard_stales_a_detached_node_and_keeps_an_attached_one();
}

/// A journal removes `a`, which has a child. Until the journal is
/// discarded `a` and its child are detached but allocated — out of the
/// tree, and refused by `update` — and their ids still name them.
/// After `discard` both ids are stale, and stay stale when a new node
/// takes the freed slot; the negative control, `a`'s sibling `b`, which
/// the journal changed but did not detach, is still in the tree.
pub fn do_structure_discard_stales_a_detached_node_and_keeps_an_attached_one() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let (a, b) = (t[0], t[1]);
    let kid = add(&mut tree, a, TestElement::new(0, 10));
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![a], &mut builder).unwrap();
    tree.update_journaled(b, &mut builder, |e| e.value = 20).unwrap();
    let journal = builder.close(&tree);
    assert!(tree.get(a).is_some() && tree.get(kid).is_some(), "held");
    assert!(!tree.contains(a) && !tree.contains(kid), "out of the tree");
    assert_eq!(
        tree.update(kid, |e| e.value = 0),
        Err(TreeError::StaleNode(kid)),
        "a held node is not changed"
    );
    assert_eq!(tree.update(b, |e| e.value = 21), Ok(()), "an attached one is");

    journal.discard(&mut tree);
    assert!(tree.get(a).is_none(), "a is stale");
    assert!(tree.get(kid).is_none(), "and so is its child");
    assert!(tree.contains(b), "b, still attached, is not freed");
    let reused = add(&mut tree, root, TestElement::new(0, 3));
    let reused_too = add(&mut tree, root, TestElement::new(0, 4));
    assert!(
        tree.get(a).is_none() && tree.get(kid).is_none(),
        "the old ids stay stale"
    );
    assert!(tree.contains(reused) && tree.contains(reused_too));
}

#[test]
fn test_structure_discard_frees_by_the_side_last_written() {
    do_structure_discard_frees_by_the_side_last_written();
}

/// A journal inserts `s` and removes `r`. Discarded as it closed — the
/// way the oldest history entry leaves — it frees `r` and keeps `s`,
/// which is in the tree. Discarded after a revert — the way a cleared
/// redo branch leaves — it frees `s` and keeps `r`, which the revert put
/// back. The same holds after `write_interpolated` past and short of
/// the snap point.
pub fn do_structure_discard_frees_by_the_side_last_written() {
    for (case, write) in ["closed", "reverted", "written after", "written before"]
        .into_iter()
        .enumerate()
    {
        let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
        let r = t[1];
        let root = tree.root;
        let mut builder = JournalBuilder::new();
        let insert = command(StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 10)),
            at: Placement::Last,
        });
        run(&mut tree, &insert, vec![root], &mut builder).unwrap();
        let s = *row(&tree, root).last().unwrap();
        run(&mut tree, &command(StructOp::Remove), vec![r], &mut builder).unwrap();
        let journal = builder.close(&tree);
        let applied = match case {
            0 => true,
            1 => {
                journal.revert(&mut tree).unwrap();
                false
            }
            2 => {
                journal.write_interpolated(&mut tree, 0.5, Snap::Start).unwrap();
                true
            }
            _ => {
                journal.write_interpolated(&mut tree, 0.5, Snap::End).unwrap();
                false
            }
        };
        journal.discard(&mut tree);
        let (kept, freed) = if applied { (s, r) } else { (r, s) };
        assert!(tree.contains(kept), "{write}: {kept} is in the tree and kept");
        assert!(tree.get(freed).is_none(), "{write}: {freed} is freed");
    }
}

#[test]
fn test_structure_discard_leaves_a_subtree_a_later_journal_removed() {
    do_structure_discard_leaves_a_subtree_a_later_journal_removed();
}

/// `j1` inserts `s`; `j2`, later, removes it. When `j1` leaves the
/// history — the oldest entry, applied — `s` is detached, but `j2`
/// detached it, not `j1`: `j1` must not free it, or undoing `j2` would
/// find it stale. The control: discarding `j2` frees it.
pub fn do_structure_discard_leaves_a_subtree_a_later_journal_removed() {
    let (mut tree, _) = flat_tree(&[(0, 1)]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    let insert = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 10)),
        at: Placement::Last,
    });
    run(&mut tree, &insert, vec![root], &mut builder).unwrap();
    let s = row(&tree, root)[1];
    let j1 = builder.close(&tree);
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![s], &mut builder).unwrap();
    let j2: Journal<TestElement> = builder.close(&tree);
    let held = snapshot(&tree);

    j1.discard(&mut tree);
    assert!(
        tree.get(s).is_some(),
        "j1 did not detach s, so it does not free it"
    );
    assert_eq!(snapshot(&tree), held);
    let undo = j2.clone();
    undo.revert(&mut tree).unwrap();
    assert_eq!(row(&tree, root)[1], s, "undoing j2 puts s back");
    undo.replay(&mut tree).unwrap();
    j2.discard(&mut tree);
    assert!(tree.get(s).is_none(), "j2 detached s, and frees it");
}

#[test]
fn test_structure_discard_frees_only_what_is_still_detached() {
    do_structure_discard_frees_only_what_is_still_detached();
}

/// A journal removes `r` and is discarded as it closed, so `r` is what
/// it holds — but a copy of the journal has been reverted in the
/// meantime, which put `r` back. `discard` frees only a subtree that is
/// still detached, so `r`, attached again, stays in the tree. The
/// control: once the copy is replayed and `r` is detached again,
/// discarding frees it.
pub fn do_structure_discard_frees_only_what_is_still_detached() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let r = t[1];
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![r], &mut builder).unwrap();
    let journal = builder.close(&tree);
    let copy = journal.clone();
    copy.revert(&mut tree).unwrap();
    assert!(tree.contains(r), "the copy put r back");
    journal.clone().discard(&mut tree);
    assert!(tree.contains(r), "r, attached again, is not freed");
    copy.replay(&mut tree).unwrap();
    journal.discard(&mut tree);
    assert!(tree.get(r).is_none(), "detached again, r is freed");
}

#[test]
fn test_structure_refuses_a_split_or_merge_that_changes_the_key() {
    do_structure_refuses_a_split_or_merge_that_changes_the_key();
}

/// An element type that breaks `split`'s and `merge`'s contract — its
/// parts or its merged element on another channel — is refused with
/// `PlacementCrossesKey`, and the tree is unchanged: the second part
/// would land outside the run of the first's key. `SpatialTestElement`
/// misbehaves this way on purpose. The control: the merge's target has
/// a childless next sibling of its key, so only the key check refuses
/// it.
pub fn do_structure_refuses_a_split_or_merge_that_changes_the_key() {
    let mut tree = Tree::new_non_indexed_with(SpatialTestElement::unboxed(0, 0));
    let root = tree.root;
    let a = tree
        .insert_child(root, SpatialTestElement::unboxed(0, 1), Placement::Last)
        .unwrap();
    let b = tree
        .insert_child(root, SpatialTestElement::unboxed(0, 2), Placement::Last)
        .unwrap();
    let before: Vec<(NodeId, SpatialTestElement)> = tree
        .descendants()
        .map(|id| (id, tree.arena()[id].get().clone()))
        .collect();
    let snapshot = |tree: &Tree<SpatialTestElement>| -> Vec<(NodeId, SpatialTestElement)> {
        tree.descendants()
            .map(|id| (id, tree.arena()[id].get().clone()))
            .collect()
    };
    assert_eq!(tree.arena()[b].get().key(), tree.arena()[a].get().key());
    assert!(tree.arena()[b].first_child().is_none());
    for op in [StructOp::Split { at: () }, StructOp::Merge] {
        let mutators: MutatorTree<SpatialTestElement, TestMutation> =
            MutatorTree::new_with(Mutator::new_structure(op, 0));
        let result = mutators.apply_at_journaled(
            &mut tree,
            &Anchors::Nodes(vec![a]),
            |_, _| 0,
            &mut JournalBuilder::new(),
        );
        assert_eq!(result, Err(TreeError::PlacementCrossesKey));
        assert_eq!(snapshot(&tree), before);
    }
}

// ── structure part-way ────────────────────────────────────────────

#[test]
fn test_structure_snaps_in_write_interpolated() {
    do_structure_snaps_in_write_interpolated();
}

/// One builder removes `a`, inserts `z` and raises `b` from 10 to 30.
/// Halfway, `b` is 20 either way; with `Snap::Start` the structure is
/// already changed — `a` out, `z` in — and with `Snap::End` it is not.
/// At 0 and 1 the tree is the reverted and the replayed one.
pub fn do_structure_snaps_in_write_interpolated() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 10)]);
    let (a, b) = (t[0], t[1]);
    let root = tree.root;
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![a], &mut builder).unwrap();
    let insert = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 50)),
        at: Placement::Last,
    });
    run(&mut tree, &insert, vec![root], &mut builder).unwrap();
    let z = row(&tree, root)[1];
    tree.update_journaled(b, &mut builder, |e| e.value = 30).unwrap();
    let after = snapshot(&tree);
    let journal = builder.close(&tree);

    journal.write_interpolated(&mut tree, 0.5, Snap::Start).unwrap();
    assert_eq!(row(&tree, root), vec![b, z]);
    assert_eq!(value(&tree, b), 20);
    journal.write_interpolated(&mut tree, 0.5, Snap::End).unwrap();
    assert_eq!(row(&tree, root), vec![a, b]);
    assert_eq!(value(&tree, b), 20);
    journal.write_interpolated(&mut tree, 0.0, Snap::Start).unwrap();
    assert_eq!(snapshot(&tree), before);
    journal.write_interpolated(&mut tree, 1.0, Snap::End).unwrap();
    assert_eq!(snapshot(&tree), after);
}

// ── walks without a journal ───────────────────────────────────────

#[test]
fn test_structure_without_a_journal_is_all_or_nothing_and_frees_what_it_removes() {
    do_structure_without_a_journal_is_all_or_nothing_and_frees_what_it_removes();
}

/// A walk with no journal still runs its commands all or nothing. One
/// that raises the children and then fails on a merge leaves the tree
/// as it was, with no node left allocated by the insert before the
/// failing command — `apply_at` returns the error, `apply_to` logs it.
/// One that succeeds removes `a` for good: nothing can put it back, so
/// its id is stale.
pub fn do_structure_without_a_journal_is_all_or_nothing_and_frees_what_it_removes() {
    let (mut tree, t) = flat_tree(&[(0, 1), (1, 2)]);
    let (a, b) = (t[0], t[1]);
    let root = tree.root;
    let before = snapshot(&tree);
    let live = live_nodes(&tree);
    let mut failing = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, 9)),
            at: Placement::Last,
        },
        plain(0),
    ));
    let mroot = failing.root;
    add_mutator(&mut failing, mroot, Mutator::new(TestMutation::Add(5), plain(0)));
    add_mutator(
        &mut failing,
        mroot,
        Mutator::new_structure(StructOp::Merge, plain(1)),
    );
    let result = failing.apply_at(&mut tree, &Anchors::Nodes(vec![root]), state_above);
    assert_eq!(result, Err(TreeError::Unmergeable(b)));
    assert_eq!(snapshot(&tree), before);
    assert_eq!(live_nodes(&tree), live, "the inserted node was freed");
    failing.apply_to(&mut tree, &0);
    assert_eq!(snapshot(&tree), before);
    assert_eq!(live_nodes(&tree), live);

    command(StructOp::Remove)
        .apply_at(&mut tree, &Anchors::Nodes(vec![a]), state_above)
        .unwrap();
    assert_eq!(row(&tree, root), vec![b]);
    assert!(tree.get(a).is_none(), "a removed without a journal is freed");
}

// ── serde ─────────────────────────────────────────────────────────

#[test]
fn test_structure_is_never_serialized() {
    do_structure_is_never_serialized();
}

/// A structural mutator has no JSON form: serializing one is refused
/// by name, and a `Structure` object is an unknown variant. The
/// controls: a `Single` serializes and reads back, and a `Void` written
/// with the same key spelling as the `Structure` object reads — so the
/// refusal is the variant's, not the key's.
pub fn do_structure_is_never_serialized() {
    let structure: Mutator<TestElement, TestMutation> = Mutator::new_structure(StructOp::Remove, plain(0));
    let written = serde_json::to_string(&structure).unwrap_err().to_string();
    assert!(written.contains("Structure"), "{written}");
    let key = r#"{"kind":"Plain","channel":0}"#;
    let read = serde_json::from_str::<Mutator<TestElement, TestMutation>>(&format!(
        r#"{{"Structure":{{"key":{key}}}}}"#
    ))
    .unwrap_err()
    .to_string();
    assert!(read.contains("unknown variant `Structure`"), "{read}");
    let void: Mutator<TestElement, TestMutation> =
        serde_json::from_str(&format!(r#"{{"Void":{{"key":{key}}}}}"#)).unwrap();
    assert_eq!(void.key(), plain(0));
    let single: Mutator<TestElement, TestMutation> = Mutator::new(TestMutation::Add(1), plain(0));
    let json = serde_json::to_string(&single).unwrap();
    let back: Mutator<TestElement, TestMutation> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.key(), plain(0));
}

// ── bench body ────────────────────────────────────────────────────

#[test]
fn test_structure_batch_on_a_wide_tree() {
    do_structure_batch_on_a_wide_tree();
}

/// The batch a structural edit makes: 64 children on two kinds and
/// four channels, each with a child of its own. One journaled walk splits
/// every `Plain` child, inserts a subtree under each `Marked` one and
/// moves the first `Marked` child under the last `Plain` one; a second
/// merges the split parts back and removes every other `Marked` child.
/// The journal reverts to the tree it started from and replays to the
/// one it closed on.
pub fn do_structure_batch_on_a_wide_tree() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    for i in 0..64 {
        let element = if i % 2 == 0 {
            TestElement::new(i % 4, 10 + i as i64)
        } else {
            TestElement::marked(i % 4, 10 + i as i64)
        };
        let child = add(&mut tree, root, element);
        add(&mut tree, child, TestElement::new(0, 1));
    }
    let plains: Vec<NodeId> = row(&tree, root)
        .into_iter()
        .filter(|&id| tree.arena()[id].get().kind == TestKind::Plain)
        .collect();
    let markeds: Vec<NodeId> = row(&tree, root)
        .into_iter()
        .filter(|&id| !plains.contains(&id))
        .collect();
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    run(
        &mut tree,
        &command(StructOp::Split { at: 3 }),
        plains.clone(),
        &mut builder,
    )
    .unwrap();
    let insert = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Insert {
            subtree: Subtree {
                root: TestElement::new(1, 5),
                children: vec![Subtree::leaf(TestElement::new(0, 6))],
            },
            at: Placement::Last,
        },
        marked(0),
    ));
    run(&mut tree, &insert, markeds.clone(), &mut builder).unwrap();
    let last_plain = *plains.last().unwrap();
    let move_first = MutatorTree::new_with(Mutator::new_structure(
        StructOp::Move {
            to: last_plain,
            at: Placement::Last,
        },
        marked(0),
    ));
    run(&mut tree, &move_first, vec![markeds[0]], &mut builder).unwrap();
    run(&mut tree, &command(StructOp::Merge), plains.clone(), &mut builder).unwrap();
    let removed: Vec<NodeId> = markeds.iter().copied().skip(1).step_by(2).collect();
    let remove = MutatorTree::new_with(Mutator::new_structure(StructOp::Remove, marked(0)));
    run(&mut tree, &remove, removed.clone(), &mut builder).unwrap();
    tree.check_sorted().unwrap();
    assert_eq!(row(&tree, root).len(), 64 - 1 - removed.len());
    let after = snapshot(&tree);
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), before);
    journal.replay(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), after);
}

// ── the discard property ──────────────────────────────────────────

#[test]
fn test_structure_discard_frees_exactly_what_the_journal_holds() {
    do_structure_discard_frees_exactly_what_the_journal_holds();
}

/// For every seed of the structural runs, a journal discarded as it
/// closed (even seeds) or after a revert (odd seeds) leaves the tree
/// exactly as it was, and frees every node that is out of the tree on
/// that side and in it on the other: the removed ones when it is
/// applied, the inserted ones when it is reverted. The snapshots it is
/// checked against come from the tree, not from the journal. The run
/// must free nodes on both sides.
pub fn do_structure_discard_frees_exactly_what_the_journal_holds() {
    let mut stats = StructuralStats::default();
    let (mut freed_applied, mut freed_reverted) = (0, 0);
    for seed in 0..SEEDS {
        let StructuralRun {
            mut tree,
            before,
            after,
            journal,
        } = run_structural(seed, &mut stats);
        let reverted = seed % 2 == 1;
        if reverted {
            journal.revert(&mut tree).unwrap();
        }
        let (now, other) = if reverted {
            (&before, &after)
        } else {
            (&after, &before)
        };
        let held = snapshot(&tree);
        journal.discard(&mut tree);
        assert_eq!(
            snapshot(&tree),
            held,
            "seed {seed}: discard changes nothing in the tree"
        );
        let in_tree: FxHashSet<NodeId> = now.iter().map(|(id, _, _)| *id).collect();
        for (id, _, _) in other {
            if !in_tree.contains(id) {
                assert!(
                    tree.get(*id).is_none(),
                    "seed {seed}: {id}, held out of the tree, is freed"
                );
                if reverted {
                    freed_reverted += 1;
                } else {
                    freed_applied += 1;
                }
            }
        }
    }
    assert!(
        freed_applied >= SEEDS / 10,
        "applied journals must free nodes: {freed_applied}"
    );
    assert!(
        freed_reverted >= SEEDS / 10,
        "reverted journals must free nodes: {freed_reverted}"
    );
}

// ── review round: parents, order across calls, walk_tree_from ─────

#[test]
fn test_structure_journal_names_the_parents_a_node_left_and_took() {
    do_structure_journal_names_the_parents_a_node_left_and_took();
}

/// One builder removes `b`, which has a child; moves `x` from `p` to
/// `q`; inserts `s` under `a`; raises `c`; and re-keys `d` within its
/// row. `parents` names the parent `b` left — which the arena no longer
/// links it to — both of `x`'s, and the one `s` took. The negative
/// controls answer `None`: `b`'s child, which kept its parent; `c`,
/// whose state alone changed; `d`, which moved within its row; and `e`,
/// which nothing touched.
pub fn do_structure_journal_names_the_parents_a_node_left_and_took() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3), (0, 4), (1, 5), (1, 6), (2, 7)]);
    let (a, b, c, d, p, q, e) = (t[0], t[1], t[2], t[3], t[4], t[5], t[6]);
    let root = tree.root;
    let kid = add(&mut tree, b, TestElement::new(0, 20));
    let x = add(&mut tree, p, TestElement::new(0, 30));
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![b], &mut builder).unwrap();
    let to_q = command(StructOp::Move {
        to: q,
        at: Placement::Last,
    });
    run(&mut tree, &to_q, vec![x], &mut builder).unwrap();
    let insert = command(StructOp::Insert {
        subtree: Subtree::leaf(TestElement::new(0, 40)),
        at: Placement::Last,
    });
    run(&mut tree, &insert, vec![a], &mut builder).unwrap();
    let s = row(&tree, a)[0];
    tree.update_journaled(c, &mut builder, |e| e.value = 33).unwrap();
    tree.update_journaled(d, &mut builder, |e| e.channel = 3).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        tree.arena()[b].parent(),
        None,
        "the arena no longer links b to root"
    );

    assert_eq!(journal.parents(b), Some((Some(root), None)));
    assert_eq!(journal.parents(x), Some((Some(p), Some(q))));
    assert_eq!(journal.parents(s), Some((None, Some(a))));
    assert!(journal.before(d).is_some(), "d is in the journal");
    assert!(journal.before(c).is_some(), "and so is c");
    for (name, node) in [("b's child", kid), ("c", c), ("d", d), ("e", e)] {
        assert_eq!(journal.parents(node), None, "{name} kept its parent");
    }
    journal.discard(&mut tree);
}

#[test]
fn test_structure_touched_names_an_entering_subtree_from_its_top() {
    do_structure_touched_names_an_entering_subtree_from_its_top();
}

/// One builder inserts `u` under `p`, then `s` under `q`, then moves `u`
/// into `s` — three calls. `u` was recorded first, but `s` is the top of
/// the subtree that entered, so `touched` names `s` and then `u`, never
/// a child before its parent.
pub fn do_structure_touched_names_an_entering_subtree_from_its_top() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let (p, q) = (t[0], t[1]);
    let leaf = |value| {
        command(StructOp::Insert {
            subtree: Subtree::leaf(TestElement::new(0, value)),
            at: Placement::Last,
        })
    };
    let mut builder = JournalBuilder::new();
    run(&mut tree, &leaf(10), vec![p], &mut builder).unwrap();
    let u = row(&tree, p)[0];
    run(&mut tree, &leaf(20), vec![q], &mut builder).unwrap();
    let s = row(&tree, q)[0];
    let into_s = command(StructOp::Move {
        to: s,
        at: Placement::Last,
    });
    run(&mut tree, &into_s, vec![u], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(s, Touch::Inserted), (u, Touch::Inserted)]
    );
}

#[test]
fn test_structure_touched_names_a_removed_subtree_in_one_pre_order() {
    do_structure_touched_names_a_removed_subtree_in_one_pre_order();
}

/// `a` holds `a1` (which holds `a12`) and `a2`. One builder removes
/// `a1`, then `a`. The subtree that left is `a`'s as it stood before,
/// so `touched` names `a, a1, a12, a2` — its pre-order then — where the
/// entries alone, oldest first, would name `a1`'s part before `a`.
pub fn do_structure_touched_names_a_removed_subtree_in_one_pre_order() {
    let (mut tree, t) = flat_tree(&[(0, 1)]);
    let a = t[0];
    let a1 = add(&mut tree, a, TestElement::new(0, 10));
    let a12 = add(&mut tree, a1, TestElement::new(0, 11));
    let a2 = add(&mut tree, a, TestElement::new(0, 20));
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![a1], &mut builder).unwrap();
    run(&mut tree, &command(StructOp::Remove), vec![a], &mut builder).unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![
            (a, Touch::Removed),
            (a1, Touch::Removed),
            (a12, Touch::Removed),
            (a2, Touch::Removed),
        ]
    );
    journal.discard(&mut tree);
}

#[test]
fn test_structure_walk_tree_from_refuses_a_node_out_of_the_tree() {
    do_structure_walk_tree_from_refuses_a_node_out_of_the_tree();
}

/// A journal removes `a`, which has a child. `walk_tree_from` at that
/// child — held by the journal, out of the tree — applies nothing, so
/// undoing the removal brings the child back as it was. The control:
/// the same walk at `b`, in the tree, raises it.
pub fn do_structure_walk_tree_from_refuses_a_node_out_of_the_tree() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let (a, b) = (t[0], t[1]);
    let kid = add(&mut tree, a, TestElement::new(0, 10));
    let mut builder = JournalBuilder::new();
    run(&mut tree, &command(StructOp::Remove), vec![a], &mut builder).unwrap();
    let journal = builder.close(&tree);
    let raise: TestMutatorTree = MutatorTree::new_with(Mutator::new(TestMutation::Add(5), plain(0)));
    let mroot = raise.root;
    walk_tree_from(&mut tree, &raise, kid, mroot, &0);
    assert_eq!(value(&tree, kid), 10, "the held child is not changed");
    walk_tree_from(&mut tree, &raise, b, mroot, &0);
    assert_eq!(value(&tree, b), 7, "the control: b, in the tree, is raised");
    journal.revert(&mut tree).unwrap();
    assert_eq!(row(&tree, a), vec![kid]);
    assert_eq!(value(&tree, kid), 10);
}

#[test]
fn test_structure_rebuilding_a_row_as_it_stood_before_is_one_pass() {
    do_structure_rebuilding_a_row_as_it_stood_before_is_one_pass();
}

/// A row of 4000 siblings, half of them moved — the first two of every
/// four, so a moved node follows the row's front, a kept sibling, or
/// another moved node. Rebuilding the row as it stood before (what
/// `touched` walks for a subtree that left) gives back the original
/// order, in exactly one step per moved node indexed, per kept sibling
/// and per moved node placed: no scan of the row per moved node, which
/// would be about 4 million steps here.
pub fn do_structure_rebuilding_a_row_as_it_stood_before_is_one_pass() {
    const SIBLINGS: usize = 4000;
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let ids: Vec<NodeId> = (0..SIBLINGS)
        .map(|i| add(&mut tree, root, TestElement::new(0, i as i64)))
        .collect();
    let mut before_parent: FxHashMap<NodeId, Option<NodeId>> = FxHashMap::default();
    let mut moved: Vec<(NodeId, Option<NodeId>)> = Vec::new();
    for (i, &id) in ids.iter().enumerate().filter(|(i, _)| i % 4 < 2).rev() {
        before_parent.insert(id, Some(root));
        moved.push((id, i.checked_sub(1).map(|prev| ids[prev])));
    }
    assert_eq!(moved.len(), SIBLINGS / 2, "precondition: half the row moved");
    let moved_from = FxHashMap::from_iter([(root, moved)]);
    let mut follows = FxHashMap::default();
    let mut row = Vec::new();
    let steps = children_before(
        tree.arena(),
        root,
        &before_parent,
        &moved_from,
        &mut follows,
        &mut row,
    );
    assert_eq!(row, ids, "the row as it stood before");
    assert_eq!(
        steps,
        SIBLINGS + SIBLINGS / 2,
        "one step per moved node indexed, per kept sibling and per moved node placed"
    );
}
