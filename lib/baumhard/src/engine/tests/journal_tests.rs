// SPDX-License-Identifier: MPL-2.0

//! Journals over [`TestElement`]: the seeded properties — a journaled
//! apply reverts and replays exactly, records one entry per element,
//! and is empty exactly when it changed nothing — and the cases a
//! property draws too rarely to rely on: positions restored after key
//! changes across several calls, a failing call leaving the tree as it
//! was, stale nodes refused before any write.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::engine::anchor::Anchors;
use crate::engine::journal::{JournalBuilder, Touch};
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::tests::journal_fixtures::{
    no_op_scenario, order, scenario, snapshot, state_above, Call, Scenario, SEEDS,
};
use crate::engine::tests::test_element::{
    add, add_mutator, plain, value, TestElement, TestField, TestMutation,
};
use crate::engine::tree::{MutatorTree, Tree, TreeError};
use indextree::NodeId;
use rustc_hash::FxHashSet;

/// The values of `parent`'s children, in sibling order — the row's
/// order, read through values the fixtures keep distinct.
fn row_values(tree: &Tree<TestElement>, parent: NodeId) -> Vec<i64> {
    parent.children(tree.arena()).map(|id| value(tree, id)).collect()
}

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

/// A one-node mutator tree: `mutation` on a `Plain` root.
fn root_mutator(mutation: TestMutation) -> MutatorTree<TestElement, TestMutation> {
    MutatorTree::new_with(Mutator::new(mutation, plain(0)))
}

// ── the properties ────────────────────────────────────────────────

#[test]
fn test_journal_revert_and_replay_are_exact() {
    do_journal_revert_and_replay_are_exact();
}

/// For every seed: run the scenario's journaled calls — walks and
/// updates, one builder — then close. `revert` must give back the
/// snapshot taken before the first call (same nodes, structure, sibling
/// order and states), `replay` the snapshot taken after the last, and
/// a second revert / replay cycle the same again. Both arrangements are
/// sorted.
///
/// The snapshots are taken from the tree, not from the journal, so the
/// expected values share no code with what is under test. The run is
/// required to have exercised what the property is about: seeds whose
/// calls changed values, seeds whose calls moved elements, and seeds
/// where elements moved across several calls of one builder.
pub fn do_journal_revert_and_replay_are_exact() {
    let mut changed = 0;
    let mut reordered = 0;
    let mut reordered_across_calls = 0;
    for seed in 0..SEEDS {
        let Scenario { mut tree, calls } = scenario(seed);
        let before = snapshot(&tree);
        let mut builder = JournalBuilder::new();
        for call in &calls {
            call.run(&mut tree, &mut builder)
                .unwrap_or_else(|e| panic!("seed {seed}: a generated call is valid, got {e}"));
        }
        let after = snapshot(&tree);
        tree.check_sorted().unwrap();
        let journal = builder.close(&tree);

        if before != after {
            changed += 1;
        }
        if order(&before) != order(&after) {
            reordered += 1;
            if calls.len() > 1 {
                reordered_across_calls += 1;
            }
        }
        assert_eq!(
            journal.is_empty(),
            before == after,
            "seed {seed}: empty exactly when nothing changed"
        );

        for cycle in 0..2 {
            journal.revert(&mut tree).unwrap();
            assert_eq!(
                snapshot(&tree),
                before,
                "seed {seed}, cycle {cycle}: revert restores the tree"
            );
            tree.check_sorted().unwrap();
            journal.replay(&mut tree).unwrap();
            assert_eq!(
                snapshot(&tree),
                after,
                "seed {seed}, cycle {cycle}: replay reproduces the apply"
            );
        }
    }
    assert!(
        changed > SEEDS / 2,
        "most seeds must change something, or this proves little: {changed}"
    );
    assert!(
        reordered >= SEEDS / 10,
        "seeds must move elements among their siblings: {reordered}"
    );
    assert!(
        reordered_across_calls >= SEEDS / 20,
        "seeds must move elements across several calls of one builder: {reordered_across_calls}"
    );
}

#[test]
fn test_journal_records_one_entry_per_element() {
    do_journal_records_one_entry_per_element();
}

/// For every seed: the scenario's calls run twice side by side, once
/// with one shared builder and once with a fresh builder per call. The
/// shared builder must hold exactly one entry per distinct element the
/// fresh builders recorded between them, however many calls applied to
/// it. The run must include elements applied to by more than one call,
/// or the property says nothing about repeats.
pub fn do_journal_records_one_entry_per_element() {
    let mut repeated = 0;
    for seed in 0..SEEDS {
        let Scenario { tree, calls } = scenario(seed);
        let mut shared_tree = tree.clone();
        let mut shared = JournalBuilder::new();
        let mut fresh_tree = tree;
        let mut distinct: FxHashSet<NodeId> = FxHashSet::default();
        let mut touches = 0;
        for call in &calls {
            call.run(&mut shared_tree, &mut shared).unwrap();
            let mut fresh = JournalBuilder::new();
            call.run(&mut fresh_tree, &mut fresh).unwrap();
            touches += fresh.recorded_len();
            distinct.extend(fresh.recorded_nodes());
        }
        assert_eq!(
            shared.recorded_len(),
            distinct.len(),
            "seed {seed}: one entry per element"
        );
        if touches > distinct.len() {
            repeated += 1;
        }
        let journal = shared.close(&shared_tree);
        let touched: Vec<NodeId> = journal.touched().map(|(id, _)| id).collect();
        let unique: FxHashSet<NodeId> = touched.iter().copied().collect();
        assert_eq!(
            unique.len(),
            touched.len(),
            "seed {seed}: a closed journal names each node once"
        );
    }
    assert!(
        repeated >= SEEDS / 10,
        "seeds must apply to some element in several calls: {repeated}"
    );
}

#[test]
fn test_journal_of_an_apply_that_changes_nothing_is_empty() {
    do_journal_of_an_apply_that_changes_nothing_is_empty();
}

/// For every seed: a walk whose every payload undoes itself within one
/// apply — `Add(0)`, an `Add` and its inverse, a channel change and its
/// inverse — closes to an empty journal, although the walk applied to
/// elements. The control: the same walk with `Add(1)` appended to its
/// root's payload closes to a journal that is not empty and names
/// every anchor the root applied to.
pub fn do_journal_of_an_apply_that_changes_nothing_is_empty() {
    let mut applied = 0;
    let mut controlled = 0;
    for seed in 0..SEEDS {
        let Scenario { mut tree, calls } = no_op_scenario(seed);
        let control_tree = tree.clone();
        let mut builder = JournalBuilder::new();
        for call in &calls {
            call.run(&mut tree, &mut builder).unwrap();
        }
        if builder.recorded_len() > 0 {
            applied += 1;
        }
        let journal = builder.close(&tree);
        assert!(
            journal.is_empty(),
            "seed {seed}: nothing changed, so nothing is journaled"
        );
        assert_eq!(journal.touched().count(), 0);

        // The control: the same walk, its root's payload replaced by a
        // real change. Every element of the root's kind among the
        // anchors gains 1; nothing else changes.
        let [Call::Walk { mutators, anchors }] = calls.as_slice() else {
            panic!("a no-op scenario is one walk");
        };
        let mut changing = mutators.clone();
        let root = changing.root;
        let root_key = changing.arena[root].get().key();
        *changing.arena[root].get_mut() = Mutator::new(TestMutation::Add(1), root_key);
        let mut tree = control_tree;
        let before = snapshot(&tree);
        let mut builder = JournalBuilder::new();
        changing
            .apply_at_journaled(&mut tree, anchors, state_above, &mut builder)
            .unwrap();
        let journal = builder.close(&tree);
        let raised = before
            .iter()
            .zip(snapshot(&tree))
            .filter(|((_, _, b), (_, _, a))| b.value + 1 == a.value)
            .count();
        assert_eq!(
            journal.touched().count(),
            raised,
            "seed {seed}: the control journals each raised element"
        );
        if raised > 0 {
            controlled += 1;
            assert!(!journal.is_empty());
        }
    }
    assert!(
        applied > SEEDS / 3,
        "the no-op walks must apply to elements: {applied}"
    );
    assert!(
        controlled > SEEDS / 2,
        "the controls must change something: {controlled}"
    );
}

// ── one entry per element, within one walk ────────────────────────

#[test]
fn test_journal_records_an_element_touched_twice_in_one_walk_once() {
    do_journal_records_an_element_touched_twice_in_one_walk_once();
}

/// A `RepeatWhile` whose predicate fails at once hands the target to
/// every after-mutator on its key: here two, so one walk applies to the
/// same element twice (`+10`, then `+100`). The journal holds one entry
/// for it, whose before is the value ahead of both applies and whose
/// after is the value behind both.
pub fn do_journal_records_an_element_touched_twice_in_one_walk_once() {
    let (mut tree, t) = flat_tree(&[(0, 500)]);
    let mut predicate = Predicate::new();
    predicate.fields.push((TestField::Value(100), Comparator::less()));
    let mut mt = MutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(predicate),
        plain(0),
    ));
    let mroot = mt.root;
    let repeated = add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), plain(0)));
    add_mutator(&mut mt, repeated, Mutator::new(TestMutation::Add(10), plain(0)));
    add_mutator(&mut mt, repeated, Mutator::new(TestMutation::Add(100), plain(0)));

    let root = tree.root;
    let mut builder = JournalBuilder::new();
    mt.apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    assert_eq!(
        value(&tree, t[0]),
        610,
        "both after-mutators applied to the one target"
    );
    assert_eq!(builder.recorded_len(), 1, "two applies, one entry");
    let journal = builder.close(&tree);
    assert_eq!(journal.touched().collect::<Vec<_>>(), vec![(t[0], Touch::Field)]);
    assert_eq!(journal.before(t[0]).map(|e| e.value), Some(500));
    assert_eq!(journal.after(t[0]).map(|e| e.value), Some(610));
    assert_eq!(
        journal.before(tree.root),
        None,
        "the root was visited, not applied to"
    );
}

// ── positions ─────────────────────────────────────────────────────

#[test]
fn test_journal_restores_the_order_of_siblings_a_walk_rekeyed() {
    do_journal_restores_the_order_of_siblings_a_walk_rekeyed();
}

/// Row `a b c d` on channels `0 0 0 1`. One walk moves `a` and `b` to
/// channel 1, so the row settles as `c d a b`. `revert` must give back
/// `a b c d` — restoring `b` behind `a` while `a` is still out of place
/// would not — and `replay` must give `c d a b` again.
pub fn do_journal_restores_the_order_of_siblings_a_walk_rekeyed() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3), (1, 4)]);
    let root = tree.root;
    // `RepeatWhile(value < 3)` picks `a` and `b` by value.
    let mut predicate = Predicate::new();
    predicate.fields.push((TestField::Value(3), Comparator::less()));
    let mut mt = MutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(predicate),
        plain(0),
    ));
    let mroot = mt.root;
    add_mutator(
        &mut mt,
        mroot,
        Mutator::new(TestMutation::SetChannel(1), plain(0)),
    );

    let mut builder = JournalBuilder::new();
    mt.apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    assert_eq!(
        row_values(&tree, root),
        vec![3, 4, 1, 2],
        "the walk moved a and b behind d"
    );
    let journal = builder.close(&tree);
    assert_eq!(journal.touched().count(), 2);

    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3, 4]);
    assert_eq!(tree.arena()[t[0]].get().channel, 0);
    tree.check_sorted().unwrap();
    journal.replay(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 4, 1, 2]);
    tree.check_sorted().unwrap();
}

#[test]
fn test_journal_restores_an_element_moved_twice_around_another_move() {
    do_journal_restores_an_element_moved_twice_around_another_move();
}

/// Row `A B C D E` keyed `0 0 0 2 3`, three updates on one builder:
/// `A → 1` (row `B C A D E`), then `D → 0` (row `B C D A E` — `D` left
/// from behind `A`), then `A → 4` (row `B C D E A`). The position `D`
/// had when the builder was created is behind `C`, not behind `A`,
/// where it stood at its own first change: a journal that took that
/// later position would put `D` back behind `A` and then move `A` to
/// the front, giving `A B C E D`. `revert` must give `A B C D E`.
pub fn do_journal_restores_an_element_moved_twice_around_another_move() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3), (2, 4), (3, 5)]);
    let root = tree.root;
    let (a, d) = (t[0], t[3]);
    let mut builder = JournalBuilder::new();
    tree.update_journaled(a, &mut builder, |e| e.channel = 1).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 1, 4, 5]);
    tree.update_journaled(d, &mut builder, |e| e.channel = 0).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 4, 1, 5]);
    tree.update_journaled(a, &mut builder, |e| e.channel = 4).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 4, 5, 1]);
    let journal = builder.close(&tree);

    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3, 4, 5]);
    tree.check_sorted().unwrap();
    journal.replay(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 4, 5, 1]);
}

#[test]
fn test_journal_restores_an_element_whose_neighbors_moved_before_it_was_recorded() {
    do_journal_restores_an_element_whose_neighbors_moved_before_it_was_recorded();
}

/// Row `A B C` keyed `0 0 1`. An update moves `A` to channel 2 (row
/// `B C A`); a second one, on the same builder, moves `B` to channel 3
/// (row `C A B`). When `B` is recorded its previous sibling is none —
/// `A` has left — but when the builder was created it was `A`. `revert`
/// must give `A B C`; restoring `B` to the front of the row would give
/// `B A C` or `A C B`, depending on the order.
pub fn do_journal_restores_an_element_whose_neighbors_moved_before_it_was_recorded() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (1, 3)]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 2)
        .unwrap();
    tree.update_journaled(t[1], &mut builder, |e| e.channel = 3)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 1, 2]);
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3]);
    journal.replay(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 1, 2]);
}

#[test]
fn test_journal_restores_an_element_a_walk_moved_a_sibling_in_front_of() {
    do_journal_restores_an_element_a_walk_moved_a_sibling_in_front_of();
}

/// Row `A B C` keyed `0 0 2`. A journaled walk moves `A` to channel 1,
/// and its settle puts `A` between `B` and `C` (row `B A C`); then a
/// journaled update on the same builder moves `C` to channel 0 (row
/// `B C A`). When `C` is recorded its previous sibling is `A`, which the
/// walk put there; when the builder was created it was `B`. `revert`
/// must give `A B C` — `C` restored behind `A` would give `A C B`.
pub fn do_journal_restores_an_element_a_walk_moved_a_sibling_in_front_of() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (2, 3)]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    root_mutator(TestMutation::SetChannel(1))
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![t[0]]), |_, _| 0, &mut builder)
        .unwrap();
    assert_eq!(
        row_values(&tree, root),
        vec![2, 1, 3],
        "the walk's settle put A between B and C"
    );
    tree.update_journaled(t[2], &mut builder, |e| e.channel = 0)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 1]);
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3]);
    journal.replay(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 3, 1]);
}

#[test]
fn test_journal_puts_back_an_element_that_followed_a_moved_one() {
    do_journal_puts_back_an_element_that_followed_a_moved_one();
}

/// Row `P M U` keyed `0 0 0`. One builder moves `P` to channel 1
/// (`M U P`), then `M` to channel 1 (`U P M`). `M` stands behind `P` in
/// both arrangements, so its own recorded position did not change — but
/// `P` moved, and `M` with it. The journal must count `M` as moved:
/// `revert` gives `P M U`, where treating `M` as in place would leave
/// `M` behind `U`. Its value never changed, so a journal that dropped
/// it would also be wrong about what changed.
pub fn do_journal_puts_back_an_element_that_followed_a_moved_one() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3)]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 1)
        .unwrap();
    tree.update_journaled(t[1], &mut builder, |e| e.channel = 1)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 1, 2]);
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3]);
    journal.replay(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![3, 1, 2]);
}

#[test]
fn test_journal_of_a_move_there_and_back_is_empty() {
    do_journal_of_a_move_there_and_back_is_empty();
}

/// Row `A B C` keyed `0 0 1`. Two updates on one builder move `B` to
/// channel 2 and back to channel 0: it lands at the end of the channel-0
/// run, which is where it started, so nothing changed and the journal is
/// empty. The control: the same there-and-back on `A`, which is not
/// last in its run, ends behind `B` — a change — and the journal
/// restores it.
pub fn do_journal_of_a_move_there_and_back_is_empty() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (1, 3)]);
    let root = tree.root;
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[1], &mut builder, |e| e.channel = 2)
        .unwrap();
    tree.update_journaled(t[1], &mut builder, |e| e.channel = 0)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3], "B is back where it was");
    assert!(builder.close(&tree).is_empty());

    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 2)
        .unwrap();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 0)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![2, 1, 3], "A ends the channel-0 run");
    let journal = builder.close(&tree);
    assert!(!journal.is_empty(), "A changed place, though not state");
    journal.revert(&mut tree).unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 2, 3]);
}

// ── failure ───────────────────────────────────────────────────────

#[test]
fn test_a_failing_journaled_call_leaves_the_tree_as_the_builder_found_it() {
    do_a_failing_journaled_call_leaves_the_tree_as_the_builder_found_it();
}

/// A builder takes two calls: the first applies `+10` to every child
/// and moves one of them; the second names a removed node among its
/// anchors and fails. The failing call reverts the whole builder before
/// returning its error — the tree is again exactly as it was before the
/// first call, and the builder is empty. A failing journaled update
/// does the same.
pub fn do_a_failing_journaled_call_leaves_the_tree_as_the_builder_found_it() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (1, 3)]);
    let root = tree.root;
    let gone = add(&mut tree, root, TestElement::new(2, 9));
    tree.remove_subtree(gone).unwrap();
    let before = snapshot(&tree);

    let mut adds = MutatorTree::new();
    let mroot = adds.root;
    add_mutator(
        &mut adds,
        mroot,
        Mutator::new_macro(vec![TestMutation::Add(10)], plain(0)),
    );
    add_mutator(&mut adds, mroot, Mutator::new(TestMutation::Add(10), plain(1)));
    let mut builder = JournalBuilder::new();
    adds.apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 5)
        .unwrap();
    assert_ne!(snapshot(&tree), before, "the first calls changed the tree");

    let result = root_mutator(TestMutation::Add(1)).apply_at_journaled(
        &mut tree,
        &Anchors::Nodes(vec![t[1], gone]),
        |_, _| 0,
        &mut builder,
    );
    assert_eq!(result, Err(TreeError::StaleNode(gone)));
    assert_eq!(
        snapshot(&tree),
        before,
        "the failing call reverted the whole builder"
    );
    assert_eq!(builder.recorded_len(), 0, "and left it empty");

    adds.apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    assert_ne!(snapshot(&tree), before);
    let result = tree.update_journaled(gone, &mut builder, |e| e.value = 0);
    assert_eq!(result, Err(TreeError::StaleNode(gone)));
    assert_eq!(
        snapshot(&tree),
        before,
        "a failing update reverts the builder too"
    );
    assert_eq!(builder.recorded_len(), 0);
}

#[test]
fn test_revert_and_replay_refuse_a_stale_node_before_writing() {
    do_revert_and_replay_refuse_a_stale_node_before_writing();
}

/// A journal changes three children and moves one; then the last child
/// it names is removed from the tree. `revert` and `replay` both return
/// `StaleNode` naming it, and the tree is unchanged — the two entries
/// that could still be written were not.
pub fn do_revert_and_replay_refuse_a_stale_node_before_writing() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3)]);
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 1)
        .unwrap();
    tree.update_journaled(t[1], &mut builder, |e| e.value = 20)
        .unwrap();
    tree.update_journaled(t[2], &mut builder, |e| e.value = 30)
        .unwrap();
    let journal = builder.close(&tree);
    assert_eq!(journal.touched().count(), 3);

    tree.remove_subtree(t[2]).unwrap();
    let held = snapshot(&tree);
    assert_eq!(journal.revert(&mut tree), Err(TreeError::StaleNode(t[2])));
    assert_eq!(snapshot(&tree), held, "revert wrote nothing");
    assert_eq!(journal.replay(&mut tree), Err(TreeError::StaleNode(t[2])));
    assert_eq!(snapshot(&tree), held, "replay wrote nothing");
}

#[test]
fn test_journal_names_every_change_as_a_field_touch() {
    do_journal_names_every_change_as_a_field_touch();
}

/// The structural touches are defined but no journal produces them:
/// a value change and a key change are both `Touch::Field`, oldest
/// first, and `before` / `after` answer for exactly the touched nodes.
pub fn do_journal_names_every_change_as_a_field_touch() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2), (0, 3)]);
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[2], &mut builder, |e| e.value = 30)
        .unwrap();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 4)
        .unwrap();
    let journal = builder.close(&tree);
    assert_eq!(
        journal.touched().collect::<Vec<_>>(),
        vec![(t[2], Touch::Field), (t[0], Touch::Field)]
    );
    assert_eq!(journal.before(t[2]).map(|e| e.value), Some(3));
    assert_eq!(journal.after(t[2]).map(|e| e.value), Some(30));
    assert_eq!(journal.before(t[0]).map(|e| e.channel), Some(0));
    assert_eq!(journal.after(t[0]).map(|e| e.channel), Some(4));
    assert!(journal.before(t[1]).is_none());
    assert!(journal.after(t[1]).is_none());
}

#[test]
fn test_revert_refuses_a_tree_the_journal_does_not_fit() {
    do_revert_refuses_a_tree_the_journal_does_not_fit();
}

/// A journal moves the first of two children (`root → [a, b]`, `a`
/// ends behind `b`). Applied to another tree whose ids exist but are
/// arranged otherwise (`root → [c → [d]]`, where `b`'s id names `d`,
/// a grandchild), `revert` finds the sibling it would place against
/// under another parent and refuses with `NotAChild`, writing nothing.
pub fn do_revert_refuses_a_tree_the_journal_does_not_fit() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let mut builder = JournalBuilder::new();
    tree.update_journaled(t[0], &mut builder, |e| e.channel = 1)
        .unwrap();
    let journal = builder.close(&tree);

    let mut other = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let other_root = other.root;
    let c = add(&mut other, other_root, TestElement::new(0, 7));
    let d = add(&mut other, c, TestElement::new(0, 8));
    assert_eq!((c, d), (t[0], t[1]), "the other tree reuses the journal's ids");
    let held = snapshot(&other);
    assert_eq!(
        journal.revert(&mut other),
        Err(TreeError::NotAChild {
            parent: other_root,
            node: d
        })
    );
    assert_eq!(snapshot(&other), held);
}

#[test]
fn test_journal_skips_elements_visited_but_not_applied_to() {
    do_journal_skips_elements_visited_but_not_applied_to();
}

/// A `MapChildren` pairs its children with the target's by position
/// and applies each one's payload: here a `Void`, then an `Add(1)`.
/// The first child is visited and paired, but nothing is applied to
/// it, so the journal records only the second — and the root, which
/// the payload-free instruction steers from, not at all.
pub fn do_journal_skips_elements_visited_but_not_applied_to() {
    let (mut tree, t) = flat_tree(&[(0, 1), (0, 2)]);
    let root = tree.root;
    let mut mt = MutatorTree::new_with(Mutator::new_instruction(Instruction::MapChildren, plain(0)));
    let mroot = mt.root;
    add_mutator(&mut mt, mroot, Mutator::new_void(plain(0)));
    add_mutator(&mut mt, mroot, Mutator::new(TestMutation::Add(1), plain(0)));
    let mut builder = JournalBuilder::new();
    mt.apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| 0, &mut builder)
        .unwrap();
    assert_eq!(row_values(&tree, root), vec![1, 3]);
    assert_eq!(builder.recorded_nodes().collect::<Vec<_>>(), vec![t[1]]);
}
